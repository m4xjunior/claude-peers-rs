//! `ClienteAdmin`: cliente HTTP hacia el broker para la TUI.
//!
//! Reusa el patrón de `peers-client/src/broker.rs` — un cliente reqwest reutilizable que
//! inyecta `X-Peers-Token` en cada petición. La diferencia clave es la ROBUSTEZ: la TUI no
//! puede crashear si el broker está caído o devuelve 401. Por eso ninguna llamada hace
//! `.unwrap()` sobre la red; todo devuelve `Result` y los handlers de pantalla distinguen
//! explícitamente el caso "offline" del caso "401 token inválido" mediante `ErrorBroker`.

use peers_core::*;
use reqwest::{Client, StatusCode};
use std::time::Duration;

/// Errores que la TUI necesita distinguir para pintar el banner correcto. NO es `anyhow`
/// porque la UI ramifica por variante (offline vs 401 vs otro), no solo muestra un string.
#[derive(Debug, Clone)]
pub enum ErrorBroker {
    /// El broker no respondió (conexión rechazada, timeout, DNS…). → banner "broker offline".
    Offline(String),
    /// El broker respondió 401 → token ausente o inválido. → banner "token inválido (401)".
    NoAutorizado,
    /// Otro error HTTP (4xx/5xx) con su código y cuerpo, o respuesta ilegible.
    Otro(String),
}

impl std::fmt::Display for ErrorBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorBroker::Offline(d) => write!(f, "broker offline ({d})"),
            ErrorBroker::NoAutorizado => write!(f, "token inválido (401)"),
            ErrorBroker::Otro(d) => write!(f, "{d}"),
        }
    }
}

/// Resultado de una llamada al broker. Alias para legibilidad en las firmas.
pub type ResultadoBroker<T> = Result<T, ErrorBroker>;

/// Respuesta de `POST /admin/reenviar`. El broker responde con un JSON ad-hoc:
/// `{ "ok": true, "msg_id": <i64> }` si reenvió, o `{ "ok": false, "error": "..." }` si el
/// mensaje no existía. Lo modelamos tipado para que la TUI ramifique sin tocar `serde_json`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RespuestaReenviar {
    pub ok: bool,
    #[serde(default)]
    pub msg_id: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Cliente del broker para la TUI: URL base, token opcional y cliente HTTP reutilizable.
#[derive(Clone)]
pub struct ClienteAdmin {
    http: Client,
    base: String,
    token: Option<String>,
}

impl ClienteAdmin {
    /// Construye el cliente. Si el builder de reqwest fallara (extremadamente raro: TLS roto),
    /// cae a `Client::default()` en vez de entrar en pánico — la TUI debe arrancar igual.
    pub fn nuevo(base: impl Into<String>, token: Option<String>) -> Self {
        let http = Client::builder()
            // Timeout corto: el loop de la TUI es secuencial, así que una petición lenta
            // congela el render hasta este límite. 1.5s acota el peor caso (broker que acepta
            // la conexión pero no responde) sin afectar el caso normal (respuesta en ms).
            .timeout(Duration::from_millis(1500))
            .build()
            .unwrap_or_default();
        Self {
            http,
            base: base.into(),
            token,
        }
    }

    /// Mapea un error de transporte de reqwest a `ErrorBroker::Offline`.
    fn offline(e: reqwest::Error) -> ErrorBroker {
        ErrorBroker::Offline(e.to_string())
    }

    /// GET genérico que deserializa JSON. Distingue 401 → `NoAutorizado`.
    async fn get<R: serde::de::DeserializeOwned>(&self, ruta: &str) -> ResultadoBroker<R> {
        let url = format!("{}{}", self.base, ruta);
        let mut req = self.http.get(&url);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req.send().await.map_err(Self::offline)?;
        self.deserializar(resp, ruta).await
    }

    /// GET con query string serializada desde `q` (usa el `Serialize` del tipo). Reqwest
    /// codifica los pares; los `Option` con `skip_serializing_if` no aparecen si son `None`.
    async fn get_con_query<Q: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        ruta: &str,
        q: &Q,
    ) -> ResultadoBroker<R> {
        let url = format!("{}{}", self.base, ruta);
        let mut req = self.http.get(&url).query(q);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req.send().await.map_err(Self::offline)?;
        self.deserializar(resp, ruta).await
    }

    /// POST genérico con cuerpo JSON. Distingue 401 → `NoAutorizado`.
    async fn post<B: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        ruta: &str,
        cuerpo: &B,
    ) -> ResultadoBroker<R> {
        let url = format!("{}{}", self.base, ruta);
        let mut req = self.http.post(&url).json(cuerpo);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req.send().await.map_err(Self::offline)?;
        self.deserializar(resp, ruta).await
    }

    /// Convierte la respuesta HTTP en `R` o el `ErrorBroker` adecuado. NUNCA hace unwrap.
    async fn deserializar<R: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
        ruta: &str,
    ) -> ResultadoBroker<R> {
        let estado = resp.status();
        if estado == StatusCode::UNAUTHORIZED {
            return Err(ErrorBroker::NoAutorizado);
        }
        if !estado.is_success() {
            let texto = resp.text().await.unwrap_or_default();
            return Err(ErrorBroker::Otro(format!("{ruta}: {estado} {texto}")));
        }
        resp.json::<R>()
            .await
            .map_err(|e| ErrorBroker::Otro(format!("respuesta inválida en {ruta}: {e}")))
    }

    // --- Llamadas concretas usadas por las pantallas ---

    /// `GET /salud` → estado del broker. Para la pantalla Broker.
    pub async fn salud(&self) -> ResultadoBroker<RespuestaSalud> {
        self.get("/salud").await
    }

    /// `GET /admin/info` → host, puerto, instancias, version. Pantalla Broker.
    pub async fn admin_info(&self) -> ResultadoBroker<RespuestaAdminInfo> {
        self.get("/admin/info").await
    }

    /// `GET /factor-estimacion` → el factor de corrección global aprendido + nº de muestras
    /// (R10/R11). Lo muestra la pantalla Broker. Degrada como el resto: si falla, el banner
    /// lo refleja y la pantalla conserva el último factor bueno (o muestra "(sin datos)").
    pub async fn factor_estimacion(&self) -> ResultadoBroker<FactorEstimacion> {
        self.get("/factor-estimacion").await
    }

    /// `GET /admin/redis` → colas y outbox pendientes por peer. Pantalla Redis.
    pub async fn admin_redis(&self) -> ResultadoBroker<RespuestaAdminRedis> {
        self.get("/admin/redis").await
    }

    /// `POST /admin/purgar` → borra cola + outbox de un id. Acción 'p' en pantalla Redis.
    pub async fn purgar(&self, id: &str) -> ResultadoBroker<RespuestaOk> {
        self.post("/admin/purgar", &PeticionPurgar { id: id.to_string() }).await
    }

    /// `POST /listar` con alcance Máquina → todos los peers de la máquina. Pantalla Peers.
    /// `directorio` se manda vacío: con alcance Máquina el broker lo ignora.
    pub async fn listar(&self) -> ResultadoBroker<Vec<Instancia>> {
        let p = PeticionListar {
            alcance: Alcance::Maquina,
            directorio: String::new(),
            repo_git: None,
            excluir_id: None,
        };
        self.post("/listar", &p).await
    }

    /// `POST /enviar` → encola un mensaje a un peer. Acción 'm' en pantalla Peers.
    /// El remitente es la propia TUI (`de_id` fijo) para que el peer sepa que viene del panel.
    pub async fn enviar(&self, para_id: &str, texto: &str) -> ResultadoBroker<RespuestaEnviar> {
        let p = PeticionEnviar {
            de_id: "peers-tui".to_string(),
            para_id: para_id.to_string(),
            texto: texto.to_string(),
        };
        self.post("/enviar", &p).await
    }

    /// `POST /salir` → da de baja a un peer (kick). Acción 'k' en pantalla Peers.
    pub async fn salir(&self, id: &str) -> ResultadoBroker<RespuestaOk> {
        self.post("/salir", &PeticionSalir { id: id.to_string() }).await
    }

    /// `POST /definir-resumen` → cambia el resumen de un peer. Acción 'r' en pantalla Peers.
    pub async fn definir_resumen(&self, id: &str, resumen: &str) -> ResultadoBroker<RespuestaOk> {
        let p = PeticionDefinirResumen {
            id: id.to_string(),
            resumen: resumen.to_string(),
        };
        self.post("/definir-resumen", &p).await
    }

    /// `GET /admin/historial?id=…` → historial durable de la cola de un peer (orden
    /// cronológico). Pantalla Trazabilidad. Pedimos sin `desde`/`estado` (vista completa);
    /// el filtrado por estado se hace en cliente sobre la lista cacheada.
    pub async fn historial(&self, id: &str) -> ResultadoBroker<Vec<Mensaje>> {
        let q = PeticionHistorial {
            id: id.to_string(),
            desde: None,
            estado: None,
        };
        self.get_con_query("/admin/historial", &q).await
    }

    /// `POST /admin/reenviar {msg_id}` → re-encola un mensaje del historial como uno nuevo
    /// (R2.3). Tecla `r` en la pantalla Trazabilidad. Devuelve la respuesta tipada para que
    /// la UI distinga el caso "reenviado" (ok=true) del "no existe" (ok=false).
    pub async fn reenviar(&self, msg_id: i64) -> ResultadoBroker<RespuestaReenviar> {
        self.post("/admin/reenviar", &PeticionReenviar { msg_id }).await
    }
}
