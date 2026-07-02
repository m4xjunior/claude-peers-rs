//! Pantalla Broker — salud e info de arranque del broker, con el tema Ethos aplicado.
//!
//! Espejo de `peers-tui/src/ui/broker.rs`: tres bloques de key-values obtenidos de
//! `GET /admin/info` (host, puerto, versión, instancias), `GET /salud` (estado + instancias
//! vivas) y `GET /factor-estimacion` (factor aprendido + nº de muestras). La app rellena
//! `EstadoPantalla::{info,salud,factor}` con `cx.spawn`; aquí sólo se pinta lo que haya.
//!
//! INTENCIÓN de diseño (por qué): esta pantalla es SOLO LECTURA (la TUI no permite mutar nada
//! aquí). Por eso la vista permanece PURA —`render_broker(&EstadoPantalla) -> impl IntoElement`,
//! sin `cx` ni estado propio— y no cablea callbacks. La única interacción posible es RECARGAR los
//! tres endpoints; se ofrece como botón que DESPACHA `RecargarBroker`, que `AppDesktop` maneja en
//! Fase 3 con `.on_action(cx.listener(...))` (mismo patrón que `vista/alertas.rs`). Sin esa acción
//! la pantalla sigue siendo válida (se refresca con el ciclo de carga general de la app).
//!
//! TEMA: todo el color viene de `crate::tema` (tinta/papel/brasa/humo/línea). Los estados
//! semánticos (salud ok/degradada, dato destacado) se pintan con `chip_estado` del tema, no con
//! azules/verdes hardcodeados: "ok" en brasa (acento de marca), degradado en humo. Los valores de
//! datos usan la fuente mono (`FUENTE_MONO`); las etiquetas de columna, `eyebrow` (mayúsculas,
//! humo). Los paneles son `superficie_card`.

use gpui::{
    div, px, Action, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled,
};

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — la vista es pura (sin `cx`): DESPACHA acciones que `AppDesktop` maneja con
// `.on_action(cx.listener(...))` en su contenedor raíz (mismo patrón documentado en
// `vista/alertas.rs`). Namespace `broker` para no colisionar con otras pantallas. `no_json` evita
// exigir `serde::Deserialize`/`schemars::JsonSchema`: sólo se despachan por código, nunca desde un
// keymap, así que basta `Clone + PartialEq`.
// -------------------------------------------------------------------------------------------------

/// Recargar los tres bloques del broker (`GET /admin/info` + `GET /salud` + `GET /factor-estimacion`).
/// Sin payload: `AppDesktop` sabe qué endpoints refrescar. Botón "Recargar" de la cabecera.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = broker, no_json)]
pub struct RecargarBroker;

// -------------------------------------------------------------------------------------------------
// HELPERS DE FILA — key-value con la etiqueta como `eyebrow` (mayúsculas/humo/mono, ancho fijo) y
// el valor en mono/papel. Centraliza el layout para no repetirlo por campo.
// -------------------------------------------------------------------------------------------------

/// Ancho fijo de la columna de etiqueta (px). Alinea los valores de los tres bloques.
const ANCHO_ETIQUETA: f32 = 160.0;

/// Fila `etiqueta → valor`: etiqueta como eyebrow (ancho fijo) + valor en mono/papel.
fn campo(etiqueta: impl Into<SharedString>, valor: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_4()
        .py_1()
        .child(tema::eyebrow(etiqueta).w(px(ANCHO_ETIQUETA)))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::PAPEL)
                .child(valor.into()),
        )
}

/// Fila `etiqueta → chip`: para el estado de salud (chip coloreado en vez de texto plano).
fn campo_chip(etiqueta: impl Into<SharedString>, chip: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_4()
        .py_1()
        .child(tema::eyebrow(etiqueta).w(px(ANCHO_ETIQUETA)))
        .child(chip)
}

/// Aviso "sin datos todavía" para un bloque cuyo endpoint aún no respondió: texto terciario (humo).
fn sin_datos(texto: impl Into<SharedString>) -> impl IntoElement {
    tema::texto_terciario(texto).py_1()
}

/// Un panel: `superficie_card` con un `eyebrow` de título arriba y el contenido debajo.
fn panel(titulo: impl Into<SharedString>, contenido: impl IntoElement) -> impl IntoElement {
    tema::superficie_card()
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .child(tema::eyebrow(titulo))
        .child(contenido)
}

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

pub fn render_broker(datos: &EstadoPantalla) -> impl IntoElement {
    // Bloque 1 — arranque del broker (GET /admin/info). Si no hay datos, aviso terciario.
    let bloque_info = match &datos.info {
        Some(info) => div()
            .flex()
            .flex_col()
            .child(campo("host", info.host.clone()))
            .child(campo("puerto", info.puerto.to_string()))
            .child(campo("version", info.version.clone()))
            .child(campo("instancias", info.instancias.to_string()))
            .into_any_element(),
        None => sin_datos("Sin datos de /admin/info todavía").into_any_element(),
    };

    // Bloque 2 — salud (GET /salud). El estado se pinta como chip: brasa si "ok", humo si no.
    let bloque_salud = match &datos.salud {
        Some(s) => {
            let color = if s.estado == "ok" { tema::BRASA } else { tema::HUMO };
            div()
                .flex()
                .flex_col()
                .child(campo_chip("estado", tema::chip_estado(s.estado.clone(), color)))
                .child(campo("instancias vivas", s.instancias.to_string()))
                .into_any_element()
        }
        None => sin_datos("Sin datos de /salud todavía").into_any_element(),
    };

    // Bloque 3 — factor de estimación (GET /factor-estimacion). El broker lo aprende del tiempo
    // real; aquí sólo se muestra: "6.2x" destacado en brasa + "· N muestras" tenue (humo).
    let bloque_factor = match &datos.factor {
        Some(fe) => div()
            .flex()
            .flex_row()
            .items_baseline()
            .gap_2()
            .child(
                div()
                    .font_family(tema::FUENTE_MONO)
                    .text_size(px(28.0))
                    .text_color(tema::BRASA)
                    .child(SharedString::from(format!("{:.1}x", fe.factor))),
            )
            .child(
                div()
                    .font_family(tema::FUENTE_MONO)
                    .text_color(tema::HUMO)
                    .child(SharedString::from(format!("· {} muestras", fe.muestras))),
            )
            .into_any_element(),
        None => sin_datos("Sin datos de /factor-estimacion todavía").into_any_element(),
    };

    // Cabecera: título display + botón secundario "Recargar" (despacha `RecargarBroker`).
    let cabecera = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(tema::titulo("Broker"))
        .child(
            tema::boton_secundario("broker-recargar", "Recargar").on_click(
                move |_evento, window, cx| {
                    // Despacha; `AppDesktop` refresca los tres endpoints. La vista no toca `cx`.
                    window.dispatch_action(Box::new(RecargarBroker), cx);
                },
            ),
        );

    // Layout: fondo Ethos (tinta/papel/Inter) + cabecera + tres paneles apilados en columna.
    tema::fondo_app()
        .flex()
        .flex_col()
        .gap_5()
        .p_6()
        .child(cabecera)
        .child(panel("Arranque", bloque_info))
        .child(panel("Salud", bloque_salud))
        .child(panel("Estimación", bloque_factor))
}
