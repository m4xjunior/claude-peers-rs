//! Pantalla Trazabilidad — historial durable de la cola de un peer (R2.3/R2.4), portada de la
//! TUI (`peers-tui/src/ui/trazabilidad.rs`) a GPUI.
//!
//! QUÉ MUESTRA (espejo 1:1 de la TUI):
//!   1. Cabecera con el peer en foco y el nº de mensajes de su historial.
//!   2. Tabla cronológica: `id · de · texto · estado · enviado`, con la columna "estado"
//!      COLOREADA por la máquina de estados (R2.4): ○ enviado/gris · ◑ entregado-leído/amarillo
//!      · ● procesado/verde · ✕ fallido-dlq/rojo. La fila seleccionada se resalta.
//!   3. Timeline del mensaje seleccionado (cuando `traza_timeline` está activo): cada hito de la
//!      máquina con su timestamp timbrado por el broker, más intentos/reenvíos. En la TUI es un
//!      modal (`Enter`); aquí se despliega INLINE bajo la tabla — decisión de Fundación: el
//!      `Dialog` del kit es stateful (exige `Entity`/`cx`) y la firma del stub es síncrona
//!      (`&EstadoPantalla`); un panel inline da la misma información sin acoplar la firma al
//!      ciclo de vida de un componente con estado. Migrable a `Dialog` cuando se fije la revisión.
//!
//! DEGRADACIÓN: sin peer en foco o sin historial, se pinta un texto guía (nunca crashea). Todo el
//! render es síncrono y sin estado: lee `&EstadoPantalla` y pinta; la recarga la dispara la app
//! con `cx.spawn` sobre `ClienteBroker::historial` (ver `cliente.rs`).

use gpui::{div, IntoElement, ParentElement, SharedString, Styled};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;
use peers_core::{EstadoMensaje, Mensaje};

use crate::app::EstadoPantalla;

/// Paleta de la columna "estado", espejo exacto de `peers_tui::app::estilo_estado` (R2.4).
/// Se replica aquí (no se acopla al crate de la TUI) porque el contrato es trivial y estable:
/// símbolo+etiqueta + color RGBA para GPUI. Devuelve el color como `Rgba` de gpui para pintarlo
/// directamente con `.text_color(..)`.
///
/// INTENCIÓN: que un solo sitio decida etiqueta y color de cada estado, igual que en la TUI, para
/// que ambas interfaces muestren la misma semántica visual del ciclo de vida del mensaje.
fn estilo_estado(estado: EstadoMensaje) -> (&'static str, gpui::Rgba) {
    match estado {
        // Gris: encolado, aún no confirmado por el destino.
        EstadoMensaje::Enviado => ("○ enviado", gpui::rgb(0x9ca3af)),
        // Amarillo: en tránsito de entrega/lectura (dos subestados, mismo color como en la TUI).
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

/// Punto de entrada de la pantalla (firma estable de Fundación). Ramifica: guía si no hay datos,
/// tabla + (timeline opcional) si los hay.
pub fn render_trazabilidad(datos: &EstadoPantalla) -> impl IntoElement {
    let foco = datos.traza_peer.as_deref();

    // Cabecera: peer en foco + nº de mensajes, o aviso de que no hay foco.
    let titulo = match foco {
        Some(id) => format!("Trazabilidad · {} ({} msgs)", id, datos.historial.len()),
        None => "Trazabilidad (sin peer — selecciónalo en la pantalla Peers)".to_string(),
    };

    let cabecera = div().text_xl().child(SharedString::from(titulo));

    // Sin foco o sin mensajes: texto guía, sin construir tabla (evita render vacío confuso).
    if foco.is_none() || datos.historial.is_empty() {
        let guia = if foco.is_none() {
            "No hay peer seleccionado. Ve a la pantalla Peers y elige uno."
        } else {
            "Este peer aún no tiene mensajes en el historial."
        };
        return div()
            .v_flex()
            .size_full()
            .gap_3()
            .p_6()
            .child(cabecera)
            .child(
                div()
                    .text_sm()
                    .text_color(gpui::rgb(0x9ca3af))
                    .child(SharedString::from(guia)),
            );
    }

    // Tabla: encabezado + una fila por mensaje. La fila seleccionada se resalta con fondo.
    let sel = datos.traza_seleccion.min(datos.historial.len().saturating_sub(1));
    let filas = datos
        .historial
        .iter()
        .enumerate()
        .map(|(idx, m)| fila_mensaje(m, idx == sel))
        .collect::<Vec<_>>();

    let tabla = div()
        .v_flex()
        .size_full()
        .child(encabezado_tabla())
        .child(div().v_flex().children(filas));

    // Panel de timeline del mensaje seleccionado (equivalente inline al modal `Enter` de la TUI).
    let timeline = if datos.traza_timeline {
        datos.historial.get(sel).map(panel_timeline)
    } else {
        None
    };

    let mut columna = div()
        .v_flex()
        .size_full()
        .gap_3()
        .p_6()
        .child(cabecera)
        .child(tabla);
    if let Some(panel) = timeline {
        columna = columna.child(panel);
    }
    columna
}

/// Fila de encabezado de la tabla, con los mismos rótulos y proporciones de columna que la TUI.
/// Los anchos fijos (id/de/estado/enviado) usan `w(px)`; "texto" es la columna flexible (`flex_1`).
fn encabezado_tabla() -> impl IntoElement {
    let estilo_col = gpui::rgb(0xeab308); // amarillo, como el header de la tabla en la TUI
    div()
        .h_flex()
        .w_full()
        .gap_2()
        .px_2()
        .py_1()
        .text_sm()
        .text_color(estilo_col)
        .child(div().w(gpui::px(72.0)).child(SharedString::from("id")))
        .child(div().w(gpui::px(120.0)).child(SharedString::from("de")))
        .child(div().flex_1().child(SharedString::from("texto")))
        .child(div().w(gpui::px(120.0)).child(SharedString::from("estado")))
        .child(div().w(gpui::px(90.0)).child(SharedString::from("enviado")))
}

/// Construye una fila de la tabla para un mensaje: 4 celdas neutras + la celda "estado" coloreada.
/// La fila seleccionada lleva un fondo tenue (espejo del resalte cian de la TUI) sin perder el
/// color del estado, que se mantiene legible por encima del fondo.
fn fila_mensaje(m: &Mensaje, resaltada: bool) -> impl IntoElement {
    let (etiqueta, color) = estilo_estado(m.estado);

    let mut fila = div()
        .h_flex()
        .w_full()
        .gap_2()
        .px_2()
        .py_1()
        .text_sm()
        .rounded_sm()
        .child(div().w(gpui::px(72.0)).child(SharedString::from(m.id.to_string())))
        .child(div().w(gpui::px(120.0)).child(SharedString::from(recortar(&m.de_id, 14))))
        // "texto" es columna flexible: recorte generoso; el layout la envuelve al ancho real.
        .child(div().flex_1().child(SharedString::from(recortar(&m.texto, 80))))
        .child(
            div()
                .w(gpui::px(120.0))
                .text_color(color)
                .child(SharedString::from(etiqueta)),
        )
        .child(div().w(gpui::px(90.0)).child(SharedString::from(hora_iso(&m.enviado_en))));

    if resaltada {
        // Fondo tenue azulado para la fila activa (equivalente al bg Rgb(40,40,60) de la TUI).
        fila = fila.bg(gpui::rgba(0x28283c80));
    }
    fila
}

/// Panel de timeline del mensaje seleccionado: cada hito de la máquina de estados con su
/// timestamp (o "—" en gris si aún no se alcanzó), más el estado actual e intentos/reenvíos.
/// Sólo lectura. Réplica del contenido del modal `dibujar_timeline` de la TUI.
fn panel_timeline(m: &Mensaje) -> impl IntoElement {
    let (etiqueta_estado, color_estado) = estilo_estado(m.estado);

    // Cabecera del panel: identificador y ruta del mensaje.
    let cabecera = div()
        .text_lg()
        .text_color(gpui::rgb(0x38bdf8)) // cian, como el título del modal en la TUI
        .child(SharedString::from(format!(
            "mensaje #{}  ({} → {})",
            m.id, m.de_id, m.para_id
        )));

    // Cuerpo completo del mensaje (sin recortar: aquí sí cabe el texto entero).
    let cuerpo = div()
        .text_sm()
        .child(SharedString::from(m.texto.clone()));

    // Hitos de la máquina de estados con su timestamp. `None`/vacío ⇒ aún no alcanzado.
    let hitos = div()
        .v_flex()
        .gap_1()
        .child(hito("○ enviado", Some(m.enviado_en.as_str()), gpui::rgb(0x9ca3af)))
        .child(hito("◑ entregado", m.entregado_en.as_deref(), gpui::rgb(0xeab308)))
        .child(hito("◑ leído", m.leido_en.as_deref(), gpui::rgb(0xeab308)))
        .child(hito("● procesado", m.procesado_en.as_deref(), gpui::rgb(0x22c55e)));

    // Estado actual coloreado + contadores de intentos/reenvíos.
    let estado_actual = div()
        .h_flex()
        .gap_2()
        .text_sm()
        .child(SharedString::from("estado actual:"))
        .child(
            div()
                .text_color(color_estado)
                .child(SharedString::from(etiqueta_estado)),
        );

    let contadores = div()
        .text_sm()
        .text_color(gpui::rgb(0x9ca3af))
        .child(SharedString::from(format!(
            "intentos: {}  ·  reenvíos: {}",
            m.intentos, m.reenvios
        )));

    let mut panel = div()
        .v_flex()
        .w_full()
        .gap_2()
        .p_4()
        .rounded_md()
        .bg(gpui::rgba(0x0000004d))
        // Borde cian sutil, como el `border_style` del modal de la TUI.
        .border_1()
        .border_color(gpui::rgb(0x38bdf8))
        .child(cabecera)
        .child(cuerpo)
        .child(hitos)
        .child(estado_actual)
        .child(contadores);

    // Traza de reenvío: sólo si este mensaje es a su vez un reenvío de otro (magenta como la TUI).
    if let Some(orig) = m.reenviado_de {
        panel = panel.child(
            div()
                .text_sm()
                .text_color(gpui::rgb(0xd946ef))
                .child(SharedString::from(format!("reenvío del mensaje #{orig}"))),
        );
    }

    panel
}

/// Una línea de hito del timeline: etiqueta a la izquierda (ancho fijo para alinear en columna) y
/// timestamp a la derecha, o "—" en gris si el hito aún no ocurrió. Espejo de `hito` de la TUI.
fn hito(etiqueta: &str, ts: Option<&str>, color: gpui::Rgba) -> impl IntoElement {
    // Un hito se considera alcanzado si trae timestamp no vacío.
    let alcanzado = ts.map(|t| !t.is_empty()).unwrap_or(false);
    let (color_etiqueta, valor) = if alcanzado {
        (color, ts.unwrap_or("").to_string())
    } else {
        // Gris apagado + "—" cuando la transición todavía no se timbró.
        (gpui::rgb(0x6b7280), "—".to_string())
    };

    div()
        .h_flex()
        .gap_3()
        .text_sm()
        .child(
            div()
                .w(gpui::px(110.0))
                .text_color(color_etiqueta)
                .child(SharedString::from(etiqueta.to_string())),
        )
        .child(div().child(SharedString::from(valor)))
}
