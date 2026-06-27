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
