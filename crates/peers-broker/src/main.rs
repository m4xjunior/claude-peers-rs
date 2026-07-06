//! peers-broker — daemon HTTP de la red claude-peers-rs (fase 2).
//!
//! Servidor axum+tokio. La persistencia está detrás del trait `Almacen` (peers-core):
//! por defecto `AlmacenRedis`; con `--features sqlite`, `AlmacenSqlite` (binario 100%
//! autocontenido). El broker habla SOLO con `dyn Almacen` — agnóstico del motor.
//!
//! Añade la jornada medida por el broker (el reloj es del servidor, la IA nunca estima)
//! y, si hay token, la integración con GitHub Issues (degradación graciosa: si falta o
//! falla, el broker opera igual).
//!
//! No entra en pánico en producción: anyhow en `main`, handlers devuelven `Result`.

mod bitacora;
mod github;
mod jornada;
mod store;
#[cfg(feature = "sqlite")]
mod db;

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use clap::Parser;
use github::GitHub;
use peers_core::{
    corregir_estimado, supera_umbral, AccionRegistrada, Alerta, Almacen, BloqueoComunicacion,
    ColaResumen, DecisionPolitica, EstadoMensaje, EstadoTarea,
    FactorEstimacion, Mensaje, PeticionAbrirTarea, PeticionAcciones, PeticionAsignarTarea,
    PeticionCerrarTarea,
    PeticionConfirmar, PeticionDefinirResumen, PeticionEditarTarea, PeticionEnviar,
    PeticionEstadoTarea, PeticionForzarTarea, PeticionHistorial, PeticionJornada, PeticionLatido,
    PeticionListar, PeticionPurgar, PeticionReasignarTarea, PeticionRecibir, PeticionRegistrar,
    PeticionReenviar, PeticionReportarTarea, PeticionReportesTarea, PeticionResolverAlerta,
    PeticionSalir, Politica,
    RespuestaAbrirTarea, RespuestaAdminInfo, RespuestaAdminRedis, RespuestaEnviar,
    RespuestaJornada, RespuestaOk, RespuestaRegistrar, RespuestaSalud, Tarea, TipoAccion,
    TipoAlerta, ID_OPERADOR, PUERTO_DEFECTO, VENCIMIENTO_MS,
};
use store::AlmacenRedis;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tracing::{error, info, warn};

// NO derivamos Debug: el campo `token` es secreto y un `{args:?}` accidental (log, panic
// con backtrace) lo filtraría en claro. Implementamos Debug manual redactando el token.
#[derive(Parser)]
#[command(name = "peers-broker", about = "Daemon de la red claude-peers-rs")]
struct Args {
    #[arg(long, env = "CLAUDE_PEERS_PORT", default_value_t = PUERTO_DEFECTO)]
    puerto: u16,

    #[arg(long, env = "CLAUDE_PEERS_HOST", default_value = "127.0.0.1")]
    host: String,

    /// URL del Redis (backend por defecto). Namespace cprs: dentro de esa instancia.
    #[arg(long, env = "CLAUDE_PEERS_REDIS_URL", default_value = "redis://127.0.0.1:6379")]
    redis_url: String,

    /// Ruta del archivo SQLite (solo con --features sqlite).
    #[arg(long, env = "CLAUDE_PEERS_DB")]
    db: Option<String>,

    /// Token de acceso. Si se setea, el broker exige el header `X-Peers-Token` en todas las
    /// rutas salvo /salud. Sin token → sin auth (uso local localhost sigue funcionando igual).
    #[arg(long, env = "CLAUDE_PEERS_TOKEN")]
    token: Option<String>,

    /// Umbral OCIOSO en segundos: peer vivo sin tarea desde hace > X → alerta (R2/R8).
    #[arg(long, env = "CLAUDE_PEERS_UMBRAL_OCIOSO", default_value_t = peers_core::UMBRAL_OCIOSO_SEG)]
    umbral_ocioso: i64,

    /// Umbral ATASCO en segundos: tarea abierta sin reporte desde hace > X → alerta (R3/R8).
    #[arg(long, env = "CLAUDE_PEERS_UMBRAL_ATASCO", default_value_t = peers_core::UMBRAL_ATASCO_SEG)]
    umbral_atasco: i64,

    /// Umbral GHOSTEO en segundos: mensaje en Leido sin Procesado desde hace > X → alerta (R4/R8).
    #[arg(long, env = "CLAUDE_PEERS_UMBRAL_GHOSTEO", default_value_t = peers_core::UMBRAL_GHOSTEO_SEG)]
    umbral_ghosteo: i64,

    /// Ruta del fichero SQLite de la BITÁCORA de acciones (ADR-001). Independiente del backend
    /// (funciona igual con Redis y con --db). Default: ~/.config/claude-peers/bitacora.db.
    #[arg(long, env = "CLAUDE_PEERS_BITACORA_DB")]
    bitacora_db: Option<String>,

    /// Anti-spoofing del `de_id` (E-10): si está activo, `/enviar` RESUELVE el emisor real desde el
    /// secreto de sesión (header `X-Peers-Secreto`) y sobrescribe el `de_id` declarado. Si está
    /// desactivado (DEFAULT), el broker vuelve a CONFIAR en el `de_id` del payload tal cual (estado
    /// pre-E-10) — reabre el vector de suplantación, decisión explícita de Max (2026-07-06). El
    /// código de E-10 queda intacto: reactivar es solo poner esta flag a true y recargar. `/registrar`
    /// sigue emitiendo secreto en ambos modos (inofensivo si no se usa; permite reactivar sin re-registro).
    #[arg(long, env = "CLAUDE_PEERS_ANTISPOOF", default_value_t = false)]
    antispoofing: bool,
}

/// Umbrales del supervisor (R8): configurables vía env/flags del broker, con los defaults
/// de `peers-core`. Se resuelven una vez en `main` y se mueven al spawn periódico.
#[derive(Debug, Clone, Copy)]
struct Umbrales {
    ocioso_seg: i64,
    atasco_seg: i64,
    ghosteo_seg: i64,
}

impl std::fmt::Debug for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Args")
            .field("puerto", &self.puerto)
            .field("host", &self.host)
            .field("redis_url", &self.redis_url)
            .field("db", &self.db)
            // El token NUNCA se imprime en claro: se redacta a "<presente>"/"<ninguno>".
            .field("token", &self.token.as_ref().map(|_| "<presente>").unwrap_or("<ninguno>"))
            .field("bitacora_db", &self.bitacora_db)
            .field("antispoofing", &self.antispoofing)
            .finish()
    }
}

/// Estado de la aplicación: el almacén (tras el trait), el cliente GitHub opcional y los
/// datos de escucha (host/puerto) que el panel de admin reporta en /admin/info.
struct EstadoApp {
    almacen: Arc<dyn Almacen>,
    github: Option<GitHub>,
    host: String,
    puerto: u16,
    /// Lock que serializa el REGISTRO de instancias. INTENCIÓN: `resolver_id_sin_colision`
    /// (lee el índice) y `almacen.registrar` (escribe) son dos pasos; sin este lock, dos
    /// `claude` arrancando a la vez en el MISMO directorio (mismo id base, ambos vivos) leen
    /// ambos "id libre" ANTES de que ninguno escriba (TOCTOU) → se registran con el MISMO id y
    /// se pisan la cola. Con el lock, el segundo ve al primero ya escrito y se sufija (-2, -3…),
    /// que es justo el comportamiento deseado: varias instancias por dir, filtrables por nombre.
    /// El broker es un único proceso, así que un Mutex async basta (no hace falta lock distribuido).
    registro_lock: tokio::sync::Mutex<()>,
    /// Política de comunicación VIGENTE en memoria (RFC politica-comunicacion R9): se carga del
    /// almacén UNA vez al arrancar y se refresca en caliente en `POST /admin/politica`. `enviar`
    /// la evalúa AQUÍ, nunca contra el store (es la ruta caliente). `RwLock` porque las lecturas
    /// (cada envío) dominan a las escrituras (ediciones de Max). Regla de oro del proyecto: el
    /// guard JAMÁS se sostiene a través de un `.await` — se lee/escribe y se suelta.
    politica: tokio::sync::RwLock<Politica>,
    /// Bitácora de acciones (RFC registro-acciones / ADR-001): fichero SQLite propio vía SQLx,
    /// transversal a ambos backends. `None` = desactivada (no se pudo abrir el fichero): el
    /// broker opera igual — la bitácora es observabilidad, nunca condición de negocio (R9).
    bitacora: Option<bitacora::Bitacora>,
    /// Anti-spoofing del `de_id` (E-10) activo o no. `false` (default) = el broker confía en el
    /// `de_id` del payload (pre-E-10); `true` = resuelve el emisor por el secreto de sesión. Es el
    /// interruptor de la reversión (decisión de Max): apagado devuelve el comportamiento anterior
    /// sin borrar el código de E-10.
    antispoofing: bool,
}

type Estado = Arc<EstadoApp>;

fn ahora_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

fn vencidas_antes_iso() -> String {
    (OffsetDateTime::now_utc() - time::Duration::milliseconds(VENCIMIENTO_MS))
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

/// Error → 500 JSON. Evita el panic en los handlers.
struct ErrorApp(anyhow::Error);

impl IntoResponse for ErrorApp {
    fn into_response(self) -> axum::response::Response {
        error!("error en handler: {:#}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ErrorApp {
    fn from(e: E) -> Self {
        ErrorApp(e.into())
    }
}

/// Genera un id corto sin colisión local (entropía: nanos + pid, xorshift).
fn generar_id() -> String {
    let ahora = OffsetDateTime::now_utc().unix_timestamp_nanos() as u64;
    let mut x = ahora ^ (std::process::id() as u64).wrapping_mul(0x9E3779B97F4A7C15);
    const ALFABETO: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut id = String::with_capacity(8);
    for _ in 0..8 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        id.push(ALFABETO[(x % ALFABETO.len() as u64) as usize] as char);
    }
    id
}

/// Genera un secreto de sesión (E-10, anti-spoofing del `de_id`): `SECRETO_BYTES` bytes de un
/// CSPRNG del SO (`getrandom`) codificados a base62 imprimible (apto para un header HTTP). A
/// diferencia de `generar_id` (xorshift sembrado por nanos+pid, PREDECIBLE — vale para un id que
/// NO es secreto), aquí la entropía DEBE ser criptográfica: un secreto adivinable no protege de
/// nada. Si `getrandom` fallara (entorno sin fuente de entropía, extremadamente raro), degradamos
/// al sembrado débil de `generar_id` repetido — es un fallback de disponibilidad, no de seguridad;
/// se registra un `warn` porque un secreto débil es una anomalía que Max debe ver.
fn generar_secreto() -> String {
    let mut bytes = [0u8; peers_core::SECRETO_BYTES];
    match getrandom::getrandom(&mut bytes) {
        Ok(()) => peers_core::codificar_secreto(&bytes),
        Err(err) => {
            warn!("getrandom falló al generar el secreto de sesión ({err}); uso fallback DÉBIL");
            // Fallback de disponibilidad: dos ids concatenados (16 chars xorshift). NO es seguro,
            // pero mantiene el broker operativo; el warn deja rastro de la degradación.
            format!("{}{}", generar_id(), generar_id())
        }
    }
}

/// Decide si una petición está autorizada. Pura y testeable.
/// - Sin token configurado (None) → siempre autorizado (compat local).
/// - Con token configurado → autorizado solo si el recibido coincide exacto.
fn token_autorizado(configurado: Option<&str>, recibido: Option<&str>) -> bool {
    match configurado {
        None => true,
        Some(esperado) => recibido == Some(esperado),
    }
}

/// ¿El host de escucha es loopback (solo accesible desde la propia máquina)?
/// Cubre IPv4 (127.0.0.0/8), IPv6 (::1) y el nombre "localhost". Cualquier otro host
/// implica exposición en red → el caller exige token o avisa. Pura y testeable.
fn host_es_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// Middleware axum: aplica `token_autorizado` usando el header `X-Peers-Token`.
/// /salud queda exento (se monta fuera de esta capa).
async fn verificar_token(
    axum::extract::State(token): axum::extract::State<Option<String>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let recibido = req
        .headers()
        .get("x-peers-token")
        .and_then(|v| v.to_str().ok());
    if token_autorizado(token.as_deref(), recibido) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "token inválido o ausente").into_response()
    }
}

// --- Handlers fase 1 ---

async fn salud(State(e): State<Estado>) -> Result<Json<RespuestaSalud>, ErrorApp> {
    Ok(Json(RespuestaSalud {
        estado: "ok".into(),
        instancias: e.almacen.contar_instancias().await?,
    }))
}

/// Resuelve el id de registro evitando colisión entre peers DISTINTOS.
///
/// Caso re-registro legítimo (mismo peer reiniciando): el id pedido ya existe pero su PID
/// coincide con el que registra, o el PID viejo ya no está vivo → se reusa el id tal cual,
/// para HEREDAR la cola (es el FIX del id estable). Caso colisión real (dos peers vivos con
/// el mismo id, p.ej. dos Claude en la misma carpeta sin --id): el id pedido existe con OTRO
/// PID que sigue vivo → se sufija -2, -3… hasta encontrar uno libre. Sin id_preferido →
/// id aleatorio (sin colisión posible).
async fn resolver_id_sin_colision(
    e: &Estado,
    id_preferido: Option<&str>,
    pid_nuevo: i64,
    host_nuevo: &str,
) -> String {
    let Some(base) = id_preferido else {
        return generar_id();
    };
    // ¿El id base está ocupado por un peer DISTINTO y vivo? (distingue local vs remoto por host)
    if !id_ocupado_por_otro_vivo(e, base, pid_nuevo, host_nuevo).await {
        return base.to_string();
    }
    // Colisión real: busca el primer sufijo libre (-2, -3, …). Cota defensiva en 99.
    for n in 2..=99 {
        let candidato = format!("{base}-{n}");
        if !id_ocupado_por_otro_vivo(e, &candidato, pid_nuevo, host_nuevo).await {
            warn!("id '{base}' ocupado por otro peer vivo; esta instancia usa '{candidato}'");
            return candidato;
        }
    }
    // Improbable (99 colisiones): cae a aleatorio para no bloquear el registro.
    generar_id()
}

/// true si `id` ya está registrado por OTRO peer que sigue vivo → colisión real (hay que sufijar).
/// false si está libre o es un re-registro del mismo peer (reusa el id para heredar la cola).
///
/// CROSS-HOST (fix 2026-06-30): el broker solo puede comprobar `pid_vivo` para peers de SU
/// PROPIA máquina (kill -0). Para un peer remoto el PID no existe localmente y se tomaría por
/// "muerto" → DOS sesiones remotas distintas del mismo dir pisaban el mismo id (bug real: 3
/// sesiones del server registradas como "aistudio", robándose la cola). Ahora distinguimos por
/// hostname:
///   - Mismo host que el registrante → es local: la liveness por PID es válida.
///   - Host distinto (remoto) → el PID es inverificable aquí; lo tratamos como VIVO si el latido
///     es reciente (no vencido). Así dos sesiones remotas distintas SÍ colisionan y se sufijan.
///   - hostname vacío en ambos lados (clients viejos) → degradación a la lógica previa (solo PID).
async fn id_ocupado_por_otro_vivo(
    e: &Estado,
    id: &str,
    pid_nuevo: i64,
    host_nuevo: &str,
) -> bool {
    match e.almacen.instancia_obtener(id).await {
        Ok(Some(inst)) => {
            // Re-registro del MISMO peer (mismo host + mismo pid) → NO es colisión: hereda la cola.
            let mismo_host = !host_nuevo.is_empty()
                && !inst.hostname.is_empty()
                && inst.hostname == host_nuevo;
            if mismo_host && inst.pid == pid_nuevo {
                return false;
            }
            // Host conocido y DISTINTO → peer remoto distinto: colisión si su latido sigue fresco.
            // RFC3339 con misma zona (UTC) es comparable lexicográficamente: visto_en >= límite
            // de vencimiento ⇒ latido reciente ⇒ el otro peer remoto sigue vivo ⇒ colisión real.
            if !host_nuevo.is_empty() && !inst.hostname.is_empty() && inst.hostname != host_nuevo {
                return inst.visto_en.as_str() >= vencidas_antes_iso().as_str();
            }
            // Mismo host (o host desconocido): la verificación por PID local es válida/única opción.
            inst.pid != pid_nuevo && pid_vivo(inst.pid)
        }
        // No existe, o error de lectura → tratamos como libre (no bloqueamos el registro).
        _ => false,
    }
}

/// Comprueba si un PID está vivo en esta máquina (señal 0, no mata). Solo válido para peers
/// locales; un peer remoto (cross-host) tiene un PID que aquí no existe → se trataría como
/// "muerto", lo cual es aceptable: cross-host se distingue mejor por id de rol explícito.
fn pid_vivo(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    // kill(pid, 0): no envía señal, solo comprueba existencia/permisos del proceso.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

async fn registrar(
    State(e): State<Estado>,
    Json(p): Json<PeticionRegistrar>,
) -> Result<Json<RespuestaRegistrar>, ErrorApp> {
    // Sección crítica: resolver el id (lee el índice) + escribir el registro deben ser atómicos
    // frente a otros registros concurrentes. Sin el lock hay TOCTOU (dos peers mismo dir → mismo
    // id). El guard se sostiene hasta el final de la escritura del índice de instancias.
    let _guard = e.registro_lock.lock().await;
    let id = resolver_id_sin_colision(&e, p.id_preferido.as_deref(), p.pid, &p.hostname).await;
    let ahora = ahora_iso();
    // E-10: secreto de sesión CSPRNG para atar el `de_id` a esta instancia. Se genera en cada
    // registro (alta y re-registro rotan credencial) y se devuelve UNA vez al peer, que lo
    // presenta luego en el header `X-Peers-Secreto`. El store lo persiste; nunca sale en /listar.
    let secreto = generar_secreto();
    let es_alta = e
        .almacen
        .registrar(
            &id,
            p.pid,
            &p.hostname,
            &p.directorio,
            p.repo_git.as_deref(),
            p.repo_github.as_deref(),
            p.tty.as_deref(),
            &p.resumen,
            &ahora,
            &secreto,
        )
        .await?;
    // FIX sesiones fantasma: en un RE-registro (peer reconectando — SSH/VPN inestable, el mismo
    // proceso perdió el latido y volvió), `abrir_sesion` se llamaba SIEMPRE sin cerrar la sesión
    // anterior, que quedaba huérfana para siempre (`fin=None`, `duracion_seg=None`) — inflaba el
    // contador de "sesiones" y dejaba el "total trabajado" en 0s (ninguna sesión cerrada aporta
    // duración). `cerrar_sesion` es idempotente/no-op si no hay ninguna sesión abierta de este id
    // (busca la última con `fin.is_none()`, no falla si no encuentra), así que es seguro llamarla
    // SIEMPRE antes de abrir la nueva — cierra la huérfana con el momento del re-registro como su
    // fin real (mejor aproximación disponible: no sabemos cuándo se cayó la conexión, pero "ahora"
    // acota el error a como mucho la ventana entre latidos perdidos).
    if !es_alta {
        jornada::cerrar_sesion(&e.almacen, &id, &ahora).await?;
    }
    // Abre una sesión de jornada para esta instancia (timbrada por el broker).
    jornada::abrir_sesion(&e.almacen, &format!("ses-{id}-{ahora}"), &id, &ahora).await?;
    // Bitácora (#2/#8): el alta de una instancia NUEVA deja rastro — antes, ni local ni remoto
    // generaban este evento (no existía `TipoAccion::Registrar`). El re-registro NO se registra
    // aquí a propósito (sería ruido: un peer con latido inestable re-registraría cada pocos
    // segundos, inundando su propia bitácora con "altas" que en realidad son la MISMA presencia).
    if es_alta {
        registrar_accion(&e, &id, TipoAccion::Registrar, None, Some(p.hostname.clone()), None, None).await;
    }
    info!("instancia registrada: {id}");
    Ok(Json(RespuestaRegistrar { id, secreto: Some(secreto) }))
}

async fn latido(
    State(e): State<Estado>,
    Json(p): Json<PeticionLatido>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    e.almacen.latido(&p.id, &ahora_iso()).await?;
    Ok(Json(RespuestaOk { ok: true }))
}

async fn definir_resumen(
    State(e): State<Estado>,
    Json(p): Json<PeticionDefinirResumen>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    e.almacen.definir_resumen(&p.id, &p.resumen).await?;
    // Bitácora (R4): el nuevo resumen (recortado) como detalle — el historial de resúmenes
    // que hoy se pierde (solo quedaba el último en la instancia).
    registrar_accion(
        &e,
        &p.id,
        TipoAccion::DefinirResumen,
        None,
        Some(recortar_detalle(&p.resumen, 120)),
        None,
        None,
    )
    .await;
    Ok(Json(RespuestaOk { ok: true }))
}

async fn listar(
    State(e): State<Estado>,
    Json(p): Json<PeticionListar>,
) -> Result<Json<Vec<peers_core::Instancia>>, ErrorApp> {
    let r = e
        .almacen
        .listar(
            p.alcance,
            &p.directorio,
            p.repo_git.as_deref(),
            p.excluir_id.as_deref(),
            &vencidas_antes_iso(),
        )
        .await?;
    Ok(Json(r))
}

/// Ruta por defecto del fichero de bitácora: `~/.config/claude-peers/bitacora.db` (junto a la
/// config de la TUI). Sin HOME (entorno raro) cae al directorio de trabajo — nunca panic.
fn ruta_bitacora_defecto() -> String {
    match std::env::var("HOME") {
        Ok(home) => format!("{home}/.config/claude-peers/bitacora.db"),
        Err(_) => "bitacora.db".to_string(),
    }
}

/// Registra una acción en la bitácora (RFC registro-acciones R4/R5/R9). El `cuando` lo timbra
/// el broker AQUÍ (regla sagrada). SIEMPRE best-effort: con la bitácora desactivada es un no-op
/// y un fallo del INSERT se queda en `warn!` — la mutación de negocio ya ocurrió y JAMÁS se
/// revierte ni se propaga error al cliente por culpa de la observabilidad.
///
/// `quien` (R5): el emisor real del payload cuando existe (`de_id`/`id`/instancia dueña); las
/// acciones de los paneles del operador (asignar/reasignar/editar/estado/forzar/purgar/
/// alerta-resolver — payloads SIN identidad de actor) se atribuyen a `ID_OPERADOR`, el id
/// reservado unificado con la política de comunicación.
async fn registrar_accion(
    e: &Estado,
    quien: &str,
    accion: TipoAccion,
    sujeto: Option<String>,
    detalle: Option<String>,
    tarea: Option<&Tarea>,
    evidencia: Option<String>,
) {
    let Some(b) = &e.bitacora else { return };
    let a = AccionRegistrada {
        quien: quien.to_string(),
        accion,
        sujeto,
        detalle,
        cuando: ahora_iso(),
        evidencia,
    };
    if let Err(err) = b.registrar(&a, tarea).await {
        warn!("no se pudo registrar la acción {accion:?} de '{quien}' (la mutación ya ocurrió): {err:#}");
    }
}

/// Forma de wire (lowercase) de un `EstadoTarea` para el `detalle` de la bitácora — la misma
/// cadena que emite serde en el protocolo.
fn estado_tarea_texto(e: EstadoTarea) -> String {
    serde_json::to_string(&e)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

/// Recorta un texto libre a `max` caracteres para el campo `detalle` de la bitácora (evita
/// eventos kilométricos; el contenido completo vive donde siempre — mensaje/reporte/resumen).
fn recortar_detalle(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        texto.to_string()
    } else {
        let corto: String = texto.chars().take(max).collect();
        format!("{corto}…")
    }
}

/// Evalúa la política de comunicación para un envío (RFC politica-comunicacion R2/R3/R9).
///
/// Lee la copia EN MEMORIA (`RwLock`) — nunca el store en la ruta caliente (R9) — y suelta el
/// guard en la misma expresión: la evaluación es síncrona y la decisión se devuelve por valor,
/// así el lock jamás cruza un `.await` (regla Rust del proyecto).
///
/// DECISIÓN R5 (documentada para que no sorprenda): la política se evalúa SOLO en `/enviar`.
/// Los demás caminos que encolan (`tarea/forzar`, `tarea/asignar`, `tarea/reasignar`,
/// `admin/reenviar`) son acciones del OPERADOR y escriben con `de_id = "broker"` → exentos por
/// R3 igualmente (forzar una tarea nunca se bloquea: es Max actuando). NOTA de alcance: la
/// exención confía en el `de_id` DECLARADO por el cliente; hacerlo no-suplantable es el fix
/// aparte de STATE.md (registro atómico / id reservado), fuera de esta feature.
async fn evaluar_politica(e: &Estado, de_id: &str, para_id: &str) -> DecisionPolitica {
    e.politica.read().await.evaluar(de_id, para_id)
}

async fn enviar(
    State(e): State<Estado>,
    headers: axum::http::HeaderMap,
    Json(mut p): Json<PeticionEnviar>,
) -> Result<Json<RespuestaEnviar>, ErrorApp> {
    // E-10 (anti-spoofing, decisión C de Max): el broker NUNCA confía en el `de_id` del payload.
    // Toma el secreto del header `X-Peers-Secreto`, RESUELVE la identidad real (búsqueda inversa
    // secreto→id) y SOBRESCRIBE `p.de_id` con ella. Así declararse `operador`/otro es inofensivo:
    // el `de_id` declarado nunca tiene efecto. Se hace ANTES de la política (que consume `de_id`).
    //
    // REVERSIÓN (decisión de Max, 2026-07-06): si el anti-spoofing está DESACTIVADO (flag
    // `CLAUDE_PEERS_ANTISPOOF` off, default), se SALTA todo este bloque y el broker confía en el
    // `de_id` declarado tal cual (comportamiento pre-E-10). El código de abajo queda intacto para
    // reactivarlo con solo poner la flag. NOTA: apagado reabre el vector de suplantación a sabiendas.
    if e.antispoofing {
    let presentado = headers
        .get(peers_core::HEADER_SECRETO)
        .and_then(|v| v.to_str().ok());
    let id_del_secreto = match presentado {
        Some(s) => e.almacen.id_por_secreto(s).await?,
        None => None,
    };
    match peers_core::resolver_emisor(presentado, id_del_secreto) {
        peers_core::ResolucionEmisor::Resuelto(id_real) => {
            // Corrección silenciosa (decisión C): el de_id real es el dueño del secreto. Si venía
            // desalineado, lo dejamos trazado en debug (no es un error de negocio, es lo esperado).
            if p.de_id != id_real {
                info!(
                    "E-10: de_id declarado '{}' corregido a '{}' (resuelto por secreto)",
                    p.de_id, id_real
                );
            }
            p.de_id = id_real;
        }
        peers_core::ResolucionEmisor::Invalido => {
            // Se presentó un secreto que no es de ninguna instancia: no hay emisor real que
            // resolver. RECHAZO (ok:false), respuesta de negocio, nunca un 500. Queda trazado.
            warn!("ENVÍO RECHAZADO (E-10): secreto presentado no corresponde a ninguna instancia → '{}'", p.para_id);
            return Ok(Json(RespuestaEnviar {
                ok: false,
                error: Some("identidad no verificada: el secreto de sesión no corresponde a ninguna instancia".to_string()),
            }));
        }
        peers_core::ResolucionEmisor::SinCredencial => {
            // Ventana de compat: sin header (client pre-E-10, o un panel del operador que hoy no
            // se registra con secreto). No hay secreto que resolver → se respeta el `de_id`
            // declarado, como HOY. Se cierra cuando toda la flota reconecte (migración por
            // re-registro). Rastro solo para remitentes no-exentos (los paneles son esperados).
            if !peers_core::remitente_exento(&p.de_id) {
                warn!(
                    "envío de '{}' sin secreto (client pre-E-10): se respeta el de_id declarado en compat",
                    p.de_id
                );
            }
        }
    }
    } // fin `if e.antispoofing` — con la flag OFF, `p.de_id` queda tal como llegó (pre-E-10).
    // No descartar en silencio: si el destino no existe, error claro.
    if !e.almacen.instancia_existe(&p.para_id).await? {
        return Ok(Json(RespuestaEnviar {
            ok: false,
            error: Some(format!("La instancia '{}' no existe", p.para_id)),
        }));
    }
    // Política de comunicación (R4): tras el chequeo de existencia y ANTES de encolar. Un
    // bloqueo es una respuesta de NEGOCIO (`ok:false` con motivo), nunca un 500: el emisor la
    // ve y decide. El mensaje NO se encola.
    if let DecisionPolitica::Bloqueada { motivo } =
        evaluar_politica(&e, &p.de_id, &p.para_id).await
    {
        // R7: el intento queda trazado (best-effort: si el registro falla, el bloqueo sigue
        // siendo la respuesta — un warn, jamás un error hacia el emisor).
        let bloqueo = BloqueoComunicacion {
            de_id: p.de_id.clone(),
            para_id: p.para_id.clone(),
            motivo: motivo.clone(),
            cuando: ahora_iso(),
        };
        if let Err(err) = e.almacen.registrar_bloqueo(&bloqueo).await {
            warn!("no se pudo registrar el bloqueo de política (se responde igual): {err:#}");
        }
        info!("envío bloqueado por política: '{}' → '{}' ({motivo})", p.de_id, p.para_id);
        return Ok(Json(RespuestaEnviar {
            ok: false,
            error: Some(format!("comunicación bloqueada por política: {motivo}")),
        }));
    }
    e.almacen
        .encolar_mensaje(&p.de_id, &p.para_id, &p.texto, &ahora_iso())
        .await?;
    // Bitácora (R4/R5): el emisor declarado es el actor. Sin el texto del mensaje (vive en el
    // historial de la cola); el sujeto es el destinatario.
    registrar_accion(
        &e,
        &p.de_id,
        TipoAccion::EnviarMensaje,
        Some(p.para_id.clone()),
        None,
        None,
        None,
    )
    .await;
    Ok(Json(RespuestaEnviar { ok: true, error: None }))
}

async fn recibir(
    State(e): State<Estado>,
    Json(p): Json<PeticionRecibir>,
) -> Result<Json<peers_core::RespuestaRecibir>, ErrorApp> {
    let mensajes = e.almacen.recibir_mensajes(&p.id).await?;
    Ok(Json(peers_core::RespuestaRecibir { mensajes }))
}

async fn salir(
    State(e): State<Estado>,
    Json(p): Json<PeticionSalir>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    // Cierra la jornada (timbrada por el broker) antes de dar de baja.
    jornada::cerrar_sesion(&e.almacen, &p.id, &ahora_iso()).await?;
    e.almacen.salir(&p.id).await?;
    // Bitácora (R4): la baja queda en el histórico DURABLE aunque la instancia desaparezca
    // del registro efímero (ese es justo el punto del ADR-001).
    registrar_accion(&e, &p.id, TipoAccion::Kick, None, None, None, None).await;
    Ok(Json(RespuestaOk { ok: true }))
}

// --- Handlers de jornada (fase 2) ---

/// Abre una tarea timbrada por el broker, persistiendo el `estimado_seg` (si vino) y, si la
/// integración GitHub está activa, cosiendo la issue espejo en el repo dinámico del peer.
/// Devuelve `(tarea, issue_number)`. Lógica compartida por `/tarea/abrir` y `/crear-tarea`
/// (no se duplica). Degradación graciosa: si GitHub falla o el peer no tiene repo, la tarea
/// sigue local. El estimado NO se corrige aquí (eso lo decide cada handler con el factor).
async fn abrir_tarea_con_estimado(
    e: &Estado,
    p: &PeticionAbrirTarea,
    ahora: &str,
) -> Result<(Tarea, Option<u64>), ErrorApp> {
    // #4: valida el rango plausible del estimado ANTES de crear nada. Rechaza la confusión
    // min↔seg típica (p.ej. 30 = 30s para una tarea de 30 min) con un mensaje claro, en vez de
    // dejar que envenene el factor con un ratio 60×. Solo valida si vino estimado.
    if let Some(estimado) = p.estimado_seg {
        if !peers_core::estimado_en_rango(estimado) {
            return Err(ErrorApp(anyhow::anyhow!(
                "estimado_seg={estimado} fuera de rango plausible [{}s, {}s] (~30s a 30 días). \
                 Recuerda: el estimado va en SEGUNDOS (1800 = 30 min).",
                peers_core::ESTIMADO_MIN_SEG,
                peers_core::ESTIMADO_MAX_SEG
            )));
        }
    }

    // #15/R5: id global atómico vía INCR cprs:tareaseq (espejo de cprs:msgseq) → `tar-{seq}`.
    // Dos crear_tarea simultáneas del mismo peer NUNCA colisionan. Degradación graciosa: si el
    // contador fallara (Redis caído), caemos al esquema viejo `tar-{inst}-{ahora}` para no perder
    // la tarea — sigue siendo legible por `tarea_obtener` (compat AC5).
    let tarea_id = match e.almacen.siguiente_tarea_seq().await {
        Ok(seq) => format!("tar-{seq}"),
        Err(err) => {
            tracing::warn!("siguiente_tarea_seq falló ({err:#}); uso id por fecha como fallback");
            format!("tar-{}-{ahora}", p.instancia_id)
        }
    };
    // La sesión activa es la abierta más reciente de la instancia.
    let (sesiones, _) = e.almacen.jornada(&p.instancia_id).await?;
    let sesion_id = sesiones
        .iter()
        .rev()
        .find(|s| s.fin.is_none())
        .map(|s| s.id.clone())
        .unwrap_or_else(|| format!("ses-{}-{ahora}", p.instancia_id));

    let mut tarea = jornada::abrir_tarea(
        &e.almacen,
        &tarea_id,
        &p.instancia_id,
        &sesion_id,
        &p.descripcion,
        ahora,
    )
    .await?;

    // Persiste el estimado ingenuo de la IA en la tarea (R1). Tarea sin estimado → None (no
    // contamina el factor al cerrar). Re-guarda solo si vino estimado.
    if p.estimado_seg.is_some() {
        tarea.estimado_seg = p.estimado_seg;
        e.almacen.tarea_guardar(&tarea).await?;
    }

    // Integración GitHub: cose el issue_number en la tarea (mi fatia, no la de Aluísio).
    // Degradación graciosa: si GH no está o falla, la tarea sigue local.
    let mut issue_number = None;
    if let Some(gh) = &e.github {
        // Repo DINÁMICO: el owner/repo sale del repo_github de la instancia DUEÑA de la tarea
        // (el repo donde ese peer trabaja). Si la instancia no tiene repo_github → degradación:
        // no creamos issue, la tarea sigue local.
        match repo_de_instancia(e, &p.instancia_id).await {
            Some((owner, repo)) => {
                let labels = match &p.area {
                    Some(a) => vec![p.instancia_id.clone(), a.clone()],
                    None => vec![p.instancia_id.clone()],
                };
                match gh.crear_issue(&owner, &repo, &p.descripcion, &labels).await {
                    Ok(n) => {
                        issue_number = Some(n);
                        tarea.issue_number = Some(n);
                        // Re-guarda la tarea con el número de issue cosido.
                        if let Err(err) = e.almacen.tarea_guardar(&tarea).await {
                            warn!("no se pudo guardar issue_number en la tarea: {err:#}");
                        }
                    }
                    Err(err) => warn!("GitHub no creó la issue en {owner}/{repo} (se sigue local): {err:#}"),
                }
            }
            None => warn!(
                "instancia {} sin repo_github (dir sin repo GitHub): se sigue local sin issue",
                p.instancia_id
            ),
        }
    }

    Ok((tarea, issue_number))
}

async fn tarea_abrir(
    State(e): State<Estado>,
    Json(p): Json<PeticionAbrirTarea>,
) -> Result<Json<RespuestaAbrirTarea>, ErrorApp> {
    let ahora = ahora_iso();
    let (tarea, issue_number) = abrir_tarea_con_estimado(&e, &p, &ahora).await?;

    // Bitácora (R4/R5): la creación se atribuye al peer dueño (abre su propia tarea).
    registrar_accion(
        &e,
        &p.instancia_id,
        TipoAccion::CrearTarea,
        Some(tarea.id.clone()),
        None,
        Some(&tarea),
        None,
    )
    .await;

    // El estimado corregido + factor/muestras se calculan en `/crear-tarea` (el endpoint de las
    // tools MCP). `/tarea/abrir` es el legacy y devuelve neutros para no cambiar su contrato.
    Ok(Json(RespuestaAbrirTarea {
        tarea_id: tarea.id,
        issue_number,
        estimado_corregido_seg: None,
        factor: 1.0,
        muestras: 0,
    }))
}

async fn tarea_reportar(
    State(e): State<Estado>,
    Json(p): Json<PeticionReportarTarea>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    // Persiste el report en el historial local SIEMPRE (R9/AC5): el detalle de la TUI lo lee de
    // aquí, no de GitHub. El reloj lo pone el broker.
    let ahora = ahora_iso();
    e.almacen.tarea_reportar(&p.tarea_id, &p.texto, &ahora).await?;
    // Bitácora (R4/R5): el reporte se atribuye al dueño de la tarea (las tools MCP reportan
    // sobre tareas propias). Refetch barato — ruta fría; degrada si la tarea ya no está.
    if let Some(t) = e.almacen.tarea_obtener(&p.tarea_id).await? {
        registrar_accion(
            &e,
            &t.instancia_id,
            TipoAccion::ReportarTarea,
            Some(t.id.clone()),
            Some(recortar_detalle(&p.texto, 120)),
            Some(&t),
            None,
        )
        .await;
    }
    // Si la tarea tiene issue y hay GitHub, comenta también. Degrada si falla.
    if let Some(gh) = &e.github {
        if let Some(tarea) = e.almacen.tarea_obtener(&p.tarea_id).await? {
            if let Some(n) = tarea.issue_number {
                // Mismo repo dinámico: el de la instancia dueña de la tarea.
                if let Some((owner, repo)) = repo_de_instancia(&e, &tarea.instancia_id).await {
                    if let Err(err) = gh.comentar_issue(&owner, &repo, n, &p.texto).await {
                        warn!("GitHub no comentó la issue (no crítico): {err:#}");
                    }
                }
            }
        }
    }
    Ok(Json(RespuestaOk { ok: true }))
}

/// Aprendizaje unificado del factor al llegar una tarea a `Hecha` (#1/#3/#7/#8/#9). UN solo punto
/// para las dos vías de cierre (`/cerrar-tarea` y `/tarea/estado`→Hecha): así no divergen las
/// reglas. El real lo timbró SIEMPRE el broker (regla sagrada): se lee de `tarea.duracion_seg`.
///
/// Gates (en orden):
///   #3 idempotencia → solo aprende si `!tarea.factor_aprendido` (una tarea aporta UNA vez).
///   estimado/real válidos → estimado presente y > 0, real presente y > 0.
///   #4 rango → el estimado debe estar en `[ESTIMADO_MIN_SEG, ESTIMADO_MAX_SEG]` (no envenenar con min↔seg).
///   #8 cierre sospechoso → si `cierre_sospechoso(real)` (real < UMBRAL_REAL_MINIMO_SEG): NO aprende
///      y emite `TipoAlerta::CierreSospechoso` (sujeto = tarea_id).
///   #9 por peer → SIEMPRE aprende el factor del peer (`actualizar_factor_peer`).
///   #7 evidencia → SOLO si la tarea trae `evidencia`, además alimenta el factor GLOBAL. Sin
///      evidencia, la tarea es "no-verificada": contribuye solo al factor del peer, no al global.
///
/// Al aprender (no sospechoso) marca `factor_aprendido = true` y lo persiste (idempotencia #3).
/// Degrada: cualquier fallo de almacén deja la tarea Hecha igual (warn, sin panic).
async fn aprender_factor_de_tarea(e: &Estado, tarea: &Tarea, ahora: &str) {
    let (Some(estimado), Some(real)) = (tarea.estimado_seg, tarea.duracion_seg) else {
        return;
    };
    if estimado <= 0 || real <= 0 {
        return;
    }

    // #3: una tarea ya aprendida no vuelve a contar (reabrir+recerrar o doble cierre).
    if tarea.factor_aprendido {
        return;
    }

    // #4: estimado fuera de rango plausible no alimenta el factor (confusión min↔seg).
    if !peers_core::estimado_en_rango(estimado) {
        warn!(
            "tarea {} con estimado {}s fuera de rango plausible: NO alimenta el factor (#4)",
            tarea.id, estimado
        );
        return;
    }

    // #8: cierre instantáneo (real < UMBRAL_REAL_MINIMO_SEG) → alerta + NO aprende.
    if peers_core::cierre_sospechoso(real) {
        let alerta = Alerta {
            tipo: TipoAlerta::CierreSospechoso,
            sujeto: tarea.id.clone(),
            detalle: format!(
                "tarea '{}' de '{}' cerrada en {}s (< {}s): cierre sospechoso, NO alimenta el factor",
                tarea.id, tarea.instancia_id, real, peers_core::UMBRAL_REAL_MINIMO_SEG
            ),
            creada_en: ahora.to_string(),
        };
        match e.almacen.alerta_emitir(&alerta).await {
            Ok(true) => info!("alerta CIERRE_SOSPECHOSO emitida: {}", tarea.id),
            Ok(false) => {}
            Err(err) => warn!("no se pudo emitir alerta de cierre sospechoso: {err:#}"),
        }
        return;
    }

    let ratio = estimado as f64 / real as f64;

    // #9: SIEMPRE aprende el factor del peer (un mentiroso solo se corrige a sí mismo).
    match e.almacen.actualizar_factor_peer(&tarea.instancia_id, ratio, ahora).await {
        Ok(f) => info!(
            "factor PEER '{}' aprendido: tarea {} ratio {:.2} → factor {:.2} ({} muestras)",
            tarea.instancia_id, tarea.id, ratio, f.factor, f.muestras
        ),
        Err(err) => warn!("no se pudo actualizar el factor por peer (tarea cerrada igual): {err:#}"),
    }

    // #7: solo con evidencia (prueba de trabajo) alimenta también el factor GLOBAL. Sin evidencia,
    // la tarea es "no-verificada" y no contamina el número que Max confía para TODOS los peers.
    if tarea.evidencia.is_some() {
        match e.almacen.actualizar_factor(ratio, ahora).await {
            Ok(f) => info!(
                "factor GLOBAL aprendido (con evidencia): tarea {} ratio {:.2} → factor {:.2} ({} muestras)",
                tarea.id, ratio, f.factor, f.muestras
            ),
            Err(err) => warn!("no se pudo actualizar el factor global (tarea cerrada igual): {err:#}"),
        }
    } else {
        info!(
            "tarea {} sin evidencia: no-verificada, alimenta solo el factor por peer (#7)",
            tarea.id
        );
    }

    // #3: marca la tarea como aprendida para que no vuelva a contar.
    let mut aprendida = tarea.clone();
    aprendida.factor_aprendido = true;
    if let Err(err) = e.almacen.tarea_guardar(&aprendida).await {
        warn!("no se pudo marcar factor_aprendido en la tarea {} (puede recontar): {err:#}", tarea.id);
    }
}

/// Cierra una tarea (el broker mide el real con SU reloj), aprende el factor (#1/#3/#7/#8/#9) y
/// cierra la issue espejo en GitHub si procede. Lógica compartida por `/tarea/cerrar` y
/// `/cerrar-tarea` (no se duplica). Devuelve la tarea ya cerrada.
///
/// `evidencia`: prueba de trabajo opcional (#7). Si viene, se persiste en la tarea ANTES de
/// aprender, para que el gate de evidencia la vea y alimente también el factor global.
async fn cerrar_tarea_y_aprender(
    e: &Estado,
    tarea_id: &str,
    evidencia: Option<&str>,
    ahora: &str,
) -> Result<Tarea, ErrorApp> {
    let mut tarea = jornada::cerrar_tarea(&e.almacen, tarea_id, ahora).await?;

    // #7: persiste la evidencia (prueba de trabajo) antes de aprender. Solo si vino y la tarea
    // aún no la tenía (no la borramos en reintentos idempotentes sin evidencia).
    if let Some(ev) = evidencia {
        if tarea.evidencia.as_deref() != Some(ev) {
            tarea.evidencia = Some(ev.to_string());
            if let Err(err) = e.almacen.tarea_guardar(&tarea).await {
                warn!("no se pudo persistir la evidencia de la tarea {tarea_id}: {err:#}");
            }
        }
    }

    aprender_factor_de_tarea(e, &tarea, ahora).await;

    if let Some(gh) = &e.github {
        if let Some(n) = tarea.issue_number {
            // Mismo repo dinámico: el de la instancia dueña de la tarea.
            if let Some((owner, repo)) = repo_de_instancia(e, &tarea.instancia_id).await {
                let cierre = format!(
                    "Tarea cerrada. Duración medida por el broker: {}s.",
                    tarea.duracion_seg.unwrap_or(0)
                );
                if let Err(err) = gh.cerrar_issue(&owner, &repo, n, Some(&cierre)).await {
                    warn!("GitHub no cerró la issue (no crítico): {err:#}");
                }
            }
        }
    }
    Ok(tarea)
}

async fn tarea_cerrar(
    State(e): State<Estado>,
    Json(p): Json<PeticionCerrarTarea>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    let tarea = cerrar_tarea_y_aprender(&e, &p.tarea_id, p.evidencia.as_deref(), &ahora_iso()).await?;
    // Bitácora (R4/R5/#11): el cierre se atribuye al dueño (las tools MCP cierran tareas propias).
    // La evidencia YA llegaba en el payload (`p.evidencia`, persistida en `Tarea.evidencia` desde
    // #7) pero se descartaba aquí — #11 es justamente conectar ese último tramo a la bitácora.
    registrar_accion(
        &e,
        &tarea.instancia_id,
        TipoAccion::CerrarTarea,
        Some(tarea.id.clone()),
        None,
        Some(&tarea),
        p.evidencia.clone(),
    )
    .await;
    Ok(Json(RespuestaOk { ok: true }))
}

// --- Handlers de tareas autogestionadas + aprendizaje (los que usan las tools MCP) ---
//
// `/crear-tarea`, `/cerrar-tarea`, `/listar-tareas`, `/factor-estimacion`. Reutilizan la
// jornada/fichaje existente (abrir_tarea_con_estimado / cerrar_tarea_y_aprender) y el factor
// del almacén. Degradan: si algo falla, devuelven error 500 JSON, nunca panic.

/// `POST /crear-tarea` { instancia_id, descripcion, area?, estimado_seg? }. Abre la tarea
/// (reusa la jornada), persiste el estimado, lee el factor vigente y devuelve el estimado YA
/// corregido (`estimado / factor`) + factor + muestras, para que el peer reajuste su plan en
/// vivo (R6/R8/AC2). Si no vino estimado, `estimado_corregido_seg` es None.
async fn crear_tarea(
    State(e): State<Estado>,
    Json(p): Json<PeticionAbrirTarea>,
) -> Result<Json<RespuestaAbrirTarea>, ErrorApp> {
    let ahora = ahora_iso();
    let (tarea, issue_number) = abrir_tarea_con_estimado(&e, &p, &ahora).await?;

    // Bitácora (R4/R5): la creación se atribuye al peer dueño (tool MCP crear_tarea).
    registrar_accion(
        &e,
        &p.instancia_id,
        TipoAccion::CrearTarea,
        Some(tarea.id.clone()),
        None,
        Some(&tarea),
        None,
    )
    .await;

    // #9: usa el factor del PEER si tiene historial propio (muestras > 0); si no, cae al GLOBAL
    // como fallback. Así un peer con sesgo conocido se corrige con SU número, y uno nuevo hereda
    // el del equipo. Ambas lecturas degradan a neutro (factor 1.0, 0 muestras) — nunca panic.
    let factor_peer = e.almacen.factor_estimacion_peer(&p.instancia_id).await?;
    let factor = if factor_peer.muestras > 0 {
        factor_peer
    } else {
        e.almacen.factor_estimacion().await?
    };
    let estimado_corregido_seg = p
        .estimado_seg
        .map(|estimado| corregir_estimado(estimado, factor.factor));

    Ok(Json(RespuestaAbrirTarea {
        tarea_id: tarea.id,
        issue_number,
        estimado_corregido_seg,
        factor: factor.factor,
        muestras: factor.muestras,
    }))
}

/// `POST /cerrar-tarea` { tarea_id }. Cierra la tarea (el broker mide el real), aprende el
/// factor si procede (R3/R9) y cierra la issue espejo. Idempotencia: si la tarea no existe,
/// `cerrar_tarea` devuelve error → 500 JSON (el cliente degrada).
async fn cerrar_tarea(
    State(e): State<Estado>,
    Json(p): Json<PeticionCerrarTarea>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    let tarea = cerrar_tarea_y_aprender(&e, &p.tarea_id, p.evidencia.as_deref(), &ahora_iso()).await?;
    // Bitácora (R4/R5/#11): el cierre se atribuye al dueño (las tools MCP cierran tareas propias).
    // La evidencia YA llegaba en el payload (`p.evidencia`, persistida en `Tarea.evidencia` desde
    // #7) pero se descartaba aquí — #11 es justamente conectar ese último tramo a la bitácora.
    registrar_accion(
        &e,
        &tarea.instancia_id,
        TipoAccion::CerrarTarea,
        Some(tarea.id.clone()),
        None,
        Some(&tarea),
        p.evidencia.clone(),
    )
    .await;
    Ok(Json(RespuestaOk { ok: true }))
}

// --- Handlers de gestión interactiva de tareas (R4–R9): el jefe (TUI) dirige a sus peers ---
//
// Cuelgan de `rutas_protegidas` (token). Reusan lo existente: `tarea_editar`/`tarea_estado`/
// `tarea_obtener`/`tarea_guardar` del almacén, `abrir_tarea_con_estimado` (= /crear-tarea),
// `encolar_mensaje` (= /enviar) y `cerrar_tarea_y_aprender` (aprendizaje del factor). Nunca
// reinventan. Degradan: notificaciones best-effort (warn + sigue), 404 si la tarea no existe.

/// Formatea segundos a "~XhYY" / "~XXm" / "~XXs" para los textos de notificación. Solo presentación.
fn humanizar_estimado(seg: i64) -> String {
    if seg <= 0 {
        return "sin estimar".to_string();
    }
    let h = seg / 3600;
    let m = (seg % 3600) / 60;
    if h > 0 {
        format!("~{h}h{m:02}")
    } else if m > 0 {
        format!("~{m}m")
    } else {
        format!("~{seg}s")
    }
}

/// `POST /tarea/editar` { tarea_id, descripcion?, estimado_seg? } → edita metadatos (R4/AC1).
/// Parche parcial: `None` no toca el campo. 404 si la tarea no existe. NO toca estado/tiempos/factor.
async fn tarea_editar(
    State(e): State<Estado>,
    Json(p): Json<PeticionEditarTarea>,
) -> Result<Json<Tarea>, ErrorApp> {
    // #4: si la edición trae un estimado, valida el rango plausible (mismo gate que al crear).
    if let Some(estimado) = p.estimado_seg {
        if !peers_core::estimado_en_rango(estimado) {
            return Err(ErrorApp(anyhow::anyhow!(
                "estimado_seg={estimado} fuera de rango plausible [{}s, {}s] (~30s a 30 días). \
                 El estimado va en SEGUNDOS (1800 = 30 min).",
                peers_core::ESTIMADO_MIN_SEG,
                peers_core::ESTIMADO_MAX_SEG
            )));
        }
    }
    match e
        .almacen
        .tarea_editar(&p.tarea_id, p.descripcion.as_deref(), p.estimado_seg)
        .await?
    {
        Some(tarea) => {
            // Bitácora (R4/R5): edición desde los paneles del operador (payload sin actor).
            registrar_accion(
                &e,
                ID_OPERADOR,
                TipoAccion::EditarTarea,
                Some(tarea.id.clone()),
                None,
                Some(&tarea),
                None,
            )
            .await;
            Ok(Json(tarea))
        }
        None => Err(ErrorApp(anyhow::anyhow!(
            "la tarea '{}' no existe",
            p.tarea_id
        ))),
    }
}

/// `POST /tarea/estado` { tarea_id, estado, motivo? } → transiciona (R5/AC2). El store valida
/// con `transicion_valida` y timbra (Hecha → mide real con el reloj del broker; Bloqueada → motivo).
/// SOLO al pasar a `Hecha` con estimado+real válidos se aprende el factor (Cancelada/otros NO lo
/// contaminan): reusa la misma lógica de `cerrar_tarea_y_aprender`. Responde la tarea ya actualizada.
async fn tarea_estado(
    State(e): State<Estado>,
    Json(p): Json<PeticionEstadoTarea>,
) -> Result<Json<Tarea>, ErrorApp> {
    let ahora = ahora_iso();
    let Some(mut tarea) = e
        .almacen
        .tarea_estado(&p.tarea_id, p.estado, p.motivo.as_deref(), &ahora)
        .await?
    else {
        return Err(ErrorApp(anyhow::anyhow!(
            "la tarea '{}' no existe",
            p.tarea_id
        )));
    };

    // Aprendizaje del factor: SOLO en Hecha (R3/R5/AC2). El real lo timbró el broker en
    // `tarea_estado` (regla sagrada): se lee de la tarea, nunca del cliente. Cancelada/Bloqueada/
    // etc. NO tocan el factor. La lógica (gates #3/#4/#7/#8/#9) vive en `aprender_factor_de_tarea`,
    // exactamente igual que en `/cerrar-tarea` (una sola semántica de aprendizaje, #1).
    if p.estado == EstadoTarea::Hecha {
        // #7: persiste la evidencia (prueba de trabajo) antes de aprender, si vino.
        if let Some(ev) = p.evidencia.as_deref() {
            if tarea.evidencia.as_deref() != Some(ev) {
                tarea.evidencia = Some(ev.to_string());
                if let Err(err) = e.almacen.tarea_guardar(&tarea).await {
                    warn!("no se pudo persistir la evidencia de la tarea {}: {err:#}", p.tarea_id);
                }
            }
        }

        aprender_factor_de_tarea(&e, &tarea, &ahora).await;

        // Cierra la issue espejo en GitHub si procede (best-effort, igual que /cerrar-tarea).
        if let Some(gh) = &e.github {
            if let Some(n) = tarea.issue_number {
                if let Some((owner, repo)) = repo_de_instancia(&e, &tarea.instancia_id).await {
                    let cierre = format!(
                        "Tarea marcada Hecha. Duración medida por el broker: {}s.",
                        tarea.duracion_seg.unwrap_or(0)
                    );
                    if let Err(err) = gh.cerrar_issue(&owner, &repo, n, Some(&cierre)).await {
                        warn!("GitHub no cerró la issue (no crítico): {err:#}");
                    }
                }
            }
        }
    }

    // Bitácora (R4/R5): transición desde los paneles del operador; el estado destino va como
    // detalle para que el feed lea "cambiar_estado_tarea → hecha" sin abrir la tarea.
    registrar_accion(
        &e,
        ID_OPERADOR,
        TipoAccion::CambiarEstadoTarea,
        Some(tarea.id.clone()),
        Some(estado_tarea_texto(p.estado)),
        Some(&tarea),
        None,
    )
    .await;

    Ok(Json(tarea))
}

/// `POST /tarea/asignar` { instancia_id, descripcion, estimado_seg? } → crea una tarea asignada
/// a un peer (reusa `abrir_tarea_con_estimado`, igual que /crear-tarea) Y le notifica por canal
/// (R6/AC4). Responde `{ tarea_id }`. La notificación es best-effort: si el peer no existe, la
/// tarea ya quedó creada (la verá al listar) — degradación graciosa.
async fn tarea_asignar(
    State(e): State<Estado>,
    Json(p): Json<PeticionAsignarTarea>,
) -> Result<Json<serde_json::Value>, ErrorApp> {
    let ahora = ahora_iso();
    let peticion = PeticionAbrirTarea {
        instancia_id: p.instancia_id.clone(),
        descripcion: p.descripcion.clone(),
        area: None,
        estimado_seg: p.estimado_seg,
    };
    let (tarea, _issue) = abrir_tarea_con_estimado(&e, &peticion, &ahora).await?;

    // Notifica al peer dueño (el "de_id" es el broker/jefe). best-effort: si no existe, warn.
    let texto = format!(
        "📋 Nueva tarea asignada: {} (estimado {})",
        p.descripcion,
        humanizar_estimado(p.estimado_seg.unwrap_or(0))
    );
    if e.almacen.instancia_existe(&p.instancia_id).await? {
        if let Err(err) = e
            .almacen
            .encolar_mensaje("broker", &p.instancia_id, &texto, &ahora)
            .await
        {
            warn!("no se pudo notificar la tarea asignada a '{}': {err:#}", p.instancia_id);
        }
    } else {
        warn!(
            "tarea asignada a '{}' creada, pero el peer no está vivo: no se notifica (la verá al listar)",
            p.instancia_id
        );
    }

    // Bitácora (R4/R5/AC2): asignar es acción del OPERADOR (aparece en la jornada de Max, no
    // en la del peer destino); el destino queda como detalle.
    registrar_accion(
        &e,
        ID_OPERADOR,
        TipoAccion::CrearTarea,
        Some(tarea.id.clone()),
        Some(format!("asignada a {}", p.instancia_id)),
        Some(&tarea),
        None,
    )
    .await;

    info!("tarea {} asignada a '{}'", tarea.id, p.instancia_id);
    Ok(Json(serde_json::json!({ "ok": true, "tarea_id": tarea.id })))
}

/// `POST /tarea/reasignar` { tarea_id, nuevo_instancia_id } → cambia el dueño de la tarea y
/// notifica al nuevo (R7/AC4). Usa `almacen.tarea_reasignar` (#11): quita el id de la lista del
/// dueño VIEJO (LREM) y lo añade a la del nuevo (RPUSH) de forma atómica, para que la tarea no
/// aparezca en DOS jornadas (el bug del guardar manual). 404 si la tarea no existe.
async fn tarea_reasignar(
    State(e): State<Estado>,
    Json(p): Json<PeticionReasignarTarea>,
) -> Result<Json<Tarea>, ErrorApp> {
    let ahora = ahora_iso();
    // Capturamos el dueño anterior ANTES de reasignar (para el log de auditoría).
    let dueno_anterior = match e.almacen.tarea_obtener(&p.tarea_id).await? {
        Some(t) => t.instancia_id,
        None => {
            return Err(ErrorApp(anyhow::anyhow!(
                "la tarea '{}' no existe",
                p.tarea_id
            )))
        }
    };
    let Some(tarea) = e
        .almacen
        .tarea_reasignar(&p.tarea_id, &p.nuevo_instancia_id)
        .await?
    else {
        return Err(ErrorApp(anyhow::anyhow!(
            "la tarea '{}' no existe",
            p.tarea_id
        )));
    };

    // Notifica al nuevo dueño (best-effort). El de_id es el broker/jefe.
    let texto = format!("📋 Tarea reasignada a ti: {}", tarea.descripcion);
    if e.almacen.instancia_existe(&p.nuevo_instancia_id).await? {
        if let Err(err) = e
            .almacen
            .encolar_mensaje("broker", &p.nuevo_instancia_id, &texto, &ahora)
            .await
        {
            warn!("no se pudo notificar la reasignación a '{}': {err:#}", p.nuevo_instancia_id);
        }
    } else {
        warn!(
            "tarea {} reasignada a '{}', pero el peer no está vivo: no se notifica",
            tarea.id, p.nuevo_instancia_id
        );
    }

    // Bitácora (R4/R5): reasignar es acción del operador; la ruta vieja→nueva como detalle.
    registrar_accion(
        &e,
        ID_OPERADOR,
        TipoAccion::ReasignarTarea,
        Some(tarea.id.clone()),
        Some(format!("de '{dueno_anterior}' a '{}'", p.nuevo_instancia_id)),
        Some(&tarea),
        None,
    )
    .await;

    info!(
        "tarea {} reasignada: '{}' → '{}'",
        tarea.id, dueno_anterior, p.nuevo_instancia_id
    );
    Ok(Json(tarea))
}

/// `POST /tarea/forzar` { tarea_id } → "tócale el hombro" (R8/AC3): empuja un recordatorio de
/// la tarea a la sesión del peer dueño (reusa `encolar_mensaje`, igual que /enviar). 404 si la
/// tarea no existe. La entrega es best-effort: si el peer no está vivo, warn (sin crash).
async fn tarea_forzar(
    State(e): State<Estado>,
    Json(p): Json<PeticionForzarTarea>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    let ahora = ahora_iso();
    let Some(tarea) = e.almacen.tarea_obtener(&p.tarea_id).await? else {
        return Err(ErrorApp(anyhow::anyhow!(
            "la tarea '{}' no existe",
            p.tarea_id
        )));
    };
    let texto = format!("⏰ Recordatorio de tu tarea: {}", tarea.descripcion);
    if e.almacen.instancia_existe(&tarea.instancia_id).await? {
        e.almacen
            .encolar_mensaje("broker", &tarea.instancia_id, &texto, &ahora)
            .await?;
        // Bitácora (R4/R5): "tócale el hombro" es acción del operador sobre la tarea.
        registrar_accion(
            &e,
            ID_OPERADOR,
            TipoAccion::ForzarTarea,
            Some(tarea.id.clone()),
            None,
            Some(&tarea),
            None,
        )
        .await;
        info!("tarea {} forzada al peer '{}'", tarea.id, tarea.instancia_id);
        Ok(Json(RespuestaOk { ok: true }))
    } else {
        warn!(
            "no se pudo forzar la tarea {}: el peer dueño '{}' no está vivo",
            tarea.id, tarea.instancia_id
        );
        Ok(Json(RespuestaOk { ok: false }))
    }
}

/// `GET /tarea/reportes?tarea_id=` → historial de reportes de progreso de la tarea (R9/AC5).
/// Lo consume el modal DETALLE de la TUI. SOLO LECTURA. Vacío si no hay reportes.
async fn tarea_reportes(
    State(e): State<Estado>,
    axum::extract::Query(p): axum::extract::Query<PeticionReportesTarea>,
) -> Result<Json<Vec<String>>, ErrorApp> {
    Ok(Json(e.almacen.tarea_reportes(&p.tarea_id).await?))
}

/// `POST /listar-tareas` { instancia_id } → las tareas de la jornada de esa instancia (R10).
async fn listar_tareas(
    State(e): State<Estado>,
    Json(p): Json<PeticionJornada>,
) -> Result<Json<Vec<Tarea>>, ErrorApp> {
    let (_sesiones, tareas) = e.almacen.jornada(&p.instancia_id).await?;
    Ok(Json(tareas))
}

/// `GET /factor-estimacion` → el factor de corrección global aprendido (R4/R10/R11). Lo
/// consume la TUI (pantalla Broker). Default neutro si aún no hay muestras.
async fn factor_estimacion(
    State(e): State<Estado>,
) -> Result<Json<FactorEstimacion>, ErrorApp> {
    Ok(Json(e.almacen.factor_estimacion().await?))
}

/// `GET /factor-estimacion-peer?instancia_id=<id>` → el factor aprendido SOLO de ese peer (#9).
/// Es la pieza de accountability individual: un peer mentiroso sesga su PROPIO factor, no el
/// global de los honestos. Default neutro si ese peer aún no tiene muestras.
async fn factor_estimacion_peer(
    State(e): State<Estado>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<FactorEstimacion>, ErrorApp> {
    let id = q.get("instancia_id").map(String::as_str).unwrap_or_default();
    Ok(Json(e.almacen.factor_estimacion_peer(id).await?))
}

/// Resuelve (owner, repo) del repo_github de una instancia. None si la instancia no existe
/// o no tiene repo_github válido ("owner/repo") → el broker degrada (sin issue).
async fn repo_de_instancia(e: &Estado, instancia_id: &str) -> Option<(String, String)> {
    let inst = match e.almacen.instancia_obtener(instancia_id).await {
        Ok(i) => i?,
        Err(err) => {
            warn!("no se pudo leer la instancia {instancia_id} para resolver repo: {err:#}");
            return None;
        }
    };
    let repo_github = inst.repo_github?;
    let (owner, repo) = repo_github.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

async fn jornada_consolidada(
    State(e): State<Estado>,
    Json(p): Json<PeticionJornada>,
) -> Result<Json<RespuestaJornada>, ErrorApp> {
    Ok(Json(jornada::consolidar(&e.almacen, &p.instancia_id).await?))
}

// --- Handlers de admin (panel TUI: pantallas Peers/Redis/Broker) ---
//
// Todos cuelgan de `rutas_protegidas` → exigen el token (X-Peers-Token) igual que el resto;
// nunca van en /salud. Son introspección de SOLO LECTURA salvo /admin/purgar (mantenimiento
// explícito). version = la del propio crate del broker en compilación.

/// `GET /admin/info` → { host, puerto, instancias, version }.
async fn admin_info(State(e): State<Estado>) -> Result<Json<RespuestaAdminInfo>, ErrorApp> {
    Ok(Json(RespuestaAdminInfo {
        host: e.host.clone(),
        puerto: e.puerto,
        instancias: e.almacen.contar_instancias().await?,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }))
}

/// `GET /admin/redis` → resumen de colas y outbox por instancia. SOLO LECTURA: usa
/// `contar_mensajes_pendientes` (LLEN) y `outbox_pendientes(..).len()`, nunca drena.
async fn admin_redis(State(e): State<Estado>) -> Result<Json<RespuestaAdminRedis>, ErrorApp> {
    let ids = e.almacen.listar_ids().await?;
    let mut colas = Vec::with_capacity(ids.len());
    let mut outbox = Vec::with_capacity(ids.len());
    for id in &ids {
        let pendientes_msg = e.almacen.contar_mensajes_pendientes(id).await?;
        if pendientes_msg > 0 {
            colas.push(ColaResumen { id: id.clone(), pendientes: pendientes_msg });
        }
        let pendientes_ob = e.almacen.outbox_pendientes(id).await?.len();
        if pendientes_ob > 0 {
            outbox.push(ColaResumen { id: id.clone(), pendientes: pendientes_ob });
        }
    }
    Ok(Json(RespuestaAdminRedis {
        total_instancias: ids.len(),
        colas,
        outbox,
    }))
}

/// `POST /admin/purgar` { id } → borra cola de mensajes + outbox de ese id. Idempotente.
async fn admin_purgar(
    State(e): State<Estado>,
    Json(p): Json<PeticionPurgar>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    e.almacen.purgar(&p.id).await?;
    // Bitácora (R4/R5): purga = acción del operador; el peer purgado es el sujeto.
    registrar_accion(&e, ID_OPERADOR, TipoAccion::Purgar, Some(p.id.clone()), None, None, None).await;
    info!("admin: purgada la cola y outbox de '{}'", p.id);
    Ok(Json(RespuestaOk { ok: true }))
}

// --- Handlers de entrega durable + trazabilidad (fase 1/2) ---

/// `POST /confirmar` { ids, estado } → el cliente confirma el avance de estado de los
/// mensajes que recibió. El broker timbra el tiempo con SU reloj (R1.2/R1.4). Idempotente:
/// `transicionar_mensaje` solo avanza si el rango sube; los ids inexistentes son no-op.
/// `recibir` NO transiciona: la confirmación es explícita y separada (R1.4). Va en
/// `rutas_protegidas` (token), NUNCA en /salud.
async fn confirmar(
    State(e): State<Estado>,
    Json(p): Json<PeticionConfirmar>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    let ahora = ahora_iso();
    for id in &p.ids {
        // El timbre lo pone el broker (nunca la IA). Idempotente por id.
        e.almacen.transicionar_mensaje(*id, p.estado, &ahora).await?;
    }
    Ok(Json(RespuestaOk { ok: true }))
}

/// `GET /admin/historial?id=&desde=&estado=` → historial durable de la cola (R2.2). Lo
/// consume la TUI (pantalla Trazabilidad). SOLO LECTURA. Va en `rutas_protegidas` (token).
/// `desde` = cursor exclusivo (msg_id > desde); `estado` = filtro opcional.
async fn admin_historial(
    State(e): State<Estado>,
    axum::extract::Query(p): axum::extract::Query<PeticionHistorial>,
) -> Result<Json<Vec<Mensaje>>, ErrorApp> {
    let msgs = e.almacen.historial(&p.id, p.desde, p.estado).await?;
    Ok(Json(msgs))
}

/// `POST /admin/reenviar` { msg_id } → re-encola un mensaje del historial como uno NUEVO
/// (msgseq fresco, estado `Enviado`, `reenviado_de = msg_id`, `reenvios + 1`) en la bandeja
/// del `para_id` original (R2.3). Devuelve el `msg_id` del nuevo. Va en `rutas_protegidas`.
async fn admin_reenviar(
    State(e): State<Estado>,
    Json(p): Json<PeticionReenviar>,
) -> Result<Json<serde_json::Value>, ErrorApp> {
    let Some(original) = e.almacen.mensaje_obtener(p.msg_id).await? else {
        // Mensaje inexistente: no es un 500, es una petición sin efecto. ok=false explícito.
        return Ok(Json(serde_json::json!({
            "ok": false,
            "error": format!("El mensaje {} no existe en el historial", p.msg_id),
        })));
    };
    let nuevo_id = e.almacen.encolar_reenvio(&original, &ahora_iso()).await?;
    info!(
        "admin: reenviado msg {} → nuevo msg {} (para '{}')",
        p.msg_id, nuevo_id, original.para_id
    );
    Ok(Json(serde_json::json!({ "ok": true, "msg_id": nuevo_id })))
}

// --- Supervisor (fase 5): detección de ociosos / atascados / ghosteo ---

/// `GET /admin/alertas` → las alertas vigentes de la cola `cprs:alertas` (R6). SOLO LECTURA.
/// La TUI la pinta como banner. Va en `rutas_protegidas` (token, AC3); nunca en /salud.
async fn admin_alertas(State(e): State<Estado>) -> Result<Json<Vec<Alerta>>, ErrorApp> {
    Ok(Json(e.almacen.alertas().await?))
}

/// `POST /admin/alerta-resolver` → el jefe descarta una alerta a mano desde la TUI. Reusa
/// `alerta_resolver(tipo, sujeto)` (el mismo que el supervisor llama cuando la condición se
/// resuelve sola). Idempotente: descartar una alerta ya inexistente no es error. Va en
/// `rutas_protegidas` (token); nunca en /salud.
async fn admin_alerta_resolver(
    State(e): State<Estado>,
    Json(p): Json<PeticionResolverAlerta>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    e.almacen.alerta_resolver(&p.tipo, &p.sujeto).await?;
    // Bitácora (R4/R5): descarte manual del operador; clave tipo:sujeto como sujeto del evento.
    registrar_accion(
        &e,
        ID_OPERADOR,
        TipoAccion::ResolverAlerta,
        Some(format!("{}:{}", p.tipo, p.sujeto)),
        None,
        None,
        None,
    )
    .await;
    info!("admin: alerta '{}' / '{}' descartada manualmente", p.tipo, p.sujeto);
    Ok(Json(RespuestaOk { ok: true }))
}

/// `GET /admin/tareas` → vista global del jefe: TODAS las tareas de TODAS las instancias, cada
/// una con su `instancia_id`, ordenadas por inicio desc (#14/R1/AC1). SOLO LECTURA. Lo consume
/// la TUI en "vista global" (tecla `g`), que las filtra por estado y pone overrun-primero. Va en
/// `rutas_protegidas` (token); nunca en /salud. El orden y el join sin KEYS los hace el almacén.
async fn admin_tareas(State(e): State<Estado>) -> Result<Json<Vec<Tarea>>, ErrorApp> {
    Ok(Json(e.almacen.tareas_todas().await?))
}

// --- Handlers de la política de comunicación (RFC politica-comunicacion R6/R7) ---

/// `GET /admin/politica` → la política VIGENTE: la copia en memoria que `enviar` está usando
/// (la verdad operativa), no una relectura del store. Bajo token; nunca en /salud.
async fn admin_politica_leer(State(e): State<Estado>) -> Json<Politica> {
    Json(e.politica.read().await.clone())
}

/// `POST /admin/politica` → REEMPLAZA la política completa (R6: reemplazo idempotente, no
/// parche) y la aplica EN CALIENTE (R9/AC4: el siguiente `/enviar` ya la respeta, sin
/// reiniciar). Los patrones llegan validados por serde (`Patron` rechaza vacío → 422 del
/// extractor Json). Orden deliberado: persistir PRIMERO, refrescar la copia caliente DESPUÉS —
/// si el store falla, la vigente no cambia (500 y todo queda como estaba). El write-guard se
/// toma y suelta sin ningún `.await` dentro.
async fn admin_politica_guardar(
    State(e): State<Estado>,
    Json(p): Json<Politica>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    e.almacen.politica_guardar(&p).await?;
    let reglas = p.reglas.len();
    *e.politica.write().await = p;
    info!("política de comunicación actualizada en caliente: {reglas} regla(s)");
    Ok(Json(RespuestaOk { ok: true }))
}

/// `GET /admin/politica/bloqueos` → últimos intentos bloqueados (R7/AC5), más reciente
/// primero, acotados a MAX_BLOQUEOS por el almacén. La UI pinta el contador/lista.
async fn admin_politica_bloqueos(
    State(e): State<Estado>,
) -> Result<Json<Vec<BloqueoComunicacion>>, ErrorApp> {
    Ok(Json(e.almacen.bloqueos_recientes().await?))
}

/// `GET /acciones?instancia_id=&desde=&limite=` (RFC registro-acciones R6, bajo token) →
/// bitácora del peer, más reciente primero. Bitácora desactivada → lista vacía (degradación,
/// AC5: nunca un error por observabilidad ausente). `limite` default 100 (acotado en Bitacora).
async fn acciones(
    State(e): State<Estado>,
    axum::extract::Query(p): axum::extract::Query<PeticionAcciones>,
) -> Result<Json<Vec<AccionRegistrada>>, ErrorApp> {
    let Some(b) = &e.bitacora else {
        return Ok(Json(vec![]));
    };
    Ok(Json(
        b.listar(&p.instancia_id, p.desde.as_deref(), p.limite.unwrap_or(100))
            .await?,
    ))
}

/// Evalúa los 3 detectores del supervisor y emite/resuelve alertas (R1–R4, R7).
///
/// El tiempo lo mide el broker: recibe `ahora` (su reloj) y lo compara contra los timestamps
/// de cada sujeto con `peers_core::supera_umbral` (función pura). La idempotencia (R7/AC4) la
/// garantiza el almacén: `alerta_emitir` solo emite si `(tipo+sujeto)` no estaba ya activa.
///
/// DEGRADACIÓN (R8/AC5): cada detector va en su propio bloque con manejo de error (warn +
/// sigue). Un detector que falle leyendo su cola NO impide los otros ni tumba el broker.
async fn detectar_alertas(estado: &Estado, umbrales: Umbrales, ahora: &str) {
    if let Err(err) = detectar_ociosos(estado, umbrales.ocioso_seg, ahora).await {
        warn!("detector OCIOSO falló (se sigue con los demás): {err:#}");
    }
    if let Err(err) = detectar_atascados(estado, umbrales.atasco_seg, ahora).await {
        warn!("detector ATASCADO falló (se sigue con los demás): {err:#}");
    }
    if let Err(err) = detectar_ghosteo(estado, umbrales.ghosteo_seg, ahora).await {
        warn!("detector GHOSTEO falló (se sigue con los demás): {err:#}");
    }
    if let Err(err) = detectar_cancelacion_excesiva(estado, ahora).await {
        warn!("detector CANCELACION_EXCESIVA falló (se sigue con los demás): {err:#}");
    }
}

/// #10 — Cancelación excesiva: por cada instancia VIVA, cuenta sus tareas Canceladas vs el total
/// y, si el ratio supera `UMBRAL_CANCELACION` (regla pura `cancelacion_excesiva`), emite
/// `TipoAlerta::CancelacionExcesiva` (sujeto = id del peer). Cancelar es la vía de escape para
/// esconder fallos (Cancelada NO alimenta el factor), así que un ratio alto sesga la métrica por
/// omisión. Cuando el ratio vuelve a bajar del umbral, se resuelve la alerta (AC2).
async fn detectar_cancelacion_excesiva(estado: &Estado, ahora: &str) -> anyhow::Result<()> {
    let vivas = estado
        .almacen
        .listar(
            peers_core::Alcance::Maquina,
            "",
            None,
            None,
            &vencidas_antes_iso(),
        )
        .await?;

    for inst in &vivas {
        let (_sesiones, tareas) = estado.almacen.jornada(&inst.id).await?;
        let total = u32::try_from(tareas.len()).unwrap_or(u32::MAX);
        let canceladas = u32::try_from(
            tareas
                .iter()
                .filter(|t| t.estado == EstadoTarea::Cancelada)
                .count(),
        )
        .unwrap_or(u32::MAX);

        if peers_core::cancelacion_excesiva(canceladas, total) {
            let alerta = Alerta {
                tipo: TipoAlerta::CancelacionExcesiva,
                sujeto: inst.id.clone(),
                detalle: format!(
                    "peer '{}' canceló {}/{} tareas (> {:.0}%): posible vía de escape",
                    inst.id,
                    canceladas,
                    total,
                    peers_core::UMBRAL_CANCELACION * 100.0
                ),
                creada_en: ahora.to_string(),
            };
            if estado.almacen.alerta_emitir(&alerta).await? {
                info!("alerta CANCELACION_EXCESIVA emitida: {}", inst.id);
            }
        } else {
            // La condición cesó (ratio bajo el umbral): resuelve la alerta activa (AC2).
            estado
                .almacen
                .alerta_resolver("cancelacion_excesiva", &inst.id)
                .await?;
        }
    }
    Ok(())
}

/// R2 — Ocioso: por cada instancia VIVA (visto < VENCIMIENTO) sin tarea en curso, mira cuánto
/// lleva sin actividad. La "actividad" más reciente es el inicio de su tarea/sesión más reciente
/// (o su registro). Si ese instante está a más de `umbral_seg` del `ahora` del broker → alerta.
/// Cuando recupera una tarea abierta, se resuelve la alerta (AC2: la condición cesa).
async fn detectar_ociosos(estado: &Estado, umbral_seg: i64, ahora: &str) -> anyhow::Result<()> {
    // Instancias VIVAS según la liveness por latido (mismo criterio que `listar`).
    let vivas = estado
        .almacen
        .listar(
            peers_core::Alcance::Maquina,
            "",
            None,
            None,
            &vencidas_antes_iso(),
        )
        .await?;

    for inst in &vivas {
        let (sesiones, tareas) = estado.almacen.jornada(&inst.id).await?;
        let tiene_tarea_abierta = tareas.iter().any(|t| t.fin.is_none());

        if tiene_tarea_abierta {
            // La condición cesó: si había una alerta de ocioso activa, se resuelve (AC2).
            estado.almacen.alerta_resolver("ocioso", &inst.id).await?;
            continue;
        }

        // Instante de la última actividad conocida: el inicio más reciente entre las tareas y
        // las sesiones; si no hay ninguna, su `visto_en` (latido). Es el ancla del "desde".
        let ultima_actividad = tareas
            .iter()
            .map(|t| t.inicio.as_str())
            .chain(sesiones.iter().map(|s| s.inicio.as_str()))
            .max()
            .unwrap_or(inst.visto_en.as_str());

        if supera_umbral(ultima_actividad, ahora, umbral_seg) {
            let alerta = Alerta {
                tipo: TipoAlerta::Ocioso,
                sujeto: inst.id.clone(),
                detalle: format!(
                    "peer '{}' vivo sin tarea en curso desde hace > {}s",
                    inst.id, umbral_seg
                ),
                creada_en: ahora.to_string(),
            };
            if estado.almacen.alerta_emitir(&alerta).await? {
                info!("alerta OCIOSO emitida: {}", inst.id);
            }
        }
    }
    Ok(())
}

/// R3 — Atascado: por cada tarea abierta (sin `fin`) cuyo `inicio` está a más de `umbral_seg`
/// del `ahora` del broker → alerta. Al cerrarse la tarea (deja de aparecer abierta) se resuelve.
/// Itera sobre las instancias VIVAS y sus jornadas (reusa lo existente, sin nuevos métodos).
async fn detectar_atascados(estado: &Estado, umbral_seg: i64, ahora: &str) -> anyhow::Result<()> {
    let vivas = estado
        .almacen
        .listar(
            peers_core::Alcance::Maquina,
            "",
            None,
            None,
            &vencidas_antes_iso(),
        )
        .await?;

    for inst in &vivas {
        let (_sesiones, tareas) = estado.almacen.jornada(&inst.id).await?;
        for tarea in &tareas {
            if tarea.fin.is_some() {
                // Tarea cerrada: la condición cesó, se resuelve su alerta (AC2).
                estado.almacen.alerta_resolver("atascado", &tarea.id).await?;
                continue;
            }
            if supera_umbral(&tarea.inicio, ahora, umbral_seg) {
                let alerta = Alerta {
                    tipo: TipoAlerta::Atascado,
                    sujeto: tarea.id.clone(),
                    detalle: format!(
                        "tarea '{}' de '{}' abierta sin reporte desde hace > {}s: {}",
                        tarea.id, inst.id, umbral_seg, tarea.descripcion
                    ),
                    creada_en: ahora.to_string(),
                };
                if estado.almacen.alerta_emitir(&alerta).await? {
                    info!("alerta ATASCADO emitida: {}", tarea.id);
                }
            }
        }
    }
    Ok(())
}

/// R4 — Ghosteo: mensajes en estado `Leido` (no `Procesado`) cuyo `leido_en` está a más de
/// `umbral_seg` del `ahora` del broker → alerta. Cuando el mensaje pasa a `Procesado` deja de
/// aparecer en `mensajes_en_estado(Leido)`, así que aquí lo resolvemos explícitamente (AC2):
/// recogemos los `Procesado` recientes y limpiamos su alerta activa.
async fn detectar_ghosteo(estado: &Estado, umbral_seg: i64, ahora: &str) -> anyhow::Result<()> {
    // (AC2) Resolver: todo lo que ya está Procesado deja de ghostear.
    let procesados = estado
        .almacen
        .mensajes_en_estado(EstadoMensaje::Procesado)
        .await?;
    for (_para_id, msg) in &procesados {
        estado
            .almacen
            .alerta_resolver("ghosteo", &format!("msg:{}", msg.id))
            .await?;
    }

    // Detectar: Leido sin Procesado por encima del umbral.
    let leidos = estado.almacen.mensajes_en_estado(EstadoMensaje::Leido).await?;
    for (para_id, msg) in &leidos {
        // El "desde" es el instante en que se marcó Leido; si falta, no se puede medir → skip.
        let Some(leido_en) = msg.leido_en.as_deref() else {
            continue;
        };
        if supera_umbral(leido_en, ahora, umbral_seg) {
            let sujeto = format!("msg:{}", msg.id);
            let alerta = Alerta {
                tipo: TipoAlerta::Ghosteo,
                sujeto: sujeto.clone(),
                detalle: format!(
                    "mensaje {} de '{}' para '{}' leído sin procesar desde hace > {}s",
                    msg.id, msg.de_id, para_id, umbral_seg
                ),
                creada_en: ahora.to_string(),
            };
            if estado.almacen.alerta_emitir(&alerta).await? {
                info!("alerta GHOSTEO emitida: {sujeto}");
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    // Selección del backend de persistencia. Redis por defecto; SQLite tras feature.
    let almacen: Arc<dyn Almacen> = construir_almacen(&args)?;

    // Cliente GitHub opcional (None si falta GITHUB_TOKEN → degradación). El repo destino
    // ya NO es fijo: es dinámico, lo aporta cada peer (su repo_github) al abrir la tarea.
    let github = GitHub::desde_entorno();
    if github.is_some() {
        info!("integración GitHub Issues ACTIVA (repo dinámico por peer)");
    } else {
        info!("integración GitHub Issues inactiva (sin GITHUB_TOKEN)");
    }

    // Limpieza inicial de instancias muertas.
    if let Ok(n) = almacen.limpiar_vencidas(&vencidas_antes_iso()).await {
        if n > 0 {
            info!("limpieza inicial: {n} instancia(s) vencida(s)");
        }
    }

    // Política de comunicación (R9): se carga UNA vez al arrancar; `POST /admin/politica` la
    // refresca en caliente. Si el store falla la lectura, se arranca con el default Permitir y
    // un warn (fail-open, AC6: la falta de política nunca deja al equipo sin habla).
    let politica_inicial = match almacen.politica_leer().await {
        Ok(p) => p,
        Err(err) => {
            warn!("no se pudo leer la política de comunicación (arranco con default Permitir): {err:#}");
            Politica::default()
        }
    };
    if politica_inicial.reglas.is_empty() {
        info!("política de comunicación: sin reglas (todo permitido)");
    } else {
        info!(
            "política de comunicación activa: {} regla(s), default {:?}",
            politica_inicial.reglas.len(),
            politica_inicial.accion_por_defecto
        );
    }

    // Bitácora de acciones (ADR-001): fichero propio, independiente del backend. Degradación
    // graciosa: si no se puede abrir, warn y el broker sigue SIN bitácora (jamás bloquea el
    // arranque — es observabilidad, no negocio).
    let ruta_bitacora = args
        .bitacora_db
        .clone()
        .unwrap_or_else(ruta_bitacora_defecto);
    let bitacora = match bitacora::Bitacora::abrir(&ruta_bitacora).await {
        Ok(b) => {
            info!("bitácora de acciones activa: {ruta_bitacora}");
            Some(b)
        }
        Err(err) => {
            warn!("bitácora de acciones DESACTIVADA (no se pudo abrir {ruta_bitacora}): {err:#}");
            None
        }
    };

    let estado: Estado = Arc::new(EstadoApp {
        almacen,
        github,
        host: args.host.clone(),
        puerto: args.puerto,
        registro_lock: tokio::sync::Mutex::new(()),
        politica: tokio::sync::RwLock::new(politica_inicial),
        bitacora,
        antispoofing: args.antispoofing,
    });

    // Umbrales del supervisor (R8): resueltos una vez, copiados al spawn (Copy).
    let umbrales = Umbrales {
        ocioso_seg: args.umbral_ocioso,
        atasco_seg: args.umbral_atasco,
        ghosteo_seg: args.umbral_ghosteo,
    };
    info!(
        "supervisor activo — umbrales (s): ocioso={} atasco={} ghosteo={}",
        umbrales.ocioso_seg, umbrales.atasco_seg, umbrales.ghosteo_seg
    );

    // Limpieza periódica por latido (cada 30s). En el MISMO ciclo corre el supervisor (R1):
    // detecta ociosos/atascados/ghosteo y emite/resuelve alertas. El tiempo lo pone el broker.
    let limpieza = estado.clone();
    tokio::spawn(async move {
        let mut intervalo = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            intervalo.tick().await;
            match limpieza.almacen.limpiar_vencidas(&vencidas_antes_iso()).await {
                Ok(n) if n > 0 => info!("limpieza periódica: {n} instancia(s) vencida(s)"),
                Ok(_) => {}
                Err(e) => error!("fallo en limpieza periódica: {e:#}"),
            }
            // Retención del historial durable (R2.1): recorta cada cola a los últimos N.
            if let Err(e) = limpieza
                .almacen
                .podar_historial(peers_core::RETENCION_HISTORIAL)
                .await
            {
                error!("fallo al podar historial: {e:#}");
            }
            // #15/R6: retención de tareas por instancia — conserva las RETENCION_TAREAS más
            // recientes por peer y purga las viejas (espejo de podar_historial). Degrada: si
            // falla, se loguea y el ciclo sigue (no tumba el broker).
            if let Err(e) = limpieza
                .almacen
                .podar_tareas(peers_core::RETENCION_TAREAS)
                .await
            {
                error!("fallo al podar tareas: {e:#}");
            }
            // R7 registro-acciones: poda de la bitácora a las últimas N por peer (ADR-001).
            if let Some(b) = &limpieza.bitacora {
                if let Err(e) = b.podar(peers_core::RETENCION_ACCIONES as i64).await {
                    error!("fallo al podar la bitácora de acciones: {e:#}");
                }
            }
            // Supervisor (R1): el `ahora` lo timbra el broker en cada tick. La función ya
            // aísla cada detector (warn + sigue), así que un fallo no rompe el ciclo (AC5).
            detectar_alertas(&limpieza, umbrales, &ahora_iso()).await;
        }
    });

    use axum::middleware::from_fn_with_state;

    // Warning ruidoso si se expone en red sin token (agujero accidental).
    // Cubre IPv4 e IPv6: cualquier host que NO sea loopback y sin token → aviso.
    if !host_es_loopback(&args.host) && args.token.is_none() {
        warn!("broker EXPUESTO en {} SIN token — cualquiera en la red puede conectarse. \
               Usa --token para protegerlo.", args.host);
    }

    let rutas_protegidas = Router::new()
        .route("/registrar", post(registrar))
        .route("/latido", post(latido))
        .route("/definir-resumen", post(definir_resumen))
        .route("/listar", post(listar))
        .route("/enviar", post(enviar))
        .route("/recibir", post(recibir))
        .route("/confirmar", post(confirmar))
        .route("/salir", post(salir))
        .route("/tarea/abrir", post(tarea_abrir))
        .route("/tarea/reportar", post(tarea_reportar))
        .route("/tarea/cerrar", post(tarea_cerrar))
        // Gestión interactiva de tareas (R4–R9): el jefe (TUI) dirige a sus peers.
        .route("/tarea/editar", post(tarea_editar))
        .route("/tarea/estado", post(tarea_estado))
        .route("/tarea/asignar", post(tarea_asignar))
        .route("/tarea/reasignar", post(tarea_reasignar))
        .route("/tarea/forzar", post(tarea_forzar))
        .route("/tarea/reportes", get(tarea_reportes))
        .route("/jornada", post(jornada_consolidada))
        // Tareas autogestionadas + aprendizaje (las que usan las tools MCP del peers-client).
        .route("/crear-tarea", post(crear_tarea))
        .route("/cerrar-tarea", post(cerrar_tarea))
        .route("/listar-tareas", post(listar_tareas))
        .route("/factor-estimacion", get(factor_estimacion))
        .route("/factor-estimacion-peer", get(factor_estimacion_peer))
        // Admin (panel TUI): introspección protegida por el mismo token, nunca en /salud.
        .route("/admin/info", get(admin_info))
        .route("/admin/redis", get(admin_redis))
        .route("/admin/purgar", post(admin_purgar))
        .route("/admin/historial", get(admin_historial))
        .route("/admin/reenviar", post(admin_reenviar))
        // Supervisor (fase 5): alertas vigentes para el banner de la TUI (R6/AC3).
        .route("/admin/alertas", get(admin_alertas))
        .route("/admin/alerta-resolver", post(admin_alerta_resolver))
        // #14/R1: vista global de todas las tareas (cuadro de mando del jefe).
        .route("/admin/tareas", get(admin_tareas))
        // Política de comunicación (RFC politica-comunicacion R6/R7): leer/reemplazar en
        // caliente + trazabilidad de bloqueos. Bajo token, nunca en /salud.
        .route(
            "/admin/politica",
            get(admin_politica_leer).post(admin_politica_guardar),
        )
        .route("/admin/politica/bloqueos", get(admin_politica_bloqueos))
        // Bitácora de acciones (RFC registro-acciones R6): el feed que la Jornada pinta.
        .route("/acciones", get(acciones))
        .layer(from_fn_with_state(args.token.clone(), verificar_token));

    let app = Router::new()
        .route("/salud", get(salud))   // exenta de auth
        .merge(rutas_protegidas)
        .with_state(estado);

    let direccion = format!("{}:{}", args.host, args.puerto);
    let listener = tokio::net::TcpListener::bind(&direccion).await?;
    info!("peers-broker escuchando en {direccion}");
    // E-10: deja explícito en qué modo corre (auditoría). Apagado = confía en el de_id declarado.
    if args.antispoofing {
        info!("anti-spoofing del de_id (E-10): ACTIVO — el de_id se resuelve por secreto de sesión");
    } else {
        warn!("anti-spoofing del de_id (E-10): DESACTIVADO — el broker confía en el de_id declarado (pre-E-10). Activar con CLAUDE_PEERS_ANTISPOOF=true");
    }
    axum::serve(listener, app).await?;
    Ok(())
}

/// Construye el almacén según las features compiladas. Redis por defecto.
#[cfg(not(feature = "sqlite"))]
fn construir_almacen(args: &Args) -> anyhow::Result<Arc<dyn Almacen>> {
    info!("backend de persistencia: Redis ({})", args.redis_url);
    Ok(Arc::new(AlmacenRedis::nuevo(&args.redis_url)?))
}

/// Con la feature sqlite, si se pasa --db usa SQLite; si no, sigue Redis.
#[cfg(feature = "sqlite")]
fn construir_almacen(args: &Args) -> anyhow::Result<Arc<dyn Almacen>> {
    match &args.db {
        Some(ruta) => {
            info!("backend de persistencia: SQLite ({ruta})");
            Ok(Arc::new(db::AlmacenSqlite::abrir(ruta)?))
        }
        None => {
            info!("backend de persistencia: Redis ({})", args.redis_url);
            Ok(Arc::new(AlmacenRedis::nuevo(&args.redis_url)?))
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::{host_es_loopback, pid_vivo, token_autorizado, Args};

    #[test]
    fn token_correcto_pasa_y_ausente_falla() {
        // Lógica pura de decisión del middleware, sin levantar axum.
        assert!(token_autorizado(Some("abc"), Some("abc")));   // coincide → pasa
        assert!(!token_autorizado(Some("abc"), Some("xyz")));  // distinto → falla
        assert!(!token_autorizado(Some("abc"), None));         // falta → falla
        assert!(token_autorizado(None, None));                 // sin token configurado → pasa
        assert!(token_autorizado(None, Some("lo-que-sea")));   // sin config → pasa (ignora)
    }

    #[test]
    fn loopback_cubre_ipv4_ipv6_y_localhost() {
        assert!(host_es_loopback("127.0.0.1"));
        assert!(host_es_loopback("127.0.0.5"));  // todo 127.0.0.0/8 es loopback
        assert!(host_es_loopback("::1"));         // IPv6 loopback
        assert!(host_es_loopback("localhost"));
        // Expuestos en red → NO loopback (deben disparar el warning sin token):
        assert!(!host_es_loopback("0.0.0.0"));    // wildcard IPv4
        assert!(!host_es_loopback("::"));         // wildcard IPv6 (el hueco que se cerró)
        assert!(!host_es_loopback("10.0.0.67"));  // LAN
        assert!(!host_es_loopback("192.168.1.5"));
    }

    #[test]
    fn debug_de_args_no_filtra_el_token() {
        // El Debug manual debe REDACTAR el token, nunca imprimirlo en claro.
        let args = Args {
            puerto: 7899,
            host: "0.0.0.0".into(),
            redis_url: "redis://127.0.0.1:6379".into(),
            db: None,
            token: Some("secreto-super-sensible".into()),
            umbral_ocioso: peers_core::UMBRAL_OCIOSO_SEG,
            umbral_atasco: peers_core::UMBRAL_ATASCO_SEG,
            umbral_ghosteo: peers_core::UMBRAL_GHOSTEO_SEG,
            bitacora_db: None,
            antispoofing: false,
        };
        let s = format!("{args:?}");
        assert!(!s.contains("secreto-super-sensible"), "el token NO debe aparecer en claro");
        assert!(s.contains("<presente>"), "debe indicar que hay token, redactado");
    }

    #[test]
    fn pid_propio_esta_vivo() {
        // El PID de este proceso de test SIEMPRE está vivo.
        assert!(pid_vivo(std::process::id() as i64));
    }

    #[test]
    fn pid_imposible_o_invalido_esta_muerto() {
        // PIDs <= 0 no son válidos → muertos.
        assert!(!pid_vivo(0));
        assert!(!pid_vivo(-1));
        // Un PID enorme casi seguro no existe → muerto (kill(pid,0) da ESRCH).
        assert!(!pid_vivo(2_000_000_000));
    }
}
