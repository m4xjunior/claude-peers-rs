//! Pantalla 8 — Tareas (panel de gestión, R11/R12). Lista las tareas del peer enfocado con
//! descripción, estado COLOREADO (Abierta/gris, EnCurso/cian, Bloqueada/naranja, Hecha/verde,
//! Cancelada/rojo), estimado (lo que la IA dijo) vs real (lo que el broker timbró). Datos de
//! `POST /listar-tareas`.
//!
//! El peer enfocado es el de `traza_peer_actual` (ciclado con [ ]). Es una tabla seleccionable
//! (↑↓/click mueven la fila). Sobre ella, el modal DETALLE (Enter) muestra descripción completa,
//! tiempos, estado, motivo de bloqueo y reportes de progreso (R12/AC5). Render síncrono y sin
//! estado: lee `&App` y pinta. Offline/401 → banner común; conserva la última lista buena.

use crate::app::{color_estado_tarea, etiqueta_estado_tarea, fila_tarea, App};
use peers_core::Tarea;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};

pub fn dibujar(f: &mut Frame, area: Rect, app: &App) {
    // La lista a pintar y su orden los decide `App`: en vista global filtra por estado y sube las
    // overrun/atascadas (#14/R3); en vista peer es el orden del broker. La selección indexa ESTA
    // lista (ver `App::tarea_seleccionada`), así las acciones operan sobre la fila correcta (R4).
    let visibles = app.tareas_ordenadas(&app.ahora);

    // Título: en vista global anuncia GLOBAL + filtro activo; en vista peer, el peer enfocado.
    let titulo = if app.vista_global {
        format!(" Tareas · GLOBAL · filtro: {} ({}) ", app.etiqueta_filtro(), visibles.len())
    } else {
        match app.traza_peer_actual() {
            Some(id) => format!(" Tareas · {} ({}) ", id, visibles.len()),
            None => " Tareas (sin peer — selecciona uno en pantalla 1) ".to_string(),
        }
    };

    // Caso vacío: guía contextual distinta para vista peer (sin foco / sin tareas) y global.
    if visibles.is_empty() {
        let guia = if app.vista_global {
            "No hay tareas que coincidan con el filtro. Pulsa 0 para ver todas, g para volver a la vista peer."
        } else if app.traza_peer_actual().is_none() {
            "No hay peers. Ve a la pantalla 1 (Peers) y selecciona uno. Pulsa g para la vista global."
        } else {
            "Este peer aún no ha abierto tareas en su jornada. Pulsa g para la vista global."
        };
        let p = Paragraph::new(Span::styled(guia, Style::default().fg(Color::DarkGray)))
            .block(Block::default().borders(Borders::ALL).title(titulo));
        f.render_widget(p, area);
        return;
    }

    // En vista global añadimos la columna PEER (dueño de la tarea) — el jefe ve de quién es cada una.
    if app.vista_global {
        let encabezado = Row::new(["peer", "tarea", "estado", "estimado", "real"])
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        let filas: Vec<Row> = visibles
            .iter()
            .enumerate()
            .map(|(idx, t)| fila_render_global(idx, t, app.seleccion, &app.ahora))
            .collect();
        let tabla = Table::new(
            filas,
            [
                Constraint::Length(16),
                Constraint::Min(20),
                Constraint::Length(11),
                Constraint::Length(10),
                Constraint::Length(10),
            ],
        )
        .header(encabezado)
        .block(Block::default().borders(Borders::ALL).title(titulo));
        f.render_widget(tabla, area);
    } else {
        let encabezado = Row::new(["tarea", "estado", "estimado", "real"])
            .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        let filas: Vec<Row> = visibles
            .iter()
            .enumerate()
            .map(|(idx, t)| fila_render(idx, t, app.seleccion))
            .collect();
        let tabla = Table::new(
            filas,
            [
                Constraint::Min(20),
                Constraint::Length(11),
                Constraint::Length(10),
                Constraint::Length(10),
            ],
        )
        .header(encabezado)
        .block(Block::default().borders(Borders::ALL).title(titulo));
        f.render_widget(tabla, area);
    }

    // Modal DETALLE encima de la tabla (Enter sobre una fila).
    if let Some(tarea) = &app.tarea_detalle {
        dibujar_detalle(f, area, tarea, &app.tarea_reportes);
    }
}

/// Fila de tarea con resalte de la seleccionada y estado coloreado por `color_estado_tarea`.
/// La columna "real" muestra lo timbrado por el broker (`duracion_seg`); "estimado" lo de la IA.
fn fila_render(idx: usize, t: &Tarea, seleccion: usize) -> Row<'static> {
    let [desc, estimado, real, _estado_viejo] = fila_tarea(t);
    let color = color_estado_tarea(t.estado);
    let etiqueta = etiqueta_estado_tarea(t.estado);

    let estilo_fila = if idx == seleccion {
        Style::default().bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Row::new(vec![
        Cell::from(desc),
        Cell::from(etiqueta).style(Style::default().fg(color)),
        Cell::from(estimado),
        Cell::from(real),
    ])
    .style(estilo_fila)
}

/// Fila de tarea en VISTA GLOBAL: añade la columna PEER (dueño) al principio. Marca las
/// overrun/atascadas con un `⚠` rojo en el peer para que salten a la vista del jefe (#14/R3).
/// `ahora_iso` se usa para evaluar el overrun (mismo reloj que el orden). Función pura.
fn fila_render_global(idx: usize, t: &Tarea, seleccion: usize, ahora_iso: &str) -> Row<'static> {
    let [desc, estimado, real, _estado_viejo] = fila_tarea(t);
    let color = color_estado_tarea(t.estado);
    let etiqueta = etiqueta_estado_tarea(t.estado);
    let overrun = App::tarea_overrun(t, ahora_iso);

    let estilo_fila = if idx == seleccion {
        Style::default().bg(Color::Rgb(40, 40, 60)).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let peer_txt = crate::app::recortar(&t.instancia_id, 14);
    let celda_peer = if overrun {
        // Atascada: prefijo ⚠ y color rojo para destacarla por encima del resto.
        Cell::from(format!("⚠ {peer_txt}")).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    } else {
        Cell::from(peer_txt)
    };

    Row::new(vec![
        celda_peer,
        Cell::from(desc),
        Cell::from(etiqueta).style(Style::default().fg(color)),
        Cell::from(estimado),
        Cell::from(real),
    ])
    .style(estilo_fila)
}

/// Modal centrado con el detalle completo de la tarea: descripción, tiempos, estado (coloreado),
/// motivo de bloqueo y lista de reportes de progreso (AC5). Solo lectura — las acciones se hacen
/// con teclas sobre la tabla (la barra de ayuda las lista). Se cierra con Esc/Enter/q.
fn dibujar_detalle(f: &mut Frame, area: Rect, t: &Tarea, reportes: &[String]) {
    let modal = centrar(72, 18, area);
    f.render_widget(Clear, modal);

    let color = color_estado_tarea(t.estado);
    let estimado = crate::app::formatear_duracion(t.estimado_seg);
    let real = crate::app::formatear_duracion(t.duracion_seg);

    let mut lineas: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Tarea: ", Style::default().fg(Color::DarkGray)),
            Span::raw(t.descripcion.clone()),
        ]),
        Line::from(vec![
            Span::styled("Estado: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                etiqueta_estado_tarea(t.estado),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Estimado: ", Style::default().fg(Color::DarkGray)),
            Span::raw(estimado),
            Span::styled("   Real: ", Style::default().fg(Color::DarkGray)),
            Span::raw(real),
        ]),
        Line::from(vec![
            Span::styled("Dueño: ", Style::default().fg(Color::DarkGray)),
            Span::raw(t.instancia_id.clone()),
        ]),
    ];

    if let Some(motivo) = &t.bloqueo_motivo {
        lineas.push(Line::from(vec![
            Span::styled("Bloqueo: ", Style::default().fg(Color::Rgb(255, 140, 0))),
            Span::raw(motivo.clone()),
        ]));
    }

    lineas.push(Line::from(""));
    lineas.push(Line::from(Span::styled(
        format!("Reportes de progreso ({}):", reportes.len()),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    if reportes.is_empty() {
        lineas.push(Line::from(Span::styled(
            "(sin reportes)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for r in reportes {
            lineas.push(Line::from(Span::raw(format!("• {r}"))));
        }
    }
    lineas.push(Line::from(""));
    lineas.push(Line::from(Span::styled(
        "Esc/Enter cierra · e editar · + ampliar · f forzar · b/h/c/R estado",
        Style::default().fg(Color::DarkGray),
    )));

    let bloque = Block::default()
        .borders(Borders::ALL)
        .title(" Detalle de tarea ")
        .border_style(Style::default().fg(color));
    f.render_widget(Paragraph::new(lineas).block(bloque), modal);
}

/// Rectángulo centrado de `ancho` × `alto` dentro de `area` (acotado al tamaño disponible).
fn centrar(ancho: u16, alto: u16, area: Rect) -> Rect {
    let ancho = ancho.min(area.width);
    let alto = alto.min(area.height);
    let x = area.x + (area.width.saturating_sub(ancho)) / 2;
    let y = area.y + (area.height.saturating_sub(alto)) / 2;
    Rect { x, y, width: ancho, height: alto }
}
