//! Pantalla 7 — Jornada. El "fichaje" visible de un peer: sus sesiones de trabajo
//! (inicio/fin/duración, timbradas por el broker) más un resumen del tiempo total, y debajo
//! sus tareas con estimado vs real. Datos de `POST /jornada` (sesiones + tareas).
//!
//! El peer enfocado es el seleccionado en la pantalla Peers (vía `traza_peer_actual`, el mismo
//! criterio que Trazabilidad). Todo el render es síncrono y sin estado: lee `&App` y pinta.
//! Si el broker está offline/401, el banner común ya lo refleja y aquí se muestra el último
//! dato bueno cacheado (o un texto guía si aún no hay foco/datos).

use crate::app::{fila_sesion, fila_tarea, formatear_duracion, App};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let foco = app.traza_peer_actual();

    // Sin foco: nada que fichar.
    let Some(id) = foco else {
        let p = Paragraph::new(Span::styled(
            "No hay peers. Ve a la pantalla 1 (Peers) y selecciona uno.",
            Style::default().fg(Color::DarkGray),
        ))
        .block(Block::default().borders(Borders::ALL).title(" Jornada (sin peer) "));
        f.render_widget(p, area);
        return;
    };

    let (sesiones, tareas): (&[_], &[_]) = match &app.datos.jornada {
        Some(j) => (j.sesiones.as_slice(), j.tareas.as_slice()),
        None => (&[], &[]),
    };

    // Layout vertical: resumen (3) · sesiones (mitad) · tareas (resto).
    let zonas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(50),
        Constraint::Min(4),
    ])
    .split(area);

    // --- Resumen del fichaje: nº de sesiones, tiempo total trabajado, sesión abierta ---
    let total_seg: i64 = sesiones.iter().filter_map(|s| s.duracion_seg).sum();
    let abierta = sesiones.iter().any(|s| s.fin.as_deref().map(|x| x.is_empty()).unwrap_or(true));
    let resumen = Line::from(vec![
        Span::styled("  sesiones ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(sesiones.len().to_string()),
        Span::styled("   ·   total trabajado ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(formatear_duracion(Some(total_seg)), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        if abierta {
            Span::styled("   ·   ● en curso", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("   ·   ○ sin sesión abierta", Style::default().fg(Color::DarkGray))
        },
    ]);
    f.render_widget(
        Paragraph::new(resumen).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Jornada · {id} ")),
        ),
        zonas[0],
    );

    // --- Tabla de sesiones ---
    let filas_ses: Vec<Row> = sesiones
        .iter()
        .map(|s| Row::new(fila_sesion(s).into_iter().map(Cell::from).collect::<Vec<_>>()))
        .collect();
    let tabla_ses = Table::new(
        filas_ses,
        [Constraint::Length(12), Constraint::Length(14), Constraint::Min(8)],
    )
    .header(
        Row::new(["inicio", "fin", "duración"])
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(format!(" Sesiones ({}) ", sesiones.len())));
    f.render_widget(tabla_ses, zonas[1]);

    // --- Tabla de tareas (estimado vs real) ---
    let filas_tar: Vec<Row> = tareas
        .iter()
        .map(|t| Row::new(fila_tarea(t).into_iter().map(Cell::from).collect::<Vec<_>>()))
        .collect();
    let tabla_tar = Table::new(
        filas_tar,
        [
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(["tarea", "estimado", "real", "estado"])
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    )
    .block(Block::default().borders(Borders::ALL).title(format!(" Tareas ({}) ", tareas.len())));
    f.render_widget(tabla_tar, zonas[2]);
}
