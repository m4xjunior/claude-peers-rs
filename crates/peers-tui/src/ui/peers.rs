//! Pantalla 1 — Peers. Tabla viva de instancias (id, dir, resumen, visto) desde POST /listar.

use crate::app::{fila_peer, App};
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let encabezado = Row::new(["id", "directorio", "resumen", "visto"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    // Offset de scroll: mantiene la fila seleccionada dentro del viewport (fix scroll 2026-06-30).
    let offset = crate::ui::offset_scroll(app.seleccion, app.datos.peers.len(), area.height);
    // Ancho real de la columna flexible `resumen` (col 3, Min). Fijas: 16+34+10=60, 4 columnas.
    let ancho_resumen = crate::ui::ancho_columna_flexible(area.width, 60, 4);
    let filas: Vec<Row> = app
        .datos
        .peers
        .iter()
        .enumerate()
        .skip(offset) // viewport: solo desde la primera fila visible
        .map(|(idx, inst)| {
            let mut celdas = fila_peer(inst);
            // resumen (celda 2) viene sin recortar: lo ajustamos al ancho real de su columna.
            celdas[2] = crate::app::recortar(&celdas[2], ancho_resumen);
            let estilo = if idx == app.seleccion {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(celdas.into_iter().map(Cell::from).collect::<Vec<_>>()).style(estilo)
        })
        .collect();

    let titulo = format!(" Peers ({}) ", app.datos.peers.len());
    let tabla = Table::new(
        filas,
        [
            Constraint::Length(16),
            Constraint::Length(34),
            Constraint::Min(20),
            Constraint::Length(10),
        ],
    )
    .header(encabezado)
    .block(Block::default().borders(Borders::ALL).title(titulo));

    f.render_widget(tabla, area);
}
