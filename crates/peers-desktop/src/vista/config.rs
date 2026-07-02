//! Pantalla Config — porta la pantalla 5 de la TUI (`peers-tui/src/ui/config.rs`) a GPUI.
//!
//! Qué muestra (espejo de la TUI): los tres parámetros configurables del archivo
//! `~/.config/claude-peers/config.toml` — `broker_url`, `token` y `refresh_ms` — como campos
//! EDITABLES (Input de gpui-component) y un botón "Guardar" que persiste el TOML (equivalente a
//! la tecla 's' de la TUI). La TUI resaltaba un campo y abría un modal para editarlo; en desktop
//! los tres se editan a la vez inline, más natural para un formulario nativo.
//!
//! INTENCIÓN de diseño (por qué un componente stateful y no una función pura): los `Input` del
//! kit requieren `Entity<InputState>`, que sólo puede crearse con `Window`/`Context` y debe vivir
//! en una vista (no en un render efímero). Por eso la pantalla es un `PanelConfig: Render` con sus
//! Inputs; el stub `render_config` (firma de la Fundación intacta) sólo delega en la `Entity`
//! creada por la app. Así no se rompe el contrato `render_config(&EstadoPantalla)` ni el sidebar.

use gpui::{
    div, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    ActiveTheme, StyledExt,
};

use crate::config::Config;
use crate::app::EstadoPantalla;

/// Resultado del último intento de guardado, para pintar feedback bajo el botón. No usamos
/// Notification del kit aquí para mantener la pantalla autocontenida; un texto de estado basta.
enum EstadoGuardado {
    /// Aún no se ha pulsado Guardar en esta sesión.
    Inicial,
    /// Guardado correcto en la ruta indicada.
    Ok(String),
    /// Falló el guardado (permisos, disco…) con el mensaje del error.
    Error(String),
}

/// Panel de configuración con estado propio: un `InputState` por campo editable + el resultado
/// del último guardado. Vive como `Entity<PanelConfig>` dentro de `EstadoPantalla`.
pub struct PanelConfig {
    /// Input de la URL base del broker.
    entrada_broker_url: Entity<InputState>,
    /// Input del token (con toggle de máscara: el token es un secreto).
    entrada_token: Entity<InputState>,
    /// Input del refresco en ms. Se valida al guardar (debe ser un entero > 0).
    entrada_refresh: Entity<InputState>,
    /// Feedback del último guardado.
    ultimo_guardado: EstadoGuardado,
}

impl PanelConfig {
    /// Crea el panel cargando la config actual del disco y sembrando cada Input con su valor.
    /// Si la config está corrupta o ausente, se parte de `Config::default()` (nunca crashea).
    pub fn nuevo(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Ausente/corrupta → defaults: la pantalla debe abrirse siempre.
        let cfg = Config::cargar().unwrap_or_default();

        let entrada_broker_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("http://127.0.0.1:7899")
                .default_value(cfg.broker_url.clone())
        });
        // El token no se enmascara al mostrarlo aquí (es un campo editable): se ofrece el toggle
        // de máscara del propio Input para ocultarlo a la vista cuando el usuario quiera.
        let entrada_token = cx.new(|cx| {
            let s = InputState::new(window, cx).placeholder("(sin token)");
            match cfg.token.as_deref() {
                Some(t) if !t.is_empty() => s.default_value(t.to_string()),
                _ => s,
            }
        });
        let entrada_refresh = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1000")
                .default_value(cfg.refresh_ms.to_string())
        });

        Self {
            entrada_broker_url,
            entrada_token,
            entrada_refresh,
            ultimo_guardado: EstadoGuardado::Inicial,
        }
    }

    /// Lee los tres Inputs, valida y persiste el TOML. Equivale a la tecla 's' de la TUI.
    /// No hace `.unwrap()` sobre IO: el error de guardado se refleja en `ultimo_guardado`.
    fn guardar(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let broker_url = self.entrada_broker_url.read(cx).value().trim().to_string();
        let token_bruto = self.entrada_token.read(cx).value().trim().to_string();
        let refresh_bruto = self.entrada_refresh.read(cx).value().trim().to_string();

        // Validación de frontera (parse, don't validate): broker_url no puede ir vacío.
        if broker_url.is_empty() {
            self.ultimo_guardado =
                EstadoGuardado::Error("broker_url no puede estar vacío".to_string());
            cx.notify();
            return;
        }

        // refresh_ms debe ser un entero > 0; si no parsea, se avisa sin tocar el disco.
        let refresh_ms = match refresh_bruto.parse::<u64>() {
            Ok(n) if n > 0 => n,
            _ => {
                self.ultimo_guardado = EstadoGuardado::Error(format!(
                    "refresh_ms inválido: «{refresh_bruto}» (debe ser un entero > 0)"
                ));
                cx.notify();
                return;
            }
        };

        // Token vacío → None (el broker corre sin token); no persistimos una cadena vacía.
        let token = if token_bruto.is_empty() {
            None
        } else {
            Some(token_bruto)
        };

        let cfg = Config {
            broker_url,
            token,
            refresh_ms,
        };

        match cfg.guardar() {
            Ok(()) => {
                self.ultimo_guardado =
                    EstadoGuardado::Ok(format!("Guardado en {}", Config::ruta().display()));
            }
            Err(e) => {
                self.ultimo_guardado = EstadoGuardado::Error(format!("No se pudo guardar: {e}"));
            }
        }
        cx.notify();
    }

    /// Construye un campo etiquetado (etiqueta encima + input debajo), replicando el layout de
    /// filas de la pantalla de config de la TUI.
    fn campo(
        etiqueta: &'static str,
        ayuda: &'static str,
        input: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .v_flex()
            .gap_1()
            .child(div().text_sm().font_bold().child(SharedString::from(etiqueta)))
            .child(input)
            .child(
                div()
                    .text_xs()
                    .text_color(gpui::rgb(0x9ca3af))
                    .child(SharedString::from(ayuda)),
            )
    }
}

impl Render for PanelConfig {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Línea de feedback bajo el botón: color según el resultado del último guardado.
        let feedback = match &self.ultimo_guardado {
            EstadoGuardado::Inicial => div(),
            EstadoGuardado::Ok(msg) => div()
                .text_sm()
                .text_color(gpui::rgb(0x22c55e))
                .child(SharedString::from(msg.clone())),
            EstadoGuardado::Error(msg) => div()
                .text_sm()
                .text_color(gpui::rgb(0xef4444))
                .child(SharedString::from(msg.clone())),
        };

        div()
            .v_flex()
            .size_full()
            .gap_4()
            .p_6()
            // Cabecera de la pantalla.
            .child(div().text_xl().font_bold().child(SharedString::from("Config")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(
                        "Parámetros del broker. Guardar persiste ~/.config/claude-peers/config.toml.",
                    )),
            )
            // Formulario: los tres campos configurables.
            .child(Self::campo(
                "broker_url",
                "URL base del broker (sin barra final).",
                Input::new(&self.entrada_broker_url).cleanable(true),
            ))
            .child(Self::campo(
                "token",
                "Token X-Peers-Token. Vacío = broker sin token.",
                Input::new(&self.entrada_token).cleanable(true).mask_toggle(),
            ))
            .child(Self::campo(
                "refresh_ms",
                "Periodo de refresco de las pantallas, en milisegundos (entero > 0).",
                Input::new(&self.entrada_refresh).cleanable(true),
            ))
            // Acción de guardado + feedback.
            .child(
                div().h_flex().gap_3().items_center().child(
                    Button::new("config-guardar")
                        .primary()
                        .label("Guardar")
                        .on_click(cx.listener(|esta, _evento, window, cx| {
                            esta.guardar(window, cx);
                        })),
                ),
            )
            .child(feedback)
    }
}

/// Stub de la Fundación (firma intacta): la pantalla Config delega en su `Entity<PanelConfig>`,
/// que la app crea al arrancar y guarda en `EstadoPantalla`. Si por lo que sea el panel no está
/// inicializado, se pinta un aviso en vez de crashear (nunca `.unwrap()` sobre el Option).
pub fn render_config(datos: &EstadoPantalla) -> impl IntoElement {
    match &datos.panel_config {
        Some(panel) => div().size_full().child(panel.clone()),
        None => div()
            .v_flex()
            .size_full()
            .p_6()
            .child(SharedString::from("Config no inicializada.")),
    }
}

/// Helper de la app para construir la `Entity<PanelConfig>` al arrancar. Se expone aquí para no
/// filtrar el tipo interno del panel a `app.rs`; la app sólo llama a este constructor.
pub fn nuevo_panel(window: &mut Window, cx: &mut App) -> Entity<PanelConfig> {
    cx.new(|cx| PanelConfig::nuevo(window, cx))
}
