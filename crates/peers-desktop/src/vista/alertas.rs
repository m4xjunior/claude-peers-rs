//! Pantalla Alertas — alertas vigentes del supervisor (R6). Puerto GPUI de
//! `peers-tui/src/ui/alertas.rs`.
//!
//! INTENCIÓN: replicar 1:1 lo que muestra la TUI — una tabla `tipo/sujeto/detalle/creada` con la
//! severidad coloreada por tipo (ocioso→amarillo, atascado→naranja, ghosteo→rojo, las dos de
//! accountability→magenta), más el detalle íntegro de una alerta y la acción de descartarla. La
//! TUI es SOLO LECTURA salvo la tecla `d` (descartar); aquí esa única escritura es el botón
//! "Descartar", que hace `POST /admin/alerta-resolver` a través del `ClienteBroker`.
//!
//! Decisión de diseño (por qué): igual que la Fundación evitó el componente `Sidebar` por
//! inestabilidad de API, la tabla se pinta con `div().v_flex()` de filas y el detalle se despliega
//! inline (no en un `Dialog`, que exige una `Entity` y `window.open_dialog`). Así el stub conserva
//! la firma de la Fundación —`render_alertas(&EstadoPantalla) -> impl IntoElement`, sin `cx` ni
//! estado propio— y la vista permanece PURA: sólo LEE `EstadoPantalla` y DESPACHA acciones. Toda
//! mutación (seleccionar, abrir/cerrar detalle, descartar y recargar) vive en `AppDesktop`, que es
//! quien tiene `cx` y registra los manejadores con `.on_action(cx.listener(...))`. La severidad se
//! pinta como "chip" (`div` con fondo = color del tipo) porque el `Badge` del kit es un overlay
//! de contador/punto sobre un hijo, no una etiqueta de texto de 5 colores fijos como necesitamos.
//! El botón de acción sí es `Button` de gpui-component (variante `danger`/`ghost`).

use gpui::{
    div, actions, rgb, Action, AnyElement, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    StyledExt,
};
use peers_core::{Alerta, TipoAlerta};

use crate::app::EstadoPantalla;

// -------------------------------------------------------------------------------------------------
// ACCIONES — la vista es pura (sin `cx`), así que no cablea callbacks: DESPACHA acciones que
// `AppDesktop` maneja con `.on_action(cx.listener(...))` en su contenedor raíz. GPUI hace burbujear
// la acción por el árbol desde el elemento clicado hasta ese manejador, sin depender del foco. El
// payload lleva lo mínimo para que el manejador actúe sin volver a mirar el estado de la vista.
// Namespace `alertas` para no colisionar con acciones de otras pantallas.
//
// `#[action(namespace = alertas, no_json)]`: `no_json` evita exigir `serde::Deserialize` +
// `schemars::JsonSchema` (que la macro pide para cargar keymaps desde JSON). Estas acciones SÓLO se
// despachan por código (`window.dispatch_action`), nunca desde un keymap, así que no las
// necesitamos y así no arrastramos `schemars` como dependencia del crate. Basta `Clone + PartialEq`.
// -------------------------------------------------------------------------------------------------

// Acción sin datos: cerrar el panel de detalle desplegado (botón "Cerrar").
actions!(alertas, [CerrarDetalleAlerta]);

/// Abrir el detalle de la fila `indice` (click en la fila). Índice en la lista actual de alertas.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = alertas, no_json)]
pub struct AbrirDetalle {
    pub indice: usize,
}

/// Descartar una alerta concreta → `POST /admin/alerta-resolver`. `tipo` es la cadena serializada
/// del `TipoAlerta` (lo que el broker espera) y `sujeto` completa la clave idempotente (R7).
/// `indice` sirve para acotar la selección tras descartar sin recalcularla.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = alertas, no_json)]
pub struct Descartar {
    pub tipo: String,
    pub sujeto: String,
    pub indice: usize,
}

// -------------------------------------------------------------------------------------------------
// HELPERS PUROS — espejo de los de la TUI (`color_alerta`, `etiqueta_alerta`, `recortar`,
// `hora_iso`) pero devolviendo tipos de GPUI (`Hsla`) en vez de `ratatui::Color`. No se reusan los
// de la TUI porque están acoplados a ratatui; la lógica de mapeo es idéntica y queda documentada.
// -------------------------------------------------------------------------------------------------

/// Color de una alerta según su tipo (R6), en `Hsla`. Mapeo idéntico a la TUI: ocioso→amarillo,
/// atascado→naranja, ghosteo→rojo, y las dos de accountability→magenta (graves, destacan).
fn color_alerta(tipo: TipoAlerta) -> Hsla {
    match tipo {
        TipoAlerta::Ocioso => rgb(0xEAB308).into(),             // amarillo
        TipoAlerta::Atascado => rgb(0xFF8C00).into(),           // naranja (RGB, como en la TUI)
        TipoAlerta::Ghosteo => rgb(0xEF4444).into(),            // rojo
        TipoAlerta::CierreSospechoso => rgb(0xD946EF).into(),   // magenta
        TipoAlerta::CancelacionExcesiva => rgb(0xD946EF).into(), // magenta
    }
}

/// Etiqueta corta del tipo de alerta para el chip de severidad. Espejo de `etiqueta_alerta`.
fn etiqueta_alerta(tipo: TipoAlerta) -> &'static str {
    match tipo {
        TipoAlerta::Ocioso => "ocioso",
        TipoAlerta::Atascado => "atascado",
        TipoAlerta::Ghosteo => "ghosteo",
        TipoAlerta::CierreSospechoso => "cierre sospechoso",
        TipoAlerta::CancelacionExcesiva => "cancelación excesiva",
    }
}

/// Cadena que el broker espera en el POST (`serde(rename_all = "lowercase")` con renames explícitos
/// para las compuestas). Debe coincidir EXACTAMENTE con lo que `peers-core` serializa; se centraliza
/// aquí para no dispersar literales por la UI y no re-serializar con `serde_json` por un solo campo.
fn tipo_serializado(tipo: TipoAlerta) -> &'static str {
    match tipo {
        TipoAlerta::Ocioso => "ocioso",
        TipoAlerta::Atascado => "atascado",
        TipoAlerta::Ghosteo => "ghosteo",
        TipoAlerta::CierreSospechoso => "cierre_sospechoso",
        TipoAlerta::CancelacionExcesiva => "cancelacion_excesiva",
    }
}

/// Recorta a `max` caracteres respetando límites de char, con elipsis. Espejo de `recortar`.
fn recortar(texto: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let n = texto.chars().count();
    if n <= max {
        return texto.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let recortado: String = texto.chars().take(max - 1).collect();
    format!("{recortado}…")
}

/// Extrae `HH:MM:SS` de un ISO 8601 para la columna "creada". Espejo de `hora_iso`.
fn hora_iso(iso: &str) -> String {
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        let fin = resto.find(['.', 'Z', '+']).unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

// Anchos fijos (px) de las columnas no flexibles, espejo de los Constraint de la TUI (10/18/…/10
// chars). `detalle` es la flexible (`flex_1`).
const COL_TIPO: f32 = 150.0;
const COL_SUJETO: f32 = 200.0;
const COL_CREADA: f32 = 90.0;

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Punto de entrada de la pantalla (firma estable de la Fundación). Compone: cabecera con el
/// conteo, banner de error (si hubo), tabla de alertas y, si `alerta_detalle` apunta a una fila,
/// el panel de detalle inline debajo.
pub fn render_alertas(datos: &EstadoPantalla) -> impl IntoElement {
    let total = datos.alertas.len();

    let mut raiz = div().v_flex().size_full().gap_3().p_6();

    // Cabecera: título + conteo, espejo del título de la TUI (" Alertas (N) · … ").
    raiz = raiz.child(
        div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(div().text_xl().child(SharedString::from("Alertas")))
            .child(
                div()
                    .text_sm()
                    .opacity(0.6)
                    .child(SharedString::from(format!("({total} vigentes)"))),
            ),
    );

    // Banner de error del broker (offline / 401 / otro) si la última operación falló. Se pinta en
    // vez de dejar la tabla muda; la lista previa se conserva igualmente debajo.
    if let Some(err) = &datos.error_alertas {
        raiz = raiz.child(banner_error(&err.to_string()));
    }

    // Cuerpo: tabla o estado vacío (mismo texto guía que la TUI cuando no hay alertas).
    if datos.alertas.is_empty() {
        raiz = raiz.child(estado_vacio());
        return raiz;
    }

    raiz = raiz.child(tabla(datos));

    // Panel de detalle inline (espejo del modal `Enter` de la TUI): muestra el detalle ÍNTEGRO,
    // que la celda de la tabla recorta. Sólo si el índice guardado sigue siendo válido.
    if let Some(idx) = datos.alerta_detalle {
        if let Some(a) = datos.alertas.get(idx) {
            raiz = raiz.child(panel_detalle(a, idx));
        }
    }

    raiz
}

/// Banner rojo tenue con el motivo del fallo del broker. Neutro respecto a la variante concreta:
/// el `Display` de `ErrorBroker` ya da el texto correcto (offline/401/otro).
fn banner_error(texto: &str) -> AnyElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x7F1D1D))
        .text_color(rgb(0xFEE2E2))
        .child(SharedString::from(format!("⚠ {texto}")))
        .into_any_element()
}

/// Estado vacío: mismo mensaje guía que la TUI cuando no hay alertas vigentes.
fn estado_vacio() -> AnyElement {
    div()
        .v_flex()
        .items_center()
        .justify_center()
        .size_full()
        .text_color(rgb(0x6B7280))
        .child(SharedString::from(
            "Sin alertas vigentes. El supervisor no ha detectado ocioso/atascado/ghosteo.",
        ))
        .into_any_element()
}

/// Tabla completa: fila de encabezado + una fila por alerta. `div().v_flex()` por la decisión de
/// diseño documentada arriba (no `Table` del kit).
fn tabla(datos: &EstadoPantalla) -> AnyElement {
    let encabezado = div()
        .h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .text_color(rgb(0xEAB308))
        .child(celda("tipo", COL_TIPO))
        .child(celda("sujeto", COL_SUJETO))
        .child(div().flex_1().child(SharedString::from("detalle")))
        .child(celda("creada", COL_CREADA));

    let mut cuerpo = div().v_flex().w_full().gap_1();
    for (idx, a) in datos.alertas.iter().enumerate() {
        cuerpo = cuerpo.child(fila(idx, a, datos.alertas_seleccion));
    }

    div()
        .v_flex()
        .w_full()
        .gap_1()
        .child(encabezado)
        .child(cuerpo)
        .into_any_element()
}

/// Celda de ancho fijo con texto plano.
fn celda(texto: &str, ancho: f32) -> impl IntoElement {
    div()
        .w(gpui::px(ancho))
        .child(SharedString::from(texto.to_string()))
}

/// Chip de severidad: etiqueta del tipo sobre fondo del color del tipo. Sustituye al `Badge` del
/// kit (que es overlay de contador/punto). Texto oscuro para contraste sobre los colores claros.
fn chip_severidad(tipo: TipoAlerta) -> impl IntoElement {
    div()
        .px_2()
        .py(gpui::px(1.0))
        .rounded_md()
        .bg(color_alerta(tipo))
        .text_color(rgb(0x111111))
        .text_sm()
        .child(SharedString::from(etiqueta_alerta(tipo)))
}

/// Una fila de alerta: chip de severidad (tipo), sujeto, detalle recortado y hora. Toda la fila es
/// clicable → despacha `AbrirDetalle { indice }`. La fila seleccionada se resalta con fondo tenue.
fn fila(idx: usize, a: &Alerta, seleccion: usize) -> impl IntoElement {
    let seleccionada = idx == seleccion;

    let base = div()
        // id único por fila: requisito de GPUI para elementos interactivos.
        .id(SharedString::from(format!("alerta-fila-{idx}")))
        .h_flex()
        .w_full()
        .items_center()
        .px_2()
        .py_1()
        .gap_2()
        .rounded_md()
        .cursor_pointer();

    // Resalte de la fila seleccionada (equivalente al fondo tenue de la TUI).
    let base = if seleccionada {
        base.bg(rgb(0x28283C))
    } else {
        base
    };

    base
        // Chip de severidad como celda "tipo".
        .child(div().w(gpui::px(COL_TIPO)).child(chip_severidad(a.tipo)))
        .child(celda(&recortar(&a.sujeto, 24), COL_SUJETO))
        // detalle: columna flexible → recorte generoso; el íntegro se ve en el panel de detalle.
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .child(SharedString::from(recortar(&a.detalle, 80))),
        )
        .child(celda(&hora_iso(&a.creada_en), COL_CREADA))
        .on_click(move |_evento, window, cx| {
            // Despacha la acción; `AppDesktop` la maneja y muta el estado. La vista no toca `cx`.
            window.dispatch_action(Box::new(AbrirDetalle { indice: idx }), cx);
        })
}

/// Panel de detalle inline: tipo (chip coloreado), sujeto, creada, detalle ÍNTEGRO y las dos
/// acciones — Cerrar y Descartar. Espejo del modal `Enter` de la TUI, desplegado bajo la tabla.
fn panel_detalle(a: &Alerta, idx: usize) -> AnyElement {
    let color = color_alerta(a.tipo);
    // Datos que la acción de descartar necesita, capturados por valor para el closure.
    let tipo_str = tipo_serializado(a.tipo).to_string();
    let sujeto = a.sujeto.clone();

    div()
        .v_flex()
        .w_full()
        .gap_2()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(color)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(chip_severidad(a.tipo))
                .child(div().child(SharedString::from("Detalle de alerta"))),
        )
        .child(campo("sujeto", &a.sujeto))
        .child(campo("creada", &a.creada_en))
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(div().opacity(0.6).child(SharedString::from("detalle")))
                // Detalle íntegro (sin recortar): el jefe lee el mensaje completo del supervisor.
                .child(div().child(SharedString::from(a.detalle.clone()))),
        )
        .child(
            div()
                .h_flex()
                .gap_2()
                .pt_2()
                .child(
                    Button::new("alerta-cerrar")
                        .ghost()
                        .label("Cerrar")
                        .on_click(move |_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarDetalleAlerta), cx);
                        }),
                )
                .child(
                    Button::new("alerta-descartar")
                        .danger()
                        .label("Descartar")
                        .on_click(move |_e, window, cx| {
                            // Clona: el closure es `Fn` y puede invocarse varias veces.
                            window.dispatch_action(
                                Box::new(Descartar {
                                    tipo: tipo_str.clone(),
                                    sujeto: sujeto.clone(),
                                    indice: idx,
                                }),
                                cx,
                            );
                        }),
                ),
        )
        .into_any_element()
}

/// Fila `etiqueta: valor` del panel de detalle (sujeto/creada). Etiqueta atenuada, valor normal.
fn campo(etiqueta: &str, valor: &str) -> impl IntoElement {
    div()
        .h_flex()
        .gap_2()
        .child(
            div()
                .w(gpui::px(70.0))
                .opacity(0.6)
                .child(SharedString::from(format!("{etiqueta}:"))),
        )
        .child(div().child(SharedString::from(valor.to_string())))
}
