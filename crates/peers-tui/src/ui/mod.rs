//! Render de la TUI. `dibujar` arma el marco común (tabs arriba, banner de red, ayuda abajo,
//! modal de input si está activo) y delega el cuerpo a la pantalla activa.
//!
//! Todo el render es síncrono y sin estado propio: lee `&App` y pinta. La E/S de red vive
//! fuera (loop principal), de modo que un render nunca puede colgar la UI.

mod acceso;
mod alertas;
mod broker;
mod config;
mod jornada;
mod peers;
mod redis;
mod tareas;
mod trazabilidad;

use crate::app::{App, EstadoRed, Pantalla};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Tabs},
    Frame,
};

/// Punto de entrada del render de un frame.
pub fn dibujar(f: &mut Frame, app: &App) {
    let areas = Layout::vertical([
        Constraint::Length(3), // barra de pestañas
        Constraint::Length(1), // banner de estado de red
        Constraint::Min(1),    // cuerpo de la pantalla
        Constraint::Length(1), // barra de ayuda
    ])
    .split(f.area());

    dibujar_tabs(f, areas[0], app);
    dibujar_banner(f, areas[1], app);
    dibujar_cuerpo(f, areas[2], app);
    dibujar_ayuda(f, areas[3], app);

    // El modal de input se pinta encima de todo si está activo.
    if app.input.esta_activo() {
        dibujar_input(f, f.area(), app);
    }
}

fn dibujar_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titulos: Vec<Line> = Pantalla::TODAS
        .iter()
        .map(|p| Line::from(p.titulo()))
        .collect();
    let tabs = Tabs::new(titulos)
        .block(Block::default().borders(Borders::ALL).title(" claude-peers · panel "))
        .select(app.pantalla.indice())
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))
        .divider("│");
    f.render_widget(tabs, area);
}

/// Banner de estado de red: verde si Ok, rojo si Error (offline/401/otro). NUNCA crashea —
/// es solo texto. Si hay un flash efímero, tiene prioridad de muestra.
fn dibujar_banner(f: &mut Frame, area: Rect, app: &App) {
    if let Some(flash) = &app.flash {
        let p = Paragraph::new(Span::styled(
            format!(" {flash}"),
            Style::default().fg(Color::Black).bg(Color::Green),
        ));
        f.render_widget(p, area);
        return;
    }
    let (texto, estilo) = match &app.red {
        EstadoRed::Desconocido => (
            " conectando…".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        EstadoRed::Ok => (
            " ● conectado".to_string(),
            Style::default().fg(Color::Green),
        ),
        EstadoRed::Error(e) => (
            format!(" ✕ {e} — reintentando…"),
            Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    };
    f.render_widget(Paragraph::new(Span::styled(texto, estilo)), area);
}

fn dibujar_cuerpo(f: &mut Frame, area: Rect, app: &App) {
    match app.pantalla {
        Pantalla::Peers => peers::dibujar(f, area, app),
        Pantalla::Acceso => acceso::dibujar(f, area, app),
        Pantalla::Redis => redis::dibujar(f, area, app),
        Pantalla::Broker => broker::dibujar(f, area, app),
        Pantalla::Config => config::dibujar(f, area, app),
        Pantalla::Trazabilidad => trazabilidad::dibujar(f, area, app),
        Pantalla::Jornada => jornada::dibujar(f, area, app),
        Pantalla::Tareas => tareas::dibujar(f, area, app),
        Pantalla::Alertas => alertas::dibujar(f, area, app),
    }
}

/// Barra de ayuda contextual al pie. Cambia las teclas según la pantalla.
fn dibujar_ayuda(f: &mut Frame, area: Rect, app: &App) {
    let comun = "Tab cambia · 1-9 pantalla · q/Esc salir";
    let especifico = match app.pantalla {
        Pantalla::Peers => "↑↓ mover · m mensaje · k kick · r resumen",
        Pantalla::Acceso => "(solo lectura — edita en Config)",
        Pantalla::Redis => "↑↓ mover · p purgar cola",
        Pantalla::Broker => "(solo lectura)",
        Pantalla::Config => "e editar campo activo · ↑↓ campo · s guardar",
        Pantalla::Trazabilidad => "[ ] cambiar peer · ↑↓ mover · Enter timeline · r reenviar",
        Pantalla::Jornada => "[ ] cambiar peer · (solo lectura — fichaje)",
        Pantalla::Tareas => {
            "[ ] peer · ↑↓ · Enter detalle · e editar · + ampliar · f forzar · n nueva · a reasignar · b/h/c bloq/hecha/cancel · R reabrir"
        }
        Pantalla::Alertas => "↑↓ mover · (solo lectura)",
    };
    let linea = Line::from(vec![
        Span::styled(format!(" {especifico}"), Style::default().fg(Color::Cyan)),
        Span::styled(format!("  ·  {comun}"), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(linea), area);
}

/// Modal centrado de input (mensaje / resumen / campo de config). Enter confirma, Esc cancela.
fn dibujar_input(f: &mut Frame, area: Rect, app: &App) {
    let modal = centrar(60, 5, area);
    f.render_widget(Clear, modal);
    let titulo = format!(" {} ", app.input.etiqueta());
    let bloque = Block::default()
        .borders(Borders::ALL)
        .title(titulo)
        .border_style(Style::default().fg(Color::Cyan));
    // El token es secreto: ni siquiera al editarlo se muestra en claro. Se pinta con
    // bullets (un • por carácter) para no revelarlo en pantalla. El resto de campos, normal.
    let mostrado = if matches!(app.input, crate::app::Input::ConfigToken) {
        "•".repeat(app.buffer.chars().count())
    } else {
        app.buffer.clone()
    };
    // Cursor de bloque al final del texto para señalar el punto de inserción.
    let contenido = Line::from(vec![
        Span::raw(mostrado),
        Span::styled("▏", Style::default().fg(Color::Cyan)),
    ]);
    let p = Paragraph::new(vec![
        contenido,
        Line::from(Span::styled(
            "Enter confirma · Esc cancela",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(bloque);
    f.render_widget(p, modal);
}

/// Calcula un rectángulo centrado de `ancho` × `alto` dentro de `area`.
fn centrar(ancho: u16, alto: u16, area: Rect) -> Rect {
    let ancho = ancho.min(area.width);
    let alto = alto.min(area.height);
    let x = area.x + (area.width.saturating_sub(ancho)) / 2;
    let y = area.y + (area.height.saturating_sub(alto)) / 2;
    Rect { x, y, width: ancho, height: alto }
}
