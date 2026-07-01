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
        Pantalla::Tareas if app.vista_global => {
            "GLOBAL · g vista peer · 1-5 filtra · 0 todas · Tab cambia pantalla · ↑↓ · Enter detalle · e/+/f/b/h/c/R/a acciones"
        }
        Pantalla::Tareas => {
            "g global · [ ] peer · ↑↓ · Enter detalle · e editar · + ampliar · f forzar · n nueva · a reasignar · b/h/c · R reabrir"
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

/// Offset de scroll del viewport de una tabla: primera fila a mostrar para que `seleccion`
/// quede siempre visible. BUG arreglado (2026-06-30): las tablas usaban `render_widget` con
/// TODAS las filas desde la 0 → cuando la lista superaba el alto visible, ratatui recortaba por
/// abajo y la selección "bajaba" pero el viewport no la seguía ("no desce, não mostra nada").
///
/// Estrategia: mantener la selección dentro de la ventana [offset, offset+capacidad). Si la
/// selección cae por debajo del borde inferior, se desplaza el offset para que quede en la
/// última fila visible. `area_alto` es la altura del bloque; se descuentan 2 del borde y 1 del
/// encabezado de la tabla para obtener las filas de datos realmente visibles.
pub fn offset_scroll(seleccion: usize, total: usize, area_alto: u16) -> usize {
    // Filas de datos visibles = alto del área − borde superior/inferior (2) − encabezado (1).
    let capacidad = (area_alto as usize).saturating_sub(3).max(1);
    if total <= capacidad || seleccion < capacidad {
        0
    } else {
        // La selección debe quedar como ÚLTIMA fila visible: offset = seleccion − capacidad + 1.
        (seleccion + 1).saturating_sub(capacidad)
    }
}

/// Ancho disponible (en columnas) para la ÚNICA columna flexible de una tabla, dado el ancho del
/// área y la suma de los anchos fijos de las demás columnas. Sirve para recortar el texto de esa
/// columna al ESPACIO REAL en vez de a una constante fija (fix "textos cortados" 2026-07-01): así
/// en una terminal ancha se ve más contenido. Descuenta 2 del borde del bloque y 1 por columna
/// para el separador. `min` de seguridad para no devolver 0 (que ocultaría toda la celda).
pub fn ancho_columna_flexible(area_ancho: u16, fijas: u16, num_columnas: u16) -> usize {
    let separadores = num_columnas.saturating_sub(1); // ratatui deja 1 col entre columnas
    (area_ancho.saturating_sub(2).saturating_sub(fijas).saturating_sub(separadores)).max(8) as usize
}

#[cfg(test)]
mod pruebas {
    use super::{ancho_columna_flexible, offset_scroll};

    #[test]
    fn ancho_flexible_descuenta_fijas_borde_y_separadores() {
        // área 100, 3 columnas fijas sumando 40, 4 columnas totales →
        // 100 − 2 (borde) − 40 (fijas) − 3 (separadores) = 55.
        assert_eq!(ancho_columna_flexible(100, 40, 4), 55);
    }

    #[test]
    fn ancho_flexible_nunca_menor_que_8() {
        // Terminal muy angosta: no devuelve 0 ni negativo, mínimo 8.
        assert_eq!(ancho_columna_flexible(20, 40, 4), 8);
    }

    #[test]
    fn lista_corta_no_scrollea() {
        // 5 filas en un área de alto 20 (capacidad ~17) → todo cabe, offset 0.
        assert_eq!(offset_scroll(4, 5, 20), 0);
    }

    #[test]
    fn seleccion_dentro_del_viewport_no_scrollea() {
        // área alto 10 → capacidad 7. Selección 3 < 7 → visible sin desplazar.
        assert_eq!(offset_scroll(3, 100, 10), 0);
    }

    #[test]
    fn seleccion_mas_alla_desplaza_el_viewport() {
        // área alto 10 → capacidad 7. Selección 20 → offset = 20+1−7 = 14
        // (filas 14..=20 visibles, la 20 queda como última). ESTE es el caso del bug.
        assert_eq!(offset_scroll(20, 100, 10), 14);
    }

    #[test]
    fn ultima_fila_de_lista_larga_queda_visible() {
        // 50 filas, área alto 10 (cap 7), selección en la última (49) → offset 43.
        assert_eq!(offset_scroll(49, 50, 10), 43);
    }

    #[test]
    fn area_diminuta_no_panica() {
        // alto 0/1/2 → capacidad mínima 1, sin overflow.
        assert_eq!(offset_scroll(0, 0, 0), 0);
        assert_eq!(offset_scroll(5, 10, 1), 5);
    }
}
