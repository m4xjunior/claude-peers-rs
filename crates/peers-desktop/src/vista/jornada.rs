//! Pantalla Jornada — el "fichaje" de un peer: sus sesiones (inicio/fin/duración, timbradas por el
//! broker) y sus tareas (estimado de la IA vs real del broker), más el total trabajado. Espejo de
//! `peers-tui/src/ui/jornada.rs`. Render puro y síncrono: lee `EstadoPantalla` y pinta.
//!
//! REDISEÑO ETHOS (por qué): la versión previa usaba literales azules/genéricos (0x1a1a28, 0x374151,
//! 0x22d3ee…) y sin tipografía. Ahora todo pasa por `crate::tema`: fondos tinta, texto pergamino,
//! acento dorado "brasa" con moderación, `superficie_card` para los paneles, `eyebrow` para las
//! cabeceras de columna y mono para los datos temporales. Se ve sobrio-reverente, no "un lixo".
//!
//! INTERACTIVIDAD (por qué así): la vista es PURA (`render_jornada(&EstadoPantalla) -> impl
//! IntoElement`, sin `cx`), así que NO cablea callbacks: cada fila clicable DESPACHA una Action que
//! `AppDesktop` maneja con `.on_action(cx.listener(...))` (mismo patrón que `vista/alertas.rs`).
//! La jornada es SÓLO LECTURA (no hay acciones destructivas): las Actions sólo mueven la SELECCIÓN
//! de fila (una por tabla) para dar feedback de "estoy aquí" como en la TUI. GPUI burbujea la Action
//! por el árbol desde la fila clicada hasta el manejador de `AppDesktop`, sin depender del foco.
//!
//! DECISIÓN (por qué no `Table` del kit): el contrato de la Fundación es una firma pura sin `cx`; el
//! `Table` del kit es stateful (exige `Entity` + delegate). Se pinta con `v_flex`/`h_flex` de filas
//! usando `tema::fila_seleccionable`, blindando la pantalla contra cambios de API del kit.

use gpui::{
    div, px, Action, AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;
use peers_core::{EstadoTarea, RespuestaJornada, Sesion, Tarea};

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — namespace `jornada`. La vista es pura: no toca `cx`, sólo DESPACHA. La jornada es de
// sólo lectura (R: no destructivas), así que las Actions sólo mueven la selección de fila para dar
// feedback visual (espejo del cursor de la TUI). `no_json`: se despachan por código, nunca desde un
// keymap, así no arrastramos `schemars` como dependencia (igual que en `alertas.rs`).
//
// FASE 3 debe cablearlas en `AppDesktop` con `.on_action(cx.listener(...))` en su render raíz:
//   - `SeleccionarSesion { indice }` → `self.datos.jornada_ses_sel = indice; cx.notify();`
//   - `SeleccionarTareaJornada { indice }` → `self.datos.jornada_tar_sel = indice; cx.notify();`
// NINGUNA llama al cliente (sólo mutan estado local de UI); no hay método de broker asociado.
// -------------------------------------------------------------------------------------------------

/// Seleccionar la sesión `indice` (click en una fila de la tabla de sesiones). Índice dentro de
/// `jornada.sesiones`. Sólo mueve la selección de UI; no toca el broker.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct SeleccionarSesion {
    pub indice: usize,
}

/// Seleccionar la tarea `indice` (click en una fila de la tabla de tareas). Índice dentro de
/// `jornada.tareas`. Sólo mueve la selección de UI; no toca el broker.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct SeleccionarTareaJornada {
    pub indice: usize,
}

// Acciones sin datos: alternar el dropdown del selector de peer (jornada-03) y cerrar el modal
// de detalle de sesión (jornada-02). El modal de detalle de TAREA (jornada-01) reutiliza
// `tareas::CerrarDetalleTarea` porque su contenido ES el modal de la pestaña Tareas (fase 1).
gpui::actions!(jornada, [AlternarDropdownJornada, CerrarSesionJornada]);

/// Abrir el pop-up de DETALLE de la tarea `indice` de la jornada (jornada-01): doble-click en la
/// fila de tarea. Índice dentro de `jornada.tareas`. El contenido del modal se REUTILIZA de la
/// fase 1 (`tareas::render_modal_detalle_tarea`); `AppDesktop` lo monta en `overlay_tarea_jornada`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct AbrirTareaJornada {
    pub indice: usize,
}

/// Abrir el pop-up de DETALLE de la sesión `indice` (jornada-02): doble-click en la fila de
/// sesión. Índice dentro de `jornada.sesiones`. Muestra id, inicio/fin absolutos, duración y las
/// tareas cuyo `sesion_id` coincide (correlación local, sin endpoint nuevo).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct AbrirSesionJornada {
    pub indice: usize,
}

/// Elegir un peer concreto en el DROPDOWN del selector (jornada-03, parte A de la decisión de
/// Max). `AppDesktop` cierra el dropdown, fija el foco y recarga `POST /jornada`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct ElegirPeerJornada {
    pub id: String,
}

/// Ciclar al peer anterior/siguiente con los chevrones `‹`/`›` (jornada-03, parte B de la
/// decisión de Max — espejo de `[`/`]` de la TUI). `delta` = -1 (anterior) o +1 (siguiente).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct CiclarPeerJornada {
    pub delta: i32,
}

/// Transicionar el estado de una tarea DESDE el detalle de jornada (jornada-04): Hecha /
/// Cancelada / Reabrir. `AppDesktop` cierra el modal y delega en el flujo de la fase 2
/// (`cambiar_estado_tarea` → `mutar_tarea`: validación del broker + recarga + toast).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jornada, no_json)]
pub struct CambiarEstadoTareaJornada {
    pub tarea_id: String,
    pub estado: EstadoTarea,
}

// -------------------------------------------------------------------------------------------------
// HELPERS PUROS — espejo de los de la TUI. Ahora los colores de estado usan la paleta Ethos donde
// aplica; el resto (severidades) mantiene sus hex de dominio porque son semánticos (verde=hecho…).
// -------------------------------------------------------------------------------------------------

/// Extrae `HH:MM:SS` de un ISO 8601. Espejo de `hora_iso` de la TUI. Sin patrón `T` → cadena tal cual.
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

/// Color del chip de estado de una tarea (espejo del mapeo de la TUI, R11), como `Rgba` para
/// `tema::chip_estado`. Semántico (no de marca): verde=hecho, rojo=cancelada, etc.
fn color_estado(estado: EstadoTarea) -> gpui::Rgba {
    match estado {
        EstadoTarea::Abierta => gpui::rgb(0x9CA3AF),   // gris
        EstadoTarea::EnCurso => gpui::rgb(0x38BDF8),   // cian sobrio
        EstadoTarea::Bloqueada => gpui::rgb(0xF59E0B), // ámbar
        EstadoTarea::Hecha => gpui::rgb(0x22C55E),     // verde
        EstadoTarea::Cancelada => gpui::rgb(0xEF4444), // rojo
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

// -------------------------------------------------------------------------------------------------
// CELDAS — datos temporales en mono (Ethos: timestamps/números en `IBM Plex Mono`, fallback mono).
// Texto libre (descripción) en la fuente UI heredada. Los anchos fijos alinean las columnas.
// -------------------------------------------------------------------------------------------------

/// Celda de dato temporal de ancho fijo (hora/duración): fuente mono, tamaño pequeño, papel.
fn celda_dato(texto: impl Into<SharedString>, ancho: f32) -> impl IntoElement {
    div()
        .w(px(ancho))
        .px_1()
        .font_family(tema::FUENTE_MONO)
        .text_size(px(13.0))
        .text_color(tema::PAPEL)
        .child(texto.into())
}

/// Celda de dato temporal flexible (reparte el sobrante horizontal).
fn celda_dato_flex(texto: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .overflow_hidden()
        .px_1()
        .font_family(tema::FUENTE_MONO)
        .text_size(px(13.0))
        .text_color(tema::PAPEL)
        .child(texto.into())
}

/// Celda de texto libre flexible (descripción de tarea): fuente UI heredada, papel.
fn celda_texto_flex(texto: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .overflow_hidden()
        .px_1()
        .text_size(px(14.0))
        .text_color(tema::PAPEL)
        .child(texto.into())
}

/// Cabecera de columnas: fila de `eyebrow` (mayúsculas mono/humo) con borde inferior LINEA. Cada
/// entrada es `(titulo, Some(ancho_fijo))` o `(titulo, None)` para la columna flexible.
fn cabecera(cols: &[(&'static str, Option<f32>)]) -> impl IntoElement {
    let mut fila = div()
        .h_flex()
        .gap_2()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(tema::LINEA);
    for (titulo, ancho) in cols {
        fila = match ancho {
            Some(w) => fila.child(div().w(px(*w)).px_1().child(tema::eyebrow(*titulo))),
            None => fila.child(div().flex_1().px_1().child(tema::eyebrow(*titulo))),
        };
    }
    fila
}

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

pub fn render_jornada(datos: &EstadoPantalla) -> impl IntoElement {
    // Raíz sobre fondo app (tinta + papel + Inter). `fondo_app` ya fija size_full/color/fuente.
    let base = tema::fondo_app().v_flex().gap_4().p_6();

    // Sin peer enfocado: card guía CON el selector de peer (jornada-03) — la queja era justamente
    // que había que irse a la pantalla Peers para poder ver una jornada.
    let Some(jornada) = &datos.jornada else {
        return base.child(
            tema::superficie_card()
                .v_flex()
                .gap_3()
                .p_6()
                .child(tema::eyebrow("Fichaje"))
                .child(tema::titulo("Jornada"))
                .child(tema::texto_terciario(
                    "No hay peer enfocado. Elige uno aquí mismo:",
                ))
                .child(selector_peer_jornada(datos)),
        );
    };

    let RespuestaJornada { sesiones, tareas } = jornada;
    let id_peer = datos.jornada_peer.clone().unwrap_or_default();
    let ses_sel = datos.jornada_ses_sel;
    let tar_sel = datos.jornada_tar_sel;

    // --- Resumen: nº sesiones · total trabajado · sesión abierta (card cabecera) ---
    let total_seg: i64 = sesiones.iter().filter_map(|s| s.duracion_seg).sum();
    let hay_abierta = sesiones
        .iter()
        .any(|s| s.fin.as_deref().map(str::is_empty).unwrap_or(true));

    let resumen = tema::superficie_card()
        .v_flex()
        .gap_3()
        .p_5()
        .child(tema::eyebrow("Fichaje"))
        // Título + selector de peer inline (jornada-03) en la misma fila: el selector vive donde
        // se lee el nombre del peer, sin salir de la pestaña.
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(tema::titulo(format!("Jornada · {id_peer}")))
                .child(selector_peer_jornada(datos)),
        )
        .child(
            div()
                .h_flex()
                .gap_5()
                .items_center()
                // Métrica: nº de sesiones (label humo + valor mono papel).
                .child(metrica("sesiones", &sesiones.len().to_string()))
                // Total trabajado: el dato clave → valor en acento brasa (con moderación).
                .child(metrica_acento("total trabajado", &formatear_duracion(Some(total_seg))))
                // Estado de la jornada: chip vivo/cerrado.
                .child(if hay_abierta {
                    tema::chip_estado("● en curso", gpui::rgb(0x22C55E))
                } else {
                    tema::chip_estado("○ sin sesión abierta", tema::HUMO)
                }),
        );

    // --- Tabla de sesiones ---
    let mut tabla_ses = tema::superficie_card().v_flex().gap_1().p_4().child(
        div()
            .h_flex()
            .items_center()
            .gap_2()
            .pb_1()
            .child(tema::eyebrow("Sesiones"))
            .child(tema::texto_terciario(format!("({})", sesiones.len()))),
    );
    tabla_ses = tabla_ses.child(cabecera(&[
        ("inicio", Some(120.0)),
        ("fin", Some(120.0)),
        ("duración", None),
    ]));
    for (idx, s) in sesiones.iter().enumerate() {
        tabla_ses = tabla_ses.child(fila_sesion(idx, s, ses_sel));
    }

    // --- Tabla de tareas (estimado vs real) ---
    let mut tabla_tar = tema::superficie_card().v_flex().gap_1().p_4().child(
        div()
            .h_flex()
            .items_center()
            .gap_2()
            .pb_1()
            .child(tema::eyebrow("Tareas"))
            .child(tema::texto_terciario(format!("({})", tareas.len()))),
    );
    tabla_tar = tabla_tar.child(cabecera(&[
        ("tarea", None),
        ("estimado", Some(90.0)),
        ("real", Some(90.0)),
        ("estado", Some(120.0)),
    ]));
    for (idx, t) in tareas.iter().enumerate() {
        tabla_tar = tabla_tar.child(fila_tarea(idx, t, tar_sel));
    }

    base.child(resumen).child(tabla_ses).child(tabla_tar)
}

/// Métrica del resumen: label humo pequeño encima, valor mono papel debajo.
fn metrica(label: &str, valor: &str) -> impl IntoElement {
    div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow(label.to_string()))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_size(px(16.0))
                .text_color(tema::PAPEL)
                .child(SharedString::from(valor.to_string())),
        )
}

/// Métrica destacada: como `metrica` pero el valor en acento brasa (dato clave: total trabajado).
fn metrica_acento(label: &str, valor: &str) -> impl IntoElement {
    div()
        .v_flex()
        .gap_1()
        .child(tema::eyebrow(label.to_string()))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_size(px(16.0))
                .text_color(tema::BRASA)
                .child(SharedString::from(valor.to_string())),
        )
}

/// Fila de la tabla de sesiones: inicio · fin ("(abierta)" si sigue viva) · duración. Clicable →
/// despacha `SeleccionarSesion { indice }`. `fila_seleccionable` resalta la activa (brasa tenue).
fn fila_sesion(idx: usize, s: &Sesion, seleccion: usize) -> impl IntoElement {
    let fin = match &s.fin {
        Some(f) if !f.is_empty() => hora_iso(f),
        _ => "(abierta)".to_string(),
    };
    tema::fila_seleccionable(SharedString::from(format!("jornada-ses-{idx}")), idx == seleccion)
        .child(celda_dato(hora_iso(&s.inicio), 120.0))
        .child(celda_dato(fin, 120.0))
        .child(celda_dato_flex(formatear_duracion(s.duracion_seg)))
        .on_click(move |evento, window, cx| {
            // Doble-click abre el detalle de la sesión (jornada-02); click simple selecciona.
            if evento.click_count() >= 2 {
                window.dispatch_action(Box::new(AbrirSesionJornada { indice: idx }), cx);
            } else {
                window.dispatch_action(Box::new(SeleccionarSesion { indice: idx }), cx);
            }
        })
}

/// Fila de la tabla de tareas: descripción · estimado (IA) · real (broker) · chip de estado.
/// Clicable → despacha `SeleccionarTareaJornada { indice }`.
fn fila_tarea(idx: usize, t: &Tarea, seleccion: usize) -> impl IntoElement {
    tema::fila_seleccionable(SharedString::from(format!("jornada-tar-{idx}")), idx == seleccion)
        .child(celda_texto_flex(t.descripcion.clone()))
        .child(celda_dato(formatear_duracion(t.estimado_seg), 90.0))
        .child(celda_dato(formatear_duracion(t.duracion_seg), 90.0))
        .child(
            div()
                .w(px(120.0))
                .child(tema::chip_estado(etiqueta_estado(t.estado), color_estado(t.estado))),
        )
        .on_click(move |evento, window, cx| {
            // Doble-click abre el detalle de la tarea (jornada-01, modal reutilizado de la
            // fase 1); click simple selecciona.
            if evento.click_count() >= 2 {
                window.dispatch_action(Box::new(AbrirTareaJornada { indice: idx }), cx);
            } else {
                window.dispatch_action(Box::new(SeleccionarTareaJornada { indice: idx }), cx);
            }
        })
}

// -------------------------------------------------------------------------------------------------
// SELECTOR DE PEER (jornada-03) — DECISIÓN de Max (2026-07-02): variantes A+B COMBINADAS en un
// solo control  ‹ [ peer ▾ ] › . El dropdown dorado central (A) salta a cualquier peer vivo; los
// chevrones (B) ciclan anterior/siguiente, espejo de `[`/`]` de la TUI. La lista desplegada se
// pinta INLINE bajo el control (no flotante): la vista es pura y así se evita pelear con el orden
// de pintado del árbol; el estado abierto/cerrado vive en `EstadoPantalla.jornada_dropdown`.
// -------------------------------------------------------------------------------------------------

/// El control combinado ‹ [peer ▾] › + (si está abierto) la lista de peers vivos debajo.
fn selector_peer_jornada(datos: &EstadoPantalla) -> AnyElement {
    let etiqueta = datos
        .jornada_peer
        .clone()
        .unwrap_or_else(|| "elegir peer…".to_string());

    // Pill central (variante A): borde línea radio 999, texto papel, chevron ▾ en brasa. Click →
    // alterna el desplegable.
    let pill = div()
        .id("jornada-peer-dropdown")
        .h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded(tema::radio(tema::RADIO_PILL))
        .border_1()
        .border_color(tema::LINEA)
        .cursor_pointer()
        .hover(|s| s.bg(tema::TINTA2).border_color(tema::BRASA))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_sm()
                .text_color(tema::PAPEL)
                .child(SharedString::from(etiqueta)),
        )
        .child(div().text_color(tema::BRASA).text_sm().child(SharedString::from("▾")))
        .on_click(|_e, window, cx| {
            window.dispatch_action(Box::new(AlternarDropdownJornada), cx);
        });

    let fila = div()
        .h_flex()
        .items_center()
        .gap_2()
        .child(chevron_peer("jornada-peer-prev", "‹", -1))
        .child(pill)
        .child(chevron_peer("jornada-peer-next", "›", 1));

    let mut raiz = div().v_flex().gap_2().items_end().child(fila);

    // Desplegable (abierto): lista de peers vivos, el actual resaltado. Click → salta y recarga.
    if datos.jornada_dropdown {
        let actual = datos.jornada_peer.as_deref();
        let mut lista = div()
            .id("jornada-peer-lista")
            .v_flex()
            .gap_1()
            .p_2()
            .max_h(px(180.0))
            .overflow_y_scroll()
            .rounded(tema::radio(tema::RADIO_CONTROL))
            .bg(tema::TINTA2)
            .border_1()
            .border_color(tema::LINEA);
        if datos.instancias.is_empty() {
            lista = lista.child(tema::texto_terciario("No hay peers vivos."));
        } else {
            for inst in &datos.instancias {
                let activo = actual == Some(inst.id.as_str());
                let id = inst.id.clone();
                lista = lista.child(
                    tema::fila_seleccionable(
                        SharedString::from(format!("jornada-peer-op-{}", inst.id)),
                        activo,
                    )
                    .child(
                        div()
                            .font_family(tema::FUENTE_MONO)
                            .text_sm()
                            .text_color(tema::PAPEL)
                            .child(SharedString::from(inst.id.clone())),
                    )
                    .on_click(move |_e, window, cx| {
                        window.dispatch_action(Box::new(ElegirPeerJornada { id: id.clone() }), cx);
                    }),
                );
            }
        }
        raiz = raiz.child(lista);
    }

    raiz.into_any_element()
}

/// Chevrón de ciclado (variante B de la decisión): botón ghost cuadrado con el glifo en brasa.
/// `delta` = -1 (anterior) / +1 (siguiente). Espejo de `[`/`]` de la TUI.
fn chevron_peer(id: &'static str, glifo: &'static str, delta: i32) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.0))
        .h(px(28.0))
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .text_color(tema::BRASA)
        .cursor_pointer()
        .hover(|s| s.bg(tema::TINTA2))
        .child(SharedString::from(glifo))
        .on_click(move |_e, window, cx| {
            window.dispatch_action(Box::new(CiclarPeerJornada { delta }), cx);
        })
}

// -------------------------------------------------------------------------------------------------
// MODALES (jornada-01/02/04) — contenido puro que `AppDesktop` monta como overlays. El detalle de
// TAREA reutiliza ÍNTEGRO el modal de la fase 1 (`tareas::render_modal_detalle_tarea`); aquí sólo
// se añade la card de TRANSICIONES (jornada-04) que lo acompaña en el mismo overlay.
// -------------------------------------------------------------------------------------------------

/// Card de transiciones de estado (jornada-04) que acompaña al modal de detalle de tarea en el
/// overlay de jornada: terminal → Reabrir; viva → Hecha (primario) + Cancelar (rojo). Las
/// destructivas (Cancelar/Reabrir) pasan por la confirmación de la fase 2 (`AppDesktop` decide).
pub fn render_acciones_tarea_jornada(t: &Tarea) -> AnyElement {
    let mut botones = div().h_flex().flex_wrap().gap_3();

    if t.estado.es_terminal() {
        botones = botones.child(boton_transicion(t, "Reabrir", EstadoTarea::Abierta, false));
    } else {
        botones = botones
            .child(boton_transicion(t, "Hecha", EstadoTarea::Hecha, true))
            .child(boton_transicion(t, "Cancelar", EstadoTarea::Cancelada, false));
    }

    div()
        .v_flex()
        .w(px(560.0))
        .gap_2()
        .p_4()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        .child(tema::eyebrow("transiciones"))
        .child(botones)
        .into_any_element()
}

/// Botón de transición del detalle de jornada. `primario` = relleno brasa (Hecha); el resto va
/// secundario. Despacha `CambiarEstadoTareaJornada`; el broker valida con `transicion_valida`.
fn boton_transicion(
    t: &Tarea,
    label: &'static str,
    estado: EstadoTarea,
    primario: bool,
) -> AnyElement {
    let tarea_id = t.id.clone();
    let id = SharedString::from(format!("jornada-transicion-{label}-{}", t.id));
    let al_click = move |_e: &gpui::ClickEvent, window: &mut gpui::Window, cx: &mut gpui::App| {
        window.dispatch_action(
            Box::new(CambiarEstadoTareaJornada {
                tarea_id: tarea_id.clone(),
                estado,
            }),
            cx,
        );
    };
    if primario {
        tema::boton_primario(id, label).on_click(al_click).into_any_element()
    } else {
        tema::boton_secundario(id, label).on_click(al_click).into_any_element()
    }
}

/// Contenido del pop-up "Detalle de sesión" (jornada-02): id de sesión, inicio/fin ABSOLUTOS (ISO
/// completo, en mono — aquí sí cabe, la tabla sólo muestra la hora), duración timbrada por el
/// broker, chip abierta/cerrada, y las TAREAS cuyo `sesion_id` coincide (correlación local sobre
/// los datos de `/jornada`, sin endpoint nuevo).
pub fn render_modal_sesion(s: &Sesion, tareas: &[Tarea]) -> AnyElement {
    let abierta = s.fin.as_deref().map(str::is_empty).unwrap_or(true);
    let chip = if abierta {
        tema::chip_estado("● abierta", gpui::rgb(0x22C55E))
    } else {
        tema::chip_estado("○ cerrada", tema::HUMO)
    };

    // Tareas contenidas en esta sesión (correlación por `sesion_id`).
    let contenidas: Vec<&Tarea> = tareas.iter().filter(|t| t.sesion_id == s.id).collect();

    let mut lista = div().v_flex().gap_1().max_h(px(180.0)).overflow_hidden();
    if contenidas.is_empty() {
        lista = lista.child(tema::texto_terciario("(ninguna tarea en esta sesión)"));
    } else {
        let mut scroll = div()
            .id("modal-sesion-tareas-scroll")
            .v_flex()
            .gap_1()
            .max_h(px(180.0))
            .overflow_y_scroll();
        for t in contenidas {
            scroll = scroll.child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_sm()
                            .text_color(tema::PAPEL)
                            .child(SharedString::from(recortar_texto(&t.descripcion, 56))),
                    )
                    .child(
                        div()
                            .font_family(tema::FUENTE_MONO)
                            .text_xs()
                            .text_color(tema::HUMO)
                            .child(SharedString::from(format!(
                                "{} / {}",
                                formatear_duracion(t.estimado_seg),
                                formatear_duracion(t.duracion_seg)
                            ))),
                    )
                    .child(tema::chip_estado(etiqueta_estado(t.estado), color_estado(t.estado))),
            );
        }
        lista = lista.child(scroll);
    }

    div()
        .v_flex()
        .w(px(560.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        // Cabecera: chip + título display + ✕.
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
                        .child(chip)
                        .child(tema::titulo("Detalle de sesión").text_size(px(18.0))),
                )
                .child(
                    div()
                        .id("modal-sesion-cerrar-x")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(tema::radio(tema::RADIO_CONTROL))
                        .text_color(tema::HUMO)
                        .cursor_pointer()
                        .hover(|s| s.bg(tema::TINTA).text_color(tema::PAPEL))
                        .child(SharedString::from("✕"))
                        .on_click(|_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarSesionJornada), cx);
                        }),
                ),
        )
        .child(campo_mono_sesion("id sesión", s.id.clone()))
        .child(campo_mono_sesion("inicio", s.inicio.clone()))
        .child(campo_mono_sesion(
            "fin",
            match &s.fin {
                Some(f) if !f.is_empty() => f.clone(),
                _ => "(abierta)".to_string(),
            },
        ))
        .child(campo_mono_sesion("duración", formatear_duracion(s.duracion_seg)))
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow(format!(
                    "tareas de esta sesión ({})",
                    tareas.iter().filter(|t| t.sesion_id == s.id).count()
                )))
                .child(lista),
        )
        .child(
            div().h_flex().gap_3().pt_2().child(
                tema::boton_secundario("modal-sesion-cerrar", "Cerrar").on_click(
                    |_e, window, cx| {
                        window.dispatch_action(Box::new(CerrarSesionJornada), cx);
                    },
                ),
            ),
        )
        .into_any_element()
}

/// Fila "eyebrow humo + valor mono papel" del detalle de sesión (ids/timestamps/duración).
fn campo_mono_sesion(etiqueta: impl Into<SharedString>, valor: String) -> impl IntoElement {
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

/// Recorta a `max` caracteres con elipsis, respetando fronteras de carácter. Copia local del
/// helper que usan las demás vistas (6 líneas; más barato que exponerlo cruzado entre vistas).
fn recortar_texto(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_string();
    }
    let recortado: String = texto.chars().take(max.saturating_sub(1)).collect();
    format!("{recortado}…")
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
