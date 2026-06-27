//! Cliente HTTP hacia el `peers-broker`.
//!
//! Toda la comunicación con el broker pasa por aquí. La URL es configurable (FIX #3 frente
//! al TS, que la tenía hardcoded a 127.0.0.1): el client puede apuntar a un broker remoto
//! a través de un túnel/forward. Si el broker no responde, el error es claro y no entra en
//! pánico — la instancia sigue viva y reintenta en el siguiente latido.

use anyhow::{Context, Result};
use peers_core::*;
use reqwest::Client;

/// Cliente del broker: guarda la URL base y un cliente HTTP reutilizable.
#[derive(Clone)]
pub struct ClienteBroker {
    http: Client,
    base: String,
}

impl ClienteBroker {
    pub fn nuevo(base: impl Into<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("cliente HTTP no pudo construirse");
        Self {
            http,
            base: base.into(),
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
        let resp = self
            .http
            .post(&url)
            .json(cuerpo)
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
        match self.http.get(&url).send().await {
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

    pub async fn enviar(&self, p: &PeticionEnviar) -> Result<RespuestaEnviar> {
        self.post("/enviar", p).await
    }

    pub async fn recibir(&self, id: &str) -> Result<RespuestaRecibir> {
        self.post("/recibir", &PeticionRecibir { id: id.to_string() }).await
    }

    pub async fn salir(&self, id: &str) -> Result<RespuestaOk> {
        self.post("/salir", &PeticionSalir { id: id.to_string() }).await
    }
}
