//! Carga/guardado de la config de la TUI en `~/.config/claude-peers/config.toml`.
//!
//! INTENCIÓN: la TUI nunca debe crashear por falta de config. Si el archivo no existe o está
//! corrupto, se cae a defaults sensatos (broker local sin token, refresh 1s) en vez de entrar
//! en pánico. El guardado es explícito (tecla 's' en la pantalla Config), nunca implícito.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// URL por defecto del broker: loopback en el puerto estándar (`PUERTO_DEFECTO`).
fn broker_url_defecto() -> String {
    format!("http://127.0.0.1:{}", peers_core::PUERTO_DEFECTO)
}

/// Refresco por defecto: 1s. Suficiente para una red de pocos peers sin martillar el broker.
const REFRESH_MS_DEFECTO: u64 = 1000;

/// Config persistida de la TUI. `token` es opcional: si el broker corre sin token, se omite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    /// URL base del broker (sin barra final), p.ej. `http://127.0.0.1:7899`.
    #[serde(default = "broker_url_defecto")]
    pub broker_url: String,
    /// Token de protocolo (X-Peers-Token). None si el broker no exige token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Periodo de refresco de las pantallas vivas, en milisegundos.
    #[serde(default = "refresh_ms_defecto")]
    pub refresh_ms: u64,
}

fn refresh_ms_defecto() -> u64 {
    REFRESH_MS_DEFECTO
}

impl Default for Config {
    fn default() -> Self {
        Self {
            broker_url: broker_url_defecto(),
            token: None,
            refresh_ms: REFRESH_MS_DEFECTO,
        }
    }
}

impl Config {
    /// Ruta canónica del archivo de config: `~/.config/claude-peers/config.toml`.
    /// En sistemas sin home resoluble cae a una ruta relativa (no falla aquí, falla al escribir).
    pub fn ruta() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
        base.join("claude-peers").join("config.toml")
    }

    /// Carga la config desde la ruta canónica. Si NO existe → defaults (no es error). Si existe
    /// pero está corrupta → propaga el error con contexto (el caller decide caer a default y avisar).
    pub fn cargar() -> Result<Self> {
        Self::cargar_desde(&Self::ruta())
    }

    /// Variante testeable: carga desde una ruta concreta. Archivo ausente → `Config::default()`.
    pub fn cargar_desde(ruta: &std::path::Path) -> Result<Self> {
        if !ruta.exists() {
            return Ok(Self::default());
        }
        let contenido = std::fs::read_to_string(ruta)
            .with_context(|| format!("no se pudo leer la config en {}", ruta.display()))?;
        Self::desde_toml(&contenido)
    }

    /// Parsea config desde texto TOML. Campos ausentes → sus defaults (serde).
    pub fn desde_toml(texto: &str) -> Result<Self> {
        toml::from_str(texto).context("config TOML inválida")
    }

    /// Serializa la config a TOML.
    pub fn a_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("no se pudo serializar la config a TOML")
    }

    /// Guarda en la ruta canónica, creando el directorio padre si hace falta.
    pub fn guardar(&self) -> Result<()> {
        self.guardar_en(&Self::ruta())
    }

    /// Variante testeable: guarda en una ruta concreta (crea el directorio padre).
    ///
    /// El archivo contiene el token → permisos restrictivos 0600 (solo el dueño) y el
    /// directorio 0700, para que ningún otro usuario de la máquina pueda leer el secreto.
    pub fn guardar_en(&self, ruta: &std::path::Path) -> Result<()> {
        if let Some(padre) = ruta.parent() {
            std::fs::create_dir_all(padre)
                .with_context(|| format!("no se pudo crear el directorio {}", padre.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // 0700: solo el dueño entra al directorio de config.
                let _ = std::fs::set_permissions(padre, std::fs::Permissions::from_mode(0o700));
            }
        }
        let texto = self.a_toml()?;
        std::fs::write(ruta, texto)
            .with_context(|| format!("no se pudo escribir la config en {}", ruta.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // 0600: el token solo lo lee el dueño (std::fs::write deja ~0644 por defecto).
            std::fs::set_permissions(ruta, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("no se pudo restringir permisos de {}", ruta.display()))?;
        }
        Ok(())
    }
}

/// Enmascara un token para mostrarlo en pantalla sin revelarlo: deja los primeros 5 y los
/// últimos 4 caracteres, con `…` en medio (ej. `lexus…2026`). Tokens cortos se enmascaran
/// por completo. None → "(sin token)".
pub fn enmascarar_token(token: Option<&str>) -> String {
    match token {
        None | Some("") => "(sin token)".to_string(),
        Some(t) => {
            let n = t.chars().count();
            // Si es demasiado corto para dejar pista sin filtrar, lo ocultamos entero.
            if n <= 9 {
                "•".repeat(n)
            } else {
                let prefijo: String = t.chars().take(5).collect();
                let sufijo: String = t.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                format!("{prefijo}…{sufijo}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_ausente_cae_a_defaults() {
        let ruta = std::env::temp_dir().join("claude-peers-test-no-existe-xyz.toml");
        let _ = std::fs::remove_file(&ruta);
        let cfg = Config::cargar_desde(&ruta).expect("ausente debe dar default");
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.broker_url, "http://127.0.0.1:7899");
        assert_eq!(cfg.refresh_ms, 1000);
        assert!(cfg.token.is_none());
    }

    #[test]
    fn toml_parcial_completa_con_defaults() {
        // Solo broker_url → token None y refresh_ms al default.
        let cfg = Config::desde_toml("broker_url = \"http://10.0.0.5:7899\"").unwrap();
        assert_eq!(cfg.broker_url, "http://10.0.0.5:7899");
        assert_eq!(cfg.refresh_ms, 1000);
        assert!(cfg.token.is_none());
    }

    #[test]
    fn toml_completo_se_parsea() {
        let texto = "broker_url = \"http://host:9000\"\ntoken = \"lexus-secreto-2026\"\nrefresh_ms = 500\n";
        let cfg = Config::desde_toml(texto).unwrap();
        assert_eq!(cfg.broker_url, "http://host:9000");
        assert_eq!(cfg.token.as_deref(), Some("lexus-secreto-2026"));
        assert_eq!(cfg.refresh_ms, 500);
    }

    #[test]
    fn roundtrip_guardar_cargar() {
        let cfg = Config {
            broker_url: "http://127.0.0.1:7899".to_string(),
            token: Some("lexus-secreto-2026".to_string()),
            refresh_ms: 750,
        };
        let ruta = std::env::temp_dir().join("claude-peers-test-roundtrip.toml");
        cfg.guardar_en(&ruta).unwrap();
        let leido = Config::cargar_desde(&ruta).unwrap();
        assert_eq!(cfg, leido);
        let _ = std::fs::remove_file(&ruta);
    }

    #[cfg(unix)]
    #[test]
    fn guardar_pone_permisos_0600_en_el_archivo() {
        use std::os::unix::fs::PermissionsExt;
        let cfg = Config {
            broker_url: "http://127.0.0.1:7899".to_string(),
            token: Some("secreto".to_string()),
            refresh_ms: 1000,
        };
        // Ruta única por test para no chocar con otros tests en paralelo.
        let ruta = std::env::temp_dir().join("claude-peers-test-permisos.toml");
        cfg.guardar_en(&ruta).unwrap();
        let modo = std::fs::metadata(&ruta).unwrap().permissions().mode() & 0o777;
        assert_eq!(modo, 0o600, "el config con token debe ser 0600 (solo dueño)");
        let _ = std::fs::remove_file(&ruta);
    }

    #[test]
    fn enmascarar_token_largo() {
        // "lexus-secreto-2026" → primeros 5 "lexus" + últimos 4 "2026".
        assert_eq!(enmascarar_token(Some("lexus-secreto-2026")), "lexus…2026");
    }

    #[test]
    fn enmascarar_token_none_y_vacio() {
        assert_eq!(enmascarar_token(None), "(sin token)");
        assert_eq!(enmascarar_token(Some("")), "(sin token)");
    }

    #[test]
    fn enmascarar_token_corto_se_oculta_entero() {
        // 9 chars o menos → todo puntos (no filtra pistas).
        assert_eq!(enmascarar_token(Some("abc123")), "••••••");
        assert_eq!(enmascarar_token(Some("123456789")), "•••••••••");
    }
}
