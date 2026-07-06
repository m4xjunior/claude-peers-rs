//! Cliente HTTP hacia el `peers-broker`.
//!
//! Toda la comunicación con el broker pasa por aquí. La URL es configurable (FIX #3 frente
//! al TS, que la tenía hardcoded a 127.0.0.1): el client puede apuntar a un broker remoto
//! a través de un túnel/forward. Si el broker no responde, el error es claro y no entra en
//! pánico — la instancia sigue viva y reintenta en el siguiente latido.

use anyhow::{Context, Result};
use peers_core::*;
use reqwest::Client;

/// Cliente del broker: guarda la URL base, el token opcional y un cliente HTTP reutilizable.
#[derive(Clone)]
pub struct ClienteBroker {
    http: Client,
    base: String,
    token: Option<String>,
}

impl ClienteBroker {
    pub fn nuevo(base: impl Into<String>, token: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("cliente HTTP no pudo construirse");
        Self {
            http,
            base: base.into(),
            token,
        }
    }

    /// POST genérico con cuerpo JSON que deserializa la respuesta. Mensaje de error
    /// explícito que incluye la URL, para diagnosticar "broker offline" sin ambigüedad.
    async fn post<B: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        ruta: &str,
        cuerpo: &B,
    ) -> Result<R> {
        let url = format!("{}{}", self.base, ruta);
        let mut req = self.http.post(&url).json(cuerpo);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("broker no responde en {url} (¿está levantado?)"))?;
        if !resp.status().is_success() {
            let estado = resp.status();
            let texto = resp.text().await.unwrap_or_default();
            anyhow::bail!("error del broker ({ruta}): {estado} {texto}");
        }
        resp.json::<R>()
            .await
            .with_context(|| format!("respuesta inválida del broker en {ruta}"))
    }

    /// Comprueba si el broker está vivo (GET /salud). No lanza: devuelve false si no responde.
    pub async fn esta_vivo(&self) -> bool {
        let url = format!("{}/salud", self.base);
        let mut req = self.http.get(&url);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        match req.send().await {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn registrar(&self, p: &PeticionRegistrar) -> Result<RespuestaRegistrar> {
        self.post("/registrar", p).await
    }

    pub async fn latido(&self, id: &str) -> Result<RespuestaOk> {
        self.post("/latido", &PeticionLatido { id: id.to_string() }).await
    }

    pub async fn definir_resumen(&self, id: &str, resumen: &str) -> Result<RespuestaOk> {
        self.post(
            "/definir-resumen",
            &PeticionDefinirResumen {
                id: id.to_string(),
                resumen: resumen.to_string(),
            },
        )
        .await
    }

    pub async fn listar(&self, p: &PeticionListar) -> Result<Vec<Instancia>> {
        self.post("/listar", p).await
    }

    /// Envío SIN secreto de sesión. Tras E-10, las tools usan `enviar_verificado` (presenta el
    /// header). Se conserva como API base del cliente (y por si un flujo sin identidad lo necesita);
    /// `enviar_verificado(p, None)` es equivalente. `allow(dead_code)`: hoy no hay call-site directo.
    #[allow(dead_code)]
    pub async fn enviar(&self, p: &PeticionEnviar) -> Result<RespuestaEnviar> {
        self.post("/enviar", p).await
    }

    /// Envía presentando el secreto de sesión (E-10) en el header `X-Peers-Secreto`, para que el
    /// broker ate el `de_id` a esta instancia. `secreto = None` (broker viejo sin E-10) → cae al
    /// `enviar` normal: sin header, el broker aplica su ventana de compat. Reusa `post_con_secreto`
    /// para no duplicar el manejo de token/errores del `post` genérico.
    pub async fn enviar_verificado(
        &self,
        p: &PeticionEnviar,
        secreto: Option<&str>,
    ) -> Result<RespuestaEnviar> {
        match secreto {
            Some(s) => self.post_con_secreto("/enviar", p, s).await,
            None => self.post("/enviar", p).await,
        }
    }

    /// Igual que `post` pero añade el header del secreto de sesión (E-10). Se mantiene separado del
    /// `post` genérico para no cambiar la firma de las decenas de call-sites que no lo necesitan;
    /// solo las rutas con identidad verificada (hoy `/enviar`) lo usan.
    async fn post_con_secreto<B: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        ruta: &str,
        cuerpo: &B,
        secreto: &str,
    ) -> Result<R> {
        let url = format!("{}{}", self.base, ruta);
        let mut req = self
            .http
            .post(&url)
            .json(cuerpo)
            .header(HEADER_SECRETO, secreto);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("broker no responde en {url} (¿está levantado?)"))?;
        if !resp.status().is_success() {
            let estado = resp.status();
            let texto = resp.text().await.unwrap_or_default();
            anyhow::bail!("error del broker ({ruta}): {estado} {texto}");
        }
        resp.json::<R>()
            .await
            .with_context(|| format!("respuesta inválida del broker en {ruta}"))
    }

    pub async fn recibir(&self, id: &str) -> Result<RespuestaRecibir> {
        self.post("/recibir", &PeticionRecibir { id: id.to_string() }).await
    }

    /// Chat privado (RFC-lanzador §7): el peer drena (pull) su cola de ENTRADA (mensajes del
    /// operador). El broker resuelve la identidad por el SECRETO (anti-IDOR), no por el body — por
    /// eso hay que presentarlo. `sesion_id` en el body es informativo (el broker lo ignora). Sin
    /// secreto no se puede leer: `None` → el broker devuelve vacío (no filtra).
    pub async fn chat_privado_recibir(
        &self,
        secreto: Option<&str>,
    ) -> Result<RespuestaChatPrivadoRecibir> {
        // El broker resuelve el sesion_id por el secreto; mandamos "" como placeholder (se ignora).
        let cuerpo = PeticionChatPrivadoRecibir { sesion_id: String::new() };
        match secreto {
            Some(s) => self.post_con_secreto("/chat-privado/recibir", &cuerpo, s).await,
            None => self.post("/chat-privado/recibir", &cuerpo).await,
        }
    }

    /// Chat privado (RFC-lanzador §7): el peer responde al operador (va a su cola de SALIDA). El
    /// broker fija el `de` al id real del peer resuelto por el secreto (anti-IDOR). Sin secreto,
    /// el broker responde ok:false (no se puede atribuir la respuesta).
    pub async fn chat_privado_responder(
        &self,
        texto: &str,
        secreto: Option<&str>,
    ) -> Result<RespuestaOk> {
        let cuerpo = PeticionChatPrivadoEnviar { sesion_id: String::new(), texto: texto.to_string() };
        match secreto {
            Some(s) => self.post_con_secreto("/chat-privado/responder", &cuerpo, s).await,
            None => self.post("/chat-privado/responder", &cuerpo).await,
        }
    }

    pub async fn salir(&self, id: &str) -> Result<RespuestaOk> {
        self.post("/salir", &PeticionSalir { id: id.to_string() }).await
    }

    /// Confirma al broker la transición de estado de uno o más mensajes (R1.4).
    /// El broker timbra el estado con SU reloj (idempotente y monótono). Ruta protegida (token).
    pub async fn confirmar(&self, ids: &[i64], estado: EstadoMensaje) -> Result<RespuestaOk> {
        self.post(
            "/confirmar",
            &PeticionConfirmar {
                ids: ids.to_vec(),
                estado,
            },
        )
        .await
    }

    /// Crea una tarea con el estimado ingenuo de la IA. El broker timbra el inicio con SU
    /// reloj (regla sagrada: la IA nunca timbra el tiempo) y devuelve el estimado ya corregido
    /// por el factor aprendido (`estimado/factor`), más el factor vigente y sus muestras.
    pub async fn crear_tarea(
        &self,
        instancia_id: &str,
        descripcion: &str,
        estimado_seg: Option<i64>,
    ) -> Result<RespuestaAbrirTarea> {
        self.post(
            "/crear-tarea",
            &PeticionAbrirTarea {
                instancia_id: instancia_id.to_string(),
                descripcion: descripcion.to_string(),
                area: None,
                estimado_seg,
            },
        )
        .await
    }

    /// Cierra una tarea por su id. El broker mide el tiempo REAL (timbra el fin con su reloj)
    /// y, si la tarea traía estimado, aprende el factor de corrección de la diferencia.
    pub async fn cerrar_tarea(&self, tarea_id: &str) -> Result<RespuestaOk> {
        self.post(
            "/cerrar-tarea",
            &PeticionCerrarTarea {
                tarea_id: tarea_id.to_string(),
                // La tool MCP cierra sin evidencia explícita; el broker mide el tiempo real igual.
                evidencia: None,
            },
        )
        .await
    }

    /// Añade una nota de progreso a una tarea abierta (no mide tiempo ni la cierra).
    pub async fn reportar_tarea(&self, tarea_id: &str, texto: &str) -> Result<RespuestaOk> {
        self.post(
            "/tarea/reportar",
            &PeticionReportarTarea {
                tarea_id: tarea_id.to_string(),
                texto: texto.to_string(),
            },
        )
        .await
    }

    /// Lista las tareas de una instancia (con sus tiempos timbrados por el broker).
    pub async fn listar_tareas(&self, instancia_id: &str) -> Result<Vec<Tarea>> {
        self.post(
            "/listar-tareas",
            &PeticionJornada {
                instancia_id: instancia_id.to_string(),
            },
        )
        .await
    }
}
