//! Tipos compartidos del protocolo claude-peers-rs.
//!
//! Clon refactorizado en Rust de claude-peers, para uso personal de Max. TODO en español
//! de punta a punta — columnas, structs, protocolo. NO busca compatibilidad con el TS:
//! las instancias migrarán cuando se cambien todas a la vez.
//!
//! INTENCIÓN: que estos structs sean la ÚNICA fuente de verdad del formato de red, para que
//! un desajuste entre `peers-broker` y `peers-client` sea un error de compilación, no un bug
//! en tiempo de ejecución (que era el riesgo del TS, donde los tipos se casteaban con `as`).

use serde::{Deserialize, Serialize};

mod almacen;
pub use almacen::{Almacen, AlcanceListado};

/// Puerto por defecto del broker.
pub const PUERTO_DEFECTO: u16 = 7899;

/// Tolerancia de vida: una instancia está viva si fue vista en los últimos 45s.
/// Equivale a 3 latidos (cada 15s). Pasado ese umbral se considera muerta.
pub const VENCIMIENTO_MS: i64 = 45_000;

/// Una instancia de Claude Code registrada en la red. Espejo de la tabla `instancias`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instancia {
    /// Identificador estable por papel (ej. "claudia"). Es la identidad persistente.
    pub id: String,
    pub pid: i64,
    pub directorio: String,
    pub repo_git: Option<String>,
    /// Repositorio GitHub donde la instancia trabaja ("owner/repo"), derivado del remote
    /// origin de su git_root por el CLIENT (que corre en la máquina del peer con gh logado).
    /// El broker NO lo resuelve: solo usa el valor recibido para abrir issues en ESE repo.
    /// None si el directorio no es un repo GitHub → degradación (sin issue).
    pub repo_github: Option<String>,
    pub tty: Option<String>,
    /// Resumen de 1-2 frases de lo que la instancia está haciendo (visible a los demás).
    pub resumen: String,
    pub registrada_en: String, // ISO 8601
    pub visto_en: String,      // ISO 8601 — base de la liveness por latido
}

/// Un mensaje encolado entre instancias. Espejo de la tabla `mensajes`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mensaje {
    pub id: i64,
    pub de_id: String,
    pub para_id: String,
    pub texto: String,
    pub enviado_en: String, // ISO 8601
    pub entregado: bool,
}

// --- Cuerpos de petición/respuesta del broker (protocolo en español) ---

/// `POST /registrar`.
///
/// `id_preferido` es el FIX #1 frente al TS: si viene, el broker lo usa como id estable
/// (por papel) y un re-registro con el mismo id es un UPDATE que NO borra la fila pendiente.
/// En el TS el id era aleatorio y el re-registro borraba por PID, perdiendo los mensajes
/// encolados al reiniciar la instancia.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionRegistrar {
    pub pid: i64,
    pub directorio: String,
    pub repo_git: Option<String>,
    /// "owner/repo" del remote origin del git_root, resuelto por el client. None si no-GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_github: Option<String>,
    pub tty: Option<String>,
    #[serde(default)]
    pub resumen: String,
    /// Id estable solicitado por la instancia. Si es None, el broker genera uno aleatorio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_preferido: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaRegistrar {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionLatido {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionDefinirResumen {
    pub id: String,
    pub resumen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionListar {
    pub alcance: Alcance,
    pub directorio: String,
    pub repo_git: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluir_id: Option<String>,
}

/// Alcance del descubrimiento de instancias.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Alcance {
    /// Todas las instancias de esta máquina.
    Maquina,
    /// Solo las del mismo directorio de trabajo.
    Directorio,
    /// Solo las del mismo repositorio git (cae a directorio si no hay repo).
    Repo,
}

/// `POST /enviar`. El destino debe existir; si no, se responde con error claro,
/// NUNCA se descarta el mensaje en silencio (decisión explícita del diseño).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionEnviar {
    pub de_id: String,
    pub para_id: String,
    pub texto: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaEnviar {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionRecibir {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaRecibir {
    pub mensajes: Vec<Mensaje>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionSalir {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaSalud {
    pub estado: String,
    pub instancias: usize,
}

/// Respuesta genérica `{ "ok": true }` para endpoints sin cuerpo de retorno propio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaOk {
    pub ok: bool,
}

// ===========================================================================
// FASE 2 — "Empresa": jornada (tiempo medido por el broker) y outbox durable.
// ===========================================================================

/// Una sesión de trabajo de una instancia. El broker timbra inicio/fin con SU reloj —
/// la IA nunca estima el tiempo. Se abre en /registrar y se cierra en /salir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sesion {
    pub id: String,
    pub instancia_id: String,
    pub inicio: String,          // ISO 8601, timbrado por el broker
    pub fin: Option<String>,     // None mientras está abierta
    pub duracion_seg: Option<i64>,
}

/// Una tarea dentro de una sesión. Igual que la sesión, el tiempo lo timbra el broker.
/// Si hay integración GitHub activa, `issue_number` guarda la issue espejo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tarea {
    pub id: String,
    pub instancia_id: String,
    pub sesion_id: String,
    pub descripcion: String,
    pub inicio: String,
    pub fin: Option<String>,
    pub duracion_seg: Option<i64>,
    /// Número de la issue de GitHub espejo, si la integración está activa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
}

/// Un ítem del outbox durable. Toda solicitud peer→peer que deba sobrevivir a un
/// reinicio se materializa como un ItemOutbox con ACK: si el peer cae a mitad, al
/// volver lo encuentra pendiente y lo retoma (mata el "no vi llegar el mensaje").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemOutbox {
    pub id: String,
    pub para_id: String,
    pub texto: String,
    pub creado_en: String,
    /// true cuando el destinatario confirmó recepción (ACK). Hasta entonces, persiste.
    pub confirmado: bool,
}

// --- Peticiones/respuestas de jornada ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionAbrirTarea {
    pub instancia_id: String,
    pub descripcion: String,
    /// Área/etiqueta opcional para clasificar (se usa como label en la issue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaAbrirTarea {
    pub tarea_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionReportarTarea {
    pub tarea_id: String,
    pub texto: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionCerrarTarea {
    pub tarea_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionJornada {
    pub instancia_id: String,
}

/// Vista consolidada de la jornada de una instancia: sus sesiones y tareas con tiempos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaJornada {
    pub sesiones: Vec<Sesion>,
    pub tareas: Vec<Tarea>,
}

// ===========================================================================
// ADMIN — introspección del broker para la TUI (pantallas Peers/Redis/Broker).
// Todos los endpoints viven bajo el middleware de token (no en /salud).
// ===========================================================================

/// `GET /admin/info`. Datos de arranque del broker para la pantalla "Broker" de la TUI.
/// host/puerto vienen de la config del broker; `version` es env!("CARGO_PKG_VERSION").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaAdminInfo {
    pub host: String,
    pub puerto: u16,
    pub instancias: usize,
    pub version: String,
}

/// Una cola (de mensajes o de outbox) con su número de ítems pendientes, por instancia.
/// Es el item de las listas de `RespuestaAdminRedis`. SOLO LECTURA: nunca drena.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColaResumen {
    pub id: String,
    pub pendientes: usize,
}

/// `GET /admin/redis`. Resumen del estado del almacén para la pantalla "Redis" de la TUI:
/// total de instancias y, por cada una, cuántos mensajes y cuántos ítems de outbox tiene
/// pendientes. Construido con métodos de SOLO LECTURA (no consume colas).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaAdminRedis {
    pub total_instancias: usize,
    pub colas: Vec<ColaResumen>,
    pub outbox: Vec<ColaResumen>,
}

/// `POST /admin/purgar`. Borra la cola de mensajes y el outbox de `id`. La TUI la dispara
/// desde la pantalla Redis como acción explícita de mantenimiento.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionPurgar {
    pub id: String,
}
