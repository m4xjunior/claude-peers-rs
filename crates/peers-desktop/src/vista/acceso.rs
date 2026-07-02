//! Pantalla Acceso — estado de conexión y autenticación con el broker.
//!
//! INTENCIÓN: espejo de la pantalla "Red / Acceso" de la TUI (`peers-tui/src/ui/acceso.rs`),
//! que es SOLO LECTURA: muestra a qué broker apunta la desktop (URL base), el token enmascarado
//! y —ampliando respecto a la TUI— el estado de salud anunciado por el propio broker. La edición
//! real del token/URL vive en la pantalla Config; aquí solo se consulta.
//!
//! Reusa `datos.info` (GET /admin/info: host/puerto/versión) y `datos.salud` (GET /salud:
//! estado + nº instancias), ya cargados por la app. La URL base y el token enmascarado vienen
//! cacheados en `EstadoPantalla` porque la vista no recibe el `ClienteBroker`.

use gpui::{div, prelude::FluentBuilder, IntoElement, ParentElement, SharedString, Styled};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;

use crate::app::EstadoPantalla;

// Colores del panel (RGBA fijos, no del tema del kit): se usa `div` estilizado en vez de
// componentes stateful del kit por la misma razón que la Fundación —blindar contra cambios de
// API entre commits del git de gpui-component—. Migrable a `Badge`/tema cuando se fije la rev.
const COLOR_ETIQUETA: u32 = 0xfacc15ff; // amarillo: clave del key-value (espejo del Yellow de la TUI)
const COLOR_TENUE: u32 = 0x9ca3afff; // gris: notas y valores ausentes
const COLOR_OK_FONDO: u32 = 0x16a34a33; // verde translúcido: salud "ok"
const COLOR_OK_PUNTO: u32 = 0x22c55eff; // verde sólido: dot de salud "ok"
const COLOR_MAL_FONDO: u32 = 0xef444433; // rojo translúcido: salud caída / sin datos
const COLOR_MAL_PUNTO: u32 = 0xef4444ff; // rojo sólido: dot de salud caída

/// Render de la pantalla Acceso. Panel de key-values (broker_url, endpoint, versión, token) más
/// un chip de estado de salud y, si procede, un banner de error. Solo lectura.
pub fn render_acceso(datos: &EstadoPantalla) -> impl IntoElement {
    // Endpoint real anunciado por el broker (host:puerto) si ya llegó `/admin/info`; si no,
    // se muestra la URL configurada como respaldo para no dejar el campo vacío.
    let endpoint = match &datos.info {
        Some(info) => format!("{}:{}", info.host, info.puerto),
        None => datos.acceso_url.clone(),
    };
    let version = datos
        .info
        .as_ref()
        .map(|i| i.version.clone())
        .unwrap_or_else(|| "(desconocida)".to_string());

    // Estado de salud: el broker anuncia "ok" u otro texto + nº de instancias vivas. Se traduce
    // a un chip coloreado para que el jefe vea de un vistazo si el broker responde.
    let (salud_texto, salud_ok) = match &datos.salud {
        Some(s) => (
            format!("{} · {} instancias", s.estado, s.instancias),
            s.estado == "ok",
        ),
        None => ("sin datos todavía".to_string(), false),
    };

    div()
        .v_flex()
        .size_full()
        .gap_4()
        .p_6()
        .child(div().text_xl().child(SharedString::from("Red / Acceso")))
        // Panel de key-values: filas etiqueta→valor. Se usa `div` en vez de Table porque son
        // pocos campos fijos, sin orden ni selección.
        .child(
            div()
                .v_flex()
                .gap_2()
                .child(fila_kv("broker_url", &datos.acceso_url))
                .child(fila_kv("endpoint", &endpoint))
                .child(fila_kv("versión", &version))
                .child(fila_kv("token", &datos.acceso_token))
                // El estado de salud usa un chip coloreado (dot + texto), no texto plano.
                .child(
                    div()
                        .h_flex()
                        .gap_3()
                        .items_center()
                        .child(etiqueta("salud"))
                        .child(chip_salud(salud_ok, salud_texto)),
                ),
        )
        // Banner de error si la última comprobación de acceso falló (offline / 401 / otro). El
        // mensaje de `ErrorBroker` ya distingue la causa (Display implementado en el cliente).
        .when_some(datos.error_acceso.as_ref(), |d, err| {
            d.child(
                div()
                    .rounded_md()
                    .px_3()
                    .py_2()
                    .bg(gpui::rgba(COLOR_MAL_FONDO))
                    .child(SharedString::from(format!("Error de acceso: {err}"))),
            )
        })
        // Nota de solo-lectura, igual que en la TUI: el cambio real se hace en Config.
        .child(
            div()
                .text_sm()
                .text_color(gpui::rgba(COLOR_TENUE))
                .child(SharedString::from(
                    "El token se muestra enmascarado. Para cambiarlo, ve a la pantalla Config.",
                )),
        )
}

/// Una fila etiqueta→valor del panel. Etiqueta de ancho fijo para alinear los valores en columna,
/// imitando el `{etiqueta:<12}` de la TUI.
fn fila_kv(etiqueta_txt: &str, valor: &str) -> impl IntoElement {
    div()
        .h_flex()
        .gap_3()
        .items_center()
        .child(etiqueta(etiqueta_txt))
        .child(div().child(SharedString::from(valor.to_string())))
}

/// Etiqueta de campo: ancho fijo y color de acento, para separar visualmente clave de valor.
fn etiqueta(texto: &str) -> impl IntoElement {
    div()
        .w(gpui::px(120.0))
        .text_color(gpui::rgba(COLOR_ETIQUETA))
        .child(SharedString::from(texto.to_string()))
}

/// Chip de estado de salud: píldora con un dot de color + texto. Verde si el broker anuncia
/// "ok", rojo en cualquier otro caso (caído, estado inesperado o sin datos todavía).
fn chip_salud(ok: bool, texto: String) -> impl IntoElement {
    let (fondo, punto) = if ok {
        (COLOR_OK_FONDO, COLOR_OK_PUNTO)
    } else {
        (COLOR_MAL_FONDO, COLOR_MAL_PUNTO)
    };
    div()
        .h_flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .px_3()
        .py_1()
        .bg(gpui::rgba(fondo))
        // Dot circular de color sólido a la izquierda del texto.
        .child(
            div()
                .size(gpui::px(8.0))
                .rounded_full()
                .bg(gpui::rgba(punto)),
        )
        .child(SharedString::from(texto))
}
