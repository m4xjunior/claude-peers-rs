//! peers-broker — daemon HTTP de la red claude-peers-rs.
//!
//! Servidor axum+tokio respaldado por SQLite. Registra las instancias de Claude Code,
//! rutea mensajes entre ellas y mantiene la liveness por latido (no por PID, para que
//! funcione cross-host). Binario único sin dependencias externas (SQLite va embebido).
//!
//! No entra en pánico en producción: anyhow en `main`, todos los handlers devuelven
//! `Result` y traducen el error a un 500 con JSON claro.

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
use db::Base;
use peers_core::{
    PeticionDefinirResumen, PeticionEnviar, PeticionLatido, PeticionListar,
    PeticionRecibir, PeticionRegistrar, PeticionSalir, RespuestaEnviar, RespuestaOk,
    RespuestaRecibir, RespuestaRegistrar, RespuestaSalud, PUERTO_DEFECTO, VENCIMIENTO_MS,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tracing::{error, info};

/// Argumentos de línea de comandos. Todos con variable de entorno equivalente.
#[derive(Parser, Debug)]
#[command(name = "peers-broker", about = "Daemon de la red claude-peers-rs")]
struct Args {
    /// Puerto de escucha.
    #[arg(long, env = "CLAUDE_PEERS_PORT", default_value_t = PUERTO_DEFECTO)]
    puerto: u16,

    /// Dirección de bind. Por defecto solo localhost; usar 0.0.0.0 para exponer cross-host
    /// (detrás de túnel/forward).
    #[arg(long, env = "CLAUDE_PEERS_HOST", default_value = "127.0.0.1")]
    host: String,

    /// Ruta del archivo SQLite.
    #[arg(long, env = "CLAUDE_PEERS_DB")]
    db: Option<String>,
}

/// Estado de la aplicación compartido entre handlers.
struct EstadoApp {
    base: Base,
}

type Estado = Arc<EstadoApp>;

/// Timestamp ISO 8601 (UTC) del instante actual. Formato comparable lexicográficamente,
/// que es lo que permite filtrar la liveness con un simple `>=` sobre strings.
fn ahora_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

/// Timestamp ISO del límite de vencimiento: `now - VENCIMIENTO_MS`.
fn vencidas_antes_iso() -> String {
    let limite = OffsetDateTime::now_utc()
        - time::Duration::milliseconds(VENCIMIENTO_MS);
    limite
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

/// Convierte cualquier error en un 500 con JSON `{ "error": "..." }`. Evita el panic.
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

// --- Handlers ---

async fn salud(State(estado): State<Estado>) -> Json<RespuestaSalud> {
    Json(RespuestaSalud {
        estado: "ok".into(),
        instancias: estado.base.contar_instancias(),
    })
}

async fn registrar(
    State(estado): State<Estado>,
    Json(p): Json<PeticionRegistrar>,
) -> Result<Json<RespuestaRegistrar>, ErrorApp> {
    // ID estable: si la instancia manda id_preferido, esa es su identidad persistente.
    // Si no, generamos uno aleatorio (caso sin papel asignado).
    let id = p.id_preferido.clone().unwrap_or_else(generar_id);
    let ahora = ahora_iso();
    estado.base.registrar(
        &id,
        p.pid,
        &p.directorio,
        p.repo_git.as_deref(),
        p.tty.as_deref(),
        &p.resumen,
        &ahora,
    )?;
    info!("instancia registrada: {id}");
    Ok(Json(RespuestaRegistrar { id }))
}

async fn latido(
    State(estado): State<Estado>,
    Json(p): Json<PeticionLatido>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    estado.base.latido(&p.id, &ahora_iso())?;
    Ok(Json(RespuestaOk { ok: true }))
}

async fn definir_resumen(
    State(estado): State<Estado>,
    Json(p): Json<PeticionDefinirResumen>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    estado.base.definir_resumen(&p.id, &p.resumen)?;
    Ok(Json(RespuestaOk { ok: true }))
}

async fn listar(
    State(estado): State<Estado>,
    Json(p): Json<PeticionListar>,
) -> Result<Json<Vec<peers_core::Instancia>>, ErrorApp> {
    let instancias = estado.base.listar(
        p.alcance,
        &p.directorio,
        p.repo_git.as_deref(),
        p.excluir_id.as_deref(),
        &vencidas_antes_iso(),
    )?;
    Ok(Json(instancias))
}

async fn enviar(
    State(estado): State<Estado>,
    Json(p): Json<PeticionEnviar>,
) -> Result<Json<RespuestaEnviar>, ErrorApp> {
    // FIX frente al TS conservado: si el destino no existe NO se descarta en silencio,
    // se devuelve un error claro para que el emisor sepa que su mensaje no llegó.
    if !estado.base.instancia_existe(&p.para_id) {
        return Ok(Json(RespuestaEnviar {
            ok: false,
            error: Some(format!("La instancia '{}' no existe", p.para_id)),
        }));
    }
    estado
        .base
        .encolar_mensaje(&p.de_id, &p.para_id, &p.texto, &ahora_iso())?;
    Ok(Json(RespuestaEnviar {
        ok: true,
        error: None,
    }))
}

async fn recibir(
    State(estado): State<Estado>,
    Json(p): Json<PeticionRecibir>,
) -> Result<Json<RespuestaRecibir>, ErrorApp> {
    let mensajes = estado.base.recibir_mensajes(&p.id)?;
    Ok(Json(RespuestaRecibir { mensajes }))
}

async fn salir(
    State(estado): State<Estado>,
    Json(p): Json<PeticionSalir>,
) -> Result<Json<RespuestaOk>, ErrorApp> {
    estado.base.salir(&p.id)?;
    Ok(Json(RespuestaOk { ok: true }))
}

/// Genera un id aleatorio de 8 caracteres (minúsculas + dígitos). Solo se usa cuando una
/// instancia se registra sin id_preferido.
fn generar_id() -> String {
    // Fuente de entropía simple sin dependencias extra: nanosegundos del reloj + pid,
    // mezclados con un xorshift. Suficiente para un identificador no colisionante local.
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let ruta_db = args
        .db
        .clone()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.claude-peers.db")))
        .unwrap_or_else(|| "./claude-peers.db".into());

    let base = Base::abrir(&ruta_db)?;
    // Limpieza inicial: quita lo que quedó muerto de una sesión previa.
    if let Ok(n) = base.limpiar_vencidas(&vencidas_antes_iso()) {
        if n > 0 {
            info!("limpieza inicial: {n} instancia(s) vencida(s) eliminada(s)");
        }
    }

    let estado: Estado = Arc::new(EstadoApp { base });

    // Task periódica de limpieza por latido (cada 30s), igual que el TS.
    let estado_limpieza = estado.clone();
    tokio::spawn(async move {
        let mut intervalo = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            intervalo.tick().await;
            match estado_limpieza.base.limpiar_vencidas(&vencidas_antes_iso()) {
                Ok(n) if n > 0 => info!("limpieza periódica: {n} instancia(s) vencida(s)"),
                Ok(_) => {}
                Err(e) => error!("fallo en limpieza periódica: {e:#}"),
            }
        }
    });

    let app = Router::new()
        .route("/salud", get(salud))
        .route("/registrar", post(registrar))
        .route("/latido", post(latido))
        .route("/definir-resumen", post(definir_resumen))
        .route("/listar", post(listar))
        .route("/enviar", post(enviar))
        .route("/recibir", post(recibir))
        .route("/salir", post(salir))
        .with_state(estado);

    let direccion = format!("{}:{}", args.host, args.puerto);
    let listener = tokio::net::TcpListener::bind(&direccion).await?;
    info!("peers-broker escuchando en {direccion} (db: {ruta_db})");

    axum::serve(listener, app).await?;
    Ok(())
}
