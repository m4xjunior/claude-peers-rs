//! Pantalla 3 — Redis. Colas de mensajes y outbox pendientes por peer (GET /admin/redis).
//! La selección recae sobre la lista de colas de mensajes (acción 'p' purga la seleccionada).

use crate::app::App;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    let columnas = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    let (total, colas, outbox) = match &app.datos.redis {
        Some(r) => (r.total_instancias, r.colas.as_slice(), r.outbox.as_slice()),
        None => (0, &[][..], &[][..]),
    };

    // Tabla izquierda: colas de mensajes (seleccionable).
    let filas_colas: Vec<Row> = colas
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let estilo = if idx == app.seleccion {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new([Cell::from(c.id.clone()), Cell::from(c.pendientes.to_string())]).style(estilo)
        })
        .collect();
    let tabla_colas = Table::new(
        filas_colas,
        [Constraint::Min(10), Constraint::Length(12)],
    )
    .header(encabezado(["peer", "mensajes"]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Colas de mensajes · {total} peers ")),
    );
    f.render_widget(tabla_colas, columnas[0]);

    // Tabla derecha: outbox pendiente (solo lectura).
    let filas_outbox: Vec<Row> = outbox
        .iter()
        .map(|c| Row::new([Cell::from(c.id.clone()), Cell::from(c.pendientes.to_string())]))
        .collect();
    let tabla_outbox = Table::new(
        filas_outbox,
        [Constraint::Min(10), Constraint::Length(12)],
    )
    .header(encabezado(["peer", "outbox"]))
    .block(Block::default().borders(Borders::ALL).title(" Outbox pendiente "));
    f.render_widget(tabla_outbox, columnas[1]);
}

fn encabezado(cols: [&'static str; 2]) -> Row<'static> {
    Row::new(cols).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
}
