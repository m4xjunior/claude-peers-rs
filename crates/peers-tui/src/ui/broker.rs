//! Pantalla 4 — Broker. Datos de arranque (GET /admin/info) + estado (GET /salud).

use crate::app::App;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let mut lineas = vec![Line::from("")];

    match &app.datos.info {
        Some(info) => {
            lineas.push(campo("host", info.host.clone()));
            lineas.push(campo("puerto", info.puerto.to_string()));
            lineas.push(campo("version", info.version.clone()));
            lineas.push(campo("instancias", info.instancias.to_string()));
        }
        None => lineas.push(Line::from(Span::styled(
            "  (sin datos de /admin/info todavía)",
            Style::default().fg(Color::DarkGray),
        ))),
    }

    lineas.push(Line::from(""));

    match &app.datos.salud {
        Some(s) => {
            let color = if s.estado == "ok" { Color::Green } else { Color::Yellow };
            lineas.push(Line::from(vec![
                Span::styled("  estado      ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(s.estado.clone(), Style::default().fg(color)),
            ]));
            lineas.push(campo("salud.instancias", s.instancias.to_string()));
        }
        None => lineas.push(Line::from(Span::styled(
            "  (sin datos de /salud todavía)",
            Style::default().fg(Color::DarkGray),
        ))),
    }

    let p = Paragraph::new(lineas)
        .block(Block::default().borders(Borders::ALL).title(" Broker "));
    f.render_widget(p, area);
}

fn campo(etiqueta: &str, valor: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {etiqueta:<12}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(valor),
    ])
}
