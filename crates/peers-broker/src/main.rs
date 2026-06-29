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
    Almacen, ColaResumen, PeticionAbrirTarea, PeticionCerrarTarea, PeticionDefinirResumen,
    PeticionEnviar, PeticionJornada, PeticionLatido, PeticionListar, PeticionPurgar,
    PeticionRecibir, PeticionRegistrar, PeticionReportarTarea, PeticionSalir, RespuestaAbrirTarea,
    RespuestaAdminInfo, RespuestaAdminRedis, RespuestaEnviar, RespuestaJornada, RespuestaOk,
    RespuestaRegistrar, RespuestaSalud, PUERTO_DEFECTO, VENCIMIENTO_MS,
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
async fn resolver_id_sin_colision(e: &Estado, id_preferido: Option<&str>, pid_nuevo: i64) -> String {
    let Some(base) = id_preferido else {
        return generar_id();
    };
    // ¿El id base está ocupado por un peer DISTINTO y vivo?
    if !id_ocupado_por_otro_vivo(e, base, pid_nuevo).await {
        return base.to_string();
    }
    // Colisión real: busca el primer sufijo libre (-2, -3, …). Cota defensiva en 99.
    for n in 2..=99 {
        let candidato = format!("{base}-{n}");
        if !id_ocupado_por_otro_vivo(e, &candidato, pid_nuevo).await {
            warn!("id '{base}' ocupado por otro peer vivo; esta instancia usa '{candidato}'");
            return candidato;
        }
    }
    // Improbable (99 colisiones): cae a aleatorio para no bloquear el registro.
    generar_id()
}

/// true si `id` ya está registrado por un peer con PID distinto a `pid_nuevo` y ese PID
/// sigue vivo en esta máquina. (PID muerto o mismo PID → NO es colisión: es re-registro.)
async fn id_ocupado_por_otro_vivo(e: &Estado, id: &str, pid_nuevo: i64) -> bool {
    match e.almacen.instancia_obtener(id).await {
        Ok(Some(inst)) => inst.pid != pid_nuevo && pid_vivo(inst.pid),
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
    let id = resolver_id_sin_colision(&e, p.id_preferido.as_deref(), p.pid).await;
    let ahora = ahora_iso();
    e.almacen
        .registrar(
            &id,
            p.pid,
            &p.directorio,
            p.repo_git.as_deref(),
            p.repo_github.as_deref(),
            p.tty.as_deref(),
            &p.resumen,
            &ahora,
        )
        .await?;
    // Abre una sesión de jornada para esta instancia (timbrada por el broker).
    jornada::abrir_sesion(&e.almacen, &format!("ses-{id}-{ahora}"), &id, &ahora).await?;
    info!("instancia registrada: {id}");
    Ok(Json(RespuestaRegistrar { id }))
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

async fn enviar(
    State(e): State<Estado>,
    Json(p): Json<PeticionEnviar>,
) -> Result<Json<RespuestaEnviar>, ErrorApp> {
    // No descartar en silencio: si el destino no existe, error claro.
    if !e.almacen.instancia_existe(&p.para_id).await? {
        return Ok(Json(RespuestaEnviar {
            ok: false,
            error: Some(format!("La instancia '{}' no existe", p.para_id)),
        }));
    }
    e.almacen
        .encolar_mensaje(&p.de_id, &p.para_id, &p.texto, &ahora_iso())
        .await?;
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
    Ok(Json(RespuestaOk { ok: true }))
}

// --- Handlers de jornada (fase 2) ---

async fn tarea_abrir(
    State(e): State<Estado>,
    Json(p): Json<PeticionAbrirTarea>,
) -> Result<Json<RespuestaAbrirTarea>, ErrorApp> {
    let ahora = ahora_iso();
    let tarea_id = format!("tar-{}-{ahora}", p.instancia_id);
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
        &ahora,
    )
    .await?;

    // Integración GitHub: cose el issue_number en la tarea (mi fatia, no la de Aluísio).
    // Degradación graciosa: si GH no está o falla, la tarea sigue local.
    let mut issue_number = None;
    if let Some(gh) = &e.github {
        // Repo DINÁMICO: el owner/repo sale del repo_github de la instancia DUEÑA de la tarea
        // (el repo donde ese peer trabaja). Si la instancia no tiene repo_github → degradación:
        // no creamos issue, la tarea sigue local.
        match repo_de_instancia(&e, &p.instancia_id).await {
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

    Ok(Json(RespuestaAbrirTarea { tarea_id, issue_number }))
}

async fn tarea_reportar(
    State(e): State<Estado>,
    Json(p): Json<PeticionReportarTarea>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    // Si la tarea tiene issue y hay GitHub, comenta. Degrada si falla.
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

async fn tarea_cerrar(
    State(e): State<Estado>,
    Json(p): Json<PeticionCerrarTarea>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    let tarea = jornada::cerrar_tarea(&e.almacen, &p.tarea_id, &ahora_iso()).await?;
    if let Some(gh) = &e.github {
        if let Some(n) = tarea.issue_number {
            // Mismo repo dinámico: el de la instancia dueña de la tarea.
            if let Some((owner, repo)) = repo_de_instancia(&e, &tarea.instancia_id).await {
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
    Ok(Json(RespuestaOk { ok: true }))
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
    info!("admin: purgada la cola y outbox de '{}'", p.id);
    Ok(Json(RespuestaOk { ok: true }))
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

    let estado: Estado = Arc::new(EstadoApp {
        almacen,
        github,
        host: args.host.clone(),
        puerto: args.puerto,
    });

    // Limpieza periódica por latido (cada 30s).
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
        .route("/salir", post(salir))
        .route("/tarea/abrir", post(tarea_abrir))
        .route("/tarea/reportar", post(tarea_reportar))
        .route("/tarea/cerrar", post(tarea_cerrar))
        .route("/jornada", post(jornada_consolidada))
        // Admin (panel TUI): introspección protegida por el mismo token, nunca en /salud.
        .route("/admin/info", get(admin_info))
        .route("/admin/redis", get(admin_redis))
        .route("/admin/purgar", post(admin_purgar))
        .layer(from_fn_with_state(args.token.clone(), verificar_token));

    let app = Router::new()
        .route("/salud", get(salud))   // exenta de auth
        .merge(rutas_protegidas)
        .with_state(estado);

    let direccion = format!("{}:{}", args.host, args.puerto);
    let listener = tokio::net::TcpListener::bind(&direccion).await?;
    info!("peers-broker escuchando en {direccion}");
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
