//! Pantalla 2 — Red/Acceso. Muestra broker_url y el token enmascarado (ej. `lexus…2026`).
//! Es solo lectura: la edición real vive en la pantalla Config.

use crate::app::App;
use crate::config::enmascarar_token;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let url = &app.config.broker_url;
    let token = enmascarar_token(app.config.token.as_deref());

    let lineas = vec![
        Line::from(""),
        campo("broker_url", url),
        campo("token", &token),
        Line::from(""),
        Line::from(Span::styled(
            "  El token se muestra enmascarado. Para cambiarlo, ve a la pantalla 5 (Config).",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(lineas)
        .block(Block::default().borders(Borders::ALL).title(" Red / Acceso "));
    f.render_widget(p, area);
}

fn campo<'a>(etiqueta: &'a str, valor: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {etiqueta:<12}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(valor),
    ])
}
