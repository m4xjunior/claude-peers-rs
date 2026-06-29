//! Pantalla 8 — Tareas. Lista de tareas del peer enfocado con descripción, estimado (lo que la
//! IA dijo) vs real (lo que el broker timbró) y estado fin/abierta. Datos de `POST /listar-tareas`.
//!
//! El peer enfocado es el seleccionado en la pantalla Peers (vía `traza_peer_actual`). Es una
//! tabla seleccionable (↑↓ mueven la fila). Render síncrono y sin estado: lee `&App` y pinta.
//! Offline/401 → banner común; aquí se conserva la última lista buena (o un texto guía).

use crate::app::{fila_tarea, App};
use peers_core::Tarea;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let foco = app.traza_peer_actual();

    let titulo = match &foco {
        Some(id) => format!(" Tareas · {} ({}) ", id, app.datos.tareas.len()),
        None => " Tareas (sin peer — selecciona uno en pantalla 1) ".to_string(),
    };

    if foco.is_none() || app.datos.tareas.is_empty() {
        let guia = if foco.is_none() {
            "No hay peers. Ve a la pantalla 1 (Peers) y selecciona uno."
        } else {
            "Este peer aún no ha abierto tareas en su jornada."
        };
        let p = ratatui::widgets::Paragraph::new(ratatui::text::Span::styled(
            guia,
            Style::default().fg(Color::DarkGray),
        ))
        .block(Block::default().borders(Borders::ALL).title(titulo));
        f.render_widget(p, area);
        return;
    }

    let encabezado = Row::new(["tarea", "estimado", "real", "estado"])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let filas: Vec<Row> = app
        .datos
        .tareas
        .iter()
        .enumerate()
        .map(|(idx, t)| fila_render(idx, t, app.seleccion))
        .collect();

    let tabla = Table::new(
        filas,
        [
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(encabezado)
    .block(Block::default().borders(Borders::ALL).title(titulo));

    f.render_widget(tabla, area);
}

/// Fila de tarea con resalte de la seleccionada y color del estado (verde=fin, amarillo=abierta).
fn fila_render(idx: usize, t: &Tarea, seleccion: usize) -> Row<'static> {
    let [desc, estimado, real, estado] = fila_tarea(t);
    let color_estado = if estado == "fin" { Color::Green } else { Color::Yellow };

    let estilo_fila = if idx == seleccion {
        Style::default().bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(desc),
        Cell::from(estimado),
        Cell::from(real),
        Cell::from(estado).style(Style::default().fg(color_estado)),
    ])
    .style(estilo_fila)
}
