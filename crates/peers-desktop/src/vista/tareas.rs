//! Pantalla Tareas — porta la pantalla 8 de la TUI (`peers-tui/src/ui/tareas.rs`) a GPUI.
//!
//! QUÉ MUESTRA (espejo de la TUI, R11/R12): una TABLA de tareas con el estado COLOREADO
//! (Abierta/gris, EnCurso/cian, Bloqueada/naranja, Hecha/verde, Cancelada/rojo), el ESTIMADO
//! (lo que la IA dijo al abrir la tarea) frente al REAL (lo que el broker timbró) y —en vista
//! GLOBAL— una columna PEER con el dueño de cada tarea (el jefe ve de quién es cada una). Las
//! tareas atascadas/overrun se marcan con `⚠` rojo para que salten a la vista del jefe (#14/R3).
//! Sobre cada fila hay ACCIONES: Reasignar, Forzar ("tócale el hombro") y Cerrar; arriba, un
//! botón Asignar para crear tarea nueva. Los datos vienen de `GET /admin/tareas`.
//!
//! POR QUÉ ESTE ENFOQUE (decisión de diseño, coherente con la Fundación): la tabla se construye
//! con `div()` en `v_flex`/`h_flex` (una fila = un `h_flex` de celdas), NO con el componente
//! `Table` del kit. Motivo idéntico al del sidebar de `app.rs`: el `Table` de gpui-component
//! exige un `Entity<impl TableDelegate>` con estado, imposible de construir dentro de la firma
//! pura de la Fundación `render_tareas(&EstadoPantalla) -> impl IntoElement` (no recibe `cx`). Un
//! grid de `div` es estable entre commits del kit, respeta la firma sin tocarla y deja el estado
//! coloreado y las acciones ya cableadas. Migrable a `Table` en Fase 3, cuando la app pase a un
//! patrón con `Entity`/`cx` disponibles en el render.
//!
//! POR QUÉ LOS DIÁLOGOS QUEDAN "ARMADOS" PERO NO DISPARAN AÚN: confirmar y ejecutar una acción
//! (Reasignar/Forzar/Cerrar) necesita `window`/`cx` para abrir el `Dialog` del kit y lanzar la
//! llamada al broker con `cx.spawn`. La firma pura actual no los expone. Por eso los `Button`
//! quedan pintados y con su `on_click` conectado a un punto único (`accion_tarea`) que en Fase 3
//! abrirá el diálogo de confirmación y disparará el método del cliente correspondiente
//! (`ClienteBroker::tarea_asignar/reasignar/forzar/estado`, ya declarados en `cliente.rs`).

use gpui::{
    div, px, rgb, rgba, App as GpuiApp, ClickEvent, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{Disableable, Sizable, StyledExt};

use peers_core::{EstadoTarea, Tarea};

use crate::app::EstadoPantalla;

// ---------------------------------------------------------------------------
// Helpers de presentación — espejo EXACTO de los de la TUI (`peers-tui/src/app.rs`), reescritos
// para el modelo de color de GPUI (`Rgba`) en vez de `ratatui::style::Color`. Funciones puras.
// ---------------------------------------------------------------------------

/// Etiqueta legible del estado de una tarea. Espejo de `etiqueta_estado_tarea` de la TUI.
fn etiqueta_estado(estado: EstadoTarea) -> &'static str {
    match estado {
        EstadoTarea::Abierta => "abierta",
        EstadoTarea::EnCurso => "en curso",
        EstadoTarea::Bloqueada => "bloqueada",
        EstadoTarea::Hecha => "hecha",
        EstadoTarea::Cancelada => "cancelada",
    }
}

/// Color del estado (R11), mismos valores que `color_estado_tarea` de la TUI traducidos a RGB:
/// Abierta/gris, EnCurso/cian, Bloqueada/naranja, Hecha/verde, Cancelada/rojo. Se usa como color
/// de texto (y fondo tenue) del "pill" de estado. Función pura.
fn color_estado(estado: EstadoTarea) -> gpui::Rgba {
    match estado {
        EstadoTarea::Abierta => rgb(0x9ca3af),    // gris
        EstadoTarea::EnCurso => rgb(0x22d3ee),    // cian
        EstadoTarea::Bloqueada => rgb(0xff8c00),  // naranja (RGB 255,140,0, igual que la TUI)
        EstadoTarea::Hecha => rgb(0x22c55e),      // verde
        EstadoTarea::Cancelada => rgb(0xef4444),  // rojo
    }
}

/// Formatea una duración en segundos como `HhMM` / `MMmin` / `SSs`. `None`/negativo → "—".
/// Espejo EXACTO de `formatear_duracion` de la TUI (misma lógica, sin dependencias de ratatui).
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
    let horas = min / 60;
    let resto_min = min % 60;
    format!("{horas}h{resto_min:02}")
}

/// ¿La tarea está en overrun (el real ya supera al estimado)? Marca con `⚠` las atascadas en la
/// vista del jefe (espejo aproximado de `App::tarea_overrun`). Sólo aplica a tareas VIVAS (no
/// terminales) con estimado y real conocidos: si el real timbrado supera al estimado, está
/// atascada. Sin real (tarea aún abierta) o sin estimado, no se puede afirmar → false. Función pura.
fn tarea_overrun(t: &Tarea) -> bool {
    if t.estado.es_terminal() {
        return false;
    }
    match (t.estimado_seg, t.duracion_seg) {
        (Some(est), Some(real)) if est > 0 => real > est,
        _ => false,
    }
}

/// Recorta un texto a `max` caracteres añadiendo `…` si sobra. Espejo de `recortar` de la TUI,
/// para que descripciones/ids largos no rompan el ancho de columna. Cuenta caracteres (no bytes)
/// para no partir un multibyte. Función pura.
fn recortar(texto: &str, max: usize) -> String {
    let n = texto.chars().count();
    if n <= max {
        return texto.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let recortado: String = texto.chars().take(max.saturating_sub(1)).collect();
    format!("{recortado}…")
}

// ---------------------------------------------------------------------------
// Acción de fila — punto único de cableado para Fase 3.
// ---------------------------------------------------------------------------

/// Las acciones que el jefe puede lanzar sobre una tarea. `Copy` porque es un discriminante
/// trivial que viaja en los `on_click`. Fase 3 abrirá el `Dialog` de confirmación y disparará
/// el método del cliente correspondiente según esta variante.
#[derive(Debug, Clone, Copy)]
enum AccionTarea {
    /// Cambiar el dueño de la tarea (→ `ClienteBroker::tarea_reasignar`). Elige peer destino.
    Reasignar,
    /// Empujar recordatorio a la sesión del dueño (→ `ClienteBroker::tarea_forzar`).
    Forzar,
    /// Cerrar la tarea marcándola `Hecha` (→ `ClienteBroker::tarea_estado` con `Hecha`).
    Cerrar,
    /// Crear una tarea nueva asignada a un peer (→ `ClienteBroker::tarea_asignar`). Cabecera.
    Asignar,
}

/// Punto ÚNICO donde se resolverá cada acción en Fase 3. Hoy sólo deja traza: sin `window`/`cx`
/// en el render puro no se puede abrir el `Dialog` del kit ni lanzar `cx.spawn`. Concentrar aquí
/// el `on_click` de TODOS los botones asegura que Fase 3 tenga un solo sitio que tocar: sustituir
/// este cuerpo por "abrir Dialog de confirmación → llamar al método del cliente → refrescar".
fn accion_tarea(
    accion: AccionTarea,
    tarea_id: &str,
    _ev: &ClickEvent,
    _window: &mut Window,
    _cx: &mut GpuiApp,
) {
    // Traza mínima mientras el cableado real (Dialog + broker) llega en Fase 3. No es un panic
    // ni un no-op silencioso: deja constancia de qué se pidió y sobre qué tarea.
    eprintln!("[tareas] acción {accion:?} solicitada sobre tarea '{tarea_id}' (cableado en Fase 3)");
}

// ---------------------------------------------------------------------------
// Render de la pantalla.
// ---------------------------------------------------------------------------

/// Pantalla Tareas: cabecera con contador + botón Asignar, y la tabla de todas las tareas de
/// todas las instancias (vista global) con estado coloreado, estimado vs real, columna peer y
/// acciones por fila. Lee `datos.tareas` (poblado por la app vía `admin_tareas`).
pub fn render_tareas(datos: &EstadoPantalla) -> impl IntoElement {
    let tareas = datos.tareas.as_slice();

    div()
        .v_flex()
        .size_full()
        .gap_3()
        .p_4()
        .child(cabecera(tareas.len()))
        .child(cuerpo_tabla(tareas))
}

/// Cabecera: título con el nº de tareas y el botón global "Asignar" (crear tarea nueva a un peer).
fn cabecera(total: usize) -> impl IntoElement {
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_xl()
                .child(SharedString::from(format!("Tareas · GLOBAL ({total})"))),
        )
        .child(
            // "Asignar" crea una tarea nueva; primary porque es la acción constructiva principal.
            Button::new("tareas-asignar")
                .primary()
                .small()
                .label("Asignar")
                .on_click(|ev, window, cx| accion_tarea(AccionTarea::Asignar, "", ev, window, cx)),
        )
}

/// Cuerpo: encabezado de columnas + una fila por tarea. Caso vacío → guía contextual (espejo del
/// caso vacío de la TUI). Envuelve las filas en un scroll vertical para listas largas.
fn cuerpo_tabla(tareas: &[Tarea]) -> impl IntoElement {
    if tareas.is_empty() {
        // Sin tareas: mensaje guía, no una tabla vacía muda (espejo del banner vacío de la TUI).
        return div()
            .v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_color(rgb(0x9ca3af))
                    .child(SharedString::from(
                        "No hay tareas abiertas en ningún peer todavía.",
                    )),
            )
            .into_any_element();
    }

    let filas = tareas.iter().map(fila_tarea).collect::<Vec<_>>();

    div()
        .v_flex()
        .size_full()
        .child(encabezado_columnas())
        .child(
            // Las filas scrollean si exceden el alto; el encabezado queda fijo arriba.
            div()
                .id("tareas-scroll")
                .v_flex()
                .size_full()
                .overflow_y_scroll()
                .children(filas),
        )
        .into_any_element()
}

/// Fila de encabezado con los nombres de columna (peer | tarea | estado | estimado | real |
/// acciones). Vista GLOBAL: incluye la columna peer, igual que la TUI cuando `vista_global`.
fn encabezado_columnas() -> impl IntoElement {
    let cel = |texto: &str, ancho: gpui::Pixels| {
        div()
            .w(ancho)
            .flex_shrink_0()
            .text_xs()
            .text_color(rgb(0xeab308)) // amarillo, como el header de la TUI
            .child(SharedString::from(texto.to_string()))
    };

    div()
        .h_flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_2()
        .border_b_1()
        .border_color(rgba(0xffffff20))
        .child(cel("peer", px(140.0)))
        .child(
            // La descripción es la columna flexible: ocupa el espacio restante.
            div()
                .flex_1()
                .min_w(px(160.0))
                .text_xs()
                .text_color(rgb(0xeab308))
                .child(SharedString::from("tarea")),
        )
        .child(cel("estado", px(96.0)))
        .child(cel("estimado", px(80.0)))
        .child(cel("real", px(80.0)))
        .child(cel("acciones", px(220.0)))
}

/// Una fila de tarea: peer (con `⚠` rojo si overrun), descripción, pill de estado coloreado,
/// estimado, real y los botones de acción. Espejo de `fila_render_global` de la TUI.
fn fila_tarea(t: &Tarea) -> impl IntoElement {
    let overrun = tarea_overrun(t);
    let color = color_estado(t.estado);
    let peer_txt = recortar(&t.instancia_id, 16);
    // Cada `on_click` necesita su propia copia del id (los closures son `'static`).
    let id_reasignar = t.id.clone();
    let id_forzar = t.id.clone();
    let id_cerrar = t.id.clone();

    // Celda peer: si la tarea está atascada, prefijo ⚠ y color rojo para que salte a la vista
    // del jefe (#14/R3), igual que en la TUI global.
    let celda_peer = if overrun {
        div()
            .w(px(140.0))
            .flex_shrink_0()
            .text_sm()
            .text_color(rgb(0xef4444))
            .child(SharedString::from(format!("⚠ {peer_txt}")))
    } else {
        div()
            .w(px(140.0))
            .flex_shrink_0()
            .text_sm()
            .child(SharedString::from(peer_txt))
    };

    div()
        .id(SharedString::from(format!("fila-{}", t.id)))
        .h_flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_2()
        .border_b_1()
        .border_color(rgba(0xffffff10))
        .child(celda_peer)
        // Descripción: columna flexible con recorte suave (evita romper el layout).
        .child(
            div()
                .flex_1()
                .min_w(px(160.0))
                .text_sm()
                .child(SharedString::from(recortar(&t.descripcion, 80))),
        )
        // Pill de estado coloreado: fondo tenue del color + texto del color (buen contraste).
        .child(
            div().w(px(96.0)).flex_shrink_0().child(
                div()
                    .px_2()
                    .py(px(1.0))
                    .rounded_md()
                    .bg(color.opacity(0.15))
                    .text_xs()
                    .text_color(color)
                    .child(SharedString::from(etiqueta_estado(t.estado))),
            ),
        )
        .child(
            div()
                .w(px(80.0))
                .flex_shrink_0()
                .text_sm()
                .child(SharedString::from(formatear_duracion(t.estimado_seg))),
        )
        .child(
            div()
                .w(px(80.0))
                .flex_shrink_0()
                .text_sm()
                .child(SharedString::from(formatear_duracion(t.duracion_seg))),
        )
        // Acciones por fila: Reasignar / Forzar / Cerrar. Cada `on_click` lleva su `tarea_id`
        // clonado y delega en el punto único `accion_tarea` (Fase 3 abrirá el Dialog y llamará
        // al broker). "Cerrar" es danger porque es la transición terminal (marca `Hecha`) y se
        // deshabilita si la tarea ya es terminal (no tiene sentido re-cerrarla).
        .child(
            div()
                .w(px(220.0))
                .flex_shrink_0()
                .h_flex()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("reasignar-{}", t.id)))
                        .ghost()
                        .xsmall()
                        .label("Reasignar")
                        .on_click(move |ev, window, cx| {
                            accion_tarea(AccionTarea::Reasignar, &id_reasignar, ev, window, cx)
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("forzar-{}", t.id)))
                        .warning()
                        .xsmall()
                        .label("Forzar")
                        .on_click(move |ev, window, cx| {
                            accion_tarea(AccionTarea::Forzar, &id_forzar, ev, window, cx)
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("cerrar-{}", t.id)))
                        .danger()
                        .xsmall()
                        .label("Cerrar")
                        .disabled(t.estado.es_terminal())
                        .on_click(move |ev, window, cx| {
                            accion_tarea(AccionTarea::Cerrar, &id_cerrar, ev, window, cx)
                        }),
                ),
        )
}
