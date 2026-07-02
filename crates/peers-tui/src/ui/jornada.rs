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

    // Layout vertical: resumen (3) · sesiones · tareas · acciones (registro-acciones R12).
    let zonas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(35),
        Constraint::Percentage(30),
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
    // Ancho real de la columna flexible `descripción` (col 1, Min). Fijas: 10×3=30, 4 columnas.
    let ancho_desc = crate::ui::ancho_columna_flexible(zonas[2].width, 30, 4);
    let filas_tar: Vec<Row> = tareas
        .iter()
        .map(|t| {
            let mut celdas = fila_tarea(t);
            // descripción (celda 0) viene sin recortar: al ancho real de su columna.
            celdas[0] = crate::app::recortar(&celdas[0], ancho_desc);
            Row::new(celdas.into_iter().map(Cell::from).collect::<Vec<_>>())
        })
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

    // --- Bitácora de acciones (registro-acciones R12): hora · acción · sujeto · detalle ---
    // Paridad mínima con la desktop: las últimas N del peer, más reciente primero. Vacía si
    // el broker no tiene bitácora (compat con brokers viejos) o el peer aún no actuó.
    let acciones = &app.datos.jornada_acciones;
    let ancho_detalle = crate::ui::ancho_columna_flexible(zonas[3].width, 34, 4);
    let filas_acc: Vec<Row> = acciones
        .iter()
        .map(|a| {
            let hora = a.cuando.get(11..19).unwrap_or("—").to_string();
            let etiqueta = etiqueta_accion(a.accion).to_string();
            let sujeto = a.sujeto.clone().unwrap_or_default();
            let detalle =
                crate::app::recortar(&a.detalle.clone().unwrap_or_default(), ancho_detalle);
            Row::new(vec![
                Cell::from(hora),
                Cell::from(etiqueta),
                Cell::from(sujeto),
                Cell::from(detalle),
            ])
        })
        .collect();
    let tabla_acc = Table::new(
        filas_acc,
        [
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(["hora", "acción", "sujeto", "detalle"])
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Acciones ({}) ", acciones.len())),
    );
    f.render_widget(tabla_acc, zonas[3]);
}

/// Etiqueta corta en español de un `TipoAccion` (espejo de la desktop). El comodín cubre las
/// variantes futuras del enum `#[non_exhaustive]` — un broker más nuevo no rompe la TUI.
fn etiqueta_accion(a: peers_core::TipoAccion) -> &'static str {
    use peers_core::TipoAccion;
    match a {
        TipoAccion::CrearTarea => "crear tarea",
        TipoAccion::ReportarTarea => "reporte",
        TipoAccion::CerrarTarea => "cerrar tarea",
        TipoAccion::EditarTarea => "editar tarea",
        TipoAccion::CambiarEstadoTarea => "cambio estado",
        TipoAccion::ReasignarTarea => "reasignar",
        TipoAccion::ForzarTarea => "recordatorio",
        TipoAccion::DefinirResumen => "resumen",
        TipoAccion::EnviarMensaje => "mensaje",
        TipoAccion::Kick => "salida",
        TipoAccion::Purgar => "purga",
        TipoAccion::ResolverAlerta => "alerta resuelta",
        _ => "acción",
    }
}
