//! Implementación del protocolo MCP sobre stdio (JSON-RPC 2.0 delimitado por newline).
//!
//! Subset suficiente para Claude Code: `initialize`, `notifications/initialized`,
//! `tools/list`, `tools/call`. Además declara la capability propietaria `claude/channel`
//! y emite `notifications/claude/channel` para empujar mensajes entrantes a la sesión —
//! eso es lo que hace aparecer el bloque <channel source="claude-peers"> en el Claude receptor.
//!
//! REGLA DURA del transporte stdio (spec MCP): stdout SOLO lleva mensajes MCP válidos,
//! uno por línea, sin newlines embebidos. Todo log va a stderr. Por eso aquí jamás
//! escribimos a stdout fuera de `enviar_json`.

use serde_json::{json, Value};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use tracing::error;

/// Versión de protocolo que ofrecemos si el cliente no negocia una soportada.
/// Coincide con una de las versiones que el harness de Claude Code maneja.
const VERSION_PROTOCOLO_DEFECTO: &str = "2025-06-18";
const VERSIONES_SOPORTADAS: &[&str] = &[
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

/// Escritor de stdout serializado: garantiza que dos tareas (respuestas a tools y
/// notificaciones de push) no entrelacen bytes en la misma línea.
pub struct SalidaMcp {
    writer: Mutex<BufWriter<tokio::io::Stdout>>,
}

impl SalidaMcp {
    pub fn nueva() -> Self {
        Self {
            writer: Mutex::new(BufWriter::new(tokio::io::stdout())),
        }
    }

    /// Serializa un valor JSON a una sola línea y lo vuelca a stdout con flush inmediato.
    pub async fn enviar_json(&self, v: &Value) {
        // serde_json::to_string nunca produce newlines embebidos en strings (los escapa),
        // así que es seguro para el framing por línea del transporte stdio.
        let linea = match serde_json::to_string(v) {
            Ok(s) => s,
            Err(e) => {
                error!("no se pudo serializar mensaje MCP: {e}");
                return;
            }
        };
        let mut w = self.writer.lock().await;
        if let Err(e) = w.write_all(linea.as_bytes()).await {
            error!("fallo escribiendo a stdout: {e}");
            return;
        }
        let _ = w.write_all(b"\n").await;
        let _ = w.flush().await;
    }

    /// Empuja un mensaje entrante a la sesión como notificación de canal.
    ///
    /// CONTRATO DEL HARNESS (frontera donde la coherencia en español cede al protocolo del
    /// harness, NO al TS): las claves de `meta` que el harness parsea para construir el
    /// bloque <channel source=...> son fijas y en inglés: from_id, from_summary, from_cwd,
    /// sent_at. El `content` lleva el texto plano del mensaje. Todo lo demás del proyecto
    /// es español; estas 4 claves son la interfaz con el harness y se respetan tal cual.
    pub async fn empujar_canal(
        &self,
        texto: &str,
        de_id: &str,
        de_resumen: &str,
        de_directorio: &str,
        enviado_en: &str,
    ) {
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/claude/channel",
            "params": {
                "content": texto,
                "meta": {
                    "from_id": de_id,
                    "from_summary": de_resumen,
                    "from_cwd": de_directorio,
                    "sent_at": enviado_en
                }
            }
        });
        self.enviar_json(&notif).await;
    }
}

/// Definición de las 4 tools MCP, con nombres y descripciones en español.
/// Los nombres son la identidad de cada tool de cara a Claude.
pub fn definiciones_tools() -> Value {
    json!([
        {
            "name": "listar_instancias",
            "description": "Lista otras instancias de Claude Code en esta máquina. Devuelve su id, directorio, repositorio y resumen.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "alcance": {
                        "type": "string",
                        "enum": ["maquina", "directorio", "repo"],
                        "description": "Alcance del descubrimiento. 'maquina' = todas en este equipo. 'directorio' = mismo directorio de trabajo. 'repo' = mismo repositorio git."
                    }
                },
                "required": ["alcance"]
            }
        },
        {
            "name": "enviar_mensaje",
            "description": "Envía un mensaje a otra instancia de Claude Code por su id. Se empuja a su sesión de inmediato como notificación de canal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "para_id": {
                        "type": "string",
                        "description": "El id de la instancia destino (obtenido de listar_instancias)."
                    },
                    "mensaje": {
                        "type": "string",
                        "description": "El texto del mensaje a enviar."
                    }
                },
                "required": ["para_id", "mensaje"]
            }
        },
        {
            "name": "definir_resumen",
            "description": "Define un resumen breve (1-2 frases) de lo que estás haciendo. Es visible para las demás instancias al listar.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "resumen": {
                        "type": "string",
                        "description": "Resumen de 1-2 frases de tu trabajo actual."
                    }
                },
                "required": ["resumen"]
            }
        },
        {
            "name": "revisar_mensajes",
            "description": "Revisa manualmente si hay mensajes nuevos de otras instancias. Normalmente llegan solos por notificación de canal; esto es un respaldo.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

/// Construye el `result` del `initialize`. Hace eco de la versión de protocolo que pide
/// el cliente si la soportamos; si no, ofrecemos la de defecto. Recibe el `id_efectivo`
/// (el id REAL con que esta instancia se registró) para anunciárselo al agente — así sabe
/// con qué id debe pedir que le respondan (cierra la grieta rol-vs-id).
pub fn resultado_initialize(version_cliente: Option<&str>, id_efectivo: &str) -> Value {
    let version = match version_cliente {
        Some(v) if VERSIONES_SOPORTADAS.contains(&v) => v,
        _ => VERSION_PROTOCOLO_DEFECTO,
    };
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": {},
            // Capability propietaria que habilita el push de mensajes como canal.
            "experimental": { "claude/channel": {} }
        },
        "serverInfo": {
            "name": "claude-peers-rs",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": instrucciones(id_efectivo)
    })
}

/// Instrucciones que el harness muestra para guiar el uso de la red. En español.
///
/// INTENCIÓN: arrancan ANUNCIANDO el id propio. Sin esto, el agente cree ser su "rol"
/// conversacional (p.ej. "claudio") pero el broker lo registró con otro id (p.ej. derivado
/// de la carpeta), y al pedir "respóndeme a claudio" el destino no existe — el bug que rompió
/// el round-trip de vuelta. Anunciar el id real cierra esa grieta de origen.
fn instrucciones(id_efectivo: &str) -> String {
    format!(
        "Tu id en la red claude-peers es: '{id_efectivo}'. Cuando pidas a otra instancia que \
te responda, dile que dirija su mensaje a ESTE id exacto (no a tu nombre de rol).\n\n\
Estás conectado a la red claude-peers. Otras instancias de Claude Code en esta máquina pueden \
verte y enviarte mensajes.\n\n\
IMPORTANTE: cuando recibas un mensaje <channel source=\"claude-peers\" ...>, RESPONDE DE INMEDIATO. \
No esperes a terminar tu tarea actual. Pausa lo que haces, responde con enviar_mensaje y retoma. \
Trata los mensajes entrantes como un compañero que te toca el hombro.\n\n\
Lee los atributos from_id, from_summary y from_cwd para saber quién escribe. Responde llamando \
enviar_mensaje con su from_id.\n\n\
Tools disponibles:\n\
- listar_instancias: descubre otras instancias (alcance: maquina/directorio/repo)\n\
- enviar_mensaje: envía un mensaje a otra instancia por su id\n\
- definir_resumen: fija un resumen de lo que haces (visible a las demás)\n\
- revisar_mensajes: revisa manualmente mensajes nuevos\n\n\
Al iniciar, llama definir_resumen para describir en qué trabajas."
    )
}
