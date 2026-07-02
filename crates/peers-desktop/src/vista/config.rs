//! Pantalla Config — porta la pantalla 5 de la TUI (`peers-tui/src/ui/config.rs`) a GPUI, ya
//! vestida con el Design System "Ethos" (tinta/pergamino + acento dorado "brasa").
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
//!
//! INTENCIÓN del rediseño (por qué el tema aquí y no vía `cx.theme()`): esta pantalla SÍ tiene
//! `cx` (es stateful), pero el look debe ser coherente con las 8 pantallas puras que consumen
//! `crate::tema`. Para no duplicar dos vocabularios visuales, se usan los MISMOS tokens/helpers
//! del tema Ethos (`tema::TINTA`, `superficie_card()`, `titulo()`, `eyebrow()`, `boton_primario()`).
//! Se eliminan TODOS los colores hardcodeados azules/genéricos previos (0x9ca3af, 0x22c55e,
//! 0xef4444) salvo los dos de feedback semántico (ok/error), que se calibran a tonos cálidos
//! coherentes con la paleta (verde salvia y terracota), no al verde/rojo puro genérico.
//!
//! DECISIÓN sobre acciones (por qué NO se despacha una Action al AppDesktop para Guardar): a
//! diferencia de las vistas puras (alertas, tareas…), `PanelConfig` es un `Entity` con estado y
//! `cx` propios, así que puede manejar su interacción directamente con `cx.listener(...)` sin
//! rebotar por el árbol de acciones. Guardar y "recargar desde disco" mutan SÓLO el estado local
//! del panel (los `InputState` y el feedback), no el `EstadoPantalla` global, de modo que un
//! listener local es lo correcto y lo más simple (senior, no over-engineered). Se declara igual
//! la Action `RecargarConfig` documentada abajo por si la Fase 3 quiere dispararla desde un menú
//! global; el panel la maneja internamente si se le cablea.

use gpui::{
    div, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window,
};
use gpui_component::{
    input::{Input, InputState},
    StyledExt,
};

use crate::app::EstadoPantalla;
use crate::config::Config;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — el panel es stateful y maneja Guardar con un listener local (ver nota de cabecera),
// así que NO necesita despachar Actions para su flujo normal. Se declara sólo `RecargarConfig`
// para que la Fase 3 pueda, si quiere, ofrecer un "recargar desde disco" desde un menú/atajo
// global: el AppDesktop despacharía la Action y, al manejarla, llamaría a
// `panel_config.update(cx, |p, cx| p.recargar(window, cx))`. Namespace `config` para no colisionar.
//
// `no_json`: sólo se despacha por código (nunca desde keymap), así se evita arrastrar
// `serde::Deserialize`/`schemars` como en el resto de pantallas.
// -------------------------------------------------------------------------------------------------

gpui::actions!(config, [RecargarConfig]);

/// Color de feedback OK — verde salvia apagado, coherente con la paleta cálida (no el 0x22c55e
/// genérico previo). Sólo se usa para la línea de estado tras un guardado correcto.
const VERDE_OK: gpui::Rgba = gpui::Rgba {
    r: 0x8F as f32 / 255.0,
    g: 0xB0 as f32 / 255.0,
    b: 0x7B as f32 / 255.0,
    a: 1.0,
};

/// Color de feedback de ERROR — terracota apagado, coherente con la paleta (no el 0xef4444
/// genérico previo). Se usa para validaciones fallidas y errores de IO al guardar.
const ROJO_ERROR: gpui::Rgba = gpui::Rgba {
    r: 0xC9 as f32 / 255.0,
    g: 0x6A as f32 / 255.0,
    b: 0x5A as f32 / 255.0,
    a: 1.0,
};

/// Resultado del último intento de guardado, para pintar feedback bajo el botón. No usamos
/// Notification del kit aquí para mantener la pantalla autocontenida; un texto de estado basta.
enum EstadoGuardado {
    /// Aún no se ha pulsado Guardar en esta sesión.
    Inicial,
    /// Guardado correcto en la ruta indicada.
    Ok(String),
    /// Falló el guardado (permisos, disco…) o la validación, con el mensaje del error.
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

    /// Relee la config del disco y re-siembra los tres Inputs. Punto de entrada para la Action
    /// `RecargarConfig` si la Fase 3 la cablea desde un menú global. Descarta ediciones no
    /// guardadas (intencional: "recargar" significa volver a lo persistido).
    #[allow(dead_code)] // Consumido por Fase 3 al cablear RecargarConfig; hoy queda declarado.
    pub fn recargar(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let cfg = Config::cargar().unwrap_or_default();
        self.entrada_broker_url.update(cx, |s, cx| {
            s.set_value(cfg.broker_url.clone(), _window, cx)
        });
        self.entrada_token.update(cx, |s, cx| {
            s.set_value(cfg.token.clone().unwrap_or_default(), _window, cx)
        });
        self.entrada_refresh.update(cx, |s, cx| {
            s.set_value(cfg.refresh_ms.to_string(), _window, cx)
        });
        self.ultimo_guardado = EstadoGuardado::Inicial;
        cx.notify();
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

    /// Construye un campo etiquetado del formulario con el vocabulario del tema Ethos:
    /// `eyebrow` (label en mono/humo mayúsculas) encima, el Input en medio y una nota de ayuda
    /// en texto terciario debajo. Reemplaza el `font_bold` + gris genérico previos.
    fn campo(
        etiqueta: &'static str,
        ayuda: &'static str,
        input: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .child(tema::eyebrow(etiqueta))
            .child(input)
            .child(tema::texto_terciario(ayuda))
    }
}

impl Render for PanelConfig {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Línea de feedback bajo el botón: color cálido según el resultado del último guardado.
        let feedback = match &self.ultimo_guardado {
            EstadoGuardado::Inicial => div(),
            EstadoGuardado::Ok(msg) => div()
                .text_sm()
                .text_color(VERDE_OK)
                .child(SharedString::from(format!("✓ {msg}"))),
            EstadoGuardado::Error(msg) => div()
                .text_sm()
                .text_color(ROJO_ERROR)
                .child(SharedString::from(format!("⚠ {msg}"))),
        };

        // Card contenedora del formulario: superficie elevada Ethos con padding generoso.
        let tarjeta = tema::superficie_card()
            .v_flex()
            .w_full()
            .gap_5()
            .p_6()
            // Los tres campos configurables, ya con eyebrow + ayuda terciaria.
            .child(Self::campo(
                "broker_url",
                "URL base del broker (sin barra final).",
                self.input_tematizado(&self.entrada_broker_url, false),
            ))
            .child(Self::campo(
                "token",
                "Token X-Peers-Token. Vacío = broker sin token.",
                self.input_tematizado(&self.entrada_token, true),
            ))
            .child(Self::campo(
                "refresh_ms",
                "Periodo de refresco de las pantallas, en milisegundos (entero > 0).",
                self.input_tematizado(&self.entrada_refresh, false),
            ))
            // Acción de guardado (botón Ethos dorado) + feedback semántico cálido.
            .child(
                div()
                    .v_flex()
                    .gap_3()
                    .pt_2()
                    .child(
                        tema::boton_primario("config-guardar", "Guardar")
                            .on_click(cx.listener(|esta, _evento, window, cx| {
                                esta.guardar(window, cx);
                            })),
                    )
                    .child(feedback),
            );

        // Raíz de la pantalla: fondo app Ethos + cabecera (eyebrow + título + subtítulo) + card.
        tema::fondo_app()
            .v_flex()
            .gap_5()
            .p_8()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(tema::eyebrow("Configuración"))
                    .child(tema::titulo("Broker"))
                    .child(tema::texto_terciario(
                        "Parámetros de conexión. Guardar persiste ~/.config/claude-peers/config.toml.",
                    )),
            )
            .child(tarjeta)
    }
}

impl PanelConfig {
    /// Envuelve un `Input` del kit en un contenedor con el estilo de control Ethos: borde LINEA,
    /// radio de control, superficie TINTA2 y foco dorado. El propio `Input` hereda la fuente UI y
    /// el color PAPEL del subárbol (fijados por `fondo_app`), así el widget del kit deja de verse
    /// con su tema por defecto y encaja en la paleta. `es_token` añade el toggle de máscara.
    ///
    /// Por qué envolver y no re-estilar el Input directamente: el `Input` de gpui-component pinta
    /// su propio fondo/borde con `cx.theme()`; envolverlo en un `div` tematizado y darle al Input
    /// fondo transparente deja el marco bajo nuestro control sin pelear con el tema del kit.
    fn input_tematizado(
        &self,
        estado: &Entity<InputState>,
        es_token: bool,
    ) -> impl IntoElement {
        let input = if es_token {
            Input::new(estado).cleanable(true).mask_toggle()
        } else {
            Input::new(estado).cleanable(true)
        };

        // NOTA: no se cablea `.focus(...)` en este wrapper: el foco lo posee el `InputState`
        // interno, no este `div`, así que un `focus_style` aquí nunca se activaría (sería estilo
        // muerto). El marco queda con borde LINEA estático; el cursor del propio Input da el
        // feedback de edición. Si en Fase 3 se quiere marco dorado al foco, hay que envolver con
        // `track_focus` del `focus_handle` del InputState (no trivial con la API del kit).
        div()
            .w_full()
            .px_3()
            .py_1()
            .rounded(tema::radio(tema::RADIO_CONTROL))
            .bg(tema::TINTA)
            .border_1()
            .border_color(tema::LINEA)
            .child(input)
    }
}

/// Stub de la Fundación (firma intacta): la pantalla Config delega en su `Entity<PanelConfig>`,
/// que la app crea al arrancar y guarda en `EstadoPantalla`. Si por lo que sea el panel no está
/// inicializado, se pinta un aviso tematizado en vez de crashear (nunca `.unwrap()` sobre el Option).
pub fn render_config(datos: &EstadoPantalla) -> impl IntoElement {
    match &datos.panel_config {
        Some(panel) => div().size_full().child(panel.clone()),
        None => tema::fondo_app()
            .v_flex()
            .p_8()
            .gap_2()
            .child(tema::eyebrow("Configuración"))
            .child(tema::texto_primario("Config no inicializada.")),
    }
}

/// Helper de la app para construir la `Entity<PanelConfig>` al arrancar. Se expone aquí para no
/// filtrar el tipo interno del panel a `app.rs`; la app sólo llama a este constructor.
pub fn nuevo_panel(window: &mut Window, cx: &mut App) -> Entity<PanelConfig> {
    cx.new(|cx| PanelConfig::nuevo(window, cx))
}
