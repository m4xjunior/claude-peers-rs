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
    div, prelude::FluentBuilder, px, rgb, Action, AnyElement, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled,
};
// `v_flex`/`h_flex`/`min_w_0` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;
use peers_core::{ColaResumen, Mensaje};

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
/// que es la clave que el endpoint espera (borra cola + outbox de ese id, idempotente). Desde
/// redis-04 NADIE la despacha directo desde la UI: el flujo pasa por `PedirPurga` → confirmación
/// → `ConfirmarPurga`; esta acción queda como vía programática del POST ya confirmado.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = redis, no_json)]
pub struct PurgarPeer {
    pub id: String,
}

// Acciones sin datos: cerrar el pop-up de inspección de cola (redis-01), refrescar manualmente el
// snapshot del almacén (redis-05), y confirmar/cancelar la purga pendiente (redis-04).
gpui::actions!(redis, [CerrarColaRedis, RefrescarRedis, ConfirmarPurga, CancelarPurga]);

/// Abrir el pop-up de INSPECCIÓN de la cola del peer `id` (redis-01): doble-click en la fila o
/// botón "Ver cola". `AppDesktop` carga los mensajes pendientes vía `GET /admin/historial?id=&
/// estado=enviado` (la aproximación con endpoint EXISTENTE que marca la RFC; el contenido crudo
/// exacto de la bandeja exigiría `GET /admin/bandeja`, que NO existe → pendiente de backend).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = redis, no_json)]
pub struct AbrirColaRedis {
    pub id: String,
}

/// Pedir la PURGA del peer `id` (redis-04): abre el diálogo destructivo de confirmación que
/// nombra el peer y los recuentos exactos (mensajes + outbox) que se borrarán. El POST real
/// (`/admin/purgar`) sólo sale al confirmar.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = redis, no_json)]
pub struct PedirPurga {
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

    // Cabecera Ethos: eyebrow + título + conteo, y a la derecha el REFRESCO manual (redis-05):
    // botón "↻ Refrescar" + sello "actualizado hace Xs" (edad de la última respuesta OK, contada
    // con el reloj MONOTÓNICO local — es metadato de frescura de la UI, no un tiempo de trabajo,
    // que esos siempre los timbra el broker).
    let frescura = match &datos.redis_actualizado_en {
        Some(instante) => format!("actualizado hace {}s", instante.elapsed().as_secs()),
        None => "sin datos todavía".to_string(),
    };
    raiz = raiz.child(
        div()
            .h_flex()
            .items_end()
            .justify_between()
            .child(
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
            )
            .child(
                div()
                    .v_flex()
                    .items_end()
                    .gap_1()
                    .child(
                        tema::boton_secundario("redis-refrescar", "↻ Refrescar").on_click(
                            |_e, window, cx| {
                                window.dispatch_action(Box::new(RefrescarRedis), cx);
                            },
                        ),
                    )
                    .child(tema::eyebrow(frescura)),
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
            let id_ver = id.clone();
            let id_purga = id.clone();
            fila
                .child(tema::texto_terciario(format!("Seleccionado: {id}")))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        // redis-01: inspeccionar el contenido de la cola (pop-up).
                        .child(tema::boton_primario("redis-ver-cola", "Ver cola").on_click(
                            move |_evento, window, cx| {
                                window.dispatch_action(
                                    Box::new(AbrirColaRedis { id: id_ver.clone() }),
                                    cx,
                                );
                            },
                        ))
                        // redis-04: purgar YA NO es un click directo — abre la confirmación
                        // destructiva con los recuentos exactos.
                        .child(
                            tema::boton_secundario("redis-purgar", "Purgar cola + outbox")
                                .on_click(move |_evento, window, cx| {
                                    window.dispatch_action(
                                        Box::new(PedirPurga {
                                            id: id_purga.clone(),
                                        }),
                                        cx,
                                    );
                                }),
                        ),
                )
                .into_any_element()
        }
        None => fila
            .child(tema::texto_terciario(
                "Selecciona una cola para inspeccionarla o purgarla",
            ))
            .into_any_element(),
    }
}

/// Una fila de cola SELECCIONABLE: peer + nº de mensajes pendientes. Toda la fila es clicable →
/// despacha `SeleccionarColaRedis { indice }`. La fila activa se resalta (brasa tenue + borde
/// izquierdo dorado) vía `tema::fila_seleccionable`.
fn fila_cola(idx: usize, c: &ColaResumen, activa: bool) -> impl IntoElement {
    let id_abrir = c.id.clone();
    tema::fila_seleccionable(SharedString::from(format!("redis-cola-fila-{idx}")), activa)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(tema::texto_primario(c.id.clone())),
        )
        .child(celda_num(&c.pendientes.to_string()))
        .on_click(move |evento, window, cx| {
            // Doble-click abre la inspección de la cola (redis-01); click simple selecciona.
            if evento.click_count() >= 2 {
                window.dispatch_action(
                    Box::new(AbrirColaRedis {
                        id: id_abrir.clone(),
                    }),
                    cx,
                );
            } else {
                window.dispatch_action(Box::new(SeleccionarColaRedis { indice: idx }), cx);
            }
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

// -------------------------------------------------------------------------------------------------
// MODALES (redis-01/03/04) — contenido puro de los overlays que `AppDesktop` monta en su render
// raíz. NOTA de alcance (redis-02): la inspección del OUTBOX queda PENDIENTE DE BACKEND — exigiría
// `GET /admin/outbox?id=` que hoy no existe (el broker sólo expone el contador vía /admin/redis);
// no se inventa el endpoint.
// -------------------------------------------------------------------------------------------------

/// Rojo terroso destructivo (mismo tono que el resto de confirmaciones de la app).
const ROJO_PURGA: u32 = 0xC0_4A_3E;
const ROJO_PURGA_HOVER: u32 = 0xD0_5A_4E;

/// Extrae `HH:MM:SS` de un ISO 8601 (copia local del helper de las demás vistas).
fn hora_iso(iso: &str) -> String {
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        let fin = resto.find(['.', 'Z', '+']).unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

/// Recorta a `max` caracteres con elipsis, respetando fronteras de carácter.
fn recortar(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_string();
    }
    let recortado: String = texto.chars().take(max.saturating_sub(1)).collect();
    format!("{recortado}…")
}

/// Contenido del pop-up "Cola de {peer}" (redis-01): los mensajes PENDIENTES de la bandeja del
/// peer, aproximados con `GET /admin/historial?estado=enviado` (el endpoint EXISTENTE que marca la
/// RFC; el contenido crudo exacto exigiría `/admin/bandeja`, pendiente de backend). Cada fila:
/// id + de + texto recortado + hora + botón ↻ que REUTILIZA el flujo de reenvío con confirmación
/// de Trazabilidad (redis-03, `PedirReenvio` → mini-modal → `ConfirmarReenvio`).
pub fn render_modal_cola(id: &str, mensajes: &[Mensaje], cargando: bool) -> AnyElement {
    let mut modal = div()
        .v_flex()
        .w(px(640.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(tema::eyebrow("almacén · inspección"))
                        .child(tema::titulo(format!("Cola de {id}")).text_size(px(18.0))),
                )
                .child(
                    div()
                        .id("modal-cola-cerrar-x")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(tema::radio(tema::RADIO_CONTROL))
                        .text_color(tema::HUMO)
                        .cursor_pointer()
                        .hover(|s| s.bg(tema::TINTA).text_color(tema::PAPEL))
                        .child(SharedString::from("✕"))
                        .on_click(|_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarColaRedis), cx);
                        }),
                ),
        )
        // Nota de contrato: qué es exactamente lo que se lista (transparencia sobre la fuente).
        .child(tema::texto_terciario(
            "Mensajes en estado «enviado» según el historial durable (pendientes de entrega).",
        ));

    if cargando {
        modal = modal.child(tema::texto_terciario("Cargando…"));
    } else if mensajes.is_empty() {
        modal = modal.child(tema::texto_terciario(
            "Sin mensajes pendientes en la bandeja de este peer.",
        ));
    } else {
        let mut lista = div()
            .id("modal-cola-scroll")
            .v_flex()
            .gap_1()
            .max_h(px(320.0))
            .overflow_y_scroll();
        for m in mensajes {
            let msg_id = m.id;
            lista = lista.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(tema::radio(tema::RADIO_CONTROL))
                    .hover(|s| s.bg(tema::TINTA))
                    .child(
                        div()
                            .w(px(56.0))
                            .flex_shrink_0()
                            .font_family(tema::FUENTE_MONO)
                            .text_xs()
                            .text_color(tema::HUMO)
                            .child(SharedString::from(format!("#{}", m.id))),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(tema::HUMO)
                            .child(SharedString::from(recortar(&m.de_id, 16))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_sm()
                            .text_color(tema::PAPEL)
                            .child(SharedString::from(recortar(&m.texto, 56))),
                    )
                    .child(
                        div()
                            .w(px(72.0))
                            .flex_shrink_0()
                            .font_family(tema::FUENTE_MONO)
                            .text_xs()
                            .text_color(tema::HUMO)
                            .child(SharedString::from(hora_iso(&m.enviado_en))),
                    )
                    // redis-03: reenviar el ítem — REUTILIZA la confirmación de Trazabilidad.
                    .child(
                        div()
                            .id(gpui::ElementId::Name(
                                format!("redis-reenviar-{}", m.id).into(),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(28.0))
                            .rounded(tema::radio(tema::RADIO_CONTROL))
                            .text_color(tema::HUMO)
                            .cursor_pointer()
                            .hover(|s| s.bg(tema::TINTA).text_color(tema::BRASA))
                            .child(SharedString::from("↻"))
                            .on_click(move |_e, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::vista::trazabilidad::PedirReenvio { msg_id }),
                                    cx,
                                );
                            }),
                    ),
            );
        }
        modal = modal.child(lista);
    }

    modal
        .child(
            div().h_flex().gap_3().pt_2().child(
                tema::boton_secundario("modal-cola-cerrar", "Cerrar").on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(CerrarColaRedis), cx);
                }),
            ),
        )
        .into_any_element()
}

/// Diálogo de CONFIRMACIÓN de purga (redis-04): nombra el peer y los recuentos EXACTOS que se
/// borrarán (mensajes + outbox, sacados del snapshot ya cargado). Botón de peligro rojo; Cancelar
/// (o ✕ / clic fuera / Esc) no toca nada.
pub fn render_modal_purga(id: &str, en_cola: usize, en_outbox: usize) -> AnyElement {
    div()
        .v_flex()
        .w(px(520.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        .child(tema::titulo(format!("¿Purgar {id}?")).text_size(px(18.0)))
        .child(tema::texto_primario(format!(
            "Se borrarán {en_cola} mensaje(s) de la cola y {en_outbox} ítem(s) del outbox."
        )))
        .child(tema::texto_terciario(
            "Irreversible: la bandeja activa y el outbox de este peer se vacían (el historial \
             durable se conserva).",
        ))
        .child(
            div()
                .h_flex()
                .gap_3()
                .pt_2()
                .child(
                    tema::boton_secundario("modal-purga-cancelar", "Cancelar").on_click(
                        |_e, window, cx| {
                            window.dispatch_action(Box::new(CancelarPurga), cx);
                        },
                    ),
                )
                .child(
                    div()
                        .id("modal-purga-confirmar")
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_4()
                        .py_2()
                        .rounded(px(tema::RADIO_CONTROL))
                        .bg(rgb(ROJO_PURGA))
                        .text_color(tema::PAPEL)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(ROJO_PURGA_HOVER)))
                        .child(SharedString::from("Sí, purgar"))
                        .on_click(|_e, window, cx| {
                            window.dispatch_action(Box::new(ConfirmarPurga), cx);
                        }),
                ),
        )
        .into_any_element()
}
