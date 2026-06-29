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

/// Cuántos mensajes retiene el historial durable por cola (los más recientes).
/// Usado por la limpieza periódica del broker (ZREMRANGEBYRANK). Ver R2.1.
pub const RETENCION_HISTORIAL: usize = 500;

/// Cuántas tareas retiene el almacén por instancia (las más recientes). Espejo de
/// `RETENCION_HISTORIAL` para las tareas (#15/R6). La limpieza periódica del broker
/// (`podar_tareas`) conserva las RETENCION_TAREAS más recientes por peer y purga las viejas.
pub const RETENCION_TAREAS: usize = 500;

/// Estado de un mensaje en su ciclo de vida. Máquina de estados timbrada por el broker
/// (nunca por la IA). El avance natural es `Enviado → Entregado → Leido → Procesado`;
/// `Fallido` y `DeadLetter` son terminales alternativos (R1.2).
///
/// INTENCIÓN: que el orden de las variantes refleje el avance monótono, para que una
/// transición que "retrocede" sea detectable comparando el rango (ver `rango`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EstadoMensaje {
    /// Encolado en la bandeja del destino; aún no entregado.
    Enviado,
    /// El cliente del destino confirmó que lo empujó al canal (flush OK).
    Entregado,
    /// El destinatario lo leyó (lo vio en su <channel>).
    Leido,
    /// El destinatario terminó de actuar sobre él; sale de la bandeja activa (R1.5).
    Procesado,
    /// La entrega falló de forma recuperable; candidato a reintento.
    Fallido,
    /// La entrega falló de forma definitiva; va a la cola de mensajes muertos.
    DeadLetter,
}

impl EstadoMensaje {
    /// Rango monótono del avance natural. `Enviado < Entregado < Leido < Procesado`.
    /// `Fallido`/`DeadLetter` son terminales y no encajan en el orden lineal: se les
    /// asigna un rango alto para que una transición hacia ellos siempre se acepte.
    /// INTENCIÓN: que `transicionar` decida "avanza o no" comparando rangos, no con
    /// una tabla de pares válidos que habría que mantener.
    #[must_use]
    pub fn rango(self) -> u8 {
        match self {
            EstadoMensaje::Enviado => 0,
            EstadoMensaje::Entregado => 1,
            EstadoMensaje::Leido => 2,
            EstadoMensaje::Procesado => 3,
            EstadoMensaje::Fallido => 4,
            EstadoMensaje::DeadLetter => 5,
        }
    }
}

/// Un mensaje encolado entre instancias. Espejo de la tabla `mensajes`.
///
/// COMPAT: los campos nuevos (estado + timestamps + contadores + traza de reenvío)
/// llevan `#[serde(default)]` para que un JSON viejo (con `entregado:true` y sin estos
/// campos) deserialice sin romper. `entregado` desaparece del struct, pero un JSON que
/// lo traiga simplemente lo ignora serde (campo desconocido = no error por defecto).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mensaje {
    pub id: i64,
    pub de_id: String,
    pub para_id: String,
    pub texto: String,
    pub enviado_en: String, // ISO 8601
    /// Estado actual de la máquina. Si falta en el JSON (mensaje viejo), nace `Enviado`.
    #[serde(default = "estado_por_defecto")]
    pub estado: EstadoMensaje,
    /// Timestamp ISO timbrado por el broker al pasar a `Entregado` (HSETNX = solo la 1ª vez).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entregado_en: Option<String>,
    /// Timestamp ISO al pasar a `Leido`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leido_en: Option<String>,
    /// Timestamp ISO al pasar a `Procesado`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procesado_en: Option<String>,
    /// Nº de intentos de entrega (para reintento/DLQ en fases futuras).
    #[serde(default)]
    pub intentos: u32,
    /// Si este mensaje es un reenvío, el `id` del mensaje original (R2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reenviado_de: Option<i64>,
    /// Nº de veces que se reenvió la cadena de este mensaje.
    #[serde(default)]
    pub reenvios: u32,
}

/// Estado por defecto al deserializar un mensaje sin campo `estado` (mensaje viejo).
fn estado_por_defecto() -> EstadoMensaje {
    EstadoMensaje::Enviado
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

/// Estado del ciclo de vida de una tarea, gestionado por Max como jefe (kanban).
///
/// INTENCIÓN: enum cerrado (no "stringly typed") compartido entre broker y TUI, igual que
/// `EstadoMensaje`/`TipoAlerta`. Serializa en minúsculas para encajar con el protocolo.
/// `#[serde(default)]` en `Tarea::estado` nace `Abierta` (de ahí el `impl Default`), para
/// que una tarea vieja (JSON sin `estado`) deserialice como `Abierta` sin romper (AC6/R2).
///
/// REGLA DEL FACTOR (R3): SOLO `Hecha` con estimado+real válidos alimenta el factor de
/// corrección. `Cancelada` y las demás NO lo contaminan — esa decisión vive en el broker
/// (`actualizar_factor` se llama solo en la transición a `Hecha`), aquí solo es el vocabulario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EstadoTarea {
    /// Recién creada/asignada; nadie la ha empezado. Estado por defecto (compat).
    Abierta,
    /// El peer está trabajando en ella activamente.
    EnCurso,
    /// Parada por un impedimento; `Tarea::bloqueo_motivo` explica por qué.
    Bloqueada,
    /// Terminada con éxito. Única transición que alimenta el factor (R3).
    Hecha,
    /// Abortada sin completar. NO contamina el factor de estimación.
    Cancelada,
}

/// Por defecto una tarea nace `Abierta`. Soporta `#[serde(default)]` en `Tarea::estado`
/// para que las tareas viejas (sin el campo) deserialicen como `Abierta` (AC6/R2).
impl Default for EstadoTarea {
    fn default() -> Self {
        EstadoTarea::Abierta
    }
}

impl EstadoTarea {
    /// ¿Es un estado terminal/parado (`Hecha`/`Cancelada`/`Bloqueada`)? (#1/#2/#12)
    ///
    /// INTENCIÓN: centralizar la noción de "ya no está viva" para los tres usos del store:
    ///   - #1 cierre idempotente: si la tarea ya es terminal NO se re-timbra.
    ///   - #2 reabrir: al ir a `Abierta` DESDE terminal se limpia fin/duracion y se re-timbra.
    ///   - #12 huérfanas: `limpiar_vencidas` bloquea solo las NO terminales de un peer caído.
    /// `Bloqueada` cuenta como "no viva" (parada): no se mide tiempo ni se aprende mientras lo esté.
    #[must_use]
    pub fn es_terminal(self) -> bool {
        matches!(
            self,
            EstadoTarea::Hecha | EstadoTarea::Cancelada | EstadoTarea::Bloqueada
        )
    }
}

/// ¿Es válida la transición de estado `actual` → `nuevo`? Función PURA (sin reloj ni I/O).
///
/// Tabla (R3): `Abierta ↔ EnCurso`; cualquiera → `Bloqueada`/`Hecha`/`Cancelada`;
/// los terminales/parados (`Bloqueada`/`Hecha`/`Cancelada`) → `Abierta` (REABRIR).
/// Quedarse en el mismo estado es un no-op válido (idempotente). El broker valida con esto
/// ANTES de timbrar el cambio; la TUI puede usarla para decidir qué acciones ofrecer.
#[must_use]
pub fn transicion_valida(actual: EstadoTarea, nuevo: EstadoTarea) -> bool {
    use EstadoTarea::{Abierta, Bloqueada, Cancelada, EnCurso, Hecha};
    if actual == nuevo {
        return true; // idempotente: re-aplicar el mismo estado no es un error.
    }
    match (actual, nuevo) {
        // Flujo activo: empezar y volver a la cola.
        (Abierta, EnCurso) | (EnCurso, Abierta) => true,
        // Desde cualquier estado se puede bloquear, completar o cancelar.
        (_, Bloqueada) | (_, Hecha) | (_, Cancelada) => true,
        // Reabrir desde un estado parado/terminal vuelve al inicio del flujo.
        (Bloqueada | Hecha | Cancelada, Abierta) => true,
        _ => false,
    }
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
    /// Lo que la IA estimó al abrir la tarea (segundos). El real lo timbra el broker
    /// (`duracion_seg`); este es el "ingenuo" que se compara contra el real para aprender
    /// el factor de corrección (ADR-002). `#[serde(default)]` para compat con tareas viejas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimado_seg: Option<i64>,
    /// Estado del ciclo de vida (R2). Si falta en el JSON (tarea vieja), nace `Abierta`.
    #[serde(default)]
    pub estado: EstadoTarea,
    /// Motivo del bloqueo cuando `estado == Bloqueada` (R5). None en el resto de estados.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bloqueo_motivo: Option<String>,
    /// Número de la issue de GitHub espejo, si la integración está activa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    /// IDEMPOTENCIA DEL APRENDIZAJE (#3). `true` cuando esta tarea YA contribuyó al factor de
    /// corrección. El broker solo aprende si `!factor_aprendido` y lo pone a `true` al aprender,
    /// en el mismo `tarea_guardar` atómico. Así reabrir+recerrar o un doble `cerrar_tarea` (ACK
    /// perdido) NO mete el mismo ratio dos veces en la EMA ni infla `muestras` (falsa confianza).
    /// `#[serde(default)]` → una tarea vieja nace `false` (aún no aprendida).
    #[serde(default)]
    pub factor_aprendido: bool,
    /// PRUEBA DE TRABAJO (#7). Evidencia opcional del cierre (commit SHA, PR, nota). Las tareas
    /// Hechas SIN evidencia se consideran "no-verificadas": alimentan el factor POR-PEER pero NO
    /// el global (no contaminan la métrica que Max confía como objetiva). `None` = sin evidencia.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidencia: Option<String>,
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
    /// Estimado ingenuo de la IA en segundos. None = la IA no estimó (no contamina el factor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimado_seg: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespuestaAbrirTarea {
    pub tarea_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    /// Estimado ya corregido por el factor aprendido (`estimado_seg / factor`), si la IA
    /// envió un estimado. None si no estimó. Es lo que el peer usa para reajustar su plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimado_corregido_seg: Option<i64>,
    /// El factor vigente en el momento de abrir (cuánto infla la IA históricamente).
    pub factor: f64,
    /// Nº de muestras que sostienen ese factor (poca confianza si es bajo).
    pub muestras: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionReportarTarea {
    pub tarea_id: String,
    pub texto: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionCerrarTarea {
    pub tarea_id: String,
    /// PRUEBA DE TRABAJO (#7). Evidencia opcional del cierre (commit SHA, PR, nota). Si falta,
    /// la tarea queda "no-verificada" y NO alimenta el factor global (solo el por-peer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidencia: Option<String>,
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
// APRENDIZAJE DE ESTIMACIÓN — factor de corrección global por media móvil (ADR-002).
// ===========================================================================

/// Peso de la media móvil exponencial (EMA): cuánto pesa el ratio RECIENTE frente al
/// histórico al actualizar el factor. 0.3 = el nuevo dato mueve el factor un 30% hacia él.
pub const FACTOR_ALPHA: f64 = 0.3;

/// Cota inferior del factor: la IA nunca "subestima" tanto como para corregir por debajo
/// de 0.5x (un outlier en esa dirección no hunde el factor).
pub const FACTOR_MIN: f64 = 0.5;

/// Cota superior del factor: un único outlier monstruoso (ratio 120) no enloquece el factor;
/// se clampa a 50x. Es el techo del "cuánto infla" que el sistema acepta aprender.
pub const FACTOR_MAX: f64 = 50.0;

/// Factor de corrección global aprendido de la diferencia estimado/real (ADR-002).
///
/// `factor` = cuánto INFLA la IA: `real ≈ estimado / factor`. Nace en 1.0 (sin corrección)
/// con 0 muestras; cada tarea cerrada con estimado y real válidos lo mueve por media móvil.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorEstimacion {
    /// Cuántas tareas (con estimado + real válidos) contribuyeron al factor.
    pub muestras: u32,
    /// El factor actual. `real ≈ estimado / factor`. Clamp a [FACTOR_MIN, FACTOR_MAX].
    pub factor: f64,
    /// ISO 8601 de la última actualización, timbrado por el broker.
    pub actualizado_en: String,
}

/// Aplica un paso de media móvil exponencial al factor con el ratio de UNA tarea, y clampa.
///
/// INTENCIÓN: función PURA y testeable (sin reloj ni I/O) que encapsula EXACTAMENTE el núcleo
/// del aprendizaje: `nuevo = factor + ALPHA*(ratio - factor)`, recortado a [MIN, MAX]. El broker
/// la llama dentro de `actualizar_factor`; el reloj y la persistencia quedan fuera, aquí no.
#[must_use]
pub fn aplicar_media_movil(factor_actual: f64, ratio: f64) -> f64 {
    let nuevo = factor_actual + FACTOR_ALPHA * (ratio - factor_actual);
    nuevo.clamp(FACTOR_MIN, FACTOR_MAX)
}

/// Corrige un estimado ingenuo (segundos) dividiéndolo por el factor aprendido.
///
/// Es lo que se devuelve a la IA al crear una tarea: "dijiste 5d; según tu historial ~6h".
/// Redondea al entero más cercano y nunca devuelve menos de 1s (un estimado positivo no
/// colapsa a 0 por un factor alto). Factor <= 0 se trata como 1.0 (sin corrección) por
/// seguridad — no debería ocurrir por el clamp, pero evita división por cero/negativos.
#[must_use]
pub fn corregir_estimado(estimado_seg: i64, factor: f64) -> i64 {
    if estimado_seg <= 0 {
        return estimado_seg;
    }
    let f = if factor > 0.0 { factor } else { 1.0 };
    let corregido = (estimado_seg as f64 / f).round() as i64;
    corregido.max(1)
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

// ===========================================================================
// ENTREGA DURABLE + TRAZABILIDAD — peticiones de confirmación/reenvío/historial.
// ===========================================================================

/// `POST /confirmar` (bajo token). El cliente confirma el avance de estado de uno o más
/// mensajes que recibió. El broker timbra el estado con SU reloj (R1.2/R1.4). Permite
/// confirmar en lote para no hacer un round-trip por mensaje.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionConfirmar {
    pub ids: Vec<i64>,
    pub estado: EstadoMensaje,
}

/// `POST /admin/reenviar` (bajo token). Re-encola un mensaje del historial con un
/// `msgseq` fresco, estado `Enviado` y traza `reenviado_de`/`reenvios` (R2.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionReenviar {
    pub msg_id: i64,
}

/// `GET /admin/historial` (bajo token). Lista el historial durable de una cola, con
/// filtros opcionales por cursor (`desde`) y por `estado` (R2.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionHistorial {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desde: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estado: Option<EstadoMensaje>,
}

// ===========================================================================
// GESTIÓN DE TAREAS (jefe ↔ empleados IA) — peticiones de la pantalla Tareas (R4-R9).
// Todos bajo el middleware de token. Reusan jornada/tarea_guardar/enviar/crear-tarea.
// ===========================================================================

/// `POST /tarea/editar` (R4). Edita campos de una tarea (reusa `tarea_guardar`). Los campos
/// son opcionales: `None` = no tocar ese campo (parche parcial, no reemplazo total).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionEditarTarea {
    pub tarea_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descripcion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimado_seg: Option<i64>,
}

/// `POST /tarea/estado` (R5). Transiciona el estado de una tarea (validado con
/// `transicion_valida`). Si pasa a `Hecha`, el broker mide el real y aprende el factor;
/// si pasa a `Bloqueada`, guarda `motivo` en `Tarea::bloqueo_motivo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionEstadoTarea {
    pub tarea_id: String,
    pub estado: EstadoTarea,
    /// Motivo del bloqueo (solo relevante al pasar a `Bloqueada`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motivo: Option<String>,
    /// PRUEBA DE TRABAJO (#7). Evidencia opcional al pasar a `Hecha` (commit/PR/nota). Mismo
    /// significado que en `PeticionCerrarTarea`: sin evidencia → tarea "no-verificada".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidencia: Option<String>,
}

/// `POST /tarea/asignar` (R6). Crea una tarea asignada a un peer (reusa `/crear-tarea`) Y
/// le notifica por canal ("Tienes una tarea nueva: ..."). El dueño es `instancia_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionAsignarTarea {
    pub instancia_id: String,
    pub descripcion: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimado_seg: Option<i64>,
}

/// `POST /tarea/reasignar` (R7). Cambia el dueño de una tarea existente al peer
/// `nuevo_instancia_id` y le notifica.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionReasignarTarea {
    pub tarea_id: String,
    pub nuevo_instancia_id: String,
}

/// `POST /tarea/forzar` (R8). Empuja la tarea como `<channel>` a la sesión del peer dueño
/// (reusa `/enviar` con texto formateado). Es el "tócale el hombro" de Max.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionForzarTarea {
    pub tarea_id: String,
}

/// `GET /tarea/reportes` (R9). Pide el historial de reportes de progreso de una tarea
/// (`reportar_tarea` ya los guarda). El detalle de la TUI los muestra (AC5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeticionReportesTarea {
    pub tarea_id: String,
}

// ===========================================================================
// FASE 5 — SUPERVISOR: detección de ociosos / atascados / ghosteo.
// Trazabilidad PASIVA → ACTIVA: el broker evalúa y emite alertas (el tiempo lo
// mide el broker; aquí solo viven los tipos y la comparación temporal PURA).
// ===========================================================================

/// Tipo de condición anómala detectada por el supervisor (R2/R3/R4).
///
/// INTENCIÓN: enum cerrado (no "stringly typed") para que la TUI y el broker
/// compartan exactamente el mismo vocabulario de alerta. Serializa en minúsculas
/// para encajar con el resto del protocolo (`ocioso`/`atascado`/`ghosteo`).
///
/// DECISIÓN DE NOMBRES (#8/#10): las variantes de una sola palabra siguen `rename_all =
/// "lowercase"` (`ocioso`/`atascado`/`ghosteo`). Las NUEVAS variantes son compuestas; para no
/// emitir un ilegible `cierresospechoso`, llevan `#[serde(rename = ...)]` EXPLÍCITO en
/// snake_case (`cierre_sospechoso`/`cancelacion_excesiva`). Se elige snake_case (no la palabra
/// única) porque es el separador idiomático y deja el JSON legible; queda documentado aquí para
/// que el broker y la TUI usen exactamente esas cadenas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TipoAlerta {
    /// Peer VIVO sin tarea en curso desde hace > `UMBRAL_OCIOSO_SEG` (R2).
    Ocioso,
    /// Tarea abierta (sin `fin`) desde hace > `UMBRAL_ATASCO_SEG` sin reporte (R3).
    Atascado,
    /// Mensaje en `Leido` (no `Procesado`) desde hace > `UMBRAL_GHOSTEO_SEG` (R4).
    Ghosteo,
    /// Cierre con `real < UMBRAL_REAL_MINIMO_SEG`: tarea creada y cerrada casi al instante (#8).
    /// NO alimenta el factor (envenenaría hacia el techo) y dispara esta alerta. `sujeto` = id
    /// de la tarea. Serializa como `"cierre_sospechoso"`.
    #[serde(rename = "cierre_sospechoso")]
    CierreSospechoso,
    /// Ratio canceladas/total del peer por encima de `UMBRAL_CANCELACION` (#10): el peer esconde
    /// fallos en `Cancelada` para que el factor solo aprenda de sus aciertos. `sujeto` = id del
    /// peer. Serializa como `"cancelacion_excesiva"`.
    #[serde(rename = "cancelacion_excesiva")]
    CancelacionExcesiva,
}

/// Una alerta emitida por el supervisor hacia `cprs:alertas` (R5).
///
/// `sujeto` identifica QUÉ disparó la alerta (id de peer, id de tarea o id de mensaje
/// según el `tipo`); junto al `tipo` forma la clave de idempotencia del set de activas
/// (R7), para no re-alertar lo mismo cada 30s. `creada_en` lo timbra el broker (su reloj).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alerta {
    pub tipo: TipoAlerta,
    pub sujeto: String,
    pub detalle: String,
    pub creada_en: String, // ISO 8601, timbrado por el broker
}

/// Segundos sin tarea en curso para considerar OCIOSO a un peer vivo (R2, default 10min).
pub const UMBRAL_OCIOSO_SEG: i64 = 600;

/// Segundos con tarea abierta sin reporte para considerarla ATASCADA (R3, default 30min).
pub const UMBRAL_ATASCO_SEG: i64 = 1800;

/// Segundos en estado `Leido` sin pasar a `Procesado` para considerar GHOSTEO (R4, default 5min).
pub const UMBRAL_GHOSTEO_SEG: i64 = 300;

/// Tope de alertas retenidas en la LIST `cprs:alertas` (R5: últimas 50).
pub const MAX_ALERTAS: usize = 50;

// ===========================================================================
// HONESTIDAD DEL FACTOR — umbrales y rangos plausibles (#4/#8/#10).
// Constantes y funciones PURAS; el broker decide qué hacer con ellas (no contaminar
// el factor, emitir alerta), pero la regla de "qué es plausible/sospechoso" vive aquí.
// ===========================================================================

/// Real mínimo (segundos) para que un cierre alimente el factor (#8). Un cierre por debajo de
/// este umbral (tarea abierta y cerrada casi al instante, p.ej. un peer en bucle de tool-calls)
/// produce un ratio enorme que empuja el factor al techo `FACTOR_MAX`: NO se aprende y se emite
/// `TipoAlerta::CierreSospechoso`.
pub const UMBRAL_REAL_MINIMO_SEG: i64 = 30;

/// Estimado mínimo plausible (segundos) al crear/editar una tarea (#4). Por debajo se rechaza
/// con warning y NO alimenta el factor: protege contra la confusión minutos↔segundos (una IA
/// que manda `30` para una tarea de 30 min ensucia el factor con un ratio 60× equivocado).
pub const ESTIMADO_MIN_SEG: i64 = 30;

/// Estimado máximo plausible (segundos) al crear/editar una tarea (#4). 30 días. Por encima se
/// rechaza con warning (un estimado de meses es casi siempre un error de unidad/dedo).
pub const ESTIMADO_MAX_SEG: i64 = 2_592_000; // 30 días * 24h * 3600s

/// Umbral del ratio canceladas/total por peer (#10) para emitir `TipoAlerta::CancelacionExcesiva`.
/// Por encima de 0.4 (40% canceladas) se sospecha que el peer esconde fallos en `Cancelada` para
/// que el factor solo aprenda de sus aciertos (sesgo de supervivencia → corrección optimista).
pub const UMBRAL_CANCELACION: f64 = 0.4;

/// ¿Está el estimado (segundos) dentro del rango plausible `[ESTIMADO_MIN_SEG, ESTIMADO_MAX_SEG]`? (#4)
///
/// Función PURA. El handler `crear_tarea`/`editar_tarea` la usa en la frontera: si devuelve
/// `false`, rechaza el estimado con un warning y NO lo usa para aprender el factor. Inclusiva en
/// ambos extremos (un estimado de exactamente 30s o de exactamente 30 días es válido).
#[must_use]
pub fn estimado_en_rango(seg: i64) -> bool {
    (ESTIMADO_MIN_SEG..=ESTIMADO_MAX_SEG).contains(&seg)
}

/// ¿Es sospechosamente bajo el tiempo real de un cierre? (#8)
///
/// Función PURA: `true` si `real_seg < UMBRAL_REAL_MINIMO_SEG`. El broker, al cerrar una tarea,
/// la consulta: si es `true`, NO alimenta el factor y emite `TipoAlerta::CierreSospechoso`. Un
/// real negativo (reloj hacia atrás) también es sospechoso (cae por debajo del umbral).
#[must_use]
pub fn cierre_sospechoso(real_seg: i64) -> bool {
    real_seg < UMBRAL_REAL_MINIMO_SEG
}

/// ¿El ratio de canceladas del peer supera `UMBRAL_CANCELACION`? (#10)
///
/// Función PURA: `canceladas / total > UMBRAL_CANCELACION`. Con `total == 0` devuelve `false`
/// (sin tareas no hay nada que sospechar; además evita la división por cero). El broker la usa
/// sobre el conteo por peer para decidir si emite `TipoAlerta::CancelacionExcesiva`.
#[must_use]
pub fn cancelacion_excesiva(canceladas: u32, total: u32) -> bool {
    if total == 0 {
        return false;
    }
    (f64::from(canceladas) / f64::from(total)) > UMBRAL_CANCELACION
}

/// ¿La distancia temporal entre `desde_iso` y `ahora_iso` supera `umbral_seg`?
///
/// Función PURA (sin reloj ni I/O): el broker le pasa SU `ahora` y el timestamp del sujeto.
/// `true` solo si ambos parsean como RFC3339 y la diferencia (ahora - desde) supera el umbral.
///
/// DEGRADACIÓN (AC5): si alguno NO parsea, devuelve `false` — un timestamp corrupto NO
/// dispara una falsa alerta ni tumba el detector; el broker sigue con los demás sujetos.
/// Una diferencia negativa (reloj hacia atrás / desde futuro) tampoco supera el umbral.
#[must_use]
pub fn supera_umbral(desde_iso: &str, ahora_iso: &str, umbral_seg: i64) -> bool {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;
    let parse = |s: &str| OffsetDateTime::parse(s, &Rfc3339).ok();
    match (parse(desde_iso), parse(ahora_iso)) {
        (Some(desde), Some(ahora)) => (ahora - desde).whole_seconds() > umbral_seg,
        _ => false,
    }
}

/// ¿Una tarea abierta ya superó su estimado? (#14/R3 — orden "atascadas primero")
///
/// Función PURA (sin reloj ni I/O): el caller calcula el tiempo `transcurrido_seg` (ahora -
/// inicio) y pasa el `estimado_seg` de la tarea. `true` si la tarea, estando abierta, ya consumió
/// más tiempo del que estimó (`transcurrido > estimado`). Sirve para ordenar la vista global de
/// tareas poniendo primero las que se pasaron de su estimado (overrun).
///
/// DEGRADACIÓN: si la tarea no tiene estimado (`None`) o el estimado no es positivo, devuelve
/// `false` — sin un estimado válido no hay "overrun" que detectar (no se puede superar la nada).
/// El borde exacto (`transcurrido == estimado`) NO cuenta como overrun (estrictamente mayor).
#[must_use]
pub fn tarea_overrun_seg(transcurrido_seg: i64, estimado_seg: Option<i64>) -> bool {
    match estimado_seg {
        Some(est) if est > 0 => transcurrido_seg > est,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R1.2/AC2: un Mensaje con estado Procesado y todos los campos timbrados debe
    /// serializar y deserializar sin pérdida (roundtrip).
    #[test]
    fn mensaje_procesado_roundtrip() {
        let original = Mensaje {
            id: 42,
            de_id: "claudia".into(),
            para_id: "max".into(),
            texto: "hola".into(),
            enviado_en: "2026-06-29T10:00:00Z".into(),
            estado: EstadoMensaje::Procesado,
            entregado_en: Some("2026-06-29T10:00:01Z".into()),
            leido_en: Some("2026-06-29T10:00:02Z".into()),
            procesado_en: Some("2026-06-29T10:00:03Z".into()),
            intentos: 1,
            reenviado_de: Some(7),
            reenvios: 2,
        };
        let json = serde_json::to_string(&original).expect("serializar");
        let vuelta: Mensaje = serde_json::from_str(&json).expect("deserializar");
        assert_eq!(vuelta.id, original.id);
        assert_eq!(vuelta.estado, EstadoMensaje::Procesado);
        assert_eq!(vuelta.entregado_en.as_deref(), Some("2026-06-29T10:00:01Z"));
        assert_eq!(vuelta.leido_en.as_deref(), Some("2026-06-29T10:00:02Z"));
        assert_eq!(vuelta.procesado_en.as_deref(), Some("2026-06-29T10:00:03Z"));
        assert_eq!(vuelta.intentos, 1);
        assert_eq!(vuelta.reenviado_de, Some(7));
        assert_eq!(vuelta.reenvios, 2);
    }

    /// COMPAT: un JSON de un peer viejo (con `entregado:true`, sin los campos nuevos)
    /// debe deserializar sin romper. `estado` cae a `Enviado` por defecto, los Option a
    /// None, los contadores a 0, y `entregado` (campo desconocido) se ignora.
    #[test]
    fn mensaje_viejo_deserializa_con_defaults() {
        let json_viejo = r#"{
            "id": 1,
            "de_id": "a",
            "para_id": "b",
            "texto": "viejo",
            "enviado_en": "2026-01-01T00:00:00Z",
            "entregado": true
        }"#;
        let m: Mensaje = serde_json::from_str(json_viejo).expect("deser JSON viejo");
        assert_eq!(m.id, 1);
        assert_eq!(m.estado, EstadoMensaje::Enviado);
        assert!(m.entregado_en.is_none());
        assert!(m.leido_en.is_none());
        assert!(m.procesado_en.is_none());
        assert_eq!(m.intentos, 0);
        assert!(m.reenviado_de.is_none());
        assert_eq!(m.reenvios, 0);
    }

    /// El enum serializa en minúsculas (rename_all = "lowercase").
    #[test]
    fn estado_serializa_lowercase() {
        let j = serde_json::to_string(&EstadoMensaje::DeadLetter).expect("serializar");
        assert_eq!(j, "\"deadletter\"");
        let e: EstadoMensaje = serde_json::from_str("\"procesado\"").expect("deser");
        assert_eq!(e, EstadoMensaje::Procesado);
    }

    /// El rango refleja el avance monótono natural.
    #[test]
    fn rango_es_monotono() {
        assert!(EstadoMensaje::Enviado.rango() < EstadoMensaje::Entregado.rango());
        assert!(EstadoMensaje::Entregado.rango() < EstadoMensaje::Leido.rango());
        assert!(EstadoMensaje::Leido.rango() < EstadoMensaje::Procesado.rango());
    }

    // --- Aprendizaje de estimación (fórmula pura) ---

    /// AC4: un ratio extremo (120) con factor 1 NO enloquece el factor: el paso de media móvil
    /// daría 1 + 0.3*(120-1) = 36.7, que está dentro de [0.5,50] → 36.7. Un ratio aún mayor
    /// sí debe clampar al techo 50.
    #[test]
    fn ratio_extremo_se_clampa_al_techo() {
        // 1 + 0.3*(120-1) = 36.7 (todavía bajo el techo)
        let f1 = aplicar_media_movil(1.0, 120.0);
        assert!((f1 - 36.7).abs() < 1e-9, "esperaba 36.7, fue {f1}");
        // Desde un factor ya alto, un ratio enorme empuja por encima de 50 → clamp a 50.
        let f2 = aplicar_media_movil(45.0, 200.0); // 45 + 0.3*(200-45) = 91.5 → clamp 50
        assert!((f2 - FACTOR_MAX).abs() < 1e-9, "esperaba 50, fue {f2}");
    }

    /// El clamp inferior protege contra ratios diminutos: nunca baja de FACTOR_MIN.
    #[test]
    fn ratio_diminuto_se_clampa_al_piso() {
        // Desde 0.6, un ratio 0.1 baja a 0.6 + 0.3*(0.1-0.6)=0.45 → clamp 0.5.
        let f = aplicar_media_movil(0.6, 0.1);
        assert!((f - FACTOR_MIN).abs() < 1e-9, "esperaba 0.5, fue {f}");
    }

    /// AC1: una secuencia de ratios consistentes (la IA infla ~6x siempre) converge hacia 6.
    #[test]
    fn secuencia_converge_al_ratio_estable() {
        let mut factor = 1.0_f64;
        for _ in 0..50 {
            factor = aplicar_media_movil(factor, 6.0);
        }
        assert!((factor - 6.0).abs() < 0.01, "no convergió a 6: {factor}");
    }

    /// corregir_estimado: 5 días (432000s) con factor 6.2 ≈ 19h35m (~69677s). Coherente.
    #[test]
    fn corregir_estimado_es_coherente() {
        let corregido = corregir_estimado(432_000, 6.2);
        // 432000 / 6.2 = 69677.4 → 69677
        assert_eq!(corregido, 69_677);
        // Más infla el factor, más pequeño el corregido (monotonía).
        assert!(corregir_estimado(432_000, 50.0) < corregido);
        // Estimado 0 o negativo se devuelve tal cual (sin dividir).
        assert_eq!(corregir_estimado(0, 6.2), 0);
        // Factor <= 0 se trata como 1.0 (sin corrección), nunca panic por div/0.
        assert_eq!(corregir_estimado(100, 0.0), 100);
        // Nunca colapsa a 0 con estimado positivo y factor alto.
        assert!(corregir_estimado(10, 50.0) >= 1);
    }

    /// AC5/compat: una Tarea vieja (JSON sin `estimado_seg`) deserializa OK → None.
    #[test]
    fn tarea_vieja_sin_estimado_deserializa() {
        let json_viejo = r#"{
            "id": "tar-1",
            "instancia_id": "claudio",
            "sesion_id": "ses-1",
            "descripcion": "feature vieja",
            "inicio": "2026-01-01T00:00:00Z",
            "fin": null,
            "duracion_seg": null
        }"#;
        let t: Tarea = serde_json::from_str(json_viejo).expect("deser Tarea vieja");
        assert_eq!(t.id, "tar-1");
        assert!(t.estimado_seg.is_none());
        assert!(t.issue_number.is_none());
    }

    /// Roundtrip de FactorEstimacion: serializa y vuelve sin pérdida.
    #[test]
    fn factor_estimacion_roundtrip() {
        let original = FactorEstimacion {
            muestras: 23,
            factor: 6.2,
            actualizado_en: "2026-06-29T10:00:00Z".into(),
        };
        let json = serde_json::to_string(&original).expect("serializar");
        let vuelta: FactorEstimacion = serde_json::from_str(&json).expect("deserializar");
        assert_eq!(vuelta.muestras, 23);
        assert!((vuelta.factor - 6.2).abs() < 1e-9);
        assert_eq!(vuelta.actualizado_en, "2026-06-29T10:00:00Z");
    }

    // --- Supervisor (Fase 5): tipos y comparación temporal pura ---

    /// TipoAlerta serializa en minúsculas, igual que el resto del protocolo.
    #[test]
    fn tipo_alerta_serializa_lowercase() {
        assert_eq!(
            serde_json::to_string(&TipoAlerta::Ocioso).expect("serializar"),
            "\"ocioso\""
        );
        assert_eq!(
            serde_json::to_string(&TipoAlerta::Atascado).expect("serializar"),
            "\"atascado\""
        );
        let g: TipoAlerta = serde_json::from_str("\"ghosteo\"").expect("deser");
        assert_eq!(g, TipoAlerta::Ghosteo);
    }

    /// Roundtrip de Alerta: serializa y vuelve sin pérdida.
    #[test]
    fn alerta_roundtrip() {
        let original = Alerta {
            tipo: TipoAlerta::Ghosteo,
            sujeto: "msg:42".into(),
            detalle: "leído hace 7min sin procesar".into(),
            creada_en: "2026-06-29T15:00:00Z".into(),
        };
        let json = serde_json::to_string(&original).expect("serializar");
        let vuelta: Alerta = serde_json::from_str(&json).expect("deserializar");
        assert_eq!(vuelta.tipo, TipoAlerta::Ghosteo);
        assert_eq!(vuelta.sujeto, "msg:42");
        assert_eq!(vuelta.detalle, "leído hace 7min sin procesar");
        assert_eq!(vuelta.creada_en, "2026-06-29T15:00:00Z");
    }

    /// supera_umbral: una diferencia mayor que el umbral devuelve true; una menor, false.
    /// El borde exacto (diferencia == umbral) NO lo supera (estrictamente mayor).
    #[test]
    fn supera_umbral_compara_diferencia() {
        // 11min de diferencia (660s) supera el umbral de ocioso (600s).
        assert!(supera_umbral(
            "2026-06-29T15:00:00Z",
            "2026-06-29T15:11:00Z",
            UMBRAL_OCIOSO_SEG
        ));
        // 5min (300s) NO supera el de ocioso (600s).
        assert!(!supera_umbral(
            "2026-06-29T15:00:00Z",
            "2026-06-29T15:05:00Z",
            UMBRAL_OCIOSO_SEG
        ));
        // Borde exacto: 600s NO es > 600s.
        assert!(!supera_umbral(
            "2026-06-29T15:00:00Z",
            "2026-06-29T15:10:00Z",
            UMBRAL_OCIOSO_SEG
        ));
    }

    /// AC5/degradación: un timestamp corrupto NO dispara alerta (devuelve false), no panic.
    #[test]
    fn supera_umbral_degrada_si_no_parsea() {
        assert!(!supera_umbral("basura", "2026-06-29T15:11:00Z", UMBRAL_OCIOSO_SEG));
        assert!(!supera_umbral("2026-06-29T15:00:00Z", "no-es-fecha", UMBRAL_OCIOSO_SEG));
        assert!(!supera_umbral("", "", UMBRAL_OCIOSO_SEG));
    }

    /// Una diferencia negativa (sujeto en el futuro / reloj hacia atrás) no supera el umbral.
    #[test]
    fn supera_umbral_negativo_no_dispara() {
        assert!(!supera_umbral(
            "2026-06-29T15:11:00Z",
            "2026-06-29T15:00:00Z",
            UMBRAL_OCIOSO_SEG
        ));
    }

    // --- Gestión de tareas (R1-R5): estado, transiciones y compat ---

    /// R1: EstadoTarea serializa en minúsculas, igual que el resto del protocolo.
    #[test]
    fn estado_tarea_serializa_lowercase() {
        assert_eq!(
            serde_json::to_string(&EstadoTarea::EnCurso).expect("serializar"),
            "\"encurso\""
        );
        assert_eq!(
            serde_json::to_string(&EstadoTarea::Bloqueada).expect("serializar"),
            "\"bloqueada\""
        );
        let h: EstadoTarea = serde_json::from_str("\"hecha\"").expect("deser");
        assert_eq!(h, EstadoTarea::Hecha);
        let c: EstadoTarea = serde_json::from_str("\"cancelada\"").expect("deser");
        assert_eq!(c, EstadoTarea::Cancelada);
    }

    /// El Default de EstadoTarea es Abierta (sostiene el `#[serde(default)]` de `Tarea::estado`).
    #[test]
    fn estado_tarea_default_es_abierta() {
        assert_eq!(EstadoTarea::default(), EstadoTarea::Abierta);
    }

    /// R3: tabla de transiciones válidas. Abierta↔EnCurso; cualquiera→Bloqueada/Hecha/Cancelada;
    /// terminal/parado→Abierta (reabrir); mismo estado es idempotente.
    #[test]
    fn transiciones_validas_segun_tabla() {
        use EstadoTarea::{Abierta, Bloqueada, Cancelada, EnCurso, Hecha};
        // Flujo activo bidireccional.
        assert!(transicion_valida(Abierta, EnCurso));
        assert!(transicion_valida(EnCurso, Abierta));
        // Cualquiera → Bloqueada / Hecha / Cancelada.
        for actual in [Abierta, EnCurso, Bloqueada, Hecha, Cancelada] {
            assert!(transicion_valida(actual, Bloqueada), "{actual:?}→Bloqueada");
            assert!(transicion_valida(actual, Hecha), "{actual:?}→Hecha");
            assert!(transicion_valida(actual, Cancelada), "{actual:?}→Cancelada");
        }
        // Reabrir: parado/terminal → Abierta.
        assert!(transicion_valida(Bloqueada, Abierta));
        assert!(transicion_valida(Hecha, Abierta));
        assert!(transicion_valida(Cancelada, Abierta));
        // Idempotente: mismo estado.
        for e in [Abierta, EnCurso, Bloqueada, Hecha, Cancelada] {
            assert!(transicion_valida(e, e), "{e:?}→{e:?} debe ser idempotente");
        }
    }

    /// R3: transiciones INVÁLIDAS. No se puede ir de un terminal/parado directo a EnCurso
    /// (hay que reabrir a Abierta primero).
    #[test]
    fn transiciones_invalidas_rechazadas() {
        use EstadoTarea::{Bloqueada, Cancelada, EnCurso, Hecha};
        assert!(!transicion_valida(Bloqueada, EnCurso));
        assert!(!transicion_valida(Hecha, EnCurso));
        assert!(!transicion_valida(Cancelada, EnCurso));
    }

    /// AC6/R2 compat: una Tarea vieja (JSON sin `estado` ni `bloqueo_motivo`) deserializa
    /// con `estado = Abierta` y `bloqueo_motivo = None`, sin romper.
    #[test]
    fn tarea_vieja_sin_estado_deserializa_abierta() {
        let json_viejo = r#"{
            "id": "tar-1",
            "instancia_id": "claudio",
            "sesion_id": "ses-1",
            "descripcion": "feature vieja",
            "inicio": "2026-01-01T00:00:00Z",
            "fin": null,
            "duracion_seg": null
        }"#;
        let t: Tarea = serde_json::from_str(json_viejo).expect("deser Tarea vieja");
        assert_eq!(t.estado, EstadoTarea::Abierta);
        assert!(t.bloqueo_motivo.is_none());
    }

    /// Roundtrip de una Tarea bloqueada con motivo: serializa y vuelve sin pérdida.
    #[test]
    fn tarea_bloqueada_roundtrip() {
        let original = Tarea {
            id: "tar-9".into(),
            instancia_id: "claudia".into(),
            sesion_id: "ses-3".into(),
            descripcion: "migrar store".into(),
            inicio: "2026-06-29T10:00:00Z".into(),
            fin: None,
            duracion_seg: None,
            estimado_seg: Some(3600),
            estado: EstadoTarea::Bloqueada,
            bloqueo_motivo: Some("falta credencial Redis".into()),
            issue_number: Some(12),
            factor_aprendido: false,
            evidencia: None,
        };
        let json = serde_json::to_string(&original).expect("serializar");
        let vuelta: Tarea = serde_json::from_str(&json).expect("deserializar");
        assert_eq!(vuelta.estado, EstadoTarea::Bloqueada);
        assert_eq!(vuelta.bloqueo_motivo.as_deref(), Some("falta credencial Redis"));
        assert_eq!(vuelta.estimado_seg, Some(3600));
        assert_eq!(vuelta.issue_number, Some(12));
    }

    /// bloqueo_motivo None NO se serializa (skip_serializing_if), igual que el resto de Option.
    #[test]
    fn tarea_sin_motivo_omite_campo() {
        let t = Tarea {
            id: "t".into(),
            instancia_id: "i".into(),
            sesion_id: "s".into(),
            descripcion: "x".into(),
            inicio: "2026-06-29T10:00:00Z".into(),
            fin: None,
            duracion_seg: None,
            estimado_seg: None,
            estado: EstadoTarea::Abierta,
            bloqueo_motivo: None,
            issue_number: None,
            factor_aprendido: false,
            evidencia: None,
        };
        let json = serde_json::to_string(&t).expect("serializar");
        assert!(!json.contains("bloqueo_motivo"), "no debe emitir bloqueo_motivo: {json}");
        assert!(json.contains("\"estado\":\"abierta\""), "debe emitir estado: {json}");
        // evidencia None se omite (skip_serializing_if); factor_aprendido SÍ se emite (default).
        assert!(!json.contains("evidencia"), "no debe emitir evidencia None: {json}");
        assert!(json.contains("\"factor_aprendido\":false"), "debe emitir factor_aprendido: {json}");
    }

    // --- Honestidad del factor (#3/#4/#7/#8/#10): compat, rangos y sospecha ---

    /// #3/#7 compat: una Tarea vieja (JSON sin `factor_aprendido` ni `evidencia`) deserializa con
    /// `factor_aprendido = false` (aún no aprendida) y `evidencia = None`, sin romper.
    #[test]
    fn tarea_vieja_sin_factor_aprendido_ni_evidencia() {
        let json_viejo = r#"{
            "id": "tar-1",
            "instancia_id": "claudio",
            "sesion_id": "ses-1",
            "descripcion": "feature vieja",
            "inicio": "2026-01-01T00:00:00Z",
            "fin": null,
            "duracion_seg": null
        }"#;
        let t: Tarea = serde_json::from_str(json_viejo).expect("deser Tarea vieja");
        assert!(!t.factor_aprendido, "una tarea vieja nace no-aprendida");
        assert!(t.evidencia.is_none());
    }

    /// #7: una Tarea con evidencia hace roundtrip y la conserva.
    #[test]
    fn tarea_con_evidencia_roundtrip() {
        let original = Tarea {
            id: "tar-7".into(),
            instancia_id: "claudia".into(),
            sesion_id: "ses-2".into(),
            descripcion: "fix bug".into(),
            inicio: "2026-06-29T10:00:00Z".into(),
            fin: Some("2026-06-29T11:00:00Z".into()),
            duracion_seg: Some(3600),
            estimado_seg: Some(1800),
            estado: EstadoTarea::Hecha,
            bloqueo_motivo: None,
            issue_number: None,
            factor_aprendido: true,
            evidencia: Some("commit abc123".into()),
        };
        let json = serde_json::to_string(&original).expect("serializar");
        let vuelta: Tarea = serde_json::from_str(&json).expect("deserializar");
        assert!(vuelta.factor_aprendido);
        assert_eq!(vuelta.evidencia.as_deref(), Some("commit abc123"));
    }

    /// #8: las nuevas variantes de TipoAlerta serializan en snake_case explícito; las viejas
    /// siguen en minúsculas (palabra única).
    #[test]
    fn tipo_alerta_nuevas_variantes_snake_case() {
        assert_eq!(
            serde_json::to_string(&TipoAlerta::CierreSospechoso).expect("serializar"),
            "\"cierre_sospechoso\""
        );
        assert_eq!(
            serde_json::to_string(&TipoAlerta::CancelacionExcesiva).expect("serializar"),
            "\"cancelacion_excesiva\""
        );
        let c: TipoAlerta = serde_json::from_str("\"cierre_sospechoso\"").expect("deser");
        assert_eq!(c, TipoAlerta::CierreSospechoso);
        let e: TipoAlerta = serde_json::from_str("\"cancelacion_excesiva\"").expect("deser");
        assert_eq!(e, TipoAlerta::CancelacionExcesiva);
        // Las viejas no cambiaron.
        assert_eq!(
            serde_json::to_string(&TipoAlerta::Ocioso).expect("serializar"),
            "\"ocioso\""
        );
    }

    /// #4: `estimado_en_rango` acepta el rango plausible inclusivo y rechaza fuera.
    #[test]
    fn estimado_en_rango_acepta_plausible_rechaza_extremos() {
        assert!(estimado_en_rango(ESTIMADO_MIN_SEG)); // borde inferior inclusivo (30s)
        assert!(estimado_en_rango(ESTIMADO_MAX_SEG)); // borde superior inclusivo (30 días)
        assert!(estimado_en_rango(1800)); // 30 min
        assert!(!estimado_en_rango(ESTIMADO_MIN_SEG - 1)); // 29s: la confusión min↔seg típica
        assert!(!estimado_en_rango(ESTIMADO_MAX_SEG + 1)); // > 30 días
        assert!(!estimado_en_rango(0));
        assert!(!estimado_en_rango(-5)); // negativo nunca es plausible
    }

    /// #8: `cierre_sospechoso` marca los reales por debajo del umbral (incluido el borde).
    #[test]
    fn cierre_sospechoso_marca_reales_bajos() {
        assert!(cierre_sospechoso(0)); // cierre instantáneo
        assert!(cierre_sospechoso(2)); // 2s, el caso del bucle de tool-calls
        assert!(cierre_sospechoso(UMBRAL_REAL_MINIMO_SEG - 1)); // 29s
        assert!(cierre_sospechoso(-10)); // real negativo (reloj atrás) también sospechoso
        assert!(!cierre_sospechoso(UMBRAL_REAL_MINIMO_SEG)); // 30s justo NO es sospechoso
        assert!(!cierre_sospechoso(3600)); // 1h, normal
    }

    /// #10: `cancelacion_excesiva` dispara solo por encima del umbral; total 0 nunca dispara.
    #[test]
    fn cancelacion_excesiva_segun_umbral() {
        assert!(!cancelacion_excesiva(0, 0)); // sin tareas no hay sospecha (ni div/0)
        assert!(!cancelacion_excesiva(0, 10)); // 0% canceladas
        assert!(!cancelacion_excesiva(4, 10)); // 40% justo NO supera (estricto >)
        assert!(cancelacion_excesiva(5, 10)); // 50% supera 0.4
        assert!(cancelacion_excesiva(10, 10)); // 100% canceladas
    }

    // --- Vista global de tareas (#14/R3): orden "atascadas/overrun primero" ---

    /// #14/R3: `tarea_overrun_seg` marca como overrun una tarea cuyo transcurrido supera su
    /// estimado; el borde exacto NO cuenta (estricto >); por debajo del estimado no es overrun.
    #[test]
    fn tarea_overrun_compara_transcurrido_contra_estimado() {
        assert!(tarea_overrun_seg(3601, Some(3600))); // 1h01m > 1h estimada → overrun
        assert!(!tarea_overrun_seg(3600, Some(3600))); // borde exacto NO supera (estricto >)
        assert!(!tarea_overrun_seg(1800, Some(3600))); // 30min de 1h: aún dentro del estimado
    }

    /// #14/R3 degradación: sin estimado válido NO hay overrun (None / 0 / negativo → false).
    #[test]
    fn tarea_overrun_sin_estimado_valido_es_false() {
        assert!(!tarea_overrun_seg(99_999, None)); // sin estimado no se puede superar nada
        assert!(!tarea_overrun_seg(99_999, Some(0))); // estimado 0 no es positivo
        assert!(!tarea_overrun_seg(99_999, Some(-5))); // estimado negativo (basura) no dispara
    }
}
