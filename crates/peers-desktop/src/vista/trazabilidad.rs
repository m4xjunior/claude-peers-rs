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

/// Seleccionar la fila `indice` de la tabla y ABRIR su timeline inline (click/Enter sobre la fila).
/// `AppDesktop` fija `traza_seleccion = indice` y `traza_timeline = true`. Índice sobre `historial`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct SeleccionarMensaje {
    /// Índice de la fila en `historial` (0-based, orden cronológico ascendente del broker).
    pub indice: usize,
}

/// Reenviar el mensaje `msg_id` → `ClienteBroker::reenviar(msg_id)` y luego recargar el historial.
/// `msg_id` es el `Mensaje::id` (i64), lo que el endpoint `POST /admin/reenviar` espera. Se despacha
/// desde el botón "Reenviar" del panel de timeline.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = trazabilidad, no_json)]
pub struct ReenviarMensaje {
    /// Id durable del mensaje a reenviar (`Mensaje::id`).
    pub msg_id: i64,
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

    // Sin foco o sin mensajes: cabecera + selector + texto guía (nunca render vacío confuso).
    if foco.is_none() || datos.historial.is_empty() {
        let guia = if foco.is_none() {
            "No hay peer en foco. Elige un peer arriba (o selecciónalo en la pantalla Peers)."
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

    // Panel de timeline del mensaje seleccionado (equivalente inline al modal `Enter` de la TUI).
    let timeline = if datos.traza_timeline {
        datos.historial.get(sel).map(panel_timeline)
    } else {
        None
    };

    let mut columna = div()
        .v_flex()
        .size_full()
        .gap_4()
        .p_6()
        .child(cabecera);
    if let Some(sel_ui) = selector {
        columna = columna.child(sel_ui);
    }
    columna = columna.child(tabla);
    if let Some(panel) = timeline {
        columna = columna.child(panel);
    }
    columna
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
}

/// Fila de la tabla para un mensaje: usa `tema::fila_seleccionable` (resalte brasa tenue + borde
/// izquierdo cuando activa, hover cuando no). 4 celdas neutras (papel/humo/mono) + la celda
/// "estado" coloreada por dominio. Toda la fila es clicable → despacha `SeleccionarMensaje`.
///
/// Los datos (id/hora) van en fuente mono (timestamps/números); el texto del mensaje en papel;
/// "de" atenuado en humo. La columna "estado" mantiene su color semántico por encima del resalte.
fn fila_mensaje(idx: usize, m: &Mensaje, activa: bool) -> impl IntoElement {
    let (etiqueta, color) = estilo_estado(m.estado);

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
        .on_click(move |_e, window, cx| {
            // Selecciona la fila y abre su timeline; `AppDesktop` muta el estado. La vista no toca `cx`.
            window.dispatch_action(Box::new(SeleccionarMensaje { indice: idx }), cx);
        })
        // Accesibilidad-teclado: el foco de la fila permite abrir el timeline con Enter/Espacio,
        // que GPUI traduce a `on_click` en elementos con `.id()` (paridad con la tecla Enter de la TUI).
        // (El resalte del foco lo aporta el hover del tema; no añadimos ring para no romper el look.)
}

/// Panel de timeline del mensaje seleccionado: cada hito de la máquina de estados con su timestamp
/// (o "—" en humo si aún no se alcanzó), estado actual, contadores de intentos/reenvíos y las
/// acciones (Reenviar / Cerrar). Réplica del contenido del modal `dibujar_timeline` de la TUI,
/// vestido con el tema Ethos (superficie card, borde brasa sutil, tipografía).
fn panel_timeline(m: &Mensaje) -> impl IntoElement {
    let (etiqueta_estado, color_estado) = estilo_estado(m.estado);
    let msg_id = m.id;

    // Cabecera del panel: eyebrow + identificador y ruta del mensaje.
    let cabecera = div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow("Timeline del mensaje"))
        .child(
            tema::titulo(format!("#{}", m.id))
                .text_size(gpui::px(20.0)),
        )
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .text_sm()
                .child(SharedString::from(format!("{} → {}", m.de_id, m.para_id))),
        );

    // Cuerpo completo del mensaje (sin recortar: aquí sí cabe el texto entero).
    let cuerpo = div()
        .w_full()
        .text_color(tema::PAPEL)
        .child(SharedString::from(m.texto.clone()));

    // Hitos de la máquina de estados con su timestamp. `None`/vacío ⇒ aún no alcanzado.
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

    // Acción: Reenviar (primario, brasa) — despacha `ReenviarMensaje` con el id del mensaje activo.
    // No hay botón "Cerrar" porque el timeline se cierra/alterna re-clicando la fila (paridad con la
    // tecla Enter de la TUI, que abre/cierra el mismo modal). Si Fase 3 quiere un cierre explícito,
    // basta declarar una acción sin datos `CerrarTimeline` como en Alertas (`CerrarDetalleAlerta`).
    let acciones = div()
        .h_flex()
        .gap_2()
        .pt_2()
        .child(
            // `msg_id` es i64 (Copy): el closure `Fn` puede invocarse varias veces sin clonar.
            tema::boton_primario("traza-reenviar", "Reenviar").on_click(move |_e, window, cx| {
                window.dispatch_action(Box::new(ReenviarMensaje { msg_id }), cx);
            }),
        );

    let mut panel = tema::superficie_card()
        .v_flex()
        .w_full()
        .gap_2()
        .p_4()
        // Borde brasa sutil para marcar que este panel pertenece a la fila activa (acento moderado).
        .border_color(tema::BRASA)
        .child(cabecera)
        .child(cuerpo)
        .child(hitos)
        .child(estado_actual)
        .child(contadores);

    // Traza de reenvío: sólo si este mensaje es a su vez un reenvío de otro (se marca en humo, dato).
    if let Some(orig) = m.reenviado_de {
        panel = panel.child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .text_sm()
                .child(SharedString::from(format!("reenvío del mensaje #{orig}"))),
        );
    }

    panel.child(acciones)
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
