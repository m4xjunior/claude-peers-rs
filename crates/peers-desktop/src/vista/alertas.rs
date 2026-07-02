//! Pantalla Alertas — alertas vigentes del supervisor (R6). Puerto GPUI de
//! `peers-tui/src/ui/alertas.rs`, ahora vestido con el Design System "Ethos".
//!
//! INTENCIÓN: replicar 1:1 lo que muestra la TUI — una tabla `tipo/sujeto/detalle/creada` con la
//! severidad coloreada por tipo (ocioso→amarillo, atascado→naranja, ghosteo→rojo, las dos de
//! accountability→magenta), más el detalle íntegro de una alerta y la acción de descartarla. La
//! TUI es SOLO LECTURA salvo la tecla `d` (descartar); aquí esa única escritura es el botón
//! "Descartar", que hace `POST /admin/alerta-resolver` a través del `ClienteBroker`.
//!
//! REDISEÑO ETHOS (por qué): la versión previa usaba azules/grises hardcodeados (0x28283C,
//! 0x7F1D1D, 0x6B7280…) ajenos a la paleta de marca. Ahora TODA la superficie usa los tokens del
//! `tema` (tinta/papel/brasa/humo/linea): la sección va en `superficie_card`, los labels de columna
//! son `eyebrow` (mono/humo/mayúsculas), los datos usan la fuente mono del tema y la fila
//! seleccionada se marca con `fila_seleccionable` (acento brasa tenue + borde izquierdo dorado,
//! estilo lista de reproducción). Los colores de severidad (amarillo/naranja/rojo/magenta) SÍ se
//! conservan como `Rgba` propios: son semántica de dominio (gravedad), no decoración — el ojo del
//! supervisor los lee de un vistazo igual que en la TUI. El único acento de marca (brasa) queda para
//! selección y foco, con moderación.
//!
//! Decisión de diseño (por qué): igual que la Fundación evitó el componente `Sidebar` por
//! inestabilidad de API, la tabla se pinta con `div().v_flex()` de filas y el detalle se despliega
//! inline (no en un `Dialog`, que exige una `Entity` y `window.open_dialog`). Así el stub conserva
//! la firma de la Fundación —`render_alertas(&EstadoPantalla) -> impl IntoElement`, sin `cx` ni
//! estado propio— y la vista permanece PURA: sólo LEE `EstadoPantalla` y DESPACHA acciones. Toda
//! mutación (seleccionar, abrir/cerrar detalle, descartar y recargar) vive en `AppDesktop`, que es
//! quien tiene `cx` y registra los manejadores con `.on_action(cx.listener(...))`. La severidad se
//! pinta como "chip" (`tema::chip_estado`) porque el `Badge` del kit es un overlay de contador/punto,
//! no una etiqueta de texto de 5 colores fijos como necesitamos. Las acciones (Cerrar/Descartar) se
//! pintan con los botones del tema (`boton_secundario`/`boton_primario`), no con el `Button` del kit,
//! para que el look Ethos quede exacto y no dependa del tema del kit.

use gpui::{
    div, actions, rgb, Action, AnyElement, IntoElement, ParentElement, Rgba,
    SharedString, StatefulInteractiveElement, Styled,
};
use gpui_component::StyledExt;
use peers_core::{Alerta, TipoAlerta};

use crate::app::EstadoPantalla;
use crate::tema;

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
//
// NOTA para Fase 3: estas 3 acciones YA están cableadas en `AppDesktop::render` con
// `.on_action(cx.listener(...))` → `abrir_detalle_alerta`, `cerrar_detalle_alerta`,
// `descartar_alerta`. El rediseño NO cambia sus nombres ni campos, así que el cableado sigue válido.
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
// `hora_iso`). `color_alerta` devuelve `Rgba` (no `Hsla`) para encajar directo en `tema::chip_estado`
// y en `.border_color(...)` del panel. No se reusan los de la TUI porque están acoplados a ratatui;
// la lógica de mapeo es idéntica y queda documentada.
// -------------------------------------------------------------------------------------------------

/// Color de una alerta según su tipo (R6). Mapeo idéntico a la TUI: ocioso→amarillo,
/// atascado→naranja, ghosteo→rojo, y las dos de accountability→magenta (graves, destacan). Es
/// SEMÁNTICA de gravedad, no decoración: por eso se conserva la paleta de severidad y no se
/// sustituye por el acento de marca.
fn color_alerta(tipo: TipoAlerta) -> Rgba {
    match tipo {
        TipoAlerta::Ocioso => rgb(0xEAB308),              // amarillo
        TipoAlerta::Atascado => rgb(0xFF8C00),            // naranja (RGB, como en la TUI)
        TipoAlerta::Ghosteo => rgb(0xEF4444),             // rojo
        TipoAlerta::CierreSospechoso => rgb(0xD946EF),    // magenta
        TipoAlerta::CancelacionExcesiva => rgb(0xD946EF), // magenta
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
/// `pub(crate)`: lo reutiliza `AppDesktop` para la tecla `d` (descartar la alerta seleccionada).
pub(crate) fn tipo_serializado(tipo: TipoAlerta) -> &'static str {
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
const COL_TIPO: f32 = 170.0;
const COL_SUJETO: f32 = 200.0;
const COL_CREADA: f32 = 90.0;

// Rojo tenue de dominio para el banner de error del broker (offline/401/otro). No es un token del
// Ethos porque el error es una condición excepcional, no una superficie normal; se mantiene sobrio
// (fondo apenas teñido, texto rojo claro) para no romper la calma de la paleta.
const ERROR_FONDO: u32 = 0x2A_11_11_FF;
const ERROR_TEXTO: u32 = 0xF1_A8_A8;

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Punto de entrada de la pantalla (firma estable de la Fundación). Compone, dentro de una
/// `superficie_card` Ethos: cabecera (eyebrow + título + conteo), banner de error (si hubo), tabla
/// de alertas y, si `alerta_detalle` apunta a una fila, el panel de detalle inline debajo.
pub fn render_alertas(datos: &EstadoPantalla) -> impl IntoElement {
    let total = datos.alertas.len();

    // La pantalla entera vive dentro de una card elevada, con padding generoso y separación
    // amplia entre secciones (estética sobria-reverente).
    let mut tarjeta = tema::superficie_card().v_flex().size_full().gap_4().p_6();

    // Cabecera: eyebrow (label discreto) sobre título grande + conteo atenuado.
    tarjeta = tarjeta.child(
        div()
            .v_flex()
            .gap_1()
            .child(tema::eyebrow("Supervisor · R6"))
            .child(
                div()
                    .h_flex()
                    .items_end()
                    .gap_2()
                    .child(tema::titulo("Alertas"))
                    .child(
                        div()
                            .pb(gpui::px(3.0))
                            .child(tema::texto_terciario(format!("{total} vigentes"))),
                    ),
            ),
    );

    // Banner de error del broker (offline / 401 / otro) si la última operación falló. Se pinta en
    // vez de dejar la tabla muda; la lista previa se conserva igualmente debajo.
    if let Some(err) = &datos.error_alertas {
        tarjeta = tarjeta.child(banner_error(&err.to_string()));
    }

    // Cuerpo: tabla o estado vacío (mismo texto guía que la TUI cuando no hay alertas).
    if datos.alertas.is_empty() {
        tarjeta = tarjeta.child(estado_vacio());
        return tarjeta;
    }

    tarjeta = tarjeta.child(tabla(datos));

    // Panel de detalle inline (espejo del modal `Enter` de la TUI): muestra el detalle ÍNTEGRO,
    // que la celda de la tabla recorta. Sólo si el índice guardado sigue siendo válido.
    if let Some(idx) = datos.alerta_detalle {
        if let Some(a) = datos.alertas.get(idx) {
            tarjeta = tarjeta.child(panel_detalle(a, idx));
        }
    }

    tarjeta
}

/// Banner de error sobrio: fondo rojo apenas teñido, borde y texto rojo claro, radio de control.
/// Neutro respecto a la variante concreta: el `Display` de `ErrorBroker` ya da el texto correcto.
fn banner_error(texto: &str) -> AnyElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .bg(rgb(ERROR_FONDO))
        .border_1()
        .border_color(rgb(0x4A_1D_1D))
        .text_color(rgb(ERROR_TEXTO))
        .child(SharedString::from(format!("⚠  {texto}")))
        .into_any_element()
}

/// Estado vacío: mismo mensaje guía que la TUI, en humo tenue y centrado. Sin ruido cromático.
fn estado_vacio() -> AnyElement {
    div()
        .v_flex()
        .items_center()
        .justify_center()
        .size_full()
        .gap_2()
        .child(tema::texto_terciario(
            "Sin alertas vigentes.",
        ))
        .child(tema::texto_terciario(
            "El supervisor no ha detectado ocioso / atascado / ghosteo.",
        ))
        .into_any_element()
}

/// Tabla completa: fila de encabezado (eyebrows) + una fila por alerta. `div().v_flex()` por la
/// decisión de diseño documentada arriba (no `Table` del kit). Separada del encabezado por una
/// línea tenue del tema.
fn tabla(datos: &EstadoPantalla) -> AnyElement {
    let encabezado = div()
        .h_flex()
        .w_full()
        .px_3()
        .pb_2()
        .gap_2()
        .border_b_1()
        .border_color(tema::LINEA)
        .child(celda_eyebrow("tipo", COL_TIPO))
        .child(celda_eyebrow("sujeto", COL_SUJETO))
        .child(div().flex_1().child(tema::eyebrow("detalle")))
        .child(celda_eyebrow("creada", COL_CREADA));

    let mut cuerpo = div().v_flex().w_full().gap_1().pt_2();
    for (idx, a) in datos.alertas.iter().enumerate() {
        cuerpo = cuerpo.child(fila(idx, a, datos.alertas_seleccion));
    }

    div()
        .v_flex()
        .w_full()
        .child(encabezado)
        .child(cuerpo)
        .into_any_element()
}

/// Celda de encabezado de ancho fijo con un `eyebrow` (label mono/humo/mayúsculas del tema).
fn celda_eyebrow(texto: &str, ancho: f32) -> impl IntoElement {
    div().w(gpui::px(ancho)).child(tema::eyebrow(texto.to_string()))
}

/// Celda de datos de ancho fijo, con la fuente mono del tema (timestamps/sujetos alineados) y texto
/// papel. La fuente mono cae al monospace del sistema si IBM Plex Mono no está instalada.
fn celda_dato(texto: &str, ancho: f32) -> impl IntoElement {
    div()
        .w(gpui::px(ancho))
        .font_family(tema::FUENTE_MONO)
        .text_color(tema::PAPEL)
        .child(SharedString::from(texto.to_string()))
}

/// Una fila de alerta: chip de severidad (tipo), sujeto, detalle recortado y hora. Toda la fila es
/// clicable → despacha `AbrirDetalle { indice }`. Usa `tema::fila_seleccionable`: la seleccionada se
/// resalta con acento brasa tenue + borde izquierdo dorado (estilo lista de reproducción); las demás
/// dan hover a TINTA2 para señalar que son clicables. La fila del tema ya trae `.id()` interno.
fn fila(idx: usize, a: &Alerta, seleccion: usize) -> impl IntoElement {
    let seleccionada = idx == seleccion;

    tema::fila_seleccionable(format!("alerta-fila-{idx}"), seleccionada)
        // Chip de severidad como celda "tipo" (color de dominio, no de marca).
        .child(
            div()
                .w(gpui::px(COL_TIPO))
                .child(tema::chip_estado(etiqueta_alerta(a.tipo), color_alerta(a.tipo))),
        )
        .child(celda_dato(&recortar(&a.sujeto, 24), COL_SUJETO))
        // detalle: columna flexible → recorte generoso; el íntegro se ve en el panel de detalle.
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_color(tema::PAPEL)
                .child(SharedString::from(recortar(&a.detalle, 80))),
        )
        .child(celda_dato(&hora_iso(&a.creada_en), COL_CREADA))
        .on_click(move |_evento, window, cx| {
            // Despacha la acción; `AppDesktop` la maneja y muta el estado. La vista no toca `cx`.
            window.dispatch_action(Box::new(AbrirDetalle { indice: idx }), cx);
        })
}

/// Panel de detalle inline: tipo (chip coloreado), sujeto, creada, detalle ÍNTEGRO y las dos
/// acciones — Cerrar (secundario) y Descartar (primario). Espejo del modal `Enter` de la TUI,
/// desplegado bajo la tabla dentro de su propia sub-superficie (TINTA2 sobre la card) para separar
/// visualmente el foco. El borde izquierdo toma el color de severidad como acento de contexto.
fn panel_detalle(a: &Alerta, idx: usize) -> AnyElement {
    let color = color_alerta(a.tipo);
    // Datos que la acción de descartar necesita, capturados por valor para el closure.
    let tipo_str = tipo_serializado(a.tipo).to_string();
    let sujeto = a.sujeto.clone();

    div()
        .v_flex()
        .w_full()
        .gap_3()
        .p_4()
        .mt_2()
        .rounded(tema::radio(tema::RADIO_CARD))
        .bg(tema::TINTA)
        // Acento de contexto: borde izquierdo grueso con el color de la severidad de esta alerta.
        // Sólo borde izquierdo (no `border_1` completo) porque en este rev de GPUI `border_color`
        // aplica a TODOS los lados: un `border_1`+`border_color(LINEA)` seguido de
        // `border_l_4`+`border_color(color)` pintaría los 4 lados del color de severidad. Con un
        // único borde izquierdo el acento queda limpio, estilo marcador de selección.
        .border_l_4()
        .border_color(color)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_3()
                .child(tema::chip_estado(etiqueta_alerta(a.tipo), color))
                .child(tema::titulo("Detalle de alerta").text_size(gpui::px(18.0))),
        )
        .child(campo("sujeto", &a.sujeto))
        .child(campo("creada", &a.creada_en))
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("detalle"))
                // Detalle íntegro (sin recortar): el jefe lee el mensaje completo del supervisor.
                .child(tema::texto_primario(a.detalle.clone())),
        )
        .child(
            div()
                .h_flex()
                .gap_3()
                .pt_2()
                .child(
                    tema::boton_secundario("alerta-cerrar", "Cerrar").on_click(
                        move |_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarDetalleAlerta), cx);
                        },
                    ),
                )
                .child(
                    tema::boton_primario("alerta-descartar", "Descartar").on_click(
                        move |_e, window, cx| {
                            // Clona: el closure es `Fn` y puede invocarse varias veces.
                            window.dispatch_action(
                                Box::new(Descartar {
                                    tipo: tipo_str.clone(),
                                    sujeto: sujeto.clone(),
                                    indice: idx,
                                }),
                                cx,
                            );
                        },
                    ),
                ),
        )
        .into_any_element()
}

/// Fila `etiqueta: valor` del panel de detalle (sujeto/creada). Etiqueta como `eyebrow` (label
/// discreto del tema), valor en mono/papel para alinearlo con la tabla.
fn campo(etiqueta: &str, valor: &str) -> impl IntoElement {
    div()
        .h_flex()
        .items_baseline()
        .gap_3()
        .child(div().w(gpui::px(80.0)).child(tema::eyebrow(etiqueta.to_string())))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::PAPEL)
                .child(SharedString::from(valor.to_string())),
        )
}
