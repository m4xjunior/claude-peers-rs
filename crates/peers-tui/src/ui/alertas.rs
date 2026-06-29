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
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let titulo = format!(" Alertas ({}) ", app.datos.alertas.len());

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

    let filas: Vec<Row> = app
        .datos
        .alertas
        .iter()
        .enumerate()
        .map(|(idx, a)| fila_render(idx, a, app.seleccion))
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
}

/// Fila de alerta: la celda de tipo va coloreada según su severidad; el resto neutro. La fila
/// seleccionada se resalta con fondo tenue sin perder el color del tipo.
fn fila_render(idx: usize, a: &Alerta, seleccion: usize) -> Row<'static> {
    let color = color_alerta(a.tipo);
    let estilo_fila = if idx == seleccion {
        Style::default().bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(etiqueta_alerta(a.tipo)).style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Cell::from(recortar(&a.sujeto, 18)),
        Cell::from(recortar(&a.detalle, 40)),
        Cell::from(crate::app::hora_iso(&a.creada_en)),
    ])
    .style(estilo_fila)
}
