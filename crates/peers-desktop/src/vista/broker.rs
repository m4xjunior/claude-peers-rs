//! Pantalla Broker — salud e info de arranque del broker.
//!
//! Espejo de `peers-tui/src/ui/broker.rs`: muestra tres bloques de key-values obtenidos de
//! `GET /admin/info` (host, puerto, versión, instancias), `GET /salud` (estado + instancias
//! vivas) y `GET /factor-estimacion` (factor aprendido + nº de muestras). La app rellena
//! `EstadoPantalla::{info,salud,factor}` con `cx.spawn`; aquí sólo se pinta lo que haya.
//!
//! INTENCIÓN de diseño: la firma del stub es `(&EstadoPantalla) -> impl IntoElement`, sin `cx`,
//! por el contrato estable de la Fundación. Por eso los colores semánticos (verde "ok",
//! amarillo "degradado", gris "sin datos") van como literales `rgb`, igual que la TUI usa
//! `Color::Green`/`Yellow` — no dependen del tema. Cuando se fije la revisión del kit y la
//! firma pueda recibir `cx`, migrar a `cx.theme()`.

use gpui::{div, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::group_box::{GroupBox, GroupBoxVariants as _};

use crate::app::EstadoPantalla;

/// Verde para estado saludable ("ok"), espejo de `Color::Green` de la TUI.
const COLOR_OK: u32 = 0x22c55e;
/// Amarillo para estado degradado o desconocido, espejo de `Color::Yellow` de la TUI.
const COLOR_AVISO: u32 = 0xeab308;
/// Cian para el factor de estimación (dato destacado), espejo de `Color::Cyan`.
const COLOR_FACTOR: u32 = 0x06b6d4;
/// Gris tenue para etiquetas y avisos de "sin datos", espejo de `Color::DarkGray`.
const COLOR_TENUE: u32 = 0x71717a;

/// Fila key-value: etiqueta tenue de ancho fijo a la izquierda + valor a la derecha.
/// Es el ladrillo de los tres bloques; centraliza el layout para no repetirlo por campo.
fn campo(etiqueta: impl Into<SharedString>, valor: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap_3()
        .child(
            div()
                .w(gpui::px(160.0))
                .text_color(gpui::rgb(COLOR_TENUE))
                .child(etiqueta.into()),
        )
        .child(div().child(valor.into()))
}

/// Fila key-value con valor coloreado (para el estado de salud y el factor destacado).
fn campo_color(
    etiqueta: impl Into<SharedString>,
    valor: impl Into<SharedString>,
    color: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap_3()
        .child(
            div()
                .w(gpui::px(160.0))
                .text_color(gpui::rgb(COLOR_TENUE))
                .child(etiqueta.into()),
        )
        .child(div().text_color(gpui::rgb(color)).child(valor.into()))
}

/// Aviso de "sin datos todavía" para un bloque cuyo endpoint aún no respondió.
fn sin_datos(texto: impl Into<SharedString>) -> impl IntoElement {
    div().text_color(gpui::rgb(COLOR_TENUE)).child(texto.into())
}

pub fn render_broker(datos: &EstadoPantalla) -> impl IntoElement {
    // Bloque 1 — arranque del broker (GET /admin/info). Si no hay datos, un aviso tenue.
    let bloque_info = match &datos.info {
        Some(info) => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(campo("host", info.host.clone()))
            .child(campo("puerto", info.puerto.to_string()))
            .child(campo("version", info.version.clone()))
            .child(campo("instancias", info.instancias.to_string())),
        None => div()
            .flex()
            .flex_col()
            .child(sin_datos("(sin datos de /admin/info todavía)")),
    };

    // Bloque 2 — salud (GET /salud). El estado se colorea: verde si "ok", amarillo si no.
    let bloque_salud = match &datos.salud {
        Some(s) => {
            let color = if s.estado == "ok" { COLOR_OK } else { COLOR_AVISO };
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(campo_color("estado", s.estado.clone(), color))
                .child(campo("instancias vivas", s.instancias.to_string()))
        }
        None => div()
            .flex()
            .flex_col()
            .child(sin_datos("(sin datos de /salud todavía)")),
    };

    // Bloque 3 — factor de estimación (GET /factor-estimacion). El broker lo aprende del
    // tiempo real; aquí sólo se muestra: "6.2x" destacado + "· N muestras" tenue.
    let bloque_factor = match &datos.factor {
        Some(fe) => div().flex().flex_row().gap_3().child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    div()
                        .text_color(gpui::rgb(COLOR_FACTOR))
                        .child(SharedString::from(format!("{:.1}x", fe.factor))),
                )
                .child(
                    div()
                        .text_color(gpui::rgb(COLOR_TENUE))
                        .child(SharedString::from(format!("· {} muestras", fe.muestras))),
                ),
        ),
        None => div()
            .flex()
            .flex_col()
            .child(sin_datos("(sin datos de /factor-estimacion todavía)")),
    };

    // Layout: título de pantalla + tres GroupBox apilados en columna, con scroll implícito
    // del contenedor padre. Cada bloque agrupa sus key-values bajo un título descriptivo.
    div()
        .flex()
        .flex_col()
        .size_full()
        .gap_4()
        .p_6()
        .child(div().text_xl().child(SharedString::from("Broker")))
        .child(
            GroupBox::new()
                .outline()
                .title("Arranque")
                .child(bloque_info),
        )
        .child(
            GroupBox::new()
                .outline()
                .title("Salud")
                .child(bloque_salud),
        )
        .child(
            GroupBox::new()
                .outline()
                .title("Estimación")
                .child(bloque_factor),
        )
}
