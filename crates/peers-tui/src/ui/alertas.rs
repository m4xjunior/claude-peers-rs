//! Pantalla 9 — Alertas. Lista de alertas vigentes del supervisor (R6), SOLO LECTURA. Datos de
//! `GET /admin/alertas`. Cada tipo se colorea: ocioso/amarillo, atascado/naranja, ghosteo/rojo.
//!
//! Tabla seleccionable (↑↓ mueven la fila). Render síncrono y sin estado: lee `&App` y pinta.
//! Offline/401 → banner común; aquí se conserva la última lista buena (o un texto guía si vacía).

use crate::app::{color_alerta, etiqueta_alerta, recortar, App};
use peers_core::Alerta;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let titulo = format!(
        " Alertas ({}) · Enter detalle · d descartar · g ir al sujeto ",
        app.datos.alertas.len()
    );

    if app.datos.alertas.is_empty() {
        let p = Paragraph::new(Span::styled(
            "Sin alertas vigentes. El supervisor no ha detectado ocioso/atascado/ghosteo.",
            Style::default().fg(Color::DarkGray),
        ))
        .block(Block::default().borders(Borders::ALL).title(titulo));
        f.render_widget(p, area);
        return;
    }

    let encabezado = Row::new(["tipo", "sujeto", "detalle", "creada"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    // Offset de scroll: mantiene la fila seleccionada dentro del viewport (fix scroll 2026-06-30).
    let offset = crate::ui::offset_scroll(app.seleccion, app.datos.alertas.len(), area.height);
    // Ancho real de la columna flexible `detalle` (col 3, Min). Fijas: 10+18+10=38, 4 columnas.
    let ancho_detalle = crate::ui::ancho_columna_flexible(area.width, 38, 4);
    let filas: Vec<Row> = app
        .datos
        .alertas
        .iter()
        .enumerate()
        .skip(offset) // viewport: solo desde la primera fila visible
        .map(|(idx, a)| fila_render(idx, a, app.seleccion, ancho_detalle))
        .collect();

    let tabla = Table::new(
        filas,
        [
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Min(20),
            Constraint::Length(10),
        ],
    )
    .header(encabezado)
    .block(Block::default().borders(Borders::ALL).title(titulo));

    f.render_widget(tabla, area);

    // Modal DETALLE encima de la tabla (Enter sobre una fila): muestra el detalle COMPLETO,
    // que en la tabla va recortado a 40 chars. Aquí es donde el jefe ve toda la info y decide.
    if let Some(alerta) = &app.alerta_detalle {
        dibujar_detalle(f, area, alerta, app.modal_scroll);
    }
}

/// Modal con el detalle completo de una alerta: tipo, sujeto, texto íntegro y cuándo se creó.
/// La tabla recorta el detalle; este modal NO, para que el jefe lea el mensaje entero del
/// supervisor antes de descartar (d) o saltar al sujeto (g).
fn dibujar_detalle(f: &mut Frame, area: Rect, a: &Alerta, scroll: u16) {
    let modal = centrar(70, 14, area);
    f.render_widget(Clear, modal);

    let color = color_alerta(a.tipo);
    let lineas = vec![
        Line::from(vec![
            Span::styled("tipo:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(etiqueta_alerta(a.tipo), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("sujeto:  ", Style::default().fg(Color::DarkGray)),
            Span::raw(a.sujeto.clone()),
        ]),
        Line::from(vec![
            Span::styled("creada:  ", Style::default().fg(Color::DarkGray)),
            Span::raw(a.creada_en.clone()),
        ]),
        Line::from(""),
        Line::from(Span::styled("detalle:", Style::default().fg(Color::DarkGray))),
        Line::from(Span::raw(a.detalle.clone())),
        Line::from(""),
        Line::from(Span::styled(
            "Enter/Esc cerrar · d descartar · g ir al sujeto",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(lineas)
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .title(" Detalle de alerta "),
        );
    f.render_widget(p, modal);
}

/// Centra un sub-rectángulo de tamaño fijo dentro de `area` (para modales).
fn centrar(ancho: u16, alto: u16, area: Rect) -> Rect {
    let ancho = ancho.min(area.width);
    let alto = alto.min(area.height);
    let x = area.x + (area.width.saturating_sub(ancho)) / 2;
    let y = area.y + (area.height.saturating_sub(alto)) / 2;
    Rect { x, y, width: ancho, height: alto }
}

/// Fila de alerta: la celda de tipo va coloreada según su severidad; el resto neutro. La fila
/// seleccionada se resalta con fondo tenue sin perder el color del tipo.
fn fila_render(idx: usize, a: &Alerta, seleccion: usize, ancho_detalle: usize) -> Row<'static> {
    let color = color_alerta(a.tipo);
    let estilo_fila = if idx == seleccion {
        Style::default().bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(etiqueta_alerta(a.tipo)).style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Cell::from(recortar(&a.sujeto, 18)),
        // detalle: columna flexible → recorte al ancho real de la columna.
        Cell::from(recortar(&a.detalle, ancho_detalle)),
        Cell::from(crate::app::hora_iso(&a.creada_en)),
    ])
    .style(estilo_fila)
}
