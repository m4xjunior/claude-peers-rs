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
use mcp::SalidaMcp;
use peers_core::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
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
}

/// Estado compartido de la instancia: su id asignado por el broker.
struct EstadoCliente {
    id: RwLock<Option<String>>,
    broker: ClienteBroker,
    directorio: String,
    repo_git: Option<String>,
    /// "owner/repo" del remote origin (dinámico). El broker lo usa para abrir issues
    /// en el repo donde este peer trabaja. None si el dir no es repo GitHub → sin issue.
    repo_github: Option<String>,
    salida: SalidaMcp,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs SIEMPRE a stderr: stdout está reservado al protocolo MCP.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let url = args
        .broker_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", args.puerto));

    let directorio = contexto::directorio_actual();
    let repo_git = contexto::repo_git(&directorio);
    // Resuelve "owner/repo" del remote origin AQUÍ (en la máquina del peer, con gh logado).
    // El broker no lo resuelve: recibe el valor y abre la issue en ESE repo (dinámico).
    let repo_github = contexto::repo_github(&directorio);
    let tty = contexto::tty();

    info!("directorio: {directorio}");
    info!("repo git: {}", repo_git.as_deref().unwrap_or("(ninguno)"));
    info!("repo github: {}", repo_github.as_deref().unwrap_or("(ninguno)"));
    info!("broker: {url}");

    let broker = ClienteBroker::nuevo(url.clone());
    if !broker.esta_vivo().await {
        // No abortamos: registramos el aviso y seguimos; el broker puede levantarse luego.
        warn!("el broker no responde en {url} — reintentaré en el próximo latido");
    }

    // Resumen inicial local (sin API externa). Si hay --id lo usa como papel.
    let id_papel = args.id.clone().unwrap_or_else(|| "instancia".into());
    let resumen = contexto::resumen_inicial(&id_papel, &directorio, repo_git.as_deref());

    // Registro (con id_preferido si vino --id → herencia de fila en restart).
    let id_asignado = match broker
        .registrar(&PeticionRegistrar {
            pid: std::process::id() as i64,
            directorio: directorio.clone(),
            repo_git: repo_git.clone(),
            repo_github: repo_github.clone(),
            tty,
            resumen,
            id_preferido: args.id.clone(),
        })
        .await
    {
        Ok(r) => {
            info!("registrado como instancia '{}'", r.id);
            Some(r.id)
        }
        Err(e) => {
            warn!("no se pudo registrar ahora ({e:#}); seguiré e intentaré por latido");
            None
        }
    };

    let estado = Arc::new(EstadoCliente {
        id: RwLock::new(id_asignado),
        broker,
        directorio,
        repo_git,
        repo_github,
        salida: SalidaMcp::nueva(),
    });

    // Loop de latido (cada 15s) — mantiene viva la instancia y re-registra si hizo falta.
    lanzar_latido(estado.clone(), args.id.clone());

    // Loop de recepción (cada 1s) — empuja mensajes entrantes a la sesión como canal.
    lanzar_recepcion(estado.clone());

    // Limpieza al recibir SIGINT/SIGTERM: damos de baja la instancia del broker.
    lanzar_limpieza_senales(estado.clone());

    // Bucle principal: lee mensajes MCP de stdin línea a línea y los despacha.
    bucle_stdin(estado).await;
    Ok(())
}

/// Lee stdin línea por línea (framing del transporte stdio) y despacha cada mensaje.
async fn bucle_stdin(estado: Arc<EstadoCliente>) {
    let mut lineas = BufReader::new(tokio::io::stdin()).lines();
    loop {
        match lineas.next_line().await {
            Ok(Some(linea)) => {
                let linea = linea.trim();
                if linea.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(linea) {
                    Ok(msg) => despachar(&estado, msg).await,
                    Err(e) => error!("mensaje MCP no es JSON válido: {e}"),
                }
            }
            Ok(None) => {
                // stdin cerrado: el Claude padre terminó. Salimos limpiamente.
                info!("stdin cerrado, finalizando");
                dar_de_baja(&estado).await;
                break;
            }
            Err(e) => {
                error!("error leyendo stdin: {e}");
                break;
            }
        }
    }
}

/// Despacha un mensaje JSON-RPC entrante. Solo responde a peticiones con `id`
/// (las notificaciones no llevan respuesta).
async fn despachar(estado: &Arc<EstadoCliente>, msg: Value) {
    let metodo = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned();

    match metodo {
        "initialize" => {
            let version_cliente = msg
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str);
            responder(estado, id, mcp::resultado_initialize(version_cliente)).await;
        }
        "notifications/initialized" => {
            // Notificación del cliente: nada que responder.
        }
        "tools/list" => {
            responder(estado, id, json!({ "tools": mcp::definiciones_tools() })).await;
        }
        "tools/call" => {
            let resultado = ejecutar_tool(estado, msg.get("params")).await;
            responder(estado, id, resultado).await;
        }
        "ping" => {
            responder(estado, id, json!({})).await;
        }
        otro => {
            // Método no soportado: si era una petición, devolvemos error JSON-RPC estándar.
            if id.is_some() {
                responder_error(estado, id, -32601, &format!("método no soportado: {otro}")).await;
            }
        }
    }
}

/// Ejecuta una tool y devuelve el `result` MCP (content con texto).
async fn ejecutar_tool(estado: &Arc<EstadoCliente>, params: Option<&Value>) -> Value {
    let nombre = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    let texto = match nombre {
        "listar_instancias" => tool_listar(estado, &args).await,
        "enviar_mensaje" => tool_enviar(estado, &args).await,
        "definir_resumen" => tool_definir_resumen(estado, &args).await,
        "revisar_mensajes" => tool_revisar(estado).await,
        otro => Err(format!("tool desconocida: {otro}")),
    };

    match texto {
        Ok(t) => json!({ "content": [{ "type": "text", "text": t }] }),
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
    }
}

async fn tool_listar(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
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

async fn tool_enviar(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
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

    let resp = estado
        .broker
        .enviar(&PeticionEnviar {
            de_id: mi_id,
            para_id: para_id.to_string(),
            texto: mensaje.to_string(),
        })
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

async fn tool_definir_resumen(estado: &Arc<EstadoCliente>, args: &Value) -> Result<String, String> {
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

async fn tool_revisar(estado: &Arc<EstadoCliente>) -> Result<String, String> {
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
    Ok(format!(
        "{} mensaje(s) nuevo(s):\n\n{}",
        resp.mensajes.len(),
        bloques.join("\n\n---\n\n")
    ))
}

/// Envía una respuesta JSON-RPC exitosa a una petición con `id`.
async fn responder(estado: &Arc<EstadoCliente>, id: Option<Value>, resultado: Value) {
    let Some(id) = id else { return };
    estado
        .salida
        .enviar_json(&json!({ "jsonrpc": "2.0", "id": id, "result": resultado }))
        .await;
}

/// Envía una respuesta de error JSON-RPC.
async fn responder_error(estado: &Arc<EstadoCliente>, id: Option<Value>, codigo: i64, mensaje: &str) {
    let Some(id) = id else { return };
    estado
        .salida
        .enviar_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": codigo, "message": mensaje }
        }))
        .await;
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
    let id_papel = id_preferido.clone().unwrap_or_else(|| "instancia".into());
    let resumen = contexto::resumen_inicial(&id_papel, &estado.directorio, estado.repo_git.as_deref());
    if let Ok(r) = estado
        .broker
        .registrar(&PeticionRegistrar {
            pid: std::process::id() as i64,
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
        info!("re-registrado como instancia '{}'", r.id);
    }
}

/// Lanza la tarea de recepción: cada 1s pide mensajes y los empuja como canal.
fn lanzar_recepcion(estado: Arc<EstadoCliente>) {
    tokio::spawn(async move {
        let mut intervalo = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            intervalo.tick().await;
            let mi_id = estado.id.read().await.clone();
            let Some(id) = mi_id else { continue };

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
                let emisor = instancias.iter().find(|i| i.id == m.de_id);
                let (resumen, dir) = match emisor {
                    Some(e) => (e.resumen.as_str(), e.directorio.as_str()),
                    None => ("", ""),
                };
                estado
                    .salida
                    .empujar_canal(&m.texto, &m.de_id, resumen, dir, &m.enviado_en)
                    .await;
                info!("empujado mensaje de {}: {}", m.de_id, recorte(&m.texto, 80));
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

/// Instala los manejadores de SIGINT/SIGTERM para dar de baja antes de salir.
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

/// Recorta un texto para el log sin romper en mitad de un carácter UTF-8.
fn recorte(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
