//! `ClienteBroker`: cliente HTTP async (reqwest) hacia el broker de claude-peers.
//!
//! INTENCIÓN: espejo del cliente de la TUI (`peers-tui/src/cliente.rs`) pero async y limpio,
//! pensado para llamarse desde tareas GPUI (`cx.spawn`). Reusa los DTOs de `peers-core`; no
//! redefine el protocolo. Ninguna llamada hace `.unwrap()` sobre la red: la desktop no puede
//! crashear si el broker está caído o devuelve 401 — todo devuelve `Result<_, ErrorBroker>`
//! y la UI decide qué banner pintar según la variante.

use peers_core::*;
use reqwest::{Client, StatusCode};
use std::time::Duration;

/// URL base por defecto del broker (loopback, puerto fijo del proyecto).
const URL_BASE_DEFECTO: &str = "http://127.0.0.1:7899";
/// Token por defecto si no hay `CLAUDE_PEERS_TOKEN` en el entorno.
const TOKEN_DEFECTO: &str = "lexusfx-peers-2026";

/// Errores que la UI necesita distinguir para pintar el banner correcto. No es `anyhow`
/// porque la vista ramifica por variante (offline vs 401 vs otro), no sólo muestra un string.
#[derive(Debug, Clone)]
pub enum ErrorBroker {
    /// El broker no respondió (conexión rechazada, timeout, DNS…). → "broker offline".
    Offline(String),
    /// El broker respondió 401 → token ausente o inválido. → "token inválido (401)".
    NoAutorizado,
    /// Otro error HTTP (4xx/5xx) con código y cuerpo, o respuesta ilegible.
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
/// mensaje no existía. Se modela tipado (no vive en `peers-core`, es contrato del cliente)
/// para que la UI ramifique el caso "reenviado" vs "no existe" sin tocar `serde_json`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RespuestaReenviar {
    pub ok: bool,
    #[serde(default)]
    pub msg_id: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Respuesta de `POST /tarea/asignar`. El broker devuelve JSON ad-hoc `{ ok, tarea_id }`. No vive
/// en `peers-core` (es contorno del broker, no del dominio); se modela aquí tipado para extraer el
/// `tarea_id` sin tocar `serde_json`. `tarea_id` opcional → degrada sin crash si el broker no lo da.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RespuestaAsignar {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub tarea_id: Option<String>,
}

/// Cliente del broker para la desktop: URL base, token opcional y cliente HTTP reutilizable.
/// `Clone` barato (reqwest comparte el pool internamente) para pasarlo a cada `cx.spawn`.
#[derive(Clone)]
pub struct ClienteBroker {
    http: Client,
    base: String,
    token: Option<String>,
    /// Runtime tokio propio. reqwest exige un reactor tokio corriendo, pero las tareas de GPUI
    /// (`cx.spawn`/`cx.background_spawn`) NO corren sobre tokio → sin este runtime, cada `.await`
    /// de reqwest paniquea con "there is no reactor running". Se comparte por `Arc` (el cliente se
    /// clona en cada carga). Los métodos async se ejecutan con `bloquear_en` sobre este runtime.
    runtime: std::sync::Arc<tokio::runtime::Runtime>,
}

impl Default for ClienteBroker {
    /// Cliente listo para usar leyendo el entorno: URL fija del proyecto y token de
    /// `CLAUDE_PEERS_TOKEN` (o el default). Es el constructor que usa la app al arrancar.
    fn default() -> Self {
        let token = std::env::var("CLAUDE_PEERS_TOKEN").unwrap_or_else(|_| TOKEN_DEFECTO.to_string());
        Self::nuevo(URL_BASE_DEFECTO, Some(token))
    }
}

impl ClienteBroker {
    /// Construye el cliente. Si el builder de reqwest fallara (TLS roto, extremadamente raro),
    /// cae a `Client::default()` en vez de entrar en pánico — la app debe arrancar igual.
    pub fn nuevo(base: impl Into<String>, token: Option<String>) -> Self {
        let http = Client::builder()
            // Timeout acotado: evita que una petición colgada bloquee un refresco de pantalla.
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        // Runtime tokio dedicado (1 hilo basta: son peticiones HTTP cortas). Si su creación
        // fallara —extremadamente raro— no hay forma segura de degradar, así que se `expect`:
        // sin runtime el cliente no puede hacer NINGUNA petición y la app no tiene sentido.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("no se pudo crear el runtime tokio del cliente");
        Self {
            http,
            base: base.into(),
            token,
            runtime: std::sync::Arc::new(runtime),
        }
    }

    /// Ejecuta un future del cliente sobre su runtime tokio y devuelve el resultado de forma
    /// SÍNCRONA. Pensado para llamarse desde `cx.background_spawn` (hilo de fondo de GPUI): así
    /// reqwest tiene su reactor tokio y la UI no se bloquea. `f` es un closure que produce el
    /// future (para poder capturar `&self` sin problemas de lifetime).
    pub fn bloquear_en<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.runtime.block_on(f)
    }

    /// Mapea un error de transporte de reqwest a `ErrorBroker::Offline`.
    fn offline(e: reqwest::Error) -> ErrorBroker {
        ErrorBroker::Offline(e.to_string())
    }

    /// URL base configurada (sin barra final). La pantalla Acceso la muestra tal cual para que
    /// el jefe vea a qué broker apunta la desktop sin tener que leer variables de entorno.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Reconfigura la conexión EN CALIENTE: cambia URL base y/o token sin recrear el cliente HTTP
    /// ni el runtime tokio (ambos se conservan por su coste de creación). Lo usa la pantalla Acceso
    /// (acceso-01/02) cuando el jefe edita `broker_url`/`token` in-situ: `AppDesktop` llama a este
    /// método y luego dispara una recarga (`cargar_broker`) contra el nuevo destino.
    ///
    /// INTENCIÓN (por qué mutar en vez de construir uno nuevo): reconstruir con `nuevo(...)` crearía
    /// otro `Runtime` tokio y otro pool reqwest en cada cambio de broker; mutar los dos campos de
    /// conexión reusa ambos y deja el cliente listo al instante. `token` vacío → `None` (broker sin
    /// token, modo anónimo), coherente con `Config::guardar`. La URL se normaliza quitando barras
    /// finales para que `format!("{base}{ruta}")` no genere `//` (parse, don't validate en frontera).
    pub fn reconfigurar(&mut self, base: impl Into<String>, token: Option<String>) {
        let base = base.into();
        self.base = base.trim_end_matches('/').to_string();
        // Normaliza el token: una cadena vacía equivale a "sin token" (no se manda el header).
        self.token = match token {
            Some(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
            _ => None,
        };
    }

    /// `true` si el cliente NO tiene token configurado (modo anónimo). Lo usa la pantalla Acceso
    /// para el aviso de seguridad "broker expuesto sin token" (acceso-09): sin token + broker en un
    /// host no-loopback = agujero de seguridad. Evita exponer el `Option<String>` interno.
    pub fn sin_token(&self) -> bool {
        self.token.is_none()
    }

    /// Token enmascarado para pintarlo en pantalla sin revelarlo (ej. `lexus…2026`). NUNCA
    /// exponemos el token en claro en la UI: se delega en `enmascarar_token`, que deja pista
    /// mínima. Devuelve el mismo texto que usa la TUI para que ambas interfaces coincidan.
    pub fn token_enmascarado(&self) -> String {
        enmascarar_token(self.token.as_deref())
    }

    /// GET genérico que deserializa JSON. Distingue 401 → `NoAutorizado`.
    async fn get<R: serde::de::DeserializeOwned>(&self, ruta: &str) -> ResultadoBroker<R> {
        let url = format!("{}{}", self.base, ruta);
        let mut req = self.http.get(&url);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req.send().await.map_err(Self::offline)?;
        Self::deserializar(resp, ruta).await
    }

    /// GET con parámetros de query serializados desde una struct. Igual que `get` pero adjuntando
    /// `?campo=valor`; reqwest omite los `Option::None` (los DTOs marcan `skip_serializing_if`).
    /// Lo usa el historial de Trazabilidad (`/admin/historial?id=…`).
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
        Self::deserializar(resp, ruta).await
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
        Self::deserializar(resp, ruta).await
    }

    /// Convierte la respuesta HTTP en `R` o el `ErrorBroker` adecuado. NUNCA hace unwrap sobre red.
    async fn deserializar<R: serde::de::DeserializeOwned>(
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

    // --- Lecturas que alimentan las pantallas (Fase 1: sólo lectura) ---

    /// `GET /salud` → estado del broker y nº de instancias. Pantalla Broker.
    pub async fn salud(&self) -> ResultadoBroker<RespuestaSalud> {
        self.get("/salud").await
    }

    /// `POST /listar` con alcance Máquina → todos los peers de la máquina. Pantalla Peers.
    /// `directorio` va vacío: con alcance Máquina el broker lo ignora.
    pub async fn listar_instancias(&self) -> ResultadoBroker<Vec<Instancia>> {
        let p = PeticionListar {
            alcance: Alcance::Maquina,
            directorio: String::new(),
            repo_git: None,
            excluir_id: None,
        };
        self.post("/listar", &p).await
    }

    /// `GET /admin/tareas` → TODAS las tareas de TODAS las instancias, cada `Tarea` con su
    /// `instancia_id`, ordenadas por inicio desc por el broker. Pantalla Tareas.
    pub async fn admin_tareas(&self) -> ResultadoBroker<Vec<Tarea>> {
        self.get("/admin/tareas").await
    }

    /// `GET /admin/alertas` → alertas vigentes (SOLO LECTURA). Pantalla Alertas.
    pub async fn admin_alertas(&self) -> ResultadoBroker<Vec<Alerta>> {
        self.get("/admin/alertas").await
    }

    /// `POST /admin/alerta-resolver` → el jefe descarta una alerta a mano (botón "Descartar" de
    /// la pantalla Alertas). La clave de idempotencia de una alerta es el par `(tipo, sujeto)`
    /// (R7), así que se identifica por él; `tipo` viaja como la cadena serializada del
    /// `TipoAlerta` (p. ej. "ocioso", "cierre_sospechoso"). Única escritura de esta pantalla.
    pub async fn alerta_resolver(&self, tipo: &str, sujeto: &str) -> ResultadoBroker<RespuestaOk> {
        let p = PeticionResolverAlerta {
            tipo: tipo.to_string(),
            sujeto: sujeto.to_string(),
        };
        self.post("/admin/alerta-resolver", &p).await
    }

    /// `GET /admin/info` → host, puerto, nº instancias, versión. Pantalla Broker.
    pub async fn admin_info(&self) -> ResultadoBroker<RespuestaAdminInfo> {
        self.get("/admin/info").await
    }

    /// `GET /admin/redis` → colas y outbox pendientes por peer. Pantalla Redis.
    pub async fn admin_redis(&self) -> ResultadoBroker<RespuestaAdminRedis> {
        self.get("/admin/redis").await
    }

    /// `POST /admin/purgar` { id } → borra la cola de mensajes + el outbox de ese peer. Única
    /// escritura de la pantalla Redis. El broker la trata como idempotente, así que reintentar
    /// es seguro; devuelve `RespuestaOk` (la UI sólo necesita confirmar que no falló).
    pub async fn purgar(&self, id: &str) -> ResultadoBroker<RespuestaOk> {
        self.post("/admin/purgar", &PeticionPurgar { id: id.to_string() }).await
    }

    /// `GET /factor-estimacion` → factor de corrección global aprendido + nº de muestras.
    /// Pantalla Broker. El broker es quien lo aprende del tiempo real; aquí sólo se muestra.
    pub async fn factor_estimacion(&self) -> ResultadoBroker<FactorEstimacion> {
        self.get("/factor-estimacion").await
    }

    /// `GET /admin/historial?id=…` → historial durable de la cola de un peer, en orden
    /// cronológico ascendente. Pantalla Trazabilidad. Se pide la vista completa (sin `desde`
    /// ni `estado`): el filtrado por estado, si hace falta, se hace en cliente sobre la caché.
    pub async fn historial(&self, id: &str) -> ResultadoBroker<Vec<Mensaje>> {
        let q = PeticionHistorial {
            id: id.to_string(),
            desde: None,
            estado: None,
        };
        self.get_con_query("/admin/historial", &q).await
    }

    /// `POST /jornada {instancia_id}` → vista consolidada (sesiones + tareas con tiempos) de la
    /// jornada de un peer. Pantalla Jornada. Espejo de `peers_tui::cliente::jornada`: el broker es
    /// quien timbra los tiempos (la IA nunca los estima), aquí sólo se leen.
    pub async fn jornada(&self, instancia_id: &str) -> ResultadoBroker<RespuestaJornada> {
        let p = PeticionJornada {
            instancia_id: instancia_id.to_string(),
        };
        self.post("/jornada", &p).await
    }

    /// `POST /admin/reenviar {msg_id}` → re-encola un mensaje del historial como uno nuevo
    /// (R2.3), con estado `Enviado` y traza `reenviado_de`/`reenvios`. Acción `r` de la pantalla
    /// Trazabilidad. Devuelve la respuesta tipada para distinguir "reenviado" (`ok=true`) de
    /// "no existe" (`ok=false`), sin que la UI toque `serde_json`.
    pub async fn reenviar(&self, msg_id: i64) -> ResultadoBroker<RespuestaReenviar> {
        self.post("/admin/reenviar", &PeticionReenviar { msg_id }).await
    }

    /// `POST /enviar {de_id, para_id, texto}` → encola un mensaje directo al peer (peers-02).
    /// El remitente es la propia desktop (`de_id` fijo "peers-desktop"), espejo del `de_id`
    /// "peers-tui" del cliente de la TUI: el peer sabe así que le escribe el panel del jefe, no
    /// otra instancia. Devuelve `RespuestaEnviar` tipada (`ok`/`error`) para el toast.
    pub async fn enviar(&self, para_id: &str, texto: &str) -> ResultadoBroker<RespuestaEnviar> {
        let p = PeticionEnviar {
            de_id: "peers-desktop".to_string(),
            para_id: para_id.to_string(),
            texto: texto.to_string(),
        };
        self.post("/enviar", &p).await
    }

    /// `POST /salir {id}` → da de baja al peer (kick, peers-03). Espejo de la tecla `k` de la
    /// TUI (`ClienteAdmin::salir`). La desktop SIEMPRE pide confirmación antes (acción
    /// destructiva: cierra la presencia del peer en la red).
    pub async fn salir(&self, id: &str) -> ResultadoBroker<RespuestaOk> {
        self.post("/salir", &PeticionSalir { id: id.to_string() }).await
    }

    // --- Acciones de gestión de tareas (Pantalla Tareas, R5/R6/R7/R8) ---
    // El jefe opera sobre las tareas de sus peers. Cada acción es un POST idempotente en el
    // broker; la desktop dispara y luego refresca la lista con `admin_tareas`.

    /// `POST /tarea/asignar {instancia_id, descripcion, estimado_seg?}` → crea una tarea NUEVA
    /// para el peer y le avisa por canal (R6). Devuelve el `tarea_id` recién creado (o `None` si
    /// el broker no lo incluyó). Es el "dale trabajo a este peer".
    pub async fn tarea_asignar(
        &self,
        instancia_id: &str,
        descripcion: &str,
        estimado_seg: Option<i64>,
    ) -> ResultadoBroker<RespuestaAsignar> {
        let p = PeticionAsignarTarea {
            instancia_id: instancia_id.to_string(),
            descripcion: descripcion.to_string(),
            estimado_seg,
        };
        self.post("/tarea/asignar", &p).await
    }

    /// `POST /tarea/reasignar {tarea_id, nuevo_instancia_id}` → cambia el dueño de una tarea
    /// existente al peer indicado y le notifica (R7). Devuelve la `Tarea` ya con el nuevo dueño.
    pub async fn tarea_reasignar(
        &self,
        tarea_id: &str,
        nuevo_instancia_id: &str,
    ) -> ResultadoBroker<Tarea> {
        let p = PeticionReasignarTarea {
            tarea_id: tarea_id.to_string(),
            nuevo_instancia_id: nuevo_instancia_id.to_string(),
        };
        self.post("/tarea/reasignar", &p).await
    }

    /// `POST /tarea/editar {tarea_id, descripcion?, estimado_seg?}` → parche PARCIAL de metadatos
    /// (R4): `None` no toca ese campo. Lo usan EDITAR descripción (tareas-05) y AMPLIAR estimado
    /// (tareas-06); el broker valida el rango plausible del estimado y responde 404 si la tarea no
    /// existe. Devuelve la `Tarea` ya parcheada. Espejo de `peers_tui::cliente::tarea_editar`.
    pub async fn tarea_editar(
        &self,
        tarea_id: &str,
        descripcion: Option<String>,
        estimado_seg: Option<i64>,
    ) -> ResultadoBroker<Tarea> {
        let p = PeticionEditarTarea {
            tarea_id: tarea_id.to_string(),
            descripcion,
            estimado_seg,
        };
        self.post("/tarea/editar", &p).await
    }

    /// `POST /tarea/forzar {tarea_id}` → "tócale el hombro": empuja un recordatorio a la sesión
    /// del peer dueño (R8). No cambia el estado, sólo notifica; `RespuestaOk` basta.
    pub async fn tarea_forzar(&self, tarea_id: &str) -> ResultadoBroker<RespuestaOk> {
        let p = PeticionForzarTarea {
            tarea_id: tarea_id.to_string(),
        };
        self.post("/tarea/forzar", &p).await
    }

    /// `POST /tarea/estado {tarea_id, estado, motivo?, evidencia?}` → transiciona el ciclo de
    /// vida (R5), validado por el broker con `transicion_valida`. La usa el botón "Cerrar" de la
    /// pantalla Tareas (→ `Hecha`). `motivo` sólo relevante al `Bloqueada`; `evidencia` al
    /// `Hecha`. Devuelve la `Tarea` ya en el nuevo estado.
    pub async fn tarea_estado(
        &self,
        tarea_id: &str,
        estado: EstadoTarea,
        motivo: Option<String>,
        evidencia: Option<String>,
    ) -> ResultadoBroker<Tarea> {
        let p = PeticionEstadoTarea {
            tarea_id: tarea_id.to_string(),
            estado,
            motivo,
            evidencia,
        };
        self.post("/tarea/estado", &p).await
    }
}

/// Enmascara un token para mostrarlo en pantalla sin revelarlo: deja los primeros 5 y los
/// últimos 4 caracteres, con `…` en medio (ej. `lexus…2026`). Tokens de 9 caracteres o menos
/// se ocultan por completo (no dan pista sin filtrar). `None`/vacío → "(sin token)".
///
/// INTENCIÓN: espejo exacto de `peers_tui::config::enmascarar_token`. Se replica aquí porque
/// vive en otro crate (la TUI no expone su config como librería reutilizable) y el contrato es
/// trivial y estable; duplicar unas líneas es más barato que acoplar desktop al crate de la TUI.
fn enmascarar_token(token: Option<&str>) -> String {
    match token {
        None | Some("") => "(sin token)".to_string(),
        Some(t) => {
            let n = t.chars().count();
            // Demasiado corto para dejar pista sin filtrar → se oculta entero.
            if n <= 9 {
                "•".repeat(n)
            } else {
                let prefijo: String = t.chars().take(5).collect();
                let sufijo: String = t
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("{prefijo}…{sufijo}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enmascara_token_largo_deja_pista_minima() {
        assert_eq!(enmascarar_token(Some("lexusfx-peers-2026")), "lexus…2026");
    }

    #[test]
    fn enmascara_none_y_vacio() {
        assert_eq!(enmascarar_token(None), "(sin token)");
        assert_eq!(enmascarar_token(Some("")), "(sin token)");
    }

    #[test]
    fn enmascara_token_corto_se_oculta_entero() {
        assert_eq!(enmascarar_token(Some("123456789")), "•••••••••");
    }
}
