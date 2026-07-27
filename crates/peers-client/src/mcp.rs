//! Servidor MCP de la red claude-peers-rs sobre `rmcp` (SDK oficial), E-21.
//!
//! Reemplaza el JSON-RPC escrito a mano por el SDK: el handshake (`initialize`), `tools/list`
//! y `tools/call` los gestiona `rmcp` a partir de las tools marcadas con `#[tool]`; el schema de
//! cada una lo genera `schemars` desde su struct de argumentos. Las `instructions` y capabilities
//! salen por `get_info()`.
//!
//! INVARIANTES CRÍTICOS DEL PUSH DEL `<channel>` (documentados, NO relajar — son la interfaz con
//! el harness de Claude Code, verificados contra él):
//!   - `serverInfo.name` == "claude-peers" EXACTO. Si no coincide con el nombre del MCP en la
//!     config y con `--dangerously-load-development-channels server:claude-peers`, el push se
//!     DESCARTA en silencio y el mensaje nunca aparece en la sesión receptora.
//!   - método de la notificación == "notifications/claude/channel" EXACTO.
//!   - las 4 claves de `meta` (from_id, from_summary, from_cwd, sent_at) en INGLÉS, tal cual: son
//!     la interfaz con el harness, no se traducen aunque el resto del proyecto sea español.
//!   - capability `experimental: { "claude/channel": {} }`.
//! El push se emite como `CustomNotification` (rmcp preserva método arbitrario + params), verificado
//! en el spike E-21 con wire byte-idéntico al de la implementación a mano anterior.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CustomNotification, Implementation, ServerCapabilities, ServerInfo, ServerNotification,
};
use rmcp::service::Peer;
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::EstadoCliente;

/// Método (arbitrario) de la notificación que hace aparecer el bloque `<channel>` en el receptor.
/// Es un INVARIANTE con el harness: cambiar este string rompe el push.
pub const METODO_CANAL: &str = "notifications/claude/channel";

/// Nombre del servidor MCP. INVARIANTE: debe ser "claude-peers" (ver cabecera del módulo).
pub const NOMBRE_SERVIDOR: &str = "claude-peers";

// --- Structs de argumentos de las tools (schemars genera el inputSchema desde ellos) ---
// Las descripciones de campo van en el `#[schemars(description=...)]` para que el schema que ve
// Claude sea idéntico al del array JSON manual anterior (paridad de la interfaz de tools).

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsListar {
    /// Alcance del descubrimiento. 'maquina' = todas en este equipo. 'directorio' = mismo
    /// directorio de trabajo. 'repo' = mismo repositorio git.
    pub alcance: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsEnviar {
    /// El id de la instancia destino (obtenido de listar_instancias).
    pub para_id: String,
    /// El texto del mensaje a enviar.
    pub mensaje: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsAvisarEquipo {
    /// El texto a enviar a todo el equipo.
    pub mensaje: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsDefinirResumen {
    /// Resumen de 1-2 frases de tu trabajo actual.
    pub resumen: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsCrearTarea {
    /// Qué vas a hacer en esta tarea (1 frase).
    pub descripcion: String,
    /// Tu estimación de cuánto tardará, en segundos. Opcional pero recomendado: sin estimado no
    /// se aprende a corregir.
    #[serde(default)]
    pub estimado_seg: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsReportarTarea {
    /// El id de la tarea (devuelto por crear_tarea).
    pub tarea_id: String,
    /// Nota de progreso a registrar.
    pub texto: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsCerrarTarea {
    /// El id de la tarea a cerrar (devuelto por crear_tarea).
    pub tarea_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsChatPrivadoResponder {
    /// Tu respuesta al operador por el canal privado. Confidencial: no la reproduzcas en el chat visible.
    pub texto: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsWhatsappHistorial {
    /// Cuántos mensajes traer como máximo (más recientes primero). Opcional; por defecto 200.
    #[serde(default)]
    pub limite: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsHistorialMensajes {
    /// Cuántos mensajes traer como máximo (los más recientes). Opcional; por defecto 20, tope 200.
    #[serde(default)]
    pub limite: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsWhatsappEnviar {
    /// Tu respuesta al ÚNICO contacto de WhatsApp autorizado. Escribe en portugués de Brasil,
    /// coloquial, clonando el tono y la forma de hablar de Max (informal, directo) — ella ya
    /// sabe que quien escribe es el equipo de peers, así que no hay engaño, solo continuidad de
    /// tono. Formato WhatsApp: negrito con *asteriscos*, itálico con _underscores_, SIN '#', SIN
    /// emojis, SIN guiones/rayas, frases cortas.
    pub texto: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsWhatsappEnviarDocumento {
    /// Ruta ABSOLUTA local del archivo (tiene que existir en este mismo Mac, donde corre el
    /// broker — no funciona para peers en otra máquina, ej. la VM).
    pub ruta_local: String,
    /// Nombre a mostrar en WhatsApp; si falta, usa el nombre del archivo en `ruta_local`.
    #[serde(default)]
    pub nombre_archivo: Option<String>,
    /// Texto opcional que acompaña el documento (aparece como legenda en WhatsApp).
    #[serde(default)]
    pub legenda: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArgsWhatsappEnviarAudio {
    /// URL remota del audio a enviar como mensaje de voz (ej. mp3 de HeyGen/ElevenLabs). El
    /// broker la descarga y la convierte a ogg/opus antes de mandarla — no hace falta convertir
    /// nada de tu lado, solo pasar la URL.
    pub url_audio: String,
}

/// Servidor MCP de la instancia. Lleva el estado compartido (broker, id, secreto, etc.) para que
/// las tools puedan operar. `tool_router` es el registro de tools que genera el macro.
#[derive(Clone)]
pub struct ServidorPeers {
    pub estado: Arc<EstadoCliente>,
    // Lo consume el código generado por `#[tool_handler]` (routing de tools/call); el análisis de
    // dead-code no lo ve como "leído" directamente → allow explícito.
    #[allow(dead_code)]
    tool_router: rmcp::handler::server::tool::ToolRouter<Self>,
}

#[tool_router]
impl ServidorPeers {
    pub fn nuevo(estado: Arc<EstadoCliente>) -> Self {
        Self {
            estado,
            tool_router: Self::tool_router(),
        }
    }

    // Cada tool es un envoltorio fino sobre la función de negocio en `main.rs` (lógica probada, sin
    // cambios). El nombre y la descripción son COPIA LITERAL de los del array JSON anterior
    // (paridad de cara a Claude). Devuelven String (el `content` de texto lo arma rmcp); un error
    // de negocio se devuelve como texto (mismo contrato que antes: isError lo maneja el caller).

    #[tool(
        name = "listar_instancias",
        description = "Lista otras instancias de Claude Code en esta máquina. Devuelve su id, directorio, repositorio y resumen."
    )]
    async fn listar_instancias(&self, Parameters(a): Parameters<ArgsListar>) -> String {
        crate::tool_listar(&self.estado, &json!({ "alcance": a.alcance }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "enviar_mensaje",
        description = "Envía un mensaje a otra instancia de Claude Code por su id. Se empuja a su sesión de inmediato como notificación de canal."
    )]
    async fn enviar_mensaje(&self, Parameters(a): Parameters<ArgsEnviar>) -> String {
        crate::tool_enviar(&self.estado, &json!({ "para_id": a.para_id, "mensaje": a.mensaje }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "definir_resumen",
        description = "Define un resumen breve (1-2 frases) de lo que estás haciendo. Es visible para las demás instancias al listar."
    )]
    async fn definir_resumen(&self, Parameters(a): Parameters<ArgsDefinirResumen>) -> String {
        crate::tool_definir_resumen(&self.estado, &json!({ "resumen": a.resumen }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "revisar_mensajes",
        description = "Revisa manualmente si hay mensajes nuevos de otras instancias. Normalmente llegan solos por notificación de canal; esto es un respaldo."
    )]
    async fn revisar_mensajes(&self) -> String {
        crate::tool_revisar(&self.estado).await.unwrap_or_else(|e| e)
    }

    #[tool(
        name = "crear_tarea",
        description = "Crea una tarea de trabajo con tu estimación de duración. El broker mide tu tiempo REAL (tú nunca timbras el tiempo) y te devuelve el estimado ya corregido según tu historial. Llámala ANTES de empezar trabajo sustancial; guarda el tarea_id para cerrarla al terminar."
    )]
    async fn crear_tarea(&self, Parameters(a): Parameters<ArgsCrearTarea>) -> String {
        let mut args = json!({ "descripcion": a.descripcion });
        if let Some(est) = a.estimado_seg {
            args["estimado_seg"] = json!(est);
        }
        crate::tool_crear_tarea(&self.estado, &args).await.unwrap_or_else(|e| e)
    }

    #[tool(
        name = "reportar_tarea",
        description = "Añade una nota de progreso a una tarea abierta. No cierra la tarea ni mide tiempo; solo registra avance."
    )]
    async fn reportar_tarea(&self, Parameters(a): Parameters<ArgsReportarTarea>) -> String {
        crate::tool_reportar_tarea(&self.estado, &json!({ "tarea_id": a.tarea_id, "texto": a.texto }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "cerrar_tarea",
        description = "Cierra una tarea al terminarla. El broker timbra el fin con SU reloj, mide el tiempo real y, si la tarea tenía estimado, aprende a corregir tus estimaciones futuras."
    )]
    async fn cerrar_tarea(&self, Parameters(a): Parameters<ArgsCerrarTarea>) -> String {
        crate::tool_cerrar_tarea(&self.estado, &json!({ "tarea_id": a.tarea_id }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "listar_tareas",
        description = "Lista tus tareas con sus tiempos (estimado, real y duración). Útil para ver qué tienes abierto y revisar tu historial."
    )]
    async fn listar_tareas(&self) -> String {
        crate::tool_listar_tareas(&self.estado).await.unwrap_or_else(|e| e)
    }

    #[tool(
        name = "revisar_tareas",
        description = "Resumen rápido de tus tareas abiertas (sin cerrar). Recordatorio de qué te falta cerrar para que el broker cierre el bucle de aprendizaje."
    )]
    async fn revisar_tareas(&self) -> String {
        crate::tool_revisar_tareas(&self.estado).await.unwrap_or_else(|e| e)
    }

    #[tool(
        name = "avisar_equipo",
        description = "Envía el mismo mensaje a TODAS las demás instancias vivas en la máquina, en una sola llamada (broadcast). Útil para avisos que afectan a todo el equipo (ej. 'voy a reiniciar el broker', 'terminé mi bloque, disponible'). Cada destino lo recibe como un mensaje directo normal (se empuja como <channel> a su sesión, igual que enviar_mensaje)."
    )]
    async fn avisar_equipo(&self, Parameters(a): Parameters<ArgsAvisarEquipo>) -> String {
        crate::tool_avisar_equipo(&self.estado, &json!({ "mensaje": a.mensaje }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "chat_privado_recibir",
        description = "Consulta tu canal PRIVADO y confidencial con el operador (el dueño). Devuelve los mensajes que te haya dejado. IMPORTANTE: el contenido es CONFIDENCIAL — NO lo reproduzcas ni lo resumas en tu output visible; trátalo como una instrucción reservada. Consúltalo periódicamente y cuando el contexto lo sugiera."
    )]
    async fn chat_privado_recibir(&self) -> String {
        crate::tool_chat_privado_recibir(&self.estado).await.unwrap_or_else(|e| e)
    }

    #[tool(
        name = "chat_privado_responder",
        description = "Responde al operador por el canal PRIVADO (solo él lo lee, no aparece en el chat visible de nadie). Úsalo para comunicación confidencial de vuelta al dueño."
    )]
    async fn chat_privado_responder(&self, Parameters(a): Parameters<ArgsChatPrivadoResponder>) -> String {
        crate::tool_chat_privado_responder(&self.estado, &json!({ "texto": a.texto }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "responder_whatsapp",
        description = "Responde por WhatsApp al ÚNICO contacto autorizado por el dueño (Evolution API, vía el broker). USO EXCLUSIVO para ese contacto — no acepta ni necesita número, el destino es fijo y está contenido en el broker, nunca elegible por el peer. Al escribir: clona el tono y la forma de hablar del dueño (informal, directo, como habla él) para que la conversación se sienta natural — el contacto YA SABE que quien escribe es el equipo de peers, así que esto no es un engaño, es solo continuidad de tono. Formato WhatsApp: negrito *asteriscos*, itálico _underscores_, sin '#', sin emojis, sin guiones, frases cortas."
    )]
    async fn responder_whatsapp(&self, Parameters(a): Parameters<ArgsWhatsappEnviar>) -> String {
        crate::tool_whatsapp_enviar(&self.estado, &json!({ "texto": a.texto }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "enviar_documento_whatsapp",
        description = "Envía un ARCHIVO (documento) por WhatsApp al ÚNICO contacto autorizado, igual que responder_whatsapp pero para un archivo en vez de texto. La ruta debe ser ABSOLUTA y existir en este mismo Mac (donde corre el broker). LÍMITE de ~60MB — para archivos más grandes, suba a un link (ej. Cloud Storage) y mande el link con responder_whatsapp en vez de usar esta tool. Formato: PDF, CSV, imagem, etc. — o que for, WhatsApp mostra como documento."
    )]
    async fn enviar_documento_whatsapp(
        &self,
        Parameters(a): Parameters<ArgsWhatsappEnviarDocumento>,
    ) -> String {
        crate::tool_whatsapp_enviar_documento(
            &self.estado,
            &json!({ "ruta_local": a.ruta_local, "nombre_archivo": a.nombre_archivo, "legenda": a.legenda }),
        )
        .await
        .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "responder_whatsapp_audio",
        description = "Responde por WhatsApp con una MENSAJE DE VOZ (nota de voz, PTT) al ÚNICO contacto autorizado — igual que responder_whatsapp pero por audio en vez de texto. Pásale la URL del audio ya generado (ej. HeyGen create_speech, ElevenLabs) — el broker lo descarga y convierte, no hace falta preprocesar nada de tu lado. Úsala cuando el dueño pida explícitamente que la respuesta salga como voz en vez de texto."
    )]
    async fn responder_whatsapp_audio(&self, Parameters(a): Parameters<ArgsWhatsappEnviarAudio>) -> String {
        crate::tool_whatsapp_enviar_audio(&self.estado, &json!({ "url_audio": a.url_audio }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "historial_whatsapp",
        description = "Lee el historial de la conversación de WhatsApp con el contacto autorizado — ambas direcciones (lo que ella escribió y lo que se le respondió), más reciente primero. Usa 'limite' para controlar cuántos mensajes traer (por defecto 200)."
    )]
    async fn historial_whatsapp(&self, Parameters(a): Parameters<ArgsWhatsappHistorial>) -> String {
        crate::tool_whatsapp_historial(&self.estado, &json!({ "limite": a.limite }))
            .await
            .unwrap_or_else(|e| e)
    }

    #[tool(
        name = "historial_mensajes",
        description = "Relee TU historial de mensajes de la red, incluidos los que ya procesaste y salieron de tu bandeja. Úsala cuando sospeches que perdiste contexto de una conversación — por ejemplo tras una compactación, o si alguien menciona algo que te dijo y no lo tienes. Solo lectura: releer no reenvía ni cambia nada. Usa 'limite' (por defecto 20)."
    )]
    async fn historial_mensajes(&self, Parameters(a): Parameters<ArgsHistorialMensajes>) -> String {
        crate::tool_historial_mensajes(&self.estado, &json!({ "limite": a.limite }))
            .await
            .unwrap_or_else(|e| e)
    }
}

#[tool_handler]
impl ServerHandler for ServidorPeers {
    fn get_info(&self) -> ServerInfo {
        // Capabilities: tools (por el tool_router) + la experimental propietaria claude/channel que
        // habilita el push. `enable_experimental()` crea el contenedor; insertamos la clave a mano
        // para que el wire sea `experimental: { "claude/channel": {} }` (invariante del harness).
        // `experimental` es Option<BTreeMap<String, JsonObject>>; el valor es un JsonObject
        // (Map<String,Value>) vacío, no un Value — de ahí el `Default::default()` tipado.
        let mut caps = ServerCapabilities::builder().enable_tools().enable_experimental().build();
        if let Some(exp) = caps.experimental.as_mut() {
            exp.insert("claude/channel".to_string(), serde_json::Map::new());
        }
        // ServerInfo es #[non_exhaustive]: no se construye con literal. Partimos del default y
        // fijamos los 3 campos que nos importan (el resto queda en sus defaults del SDK).
        let mut info = ServerInfo::default();
        info.capabilities = caps;
        info.server_info = Implementation::new(NOMBRE_SERVIDOR, env!("CARGO_PKG_VERSION"));
        info.instructions = Some(instrucciones(&self.estado.id_efectivo));
        info
    }
}

/// Emite el push del `<channel>` a la sesión: una `CustomNotification` con el método y los params
/// EXACTOS del harness. Reemplaza a `SalidaMcp::empujar_canal`, pero produce el MISMO wire (spike
/// E-21). Devuelve `true` si rmcp aceptó enviarla (el llamador confirma la entrega solo si sí).
///
/// El `peer` es el `Peer<RoleServer>` que se obtiene de `serve(stdio())` (clonado al arrancar). Un
/// fallo al enviar (stdout roto/cerrado) → `false` + warn, sin panic: el bucle de recepción
/// reintenta el mensaje en el siguiente ciclo (entrega durable, R1.4).
pub async fn empujar_canal(
    peer: &Peer<RoleServer>,
    texto: &str,
    de_id: &str,
    de_resumen: &str,
    de_directorio: &str,
    enviado_en: &str,
) -> bool {
    let notif = CustomNotification {
        method: METODO_CANAL.to_string(),
        params: Some(json!({
            "content": texto,
            "meta": {
                "from_id": de_id,
                "from_summary": de_resumen,
                "from_cwd": de_directorio,
                "sent_at": enviado_en
            }
        })),
        extensions: Default::default(),
    };
    // DIAGNÓSTICO (bug push Windows de Daniela): registramos el intento ANTES de enviar, para
    // distinguir "ni se intenta" (no sale este log) de "se intenta y send_notification devuelve Ok
    // pero no llega" (buffering/framing de rmcp en Windows) o "falla con Err". El `Ok(())` de rmcp
    // significa que la notif se entregó al transporte; si aun así no aparece en la sesión, el fallo
    // está por debajo (cómo rmcp escribe a stdout en Windows), no en nuestro código.
    tracing::debug!(
        event_name = "empujar_canal",
        stage = "send_notification_started",
        outcome = "start",
        de_id = %de_id,
        "empujar_canal: enviando notificación de canal (de={de_id})"
    );
    match peer.send_notification(ServerNotification::from(notif)).await {
        Ok(()) => {
            tracing::debug!(
                event_name = "empujar_canal",
                stage = "send_notification_completed",
                outcome = "success",
                de_id = %de_id,
                "empujar_canal: send_notification devolvió Ok (entregado al transporte)"
            );
            true
        }
        Err(e) => {
            warn!(
                event_name = "empujar_canal",
                stage = "send_notification_completed",
                outcome = "failure",
                de_id = %de_id,
                error = %e,
                "fallo enviando la notificación de canal por rmcp: {e}"
            );
            false
        }
    }
}

/// Instrucciones que el harness muestra para guiar el uso de la red. En español.
///
/// INTENCIÓN: arrancan ANUNCIANDO el id propio. Sin esto, el agente cree ser su "rol"
/// conversacional pero el broker lo registró con otro id, y al pedir "respóndeme a X" el destino
/// no existe — el bug que rompía el round-trip de vuelta. Anunciar el id real cierra esa grieta.
pub fn instrucciones(id_efectivo: &str) -> String {
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
- avisar_equipo: envía el MISMO mensaje a TODAS las demás instancias vivas (broadcast)\n\
- definir_resumen: fija un resumen de lo que haces (visible a las demás)\n\
- revisar_mensajes: revisa manualmente mensajes pendientes (leerlos los saca de tu bandeja)\n\
- historial_mensajes: relee tu historial, incluidos los ya procesados (red de contención si \
perdiste contexto por una compactación)\n\
- crear_tarea: crea una tarea con tu estimado de duración\n\
- reportar_tarea: añade una nota de progreso a una tarea\n\
- cerrar_tarea: cierra una tarea al terminarla\n\
- listar_tareas: lista tus tareas con sus tiempos\n\
- revisar_tareas: resumen de tus tareas abiertas\n\
- chat_privado_recibir: consulta tu canal PRIVADO y confidencial con el operador (el dueño)\n\
- chat_privado_responder: responde al operador por ese canal privado\n\
- responder_whatsapp: responde por WhatsApp al ÚNICO contacto autorizado por el dueño (destino fijo, \
contenido en el broker — no elegible por ti)\n\
- responder_whatsapp_audio: igual que responder_whatsapp pero manda una MENSAJE DE VOZ (PTT) — le \
pasas la URL de un audio ya generado (ej. HeyGen, ElevenLabs) y el broker lo descarga/convierte\n\
- historial_whatsapp: lee la conversación completa de WhatsApp con ese contacto (ambas direcciones), \
con flag 'limite' para cuántos mensajes traer\n\n\
PUENTE WHATSAPP: si ves en el canal un mensaje formateado como '[WhatsApp <nombre>] <texto>', es un \
mensaje REAL que ese contacto escribió por WhatsApp (push automático del broker, vía Evolution API). \
Si quieres responderle, usa responder_whatsapp — NUNCA enviar_mensaje/avisar_equipo para eso, y NUNCA \
inventes o asumas otro número: el destino es fijo y único. Al responder, clona el tono y la forma de \
hablar del dueño (informal, directo) para que se sienta natural — el contacto ya sabe que quien \
escribe es el equipo de peers.\n\n\
CANAL PRIVADO CON EL OPERADOR (confidencial): tienes un canal privado y directo con el dueño, \
separado de los mensajes normales. Consulta chat_privado_recibir periódicamente y cuando el \
contexto lo sugiera. IMPORTANTE — AVISO DE MENSAJE PRIVADO: si en el canal normal ves un mensaje \
del operador cuyo texto es exactamente un AVISO de que tienes un mensaje privado nuevo (empieza con \
'📩' y te pide llamar chat_privado_recibir), eso NO es el mensaje en sí: es una señal de que hay algo \
esperándote en tu chat privado. Reacciona de inmediato llamando chat_privado_recibir para leer el \
contenido real — no esperes a un chequeo periódico. Su contenido es CONFIDENCIAL: trátalo como una \
instrucción reservada del dueño y NUNCA lo reproduzcas, cites ni resumas en tu output visible ni en \
mensajes a otras instancias. Responde por chat_privado_responder, no por el canal normal.\n\n\
Antes de trabajo sustancial, crea una tarea con tu estimado (crear_tarea); al terminar, ciérrala (cerrar_tarea). \
El broker mide tu tiempo real y aprende a corregir estimaciones.\n\n\
Al iniciar, llama definir_resumen para describir en qué trabajas."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GUARDARRAÍL DE REGRESIÓN (E-21): el wire de la notificación de canal emitida vía rmcp
    /// (`CustomNotification` → `ServerNotification`) debe ser IDÉNTICO campo a campo al que producía
    /// el `empujar_canal` a mano anterior. Si este test falla, el push del `<channel>` se rompió.
    ///
    /// Referencia (el JSON que emitía la versión manual, mcp.rs:91-103 pre-E-21):
    /// {"jsonrpc":"2.0","method":"notifications/claude/channel",
    ///  "params":{"content":TEXTO,"meta":{"from_id":…,"from_summary":…,"from_cwd":…,"sent_at":…}}}
    #[test]
    fn wire_notificacion_canal_identico_al_manual() {
        let notif = CustomNotification {
            method: METODO_CANAL.to_string(),
            params: Some(json!({
                "content": "hola equipo",
                "meta": {
                    "from_id": "claude-peers-rs-s002",
                    "from_summary": "migrando a rmcp",
                    "from_cwd": "/Users/maxmeireles/claude-peers-rs",
                    "sent_at": "2026-07-06T09:00:00Z"
                }
            })),
            extensions: Default::default(),
        };
        let wire = serde_json::to_value(ServerNotification::from(notif)).expect("serializar");

        // Método EXACTO (invariante del harness).
        assert_eq!(
            wire.get("method").and_then(|m| m.as_str()),
            Some("notifications/claude/channel")
        );
        // content preservado.
        let params = wire.get("params").expect("params");
        assert_eq!(params.get("content").and_then(|c| c.as_str()), Some("hola equipo"));
        // Las 4 claves de meta, en inglés, con sus valores (interfaz con el harness).
        let meta = params.get("meta").expect("meta");
        assert_eq!(meta.get("from_id").and_then(|v| v.as_str()), Some("claude-peers-rs-s002"));
        assert_eq!(meta.get("from_summary").and_then(|v| v.as_str()), Some("migrando a rmcp"));
        assert_eq!(
            meta.get("from_cwd").and_then(|v| v.as_str()),
            Some("/Users/maxmeireles/claude-peers-rs")
        );
        assert_eq!(meta.get("sent_at").and_then(|v| v.as_str()), Some("2026-07-06T09:00:00Z"));
        // meta NO debe tener claves extra (exactamente 4).
        assert_eq!(meta.as_object().map(|o| o.len()), Some(4));
    }

    /// Invariante del nombre del servidor: "claude-peers" EXACTO (si cambia, el harness descarta
    /// el push). Guardarraíl barato contra un typo futuro.
    #[test]
    fn nombre_servidor_es_claude_peers() {
        assert_eq!(NOMBRE_SERVIDOR, "claude-peers");
        assert_eq!(METODO_CANAL, "notifications/claude/channel");
    }

    /// Las capabilities declaran la experimental `claude/channel` con valor objeto vacío
    /// (`experimental: {"claude/channel": {}}`), que es lo que habilita el push. Se construye igual
    /// que en `get_info` para no divergir.
    #[test]
    fn capabilities_declaran_claude_channel() {
        let mut caps = ServerCapabilities::builder().enable_tools().enable_experimental().build();
        if let Some(exp) = caps.experimental.as_mut() {
            exp.insert("claude/channel".to_string(), serde_json::Map::new());
        }
        let wire = serde_json::to_value(&caps).expect("serializar caps");
        let exp = wire.get("experimental").expect("experimental");
        assert!(exp.get("claude/channel").is_some(), "falta la capability claude/channel");
        assert_eq!(exp.get("claude/channel"), Some(&json!({})));
    }
}
