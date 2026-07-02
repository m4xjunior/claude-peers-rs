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
    div, prelude::FluentBuilder, px, rgb, Action, AnyElement, App, AppContext, Entity,
    InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Window,
};
use gpui_component::{
    input::{Input, InputState},
    StyledExt,
};

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

// Acción sin datos: cerrar el modal de detalle de tarea (botón "Cerrar", ✕, clic fuera o `Esc`).
gpui::actions!(tareas, [CerrarDetalleTarea]);

/// Abrir el pop-up de DETALLE de la tarea `indice` (tareas-01): doble-click en la fila o botón
/// "Abrir" de la barra de acciones. Índice dentro de `datos.tareas` (la vista es global sin filtro,
/// así que visible == real). `AppDesktop` guarda `Some(indice)` en `tarea_detalle` y monta el
/// overlay en su render raíz (patrón idéntico a `alertas::AbrirDetalle` → `overlay_alerta`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct AbrirDetalleTarea {
    pub indice: usize,
}

// -------------------------------------------------------------------------------------------------
// FORMULARIOS DEL CRUD (tareas-03..08) — un ÚNICO overlay de formulario a la vez, descrito por
// `FormTareas` en `EstadoPantalla.tareas_form`. La vista pinta el modal según la variante; los
// campos de TEXTO viven en `InputsTareas` (entidades `InputState` del kit, que exigen estado) y el
// peer elegido del selector viaja por `EstadoPantalla.tareas_form_peer`. Al confirmar, la vista
// despacha `ConfirmarFormTareas` SIN payload: `AppDesktop` (que tiene los inputs y el cliente) lee
// los valores, valida la frontera y hace el POST correspondiente.
// -------------------------------------------------------------------------------------------------

// Acciones sin datos del ciclo del formulario: abrir el de ASIGNAR (cabecera, no necesita tarea),
// cerrar el formulario activo (Cancelar / ✕ / clic fuera / Esc) y confirmarlo (botón primario).
gpui::actions!(tareas, [AbrirFormAsignar, CerrarFormTareas, ConfirmarFormTareas]);

/// Abrir el formulario de REASIGNAR (tareas-04): selector de peer destino excluyendo al dueño.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct AbrirFormReasignar {
    pub tarea_id: String,
}

/// Abrir el formulario de EDITAR descripción (tareas-05): Input precargado con la actual.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct AbrirFormEditar {
    pub tarea_id: String,
}

/// Abrir el formulario de AMPLIAR estimado (tareas-06): minutos a SUMAR al vigente.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct AbrirFormAmpliar {
    pub tarea_id: String,
}

/// Abrir el formulario de BLOQUEAR con motivo OBLIGATORIO (tareas-07).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct AbrirFormBloquear {
    pub tarea_id: String,
}

/// Pedir CONFIRMACIÓN antes de una transición destructiva (tareas-08): Cancelar (pierde el
/// trabajo dirigido) o Reabrir una terminal (revierte el cierre). `estado` es el destino.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct PedirConfirmEstado {
    pub tarea_id: String,
    pub estado: EstadoTarea,
}

/// Elegir el peer destino en el selector del formulario activo (asignar/reasignar). `AppDesktop`
/// lo guarda en `tareas_form_peer`; la fila elegida se resalta en el propio selector.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tareas, no_json)]
pub struct ElegirPeerForm {
    pub id: String,
}

/// Qué formulario del CRUD está abierto (tareas-03..08). Vive en `EstadoPantalla.tareas_form`
/// (`None` = ninguno). Cada variante lleva el CONTEXTO mínimo capturado al abrir (id de la tarea,
/// dueño a excluir, estimado vigente) para que confirmar no dependa de que la lista no haya
/// cambiado debajo (la recarga periódica puede reordenarla).
#[derive(Clone, PartialEq)]
pub enum FormTareas {
    /// tareas-03: crear tarea nueva (peer + descripción + estimado opcional en minutos).
    Asignar,
    /// tareas-04: elegir peer destino (se excluye `dueno` del selector).
    Reasignar { tarea_id: String, dueno: String },
    /// tareas-05: editar la descripción (el Input se precarga al abrir).
    Editar { tarea_id: String },
    /// tareas-06: sumar minutos al estimado vigente (`estimado_actual` para el hint y la suma).
    Ampliar { tarea_id: String, estimado_actual: Option<i64> },
    /// tareas-07: bloquear con motivo obligatorio.
    Bloquear { tarea_id: String },
    /// tareas-08: confirmación destructiva (Cancelada o Abierta=reabrir). `descripcion` recortada
    /// para el texto "¿Seguro?" (capturada al abrir: no depende de re-buscar la tarea).
    Confirmar {
        tarea_id: String,
        estado: EstadoTarea,
        descripcion: String,
    },
}

/// Las TRES entradas de texto que comparten los formularios del CRUD (sólo un formulario está
/// abierto a la vez, así que se reutilizan): descripción (asignar/editar), estimado en minutos
/// (asignar/ampliar) y motivo (bloquear). Son `Entity<InputState>` del kit — estado real que no
/// cabe en la vista pura — creadas una vez por `AppDesktop` al arrancar (patrón `PanelAcceso`).
/// `AppDesktop` las SIEMBRA al abrir cada formulario y las LEE al confirmar.
#[derive(Clone)]
pub struct InputsTareas {
    pub descripcion: Entity<InputState>,
    pub estimado: Entity<InputState>,
    pub motivo: Entity<InputState>,
}

/// Crea las entradas del CRUD de tareas. Se llama una vez desde `AppDesktop::nueva` (donde hay
/// `Window`/`App`) y se guarda en `EstadoPantalla.inputs_tareas`.
pub fn nuevos_inputs(window: &mut Window, cx: &mut App) -> InputsTareas {
    InputsTareas {
        descripcion: cx.new(|cx| {
            InputState::new(window, cx).placeholder("Descripción de la tarea…")
        }),
        estimado: cx.new(|cx| InputState::new(window, cx).placeholder("minutos (opcional)")),
        motivo: cx.new(|cx| InputState::new(window, cx).placeholder("Motivo del bloqueo…")),
    }
}

impl InputsTareas {
    /// Siembra los tres campos de golpe (los no usados por el formulario van vacíos): abrir un
    /// formulario nunca hereda texto del anterior. `AppDesktop` la llama al abrir cada variante.
    pub fn sembrar(
        &self,
        descripcion: &str,
        estimado: &str,
        motivo: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.descripcion
            .update(cx, |s, cx| s.set_value(descripcion.to_string(), window, cx));
        self.estimado
            .update(cx, |s, cx| s.set_value(estimado.to_string(), window, cx));
        self.motivo
            .update(cx, |s, cx| s.set_value(motivo.to_string(), window, cx));
    }
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
        raiz = raiz.child(barra_acciones(t, seleccion));
    }

    raiz
}

/// Cabecera: eyebrow "SUPERVISIÓN", título "Tareas · GLOBAL (N)" y el botón Asignar (crear tarea).
fn cabecera(total: usize) -> AnyElement {
    // tareas-03: el botón abre el FORMULARIO de asignación (peer + descripción + estimado); el POST
    // lo dispara `ConfirmarFormTareas` cuando el jefe confirma con los campos rellenos.
    let boton = tema::boton_primario("tareas-asignar", "Asignar").on_click(|_e, window, cx| {
        window.dispatch_action(Box::new(AbrirFormAsignar), cx);
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
        .on_click(move |e, window, cx| {
            // Doble-click abre el pop-up de detalle (tareas-01); click simple sólo selecciona.
            // `click_count()` viene del `MouseUpEvent` (los clicks de teclado cuentan como 1).
            if e.click_count() >= 2 {
                window.dispatch_action(Box::new(AbrirDetalleTarea { indice: idx }), cx);
            } else {
                window.dispatch_action(Box::new(SeleccionarTarea { indice: idx }), cx);
            }
        })
}

/// Barra de acciones sobre la tarea seleccionada: Abrir (detalle, tareas-01) + transiciones de
/// estado válidas + Reasignar + Forzar. Card propia bajo la tabla; muestra qué tarea está enfocada
/// y sólo ofrece las acciones con sentido según su estado (terminal → Abrir + Reabrir; viva →
/// Abrir + transiciones + forzar). `indice` es la posición de la tarea en `datos.tareas` (viaja en
/// `AbrirDetalleTarea` para que el overlay apunte a la fila correcta).
fn barra_acciones(t: &Tarea, indice: usize) -> AnyElement {
    let terminal = t.estado.es_terminal();

    let mut fila_botones = div()
        .h_flex()
        .flex_wrap()
        .gap_2()
        .items_center()
        // "Abrir" va siempre primero: el detalle (tareas-01) aplica a cualquier estado.
        .child(boton_abrir(indice));

    if terminal {
        // Tareas Hecha/Cancelada: la única acción con sentido es reabrirlas (→ Abierta). Reabrir
        // una Hecha revierte el cierre (destructiva) → pide confirmación (tareas-08).
        fila_botones = fila_botones.child(boton_confirmar_estado(t, "Reabrir", EstadoTarea::Abierta));
    } else {
        // Tareas vivas: transiciones del ciclo (R5). El broker valida la transición concreta;
        // ofrecemos las principales y dejamos que rechace las inválidas (banner de error).
        // Bloquear abre el formulario de MOTIVO obligatorio (tareas-07); Cancelar pide
        // confirmación (tareas-08); Reasignar abre el selector de peer destino (tareas-04).
        fila_botones = fila_botones
            .child(boton_estado(t, "En curso", EstadoTarea::EnCurso))
            .child(boton_form_bloquear(t))
            .child(boton_estado(t, "Hecha", EstadoTarea::Hecha))
            .child(boton_confirmar_estado(t, "Cancelar", EstadoTarea::Cancelada))
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

/// Botón "Reasignar" (tareas-04): abre el formulario con el SELECTOR de peer destino (excluyendo
/// al dueño actual). El POST lo dispara `ConfirmarFormTareas` con el peer elegido.
fn boton_reasignar(t: &Tarea) -> impl IntoElement {
    let tarea_id = t.id.clone();
    tema::boton_secundario(SharedString::from(format!("reasignar-{}", t.id)), "Reasignar")
        .on_click(move |_e, window, cx| {
            window.dispatch_action(
                Box::new(AbrirFormReasignar {
                    tarea_id: tarea_id.clone(),
                }),
                cx,
            );
        })
}

/// Botón "Bloquear" (tareas-07): abre el formulario de MOTIVO obligatorio en vez de bloquear a
/// ciegas con `motivo=None` (que era exactamente el defecto que la RFC señala).
fn boton_form_bloquear(t: &Tarea) -> impl IntoElement {
    let tarea_id = t.id.clone();
    tema::boton_secundario(SharedString::from(format!("bloquear-{}", t.id)), "Bloquear").on_click(
        move |_e, window, cx| {
            window.dispatch_action(
                Box::new(AbrirFormBloquear {
                    tarea_id: tarea_id.clone(),
                }),
                cx,
            );
        },
    )
}

/// Botón de transición DESTRUCTIVA (tareas-08): "Cancelar" y "Reabrir" no transicionan directo,
/// piden confirmación (`PedirConfirmEstado` abre el mini-modal "¿Seguro?").
fn boton_confirmar_estado(t: &Tarea, label: &'static str, estado: EstadoTarea) -> impl IntoElement {
    let tarea_id = t.id.clone();
    tema::boton_secundario(SharedString::from(format!("confirmar-{label}-{}", t.id)), label)
        .on_click(move |_e, window, cx| {
            window.dispatch_action(
                Box::new(PedirConfirmEstado {
                    tarea_id: tarea_id.clone(),
                    estado,
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

/// Botón "Abrir" (tareas-01): despacha `AbrirDetalleTarea{indice}`. Primario porque leer el detalle
/// es la acción principal que hoy le falta al jefe (el bloqueo #1 de la RFC).
fn boton_abrir(indice: usize) -> impl IntoElement {
    tema::boton_primario(SharedString::from(format!("abrir-tarea-{indice}")), "Abrir").on_click(
        move |_e, window, cx| {
            window.dispatch_action(Box::new(AbrirDetalleTarea { indice }), cx);
        },
    )
}

// -------------------------------------------------------------------------------------------------
// MODAL DE DETALLE (tareas-01) — misma filosofía que `alertas::render_modal_detalle`: la vista
// expone el CONTENIDO del pop-up como función pura; el overlay/backdrop/Esc/clic-fuera los aporta
// `AppDesktop`, que lo monta en su render raíz leyendo `datos.tarea_detalle`. Variante 1 de la RFC:
// dialog centrado 560px sobre tinta2, borde línea radio card, título Fraunces, cada campo con
// eyebrow humo + valor papel, y los datos (id/tiempos) en la fuente mono.
// -------------------------------------------------------------------------------------------------

/// Contenido del pop-up "Detalle de tarea" (tareas-01). Muestra TODO lo que la tabla recorta:
/// descripción íntegra (wrap + scroll), estado (chip semántico con marca ⚠ si overrun), dueño,
/// id, estimado vs real, motivo de bloqueo (si está bloqueada), evidencia (si la hay) e issue de
/// GitHub (si existe). Es el hub desde el que la RFC cuelga editar/reportes en fases siguientes.
///
/// `pub` para que `AppDesktop` lo llame desde su `render` al montar el overlay.
pub fn render_modal_detalle_tarea(t: &Tarea) -> AnyElement {
    let color = color_estado(t.estado);

    let mut modal = div()
        .v_flex()
        .w(px(560.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        // Cabecera: chip de estado + título display, y el ✕ de cierre a la derecha.
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_3()
                        .child(tema::chip_estado(etiqueta_estado(t.estado), color))
                        .child(tema::titulo("Detalle de tarea").text_size(px(18.0))),
                )
                .child(
                    div()
                        .id("modal-tarea-cerrar-x")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(px(tema::RADIO_CONTROL))
                        .text_color(tema::HUMO)
                        .cursor_pointer()
                        .hover(|s| s.bg(tema::TINTA).text_color(tema::PAPEL))
                        .child(SharedString::from("✕"))
                        .on_click(|_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarDetalleTarea), cx);
                        }),
                ),
        )
        // Descripción ÍNTEGRA: la razón de ser del pop-up (la tabla la recorta a 80). Con wrap
        // natural del layout y scroll acotado para que una descripción kilométrica no desborde.
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("descripción"))
                .child(
                    div()
                        .id("modal-tarea-descripcion-scroll")
                        .max_h(px(200.0))
                        .overflow_y_scroll()
                        .child(tema::texto_primario(t.descripcion.clone())),
                ),
        )
        // Dueño e id de la tarea: datos → fuente mono (convención DS de la RFC).
        .child(campo_mono("dueño", t.instancia_id.clone()))
        .child(campo_mono("id", t.id.clone()))
        // Estimado vs real en una sola fila, con la marca ⚠ brasa si está en overrun (#14/R3).
        .child(
            div()
                .h_flex()
                .items_baseline()
                .gap_3()
                .child(div().w(px(110.0)).flex_shrink_0().child(tema::eyebrow("estimado / real")))
                .child(
                    div()
                        .font_family(tema::FUENTE_MONO)
                        .text_color(tema::PAPEL)
                        .child(SharedString::from(format!(
                            "{} / {}",
                            formatear_duracion(t.estimado_seg),
                            formatear_duracion(t.duracion_seg)
                        ))),
                )
                .when(tarea_overrun(t), |d| {
                    d.child(
                        div()
                            .text_color(tema::BRASA)
                            .text_sm()
                            .font_family(tema::FUENTE_MONO)
                            .child(SharedString::from("⚠ overrun")),
                    )
                }),
        );

    // Motivo de bloqueo: sólo se pinta si la tarea está bloqueada. Si el motivo no se capturó
    // (hoy la desktop bloquea con motivo None — tareas-07 lo arreglará), se dice explícitamente.
    if matches!(t.estado, EstadoTarea::Bloqueada) {
        let motivo = t
            .bloqueo_motivo
            .clone()
            .unwrap_or_else(|| "(sin motivo registrado)".to_string());
        modal = modal.child(campo_texto("motivo de bloqueo", motivo));
    }

    // Evidencia (prueba de trabajo, #7): si existe se muestra en mono (suele ser SHA/PR/URL);
    // en tareas Hechas sin evidencia se marca "no-verificada" para que el jefe lo sepa (tareas-14).
    if let Some(ev) = &t.evidencia {
        modal = modal.child(campo_mono("evidencia", ev.clone()));
    } else if matches!(t.estado, EstadoTarea::Hecha) {
        modal = modal.child(campo_texto("evidencia", "(sin evidencia — no verificada)".to_string()));
    }

    // Issue de GitHub espejo, si la integración la creó. Sólo el número por ahora; el enlace
    // clicable (resolver owner/repo del dueño) es tareas-15.
    if let Some(n) = t.issue_number {
        modal = modal.child(campo_mono("issue github", format!("#{n}")));
    }

    // Pie: Cerrar + las acciones que la RFC cuelga del detalle — Editar descripción (tareas-05) y
    // Ampliar estimado (tareas-06). `AppDesktop` cierra este detalle al abrir el formulario.
    let id_editar = t.id.clone();
    let id_ampliar = t.id.clone();
    modal = modal.child(
        div()
            .h_flex()
            .gap_3()
            .pt_2()
            .child(
                tema::boton_secundario("modal-tarea-cerrar", "Cerrar").on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(CerrarDetalleTarea), cx);
                }),
            )
            .child(
                tema::boton_secundario("modal-tarea-editar", "Editar descripción").on_click(
                    move |_e, window, cx| {
                        window.dispatch_action(
                            Box::new(AbrirFormEditar {
                                tarea_id: id_editar.clone(),
                            }),
                            cx,
                        );
                    },
                ),
            )
            .child(
                tema::boton_secundario("modal-tarea-ampliar", "Ampliar estimado").on_click(
                    move |_e, window, cx| {
                        window.dispatch_action(
                            Box::new(AbrirFormAmpliar {
                                tarea_id: id_ampliar.clone(),
                            }),
                            cx,
                        );
                    },
                ),
            ),
    );

    modal.into_any_element()
}

/// Fila "eyebrow humo + valor papel" del modal, con el valor en la fuente MONO (ids, tiempos,
/// SHAs). El eyebrow tiene ancho fijo para que los valores queden alineados en columna.
fn campo_mono(etiqueta: &'static str, valor: String) -> impl IntoElement {
    div()
        .h_flex()
        .items_baseline()
        .gap_3()
        .child(div().w(px(110.0)).flex_shrink_0().child(tema::eyebrow(etiqueta)))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::PAPEL)
                .child(SharedString::from(valor)),
        )
}

/// Fila "eyebrow humo + valor papel" del modal en la fuente de UI (texto humano: motivos, avisos).
fn campo_texto(etiqueta: &'static str, valor: String) -> impl IntoElement {
    div()
        .h_flex()
        .items_baseline()
        .gap_3()
        .child(div().w(px(110.0)).flex_shrink_0().child(tema::eyebrow(etiqueta)))
        .child(tema::texto_primario(valor))
}

// -------------------------------------------------------------------------------------------------
// MODAL DE FORMULARIO (tareas-03..08) — contenido puro del overlay que `AppDesktop` monta leyendo
// `datos.tareas_form` (mismo patrón que el detalle). Los Inputs del kit se pintan desde sus
// `Entity<InputState>` (viven en `datos.inputs_tareas`); el resto es composición Ethos. Confirmar
// SIEMPRE despacha `ConfirmarFormTareas` sin payload: los valores los lee `AppDesktop`.
// -------------------------------------------------------------------------------------------------

/// Rojo terroso para el botón de confirmación DESTRUCTIVA (tareas-08). Mismo tono que usa el modal
/// de alertas para "Sí, descartar": semántica de peligro, calibrada a la paleta cálida.
const ROJO_DESTRUCTIVO: u32 = 0xC0_4A_3E;
const ROJO_DESTRUCTIVO_HOVER: u32 = 0xD0_5A_4E;

/// Contenido del modal del formulario activo (tareas-03..08). Pinta según la variante de
/// `FormTareas`: título, campos (selector de peer y/o Inputs), el error de validación de frontera
/// (si `AppDesktop` lo dejó en `tareas_form_error`) y la botonera Cancelar/confirmar.
pub fn render_modal_form(form: &FormTareas, datos: &EstadoPantalla) -> AnyElement {
    let mut modal = div()
        .v_flex()
        .w(px(520.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        // Cabecera: título display + ✕ (cierra sin confirmar).
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(tema::titulo(titulo_form(form)).text_size(px(18.0)))
                .child(
                    div()
                        .id("modal-form-cerrar-x")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(px(tema::RADIO_CONTROL))
                        .text_color(tema::HUMO)
                        .cursor_pointer()
                        .hover(|s| s.bg(tema::TINTA).text_color(tema::PAPEL))
                        .child(SharedString::from("✕"))
                        .on_click(|_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarFormTareas), cx);
                        }),
                ),
        );

    // Cuerpo según variante. Los formularios con texto exigen `inputs_tareas`; si (caso anómalo)
    // no se construyeron, se avisa en vez de crashear — nunca `.unwrap()`.
    match form {
        FormTareas::Asignar => {
            modal = modal.child(selector_peer(datos, None));
            if let Some(inputs) = &datos.inputs_tareas {
                modal = modal
                    .child(campo_form_input("descripción", &inputs.descripcion))
                    .child(campo_form_input("estimado (min, opcional)", &inputs.estimado));
            } else {
                modal = modal.child(aviso_sin_inputs());
            }
        }
        FormTareas::Reasignar { dueno, .. } => {
            modal = modal
                .child(tema::texto_terciario(format!(
                    "Dueño actual: {} — elige el peer destino.",
                    recortar(dueno, 40)
                )))
                .child(selector_peer(datos, Some(dueno)));
        }
        FormTareas::Editar { .. } => {
            if let Some(inputs) = &datos.inputs_tareas {
                modal = modal.child(campo_form_input("descripción", &inputs.descripcion));
            } else {
                modal = modal.child(aviso_sin_inputs());
            }
        }
        FormTareas::Ampliar { estimado_actual, .. } => {
            modal = modal.child(tema::texto_terciario(format!(
                "Estimado actual: {} — indica los minutos a SUMAR.",
                formatear_duracion(*estimado_actual)
            )));
            if let Some(inputs) = &datos.inputs_tareas {
                modal = modal.child(campo_form_input("minutos a sumar", &inputs.estimado));
            } else {
                modal = modal.child(aviso_sin_inputs());
            }
        }
        FormTareas::Bloquear { .. } => {
            if let Some(inputs) = &datos.inputs_tareas {
                modal = modal.child(campo_form_input("motivo (obligatorio)", &inputs.motivo));
            } else {
                modal = modal.child(aviso_sin_inputs());
            }
        }
        FormTareas::Confirmar {
            estado,
            descripcion,
            ..
        } => {
            // Texto de consecuencia claro (tareas-08): qué pasa si confirma.
            let consecuencia = match estado {
                EstadoTarea::Cancelada => "Se cancela la tarea (transición terminal).",
                EstadoTarea::Abierta => {
                    "Se reabre la tarea: vuelve a Abierta y revierte el cierre."
                }
                _ => "Se aplica la transición de estado.",
            };
            modal = modal
                .child(tema::texto_primario(format!(
                    "¿Seguro? «{}»",
                    recortar(descripcion, 80)
                )))
                .child(tema::texto_terciario(consecuencia.to_string()));
        }
    }

    // Error de validación de frontera (campo vacío, número ilegible…) que dejó `AppDesktop`.
    if let Some(err) = &datos.tareas_form_error {
        modal = modal.child(
            div()
                .text_sm()
                .text_color(rgb(0xF1_A8_A8))
                .child(SharedString::from(format!("⚠ {err}"))),
        );
    }

    // Botonera: Cancelar (secundario) + confirmar. La destructiva va en rojo (tareas-08); el
    // resto en el primario dorado del Ethos.
    let confirmar: AnyElement = if matches!(form, FormTareas::Confirmar { .. }) {
        let label = match form {
            FormTareas::Confirmar {
                estado: EstadoTarea::Abierta,
                ..
            } => "Sí, reabrir",
            _ => "Sí, cancelar tarea",
        };
        div()
            .id("modal-form-confirmar-destructivo")
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .py_2()
            .rounded(px(tema::RADIO_CONTROL))
            .bg(rgb(ROJO_DESTRUCTIVO))
            .text_color(tema::PAPEL)
            .font_weight(gpui::FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|s| s.bg(rgb(ROJO_DESTRUCTIVO_HOVER)))
            .child(SharedString::from(label))
            .on_click(|_e, window, cx| {
                window.dispatch_action(Box::new(ConfirmarFormTareas), cx);
            })
            .into_any_element()
    } else {
        tema::boton_primario("modal-form-confirmar", label_confirmar(form))
            .on_click(|_e, window, cx| {
                window.dispatch_action(Box::new(ConfirmarFormTareas), cx);
            })
            .into_any_element()
    };

    modal = modal.child(
        div()
            .h_flex()
            .gap_3()
            .pt_2()
            .child(
                tema::boton_secundario("modal-form-cancelar", "Cancelar").on_click(
                    |_e, window, cx| {
                        window.dispatch_action(Box::new(CerrarFormTareas), cx);
                    },
                ),
            )
            .child(confirmar),
    );

    modal.into_any_element()
}

/// Título del modal según el formulario activo.
fn titulo_form(form: &FormTareas) -> &'static str {
    match form {
        FormTareas::Asignar => "Asignar tarea nueva",
        FormTareas::Reasignar { .. } => "Reasignar tarea",
        FormTareas::Editar { .. } => "Editar descripción",
        FormTareas::Ampliar { .. } => "Ampliar estimado",
        FormTareas::Bloquear { .. } => "Bloquear tarea",
        FormTareas::Confirmar {
            estado: EstadoTarea::Abierta,
            ..
        } => "Reabrir tarea",
        FormTareas::Confirmar { .. } => "Cancelar tarea",
    }
}

/// Etiqueta del botón primario (dorado) según el formulario. Las destructivas no pasan por aquí.
fn label_confirmar(form: &FormTareas) -> &'static str {
    match form {
        FormTareas::Asignar => "Asignar",
        FormTareas::Reasignar { .. } => "Reasignar",
        FormTareas::Editar { .. } => "Guardar",
        FormTareas::Ampliar { .. } => "Ampliar",
        FormTareas::Bloquear { .. } => "Bloquear",
        FormTareas::Confirmar { .. } => "Confirmar",
    }
}

/// Selector de peer destino (tareas-03/04): lista scrollable de los peers vivos (`datos.instancias`,
/// que `AppDesktop` recarga al abrir el formulario), cada fila clicable → `ElegirPeerForm`. La
/// elegida (`datos.tareas_form_peer`) se resalta con el acento brasa de `fila_seleccionable`.
/// `excluir` deja fuera al dueño actual en REASIGNAR (reasignar al mismo peer es un no-op confuso).
fn selector_peer(datos: &EstadoPantalla, excluir: Option<&str>) -> AnyElement {
    let candidatos: Vec<&peers_core::Instancia> = datos
        .instancias
        .iter()
        .filter(|i| excluir != Some(i.id.as_str()))
        .collect();

    let mut cuerpo = div().v_flex().gap_1().max_h(px(180.0)).overflow_hidden();
    if candidatos.is_empty() {
        cuerpo = cuerpo.child(tema::texto_terciario(
            "No hay peers vivos disponibles (la lista se recarga al abrir este formulario).",
        ));
    } else {
        let mut lista = div()
            .id("selector-peer-scroll")
            .v_flex()
            .gap_1()
            .max_h(px(180.0))
            .overflow_y_scroll();
        for inst in candidatos {
            let elegido = datos.tareas_form_peer.as_deref() == Some(inst.id.as_str());
            let id = inst.id.clone();
            lista = lista.child(
                tema::fila_seleccionable(
                    SharedString::from(format!("peer-form-{}", inst.id)),
                    elegido,
                )
                .child(
                    div()
                        .flex_1()
                        .font_family(tema::FUENTE_MONO)
                        .text_color(tema::PAPEL)
                        .child(SharedString::from(recortar(&inst.id, 44))),
                )
                .on_click(move |_e, window, cx| {
                    window.dispatch_action(Box::new(ElegirPeerForm { id: id.clone() }), cx);
                }),
            );
        }
        cuerpo = cuerpo.child(lista);
    }

    div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow("peer destino"))
        .child(cuerpo)
        .into_any_element()
}

/// Campo de formulario: eyebrow + `Input` del kit envuelto en el marco de control Ethos (borde
/// LINEA, radio control, superficie TINTA). Mismo criterio que `PanelAcceso::input_tematizado`.
fn campo_form_input(etiqueta: &'static str, estado: &Entity<InputState>) -> impl IntoElement {
    div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow(etiqueta))
        .child(
            div()
                .w_full()
                .px_3()
                .py_1()
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .bg(tema::TINTA)
                .border_1()
                .border_color(tema::LINEA)
                .child(Input::new(estado).cleanable(true)),
        )
}

/// Aviso defensivo si los Inputs del CRUD no llegaron a construirse (nunca debería pasar).
fn aviso_sin_inputs() -> AnyElement {
    tema::texto_terciario("Los campos de edición no están inicializados; reinicia la app.")
        .into_any_element()
}
