//! Pantalla Redis — colas de mensajes y outbox pendientes por peer. Puerto GPUI de
//! `peers-tui/src/ui/redis.rs`.
//!
//! INTENCIÓN: replicar 1:1 lo que muestra la TUI — DOS tablas lado a lado: a la izquierda las
//! colas de mensajes (peer → nº de mensajes), a la derecha el outbox pendiente (peer → nº de
//! ítems). La TUI es SOLO LECTURA salvo la tecla `p` (purgar la cola seleccionada); aquí esa única
//! escritura es el botón "Purgar" por fila de la tabla de colas, que hace `POST /admin/purgar` a
//! través del `ClienteBroker` y borra cola + outbox de ese peer (idempotente).
//!
//! Decisión de diseño (por qué NO uso `Table` del kit): igual que la Fundación evitó el componente
//! `Sidebar` por inestabilidad de API entre commits del git, y que la pantalla Alertas evitó
//! `Table`/`Dialog`, aquí las dos tablas se pintan con `div().v_flex()` de filas. Así la firma del
//! stub sigue siendo la de la Fundación — `render_redis(&EstadoPantalla) -> impl IntoElement`, sin
//! `cx` ni estado propio — y la vista permanece PURA: sólo LEE `EstadoPantalla` y DESPACHA la acción
//! de purgar; toda mutación (POST + recarga) vive en `AppDesktop`, que es quien tiene `cx`. Para la
//! acción uso `Button` de gpui-component (variante `danger`, coherente con "Descartar" en Alertas).

use gpui::{
    div, prelude::FluentBuilder, px, rgb, Action, AnyElement, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    StyledExt,
};
use peers_core::ColaResumen;

use crate::app::EstadoPantalla;

// -------------------------------------------------------------------------------------------------
// ACCIÓN — la vista es pura (sin `cx`), así que no cablea callbacks: DESPACHA una acción que
// `AppDesktop` registra con `.on_action(cx.listener(...))`. El payload lleva sólo el `id` del peer,
// que es lo único que el POST `/admin/purgar` necesita (borra cola + outbox de ese id). Namespace
// propio (`redis`) para no colisionar con las acciones de otras pantallas.
// -------------------------------------------------------------------------------------------------

/// Acción de purgar la cola + outbox de un peer concreto. Lleva el `id` del peer, que es la clave
/// que `POST /admin/purgar` espera. Se despacha desde el botón "Purgar" de cada fila de colas.
///
/// `#[action(namespace = redis, no_json)]`: mismo patrón que las acciones de Alertas. En este rev
/// de gpui el registro de acciones se hace con `derive(Action)` (la macro `impl_actions!` ya no
/// existe). `no_json` evita exigir `serde::Deserialize` + `schemars::JsonSchema`: esta acción sólo
/// se despacha por código (`window.dispatch_action`), nunca desde un keymap JSON.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = redis, no_json)]
pub struct PurgarPeer {
    pub id: String,
}

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Punto de entrada de la pantalla (firma estable de la Fundación). Compone: banner de error (si
/// hubo), cabecera con el conteo de peers y las DOS tablas lado a lado (colas | outbox). Espejo del
/// layout de dos columnas al 50/50 de la TUI.
pub fn render_redis(datos: &EstadoPantalla) -> impl IntoElement {
    // Datos ya cargados por la app; `None` = aún sin primera respuesta u offline (se explica abajo).
    let (total, colas, outbox): (usize, &[ColaResumen], &[ColaResumen]) = match &datos.redis {
        Some(r) => (r.total_instancias, r.colas.as_slice(), r.outbox.as_slice()),
        None => (0, &[], &[]),
    };

    let mut raiz = div().v_flex().size_full().gap_3().p_6();

    // Cabecera: título + nº de peers, espejo del título de la TUI (" Colas de mensajes · N peers ").
    raiz = raiz.child(
        div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(div().text_xl().child(SharedString::from("Redis")))
            .child(
                div()
                    .text_sm()
                    .opacity(0.6)
                    .child(SharedString::from(format!("({total} peers)"))),
            ),
    );

    // Banner de error del broker (offline / 401 / otro), si la última operación falló. Se pinta en
    // vez de dejar las tablas mudas; los datos previos, si los hay, se conservan igualmente debajo.
    if let Some(err) = &datos.error_redis {
        raiz = raiz.child(banner_error(&err.to_string()));
    }

    // Estado sin datos: mismo espíritu que la TUI cuando el broker aún no ha respondido.
    if datos.redis.is_none() {
        raiz = raiz.child(estado_vacio());
        return raiz;
    }

    // Cuerpo: dos tablas al 50/50 (colas seleccionable con acción purgar | outbox solo lectura).
    raiz = raiz.child(
        div()
            .h_flex()
            .size_full()
            .gap_4()
            .child(div().flex_1().child(tabla_colas(colas)))
            .child(div().flex_1().child(tabla_outbox(outbox))),
    );

    raiz
}

/// Banner rojo tenue con el motivo del fallo del broker. Neutro respecto a la variante concreta:
/// el `Display` de `ErrorBroker` ya da el texto correcto (offline/401/otro).
fn banner_error(texto: &str) -> AnyElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x7F1D1D))
        .text_color(rgb(0xFEE2E2))
        .child(SharedString::from(format!("⚠ {texto}")))
        .into_any_element()
}

/// Estado sin datos: aviso mientras no ha llegado la primera respuesta de `/admin/redis`.
fn estado_vacio() -> AnyElement {
    div()
        .v_flex()
        .items_center()
        .justify_center()
        .size_full()
        .text_color(rgb(0x6B7280))
        .child(SharedString::from(
            "Sin datos del almacén todavía. Esperando la respuesta de /admin/redis…",
        ))
        .into_any_element()
}

// Anchos fijos (px) de las columnas, espejo de los Constraint de la TUI (peer flexible + contador
// de 12 chars). El nº va en columna estrecha a la derecha; el peer ocupa el resto.
const COL_CONTADOR: f32 = 90.0;
const COL_ACCION: f32 = 90.0;

/// Tabla IZQUIERDA: colas de mensajes por peer, con botón "Purgar" por fila. Espejo de la tabla
/// seleccionable de la TUI (la acción `p` purga la cola). Cabecera "peer / mensajes / ·".
fn tabla_colas(colas: &[ColaResumen]) -> AnyElement {
    let encabezado = div()
        .h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .text_color(rgb(0xEAB308))
        .font_semibold()
        .child(div().flex_1().child(SharedString::from("peer")))
        .child(celda_num("mensajes", COL_CONTADOR))
        .child(div().w(px(COL_ACCION)).child(SharedString::from("")));

    let mut cuerpo = div().v_flex().w_full().gap_1();
    if colas.is_empty() {
        cuerpo = cuerpo.child(
            div()
                .px_2()
                .py_1()
                .text_color(rgb(0x6B7280))
                .child(SharedString::from("(sin colas)")),
        );
    } else {
        for (idx, c) in colas.iter().enumerate() {
            cuerpo = cuerpo.child(fila_cola(idx, c));
        }
    }

    div()
        .v_flex()
        .size_full()
        .gap_1()
        .child(div().font_semibold().child(SharedString::from("Colas de mensajes")))
        .child(encabezado)
        .child(cuerpo)
        .into_any_element()
}

/// Una fila de cola: peer, nº de mensajes pendientes y botón "Purgar" que despacha `PurgarPeer`.
/// Filas alternas con fondo tenue para legibilidad (la TUI resalta la seleccionada; aquí, al no
/// haber selección de teclado, la acción vive por fila directamente).
fn fila_cola(idx: usize, c: &ColaResumen) -> impl IntoElement {
    // El id del peer se captura por valor para el payload de la acción (el closure es `Fn`).
    let id_peer = c.id.clone();
    let par = idx % 2 == 0;

    div()
        .id(SharedString::from(format!("redis-cola-fila-{idx}")))
        .h_flex()
        .w_full()
        .items_center()
        .px_2()
        .py_1()
        .gap_2()
        .rounded_md()
        .when(par, |d| d.bg(rgb(0x1A1A28)))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .child(SharedString::from(c.id.clone())),
        )
        .child(celda_num(&c.pendientes.to_string(), COL_CONTADOR))
        .child(
            div().w(px(COL_ACCION)).child(
                Button::new(SharedString::from(format!("redis-purgar-{idx}")))
                    .danger()
                    .label("Purgar")
                    .on_click(move |_evento, window, cx| {
                        // Despacha la acción; `AppDesktop` hace el POST y recarga. La vista no toca `cx`.
                        window.dispatch_action(Box::new(PurgarPeer { id: id_peer.clone() }), cx);
                    }),
            ),
        )
}

/// Tabla DERECHA: outbox pendiente por peer (SOLO LECTURA, sin acción). Espejo de la tabla derecha
/// de la TUI. Cabecera "peer / outbox".
fn tabla_outbox(outbox: &[ColaResumen]) -> AnyElement {
    let encabezado = div()
        .h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .text_color(rgb(0xEAB308))
        .font_semibold()
        .child(div().flex_1().child(SharedString::from("peer")))
        .child(celda_num("outbox", COL_CONTADOR));

    let mut cuerpo = div().v_flex().w_full().gap_1();
    if outbox.is_empty() {
        cuerpo = cuerpo.child(
            div()
                .px_2()
                .py_1()
                .text_color(rgb(0x6B7280))
                .child(SharedString::from("(sin outbox)")),
        );
    } else {
        for (idx, c) in outbox.iter().enumerate() {
            let par = idx % 2 == 0;
            cuerpo = cuerpo.child(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .rounded_md()
                    .when(par, |d| d.bg(rgb(0x1A1A28)))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(SharedString::from(c.id.clone())),
                    )
                    .child(celda_num(&c.pendientes.to_string(), COL_CONTADOR)),
            );
        }
    }

    div()
        .v_flex()
        .size_full()
        .gap_1()
        .child(div().font_semibold().child(SharedString::from("Outbox pendiente")))
        .child(encabezado)
        .child(cuerpo)
        .into_any_element()
}

/// Celda numérica de ancho fijo, alineada a la derecha (los contadores se leen mejor así).
fn celda_num(texto: &str, ancho: f32) -> impl IntoElement {
    div()
        .w(px(ancho))
        .flex()
        .justify_end()
        .child(SharedString::from(texto.to_string()))
}
