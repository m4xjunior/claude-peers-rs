//! Pantalla Acceso — estado de conexión y autenticación con el broker.
//!
//! INTENCIÓN: espejo de la pantalla "Red / Acceso" de la TUI (`peers-tui/src/ui/acceso.rs`),
//! que es SOLO LECTURA: muestra a qué broker apunta la desktop (URL base), el token enmascarado
//! y —ampliando respecto a la TUI— el estado de salud anunciado por el propio broker. La edición
//! real del token/URL vive en la pantalla Config; aquí solo se consulta.
//!
//! REDISEÑO ETHOS (por qué): la versión previa usaba colores hardcodeados (amarillo `0xfacc15`,
//! verde/rojo genéricos) y `text_xl` suelto, incoherente con el resto. Ahora TODO pasa por el
//! tema (`crate::tema`): fondo tinta, texto pergamino, acento brasa con moderación, `eyebrow`
//! para los labels de campo, `superficie_card` para los paneles y la fuente mono para los datos
//! (url/token/endpoint) — que es dato técnico, no prosa. El chip de salud usa `chip_estado` del
//! tema con un color de severidad decidido AQUÍ (verde brasa-friendly / rojo), porque el tema no
//! conoce el dominio "salud del broker".
//!
//! INTERACTIVIDAD (nueva respecto a la TUI): un botón "Comprobar acceso" que RE-CONSULTA
//! `/admin/info` + `/salud` para refrescar el estado sin reiniciar la app. La vista es PURA (sin
//! `cx`): sólo DESPACHA la acción `RecargarAcceso`; `AppDesktop` la maneja en Fase 3 reutilizando
//! su `cargar_broker` (que ya puebla `info`/`salud`/`factor` y escribe `error_acceso`).
//!
//! Reusa `datos.info` (GET /admin/info: host/puerto/versión) y `datos.salud` (GET /salud:
//! estado + nº instancias), ya cargados por la app. La URL base y el token enmascarado vienen
//! cacheados en `EstadoPantalla` porque la vista no recibe el `ClienteBroker`.

use gpui::{
    actions, div, prelude::FluentBuilder, px, rgba, IntoElement, ParentElement,
    Rgba, SharedString, StatefulInteractiveElement, Styled,
};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — la vista es pura (sin `cx`): DESPACHA acciones que `AppDesktop` maneja con
// `.on_action(cx.listener(...))` en su contenedor raíz (mismo patrón que `vista/alertas.rs`).
// Namespace `acceso` para no colisionar con otras pantallas. `no_json` porque sólo se despachan
// por código (`window.dispatch_action`), nunca desde un keymap → no arrastramos `schemars`.
// -------------------------------------------------------------------------------------------------

// Acción sin datos: re-consultar el estado del broker (`/admin/info` + `/salud`) para refrescar
// url/endpoint/versión/salud sin reiniciar. Fase 3 la cablea a `AppDesktop::cargar_broker`
// (que ya puebla `info`/`salud`/`factor` y escribe `error_acceso`).
actions!(acceso, [RecargarAcceso]);

// Colores de SEVERIDAD del chip de salud (los decide esta pantalla, no el tema): verde cuando el
// broker anuncia "ok", rojo (brasa-friendly, apagado) en cualquier otro caso. Se usan con
// `tema::chip_estado`, que pinta texto SALMO oscuro encima para contraste.
const SALUD_OK: Rgba = Rgba { r: 0x4A as f32 / 255.0, g: 0x9A as f32 / 255.0, b: 0x63 as f32 / 255.0, a: 1.0 }; // verde apagado
const SALUD_MAL: Rgba = Rgba { r: 0xC0 as f32 / 255.0, g: 0x4A as f32 / 255.0, b: 0x3E as f32 / 255.0, a: 1.0 }; // rojo terroso

/// Render de la pantalla Acceso. Cabecera (eyebrow + título) + una `superficie_card` con el panel
/// de conexión (key-values en mono, chip de salud) + botón "Comprobar acceso". Banner de error si
/// la última comprobación falló. Nota de solo-lectura al pie.
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
    let salud_color = if salud_ok { SALUD_OK } else { SALUD_MAL };

    tema::fondo_app()
        .v_flex()
        .gap_5()
        .p_6()
        // Cabecera: eyebrow discreto + título display, como el resto de pantallas Ethos.
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("Conexión"))
                .child(tema::titulo("Red / Acceso")),
        )
        // Panel de conexión: card elevada con los key-values (datos técnicos en mono) y el chip de
        // salud. `superficie_card` aporta fondo TINTA2, borde LINEA, radio y sombra sutil.
        .child(
            tema::superficie_card()
                .v_flex()
                .gap_3()
                .p_5()
                .child(fila_kv("broker_url", &datos.acceso_url))
                .child(fila_kv("endpoint", &endpoint))
                .child(fila_kv("versión", &version))
                .child(fila_kv("token", &datos.acceso_token))
                // Separador tenue antes de la salud (línea del tema).
                .child(div().w_full().h(px(1.0)).bg(tema::LINEA))
                // Estado de salud: label eyebrow + chip coloreado (dot + texto).
                .child(
                    div()
                        .h_flex()
                        .gap_3()
                        .items_center()
                        .child(etiqueta("salud"))
                        .child(tema::chip_estado(salud_texto, salud_color)),
                ),
        )
        // Acción: recargar el estado del broker. Botón primario (brasa) que despacha `RecargarAcceso`.
        .child(
            div().h_flex().gap_2().child(
                tema::boton_primario("acceso-recargar", "Comprobar acceso").on_click(
                    |_evento, window, cx| {
                        // La vista no toca `cx`: despacha y `AppDesktop` recarga info/salud.
                        window.dispatch_action(Box::new(RecargarAcceso), cx);
                    },
                ),
            ),
        )
        // Banner de error si la última comprobación de acceso falló (offline / 401 / otro). El
        // mensaje de `ErrorBroker` ya distingue la causa (Display implementado en el cliente).
        .when_some(datos.error_acceso.as_ref(), |d, err| {
            d.child(banner_error(&err.to_string()))
        })
        // Nota de solo-lectura, igual que en la TUI: el cambio real se hace en Config.
        .child(tema::texto_terciario(
            "El token se muestra enmascarado. Para cambiarlo, ve a la pantalla Config.",
        ))
}

/// Una fila etiqueta→valor del panel. Etiqueta de ancho fijo (eyebrow) para alinear los valores en
/// columna; el VALOR va en fuente mono porque son datos técnicos (url/host/token/versión), no prosa.
fn fila_kv(etiqueta_txt: &str, valor: &str) -> impl IntoElement {
    div()
        .h_flex()
        .gap_3()
        .items_center()
        .child(etiqueta(etiqueta_txt))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::PAPEL)
                .text_size(px(13.0))
                .child(SharedString::from(valor.to_string())),
        )
}

/// Etiqueta de campo: ancho fijo + estilo eyebrow (mono/humo/mayúsculas del tema) para separar
/// visualmente clave de valor y alinear la columna, imitando el `{etiqueta:<12}` de la TUI.
fn etiqueta(texto: &str) -> impl IntoElement {
    tema::eyebrow(texto).w(px(120.0))
}

/// Banner de error: superficie roja tenue con el motivo del fallo del broker. Neutro respecto a la
/// variante concreta: el `Display` de `ErrorBroker` ya da el texto (offline/401/otro). Se mantiene
/// coherente con el `banner_error` de las otras pantallas pero calibrado al fondo cálido.
fn banner_error(texto: &str) -> impl IntoElement {
    div()
        .w_full()
        .px_4()
        .py_2()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .border_1()
        .border_color(SALUD_MAL)
        .bg(rgba(0xC04A3E22)) // rojo terroso muy tenue sobre la tinta
        .text_color(tema::PAPEL)
        .child(SharedString::from(format!("⚠ Error de acceso: {texto}")))
}
