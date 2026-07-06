//! Pantalla Trazabilidad — historial durable de la cola de un peer (R2.3/R2.4), portada de la
//! TUI (`peers-tui/src/ui/trazabilidad.rs`) a GPUI, ahora con el Design System "Ethos" y
//! operable como la TUI (filas seleccionables, teclado/click, reenviar, selector de peer).
//!
//! QUÉ MUESTRA (espejo 1:1 de la TUI, look Ethos):
//!   1. Cabecera: eyebrow "TRAZABILIDAD" + título con el peer en foco y nº de mensajes.
//!   2. Selector de peer en foco: pills con los peers vivos (`instancias`); el activo va en brasa.
//!      Click → despacha `EnfocarPeer { id }` para recargar el historial de ese peer.
//!   3. Tabla cronológica dentro de una `superficie_card`: `id · de · texto · estado · enviado`,
//!      con la columna "estado" COLOREADA por la máquina de estados (R2.4). Cada fila es
//!      SELECCIONABLE (`fila_seleccionable`): click la selecciona y abre su timeline inline;
//!      la fila activa lleva el resalte brasa tenue del tema.
//!   4. Timeline del mensaje seleccionado (cuando `traza_timeline` está activo): cada hito de la
//!      máquina con su timestamp, intentos/reenvíos, y el botón de acción "Reenviar".
//!
//! DECISIÓN DE DISEÑO (por qué): la vista es PURA (`render_trazabilidad(&EstadoPantalla) ->
//! impl IntoElement`, sin `cx` ni estado propio), igual que Alertas. No cablea callbacks: DESPACHA
//! acciones que `AppDesktop` maneja con `.on_action(cx.listener(...))`. Toda mutación (enfocar peer,
//! seleccionar fila, abrir/cerrar timeline, reenviar y recargar) vive en `AppDesktop`, que tiene
//! `cx`. El timeline se despliega INLINE (no `Dialog` stateful del kit) para conservar esa firma.
//!
//! COLORES DE ESTADO: los estados del mensaje conservan su semántica cromática (gris/ámbar/verde/
//! rojo) porque es información de dominio, no decoración; el resto del cromatismo azul genérico se
//! reemplazó por los tokens del tema (tinta/papel/brasa/humo/línea). El color de estado se sigue
//! centralizando en un solo sitio (`estilo_estado`), espejo de `peers_tui::app::estilo_estado`.

use gpui::{
    div, Action, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;
use peers_core::{EstadoMensaje, Mensaje};

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — la vista es pura (sin `cx`): DESPACHA acciones que `AppDesktop` maneja en su raíz con
// `.on_action(cx.listener(...))`. GPUI las burbujea desde el elemento clicado hasta el manejador.
// Namespace `trazabilidad` para no colisionar con las de otras pantallas. `no_json` evita exigir
// `serde::Deserialize`/`schemars::JsonSchema` (sólo se despachan por código, nunca desde keymap).
// -------------------------------------------------------------------------------------------------

/// Cambiar el peer en foco → `AppDesktop` fija `traza_peer = Some(id)`, resetea `traza_seleccion`
/// y `traza_timeline`, y recarga el historial vía `ClienteBroker::historial(&id)`. Se despacha al
/// clicar un pill del selector de peer.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct EnfocarPeer {
    /// Id del peer a enfocar (uno de `instancias[*].id`).
    pub id: String,
}

/// Seleccionar la fila `indice` de la tabla (click simple). Desde trazabilidad-01 SOLO selecciona
/// (resalte); el detalle se abre con doble-click/Enter → `AbrirMensaje` (modal). Índice sobre
/// `historial`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct SeleccionarMensaje {
    /// Índice de la fila en `historial` (0-based, orden cronológico ascendente del broker).
    pub indice: usize,
}

/// Abrir el POP-UP "Abrir mensaje" (trazabilidad-01): modal con el timeline completo del mensaje
/// `indice` (doble-click en la fila o Enter). `AppDesktop` fija `traza_seleccion = indice` y
/// `traza_timeline = true` (el flag que antes abría el panel inline ahora abre el modal).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct AbrirMensaje {
    pub indice: usize,
}

// Acciones sin datos: cerrar el modal de timeline (✕/Cerrar/clic fuera/Esc), y confirmar/cancelar
// el reenvío pendiente (trazabilidad-05: reenviar SIEMPRE pide confirmación).
gpui::actions!(trazabilidad, [CerrarTimeline, ConfirmarReenvio, CancelarReenvio]);

/// Pedir el REENVÍO del mensaje `msg_id` (trazabilidad-05): abre el mini-modal de confirmación.
/// El POST real (`/admin/reenviar`) sólo sale al confirmar con `ConfirmarReenvio`. Se despacha
/// desde el botón ↻ de la fila y desde el botón "Reenviar" del modal de timeline.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct PedirReenvio {
    /// Id durable del mensaje a reenviar (`Mensaje::id`).
    pub msg_id: i64,
}

/// Reenviar el mensaje `msg_id` → `ClienteBroker::reenviar(msg_id)` y luego recargar el historial.
/// `msg_id` es el `Mensaje::id` (i64), lo que el endpoint `POST /admin/reenviar` espera. Desde
/// trazabilidad-05 SIEMPRE llega vía la confirmación (`PedirReenvio` → `ConfirmarReenvio`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct ReenviarMensaje {
    /// Id durable del mensaje a reenviar (`Mensaje::id`).
    pub msg_id: i64,
}

/// Filtrar el historial por estado (trazabilidad-02): recarga vía
/// `historial(id, estado)` — el filtrado lo hace el BROKER (capacidad ya existente). `None` =
/// chip "Todos" (sin filtro). Se despacha desde los chips segmentados sobre la tabla.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct FiltrarEstadoTraza {
    pub estado: Option<EstadoMensaje>,
}

// -------------------------------------------------------------------------------------------------
// HELPERS PUROS — espejo de los de la TUI, devolviendo tipos de GPUI. La lógica de mapeo es idéntica
// a `peers_tui::app` pero desacoplada de ratatui.
// -------------------------------------------------------------------------------------------------

/// Paleta de la columna "estado", espejo exacto de `peers_tui::app::estilo_estado` (R2.4). Devuelve
/// símbolo+etiqueta y color `Rgba` de gpui para pintarlo con `.text_color(..)`.
///
/// INTENCIÓN: que un solo sitio decida etiqueta y color de cada estado, igual que en la TUI, para
/// que ambas interfaces muestren la misma semántica visual del ciclo de vida del mensaje. Estos
/// colores NO se sustituyen por tokens del tema: son información de dominio (salud del mensaje),
/// no decoración, y deben leerse igual que en la TUI.
fn estilo_estado(estado: EstadoMensaje) -> (&'static str, gpui::Rgba) {
    match estado {
        // Gris: encolado, aún no confirmado por el destino.
        EstadoMensaje::Enviado => ("○ enviado", gpui::rgb(0x9ca3af)),
        // Ámbar: en tránsito de entrega/lectura (dos subestados, mismo color como en la TUI).
        EstadoMensaje::Entregado => ("◑ entregado", gpui::rgb(0xeab308)),
        EstadoMensaje::Leido => ("◑ leído", gpui::rgb(0xeab308)),
        // Verde: el destino terminó de actuar sobre él (estado saludable terminal).
        EstadoMensaje::Procesado => ("● procesado", gpui::rgb(0x22c55e)),
        // Rojo: fallos terminales (recuperable vs muerto), ambos exigen atención del jefe.
        EstadoMensaje::Fallido => ("✕ fallido", gpui::rgb(0xef4444)),
        EstadoMensaje::DeadLetter => ("✕ dead-letter", gpui::rgb(0xef4444)),
    }
}

/// Extrae `HH:MM:SS` de un timestamp ISO 8601 para la columna "enviado". Espejo de
/// `peers_tui::app::hora_iso`: si no hay patrón reconocible, devuelve el texto tal cual (no falla).
fn hora_iso(iso: &str) -> String {
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        let fin = resto.find(['.', 'Z', '+']).unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

/// Recorta un texto a `max` caracteres añadiendo `…` si sobra, respetando fronteras de carácter
/// (no de byte) para no partir un multibyte. Espejo de `peers_tui::app::recortar`.
fn recortar(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_string();
    }
    // Reservamos un hueco para el '…' final.
    let corte = max.saturating_sub(1);
    let recortado: String = texto.chars().take(corte).collect();
    format!("{recortado}…")
}

// Anchos fijos (px) de las columnas no flexibles, espejo de los Constraint de la TUI. "texto" es la
// flexible (`flex_1`).
const COL_ID: f32 = 64.0;
const COL_DE: f32 = 130.0;
const COL_ESTADO: f32 = 128.0;
const COL_ENVIADO: f32 = 96.0;

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Punto de entrada de la pantalla (firma estable de Fundación). Ramifica: guía si no hay datos,
/// tabla + (timeline opcional) si los hay. Todo sobre el fondo/tipografía del tema Ethos.
pub fn render_trazabilidad(datos: &EstadoPantalla) -> impl IntoElement {
    let foco = datos.traza_peer.as_deref();

    // Cabecera: eyebrow + título Ethos. El título lleva el peer en foco y el conteo, o el aviso.
    let titulo_txt = match foco {
        Some(id) => format!("{}  ·  {} mensajes", id, datos.historial.len()),
        None => "Sin peer en foco".to_string(),
    };
    let cabecera = div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow("Trazabilidad"))
        .child(tema::titulo(titulo_txt));

    // Selector de peer: siempre visible si hay instancias vivas, para poder cambiar de foco sin
    // volver a la pantalla Peers (mejora operativa sobre la TUI, que fija el foco desde Peers).
    let selector = selector_peer(datos);

    // Sin foco o sin mensajes: cabecera + selector (+ chips si hay foco) + texto guía. Con filtro
    // activo y 0 resultados, la guía lo dice y los chips siguen visibles para poder quitarlo.
    if foco.is_none() || datos.historial.is_empty() {
        let guia = if foco.is_none() {
            "No hay peer en foco. Elige un peer arriba (o selecciónalo en la pantalla Peers)."
        } else if datos.traza_filtro_estado.is_some() {
            "Ningún mensaje en ese estado. Pulsa «Todos» para quitar el filtro."
        } else {
            "Este peer aún no tiene mensajes en el historial."
        };
        let mut col = div()
            .v_flex()
            .size_full()
            .gap_4()
            .p_6()
            .child(cabecera);
        if let Some(sel) = selector {
            col = col.child(sel);
        }
        if foco.is_some() {
            col = col.child(barra_filtro_estado(datos.traza_filtro_estado));
        }
        return col.child(
            tema::superficie_card()
                .v_flex()
                .items_center()
                .justify_center()
                .flex_1()
                .p_6()
                .child(tema::texto_terciario(guia)),
        );
    }

    // Fila seleccionada acotada al rango válido del historial.
    let sel = datos.traza_seleccion.min(datos.historial.len().saturating_sub(1));

    // Tabla dentro de una superficie card: encabezado + filas seleccionables.
    let filas = datos
        .historial
        .iter()
        .enumerate()
        .map(|(idx, m)| fila_mensaje(idx, m, idx == sel))
        .collect::<Vec<_>>();

    let tabla = tema::superficie_card()
        .v_flex()
        .w_full()
        .p_2()
        .gap_1()
        .child(encabezado_tabla())
        .child(div().v_flex().w_full().gap_1().children(filas));

    // El timeline ya NO se pinta inline (trazabilidad-01): vive en un MODAL que `AppDesktop`
    // monta en su render raíz leyendo `traza_timeline` (ver `render_modal_timeline`).
    let mut columna = tema::raiz_scrollable()
        .v_flex()
        .gap_4()
        .p_6()
        .child(cabecera);
    if let Some(sel_ui) = selector {
        columna = columna.child(sel_ui);
    }
    // Chips de filtro por estado (trazabilidad-02), entre el selector de peer y la tabla.
    columna = columna.child(barra_filtro_estado(datos.traza_filtro_estado));
    columna = columna.child(tabla);
    columna
}

/// Barra de chips segmentados de filtro por estado (trazabilidad-02): "Todos" + un chip por cada
/// `EstadoMensaje`, cada uno con su punto de color semántico. El activo va con fondo brasa tenue +
/// borde brasa; el resto borde línea/humo. Single-select: clicar un chip recarga el historial con
/// ese filtro EN EL BROKER (`FiltrarEstadoTraza`); "Todos" lo quita.
fn barra_filtro_estado(activo: Option<EstadoMensaje>) -> impl IntoElement {
    // Los 6 estados en orden del ciclo de vida (los dos terminales de error al final).
    const ESTADOS: [EstadoMensaje; 6] = [
        EstadoMensaje::Enviado,
        EstadoMensaje::Entregado,
        EstadoMensaje::Leido,
        EstadoMensaje::Procesado,
        EstadoMensaje::Fallido,
        EstadoMensaje::DeadLetter,
    ];

    let mut barra = div().h_flex().flex_wrap().gap_2().items_center();
    barra = barra.child(chip_filtro_estado("Todos", None, activo.is_none()));
    for e in ESTADOS {
        let (etiqueta, _) = estilo_estado(e);
        barra = barra.child(chip_filtro_estado(etiqueta, Some(e), activo == Some(e)));
    }

    div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow("Estado"))
        .child(barra)
}

/// Un chip del filtro por estado. `estado = None` es el chip "Todos". El punto de color semántico
/// del estado va a la izquierda (en "Todos" no hay punto). Despacha `FiltrarEstadoTraza`.
fn chip_filtro_estado(etiqueta: &str, estado: Option<EstadoMensaje>, activo: bool) -> impl IntoElement {
    let id = format!("traza-filtro-{}", etiqueta.replace([' ', '○', '◑', '●', '✕'], ""));

    let mut chip = div()
        .id(gpui::ElementId::Name(id.into()))
        .h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py(gpui::px(4.0))
        .rounded(tema::radio(tema::RADIO_PILL))
        .border_1()
        .cursor_pointer()
        .text_xs()
        .font_family(tema::FUENTE_MONO);

    // Punto de color semántico del estado (no en "Todos").
    if let Some(e) = estado {
        let (_, color) = estilo_estado(e);
        chip = chip.child(
            div()
                .w(gpui::px(7.0))
                .h(gpui::px(7.0))
                .rounded(gpui::px(999.0))
                .bg(color),
        );
    }
    // La etiqueta sin el símbolo del estado (el punto ya lo representa).
    let texto = etiqueta
        .trim_start_matches(['○', '◑', '●', '✕', ' '])
        .to_uppercase();
    let chip = chip.child(SharedString::from(texto));

    let chip = if activo {
        chip.bg(tema::BRASA_TENUE)
            .border_color(tema::BRASA)
            .text_color(tema::PAPEL)
    } else {
        chip.border_color(tema::LINEA)
            .text_color(tema::HUMO)
            .hover(|s| s.bg(tema::TINTA2))
    };

    chip.on_click(move |_e, window, cx| {
        window.dispatch_action(Box::new(FiltrarEstadoTraza { estado }), cx);
    })
}

/// Selector de peer en foco: una fila de pills, uno por peer vivo (`instancias`). El pill del peer
/// activo va en brasa (fondo BRASA + texto SALMO); el resto es un pill de superficie con hover.
/// Click → despacha `EnfocarPeer { id }`. Devuelve `None` si no hay instancias (nada que elegir).
fn selector_peer(datos: &EstadoPantalla) -> Option<impl IntoElement> {
    if datos.instancias.is_empty() {
        return None;
    }
    let foco = datos.traza_peer.as_deref();

    let pills = datos
        .instancias
        .iter()
        .map(|inst| {
            let activo = foco == Some(inst.id.as_str());
            let id_peer = inst.id.clone();
            let base = div()
                .id(SharedString::from(format!("traza-peer-{}", inst.id)))
                .flex()
                .items_center()
                .px_3()
                .py_1()
                .rounded(tema::radio(tema::RADIO_PILL))
                .cursor_pointer()
                .text_sm()
                .child(SharedString::from(inst.id.clone()));
            let pill = if activo {
                base.bg(tema::BRASA).text_color(tema::SALMO)
            } else {
                base.border_1()
                    .border_color(tema::LINEA)
                    .text_color(tema::PAPEL)
                    .hover(|s| s.bg(tema::TINTA2))
            };
            pill.on_click(move |_e, window, cx| {
                window.dispatch_action(Box::new(EnfocarPeer { id: id_peer.clone() }), cx);
            })
        })
        .collect::<Vec<_>>();

    Some(
        div()
            .v_flex()
            .gap_1()
            .child(tema::eyebrow("Peer en foco"))
            .child(div().h_flex().flex_wrap().gap_2().children(pills)),
    )
}

/// Fila de encabezado de la tabla: rótulos como "eyebrow" (mono/humo/mayúsculas), mismas
/// proporciones de columna que la TUI. "texto" es la columna flexible (`flex_1`).
fn encabezado_tabla() -> impl IntoElement {
    div()
        .h_flex()
        .w_full()
        .gap_2()
        .px_3()
        .py_1()
        .child(div().w(gpui::px(COL_ID)).child(tema::eyebrow("id")))
        .child(div().w(gpui::px(COL_DE)).child(tema::eyebrow("de")))
        .child(div().flex_1().child(tema::eyebrow("texto")))
        .child(div().w(gpui::px(COL_ESTADO)).child(tema::eyebrow("estado")))
        .child(div().w(gpui::px(COL_ENVIADO)).child(tema::eyebrow("enviado")))
        // Columna reservada para la acción rápida de reenvío (trazabilidad-05); label vacío.
        .child(div().w(gpui::px(40.0)))
}

/// Fila de la tabla para un mensaje: usa `tema::fila_seleccionable` (resalte brasa tenue + borde
/// izquierdo cuando activa, hover cuando no). 4 celdas neutras (papel/humo/mono) + la celda
/// "estado" coloreada por dominio. Toda la fila es clicable → despacha `SeleccionarMensaje`.
///
/// Los datos (id/hora) van en fuente mono (timestamps/números); el texto del mensaje en papel;
/// "de" atenuado en humo. La columna "estado" mantiene su color semántico por encima del resalte.
fn fila_mensaje(idx: usize, m: &Mensaje, activa: bool) -> impl IntoElement {
    let (etiqueta, color) = estilo_estado(m.estado);
    // `Mensaje::id` es i64 (Copy): el closure del botón ↻ lo captura sin clonar.
    let msg_id = m.id;

    tema::fila_seleccionable(SharedString::from(format!("traza-fila-{idx}")), activa)
        .child(
            div()
                .w(gpui::px(COL_ID))
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .text_sm()
                .child(SharedString::from(m.id.to_string())),
        )
        .child(
            div()
                .w(gpui::px(COL_DE))
                .text_color(tema::HUMO)
                .text_sm()
                .child(SharedString::from(recortar(&m.de_id, 16))),
        )
        // "texto" es columna flexible: recorte generoso; el layout la envuelve al ancho real.
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_color(tema::PAPEL)
                .child(SharedString::from(recortar(&m.texto, 80))),
        )
        .child(
            div()
                .w(gpui::px(COL_ESTADO))
                .text_color(color)
                .text_sm()
                .child(SharedString::from(etiqueta)),
        )
        .child(
            div()
                .w(gpui::px(COL_ENVIADO))
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .text_sm()
                .child(SharedString::from(hora_iso(&m.enviado_en))),
        )
        // Acción rápida de REENVÍO (trazabilidad-05): botón ↻ al final de la fila que pide la
        // confirmación SIN abrir el modal (espejo de la tecla `r`). `.occlude()` evita que el clic
        // atraviese hacia el `on_click` de la fila (que la seleccionaría).
        .child(
            div()
                .id(gpui::ElementId::Name(format!("traza-reenviar-{idx}").into()))
                .occlude()
                .w(gpui::px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .text_color(tema::HUMO)
                .cursor_pointer()
                .hover(|s| s.bg(tema::TINTA2).text_color(tema::BRASA))
                .child(SharedString::from("↻"))
                .on_click(move |_e, window, cx| {
                    window.dispatch_action(Box::new(PedirReenvio { msg_id }), cx);
                }),
        )
        .on_click(move |e, window, cx| {
            // Doble-click abre el MODAL de timeline (trazabilidad-01); click simple sólo
            // selecciona. `AppDesktop` muta el estado; la vista no toca `cx`.
            if e.click_count() >= 2 {
                window.dispatch_action(Box::new(AbrirMensaje { indice: idx }), cx);
            } else {
                window.dispatch_action(Box::new(SeleccionarMensaje { indice: idx }), cx);
            }
        })
}

/// Contenido del POP-UP "Abrir mensaje" (trazabilidad-01/04): el timeline completo del mensaje en
/// un modal — cabecera `#id` + ruta, CUERPO ÍNTEGRO con wrap+scroll (sin el recorte destructivo de
/// la fila), hitos con los timestamps que timbra el broker, estado actual, contadores, traza de
/// reenvío y acciones (Cerrar / Reenviar→confirmación). Sustituye al antiguo panel inline (que
/// empujaba la tabla); el overlay/backdrop/Esc los aporta `AppDesktop` (`overlay_mensaje`).
///
/// `pub` para que `AppDesktop` lo monte desde su render raíz leyendo `traza_timeline`.
pub fn render_modal_timeline(m: &Mensaje) -> gpui::AnyElement {
    let (etiqueta_estado, color_estado) = estilo_estado(m.estado);
    let msg_id = m.id;

    // Cabecera del modal: eyebrow + #id + ruta, y el ✕ de cierre a la derecha.
    let cabecera = div()
        .h_flex()
        .items_start()
        .justify_between()
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("Timeline del mensaje"))
                .child(tema::titulo(format!("#{}", m.id)).text_size(gpui::px(20.0)))
                .child(
                    div()
                        .font_family(tema::FUENTE_MONO)
                        .text_color(tema::HUMO)
                        .text_sm()
                        .child(SharedString::from(format!("{} → {}", m.de_id, m.para_id))),
                ),
        )
        .child(
            div()
                .id("modal-traza-cerrar-x")
                .flex()
                .items_center()
                .justify_center()
                .w(gpui::px(28.0))
                .h(gpui::px(28.0))
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .text_color(tema::HUMO)
                .cursor_pointer()
                .hover(|s| s.bg(tema::TINTA).text_color(tema::PAPEL))
                .child(SharedString::from("✕"))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(CerrarTimeline), cx);
                }),
        );

    // Cuerpo ÍNTEGRO del mensaje (trazabilidad-04): wrap natural + scroll acotado para que un
    // mensaje kilométrico no desborde el modal. Nada de recortes destructivos aquí.
    let cuerpo = div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow("mensaje"))
        .child(
            div()
                .id("modal-traza-texto-scroll")
                .max_h(gpui::px(200.0))
                .overflow_y_scroll()
                .child(tema::texto_primario(m.texto.clone())),
        );

    // Hitos de la máquina de estados con su timestamp (timbrados por el BROKER, nunca por la IA).
    // `None`/vacío ⇒ aún no alcanzado (se pinta "—" en humo).
    let hitos = div()
        .v_flex()
        .gap_1()
        .pt_1()
        .child(hito("○ enviado", Some(m.enviado_en.as_str()), gpui::rgb(0x9ca3af)))
        .child(hito("◑ entregado", m.entregado_en.as_deref(), gpui::rgb(0xeab308)))
        .child(hito("◑ leído", m.leido_en.as_deref(), gpui::rgb(0xeab308)))
        .child(hito("● procesado", m.procesado_en.as_deref(), gpui::rgb(0x22c55e)));

    // Estado actual coloreado + contadores de intentos/reenvíos (mono, humo).
    let estado_actual = div()
        .h_flex()
        .items_center()
        .gap_2()
        .pt_1()
        .child(tema::texto_terciario("estado actual:"))
        .child(
            div()
                .text_color(color_estado)
                .text_sm()
                .child(SharedString::from(etiqueta_estado)),
        );

    let contadores = div()
        .font_family(tema::FUENTE_MONO)
        .text_color(tema::HUMO)
        .text_sm()
        .child(SharedString::from(format!(
            "intentos: {}  ·  reenvíos: {}",
            m.intentos, m.reenvios
        )));

    // Acciones: Cerrar (secundario) + Reenviar (primario → pide CONFIRMACIÓN, trazabilidad-05).
    let acciones = div()
        .h_flex()
        .gap_3()
        .pt_2()
        .child(
            tema::boton_secundario("modal-traza-cerrar", "Cerrar").on_click(|_e, window, cx| {
                window.dispatch_action(Box::new(CerrarTimeline), cx);
            }),
        )
        .child(
            tema::boton_primario("modal-traza-reenviar", "Reenviar").on_click(
                move |_e, window, cx| {
                    window.dispatch_action(Box::new(PedirReenvio { msg_id }), cx);
                },
            ),
        );

    let mut modal = div()
        .v_flex()
        .w(gpui::px(560.0))
        .gap_2()
        .p_5()
        .rounded(tema::radio(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        .child(cabecera)
        .child(cuerpo)
        .child(hitos)
        .child(estado_actual)
        .child(contadores);

    // Traza de reenvío: sólo si este mensaje es a su vez un reenvío de otro (se marca en humo, dato).
    if let Some(orig) = m.reenviado_de {
        modal = modal.child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .text_sm()
                .child(SharedString::from(format!("reenvío del mensaje #{orig}"))),
        );
    }

    modal.child(acciones).into_any_element()
}

/// Mini-modal de CONFIRMACIÓN de reenvío (trazabilidad-05). `mensaje` es el `Mensaje` original si
/// sigue en la caché (para mostrar contexto: destino y texto recortado); si ya no está, se
/// confirma sólo por id. Confirmar despacha `ConfirmarReenvio` (el `msg_id` pendiente lo guarda
/// `AppDesktop` en `traza_confirmar_reenvio`).
pub fn render_modal_confirmar_reenvio(msg_id: i64, mensaje: Option<&Mensaje>) -> gpui::AnyElement {
    let contexto = match mensaje {
        Some(m) => format!(
            "Se re-encola para «{}» como un mensaje NUEVO: «{}»",
            m.para_id,
            recortar(&m.texto, 70)
        ),
        None => "Se re-encola como un mensaje nuevo en la bandeja del destino original.".to_string(),
    };

    div()
        .v_flex()
        .w(gpui::px(520.0))
        .gap_3()
        .p_5()
        .rounded(tema::radio(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        .child(tema::titulo("Reenviar mensaje").text_size(gpui::px(18.0)))
        .child(tema::texto_primario(format!("¿Reenviar el mensaje #{msg_id}?")))
        .child(tema::texto_terciario(contexto))
        .child(
            div()
                .h_flex()
                .gap_3()
                .pt_2()
                .child(
                    tema::boton_secundario("traza-reenvio-cancelar", "Cancelar").on_click(
                        |_e, window, cx| {
                            window.dispatch_action(Box::new(CancelarReenvio), cx);
                        },
                    ),
                )
                .child(
                    tema::boton_primario("traza-reenvio-confirmar", "Sí, reenviar").on_click(
                        |_e, window, cx| {
                            window.dispatch_action(Box::new(ConfirmarReenvio), cx);
                        },
                    ),
                ),
        )
        .into_any_element()
}

/// Una línea de hito del timeline: etiqueta a la izquierda (ancho fijo para alinear en columna) y
/// timestamp a la derecha en mono, o "—" en humo si el hito aún no ocurrió. Espejo de `hito` de la
/// TUI. El color de la etiqueta alcanzada es el semántico del estado; el no alcanzado va en humo.
fn hito(etiqueta: &str, ts: Option<&str>, color: gpui::Rgba) -> impl IntoElement {
    // Un hito se considera alcanzado si trae timestamp no vacío.
    let alcanzado = ts.map(|t| !t.is_empty()).unwrap_or(false);
    let (color_etiqueta, valor, color_valor) = if alcanzado {
        (color, ts.unwrap_or("").to_string(), tema::PAPEL)
    } else {
        // Humo + "—" cuando la transición todavía no se timbró.
        (tema::HUMO, "—".to_string(), tema::HUMO)
    };

    div()
        .h_flex()
        .items_center()
        .gap_3()
        .text_sm()
        .child(
            div()
                .w(gpui::px(110.0))
                .text_color(color_etiqueta)
                .child(SharedString::from(etiqueta.to_string())),
        )
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(color_valor)
                .child(SharedString::from(valor)),
        )
}
