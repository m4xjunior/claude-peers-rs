//! peers-client — servidor MCP stdio de la red claude-peers-rs.
//!
//! Una instancia por cada Claude Code. Habla MCP por stdio con su Claude y HTTP con el
//! broker. Al arrancar: registra (con id estable si se pasa --id), lanza el latido y el
//! bucle de recepción que empuja mensajes entrantes a la sesión como canal.
//!
//! NO levanta el broker por su cuenta (separación limpia): si el broker no responde, lo
//! reporta por stderr y sigue vivo reintentando — nunca entra en pánico.

mod broker;
mod contexto;
mod mcp;

use std::sync::Arc;

use anyhow::Result;
use broker::ClienteBroker;
use clap::Parser;
use peers_core::*;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "peers-client", about = "Servidor MCP de la red claude-peers-rs")]
struct Args {
    /// Id estable de esta instancia (por papel, ej. "claudia"). Es el FIX clave: con el
    /// mismo id, un reinicio hereda la fila de mensajes pendientes.
    #[arg(long, env = "CLAUDE_PEERS_ID")]
    id: Option<String>,

    /// URL completa del broker. Configurable para apuntar a un broker remoto (FIX #3).
    #[arg(long, env = "CLAUDE_PEERS_BROKER_URL")]
    broker_url: Option<String>,

    /// Puerto del broker (si no se pasa --broker-url, se compone con localhost).
    #[arg(long, env = "CLAUDE_PEERS_PORT", default_value_t = PUERTO_DEFECTO)]
    puerto: u16,

    /// Token de acceso al broker (debe coincidir con el del broker si éste lo exige).
    #[arg(long, env = "CLAUDE_PEERS_TOKEN")]
    token: Option<String>,
}

/// Estado compartido de la instancia: su id asignado por el broker.
struct EstadoCliente {
    id: RwLock<Option<String>>,
    /// Secreto de sesión emitido por el broker en `/registrar` (E-10, anti-spoofing). Se guarda
    /// aquí (en memoria, nunca en disco) y se presenta en cada `/enviar` vía el header
    /// `X-Peers-Secreto`, para que el broker ate el `de_id` a ESTA instancia. Se ROTA en cada
    /// re-registro (el broker emite uno nuevo). `None` = broker viejo sin E-10 → se envía sin
    /// header (degradación graciosa: el broker cae a la ventana de compat).
    secreto: RwLock<Option<String>>,
    /// Id que ESTA instancia pidió al broker (derivado de la carpeta o de --id). Se le
    /// anuncia al agente en el initialize para que sepa con qué id deben responderle.
    /// OJO: si el broker detectó colisión puede haber asignado un sufijo (-2); el id REAL
    /// vive en `id` (RwLock). Este campo es el "preferido", suficiente para el anuncio
    /// inicial; el id real se confirma en la respuesta del registro.
    id_efectivo: String,
    broker: ClienteBroker,
    directorio: String,
    repo_git: Option<String>,
    /// "owner/repo" del remote origin (dinámico). El broker lo usa para abrir issues
    /// en el repo donde este peer trabaja. None si el dir no es repo GitHub → sin issue.
    repo_github: Option<String>,
    /// El `Peer<RoleServer>` que rmcp expone tras `serve(stdio())`: el canal de salida hacia la
    /// sesión. El bucle de recepción lo usa para emitir el push del `<channel>` (E-21). Se rellena
    /// una vez, justo tras arrancar el servicio rmcp; `None` hasta entonces (el bucle espera).
    peer: RwLock<Option<rmcp::service::Peer<rmcp::RoleServer>>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs: SIEMPRE a stderr (stdout está reservado al protocolo MCP) Y, además, a un archivo en el
    // temp del SO (`<tmp>/claude-peers-client.log`). El archivo es el canal de diagnóstico cuando el
    // stderr se pierde (plugin en Windows: el .exe corre bajo un launcher y su stderr no es visible).
    // Best-effort: si no se puede abrir el archivo, solo queda stderr.
    //
    // El archivo es NDJSON (`.json()`, una línea = un evento), no texto formateado: así el
    // diagnóstico remoto (pedirle a Daniela "mandame el .log") se puede `jq` filtrar por
    // `event_name`/`outcome` sin parsear texto libre. El stderr sigue en texto legible para
    // desarrollo interactivo (nadie hace `tail -f` de JSON a mano). Inspirado en el patrón
    // request_id/session_id de `referencias/lexusfx-code/src/agent/client.rs` (solo lectura).
    use tracing_subscriber::prelude::*;
    let filtro = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    let capa_stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filtro());
    let registro = tracing_subscriber::registry().with(capa_stderr);
    // Capa de archivo (append). El path va al temp del SO — en Windows `%TEMP%\claude-peers-client.log`.
    let ruta_log = std::env::temp_dir().join("claude-peers-client.log");
    match std::fs::OpenOptions::new().create(true).append(true).open(&ruta_log) {
        Ok(archivo) => {
            // El archivo captura hasta DEBUG del propio client (diagnóstico del push), sin ensuciar
            // el stderr (que queda en el nivel del env, típicamente info). `RUST_LOG` lo puede subir.
            let filtro_archivo = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,peers_client=debug"));
            let capa_archivo = tracing_subscriber::fmt::layer()
                .json()
                .with_writer(archivo)
                .with_filter(filtro_archivo);
            registro.with(capa_archivo).init();
        }
        Err(_) => {
            // No se pudo abrir el archivo (permisos, etc.): al menos queda el stderr.
            registro.init();
        }
    }

    let args = Args::parse();
    let url = args
        .broker_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", args.puerto));

    let directorio = contexto::directorio_actual();
    // Id efectivo: el --id/CLAUDE_PEERS_ID si vino; si no, derivado del directorio. Así
    // `claude` sin argumentos ya tiene id ESTABLE por terminal (hereda cola al reiniciar)
    // sin que Max escriba nada. A partir de aquí, el id_preferido SIEMPRE está presente.
    let id_efectivo = args
        .id
        .clone()
        .unwrap_or_else(|| contexto::id_desde_directorio(&directorio));
    let repo_git = contexto::repo_git(&directorio);
    // Resuelve "owner/repo" del remote origin AQUÍ (en la máquina del peer, con gh logado).
    // El broker no lo resuelve: recibe el valor y abre la issue en ESE repo (dinámico).
    let repo_github = contexto::repo_github(&directorio);
    let tty = contexto::tty();

    info!("directorio: {directorio}");
    info!("repo git: {}", repo_git.as_deref().unwrap_or("(ninguno)"));
    info!("repo github: {}", repo_github.as_deref().unwrap_or("(ninguno)"));
    info!("broker: {url}");

    let broker = ClienteBroker::nuevo(url.clone(), args.token.clone());
    if !broker.esta_vivo().await {
        // No abortamos: registramos el aviso y seguimos; el broker puede levantarse luego.
        warn!("el broker no responde en {url} — reintentaré en el próximo latido");
    }

    // Resumen inicial local (sin API externa). Usa el id_efectivo como papel.
    let resumen = contexto::resumen_inicial(&id_efectivo, &directorio, repo_git.as_deref());

    // Registro con id_preferido = id_efectivo (siempre presente) → herencia de fila en restart.
    let id_asignado = match broker
        .registrar(&PeticionRegistrar {
            pid: std::process::id() as i64,
            hostname: contexto::hostname(),
            directorio: directorio.clone(),
            repo_git: repo_git.clone(),
            repo_github: repo_github.clone(),
            tty,
            resumen,
            id_preferido: Some(id_efectivo.clone()),
        })
        .await
    {
        Ok(r) => {
            // El broker resuelve colisiones SUFIJANDO en silencio (`resolver_id_sin_colision`):
            // si el id pedido lo ocupa otro proceso vivo, devuelve `<id>-2` y el registro es un
            // ÉXITO — solo queda un warn! en el log DEL BROKER, que este proceso nunca ve. Para
            // una sesión interactiva es una molestia; para un consumidor no interactivo es una
            // trampa: se queda escuchando una cola distinta de aquella a la que los demás le
            // escriben, y sus comandos entrantes (incluido un paro de emergencia) esperan para
            // siempre en la cola del id que pidió. Sin error, sin rebote, sin señal.
            //
            // No podemos rechazar el registro desde aquí sin dejar al peer fuera de la red, pero
            // callarlo es lo único inaceptable. Se emite a nivel ERROR y con el par de ids para
            // que sea grepeable en el NDJSON.
            if r.id != id_efectivo {
                error!(
                    event_name = "registro_id_sufijado",
                    outcome = "failure",
                    id_pedido = %id_efectivo,
                    id_asignado = %r.id,
                    "el id '{}' ya estaba ocupado por otro proceso VIVO: esta instancia quedó como \
                     '{}'. Los mensajes dirigidos a '{}' NO llegan aquí. Si este peer recibe \
                     órdenes por la red, detenelo y relanzalo con un id libre.",
                    id_efectivo, r.id, id_efectivo
                );
            } else {
                info!("registrado como instancia '{}'", r.id);
            }
            // E-10: guardamos el secreto emitido (si el broker lo manda; broker viejo → None).
            (Some(r.id), r.secreto)
        }
        Err(e) => {
            warn!("no se pudo registrar ahora ({e:#}); seguiré e intentaré por latido");
            (None, None)
        }
    };
    let (id_asignado, secreto_inicial) = id_asignado;

    let estado = Arc::new(EstadoCliente {
        id: RwLock::new(id_asignado),
        secreto: RwLock::new(secreto_inicial),
        id_efectivo: id_efectivo.clone(),
        broker,
        directorio,
        repo_git,
        repo_github,
        peer: RwLock::new(None),
    });

    // Loop de latido (cada 15s) — mantiene viva la instancia y re-registra si hizo falta.
    // Pasa el id_efectivo (no args.id) para que el re-registro conserve el id estable.
    lanzar_latido(estado.clone(), Some(id_efectivo.clone()));

    // Loop de recepción (cada 1s) — empuja mensajes entrantes a la sesión como canal.
    // Espera a que el `peer` de rmcp esté disponible antes de emitir nada.
    lanzar_recepcion(estado.clone());

    // Limpieza al recibir SIGINT/SIGTERM: damos de baja la instancia del broker.
    lanzar_limpieza_senales(estado.clone());

    // Arranca el servidor MCP sobre stdio con rmcp (E-21): el SDK gestiona el handshake,
    // tools/list y tools/call a partir de las tools `#[tool]` de `ServidorPeers`. Guardamos el
    // `Peer` (canal de salida) en el estado para que el bucle de recepción emita el push del canal.
    use rmcp::transport::stdio;
    use rmcp::ServiceExt;
    let servidor = mcp::ServidorPeers::nuevo(estado.clone());
    let servicio = match servidor.serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            error!("no se pudo arrancar el servidor MCP (rmcp): {e:#}");
            dar_de_baja(&estado).await;
            return Ok(());
        }
    };
    // Publica el peer: a partir de aquí el bucle de recepción puede empujar el `<channel>`.
    *estado.peer.write().await = Some(servicio.peer().clone());
    info!("servidor MCP (rmcp) arrancado; peer publicado");

    // Corre hasta que la sesión (stdin) se cierre: el Claude padre terminó → salida limpia.
    if let Err(e) = servicio.waiting().await {
        warn!("el servicio MCP terminó con error: {e:#}");
    }
    info!("sesión MCP finalizada, dando de baja la instancia");
    dar_de_baja(&estado).await;
    Ok(())
}

pub(crate) async fn tool_listar(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let alcance = match args.get("alcance").and_then(Value::as_str) {
        Some("maquina") => Alcance::Maquina,
        Some("directorio") => Alcance::Directorio,
        Some("repo") => Alcance::Repo,
        _ => Alcance::Maquina,
    };
    let mi_id = estado.id.read().await.clone();
    let instancias = estado
        .broker
        .listar(&PeticionListar {
            alcance,
            directorio: estado.directorio.clone(),
            repo_git: estado.repo_git.clone(),
            excluir_id: mi_id,
        })
        .await
        .map_err(|e| format!("Error al listar instancias: {e}"))?;

    if instancias.is_empty() {
        return Ok(format!("No se encontraron otras instancias (alcance: {:?}).", alcance));
    }
    let bloques: Vec<String> = instancias
        .iter()
        .map(|p| {
            let mut partes = vec![
                format!("ID: {}", p.id),
                format!("PID: {}", p.pid),
                format!("Directorio: {}", p.directorio),
            ];
            if let Some(r) = &p.repo_git {
                partes.push(format!("Repo: {r}"));
            }
            if let Some(t) = &p.tty {
                partes.push(format!("TTY: {t}"));
            }
            if !p.resumen.is_empty() {
                partes.push(format!("Resumen: {}", p.resumen));
            }
            partes.push(format!("Visto: {}", p.visto_en));
            partes.join("\n  ")
        })
        .collect();
    Ok(format!(
        "Se encontraron {} instancia(s):\n\n{}",
        instancias.len(),
        bloques.join("\n\n")
    ))
}

pub(crate) async fn tool_enviar(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let para_id = args
        .get("para_id")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'para_id'")?;
    // OJO: la tool recibe el campo como 'mensaje' pero hacia el broker viaja como 'texto'.
    let mensaje = args
        .get("mensaje")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'mensaje'")?;

    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    // E-10: presenta el secreto de sesión para que el broker verifique que este de_id es nuestro.
    let secreto = estado.secreto.read().await.clone();

    let resp = estado
        .broker
        .enviar_verificado(
            &PeticionEnviar {
                de_id: mi_id,
                para_id: para_id.to_string(),
                texto: mensaje.to_string(),
            },
            secreto.as_deref(),
        )
        .await
        .map_err(|e| format!("Error al enviar el mensaje: {e}"))?;

    if !resp.ok {
        return Err(format!(
            "No se pudo enviar: {}",
            resp.error.unwrap_or_else(|| "destino desconocido".into())
        ));
    }
    Ok(format!("Mensaje enviado a la instancia {para_id}"))
}

/// Broadcast (bug #6, acotado): envía `mensaje` a TODAS las demás instancias vivas en la máquina,
/// en una sola llamada. NO hay endpoint de broadcast en el broker (`/enviar` sigue siendo 1:1) —
/// esta tool hace fan-out en el cliente: lista instancias (mismo alcance/exclusión que
/// `listar_instancias`) y llama `enviar()` una vez por destino. Aditivo: no toca el protocolo
/// existente, sólo lo reusa. Degradación: un fallo individual NO aborta el resto (un peer caído a
/// mitad del broadcast no debe silenciar el aviso a los demás); el resumen final distingue
/// éxitos de fallos para que el agente sepa si algún destino no lo recibió.
pub(crate) async fn tool_avisar_equipo(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let mensaje = args
        .get("mensaje")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'mensaje'")?;

    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;

    let instancias = estado
        .broker
        .listar(&PeticionListar {
            alcance: Alcance::Maquina,
            directorio: estado.directorio.clone(),
            repo_git: estado.repo_git.clone(),
            excluir_id: Some(mi_id.clone()),
        })
        .await
        .map_err(|e| format!("Error al listar instancias para el broadcast: {e}"))?;

    if instancias.is_empty() {
        return Ok("No hay otras instancias vivas — nadie a quien avisar.".to_string());
    }

    // E-10: el broadcast también presenta el secreto (cada envío del fan-out es un /enviar).
    let secreto = estado.secreto.read().await.clone();
    let mut enviados: Vec<String> = Vec::new();
    let mut fallidos: Vec<String> = Vec::new();
    for inst in &instancias {
        let resultado = estado
            .broker
            .enviar_verificado(
                &PeticionEnviar {
                    de_id: mi_id.clone(),
                    para_id: inst.id.clone(),
                    texto: mensaje.to_string(),
                },
                secreto.as_deref(),
            )
            .await;
        match resultado {
            Ok(resp) if resp.ok => enviados.push(inst.id.clone()),
            Ok(resp) => fallidos.push(format!(
                "{} ({})",
                inst.id,
                resp.error.unwrap_or_else(|| "rechazado".to_string())
            )),
            Err(e) => fallidos.push(format!("{} (error de red: {e})", inst.id)),
        }
    }

    let mut resumen = format!(
        "Avisado a {}/{} instancia(s): {}",
        enviados.len(),
        instancias.len(),
        enviados.join(", ")
    );
    if !fallidos.is_empty() {
        resumen.push_str(&format!("\nNo recibieron el aviso: {}", fallidos.join(", ")));
    }
    Ok(resumen)
}

pub(crate) async fn tool_definir_resumen(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let resumen = args
        .get("resumen")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'resumen'")?;
    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    estado
        .broker
        .definir_resumen(&mi_id, resumen)
        .await
        .map_err(|e| format!("Error al definir el resumen: {e}"))?;
    Ok(format!("Resumen actualizado: \"{resumen}\""))
}

pub(crate) async fn tool_revisar(estado: &Arc<EstadoCliente>) -> Result<String, String> {
    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    let resp = estado
        .broker
        .recibir(&mi_id)
        .await
        .map_err(|e| format!("Error al revisar mensajes: {e}"))?;
    if resp.mensajes.is_empty() {
        return Ok("No hay mensajes nuevos.".into());
    }
    let bloques: Vec<String> = resp
        .mensajes
        .iter()
        .map(|m| format!("De {} ({}):\n{}", m.de_id, m.enviado_en, m.texto))
        .collect();
    // Drena lo entregado (ver la nota larga del bucle de push sobre por qué `Procesado`).
    //
    // Esta era la vía que MÁS duplicaba, y por partida doble: `/recibir` es un peek NO
    // destructivo, esta función no tocaba `empujados` (ese HashSet vive en la task del bucle de
    // push, fuera de alcance) y NO confirmaba ningún estado — ni `Entregado` ni `Leido`. O sea:
    // devolvía la bandeja ENTERA desde el principio en cada llamada. Verificado con el peer del
    // trader el 27/07/2026 desde el lado receptor: tres llamadas seguidas le devolvieron 17 → 19
    // → 20 mensajes, siempre incluyendo todos los anteriores. Y es justo la vía que usa un
    // consumidor DELIBERADO (un agente que llama la tool a mano), no la de respaldo.
    //
    // Best-effort a propósito: si el broker no responde al confirmar, el agente YA tiene el
    // contenido delante — devolver un error aquí sería peor (perdería la lectura por un fallo
    // del acuse). Se queda en la bandeja y se drena en la próxima llamada.
    let ids: Vec<i64> = resp.mensajes.iter().map(|m| m.id).collect();
    if let Err(e) = estado.broker.confirmar(&ids, EstadoMensaje::Procesado).await {
        warn!(
            event_name = "confirmar_estado_mensaje",
            outcome = "failure",
            estado = "Procesado",
            via = "revisar_mensajes",
            cantidad = ids.len(),
            error = %e,
            "no se pudieron confirmar {} mensaje(s) leídos por revisar_mensajes: {e:#}",
            ids.len()
        );
    }
    // "pendiente(s)", no "nuevo(s)": el rótulo viejo era un `format!` sobre el largo de la
    // bandeja completa, así que afirmaba "nuevos" sobre mensajes ya leídos hacía horas. Ahora
    // que la lectura drena, lo que venga en la próxima llamada sí será nuevo de verdad.
    Ok(format!(
        "{} mensaje(s) pendiente(s):\n\n{}",
        resp.mensajes.len(),
        bloques.join("\n\n---\n\n")
    ))
}

/// Estado de entrega de los mensajes que ESTE peer envió: ¿llegaron, los leyeron, los procesaron?
///
/// INTENCIÓN: cerrar el circuito que el emisor no puede cerrar solo. "Enviado con éxito" solo
/// dice que el broker lo encoló; no dice que del otro lado haya alguien. El caso que esto cubre
/// —y que motivó la petición— es el id muerto DENTRO de la ventana de liveness de 45s: el envío
/// devuelve ok, la cola no la consume nadie, y desde el emisor es idéntico a una entrega buena.
/// Con esto, un peer que manda una validación crítica puede exigir prueba en vez de asumirla.
///
/// Cómo leer los estados: `enviado` = encolado, nadie lo vio todavía (si persiste, sospechá del
/// destinatario); `entregado`/`leido` = llegó a su sesión y se renderizó; `procesado` = además
/// salió de su bandeja. `leido` sin `procesado` NO es un problema: significa que el destinatario
/// corre un client viejo, que entrega igual pero no drena.
pub(crate) async fn tool_estado_entrega(
    estado: &Arc<EstadoCliente>,
    args: &Value,
) -> Result<String, String> {
    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    let limite = args
        .get("limite")
        .and_then(Value::as_i64)
        .unwrap_or(15)
        .clamp(1, 100) as usize;
    let para = args.get("para").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    // Pedimos un tramo amplio del feed global y filtramos por emisor acá: el endpoint no tiene
    // filtro por `de_id` y añadirlo al broker obligaría a redesplegarlo (los peers vivos no lo
    // reinician). El recorte del broker ya acota el coste.
    let todos = estado
        .broker
        .historial_global(Some(500))
        .await
        .map_err(|e| format!("Error al consultar el estado de entrega: {e}"))?;
    let mios: Vec<&Mensaje> = todos
        .iter()
        .filter(|m| m.de_id == mi_id)
        .filter(|m| para.is_none_or(|p| m.para_id == p))
        .rev()
        .take(limite)
        .collect();
    if mios.is_empty() {
        return Ok(match para {
            Some(p) => format!("No hay mensajes tuyos a '{p}' en el tramo reciente del historial."),
            None => "No hay mensajes tuyos en el tramo reciente del historial.".into(),
        });
    }
    // ¿Sigue vivo cada destino? El `para_id` YA es la identidad de quien consume — una cola solo
    // la puede drenar un peer registrado con ese id, así que no hace falta (ni existe) un registro
    // aparte de "quién lo consumió". Lo que el emisor no puede ver es si ese id sigue perteneciendo
    // a alguien: un `Enviado` hacia un id que ya no está en la red es una cola huérfana confirmada,
    // no una sospecha. Best-effort: si el listado falla, se omite la anotación en vez de fallar.
    let vivos: Vec<String> = estado
        .broker
        .listar(&PeticionListar {
            alcance: Alcance::Maquina,
            directorio: estado.directorio.clone(),
            repo_git: estado.repo_git.clone(),
            excluir_id: None,
        })
        .await
        .map(|is| is.into_iter().map(|i| i.id).collect())
        .unwrap_or_default();
    let lineas: Vec<String> = mios
        .iter()
        .rev()
        .map(|m| {
            let destino_vivo = vivos.iter().any(|v| *v == m.para_id);
            let alerta = match (m.estado, destino_vivo) {
                // Encolado y el destino ya ni existe: cola huérfana, esto no llega nunca.
                (EstadoMensaje::Enviado, false) => "  ⚠ SIN ENTREGAR — destino YA NO ESTÁ en la red",
                (EstadoMensaje::Enviado, true) => "  ⚠ SIN ENTREGAR — destino vivo pero no lo consumió",
                // Consumido por alguien que ya se fue: llegó, pero ese id no responderá más.
                (_, false) => "  (destino ya no está en la red)",
                _ => "",
            };
            format!(
                "[{:?}] → {} ({}){}\n    {}",
                m.estado,
                m.para_id,
                m.enviado_en,
                alerta,
                recorte(&m.texto, 70)
            )
        })
        .collect();
    let sin_entregar = mios
        .iter()
        .filter(|m| m.estado == EstadoMensaje::Enviado)
        .count();
    let resumen = if sin_entregar > 0 {
        format!(
            "\n\n⚠ {sin_entregar} de {} SIN entregar: encolados pero nadie los consumió. Verificá con \
             listar_instancias que el destino siga vivo — un id que cambió deja la cola huérfana.",
            mios.len()
        )
    } else {
        format!("\n\nLos {} llegaron a destino.", mios.len())
    };
    Ok(format!(
        "Estado de entrega de tus últimos {} mensaje(s):\n\n{}{}",
        mios.len(),
        lineas.join("\n"),
        resumen
    ))
}

/// Relee el historial durable de MI PROPIA cola, incluidos los mensajes ya procesados.
///
/// INTENCIÓN: la red de contención de `revisar_mensajes`. Desde que la lectura drena la bandeja,
/// un mensaje que la compactación de contexto se llevó no volvía por ninguna vía — el agente
/// quedaba vivo y convencido de haberlo leído todo. Esto le devuelve el acceso a lo que ya salió
/// de la bandeja. Es SOLO LECTURA y no cambia ningún estado: releer no "desprocesa" nada.
///
/// Acotado a `limite` (default 20, los más recientes) porque el historial retiene hasta
/// `RETENCION_HISTORIAL` (500) por cola y volcar 500 mensajes al contexto sería justo el problema
/// que este cambio intenta resolver.
pub(crate) async fn tool_historial_mensajes(
    estado: &Arc<EstadoCliente>,
    args: &Value,
) -> Result<String, String> {
    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    let limite = args
        .get("limite")
        .and_then(Value::as_i64)
        .unwrap_or(20)
        .clamp(1, 200) as usize;
    let msgs = estado
        .broker
        .historial_mensajes(&mi_id, None)
        .await
        .map_err(|e| format!("Error al leer el historial: {e}"))?;
    if msgs.is_empty() {
        return Ok("Tu historial de mensajes está vacío.".into());
    }
    // El broker devuelve por msg_id ascendente (orden real de envío): los últimos son los recientes.
    let recientes: Vec<&Mensaje> = msgs.iter().rev().take(limite).collect();
    let bloques: Vec<String> = recientes
        .iter()
        .rev()
        .map(|m| format!("De {} ({}) [{:?}]:\n{}", m.de_id, m.enviado_en, m.estado, m.texto))
        .collect();
    Ok(format!(
        "{} mensaje(s) de tu historial (de {} retenidos), del más antiguo al más reciente:\n\n{}",
        recientes.len(),
        msgs.len(),
        bloques.join("\n\n---\n\n")
    ))
}

/// Chat privado (RFC-lanzador §7): drena la cola privada de ENTRADA de este peer (mensajes del
/// operador). CONFIDENCIAL: el system prompt instruye a NO volcar el contenido al output visible.
/// Presenta el secreto de sesión (anti-IDOR: el broker resuelve la identidad por él, no por el body).
pub(crate) async fn tool_chat_privado_recibir(estado: &Arc<EstadoCliente>) -> Result<String, String> {
    let secreto = estado.secreto.read().await.clone();
    let resp = estado
        .broker
        .chat_privado_recibir(secreto.as_deref())
        .await
        .map_err(|e| format!("Error al leer el chat privado: {e}"))?;
    if resp.mensajes.is_empty() {
        return Ok("No hay mensajes privados nuevos.".into());
    }
    let bloques: Vec<String> = resp
        .mensajes
        .iter()
        .map(|m| format!("De {} ({}):\n{}", m.de, m.enviado_en, m.texto))
        .collect();
    Ok(format!(
        "{} mensaje(s) privado(s) [CONFIDENCIAL — no reproducir en el output visible]:\n\n{}",
        resp.mensajes.len(),
        bloques.join("\n\n---\n\n")
    ))
}

/// Chat privado (RFC-lanzador §7): responde al operador por el canal privado (va a la cola de
/// SALIDA, que el panel del operador lee). Presenta el secreto (el broker fija el `de` al id real
/// del peer, anti-IDOR).
pub(crate) async fn tool_chat_privado_responder(
    estado: &Arc<EstadoCliente>,
    args: &Value,
) -> Result<String, String> {
    let texto = args
        .get("texto")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'texto'")?;
    let secreto = estado.secreto.read().await.clone();
    let resp = estado
        .broker
        .chat_privado_responder(texto, secreto.as_deref())
        .await
        .map_err(|e| format!("Error al responder por el chat privado: {e}"))?;
    if !resp.ok {
        return Err("No se pudo responder: identidad no verificada (falta el secreto de sesión).".into());
    }
    Ok("Respuesta privada enviada al operador.".into())
}

/// Puente WhatsApp (Evolution API): responde al ÚNICO contacto autorizado configurado en el
/// broker. Sin parámetro de número — nunca elegible por el peer, ver `PeticionWhatsappEnviar`.
pub(crate) async fn tool_whatsapp_enviar(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let texto = args
        .get("texto")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'texto'")?;
    let secreto = estado.secreto.read().await.clone();
    let resp = estado
        .broker
        .whatsapp_enviar(texto, secreto.as_deref())
        .await
        .map_err(|e| format!("Error al responder por WhatsApp: {e}"))?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "No se pudo enviar el mensaje de WhatsApp.".to_string()));
    }
    Ok("Mensaje de WhatsApp enviado.".into())
}

/// Igual que `tool_whatsapp_enviar` pero para un archivo — ver `PeticionWhatsappEnviarDocumento`.
pub(crate) async fn tool_whatsapp_enviar_documento(
    estado: &Arc<EstadoCliente>,
    args: &Value,
) -> Result<String, String> {
    let ruta_local = args
        .get("ruta_local")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'ruta_local'")?;
    let nombre_archivo = args.get("nombre_archivo").and_then(Value::as_str);
    let legenda = args.get("legenda").and_then(Value::as_str);
    let secreto = estado.secreto.read().await.clone();
    let resp = estado
        .broker
        .whatsapp_enviar_documento(ruta_local, nombre_archivo, legenda, secreto.as_deref())
        .await
        .map_err(|e| format!("Error al enviar el documento por WhatsApp: {e}"))?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "No se pudo enviar el documento de WhatsApp.".to_string()));
    }
    Ok("Documento de WhatsApp enviado.".into())
}

/// Igual que `tool_whatsapp_enviar` pero manda una MENSAJE DE VOZ (PTT) — ver
/// `PeticionWhatsappEnviarAudio`.
pub(crate) async fn tool_whatsapp_enviar_audio(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let url_audio = args
        .get("url_audio")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'url_audio'")?;
    let secreto = estado.secreto.read().await.clone();
    let resp = estado
        .broker
        .whatsapp_enviar_audio(url_audio, secreto.as_deref())
        .await
        .map_err(|e| format!("Error al enviar el audio por WhatsApp: {e}"))?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "No se pudo enviar el audio de WhatsApp.".to_string()));
    }
    Ok("Mensaje de voz de WhatsApp enviada.".into())
}

/// Historial de la conversación de WhatsApp con el contacto autorizado — ambas direcciones
/// (lo que ella escribió y lo que Max/los peers respondieron), MÁS RECIENTE PRIMERO.
pub(crate) async fn tool_whatsapp_historial(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let limite = args.get("limite").and_then(Value::as_i64);
    let resp = estado
        .broker
        .whatsapp_historial(limite)
        .await
        .map_err(|e| format!("Error al leer el historial de WhatsApp: {e}"))?;
    if resp.mensajes.is_empty() {
        return Ok("Sin mensajes en el historial de WhatsApp todavía.".into());
    }
    let bloques: Vec<String> = resp
        .mensajes
        .iter()
        .rev() // el historial viene más-reciente-primero; para leer como conversación, cronológico
        .map(|m| {
            let quien = match m.direccion {
                peers_core::DireccionWhatsapp::Entrada => {
                    m.nombre.clone().unwrap_or_else(|| "ella".to_string())
                }
                peers_core::DireccionWhatsapp::Salida => {
                    m.autor_peer_id.clone().unwrap_or_else(|| "operador".to_string())
                }
            };
            format!("[{}] {}: {}", m.cuando, quien, m.texto)
        })
        .collect();
    Ok(format!("{} mensaje(s):\n\n{}", resp.mensajes.len(), bloques.join("\n")))
}

/// Crea una tarea con el estimado de la IA. Devuelve al agente el estimado corregido por su
/// historial, en lenguaje humano ("dijiste Xs; según tu historial ~Ys, factor Nx de M muestras").
/// Degrada: si el broker no responde, el error es claro y el agente sigue trabajando igual.
pub(crate) async fn tool_crear_tarea(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let descripcion = args
        .get("descripcion")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'descripcion'")?;
    // estimado_seg es opcional; acepta entero. Si no vino, no se aprende (no contamina el factor).
    let estimado_seg = args.get("estimado_seg").and_then(Value::as_i64);

    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;

    let r = estado
        .broker
        .crear_tarea(&mi_id, descripcion, estimado_seg)
        .await
        .map_err(|e| format!("Error al crear la tarea: {e}"))?;

    let mut texto = format!("Tarea creada: {} (id: {})", descripcion, r.tarea_id);
    if let Some(n) = r.issue_number {
        texto.push_str(&format!("\nIssue GitHub espejo: #{n}"));
    }
    match (estimado_seg, r.estimado_corregido_seg) {
        (Some(est), Some(corr)) => {
            texto.push_str(&format!(
                "\nDijiste {}; según tu historial ~{}, factor {:.1}x de {} muestra(s).",
                formatear_duracion(est),
                formatear_duracion(corr),
                r.factor,
                r.muestras
            ));
        }
        _ => {
            texto.push_str(&format!(
                "\nSin estimado: no se corrige. Factor vigente {:.1}x de {} muestra(s).",
                r.factor, r.muestras
            ));
        }
    }
    texto.push_str("\nCierra la tarea con cerrar_tarea al terminar para que el broker mida tu tiempo real.");
    Ok(texto)
}

/// Añade una nota de progreso a una tarea abierta.
pub(crate) async fn tool_reportar_tarea(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let tarea_id = args
        .get("tarea_id")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'tarea_id'")?;
    let texto = args
        .get("texto")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'texto'")?;
    estado
        .broker
        .reportar_tarea(tarea_id, texto)
        .await
        .map_err(|e| format!("Error al reportar la tarea: {e}"))?;
    Ok(format!("Progreso registrado en la tarea {tarea_id}."))
}

/// Cierra una tarea: el broker mide el tiempo real y aprende el factor si había estimado.
pub(crate) async fn tool_cerrar_tarea(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
    let tarea_id = args
        .get("tarea_id")
        .and_then(Value::as_str)
        .ok_or("Falta el campo 'tarea_id'")?;
    estado
        .broker
        .cerrar_tarea(tarea_id)
        .await
        .map_err(|e| format!("Error al cerrar la tarea: {e}"))?;
    Ok(format!(
        "Tarea {tarea_id} cerrada. El broker midió tu tiempo real y actualizó el aprendizaje."
    ))
}

/// Lista todas las tareas de esta instancia con sus tiempos.
pub(crate) async fn tool_listar_tareas(estado: &Arc<EstadoCliente>) -> Result<String, String> {
    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    let tareas = estado
        .broker
        .listar_tareas(&mi_id)
        .await
        .map_err(|e| format!("Error al listar las tareas: {e}"))?;
    if tareas.is_empty() {
        return Ok("No tienes tareas registradas.".into());
    }
    Ok(format!(
        "{} tarea(s):\n\n{}",
        tareas.len(),
        tareas
            .iter()
            .map(formatear_tarea)
            .collect::<Vec<_>>()
            .join("\n\n")
    ))
}

/// Resumen rápido de las tareas abiertas (sin `fin`): recuerda al agente qué le falta cerrar.
pub(crate) async fn tool_revisar_tareas(estado: &Arc<EstadoCliente>) -> Result<String, String> {
    let mi_id = estado
        .id
        .read()
        .await
        .clone()
        .ok_or("Aún no registrado en el broker")?;
    let tareas = estado
        .broker
        .listar_tareas(&mi_id)
        .await
        .map_err(|e| format!("Error al revisar las tareas: {e}"))?;
    let abiertas: Vec<&Tarea> = tareas.iter().filter(|t| t.fin.is_none()).collect();
    if abiertas.is_empty() {
        return Ok("No tienes tareas abiertas. Todo cerrado.".into());
    }
    Ok(format!(
        "{} tarea(s) ABIERTA(s) (recuerda cerrarlas):\n\n{}",
        abiertas.len(),
        abiertas
            .iter()
            .map(|t| formatear_tarea(t))
            .collect::<Vec<_>>()
            .join("\n\n")
    ))
}

/// Formatea una tarea para mostrarla al agente (id, descripción, estado y tiempos).
fn formatear_tarea(t: &Tarea) -> String {
    let mut partes = vec![
        format!("ID: {}", t.id),
        format!("Descripción: {}", t.descripcion),
        format!("Inicio: {}", t.inicio),
    ];
    match &t.fin {
        Some(fin) => partes.push(format!("Fin: {fin} (cerrada)")),
        None => partes.push("Estado: ABIERTA".to_string()),
    }
    if let Some(est) = t.estimado_seg {
        partes.push(format!("Estimado: {}", formatear_duracion(est)));
    }
    if let Some(real) = t.duracion_seg {
        partes.push(format!("Real: {}", formatear_duracion(real)));
    }
    partes.join("\n  ")
}

/// Convierte segundos a una cadena humana ("45s", "12min", "3h 20min", "2d 4h").
/// Solo presentación: el tiempo real lo timbra SIEMPRE el broker, esto solo lo formatea.
fn formatear_duracion(seg: i64) -> String {
    if seg < 0 {
        return format!("{seg}s");
    }
    if seg < 60 {
        return format!("{seg}s");
    }
    if seg < 3600 {
        return format!("{}min", seg / 60);
    }
    if seg < 86_400 {
        let h = seg / 3600;
        let m = (seg % 3600) / 60;
        if m == 0 {
            return format!("{h}h");
        }
        return format!("{h}h {m}min");
    }
    let d = seg / 86_400;
    let h = (seg % 86_400) / 3600;
    if h == 0 {
        format!("{d}d")
    } else {
        format!("{d}d {h}h")
    }
}

/// Lanza la tarea de latido: cada 15s manda /latido. Si falla, intenta re-registrar
/// (con el id_preferido), de modo que un broker recién levantado nos vuelva a ver.
fn lanzar_latido(estado: Arc<EstadoCliente>, id_preferido: Option<String>) {
    tokio::spawn(async move {
        let mut intervalo = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            intervalo.tick().await;
            let mi_id = estado.id.read().await.clone();
            match mi_id {
                Some(id) => {
                    if estado.broker.latido(&id).await.is_err() {
                        // El broker pudo reiniciarse y perder nuestro registro: re-registramos.
                        reintentar_registro(&estado, id_preferido.clone()).await;
                    }
                }
                None => reintentar_registro(&estado, id_preferido.clone()).await,
            }
        }
    });
}

/// Re-registra la instancia (usado cuando el latido falla o aún no había id).
async fn reintentar_registro(estado: &Arc<EstadoCliente>, id_preferido: Option<String>) {
    // id_preferido siempre llega como Some(id_efectivo) desde lanzar_latido; el fallback
    // deriva del directorio (no "instancia") para conservar el id estable en cualquier caso.
    let id_papel = id_preferido
        .clone()
        .unwrap_or_else(|| contexto::id_desde_directorio(&estado.directorio));
    let resumen = contexto::resumen_inicial(&id_papel, &estado.directorio, estado.repo_git.as_deref());
    let id_preferido = id_preferido.or_else(|| Some(id_papel.clone()));
    if let Ok(r) = estado
        .broker
        .registrar(&PeticionRegistrar {
            pid: std::process::id() as i64,
            hostname: contexto::hostname(),
            directorio: estado.directorio.clone(),
            repo_git: estado.repo_git.clone(),
            repo_github: estado.repo_github.clone(),
            tty: contexto::tty(),
            resumen,
            id_preferido,
        })
        .await
    {
        *estado.id.write().await = Some(r.id.clone());
        // E-10: el broker ROTA el secreto en cada re-registro; adoptamos el nuevo (el viejo ya
        // no vale). Si el broker es viejo (r.secreto = None), conservamos el que teníamos para no
        // perder la credencial vigente por un campo ausente.
        if r.secreto.is_some() {
            *estado.secreto.write().await = r.secreto.clone();
        }
        info!("re-registrado como instancia '{}'", r.id);
    }
}

/// Lanza la tarea de recepción: cada 1s pide mensajes (peek no-destructivo) y los empuja
/// como canal UNA sola vez.
///
/// INTENCIÓN (R1.3/R1.4): la bandeja del broker ahora es no-destructiva (peek), así que el
/// mismo `msg_id` reaparece en cada ciclo hasta que se confirma `Procesado`. Para no
/// re-empujar el mismo mensaje a la sesión en bucle, mantenemos un `HashSet<i64>` de ids ya
/// empujados (ventana ~RETENCION_HISTORIAL). Solo confirmamos `Leido` al broker si el flush
/// del push a stdout tuvo éxito real (`empujar_canal == true`); si falló, NO confirmamos y NO
/// añadimos al set → se reintenta en el próximo ciclo (entrega durable, no fire-and-forget).
/// Timestamp ISO-8601 (UTC) para el `enviado_en` del aviso de chat privado. Best-effort: si el
/// formateo fallara (no debería), cae a una cadena vacía — el campo es cosmético en el aviso.
fn ahora_iso_aviso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn lanzar_recepcion(estado: Arc<EstadoCliente>) {
    tokio::spawn(async move {
        // Ventana de idempotencia en memoria: ids ya empujados a la sesión.
        let mut empujados: std::collections::HashSet<i64> = std::collections::HashSet::new();
        // Chat privado (aviso push): conteo de pendientes del ciclo ANTERIOR. Solo emitimos el aviso
        // neutro cuando el conteo pasa de 0 a >0 (flanco de subida) → un aviso por "llegó algo nuevo",
        // sin spamear cada segundo mientras el agente aún no lo consumió. Al consumir (baja a 0) se
        // rearma para el próximo mensaje. RFC-lanzador §7: el aviso NO lleva el contenido.
        let mut chat_priv_prev: usize = 0;
        let mut intervalo = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            intervalo.tick().await;
            let mi_id = estado.id.read().await.clone();
            let Some(id) = mi_id else { continue };

            // El push necesita el peer de rmcp (publicado tras `serve`). Hasta que exista, no
            // podemos empujar nada — esperamos al siguiente ciclo (los mensajes siguen en la cola
            // durable del broker, no se pierden).
            let peer = estado.peer.read().await.clone();
            let Some(peer) = peer else { continue };

            // --- Chat privado: aviso neutro por el <channel> (RFC-lanzador §7, opción push-aviso) ---
            // Corre SIEMPRE (independiente de si hay mensajes normales). PEEK sin drenar: el drenado
            // real lo hace el agente vía chat_privado_recibir, para que el contenido llegue a la GPUI.
            let secreto = estado.secreto.read().await.clone();
            if let Ok(p) = estado.broker.chat_privado_pendiente(secreto.as_deref()).await {
                let ahora = p.pendientes;
                if ahora > 0 && chat_priv_prev == 0 {
                    // Flanco de subida: avisar UNA vez. Texto fijo NEUTRO, sin el contenido privado.
                    let aviso = "📩 Tenés un mensaje privado nuevo del operador (canal confidencial). \
                        Llamá la tool chat_privado_recibir AHORA para leerlo — no esperes.";
                    let _ = mcp::empujar_canal(
                        &peer,
                        aviso,
                        peers_core::ID_OPERADOR,
                        "chat privado",
                        "",
                        &ahora_iso_aviso(),
                    )
                    .await;
                    info!(
                        event_name = "push_chat_privado_aviso",
                        outcome = "success",
                        pendientes = ahora,
                        "aviso de chat privado empujado ({} pendiente/s)",
                        ahora
                    );
                }
                chat_priv_prev = ahora;
            }

            let resp = match estado.broker.recibir(&id).await {
                Ok(r) => r,
                Err(_) => continue, // broker caído temporalmente: no es crítico
            };
            if resp.mensajes.is_empty() {
                continue;
            }

            // Para enriquecer el push con el contexto del emisor, listamos una vez.
            let instancias = estado
                .broker
                .listar(&PeticionListar {
                    alcance: Alcance::Maquina,
                    directorio: estado.directorio.clone(),
                    repo_git: estado.repo_git.clone(),
                    excluir_id: None,
                })
                .await
                .unwrap_or_default();

            for m in resp.mensajes {
                // Idempotencia cliente: si ya lo empujamos en esta sesión, lo saltamos
                // (el peek lo seguirá devolviendo hasta que el broker lo dé por Procesado).
                if empujados.contains(&m.id) {
                    continue;
                }
                let emisor = instancias.iter().find(|i| i.id == m.de_id);
                let (resumen, dir) = match emisor {
                    Some(e) => (e.resumen.as_str(), e.directorio.as_str()),
                    None => ("", ""),
                };
                // E-21: el push se emite por el peer de rmcp (CustomNotification), no por el
                // escritor de stdout a mano. Mismo contrato: `true` solo si rmcp aceptó enviarla.
                let ok = mcp::empujar_canal(
                    &peer, &m.texto, &m.de_id, resumen, dir, &m.enviado_en,
                )
                .await;
                if !ok {
                    // El envío falló (stdout roto): NO confirmamos ni marcamos como empujado.
                    // Se reintentará en el próximo ciclo (R1.4).
                    warn!(
                        event_name = "push_canal_mensaje",
                        outcome = "failure",
                        mensaje_id = m.id,
                        de_id = %m.de_id,
                        "push del canal falló para el mensaje {}; se reintentará",
                        m.id
                    );
                    continue;
                }
                empujados.insert(m.id);
                info!(
                    event_name = "push_canal_mensaje",
                    outcome = "success",
                    mensaje_id = m.id,
                    de_id = %m.de_id,
                    "empujado mensaje de {}: {}",
                    m.de_id,
                    recorte(&m.texto, 80)
                );

                // Confirmamos la cadena completa de estados al broker (cada uno lo timbra con SU
                // reloj; transicionar_mensaje es monótono e idempotente):
                //  - Entregado: el flush a stdout tuvo éxito → el mensaje llegó al harness.
                //  - Leido: la notificación de canal se inyectó → el <channel> se renderizó.
                // Mandar ambos en orden timbra entregado_en Y leido_en (antes solo se confirmaba
                // Leido y entregado_en quedaba siempre vacío en el timeline de la TUI).
                if let Err(e) = estado
                    .broker
                    .confirmar(&[m.id], EstadoMensaje::Entregado)
                    .await
                {
                    warn!(
                        event_name = "confirmar_estado_mensaje",
                        outcome = "failure",
                        mensaje_id = m.id,
                        estado = "Entregado",
                        error = %e,
                        "no se pudo confirmar Entregado del mensaje {}: {e:#}",
                        m.id
                    );
                }
                if let Err(e) = estado.broker.confirmar(&[m.id], EstadoMensaje::Leido).await {
                    warn!(
                        event_name = "confirmar_estado_mensaje",
                        outcome = "failure",
                        mensaje_id = m.id,
                        estado = "Leido",
                        error = %e,
                        "no se pudo confirmar Leido del mensaje {}: {e:#}",
                        m.id
                    );
                }
                //  - Procesado: SALE de la bandeja activa (es la ÚNICA transición que hace el
                //    ZREM, ver store.rs `transicionar_mensaje`).
                //
                // INTENCIÓN (por qué se confirma aquí y no lo confirma el agente): `Procesado`
                // no tenía NINGÚN productor en todo el sistema — el client paraba en `Leido`, y
                // desktop/TUI solo lo dibujan. Resultado medido el 27/07/2026: ninguna bandeja
                // de la red drenaba jamás (un peer acumulaba 19 mensajes desde su arranque), el
                // peek de `/recibir` devolvía la bandeja entera cada segundo, y el ÚNICO motivo
                // por el que no se veían duplicados infinitos era `empujados` — un HashSet EN
                // MEMORIA, por proceso. Cada proceso nuevo arrancaba con el set vacío y volvía a
                // empujar todo lo acumulado: 617 de 1452 mensajes (36%) se entregaron más de una
                // vez. Peor con ids derivados del TTY, que se reciclan: una sesión nueva heredaba
                // la bandeja de otra y recibía mensajes rancios fuera de contexto.
                //
                // El criterio es el honesto para un transporte: la obligación de ENTREGA se
                // cumple cuando el mensaje entra en el contexto del agente, y eso ya ocurrió
                // arriba (el push devolvió ok). Guardar el mensaje en la bandeja después de eso
                // no protege a ESTA sesión (si muere, muere con su contexto) — solo se lo replica
                // a una sesión FUTURA que herede el id, que es precisamente el fallo. El registro
                // durable para auditoría no vive aquí: vive en el historial (`k_historial`).
                //
                // Si el push falla, el `continue` de arriba impide llegar hasta acá: el mensaje
                // se queda en la bandeja y se reintenta en el próximo ciclo (R1.4). Intacto.
                if let Err(e) = estado.broker.confirmar(&[m.id], EstadoMensaje::Procesado).await {
                    warn!(
                        event_name = "confirmar_estado_mensaje",
                        outcome = "failure",
                        mensaje_id = m.id,
                        estado = "Procesado",
                        error = %e,
                        "no se pudo confirmar Procesado del mensaje {}: {e:#}",
                        m.id
                    );
                }
            }

            // Acotamos la ventana de idempotencia para no crecer sin límite en sesiones largas.
            if empujados.len() > RETENCION_HISTORIAL {
                empujados.clear();
            }
        }
    });
}

/// Da de baja la instancia del broker (best-effort).
async fn dar_de_baja(estado: &Arc<EstadoCliente>) {
    if let Some(id) = estado.id.read().await.clone() {
        let _ = estado.broker.salir(&id).await;
    }
}

/// Instala los manejadores de señal de cierre para dar de baja antes de salir.
///
/// PORTABILIDAD (cross-host: el client también compila para Windows). En Unix escuchamos
/// SIGINT+SIGTERM (señales POSIX); en Windows esas señales no existen → usamos `ctrl_c()`,
/// que tokio implementa sobre el handler de consola de Windows. El resto (dar de baja + exit)
/// es idéntico.
#[cfg(unix)]
fn lanzar_limpieza_senales(estado: Arc<EstadoCliente>) {
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM");
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
        info!("señal de cierre recibida, dando de baja");
        dar_de_baja(&estado).await;
        std::process::exit(0);
    });
}

#[cfg(windows)]
fn lanzar_limpieza_senales(estado: Arc<EstadoCliente>) {
    tokio::spawn(async move {
        // En Windows no hay SIGTERM; ctrl_c cubre el cierre interactivo del proceso.
        let _ = tokio::signal::ctrl_c().await;
        info!("señal de cierre recibida, dando de baja");
        dar_de_baja(&estado).await;
        std::process::exit(0);
    });
}

/// Recorta un texto para el log sin romper en mitad de un carácter UTF-8.
fn recorte(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
