//! Pantalla 6 — Trazabilidad. Tabla cronológica del historial durable de un peer
//! (`GET /admin/historial`), con estado COLOREADO por la máquina de estados (R2.4):
//!   ○ enviado/gris · ◑ entregado-leído/amarillo · ● procesado/verde · ✕ fallido-dlq/rojo.
//!
//! Teclas (gestionadas en `main.rs`): ↑↓ mueven la fila, `r` reenvía el mensaje seleccionado
//! (`POST /admin/reenviar`), `Enter` abre el timeline completo del mensaje en un modal.
//!
//! Todo el render es síncrono y sin estado: lee `&App` y pinta. Si el broker está offline o
//! devuelve 401, el banner común (en `ui::mod`) ya lo refleja; aquí simplemente se muestra el
//! último historial bueno cacheado (o un texto guía si aún no hay foco/datos).

use crate::app::{estilo_estado, fila_traza, App};
use peers_core::Mensaje;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let foco = app.traza_peer_actual();

    let titulo = match &foco {
        Some(id) => format!(" Trazabilidad · {} ({} msgs) ", id, app.datos.historial.len()),
        None => " Trazabilidad (sin peer — selecciona uno en pantalla 1) ".to_string(),
    };

    // Sin foco o sin mensajes: tabla vacía con un texto guía, sin crashear.
    if foco.is_none() || app.datos.historial.is_empty() {
        let guia = if foco.is_none() {
            "No hay peers. Ve a la pantalla 1 (Peers) y selecciona uno."
        } else {
            "Este peer aún no tiene mensajes en el historial."
        };
        let p = Paragraph::new(Span::styled(guia, Style::default().fg(Color::DarkGray)))
            .block(Block::default().borders(Borders::ALL).title(titulo));
        f.render_widget(p, area);
        return;
    }

    let encabezado = Row::new(["id", "de", "texto", "estado", "enviado"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    // Offset de scroll: mantiene la fila seleccionada dentro del viewport (fix scroll 2026-06-30).
    let offset = crate::ui::offset_scroll(app.seleccion, app.datos.historial.len(), area.height);
    let filas: Vec<Row> = app
        .datos
        .historial
        .iter()
        .enumerate()
        .skip(offset) // viewport: solo desde la primera fila visible
        .map(|(idx, m)| fila_render(idx, m, app.seleccion))
        .collect();

    let tabla = Table::new(
        filas,
        [
            Constraint::Length(8),  // id
            Constraint::Length(14), // de
            Constraint::Min(20),    // texto
            Constraint::Length(14), // estado
            Constraint::Length(10), // enviado (HH:MM:SS)
        ],
    )
    .header(encabezado)
    .block(Block::default().borders(Borders::ALL).title(titulo));

    f.render_widget(tabla, area);

    // Modal de timeline completo del mensaje seleccionado (Enter).
    if app.traza_timeline {
        if let Some(m) = app.traza_mensaje_seleccionado() {
            dibujar_timeline(f, area, m);
        }
    }
}

/// Construye la fila de la tabla: 4 celdas neutras (id/de/texto/enviado) + la celda de estado
/// con su color. La fila seleccionada se resalta con fondo cian (sin perder el color del estado
/// en su celda, que se mantiene legible sobre el resalte de la fila no-seleccionada).
fn fila_render(idx: usize, m: &Mensaje, seleccion: usize) -> Row<'static> {
    let [id, de, texto, enviado] = fila_traza(m);
    let (etiqueta, color) = estilo_estado(m.estado);

    let resaltada = idx == seleccion;
    let estilo_fila = if resaltada {
        Style::default().bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let celda_estado = Cell::from(etiqueta).style(Style::default().fg(color));

    Row::new(vec![
        Cell::from(id),
        Cell::from(de),
        Cell::from(texto),
        celda_estado,
        Cell::from(enviado),
    ])
    .style(estilo_fila)
}

/// Modal centrado con el timeline completo de un mensaje: cada transición con su timestamp
/// timbrado por el broker, más la traza de reenvío. Solo lectura.
fn dibujar_timeline(f: &mut Frame, area: Rect, m: &Mensaje) {
    let modal = centrar(64, 16, area);
    f.render_widget(Clear, modal);

    let mut lineas: Vec<Line> = Vec::with_capacity(12);
    lineas.push(Line::from(Span::styled(
        format!("mensaje #{}  ({} → {})", m.id, m.de_id, m.para_id),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lineas.push(Line::from(Span::raw(m.texto.clone())));
    lineas.push(Line::from(""));

    // Hitos de la máquina de estados con su timestamp (None = aún no alcanzado).
    lineas.push(hito("○ enviado", Some(&m.enviado_en), Color::Gray));
    lineas.push(hito("◑ entregado", m.entregado_en.as_deref(), Color::Yellow));
    lineas.push(hito("◑ leído", m.leido_en.as_deref(), Color::Yellow));
    lineas.push(hito("● procesado", m.procesado_en.as_deref(), Color::Green));

    lineas.push(Line::from(""));
    let (etiqueta, color) = estilo_estado(m.estado);
    lineas.push(Line::from(vec![
        Span::raw("estado actual: "),
        Span::styled(etiqueta, Style::default().fg(color).add_modifier(Modifier::BOLD)),
    ]));
    lineas.push(Line::from(Span::raw(format!(
        "intentos: {}  ·  reenvíos: {}",
        m.intentos, m.reenvios
    ))));
    if let Some(orig) = m.reenviado_de {
        lineas.push(Line::from(Span::styled(
            format!("reenvío del mensaje #{orig}"),
            Style::default().fg(Color::Magenta),
        )));
    }
    lineas.push(Line::from(""));
    lineas.push(Line::from(Span::styled(
        "Esc/Enter cierra · r reenvía",
        Style::default().fg(Color::DarkGray),
    )));

    let bloque = Block::default()
        .borders(Borders::ALL)
        .title(" timeline ")
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(Paragraph::new(lineas).block(bloque), modal);
}

/// Una línea de hito del timeline: etiqueta + timestamp, o "—" en gris si aún no ocurrió.
fn hito(etiqueta: &str, ts: Option<&str>, color: Color) -> Line<'static> {
    match ts {
        Some(t) if !t.is_empty() => Line::from(vec![
            Span::styled(format!("{etiqueta:<14}"), Style::default().fg(color)),
            Span::raw(t.to_string()),
        ]),
        _ => Line::from(vec![
            Span::styled(format!("{etiqueta:<14}"), Style::default().fg(Color::DarkGray)),
            Span::styled("—", Style::default().fg(Color::DarkGray)),
        ]),
    }
}

/// Rectángulo centrado de `ancho` × `alto` dentro de `area` (idéntico al de `ui::mod`,
/// duplicado aquí para no exponer un helper privado entre módulos).
fn centrar(ancho: u16, alto: u16, area: Rect) -> Rect {
    let ancho = ancho.min(area.width);
    let alto = alto.min(area.height);
    let x = area.x + (area.width.saturating_sub(ancho)) / 2;
    let y = area.y + (area.height.saturating_sub(alto)) / 2;
    Rect { x, y, width: ancho, height: alto }
}
