//! Pantalla Redis — colas de mensajes y outbox pendientes por peer. Puerto GPUI de
//! `peers-tui/src/ui/redis.rs`.
//!
//! INTENCIÓN: replicar 1:1 lo que muestra la TUI — DOS tablas lado a lado: a la izquierda las
//! colas de mensajes (peer → nº de mensajes), a la derecha el outbox pendiente (peer → nº de
//! ítems). La TUI es SOLO LECTURA salvo la tecla `p` (purgar la cola seleccionada); aquí esa única
//! escritura es el botón "Purgar" del peer seleccionado, que hace `POST /admin/purgar` a través del
//! `ClienteBroker` y borra cola + outbox de ese peer (idempotente).
//!
//! REDISEÑO ETHOS (por qué se reescribió): la versión anterior usaba azules/grises genéricos
//! (0x1A1A28, 0xEAB308, 0x6B7280…) y no era operable como la TUI (filas no seleccionables). Ahora:
//!  - Todo el color sale del módulo `tema` (paleta pergamino/tinta + acento brasa). CERO literales
//!    de color hardcodeados salvo el rojo del banner de error (semántica de fallo, no de marca).
//!  - La tabla de colas es OPERABLE como la TUI: cada fila es `tema::fila_seleccionable`, clicable,
//!    y despacha `SeleccionarColaRedis { indice }`. La fila activa se resalta con brasa tenue.
//!  - La acción `Purgar` cuelga del peer SELECCIONADO (espejo de la tecla `p` sobre la fila
//!    resaltada), no una por fila: así la escritura sigue el mismo modelo mental que la TUI.
//!
//! Decisión de diseño (por qué NO uso `Table` del kit): igual que la Fundación evitó `Sidebar` y
//! Alertas evitó `Table`/`Dialog` por inestabilidad de API entre commits del git, aquí las dos
//! tablas se pintan con `div().v_flex()` de filas. La firma del stub sigue siendo la de la Fundación
//! — `render_redis(&EstadoPantalla) -> impl IntoElement`, sin `cx` ni estado propio — y la vista
//! permanece PURA: sólo LEE `EstadoPantalla` y DESPACHA acciones; toda mutación (selección, POST +
//! recarga) vive en `AppDesktop`, que es quien tiene `cx`. Los botones son `tema::boton_*` (no el
//! `Button` del kit) para mantener el look Ethos exacto y no atarnos a su tema.

use gpui::{
    div, prelude::FluentBuilder, px, rgb, Action, AnyElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled,
};
// `v_flex`/`h_flex`/`min_w_0` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;
use peers_core::ColaResumen;

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — la vista es pura (sin `cx`), así que no cablea callbacks: DESPACHA acciones que
// `AppDesktop` maneja con `.on_action(cx.listener(...))` en su contenedor raíz. GPUI hace burbujear
// la acción por el árbol desde el elemento clicado hasta ese manejador, sin depender del foco.
// Namespace propio (`redis`) para no colisionar con las acciones de otras pantallas.
//
// `#[action(namespace = redis, no_json)]`: mismo patrón que Alertas. En este rev de gpui el registro
// se hace con `derive(Action)` (la macro `impl_actions!` ya no existe). `no_json` evita exigir
// `serde::Deserialize` + `schemars::JsonSchema`: estas acciones sólo se despachan por código
// (`window.dispatch_action`), nunca desde un keymap JSON.
// -------------------------------------------------------------------------------------------------

/// Seleccionar la fila `indice` de la tabla de COLAS (click en la fila). Índice en la lista actual
/// de colas. `AppDesktop` guardará ese índice en `EstadoPantalla.redis_seleccion_cola` para que la
/// fila se resalte y el botón "Purgar" sepa sobre qué peer actúa. Espejo del cursor de la TUI.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = redis, no_json)]
pub struct SeleccionarColaRedis {
    pub indice: usize,
}

/// Purgar la cola + outbox del peer seleccionado → `POST /admin/purgar`. Lleva el `id` del peer,
/// que es la clave que el endpoint espera (borra cola + outbox de ese id, idempotente). Se despacha
/// desde el botón "Purgar", que sólo aparece cuando hay una cola seleccionada. Espejo de la tecla
/// `p` de la TUI sobre la fila resaltada.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = redis, no_json)]
pub struct PurgarPeer {
    pub id: String,
}

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Punto de entrada de la pantalla (firma estable de la Fundación). Compone: eyebrow + título con el
/// conteo de peers, banner de error (si hubo) y las DOS tablas lado a lado (colas | outbox), cada
/// una en su propia `superficie_card`. Espejo del layout de dos columnas al 50/50 de la TUI.
pub fn render_redis(datos: &EstadoPantalla) -> impl IntoElement {
    // Datos ya cargados por la app; `None` = aún sin primera respuesta u offline (se explica abajo).
    let (total, colas, outbox): (usize, &[ColaResumen], &[ColaResumen]) = match &datos.redis {
        Some(r) => (r.total_instancias, r.colas.as_slice(), r.outbox.as_slice()),
        None => (0, &[], &[]),
    };

    let mut raiz = tema::fondo_app().v_flex().gap_4().p_6();

    // Cabecera Ethos: eyebrow discreto + título display + conteo de peers atenuado.
    raiz = raiz.child(
        div()
            .v_flex()
            .gap_1()
            .child(tema::eyebrow("almacén · redis"))
            .child(
                div()
                    .h_flex()
                    .items_end()
                    .gap_3()
                    .child(tema::titulo("Colas de mensajes"))
                    .child(tema::texto_terciario(format!("{total} peers"))),
            ),
    );

    // Banner de error del broker (offline / 401 / otro), si la última operación falló. Se pinta en
    // vez de dejar las tablas mudas; los datos previos, si los hay, se conservan igualmente debajo.
    if let Some(err) = &datos.error_redis {
        raiz = raiz.child(banner_error(&err.to_string()));
    }

    // Estado sin datos: mismo espíritu que la TUI cuando el broker aún no ha respondido.
    if datos.redis.is_none() {
        raiz = raiz.child(estado_vacio(
            "Sin datos del almacén todavía. Esperando la respuesta de /admin/redis…",
        ));
        return raiz;
    }

    // Índice seleccionado en la tabla de colas (para resaltar la fila y alimentar "Purgar").
    let seleccion = datos.redis_seleccion_cola;

    // Cuerpo: dos tablas al 50/50 (colas seleccionable con acción purgar | outbox solo lectura).
    raiz = raiz.child(
        div()
            .h_flex()
            .size_full()
            .gap_4()
            .child(div().flex_1().min_w_0().child(tabla_colas(colas, seleccion)))
            .child(div().flex_1().min_w_0().child(tabla_outbox(outbox))),
    );

    raiz
}

/// Banner de error: rojo semántico tenue con el motivo del fallo del broker. Es el ÚNICO color
/// hardcodeado de la pantalla (semántica de fallo, no de marca); el resto sale del tema.
fn banner_error(texto: &str) -> AnyElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .bg(rgb(0x7F1D1D))
        .text_color(rgb(0xFEE2E2))
        .child(SharedString::from(format!("⚠ {texto}")))
        .into_any_element()
}

/// Estado sin datos / vacío: aviso atenuado centrado dentro de una superficie card.
fn estado_vacio(texto: &str) -> AnyElement {
    tema::superficie_card()
        .v_flex()
        .items_center()
        .justify_center()
        .size_full()
        .p_6()
        .child(tema::texto_terciario(texto.to_string()))
        .into_any_element()
}

// Ancho fijo (px) de la columna de contador, espejo del Constraint de la TUI (contador estrecho a la
// derecha; el peer ocupa el resto flexible).
const COL_CONTADOR: f32 = 100.0;

/// Tabla IZQUIERDA: colas de mensajes por peer, con filas SELECCIONABLES y botón "Purgar" del peer
/// seleccionado. Espejo de la tabla seleccionable de la TUI (cursor + tecla `p`). Va dentro de una
/// `superficie_card` con su título de sección y la cabecera de columnas en `eyebrow`.
fn tabla_colas(colas: &[ColaResumen], seleccion: Option<usize>) -> AnyElement {
    // Cabecera de columnas: labels discretos (eyebrow) en vez de dorado a pantalla completa.
    let encabezado = div()
        .h_flex()
        .w_full()
        .px_3()
        .py_1()
        .gap_2()
        .child(div().flex_1().min_w_0().child(tema::eyebrow("peer")))
        .child(celda_num_eyebrow("mensajes"));

    // Cuerpo: una fila seleccionable por cola, o aviso atenuado si no hay ninguna.
    let mut cuerpo = div().v_flex().w_full().gap_1();
    if colas.is_empty() {
        cuerpo = cuerpo.child(
            div()
                .px_3()
                .py_2()
                .child(tema::texto_terciario("(sin colas)")),
        );
    } else {
        for (idx, c) in colas.iter().enumerate() {
            let activa = seleccion == Some(idx);
            cuerpo = cuerpo.child(fila_cola(idx, c, activa));
        }
    }

    // Barra de acción inferior: "Purgar" del peer seleccionado (espejo de la tecla `p`). Sólo se
    // muestra si hay una cola seleccionada válida; si no, un texto guía atenuado.
    let barra_accion = barra_purgar(colas, seleccion);

    tema::superficie_card()
        .v_flex()
        .size_full()
        .gap_2()
        .p_4()
        .child(tema::texto_primario("Colas de mensajes"))
        .child(encabezado)
        .child(cuerpo)
        .child(barra_accion)
        .into_any_element()
}

/// Barra inferior de la tabla de colas con el botón "Purgar" del peer seleccionado. Modelo mental
/// de la TUI: seleccionas una fila y pulsas `p`. Aquí: seleccionas (click) y pulsas "Purgar".
fn barra_purgar(colas: &[ColaResumen], seleccion: Option<usize>) -> AnyElement {
    // Peer seleccionado (si el índice sigue siendo válido tras una recarga).
    let peer_sel = seleccion.and_then(|i| colas.get(i)).map(|c| c.id.clone());

    let fila = div().h_flex().items_center().justify_between().w_full().pt_2().gap_2();

    match peer_sel {
        Some(id) => {
            let id_payload = id.clone();
            fila
                .child(tema::texto_terciario(format!("Seleccionado: {id}")))
                .child(
                    tema::boton_secundario("redis-purgar", "Purgar cola + outbox").on_click(
                        move |_evento, window, cx| {
                            // Despacha la acción; `AppDesktop` hace el POST y recarga. La vista no toca `cx`.
                            window.dispatch_action(
                                Box::new(PurgarPeer {
                                    id: id_payload.clone(),
                                }),
                                cx,
                            );
                        },
                    ),
                )
                .into_any_element()
        }
        None => fila
            .child(tema::texto_terciario(
                "Selecciona una cola para purgarla",
            ))
            .into_any_element(),
    }
}

/// Una fila de cola SELECCIONABLE: peer + nº de mensajes pendientes. Toda la fila es clicable →
/// despacha `SeleccionarColaRedis { indice }`. La fila activa se resalta (brasa tenue + borde
/// izquierdo dorado) vía `tema::fila_seleccionable`.
fn fila_cola(idx: usize, c: &ColaResumen, activa: bool) -> impl IntoElement {
    tema::fila_seleccionable(SharedString::from(format!("redis-cola-fila-{idx}")), activa)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(tema::texto_primario(c.id.clone())),
        )
        .child(celda_num(&c.pendientes.to_string()))
        .on_click(move |_evento, window, cx| {
            // Despacha la selección; `AppDesktop` guarda el índice y muta el estado. La vista no toca `cx`.
            window.dispatch_action(Box::new(SeleccionarColaRedis { indice: idx }), cx);
        })
}

/// Tabla DERECHA: outbox pendiente por peer (SOLO LECTURA, sin acción). Espejo de la tabla derecha
/// de la TUI. Filas no seleccionables (la TUI tampoco actúa sobre el outbox). Cabecera "peer / outbox".
fn tabla_outbox(outbox: &[ColaResumen]) -> AnyElement {
    let encabezado = div()
        .h_flex()
        .w_full()
        .px_3()
        .py_1()
        .gap_2()
        .child(div().flex_1().min_w_0().child(tema::eyebrow("peer")))
        .child(celda_num_eyebrow("outbox"));

    let mut cuerpo = div().v_flex().w_full().gap_1();
    if outbox.is_empty() {
        cuerpo = cuerpo.child(
            div()
                .px_3()
                .py_2()
                .child(tema::texto_terciario("(sin outbox)")),
        );
    } else {
        for (idx, c) in outbox.iter().enumerate() {
            let par = idx % 2 == 0;
            cuerpo = cuerpo.child(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .rounded(tema::radio(tema::RADIO_CONTROL))
                    // Cebra sutil sobre la superficie card: las filas pares se sientan sobre TINTA
                    // (el fondo base), dando ritmo sin introducir un color nuevo.
                    .when(par, |d| d.bg(tema::TINTA))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(tema::texto_primario(c.id.clone())),
                    )
                    .child(celda_num(&c.pendientes.to_string())),
            );
        }
    }

    tema::superficie_card()
        .v_flex()
        .size_full()
        .gap_2()
        .p_4()
        .child(tema::texto_primario("Outbox pendiente"))
        .child(encabezado)
        .child(cuerpo)
        .into_any_element()
}

/// Celda numérica de datos, ancho fijo, alineada a la derecha y en fuente mono (los contadores se
/// leen mejor tabulados). Mono cae a monospace del sistema si IBM Plex Mono no está instalada.
fn celda_num(texto: &str) -> impl IntoElement {
    div()
        .w(px(COL_CONTADOR))
        .flex()
        .justify_end()
        .font_family(tema::FUENTE_MONO)
        .text_color(tema::PAPEL)
        .child(SharedString::from(texto.to_string()))
}

/// Label de columna numérica: eyebrow (mono/humo/mayúsculas) alineado a la derecha, mismo ancho que
/// la celda de datos para que columna y dato queden a plomo.
fn celda_num_eyebrow(texto: &str) -> impl IntoElement {
    div()
        .w(px(COL_CONTADOR))
        .flex()
        .justify_end()
        .child(tema::eyebrow(texto.to_string()))
}
