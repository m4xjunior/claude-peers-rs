//! Pantalla Chat privado (RFC-lanzador §7/R8): canal CONFIDENCIAL de Max con un peer.
//!
//! Es un canal PARALELO al `<channel>` (que se renderiza en el TUI del peer): aquí Max escribe a
//! ESE Claude y ve sus respuestas SIN que el intercambio aparezca en el pane del terminal del peer.
//! El transporte lo dan las tools MCP `chat_privado_recibir`/`responder` (pull); esta pantalla es
//! el lado del OPERADOR: escribe con `/chat-privado/enviar` (cola de Entrada del peer) y lee las
//! respuestas con `/chat-privado/leer` (cola de Salida del peer).
//!
//! v1 SIMPLE (decisión de Max): el backend NO persiste el hilo (drenar-al-leer). Por eso el hilo se
//! acumula EN MEMORIA en `datos.chat_privado_hilo`: lo que Max escribe se añade al enviar; las
//! respuestas se añaden al drenar. Se pierde al cerrar la app / cambiar de peer (hilo durable = v2).
//!
//! Vista PURA (sin `cx`): despacha acciones que `AppDesktop` maneja con `.on_action`, igual que el
//! resto de pantallas. El `Input` del composer es un `Entity<InputState>` creado por la app.

use gpui::{
    div, prelude::FluentBuilder, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
use gpui::Action;
use gpui_component::{input::Input, StyledExt};

use peers_core::{Instancia, MensajeChatPrivado, ID_OPERADOR};

use crate::app::EstadoPantalla;
use crate::tema;

// --- Acciones (namespace chat_privado) — la vista pura las despacha; AppDesktop las maneja ---

/// Elegir con qué peer chatea Max en privado. `AppDesktop` fija `chat_privado_peer = Some(id)`,
/// LIMPIA el hilo en memoria (es de otro peer) y dispara una lectura inicial de la cola de salida.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = chat_privado, no_json)]
pub struct SeleccionarChatPeer {
    pub id: String,
}

/// Enviar el texto del composer al peer seleccionado. `AppDesktop` lee el `InputState`, hace
/// `POST /chat-privado/enviar`, añade el mensaje al hilo local (burbuja de Max) y limpia el input.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = chat_privado, no_json)]
pub struct EnviarChatPrivado;

/// Refrescar: drena la cola de SALIDA del peer (`/chat-privado/leer`) y añade lo nuevo al hilo.
/// (El refresco periódico de la app también lo hace; este botón es el disparo manual.)
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = chat_privado, no_json)]
pub struct RefrescarChatPrivado;

/// Elegir el remitente APARENTE del mensaje (feature "aparentar ser", RFC-lanzador §7). `de` vacío
/// ("") = operador (default); un id de peer = el mensaje aparentará venir de ESE peer. `AppDesktop`
/// fija `chat_privado_de` (None si "", Some(id) si no).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = chat_privado, no_json)]
pub struct ElegirChatDe {
    /// id del peer a aparentar, o "" para volver a operador.
    pub de: String,
}

/// Render de la pantalla. Selector de peers arriba, hilo en el medio (scroll), composer abajo.
pub fn render_chat_privado(datos: &EstadoPantalla) -> impl IntoElement {
    tema::raiz_scrollable()
        .v_flex()
        .p_8()
        .gap_4()
        .child(tema::eyebrow("Chat privado"))
        .child(tema::texto_terciario(
            "Canal confidencial con un peer. No aparece en el terminal del peer; él lo lee cuando \
             consulta su chat privado. El hilo vive en memoria (v1).",
        ))
        .child(selector_peers(datos))
        .child(hilo(datos))
        .child(composer(datos))
        .when_some(datos.chat_privado_error.as_ref(), |el, err| {
            el.child(
                tema::texto_terciario(format!("Error del chat privado: {err}"))
                    .text_color(tema::SALMO),
            )
        })
}

/// Fila de chips con los peers vivos; el activo se resalta. Clic → `SeleccionarChatPeer`.
fn selector_peers(datos: &EstadoPantalla) -> impl IntoElement {
    let activo = datos.chat_privado_peer.clone();
    let mut fila = div().h_flex().gap_2().flex_wrap();
    if datos.instancias.is_empty() {
        return div().child(tema::texto_terciario("No hay peers vivos con quien chatear."));
    }
    for inst in &datos.instancias {
        let es_activo = activo.as_deref() == Some(inst.id.as_str());
        fila = fila.child(chip_peer(inst, es_activo));
    }
    // Encabezado con el eyebrow y, si hay peer activo, un botón para refrescar sus respuestas.
    let mut encabezado = div().h_flex().items_center().justify_between().child(tema::eyebrow("peer"));
    if activo.is_some() {
        encabezado = encabezado.child(
            tema::boton_secundario("chat-privado-refrescar", "Refrescar")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(RefrescarChatPrivado), cx)),
        );
    }
    div().v_flex().gap_1().child(encabezado).child(fila)
}

/// Un chip seleccionable por peer. Reusa `fila_seleccionable` del tema para el estado activo.
fn chip_peer(inst: &Instancia, activo: bool) -> impl IntoElement {
    let id = inst.id.clone();
    tema::fila_seleccionable(SharedString::from(format!("chatpeer-{id}")), activo)
        .px_3()
        .py_1()
        .rounded(tema::radio(tema::RADIO_PILL))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(if activo { tema::BRASA } else { tema::PAPEL })
                .child(SharedString::from(id.clone())),
        )
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(SeleccionarChatPeer { id: id.clone() }), cx)
        })
}

/// El hilo de mensajes en burbujas. Max (operador) a la derecha (BRASA-tenue), el peer a la
/// izquierda (TINTA2). Vacío → aviso. Sin peer seleccionado → invitación a elegir uno.
fn hilo(datos: &EstadoPantalla) -> impl IntoElement {
    let mut cont = tema::superficie_card().v_flex().gap_2().p_4().min_h(tema::radio(240.0));
    if datos.chat_privado_peer.is_none() {
        return cont.child(tema::texto_terciario("Elige un peer arriba para empezar a chatear."));
    }
    if datos.chat_privado_hilo.is_empty() {
        return cont.child(tema::texto_terciario(
            "Sin mensajes todavía. Escribe abajo para iniciar la conversación privada.",
        ));
    }
    let mut lista = div()
        .id("chat-hilo-scroll")
        .v_flex()
        .gap_2()
        .max_h(tema::radio(400.0))
        .overflow_y_scroll();
    for m in &datos.chat_privado_hilo {
        lista = lista.child(burbuja(m));
    }
    cont.child(lista)
}

/// Una burbuja de mensaje. `de == operador` → derecha, BRASA-tenue; si no → izquierda, TINTA2.
fn burbuja(m: &MensajeChatPrivado) -> impl IntoElement {
    let es_operador = es_de_operador(&m.de);
    let (color_fondo, alinear) = if es_operador {
        (tema::BRASA_TENUE, gpui::Length::from(gpui::relative(1.0))) // empuja a la derecha
    } else {
        (tema::TINTA2, gpui::Length::from(gpui::px(0.0)))
    };
    let etiqueta = if es_operador { "Tú (operador)" } else { m.de.as_str() };
    let burbuja = div()
        .max_w(gpui::relative(0.75))
        .px_3()
        .py_2()
        .rounded(tema::radio(tema::RADIO_CARD))
        .bg(color_fondo)
        .v_flex()
        .gap_1()
        .child(
            div()
                .text_color(tema::HUMO)
                .child(SharedString::from(format!("{etiqueta} · {}", tema::formatear_fecha(&m.enviado_en)))),
        )
        .child(tema::texto_primario(m.texto.clone()));
    // Alineación horizontal via margen izquierdo flexible (right) o cero (left).
    div().w_full().h_flex().child(div().w(alinear)).child(burbuja)
}

/// Composer: el `Input` del kit + botón Enviar. Deshabilitado si no hay peer o falta el input.
fn composer(datos: &EstadoPantalla) -> impl IntoElement {
    let hay_peer = datos.chat_privado_peer.is_some();
    let mut fila = div().h_flex().gap_2().items_center();
    if let Some(input) = &datos.input_chat_privado {
        fila = fila.child(
            div()
                .flex_1()
                .px_3()
                .py_1()
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .bg(tema::TINTA)
                .border_1()
                .border_color(tema::LINEA)
                .child(Input::new(input).cleanable(true)),
        );
    } else {
        fila = fila.child(tema::texto_terciario("Composer no inicializado; reinicia la app."));
    }
    if hay_peer {
        fila = fila.child(
            tema::boton_primario("chat-privado-enviar", "Enviar")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(EnviarChatPrivado), cx)),
        );
    } else {
        fila = fila.child(tema::texto_terciario("(elige un peer)"));
    }
    div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow("mensaje privado"))
        .when(hay_peer, |el| el.child(selector_de(datos)))
        .child(fila)
}

/// Selector "aparentar ser" (feature RFC-lanzador §7): chips con "operador" (default) + cada peer
/// vivo. El activo se resalta. Elegir un peer hace que el mensaje aparente venir de ESE id; "operador"
/// vuelve al default. NOTA de seguridad (opción C, decisión de Max): el `de` no exige credencial
/// exclusiva de operador — se acepta bajo el modelo de red interna de confianza (ver el broker).
fn selector_de(datos: &EstadoPantalla) -> impl IntoElement {
    let actual = datos.chat_privado_de.as_deref(); // None = operador
    // `flex_wrap` para que los chips envuelvan; `max_h` + scroll (con `.id()`) para que con muchos
    // peers vivos el selector no crezca sin límite y empuje el composer fuera de la vista (hallazgo UI).
    let mut fila = div()
        .id("chat-selector-de")
        .h_flex()
        .gap_2()
        .flex_wrap()
        .items_center()
        .max_h(tema::radio(120.0))
        .overflow_y_scroll();
    fila = fila.child(tema::texto_terciario("aparentar ser:"));
    // Chip "operador" (default): activo cuando no hay `de` fijado.
    fila = fila.child(chip_de("", "operador", actual.is_none()));
    // Un chip por peer vivo (no incluimos al DESTINO, aparentar ser el propio destinatario no tiene
    // sentido, pero lo dejamos por simplicidad: el usuario elige; el destino se sabe por otro selector).
    for inst in &datos.instancias {
        let activo = actual == Some(inst.id.as_str());
        fila = fila.child(chip_de(&inst.id, &inst.id, activo));
    }
    fila
}

/// Chip del selector "aparentar ser". `valor` es el id a fijar ("" para volver a operador),
/// `etiqueta` el texto mostrado, `activo` si es el remitente aparente actual.
fn chip_de(valor: &str, etiqueta: &str, activo: bool) -> impl IntoElement {
    let valor = valor.to_string();
    tema::fila_seleccionable(SharedString::from(format!("chatde-{etiqueta}")), activo)
        .px_3()
        .py_1()
        .rounded(tema::radio(tema::RADIO_PILL))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(if activo { tema::BRASA } else { tema::HUMO })
                .child(SharedString::from(etiqueta.to_string())),
        )
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(ElegirChatDe { de: valor.clone() }), cx)
        })
}

/// ¿La burbuja es de Max (operador)? Lógica pura extraída para testearla sin render GPUI (que
/// exige `Window`). La vista la usa para decidir lado (derecha/izquierda) y color de la burbuja.
fn es_de_operador(de: &str) -> bool {
    de == ID_OPERADOR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Pantalla;

    /// La pantalla Chat privado está cableada en el enum con título e id propios (guardarraíl del
    /// registro: si alguien la quita del array o duplica un id, esto lo detecta).
    #[test]
    fn pantalla_chat_privado_registrada() {
        assert!(Pantalla::TODAS.contains(&Pantalla::ChatPrivado));
        assert_eq!(Pantalla::ChatPrivado.titulo(), "Chat privado");
        // Ids de navegación únicos entre todas las pantallas.
        let ids: Vec<_> = Pantalla::TODAS.iter().map(|p| p.titulo()).collect();
        let unicos: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unicos.len(), "títulos de pantalla duplicados");
    }

    /// La burbuja distingue al operador (derecha, BRASA-tenue) de un peer (izquierda). Es la única
    /// decisión de negocio de la vista; el resto es layout.
    #[test]
    fn burbuja_identifica_al_operador() {
        assert!(es_de_operador(ID_OPERADOR));
        assert!(!es_de_operador("backend@proyecto"));
        assert!(!es_de_operador("broker"));
    }
}

