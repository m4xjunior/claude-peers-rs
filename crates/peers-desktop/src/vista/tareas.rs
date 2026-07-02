//! Pantalla Tareas — porta la pantalla 8 de la TUI (`peers-tui/src/ui/tareas.rs`) a GPUI, ahora
//! con el Design System "Ethos" (paleta pergamino/tinta + acento dorado "brasa") y OPERABLE de
//! verdad, no sólo navegable.
//!
//! QUÉ MUESTRA (espejo de la TUI, R11/R12): una TABLA de tareas de TODOS los peers (vista global)
//! con el estado como CHIP coloreado (Abierta/gris, EnCurso/cian, Bloqueada/naranja, Hecha/verde,
//! Cancelada/rojo), el ESTIMADO (lo que la IA dijo al abrir) frente al REAL (lo que el broker
//! timbró) y la columna PEER con el dueño. Las tareas atascadas/overrun se marcan con `⚠` para que
//! salten a la vista del jefe (#14/R3). Sobre la fila SELECCIONADA hay una barra de ACCIONES:
//! En curso / Bloquear / Hecha / Cancelar / Reabrir (transiciones R5), Reasignar y Forzar
//! ("tócale el hombro"); arriba, un botón Asignar para crear tarea nueva. Los datos vienen de
//! `GET /admin/tareas`.
//!
//! POR QUÉ ESTE ENFOQUE (coherente con `alertas.rs` y la Fundación): la vista es PURA
//! (`render_tareas(&EstadoPantalla) -> impl IntoElement`, sin `cx` ni estado propio). No cablea
//! callbacks que llamen al broker: DESPACHA `Action`s que `AppDesktop` maneja con
//! `.on_action(cx.listener(...))` en su contenedor raíz (Fase 3). GPUI hace burbujear la acción
//! desde el elemento clicado hasta ese manejador. Así la vista sólo LEE `EstadoPantalla` y la
//! mutación (seleccionar fila, llamar al broker, refrescar) vive donde hay `cx`. La tabla se pinta
//! con `div()`/`fila_seleccionable` del tema (no el `Table` del kit, que exige `Entity` stateful).
//!
//! MODELO DE INTERACCIÓN (como pidió el rediseño): una fila se SELECCIONA al clicarla
//! (`SeleccionarTarea{indice}`); las acciones de estado/reasignar/forzar operan sobre la tarea
//! seleccionada y despachan su `Action` propia. Tras cada mutación, `AppDesktop` recarga
//! `admin_tareas`. Los estados terminales (Hecha/Cancelada) sólo ofrecen "Reabrir"; los vivos
//! ofrecen las transiciones que tengan sentido.

use gpui::{
    div, px, rgb, Action, AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
use gpui_component::StyledExt;

use peers_core::{EstadoTarea, Tarea};

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — namespace `tareas` para no colisionar con las de otras pantallas. `no_json` evita
// arrastrar `serde`/`schemars`: sólo se despachan por código (`window.dispatch_action`), nunca
// desde un keymap. El payload lleva lo mínimo para que `AppDesktop` actúe sin re-mirar la vista.
//
// CÓMO LAS CABLEA FASE 3 (cada una en `.on_action(cx.listener(...))` del render raíz de AppDesktop):
//   - SeleccionarTarea{indice}          → datos.tareas_seleccion = indice; cx.notify()
//   - ReasignarTarea{tarea_id, nuevo}   → cliente.tarea_reasignar(&tarea_id, &nuevo); recargar
//   - ForzarTarea{tarea_id}             → cliente.tarea_forzar(&tarea_id); recargar
//   - CambiarEstadoTarea{tarea_id, estado} → cliente.tarea_estado(&tarea_id, estado, None, None); recargar
//   - AsignarTarea{instancia_id, descripcion, estimado_seg} → cliente.tarea_asignar(...); recargar
// El patrón de fetch es el de `app.rs::cargar_peers`: cx.background_executor().spawn(async {
//   cliente.bloquear_en(cliente.metodo()) }) + await en cx.spawn, y luego admin_tareas para refrescar.
// -------------------------------------------------------------------------------------------------

/// Seleccionar la fila `indice` (click en la fila). Índice dentro de `datos.tareas`. La barra de
/// acciones opera sobre la tarea seleccionada, así que este es el paso previo a cualquier acción.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct SeleccionarTarea {
    pub indice: usize,
}

/// Reasignar la tarea a otro peer (→ `ClienteBroker::tarea_reasignar`). `tarea_id` identifica la
/// tarea; `nuevo_instancia_id` es el peer destino. La vista despacha con el destino ya resuelto
/// (Fase 3 puede abrir un selector de peer; de momento se cablea el rota-al-siguiente-vivo).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct ReasignarTarea {
    pub tarea_id: String,
    pub nuevo_instancia_id: String,
}

/// "Tócale el hombro": empuja un recordatorio a la sesión del dueño (→ `ClienteBroker::tarea_forzar`).
/// No cambia el estado, sólo notifica. Sólo tiene sentido en tareas vivas (no terminales).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct ForzarTarea {
    pub tarea_id: String,
}

/// Transicionar el ciclo de vida de la tarea (→ `ClienteBroker::tarea_estado`, validado por el
/// broker con `transicion_valida`). `estado` es el destino: EnCurso/Bloqueada/Hecha/Cancelada/
/// Abierta(reabrir). Cubre TODOS los botones de estado de la barra de acciones (R5).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct CambiarEstadoTarea {
    pub tarea_id: String,
    pub estado: EstadoTarea,
}

/// Crear una tarea NUEVA asignada a un peer (→ `ClienteBroker::tarea_asignar`). Cabecera. En Fase 3
/// puede abrir un formulario (peer + descripción + estimado); de momento lleva los campos para que
/// el manejador los use tal cual. `estimado_seg` opcional (la IA no siempre estima).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct AsignarTarea {
    pub instancia_id: String,
    pub descripcion: String,
    pub estimado_seg: Option<i64>,
}

// -------------------------------------------------------------------------------------------------
// HELPERS PUROS — presentación. Espejo de los de la TUI (`peers-tui/src/app.rs`) reescritos para el
// modelo de color de GPUI. El color de estado se mantiene por SEMÁNTICA (rojo=peligro, verde=ok…):
// el Ethos usa el dorado para el CROMO (fila activa, labels), pero los estados de dominio conservan
// su color propio dentro del chip, igual que las alertas.
// -------------------------------------------------------------------------------------------------

/// Etiqueta legible del estado. Espejo de `etiqueta_estado_tarea` de la TUI.
fn etiqueta_estado(estado: EstadoTarea) -> &'static str {
    match estado {
        EstadoTarea::Abierta => "abierta",
        EstadoTarea::EnCurso => "en curso",
        EstadoTarea::Bloqueada => "bloqueada",
        EstadoTarea::Hecha => "hecha",
        EstadoTarea::Cancelada => "cancelada",
    }
}

/// Color semántico del estado (R11), mismos valores que la TUI. Se usa como fondo del chip; el
/// texto del chip va en SALMO (oscuro) para contraste, como en `tema::chip_estado`.
fn color_estado(estado: EstadoTarea) -> gpui::Rgba {
    match estado {
        EstadoTarea::Abierta => rgb(0x9ca3af),   // gris neutro
        EstadoTarea::EnCurso => rgb(0x22d3ee),   // cian
        EstadoTarea::Bloqueada => rgb(0xff8c00), // naranja
        EstadoTarea::Hecha => rgb(0x22c55e),     // verde
        EstadoTarea::Cancelada => rgb(0xef4444), // rojo
    }
}

/// Formatea una duración en segundos como `HhMM` / `MMmin` / `SSs`. `None`/negativo → "—".
/// Espejo EXACTO de `formatear_duracion` de la TUI.
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
/// vista del jefe (#14/R3). Sólo tareas VIVAS con estimado y real conocidos. Función pura.
fn tarea_overrun(t: &Tarea) -> bool {
    if t.estado.es_terminal() {
        return false;
    }
    match (t.estimado_seg, t.duracion_seg) {
        (Some(est), Some(real)) if est > 0 => real > est,
        _ => false,
    }
}

/// Recorta un texto a `max` caracteres (no bytes) con elipsis. Espejo de `recortar` de la TUI.
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

// Anchos fijos (px) de las columnas no flexibles. `tarea` es la flexible (`flex_1`).
const COL_PEER: f32 = 150.0;
const COL_ESTADO: f32 = 110.0;
const COL_ESTIMADO: f32 = 84.0;
const COL_REAL: f32 = 84.0;

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Pantalla Tareas: eyebrow + título con el conteo y botón Asignar, banner de error si lo hubo,
/// tabla global de tareas (estado coloreado, estimado vs real, columna peer) y, bajo ella, la barra
/// de acciones sobre la tarea seleccionada. Lee `datos.tareas` y `datos.tareas_seleccion`.
pub fn render_tareas(datos: &EstadoPantalla) -> impl IntoElement {
    let tareas = datos.tareas.as_slice();
    let total = tareas.len();

    let mut raiz = tema::fondo_app().v_flex().gap_4().p_6();

    // Cabecera: eyebrow + título (nº de tareas) a la izquierda, botón Asignar a la derecha.
    raiz = raiz.child(cabecera(total));

    // Banner de error del broker (offline / 401 / otro) si la última operación falló.
    if let Some(err) = &datos.error_tareas {
        raiz = raiz.child(banner_error(&err.to_string()));
    }

    if tareas.is_empty() {
        // Sin tareas: mensaje guía dentro de una card, no una tabla muda.
        raiz = raiz.child(estado_vacio());
        return raiz;
    }

    // Selección acotada al rango actual (defensivo: la lista pudo encoger tras recargar).
    let seleccion = datos.tareas_seleccion.min(total.saturating_sub(1));

    raiz = raiz.child(tabla(tareas, seleccion));

    // Barra de acciones sobre la tarea seleccionada (bajo la tabla, siempre visible si hay tareas).
    if let Some(t) = tareas.get(seleccion) {
        raiz = raiz.child(barra_acciones(t));
    }

    raiz
}

/// Cabecera: eyebrow "SUPERVISIÓN", título "Tareas · GLOBAL (N)" y el botón Asignar (crear tarea).
fn cabecera(total: usize) -> AnyElement {
    // El botón Asignar despacha `AsignarTarea`. Sin formulario aún: se despacha con campos vacíos y
    // Fase 3 decide si abre un diálogo de captura antes de llamar al broker (documentado arriba).
    let boton = tema::boton_primario("tareas-asignar", "Asignar").on_click(|_e, window, cx| {
        window.dispatch_action(
            Box::new(AsignarTarea {
                instancia_id: String::new(),
                descripcion: String::new(),
                estimado_seg: None,
            }),
            cx,
        );
    });

    div()
        .h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("Supervisión"))
                .child(tema::titulo(format!("Tareas · GLOBAL ({total})"))),
        )
        .child(boton)
        .into_any_element()
}

/// Banner con el motivo del fallo del broker. Usa el rojo semántico de severidad (no un token del
/// Ethos): un error debe leerse como error. Superficie card para integrarse con el resto.
fn banner_error(texto: &str) -> AnyElement {
    tema::superficie_card()
        .w_full()
        .px_4()
        .py_3()
        .border_color(color_estado(EstadoTarea::Cancelada))
        .child(
            tema::texto_primario(format!("⚠ {texto}"))
                .text_color(color_estado(EstadoTarea::Cancelada)),
        )
        .into_any_element()
}

/// Estado vacío: mensaje guía centrado en una card (espejo del banner vacío de la TUI).
fn estado_vacio() -> AnyElement {
    tema::superficie_card()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(tema::texto_terciario(
            "No hay tareas abiertas en ningún peer todavía.",
        ))
        .into_any_element()
}

/// Tabla completa: encabezado de columnas (eyebrows) + filas seleccionables scrollables, todo
/// dentro de una superficie card.
fn tabla(tareas: &[Tarea], seleccion: usize) -> AnyElement {
    let filas = tareas
        .iter()
        .enumerate()
        .map(|(idx, t)| fila_tarea(idx, t, idx == seleccion))
        .collect::<Vec<_>>();

    tema::superficie_card()
        .v_flex()
        .size_full()
        .overflow_hidden()
        .child(encabezado_columnas())
        .child(
            // Las filas scrollean si exceden el alto; el encabezado queda fijo arriba.
            div()
                .id("tareas-scroll")
                .v_flex()
                .size_full()
                .p_2()
                .gap_1()
                .overflow_y_scroll()
                .children(filas),
        )
        .into_any_element()
}

/// Fila de encabezado con eyebrows por columna (peer | tarea | estado | estimado | real).
fn encabezado_columnas() -> impl IntoElement {
    let cel = |texto: &str, ancho: f32| div().w(px(ancho)).flex_shrink_0().child(tema::eyebrow(texto));

    div()
        .h_flex()
        .items_center()
        .gap_2()
        .px_5()
        .py_3()
        .border_b_1()
        .border_color(tema::LINEA)
        .child(cel("peer", COL_PEER))
        .child(div().flex_1().min_w(px(160.0)).child(tema::eyebrow("tarea")))
        .child(cel("estado", COL_ESTADO))
        .child(cel("estimado", COL_ESTIMADO))
        .child(cel("real", COL_REAL))
}

/// Una fila de tarea seleccionable: peer (con `⚠` dorado si overrun), descripción, chip de estado,
/// estimado y real. Clic → despacha `SeleccionarTarea{indice}`. La fila activa se resalta con el
/// acento brasa tenue (lo hace `tema::fila_seleccionable`).
fn fila_tarea(idx: usize, t: &Tarea, activa: bool) -> impl IntoElement {
    let overrun = tarea_overrun(t);
    let peer_txt = recortar(&t.instancia_id, 16);
    let color = color_estado(t.estado);

    // Celda peer: si la tarea está atascada, prefijo ⚠ y color brasa (llama la atención dentro del
    // lenguaje del Ethos, sin gritar en rojo salvo que el estado ya sea rojo).
    let celda_peer = if overrun {
        div()
            .w(px(COL_PEER))
            .flex_shrink_0()
            .text_color(tema::BRASA)
            .child(SharedString::from(format!("⚠ {peer_txt}")))
    } else {
        div()
            .w(px(COL_PEER))
            .flex_shrink_0()
            .text_color(tema::PAPEL)
            .child(SharedString::from(peer_txt))
    };

    tema::fila_seleccionable(SharedString::from(format!("tarea-fila-{idx}")), activa)
        .child(celda_peer)
        // Descripción: columna flexible con recorte suave. Datos → fuente mono del Ethos.
        .child(
            div()
                .flex_1()
                .min_w(px(160.0))
                .text_color(tema::PAPEL)
                .child(SharedString::from(recortar(&t.descripcion, 80))),
        )
        // Chip de estado coloreado por semántica (fondo = color del estado, texto SALMO).
        .child(
            div()
                .w(px(COL_ESTADO))
                .flex_shrink_0()
                .child(tema::chip_estado(etiqueta_estado(t.estado), color)),
        )
        // Estimado y real → fuente mono (datos), color humo para no competir con la descripción.
        .child(
            div()
                .w(px(COL_ESTIMADO))
                .flex_shrink_0()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .child(SharedString::from(formatear_duracion(t.estimado_seg))),
        )
        .child(
            div()
                .w(px(COL_REAL))
                .flex_shrink_0()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .child(SharedString::from(formatear_duracion(t.duracion_seg))),
        )
        .on_click(move |_e, window, cx| {
            window.dispatch_action(Box::new(SeleccionarTarea { indice: idx }), cx);
        })
}

/// Barra de acciones sobre la tarea seleccionada: transiciones de estado válidas + Reasignar +
/// Forzar. Card propia bajo la tabla; muestra qué tarea está enfocada y sólo ofrece las acciones
/// con sentido según su estado (terminal → sólo Reabrir; viva → transiciones + forzar).
fn barra_acciones(t: &Tarea) -> AnyElement {
    let terminal = t.estado.es_terminal();

    let mut fila_botones = div().h_flex().flex_wrap().gap_2().items_center();

    if terminal {
        // Tareas Hecha/Cancelada: la única acción con sentido es reabrirlas (→ Abierta).
        fila_botones = fila_botones.child(boton_estado(t, "Reabrir", EstadoTarea::Abierta));
    } else {
        // Tareas vivas: transiciones del ciclo (R5). El broker valida la transición concreta;
        // ofrecemos las principales y dejamos que rechace las inválidas (banner de error).
        fila_botones = fila_botones
            .child(boton_estado(t, "En curso", EstadoTarea::EnCurso))
            .child(boton_estado(t, "Bloquear", EstadoTarea::Bloqueada))
            .child(boton_estado(t, "Hecha", EstadoTarea::Hecha))
            .child(boton_estado(t, "Cancelar", EstadoTarea::Cancelada))
            .child(boton_reasignar(t))
            .child(boton_forzar(t));
    }

    tema::superficie_card()
        .v_flex()
        .w_full()
        .gap_3()
        .p_4()
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow(format!(
                    "Acción sobre · {}",
                    recortar(&t.instancia_id, 24)
                )))
                .child(tema::texto_primario(recortar(&t.descripcion, 100))),
        )
        .child(fila_botones)
        .into_any_element()
}

/// Botón de transición de estado: despacha `CambiarEstadoTarea{tarea_id, estado}`. Secundario
/// (borde) salvo "Hecha" que va primario (dorado) por ser la transición constructiva principal.
fn boton_estado(t: &Tarea, label: &'static str, estado: EstadoTarea) -> impl IntoElement {
    let tarea_id = t.id.clone();
    let id = SharedString::from(format!("estado-{label}-{}", t.id));

    let boton = if matches!(estado, EstadoTarea::Hecha) {
        tema::boton_primario(id, label)
    } else {
        tema::boton_secundario(id, label)
    };

    boton.on_click(move |_e, window, cx| {
        window.dispatch_action(
            Box::new(CambiarEstadoTarea {
                tarea_id: tarea_id.clone(),
                estado,
            }),
            cx,
        );
    })
}

/// Botón "Reasignar": despacha `ReasignarTarea`. El destino se resuelve en Fase 3 (selector de
/// peer); de momento despacha con `nuevo_instancia_id` vacío para que el manejador decida la ruta.
fn boton_reasignar(t: &Tarea) -> impl IntoElement {
    let tarea_id = t.id.clone();
    tema::boton_secundario(SharedString::from(format!("reasignar-{}", t.id)), "Reasignar")
        .on_click(move |_e, window, cx| {
            window.dispatch_action(
                Box::new(ReasignarTarea {
                    tarea_id: tarea_id.clone(),
                    nuevo_instancia_id: String::new(),
                }),
                cx,
            );
        })
}

/// Botón "Forzar" ("tócale el hombro"): despacha `ForzarTarea`. No cambia estado, sólo notifica.
fn boton_forzar(t: &Tarea) -> impl IntoElement {
    let tarea_id = t.id.clone();
    tema::boton_secundario(SharedString::from(format!("forzar-{}", t.id)), "Forzar").on_click(
        move |_e, window, cx| {
            window.dispatch_action(
                Box::new(ForzarTarea {
                    tarea_id: tarea_id.clone(),
                }),
                cx,
            );
        },
    )
}
