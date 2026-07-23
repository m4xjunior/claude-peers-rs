//! Puente WhatsApp (Evolution API) — pedido directo de Max: "quiero las respuestas de ella
//! como push, integrado en mi app, y si hace falta actualizamos el binario y el broker, para
//! contener la Evo y mi instancia y las credenciales para que esto funcione".
//!
//! DISEÑO EVENT-DRIVEN (webhook `MESSAGES_UPSERT`), NO polling: el `findMessages` de Evolution
//! no filtra `fromMe` en el `where` de esta versión (bug confirmado por el equipo LexusFX vía
//! context7) — un poll siempre traía de vuelta los propios mensajes salientes, indistinguibles
//! de los de ella. El webhook sí trae `key.fromMe` correcto por evento; el filtro se hace en
//! código, nunca confiando en el `where` de la API.
//!
//! CONTENCIÓN DE CREDENCIALES (pedido explícito de Max): la URL/API-key/instancia de Evolution
//! y el número de destino viven SOLO en el entorno del broker — nunca en el repo, nunca en el
//! payload de un peer. El número es FIJO y de UN SOLO destino autorizado: ningún peer puede
//! parametrizar "a quién" se le escribe, la clase de bug ("número equivocado" a una persona
//! real) que más preocupó al equipo durante la coordinación de esta feature.

use anyhow::Context as _;
use serde_json::Value;

/// Id del peer SINTÉTICO que representa el contacto autorizado en el roster (`listar_instancias`,
/// pantalla Peers del desktop). Es un peer REAL a ojos del broker (liveness sintético, registrado
/// al arrancar) pero NO es una sesión Claude Code — es el canal WhatsApp/Evolution. Constante fija
/// (no configurable): un solo contacto por diseño, ver nota de módulo.
pub const PEER_ID: &str = "leticia";

/// Resumen que se le fija al registrar el peer sintético — deja claro en la UI que no es una
/// sesión Claude Code, para que nadie se confunda pensando que puede "matarla" como a un proceso.
pub const PEER_RESUMEN: &str =
    "Contato WhatsApp (Evolution API) — não é uma sessão Claude Code, é o canal dela.";

/// Config de la integración, leída UNA vez al arrancar. Cualquier variable ausente/vacía →
/// `desde_entorno` devuelve `None` y el broker opera SIN el puente (R9: degradación graciosa,
/// jamás bloquea el arranque — igual que `GitHub::desde_entorno`).
#[derive(Debug, Clone)]
pub struct Config {
    /// Base de la API de Evolution, ej. `https://whatsapp.lexusfx.com` (o la LAN/túnel vigente).
    pub url: String,
    pub api_key: String,
    /// Nombre de la instancia de Evolution, ej. `lexusfx_604`.
    pub instancia: String,
    /// Destino FIJO autorizado (ej. `34604458061`), SIN el sufijo `@s.whatsapp.net`.
    pub numero: String,
    /// Id opaco "LID" del MISMO contacto (SIN el sufijo `@lid`, mismo criterio que `numero` sin
    /// `@s.whatsapp.net`) — privacidad de WhatsApp: un contacto puede aparecer como
    /// `@s.whatsapp.net` (número) O `@lid` (id opaco) según cómo negocie el hilo, comprobado en
    /// vivo (`remoteJidAlt` en el payload de Evolution confirma que ambos JIDs son la MISMA
    /// persona). Opcional: sin `EVOLUTION_LID_DESTINO`, solo se acepta el JID por número.
    pub lid: Option<String>,
    /// Clave compartida que el webhook debe presentar en `?clave=`. Es el ÚNICO guardián de
    /// `/evo/inbound`: ese endpoint no puede exigir el `X-Peers-Token` normal porque quien lo
    /// llama es Evolution (o un fan-out externo), no un peer registrado con secreto de sesión.
    pub webhook_clave: String,
}

impl Config {
    pub fn desde_entorno() -> Option<Self> {
        let url = env_no_vacio("EVOLUTION_URL")?;
        let api_key = env_no_vacio("EVOLUTION_API_KEY")?;
        let instancia = env_no_vacio("EVOLUTION_INSTANCIA")?;
        let numero = env_no_vacio("EVOLUTION_NUMERO_DESTINO")?;
        let webhook_clave = env_no_vacio("EVOLUTION_WEBHOOK_CLAVE")?;
        let lid = env_no_vacio("EVOLUTION_LID_DESTINO");
        Some(Self { url, api_key, instancia, numero, lid, webhook_clave })
    }

    fn remoto_esperado(&self) -> String {
        format!("{}@s.whatsapp.net", self.numero)
    }

    fn lid_esperado(&self) -> Option<String> {
        self.lid.as_deref().map(|l| format!("{l}@lid"))
    }
}

fn env_no_vacio(clave: &str) -> Option<String> {
    match std::env::var(clave) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Invoca `curl` como subproceso en vez de usar `reqwest` directamente: en esta máquina se
/// confirmó (con reqwest, `tokio::net::TcpStream` y `std::net::TcpStream`, los tres, de forma
/// determinística) un "No route to host" (EHOSTUNREACH) contra la VM de Evolution que SOLO
/// afecta a binarios Rust compilados — `curl`/`nc` invocados en el mismo instante, intercalados,
/// SIEMPRE tuvieron éxito (5/5 en pruebas). Causa raíz no resuelta (posible diferencia de
/// nivel de kernel/TCC entre binarios ad-hoc-signed y el `curl` firmado por Apple, contra la ruta
/// interface-scoped de bridge100/Parallels) — mientras no se resuelva a nivel de SO, delegar el
/// connect a `curl` es lo único que funciona de verdad en esta máquina.
async fn curl_post_json_intento(cfg: &Config, url: &str, cuerpo: &Value) -> anyhow::Result<(u16, String)> {
    let salida = tokio::process::Command::new("curl")
        .arg("-sS")
        .arg("-m")
        .arg("10")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg(format!("apikey: {}", cfg.api_key))
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-d")
        .arg(cuerpo.to_string())
        .arg("-w")
        .arg("\n%{http_code}")
        .arg(url)
        .output()
        .await
        .context("ejecutando curl hacia Evolution")?;
    if !salida.status.success() {
        anyhow::bail!("curl salió con {}: {}", salida.status, String::from_utf8_lossy(&salida.stderr));
    }
    let texto = String::from_utf8_lossy(&salida.stdout).into_owned();
    let (cuerpo_resp, codigo) =
        texto.rsplit_once('\n').context("salida de curl sin el código HTTP esperado")?;
    let codigo: u16 = codigo.trim().parse().context("código HTTP de curl no numérico")?;
    Ok((codigo, cuerpo_resp.to_string()))
}

/// Reintenta `curl_post_json_intento` hasta 3 veces (backoff 1s/2s) ante fallos TRANSITORIOS:
/// el `curl` en sí fallando (blip de red — confirmado en vivo: la VM de Evolution tiene cortes
/// de segundos, aprox. una vez por hora, que se autorresuelven) o un 5xx de Evolution (ej.
/// "Connection Closed" cuando la sesión de WhatsApp/Baileys tiene un hipo — visto en vivo el
/// 2026-07-23, dos envíos reales fallaron así sin reintento). Un 4xx (ej. remitente inválido) NO
/// se reintenta: es un error real, no algo que un segundo intento vaya a arreglar.
async fn curl_post_json(cfg: &Config, url: &str, cuerpo: &Value) -> anyhow::Result<(u16, String)> {
    let mut ultimo_error = None;
    for intento in 0..3 {
        if intento > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(intento)).await;
        }
        match curl_post_json_intento(cfg, url, cuerpo).await {
            Ok((codigo, cuerpo_resp)) if codigo >= 500 => {
                ultimo_error = Some(anyhow::anyhow!("Evolution respondió {codigo}: {cuerpo_resp}"));
            }
            Ok(exito) => return Ok(exito),
            Err(err) => ultimo_error = Some(err),
        }
    }
    Err(ultimo_error.unwrap_or_else(|| anyhow::anyhow!("curl falló sin detalle")))
}

/// Envía un texto al destino FIJO configurado (ver nota de módulo: nunca parametrizable).
/// `_cliente` ya no se usa (ver `curl_post_json`); se mantiene el parámetro para no tocar las
/// firmas en los call-sites de `main.rs`.
pub async fn enviar_texto(_cliente: &reqwest::Client, cfg: &Config, texto: &str) -> anyhow::Result<()> {
    let url = format!("{}/message/sendText/{}", cfg.url.trim_end_matches('/'), cfg.instancia);
    let cuerpo = serde_json::json!({ "number": cfg.numero, "text": texto });
    let (codigo, cuerpo_resp) =
        curl_post_json(cfg, &url, &cuerpo).await.context("llamando a Evolution sendText")?;
    if !(200..300).contains(&codigo) {
        anyhow::bail!("Evolution respondió {codigo}: {cuerpo_resp}");
    }
    Ok(())
}

/// Un evento ya filtrado y normalizado: lo único que el broker necesita para decidir si lo
/// reenvía a los peers y cómo lo persiste en el historial.
pub struct EventoEntrante {
    /// Vacío cuando `adjunto` es `Some` sin resolver todavía — `resolver_adjunto` lo completa
    /// (main.rs, `procesar_entrante_whatsapp`) ANTES de persistir/empujar, nunca queda vacío
    /// en lo que llega a los peers.
    pub texto: String,
    pub nombre: Option<String>,
    pub evo_message_id: String,
    /// `true` si el mensaje lo escribió Max (no un eco automático nuestro — ver nota en
    /// `extraer_entrante`): en el hilo LID ambas direcciones son diálogo humano real, así que
    /// las dos entran al historial. `false` = ella escribió.
    pub from_me: bool,
    /// `Some` cuando el mensaje es imagen/documento/audio/video en vez de texto plano — pedido
    /// directo de Max ("o time n ta recebendo os html dela nem arquivo"; "qnd ela mandar audio
    /// vcs tem q transcrever"). Ver `resolver_adjunto`.
    pub adjunto: Option<Adjunto>,
}

/// Adjunto detectado en un mensaje entrante: lo mínimo para pedirle a Evolution el contenido
/// real vía `getBase64FromMediaMessage` — esa llamada exige la `key` ORIGINAL del mensaje (no
/// solo el id), por eso se guarda completa en vez de reconstruirla después.
pub struct Adjunto {
    pub tipo: &'static str,
    pub file_name: Option<String>,
    pub mimetype: Option<String>,
    pub caption: Option<String>,
    pub key: Value,
}

/// Extrae el evento entrante del payload del webhook, si corresponde. Tolerante a DOS formas
/// (el payload puede llegar directo de Evolution → `{key, pushName, message, ...}` en la raíz,
/// o reenvuelto por un fan-out/relay externo → anidado en `.data`): prueba la raíz primero, y
/// si falta `key` cae a `.data`.
///
/// Devuelve `None` (nunca un error) cuando:
/// - el remitente no es el contacto autorizado (otros contactos de la misma instancia — PF-1);
/// - es eco de un automatismo nuestro en el canal de negocio puro (`fromMe:true` en el JID por
///   número — ahí `fromMe` SÍ es un recordatorio automático, no diálogo real);
/// - el payload no trae texto reconocible (evento que no es de mensaje, adjunto sin caption…).
///
/// En el JID "LID" (pedido de Max: es donde ella participa de verdad, escribiendo natural con
/// él) SÍ se acepta `fromMe:true` — ahí ambas direcciones son diálogo humano, no eco de bot.
///
/// Un payload no reconocido es "nada que reenviar", nunca un 500: el webhook de un tercero no
/// puede tumbar el broker por mandar una forma inesperada (R9).
pub fn extraer_entrante(cfg: &Config, payload: &Value) -> Option<EventoEntrante> {
    let objeto = if payload.get("key").is_some() { payload } else { payload.get("data")? };
    let remote_jid = objeto.pointer("/key/remoteJid")?.as_str()?;
    let es_lid = cfg.lid_esperado().as_deref() == Some(remote_jid);
    if !es_lid && remote_jid != cfg.remoto_esperado() {
        return None; // PF-1: remitente no autorizado, se descarta antes de tocar nada más
    }
    let from_me = objeto.pointer("/key/fromMe")?.as_bool()?;
    if from_me && !es_lid {
        return None; // eco de un recordatorio automático nuestro en el canal de negocio puro
    }
    let evo_message_id = objeto.pointer("/key/id")?.as_str()?.to_string();
    let nombre = objeto.get("pushName").and_then(Value::as_str).map(str::to_string);
    let mensaje = objeto.get("message")?;

    if let Some(texto) = mensaje
        .get("conversation")
        .and_then(Value::as_str)
        .or_else(|| mensaje.pointer("/extendedTextMessage/text").and_then(Value::as_str))
    {
        return Some(EventoEntrante {
            texto: texto.to_string(),
            nombre,
            evo_message_id,
            from_me,
            adjunto: None,
        });
    }

    // Sin texto plano: ¿imagen/documento/audio/video? El contenido real se resuelve APARTE
    // (`resolver_adjunto`, en main.rs) porque exige otra llamada a Evolution — aquí solo se
    // detecta y se guarda lo necesario (la `key` completa) para pedirlo después.
    for (campo, tipo) in [
        ("imageMessage", "imagen"),
        ("documentMessage", "documento"),
        ("audioMessage", "audio"),
        ("videoMessage", "video"),
    ] {
        let Some(m) = mensaje.get(campo) else { continue };
        let caption = m.get("caption").and_then(Value::as_str).map(str::to_string);
        let file_name = m
            .get("fileName")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| m.get("title").and_then(Value::as_str).map(str::to_string));
        let mimetype = m.get("mimetype").and_then(Value::as_str).map(str::to_string);
        let key = objeto.get("key").cloned().unwrap_or(Value::Null);
        return Some(EventoEntrante {
            texto: String::new(),
            nombre,
            evo_message_id,
            from_me,
            adjunto: Some(Adjunto { tipo, file_name, mimetype, caption, key }),
        });
    }

    None // reacción, evento sin media/texto reconocible, etc. (R9: se descarta, nunca falla)
}

/// Directorio donde se guardan los adjuntos recibidos de Letícia — mismo criterio de `HOME`
/// que `ruta_bitacora_defecto` en `main.rs` (nunca panic si falta HOME).
fn dir_adjuntos() -> std::path::PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join(".config/claude-peers/whatsapp_anexos")
}

/// Ruta fija del modelo whisper.cpp ya presente en esta máquina (descargado por la skill
/// `hyperframes`; reutilizado aquí para no duplicar ~1.5GB). Si no existe (otra máquina, modelo
/// movido), falla con un mensaje claro en vez de un "file not found" críptico de whisper-cli.
fn whisper_modelo_path() -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var("HOME").context("HOME no definido")?;
    let ruta = std::path::PathBuf::from(home).join(".cache/hyperframes/whisper/models/ggml-medium.bin");
    if !ruta.exists() {
        anyhow::bail!("modelo whisper no encontrado en {}", ruta.display());
    }
    Ok(ruta)
}

/// Convierte a WAV (`ffmpeg`) y transcribe con `whisper-cli` — whisper.cpp/miniaudio no
/// decodifica opus directo (confirmado en vivo contra un audio real de Letícia antes de este
/// código), por eso el paso intermedio. Idioma fijo `pt`: ella escribe/habla en PT-BR.
async fn transcribir_audio(ruta_audio: &std::path::Path) -> anyhow::Result<String> {
    let ruta_wav = ruta_audio.with_extension("wav");
    let salida = tokio::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(ruta_audio)
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&ruta_wav)
        .output()
        .await
        .context("ejecutando ffmpeg")?;
    if !salida.status.success() {
        anyhow::bail!("ffmpeg falló: {}", String::from_utf8_lossy(&salida.stderr));
    }

    let modelo = whisper_modelo_path()?;
    let salida = tokio::process::Command::new("whisper-cli")
        .arg("-m")
        .arg(&modelo)
        .arg("-f")
        .arg(&ruta_wav)
        .arg("-l")
        .arg("pt")
        .arg("--no-timestamps")
        .arg("-nt")
        .output()
        .await
        .context("ejecutando whisper-cli")?;
    let _ = tokio::fs::remove_file(&ruta_wav).await; // limpieza best-effort, no bloquea el resultado
    if !salida.status.success() {
        anyhow::bail!("whisper-cli falló: {}", String::from_utf8_lossy(&salida.stderr));
    }
    let texto = String::from_utf8_lossy(&salida.stdout).trim().to_string();
    if texto.is_empty() {
        anyhow::bail!("whisper-cli no produjo texto (audio vacío o silencio)");
    }
    Ok(texto)
}

/// Resuelve un `Adjunto` detectado por `extraer_entrante`: pide el contenido real a Evolution
/// (`getBase64FromMediaMessage` — única forma de desencriptar el media, la `url` del payload
/// original es un blob `.enc` inservible sin esto), lo guarda en disco, y si es audio lo
/// transcribe. Devuelve el texto final a persistir/empujar — nunca vacío, aunque falle la
/// transcripción (degrada a "avisar que llegó, hay que oírlo a mano").
pub async fn resolver_adjunto(cfg: &Config, evo_message_id: &str, adjunto: &Adjunto) -> anyhow::Result<String> {
    let url = format!("{}/chat/getBase64FromMediaMessage/{}", cfg.url.trim_end_matches('/'), cfg.instancia);
    let cuerpo = serde_json::json!({ "message": { "key": adjunto.key }, "convertToMp4": false });
    let (codigo, resp) = curl_post_json(cfg, &url, &cuerpo)
        .await
        .with_context(|| format!("descargando adjunto de Evolution ({evo_message_id})"))?;
    if !(200..300).contains(&codigo) {
        anyhow::bail!("Evolution getBase64FromMediaMessage ({evo_message_id}) respondió {codigo}");
    }
    let json: Value =
        serde_json::from_str(&resp).context("parseando respuesta de getBase64FromMediaMessage")?;
    let base64_str = json.get("base64").and_then(Value::as_str).context("respuesta sin campo 'base64'")?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, base64_str)
        .context("decodificando base64 del adjunto")?;

    let dir = dir_adjuntos();
    tokio::fs::create_dir_all(&dir).await.context("creando directorio de adjuntos")?;
    // WhatsApp casi nunca manda `fileName` en imagen/audio (solo en documentos) — sin extensión
    // el archivo guardado no se puede abrir con doble-click. Se infiere del `mimetype`.
    let nombre_archivo = adjunto.file_name.clone().unwrap_or_else(|| {
        let ext = match adjunto.mimetype.as_deref() {
            Some("image/jpeg") => "jpg",
            Some("image/png") => "png",
            Some("image/webp") => "webp",
            Some(m) if m.starts_with("audio/ogg") => "ogg",
            Some("video/mp4") => "mp4",
            _ => "bin",
        };
        format!("{evo_message_id}.{ext}")
    });
    let nombre_seguro = nombre_archivo.replace(['/', '\\'], "_");
    let ruta = dir.join(format!("{evo_message_id}_{nombre_seguro}"));
    tokio::fs::write(&ruta, &bytes).await.context("guardando adjunto en disco")?;

    let es_audio = adjunto.mimetype.as_deref().unwrap_or_default().starts_with("audio/");
    if es_audio {
        return Ok(match transcribir_audio(&ruta).await {
            Ok(transcripcion) => format!(
                "[Áudio de Letícia — transcrito automaticamente]\n\"{}\"\n(arquivo salvo em: {})",
                transcripcion,
                ruta.display()
            ),
            Err(err) => {
                tracing::warn!("whatsapp: fallo transcribiendo audio de '{evo_message_id}': {err:#}");
                format!(
                    "[Áudio de Letícia recebido — transcrição falhou, ouvir manualmente]\n(arquivo salvo em: {})",
                    ruta.display()
                )
            }
        });
    }

    let caption = adjunto.caption.as_deref().unwrap_or_default();
    let etiqueta = if caption.is_empty() { String::new() } else { format!("\nlegenda: {caption}") };
    Ok(format!(
        "[Letícia enviou um anexo — {} ({})]\nsalvo em: {}{etiqueta}",
        nombre_archivo,
        adjunto.mimetype.as_deref().unwrap_or("tipo desconhecido"),
        ruta.display()
    ))
}

/// Polling de RESPALDO: mientras el webhook (o el fan-out de la VM) no esté conectado, esta es
/// la vía que SÍ funciona sin depender de nadie más. Consulta `POST /chat/findMessages` UNA VEZ
/// POR CADA JID autorizado (número + LID si está configurado — el contacto puede tener el hilo
/// bajo cualquiera de los dos, comprobado en vivo) y aplica el MISMO filtro que el webhook
/// (`extraer_entrante`) a cada registro. No hace falta un cursor preciso: el dedupe real lo hace
/// el índice único de `evo_message_id` en la bitácora — pedir de más y descartar duplicados es
/// más simple y más robusto que llevar un cursor exacto para un volumen tan bajo.
pub async fn buscar_recientes(cliente: &reqwest::Client, cfg: &Config) -> anyhow::Result<Vec<EventoEntrante>> {
    let mut jids = vec![cfg.remoto_esperado()];
    if let Some(lid) = cfg.lid_esperado() {
        jids.push(lid);
    }
    let mut eventos = Vec::new();
    for jid in jids {
        let registros = buscar_mensajes_de(cliente, cfg, &jid).await?;
        eventos.extend(registros.iter().filter_map(|r| extraer_entrante(cfg, r)));
    }
    Ok(eventos)
}

/// Trae TODOS los registros de `POST /chat/findMessages` para un `remoteJid` puntual.
async fn buscar_mensajes_de(_cliente: &reqwest::Client, cfg: &Config, remote_jid: &str) -> anyhow::Result<Vec<Value>> {
    let url = format!("{}/chat/findMessages/{}", cfg.url.trim_end_matches('/'), cfg.instancia);
    let cuerpo = serde_json::json!({ "where": { "key": { "remoteJid": remote_jid } } });
    let (codigo, cuerpo_resp) = curl_post_json(cfg, &url, &cuerpo)
        .await
        .with_context(|| format!("llamando a Evolution findMessages ({remote_jid})"))?;
    if !(200..300).contains(&codigo) {
        anyhow::bail!("Evolution findMessages ({remote_jid}) respondió {codigo}");
    }
    let cuerpo_json: Value = serde_json::from_str(&cuerpo_resp)
        .with_context(|| format!("parseando la respuesta de findMessages ({remote_jid})"))?;
    Ok(cuerpo_json
        .pointer("/messages/records")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn cfg() -> Config {
        Config {
            url: "https://whatsapp.lexusfx.com".into(),
            api_key: "clave".into(),
            instancia: "lexusfx_604".into(),
            numero: "34604458061".into(),
            lid: Some("62032481095777".into()),
            webhook_clave: "secreto".into(),
        }
    }

    #[test]
    fn ignora_eco_de_nuestros_propios_envios() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": true, "id": "ABC"},
            "pushName": "Letícia",
            "message": {"conversation": "hola"},
        });
        assert!(extraer_entrante(&cfg(), &payload).is_none());
    }

    #[test]
    fn ignora_remitente_no_autorizado() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34600000000@s.whatsapp.net", "fromMe": false, "id": "XYZ"},
            "message": {"conversation": "hola"},
        });
        assert!(extraer_entrante(&cfg(), &payload).is_none());
    }

    #[test]
    fn acepta_mensaje_real_en_la_raiz() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": false, "id": "MSG1"},
            "pushName": "Letícia",
            "message": {"conversation": "oi, gostei muito!"},
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("debe reconocer el mensaje");
        assert_eq!(evento.texto, "oi, gostei muito!");
        assert_eq!(evento.nombre.as_deref(), Some("Letícia"));
        assert_eq!(evento.evo_message_id, "MSG1");
    }

    #[test]
    fn acepta_mensaje_anidado_en_data_extended_text() {
        let payload = serde_json::json!({
            "event": "messages.upsert",
            "data": {
                "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": false, "id": "MSG2"},
                "message": {"extendedTextMessage": {"text": "respondendo a citação"}},
            },
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("debe reconocer el mensaje anidado");
        assert_eq!(evento.texto, "respondendo a citação");
        assert_eq!(evento.nombre, None);
    }

    #[test]
    fn ignora_eco_fromme_en_el_numero_de_negocio_puro() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": true, "id": "REC1"},
            "message": {"conversation": "Oi, lembrete automático"},
        });
        assert!(extraer_entrante(&cfg(), &payload).is_none(), "eco automático en el canal de negocio se descarta");
    }

    #[test]
    fn acepta_fromme_true_en_el_hilo_lid_como_dialogo_real() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "62032481095777@lid", "fromMe": true, "id": "MAX1"},
            "message": {"conversation": "mor vou dormir"},
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("hilo LID: fromMe=true es diálogo real");
        assert!(evento.from_me);
        assert_eq!(evento.texto, "mor vou dormir");
    }

    #[test]
    fn acepta_fromme_false_en_el_hilo_lid() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "62032481095777@lid", "fromMe": false, "id": "LET1"},
            "message": {"conversation": "Top dms"},
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("hilo LID: fromMe=false es ella");
        assert!(!evento.from_me);
    }

    #[test]
    fn ignora_payload_sin_texto_ni_adjunto_reconocible() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": false, "id": "MSG3"},
            "message": {"reactionMessage": {"text": "👍"}},
        });
        assert!(extraer_entrante(&cfg(), &payload).is_none());
    }

    #[test]
    fn acepta_documento_sin_legenda_como_adjunto() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": false, "id": "DOC1"},
            "message": {"documentMessage": {
                "fileName": "lexusfx-email.html",
                "mimetype": "text/html",
            }},
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("documento sin legenda debe reconocerse");
        let adj = evento.adjunto.expect("debe traer adjunto");
        assert_eq!(adj.tipo, "documento");
        assert_eq!(adj.file_name.as_deref(), Some("lexusfx-email.html"));
        assert_eq!(adj.mimetype.as_deref(), Some("text/html"));
        assert_eq!(adj.caption, None);
    }

    #[test]
    fn acepta_imagen_con_legenda_como_adjunto() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "34604458061@s.whatsapp.net", "fromMe": false, "id": "IMG1"},
            "message": {"imageMessage": {"caption": "olha isso", "mimetype": "image/jpeg"}},
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("imagen con legenda debe reconocerse");
        let adj = evento.adjunto.expect("debe traer adjunto");
        assert_eq!(adj.tipo, "imagen");
        assert_eq!(adj.caption.as_deref(), Some("olha isso"));
    }

    #[test]
    fn acepta_audio_como_adjunto() {
        let payload = serde_json::json!({
            "key": {"remoteJid": "62032481095777@lid", "fromMe": false, "id": "AUD1"},
            "message": {"audioMessage": {"mimetype": "audio/ogg; codecs=opus"}},
        });
        let evento = extraer_entrante(&cfg(), &payload).expect("audio debe reconocerse");
        let adj = evento.adjunto.expect("debe traer adjunto");
        assert_eq!(adj.tipo, "audio");
        assert_eq!(adj.mimetype.as_deref(), Some("audio/ogg; codecs=opus"));
    }
}
