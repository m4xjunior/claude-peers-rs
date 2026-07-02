//! Pantalla Jornada — el "fichaje" de un peer: sus sesiones (inicio/fin/duración, timbradas por el
//! broker) y sus tareas (estimado de la IA vs real del broker), más el total trabajado. Espejo de
//! `peers-tui/src/ui/jornada.rs`. Render puro y síncrono: lee `EstadoPantalla` y pinta.
//!
//! DECISIÓN (por qué no `Table`/`Badge` del kit): el contrato de firma de la Fundación es
//! `render_jornada(&EstadoPantalla) -> impl IntoElement` (puro, sin `cx`). El `Table` del kit es
//! stateful (exige `Entity` + delegate + contexto), así que se pinta con `v_flex`/`h_flex` de filas
//! — mismo criterio con el que la Fundación construyó el sidebar y las otras vistas, blindando la
//! pantalla contra cambios de API entre commits del git del kit.

use gpui::{div, px, rgb, IntoElement, ParentElement, SharedString, Styled};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;
use peers_core::{EstadoTarea, RespuestaJornada, Sesion, Tarea};

use crate::app::EstadoPantalla;

/// Extrae `HH:MM:SS` de un ISO 8601. Espejo de `hora_iso` de la TUI (vive en otro crate no expuesto
/// como lib; duplicar 6 líneas es más barato que acoplar). Sin patrón `T` → cadena tal cual.
fn hora_iso(iso: &str) -> String {
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        let fin = resto.find(['.', 'Z', '+']).unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

/// Formatea segundos como `HhMM`/`MMmin`/`SSs`. `None`/negativo → "—" (abierta: sin fin timbrado).
/// Espejo de `formatear_duracion` de la TUI.
fn formatear_duracion(seg: Option<i64>) -> String {
    let s = match seg {
        Some(s) if s >= 0 => s,
        _ => return "—".to_string(),
    };
    if s < 60 {
        return format!("{s}s");
    }
    let min = s / 60;
    if min < 60 {
        return format!("{min}min");
    }
    format!("{}h{:02}", min / 60, min % 60)
}

/// Color del chip de estado de una tarea (espejo del mapeo de la TUI, R11). RGB sólido (sin `cx`).
fn color_estado(estado: EstadoTarea) -> u32 {
    match estado {
        EstadoTarea::Abierta => 0x9ca3af,   // gris
        EstadoTarea::EnCurso => 0x22d3ee,   // cian
        EstadoTarea::Bloqueada => 0xf59e0b, // naranja
        EstadoTarea::Hecha => 0x22c55e,     // verde
        EstadoTarea::Cancelada => 0xef4444, // rojo
    }
}

/// Etiqueta corta del estado para el chip.
fn etiqueta_estado(estado: EstadoTarea) -> &'static str {
    match estado {
        EstadoTarea::Abierta => "abierta",
        EstadoTarea::EnCurso => "en curso",
        EstadoTarea::Bloqueada => "bloqueada",
        EstadoTarea::Hecha => "hecha",
        EstadoTarea::Cancelada => "cancelada",
    }
}

/// Celda de texto de ancho fijo (columnas de tiempo/hora). `celda_flex` reparte el sobrante.
fn celda(texto: impl Into<SharedString>, ancho: f32) -> impl IntoElement {
    div().w(px(ancho)).px_1().text_sm().child(texto.into())
}

fn celda_flex(texto: impl Into<SharedString>) -> impl IntoElement {
    div().flex_1().overflow_hidden().px_1().text_sm().child(texto.into())
}

/// Cabecera común de una tabla: fila resaltada con los títulos de columna. Cada entrada es
/// `(titulo, Some(ancho_fijo))` o `(titulo, None)` para la columna flexible.
fn cabecera(cols: &[(&'static str, Option<f32>)]) -> impl IntoElement {
    let mut fila = div()
        .h_flex()
        .gap_2()
        .px_1()
        .py_1()
        .border_b_1()
        .border_color(rgb(0x374151))
        .text_color(rgb(0xfbbf24)); // amarillo (cabeceras de la TUI)
    for (titulo, ancho) in cols {
        fila = match ancho {
            Some(w) => fila.child(celda(*titulo, *w)),
            None => fila.child(celda_flex(*titulo)),
        };
    }
    fila
}

pub fn render_jornada(datos: &EstadoPantalla) -> impl IntoElement {
    let base = div().v_flex().size_full().gap_3().p_4();

    // Sin peer enfocado: nada que fichar (espejo del `else` de la TUI).
    let Some(jornada) = &datos.jornada else {
        return base.child(
            div()
                .v_flex()
                .gap_1()
                .child(div().text_xl().child(SharedString::from("Jornada")))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x9ca3af))
                        .child(SharedString::from(
                            "No hay peer enfocado. Selecciona uno en la pantalla Peers.",
                        )),
                ),
        );
    };

    let RespuestaJornada { sesiones, tareas } = jornada;
    let id_peer = datos.jornada_peer.clone().unwrap_or_default();

    // --- Resumen: nº sesiones · total trabajado · sesión abierta ---
    let total_seg: i64 = sesiones.iter().filter_map(|s| s.duracion_seg).sum();
    let hay_abierta = sesiones
        .iter()
        .any(|s| s.fin.as_deref().map(str::is_empty).unwrap_or(true));

    let resumen = div()
        .h_flex()
        .gap_4()
        .items_center()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x1a1a28))
        .child(div().text_lg().child(SharedString::from(format!("Jornada · {id_peer}"))))
        .child(
            div()
                .text_sm()
                .child(SharedString::from(format!("sesiones {}", sesiones.len()))),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x22d3ee))
                .child(SharedString::from(format!(
                    "total trabajado {}",
                    formatear_duracion(Some(total_seg))
                ))),
        )
        .child(if hay_abierta {
            div()
                .text_sm()
                .text_color(rgb(0x22c55e))
                .child(SharedString::from("● en curso"))
        } else {
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child(SharedString::from("○ sin sesión abierta"))
        });

    // --- Tabla de sesiones ---
    let mut tabla_ses = div()
        .v_flex()
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child(SharedString::from(format!("Sesiones ({})", sesiones.len()))),
        )
        .child(cabecera(&[
            ("inicio", Some(120.0)),
            ("fin", Some(120.0)),
            ("duración", None),
        ]));
    for s in sesiones {
        tabla_ses = tabla_ses.child(fila_sesion(s));
    }

    // --- Tabla de tareas (estimado vs real) ---
    let mut tabla_tar = div()
        .v_flex()
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x9ca3af))
                .child(SharedString::from(format!("Tareas ({})", tareas.len()))),
        )
        .child(cabecera(&[
            ("tarea", None),
            ("estimado", Some(90.0)),
            ("real", Some(90.0)),
            ("estado", Some(110.0)),
        ]));
    for t in tareas {
        tabla_tar = tabla_tar.child(fila_tarea(t));
    }

    base.child(resumen).child(tabla_ses).child(tabla_tar)
}

/// Fila de la tabla de sesiones: inicio · fin ("(abierta)" si sigue viva) · duración.
fn fila_sesion(s: &Sesion) -> impl IntoElement {
    let fin = match &s.fin {
        Some(f) if !f.is_empty() => hora_iso(f),
        _ => "(abierta)".to_string(),
    };
    div()
        .h_flex()
        .gap_2()
        .px_1()
        .py_1()
        .border_b_1()
        .border_color(rgb(0x1f2937))
        .child(celda(hora_iso(&s.inicio), 120.0))
        .child(celda(fin, 120.0))
        .child(celda_flex(formatear_duracion(s.duracion_seg)))
}

/// Fila de la tabla de tareas: descripción · estimado (IA) · real (broker) · chip de estado.
fn fila_tarea(t: &Tarea) -> impl IntoElement {
    div()
        .h_flex()
        .gap_2()
        .items_center()
        .px_1()
        .py_1()
        .border_b_1()
        .border_color(rgb(0x1f2937))
        .child(celda_flex(t.descripcion.clone()))
        .child(celda(formatear_duracion(t.estimado_seg), 90.0))
        .child(celda(formatear_duracion(t.duracion_seg), 90.0))
        .child(
            div().w(px(110.0)).child(
                div()
                    .px_2()
                    .py(px(1.0))
                    .rounded_md()
                    .text_xs()
                    .bg(rgb(color_estado(t.estado)))
                    .text_color(rgb(0x111111))
                    .child(SharedString::from(etiqueta_estado(t.estado))),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duracion_none_es_guion() {
        assert_eq!(formatear_duracion(None), "—");
        assert_eq!(formatear_duracion(Some(-1)), "—");
    }

    #[test]
    fn duracion_formatea_por_tramos() {
        assert_eq!(formatear_duracion(Some(45)), "45s");
        assert_eq!(formatear_duracion(Some(600)), "10min");
        assert_eq!(formatear_duracion(Some(3720)), "1h02");
    }

    #[test]
    fn hora_iso_extrae_hh_mm_ss() {
        assert_eq!(hora_iso("2026-07-01T09:05:00Z"), "09:05:00");
    }
}
