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

    let filas: Vec<Row> = app
        .datos
        .peers
        .iter()
        .enumerate()
        .map(|(idx, inst)| {
            let celdas = fila_peer(inst);
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
