//! Validación de frontera de `broker_url`, compartida por las pantallas Acceso y Config.
//!
//! INTENCIÓN (por qué se extrae aquí): antes de este módulo había TRES validaciones de la misma
//! URL viviendo por separado — `vista/config.rs::validar_url` (esquema + barra final, sin
//! tests), `cliente::ClienteBroker::reconfigurar` (normaliza trim + barra final, sin validar) y
//! `vista/acceso.rs::PanelAcceso::aplicar` (sólo comprobaba "no vacía"). Acceso-16 pide además
//! validar el puerto y sugerir una autocorrección (RFC acceso-16, variante C) — añadir una CUARTA
//! copia habría sido la definición de "no reinventar lo que ya existe" al revés. Este módulo es
//! PURO (sin `cx`, sin red) para que sea trivialmente testeable: parse, don't validate.

/// Resultado de validar una `broker_url`. `motivo` es `None` si la URL es válida tal cual;
/// `sugerencia` sólo se rellena cuando el motivo es corregible automáticamente (falta el esquema),
/// para ofrecer un botón "usar sugerencia" (acceso-16, variante C) sin obligar a la pantalla a
/// reimplementar la heurística.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidacionUrl {
    /// `None` = válida. `Some(motivo)` = inválida, con el texto a mostrar bajo el campo.
    pub motivo: Option<String>,
    /// Autocorrección sugerida (p.ej. `10.0.0.67:7899` → `http://10.0.0.67:7899`), o `None` si el
    /// fallo no es de los que se pueden arreglar con una sugerencia mecánica.
    pub sugerencia: Option<String>,
}

impl ValidacionUrl {
    /// `true` si la URL pasó todas las reglas.
    pub fn es_valida(&self) -> bool {
        self.motivo.is_none()
    }

    fn ok() -> Self {
        Self::default()
    }

    fn error(motivo: impl Into<String>) -> Self {
        Self { motivo: Some(motivo.into()), sugerencia: None }
    }

    fn error_con_sugerencia(motivo: impl Into<String>, sugerencia: impl Into<String>) -> Self {
        Self { motivo: Some(motivo.into()), sugerencia: Some(sugerencia.into()) }
    }
}

/// Valida (y diagnostica) una `broker_url` en frontera. Reglas, en orden:
///   1. no vacía;
///   2. esquema `http://`/`https://` presente — si falta pero el resto PARECE un host:puerto
///      (p.ej. `10.0.0.67:7899`), se sugiere anteponer `http://` (acceso-16, variante C);
///   3. host no vacío tras el esquema;
///   4. sin barra final (el cliente concatena rutas con `format!("{base}{ruta}")`, una barra
///      final produciría `//admin/info`);
///   5. si hay un `:puerto` explícito en el host, debe parsear como `u16` (1-65535).
///
/// Es exactamente `parse, don't validate` en la frontera: no toca red ni disco, sólo texto.
pub fn validar_url(url: &str) -> ValidacionUrl {
    let url = url.trim();
    if url.is_empty() {
        return ValidacionUrl::error("la URL no puede estar vacía");
    }

    let resto_con_esquema = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));

    let resto = match resto_con_esquema {
        Some(r) => r,
        None => {
            // Falta el esquema. Si el texto ya parece "host:puerto" (sin barras, con al menos un
            // carácter antes de un ':' numérico), sugerimos anteponer http:// en vez de sólo
            // quejarnos — es la variante C del RFC, mecánica y sin ambigüedad.
            let sugerencia = sugerir_esquema(url);
            return match sugerencia {
                Some(s) => ValidacionUrl::error_con_sugerencia(
                    "falta el esquema http:// o https://",
                    s,
                ),
                None => ValidacionUrl::error("falta el esquema http:// o https://"),
            };
        }
    };

    if resto.is_empty() {
        return ValidacionUrl::error("falta el host (p.ej. 127.0.0.1:7899)");
    }
    if url.ends_with('/') {
        return ValidacionUrl::error("sin barra final (el cliente concatena rutas)");
    }

    // Puerto explícito (después del último ':' del host, antes de cualquier '/' de ruta — aunque
    // ya rechazamos barra final, un futuro "sin validar" no debe asumir que resto no tiene '/').
    let host_y_puerto = resto.split('/').next().unwrap_or(resto);
    if let Some((_host, puerto_txt)) = host_y_puerto.rsplit_once(':') {
        if puerto_txt.parse::<u16>().is_err() || puerto_txt.parse::<u16>() == Ok(0) {
            return ValidacionUrl::error(format!(
                "puerto inválido «{puerto_txt}» (debe ser un entero entre 1 y 65535)"
            ));
        }
    }

    ValidacionUrl::ok()
}

/// Heurística de autocorrección: si `texto` (sin esquema) tiene forma de `host:puerto` —al menos
/// un carácter antes de los dos puntos y sólo dígitos después—, sugiere `http://{texto}`. No
/// intenta ser exhaustiva (no resuelve DNS ni valida el host): es sólo el caso más común de "Max
/// pegó la IP:puerto tal cual", que es el que realmente ahorra un viaje de ida y vuelta.
fn sugerir_esquema(texto: &str) -> Option<String> {
    let texto = texto.trim();
    if texto.is_empty() || texto.contains("://") {
        return None;
    }
    let (host, puerto) = texto.rsplit_once(':')?;
    if host.is_empty() || !puerto.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if puerto.parse::<u16>().is_err() {
        return None;
    }
    Some(format!("http://{texto}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacia_es_invalida() {
        let v = validar_url("");
        assert!(!v.es_valida());
        assert_eq!(v.motivo.as_deref(), Some("la URL no puede estar vacía"));
        assert!(v.sugerencia.is_none());
    }

    #[test]
    fn solo_espacios_es_invalida() {
        assert!(!validar_url("   ").es_valida());
    }

    #[test]
    fn sin_esquema_da_error_sin_sugerencia_si_no_parece_host_puerto() {
        let v = validar_url("no-es-una-url");
        assert!(!v.es_valida());
        assert_eq!(v.motivo.as_deref(), Some("falta el esquema http:// o https://"));
        assert!(v.sugerencia.is_none());
    }

    #[test]
    fn sin_esquema_con_host_puerto_sugiere_anteponer_http() {
        let v = validar_url("10.0.0.67:7899");
        assert!(!v.es_valida());
        assert_eq!(v.sugerencia.as_deref(), Some("http://10.0.0.67:7899"));
    }

    #[test]
    fn sin_host_es_invalida() {
        let v = validar_url("http://");
        assert!(!v.es_valida());
        assert_eq!(v.motivo.as_deref(), Some("falta el host (p.ej. 127.0.0.1:7899)"));
    }

    #[test]
    fn con_barra_final_es_invalida() {
        let v = validar_url("http://127.0.0.1:7899/");
        assert!(!v.es_valida());
        assert_eq!(
            v.motivo.as_deref(),
            Some("sin barra final (el cliente concatena rutas)")
        );
    }

    #[test]
    fn puerto_no_numerico_es_invalido() {
        let v = validar_url("http://127.0.0.1:abc");
        assert!(!v.es_valida());
        assert!(v.motivo.as_deref().unwrap().contains("puerto inválido"));
    }

    #[test]
    fn puerto_cero_es_invalido() {
        let v = validar_url("http://127.0.0.1:0");
        assert!(!v.es_valida());
        assert!(v.motivo.as_deref().unwrap().contains("puerto inválido"));
    }

    #[test]
    fn puerto_fuera_de_rango_u16_es_invalido() {
        let v = validar_url("http://127.0.0.1:99999");
        assert!(!v.es_valida());
        assert!(v.motivo.as_deref().unwrap().contains("puerto inválido"));
    }

    #[test]
    fn url_valida_https_sin_puerto() {
        assert!(validar_url("https://p2v.lexusfx.com").es_valida());
    }

    #[test]
    fn url_valida_http_con_puerto() {
        let v = validar_url("http://127.0.0.1:7899");
        assert!(v.es_valida());
        assert!(v.motivo.is_none());
    }

    #[test]
    fn url_valida_host_otus_con_puerto() {
        assert!(validar_url("http://10.0.0.67:7899").es_valida());
    }

    #[test]
    fn recorta_espacios_alrededor() {
        assert!(validar_url("  http://127.0.0.1:7899  ").es_valida());
    }

    #[test]
    fn sugerir_esquema_ignora_texto_con_esquema_ya_presente() {
        // No debería recomendarse a sí misma si YA tiene esquema (camino cubierto por otra rama).
        assert_eq!(sugerir_esquema("http://10.0.0.67:7899"), None);
    }

    #[test]
    fn sugerir_esquema_ignora_texto_sin_puerto_numerico() {
        assert_eq!(sugerir_esquema("localhost"), None);
        assert_eq!(sugerir_esquema("host:abc"), None);
    }
}
