//! Config persistida de la app desktop en `~/.config/claude-peers/config.toml`.
//!
//! INTENCIÓN: espejo EXACTO del modelo de config de la TUI (`peers-tui/src/config.rs`) — misma
//! ruta, mismo TOML, mismos permisos 0600. Se porta aquí (en vez de depender de peers-tui)
//! porque desktop no debe acoplarse al crate de la TUI; pero el ARCHIVO es el mismo, de modo que
//! editar la config en cualquiera de los dos frontends la ve el otro. La app nunca crashea por
//! config: si el archivo no existe o está corrupto, se cae a defaults sensatos.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// URL por defecto del broker: loopback en el puerto estándar del proyecto (`PUERTO_DEFECTO`).
fn broker_url_defecto() -> String {
    format!("http://127.0.0.1:{}", peers_core::PUERTO_DEFECTO)
}

/// Refresco por defecto: 1s. Suficiente para una red de pocos peers sin martillar el broker.
const REFRESH_MS_DEFECTO: u64 = 1000;

/// Cuántos directorios recientes se recuerdan como máximo (R1.2 de la RFC Lanzador).
pub const MAX_RECIENTES: usize = 8;

/// Cuántos eventos de conexión se recuerdan como máximo (acceso-13 de la RFC Acceso). Igual
/// criterio que `MAX_RECIENTES`: acotar para no ensuciar el `config.toml` indefinidamente.
pub const MAX_HISTORIAL_ACCESO: usize = 20;

/// Config persistida. `token` es opcional: si el broker corre sin token, se omite del archivo.
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
    /// Estado persistido de la pantalla Lanzador (RFC Lanzador Fase 1). `#[serde(default)]`
    /// garantiza que un `config.toml` viejo SIN esta sección deserialice sin error (AC10).
    #[serde(default)]
    pub lanzador: ConfigLanzador,
    /// Historial local de conexiones (acceso-13, RFC Acceso). Mismo criterio retrocompatible que
    /// `lanzador`: `#[serde(default)]` para que un `config.toml` viejo sin `[acceso]` cargue igual.
    #[serde(default)]
    pub acceso: ConfigAcceso,
}

fn refresh_ms_defecto() -> u64 {
    REFRESH_MS_DEFECTO
}

/// Estado persistido del Lanzador: directorios recientes (R1.2) y plantillas de system prompt
/// (R2.1). Se guarda en la sub-tabla `[lanzador]` del mismo `config.toml`. Ambas listas se
/// serializan sólo si NO están vacías, para no ensuciar el archivo de quien no use el Lanzador.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConfigLanzador {
    /// Últimos directorios elegidos en el file picker (R1.2), del más reciente al más antiguo.
    /// Acotado a `MAX_RECIENTES` al registrar. Rutas absolutas tal cual las devuelve el picker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recientes: Vec<String>,
    /// Plantillas de system prompt nombradas (R2.1): pares (nombre, texto) reutilizables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plantillas: Vec<PlantillaPrompt>,
}

/// Una plantilla de system prompt nombrada (R2.1): el usuario la elige de un desplegable y la
/// edita antes de lanzar. `nombre` es la clave visible; `texto` el cuerpo del `--append-system-prompt`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlantillaPrompt {
    /// Nombre visible de la plantilla (p.ej. "peer backend Rust").
    pub nombre: String,
    /// Cuerpo del system prompt que se inyectará como `--append-system-prompt "<texto>"`.
    pub texto: String,
}

impl ConfigLanzador {
    /// Registra `dir` como el directorio más reciente: lo mueve al frente (o lo inserta),
    /// deduplica y acota la lista a `MAX_RECIENTES`. No persiste por sí sola — el caller guarda
    /// la `Config` completa. Devuelve `true` si la lista cambió (para evitar escrituras inútiles).
    pub fn registrar_reciente(&mut self, dir: impl Into<String>) -> bool {
        let dir = dir.into();
        if dir.trim().is_empty() {
            return false;
        }
        // Si ya estaba justo al frente, no hay cambio que persistir.
        if self.recientes.first().is_some_and(|r| r == &dir) {
            return false;
        }
        self.recientes.retain(|r| r != &dir);
        self.recientes.insert(0, dir);
        self.recientes.truncate(MAX_RECIENTES);
        true
    }

    /// Guarda o actualiza una plantilla por nombre (R2.1): si ya existe una con ese `nombre`,
    /// reemplaza su texto; si no, la añade. Nombre vacío se ignora (devuelve `false`).
    pub fn guardar_plantilla(
        &mut self,
        nombre: impl Into<String>,
        texto: impl Into<String>,
    ) -> bool {
        let nombre = nombre.into();
        if nombre.trim().is_empty() {
            return false;
        }
        let texto = texto.into();
        if let Some(p) = self.plantillas.iter_mut().find(|p| p.nombre == nombre) {
            p.texto = texto;
        } else {
            self.plantillas.push(PlantillaPrompt { nombre, texto });
        }
        true
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            broker_url: broker_url_defecto(),
            token: None,
            refresh_ms: REFRESH_MS_DEFECTO,
            lanzador: ConfigLanzador::default(),
            acceso: ConfigAcceso::default(),
        }
    }
}

/// Estado persistido de la pantalla Acceso: hoy sólo el historial local de conexiones
/// (acceso-13). Se guarda en la sub-tabla `[acceso]` del mismo `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConfigAcceso {
    /// Eventos de conexión (Aplicar/Probar) más recientes primero, acotados a
    /// `MAX_HISTORIAL_ACCESO`. Se serializa sólo si NO está vacío (no ensucia el archivo de quien
    /// nunca abrió Acceso).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub historial: Vec<EventoConexion>,
}

/// Un evento del historial local de conexiones (acceso-13): qué operación se hizo, contra qué
/// broker, con qué resultado y cuándo. Es trazabilidad de SESIÓN (vive sólo en la desktop, nunca
/// se manda al broker) — distinto de la bitácora de acciones del broker (`peers-core::AccionRegistrada`),
/// que es del PEER hablando con el broker, no del jefe operando la conexión desde la desktop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventoConexion {
    /// Marca de tiempo en RFC 3339 UTC (mismo formato que usa el broker para `creada_en` en
    /// alertas — `time::OffsetDateTime::now_utc().format(&Rfc3339)`), para poder mostrar tanto la
    /// hora local como calcular antigüedad relativa si hiciera falta más adelante.
    pub cuando: String,
    /// Qué operación generó el evento.
    pub tipo: TipoEventoConexion,
    /// `broker_url` contra el que se operó (tal cual estaba en el momento del evento).
    pub broker_url: String,
    /// `true` si la operación tuvo éxito (aplicó/probó sin error).
    pub ok: bool,
    /// Resumen legible del resultado (p.ej. "broker vivo · token válido" o el motivo del fallo).
    pub detalle: String,
}

/// Qué operación generó un `EventoConexion` (acceso-13). Sólo dos: son las dos escrituras que
/// hace hoy `PanelAcceso` (acceso-01/02 y acceso-05); si el panel gana más acciones mutantes en el
/// futuro, se añade variante aquí, nunca se reusa una existente con otro significado.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TipoEventoConexion {
    /// «Guardar y reconectar» (acceso-01/02).
    Aplicar,
    /// «Probar conexión» (acceso-05).
    Probar,
}

impl ConfigAcceso {
    /// Registra un evento al FRENTE del historial (más reciente primero) y lo acota a
    /// `MAX_HISTORIAL_ACCESO`. No persiste por sí solo — el caller guarda la `Config` completa,
    /// igual que `ConfigLanzador::registrar_reciente`.
    pub fn registrar_evento(&mut self, evento: EventoConexion) {
        self.historial.insert(0, evento);
        self.historial.truncate(MAX_HISTORIAL_ACCESO);
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
    fn config_ausente_cae_a_defaults() {
        let ruta = std::env::temp_dir().join("peers-desktop-test-no-existe-xyz.toml");
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
    fn roundtrip_guardar_cargar() {
        let cfg = Config {
            broker_url: "http://127.0.0.1:7899".to_string(),
            token: Some("lexus-secreto-2026".to_string()),
            refresh_ms: 750,
            ..Default::default()
        };
        let ruta = std::env::temp_dir().join("peers-desktop-test-roundtrip.toml");
        cfg.guardar_en(&ruta).unwrap();
        let leido = Config::cargar_desde(&ruta).unwrap();
        assert_eq!(cfg, leido);
        let _ = std::fs::remove_file(&ruta);
    }

    #[test]
    fn enmascarar_token_largo() {
        assert_eq!(enmascarar_token(Some("lexus-secreto-2026")), "lexus…2026");
    }

    #[test]
    fn enmascarar_token_none_y_vacio() {
        assert_eq!(enmascarar_token(None), "(sin token)");
        assert_eq!(enmascarar_token(Some("")), "(sin token)");
    }

    // --- RFC Lanzador Fase 1: config `[lanzador]` (R1.2 recientes, R2.1 plantillas, AC10 compat) ---

    #[test]
    fn config_vieja_sin_lanzador_deserializa(/* AC10 */) {
        // Un config.toml previo a la RFC Lanzador NO tiene la tabla [lanzador]: debe cargar con
        // la sección al default (recientes y plantillas vacías), sin error de deserialización.
        let cfg =
            Config::desde_toml("broker_url = \"http://127.0.0.1:7899\"\nrefresh_ms = 500").unwrap();
        assert_eq!(cfg.refresh_ms, 500);
        assert!(cfg.lanzador.recientes.is_empty());
        assert!(cfg.lanzador.plantillas.is_empty());
    }

    #[test]
    fn recientes_mueve_al_frente_dedup_y_acota() {
        let mut l = ConfigLanzador::default();
        // Insertar hasta pasar el límite: sólo se conservan los MAX_RECIENTES más nuevos.
        for i in 0..(MAX_RECIENTES + 3) {
            assert!(l.registrar_reciente(format!("/dir/{i}")));
        }
        assert_eq!(l.recientes.len(), MAX_RECIENTES);
        // El último insertado queda al frente.
        assert_eq!(l.recientes[0], format!("/dir/{}", MAX_RECIENTES + 2));
        // Reelegir uno ya presente lo sube al frente sin duplicar ni crecer.
        let objetivo = l.recientes[3].clone();
        assert!(l.registrar_reciente(objetivo.clone()));
        assert_eq!(l.recientes[0], objetivo);
        assert_eq!(l.recientes.len(), MAX_RECIENTES);
        assert_eq!(l.recientes.iter().filter(|r| **r == objetivo).count(), 1);
        // Registrar el mismo que ya está al frente no cambia nada (no fuerza escritura).
        assert!(!l.registrar_reciente(objetivo));
        // Cadena vacía se ignora.
        assert!(!l.registrar_reciente("   "));
    }

    #[test]
    fn plantillas_guardar_y_actualizar_por_nombre() {
        let mut l = ConfigLanzador::default();
        assert!(l.guardar_plantilla("backend", "eres peer Rust"));
        assert!(l.guardar_plantilla("frontend", "eres peer UI"));
        assert_eq!(l.plantillas.len(), 2);
        // Mismo nombre → actualiza el texto, no duplica.
        assert!(l.guardar_plantilla("backend", "eres peer Rust senior"));
        assert_eq!(l.plantillas.len(), 2);
        let p = l.plantillas.iter().find(|p| p.nombre == "backend").unwrap();
        assert_eq!(p.texto, "eres peer Rust senior");
        // Nombre vacío se ignora.
        assert!(!l.guardar_plantilla("  ", "x"));
        assert_eq!(l.plantillas.len(), 2);
    }

    #[test]
    fn roundtrip_config_con_lanzador() {
        let mut cfg = Config::default();
        cfg.lanzador.registrar_reciente("/Users/max/proyecto");
        cfg.lanzador.guardar_plantilla("peer backend", "eres un peer");
        let ruta = std::env::temp_dir().join("peers-desktop-test-lanzador.toml");
        cfg.guardar_en(&ruta).unwrap();
        let leido = Config::cargar_desde(&ruta).unwrap();
        assert_eq!(cfg, leido);
        assert_eq!(leido.lanzador.recientes, vec!["/Users/max/proyecto".to_string()]);
        assert_eq!(leido.lanzador.plantillas.len(), 1);
        let _ = std::fs::remove_file(&ruta);
    }

    // --- RFC Acceso: historial local de conexiones `[acceso]` (acceso-13) ---

    fn evento_de_prueba(tipo: TipoEventoConexion, ok: bool) -> EventoConexion {
        EventoConexion {
            cuando: "2026-07-03T09:00:00Z".to_string(),
            tipo,
            broker_url: "http://127.0.0.1:7899".to_string(),
            ok,
            detalle: "broker vivo · token válido".to_string(),
        }
    }

    #[test]
    fn config_vieja_sin_acceso_deserializa() {
        // Un config.toml previo a la RFC Acceso NO tiene la tabla [acceso]: debe cargar con la
        // sección al default (historial vacío), sin error de deserialización — mismo criterio de
        // retrocompatibilidad que `config_vieja_sin_lanzador_deserializa` (AC10 de Lanzador).
        let cfg =
            Config::desde_toml("broker_url = \"http://127.0.0.1:7899\"\nrefresh_ms = 500").unwrap();
        assert_eq!(cfg.refresh_ms, 500);
        assert!(cfg.acceso.historial.is_empty());
    }

    #[test]
    fn registrar_evento_inserta_al_frente_y_acota() {
        let mut a = ConfigAcceso::default();
        for i in 0..(MAX_HISTORIAL_ACCESO + 3) {
            let mut ev = evento_de_prueba(TipoEventoConexion::Probar, true);
            ev.detalle = format!("intento {i}");
            a.registrar_evento(ev);
        }
        assert_eq!(a.historial.len(), MAX_HISTORIAL_ACCESO);
        // El último registrado queda al frente (más reciente primero).
        assert_eq!(a.historial[0].detalle, format!("intento {}", MAX_HISTORIAL_ACCESO + 2));
    }

    #[test]
    fn roundtrip_config_con_historial_acceso() {
        let mut cfg = Config::default();
        cfg.acceso.registrar_evento(evento_de_prueba(TipoEventoConexion::Aplicar, true));
        cfg.acceso.registrar_evento(evento_de_prueba(TipoEventoConexion::Probar, false));
        let ruta = std::env::temp_dir().join("peers-desktop-test-acceso-historial.toml");
        cfg.guardar_en(&ruta).unwrap();
        let leido = Config::cargar_desde(&ruta).unwrap();
        assert_eq!(cfg, leido);
        assert_eq!(leido.acceso.historial.len(), 2);
        // Orden preservado tras el roundtrip TOML (más reciente al frente): el último insertado
        // (Probar) queda en [0], el primero (Aplicar) en [1].
        assert_eq!(leido.acceso.historial[0].tipo, TipoEventoConexion::Probar);
        assert!(!leido.acceso.historial[0].ok);
        assert_eq!(leido.acceso.historial[1].tipo, TipoEventoConexion::Aplicar);
        assert!(leido.acceso.historial[1].ok);
        let _ = std::fs::remove_file(&ruta);
    }
}
