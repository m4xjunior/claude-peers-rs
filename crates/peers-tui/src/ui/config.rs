//! Pantalla 5 — Config. broker_url, token y refresh_ms editables; 's' guarda en el TOML.
//! El campo activo se resalta; 'e' abre el modal de input para editarlo.

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
    let token_mostrado = enmascarar_token(app.config.token.as_deref());
    let refresh = app.config.refresh_ms.to_string();

    let campos = [
        ("broker_url", app.config.broker_url.as_str()),
        ("token", token_mostrado.as_str()),
        ("refresh_ms", refresh.as_str()),
    ];

    let mut lineas = vec![Line::from("")];
    for (idx, (etiqueta, valor)) in campos.iter().enumerate() {
        let activo = idx == app.config_campo;
        let marca = if activo { "▶ " } else { "  " };
        let estilo_etiqueta = if activo {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        lineas.push(Line::from(vec![
            Span::styled(format!("{marca}{etiqueta:<12}"), estilo_etiqueta),
            Span::raw((*valor).to_string()),
        ]));
    }
    lineas.push(Line::from(""));
    lineas.push(Line::from(Span::styled(
        "  ↑↓ elige campo · e edita · s guarda en ~/.config/claude-peers/config.toml",
        Style::default().fg(Color::DarkGray),
    )));

    let p = Paragraph::new(lineas)
        .block(Block::default().borders(Borders::ALL).title(" Config "));
    f.render_widget(p, area);
}
