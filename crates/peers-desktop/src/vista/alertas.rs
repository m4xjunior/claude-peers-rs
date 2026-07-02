//! Pantalla Alertas — alertas vigentes del supervisor (R6). Puerto GPUI de
//! `peers-tui/src/ui/alertas.rs`, vestido con el Design System "Ethos" y ahora OPERABLE
//! (features ALTA de la RFC `.specs/desktop/alertas/RFC-alertas.md`).
//!
//! INTENCIÓN: replicar 1:1 lo que muestra la TUI — una tabla `tipo/sujeto/detalle/creada` con la
//! severidad coloreada por tipo (ocioso→amarillo, atascado→naranja, ghosteo→rojo, las dos de
//! accountability→magenta) — y convertirla en un centro de mando: ver el detalle en un modal
//! (Dialog con foco y Esc), triar por tipo (chips multi-select), leer la antigüedad relativa de
//! cada alerta ("hace 12 min"), descartar con confirmación + toast, y saltar al sujeto de la
//! alerta (portando la tecla `g` de la TUI). La única escritura hacia el broker sigue siendo
//! `POST /admin/alerta-resolver` (idempotente), disparada por el botón/atajo "Descartar".
//!
//! REDISEÑO ETHOS (por qué): TODA la superficie usa los tokens del `tema` (tinta/papel/brasa/
//! humo/linea). Los colores de severidad (amarillo/naranja/rojo/magenta) SÍ se conservan como
//! `Rgba` propios: son semántica de dominio (gravedad), no decoración — el ojo del supervisor los
//! lee de un vistazo igual que en la TUI. El acento de marca (brasa) queda para selección, foco y
//! filtros activos, con moderación.
//!
//! DECISIÓN DE DISEÑO (features RFC portadas aquí):
//!   - alertas-01 (modal): el detalle deja de ser inline y pasa a un `Modal` centrado (overlay
//!     con backdrop, `Esc` cierra, botón ✕ y clic fuera). Se monta en el render RAÍZ de
//!     `AppDesktop` leyendo `datos.alerta_detalle` (patrón idéntico al que ya usaba inline). La
//!     vista sigue PURA: expone `render_modal_detalle(&Alerta, idx)` que el app.rs mete en el
//!     `Modal`; la vista no crea `Entity` ni toca `cx`.
//!   - alertas-02 (ir al sujeto): botón + chip del sujeto clicable que DESPACHA `IrAlSujeto`.
//!     `AppDesktop` mapea `tipo → pantalla` (igual que la tecla `g` de la TUI) y preselecciona.
//!   - alertas-03 (filtro por tipo): chips-toggle multi-select en la cabecera que DESPACHAN
//!     `AlternarFiltroTipo`. El filtro vive en `EstadoPantalla.alertas_filtro_tipos`; el conteo y
//!     la tabla se recalculan con las alertas visibles.
//!   - alertas-07 (antigüedad relativa): badge "hace Xm" calculado localmente parseando
//!     `creada_en` (ISO 8601) contra `OffsetDateTime::now_utc()` (crate `time`, ya en el
//!     workspace). Color por rango (fresco humo → viejo brasa/rojo).
//!   - alertas-09 (descarte con confirmación + toast): el modal pide confirmación en 2 pasos
//!     (`Descartar` → `¿Seguro? [Sí, descartar]`) vía `alerta_confirmar_descarte`; el app.rs
//!     emite un `Notification` (toast) de éxito/fallo con `WindowExt::push_notification`.
//!   - alertas-10 (descarte rápido en fila): botón fantasma que aparece al hover al final de la
//!     fila y DESPACHA `Descartar` directamente, sin abrir el modal (espejo de la tecla `d`).
//!
//! Toda mutación (seleccionar, abrir/cerrar modal, filtrar, confirmar, descartar+recargar,
//! navegar al sujeto) vive en `AppDesktop`, que tiene `cx` y registra los manejadores con
//! `.on_action(cx.listener(...))`. La severidad se pinta como "chip" (`tema::chip_estado`).

use gpui::{
    div, px, rgb, Action, AnyElement, InteractiveElement, IntoElement, ParentElement, Rgba,
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
// NOTA de cableado (app.rs): `AbrirDetalle`, `CerrarDetalleAlerta` y `Descartar` YA estaban
// cableadas; las nuevas (`IrAlSujeto`, `AlternarFiltroTipo`, `LimpiarFiltroTipos`,
// `PedirConfirmacionDescarte`, `CancelarConfirmacionDescarte`) se cablean en esta misma tanda.
// -------------------------------------------------------------------------------------------------

// Acción sin datos: cerrar el modal de detalle (botón "Cerrar", ✕, clic fuera o `Esc`).
gpui::actions!(alertas, [CerrarDetalleAlerta]);

// Acción sin datos: limpiar TODOS los filtros de tipo (chip "Todos" / botón "limpiar").
gpui::actions!(alertas, [LimpiarFiltroTipos]);

// Acción sin datos: pedir confirmación de descarte DENTRO del modal (paso 1 → paso 2 de alertas-09).
gpui::actions!(alertas, [PedirConfirmacionDescarte]);

// Acción sin datos: cancelar la confirmación de descarte y volver al modal normal.
gpui::actions!(alertas, [CancelarConfirmacionDescarte]);

/// Abrir el detalle de la fila `indice` (click en la fila) → se muestra en el `Modal` (alertas-01).
/// Índice en la lista VISIBLE de alertas (tras aplicar el filtro por tipo).
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

/// "Ir al sujeto" (alertas-02, portando la tecla `g` de la TUI). `AppDesktop` mapea el `tipo` a la
/// pantalla natural del sujeto (Ocioso/CancelaciónExcesiva→Peers, Atascado/CierreSospechoso→Tareas,
/// Ghosteo→Trazabilidad) y preselecciona/filtra por `sujeto`. `tipo` es la cadena serializada (la
/// misma que espera el broker) para no arrastrar `TipoAlerta` en el payload de la acción.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = alertas, no_json)]
pub struct IrAlSujeto {
    pub tipo: String,
    pub sujeto: String,
}

/// Alternar el filtro por un `TipoAlerta` (alertas-03). `tipo` es la cadena serializada; el
/// manejador la mete/saca del conjunto `alertas_filtro_tipos`. Multi-select: varios tipos activos
/// a la vez suman (unión). Conjunto vacío = mostrar TODAS (sin filtro).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = alertas, no_json)]
pub struct AlternarFiltroTipo {
    pub tipo: String,
}

// -------------------------------------------------------------------------------------------------
// HELPERS PUROS — espejo de los de la TUI (`color_alerta`, `etiqueta_alerta`, `recortar`,
// `hora_iso`) + los nuevos de esta RFC (`antiguedad_relativa`, `color_antiguedad`,
// `tipo_desde_serializado`, `alertas_visibles`).
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

/// Inversa de `tipo_serializado`: de la cadena del broker al enum. `pub(crate)` porque `AppDesktop`
/// la necesita al manejar `AlternarFiltroTipo`/`IrAlSujeto` (que llevan el tipo como `String` en el
/// payload). Devuelve `Option` para no hacer `.unwrap()`; una cadena desconocida se ignora.
pub(crate) fn tipo_desde_serializado(s: &str) -> Option<TipoAlerta> {
    match s {
        "ocioso" => Some(TipoAlerta::Ocioso),
        "atascado" => Some(TipoAlerta::Atascado),
        "ghosteo" => Some(TipoAlerta::Ghosteo),
        "cierre_sospechoso" => Some(TipoAlerta::CierreSospechoso),
        "cancelacion_excesiva" => Some(TipoAlerta::CancelacionExcesiva),
        _ => None,
    }
}

/// Los 5 tipos de alerta en orden de gravedad ascendente, para pintar los chips de filtro
/// (alertas-03) y los stat-chips. Fuente única para no repetir la lista.
pub(crate) const TIPOS_ORDEN: [TipoAlerta; 5] = [
    TipoAlerta::Ocioso,
    TipoAlerta::Atascado,
    TipoAlerta::Ghosteo,
    TipoAlerta::CierreSospechoso,
    TipoAlerta::CancelacionExcesiva,
];

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

/// Antigüedad relativa de la alerta (alertas-07): parsea `creada_en` (ISO 8601, timbrado por el
/// broker en UTC) y devuelve algo como "hace 12m", "hace 2h", "hace 3d". Cálculo LOCAL con el crate
/// `time` (ya en el workspace). Si no puede parsear el timestamp o el reloj va "hacia atrás",
/// devuelve `"—"` (nunca hace panic ni asume). `pub(crate)` para que el modal del app la reutilice.
///
/// INTENCIÓN (por qué RFC3339 y no un parser a mano): el broker timbra con formato RFC 3339
/// (`2026-06-29T15:00:00Z`), que `time` sabe parsear directo; evitamos aritmética de fechas manual.
pub(crate) fn antiguedad_relativa(creada_en: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    let Ok(creada) = OffsetDateTime::parse(creada_en, &Rfc3339) else {
        return "—".to_string();
    };
    let ahora = OffsetDateTime::now_utc();
    let delta = ahora - creada;
    let segundos = delta.whole_seconds();
    if segundos < 0 {
        // Reloj de la desktop por detrás del broker (o alerta "del futuro"): no inventamos.
        return "recién".to_string();
    }
    if segundos < 60 {
        return "hace <1m".to_string();
    }
    let minutos = segundos / 60;
    if minutos < 60 {
        return format!("hace {minutos}m");
    }
    let horas = minutos / 60;
    if horas < 24 {
        return format!("hace {horas}h");
    }
    let dias = horas / 24;
    format!("hace {dias}d")
}

/// Color del badge de antigüedad por rango (alertas-07): fresco = humo tenue, >30m = brasa,
/// >2h = rojo. Escala visual para priorizar de un vistazo. Parsea igual que `antiguedad_relativa`;
/// si no puede, cae a humo (neutro).
fn color_antiguedad(creada_en: &str) -> Rgba {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    let Ok(creada) = OffsetDateTime::parse(creada_en, &Rfc3339) else {
        return tema::HUMO;
    };
    let minutos = (OffsetDateTime::now_utc() - creada).whole_minutes().max(0);
    if minutos >= 120 {
        rgb(0xEF4444) // rojo: lleva viva >2h
    } else if minutos >= 30 {
        tema::BRASA // brasa: >30m
    } else {
        tema::HUMO // fresca
    }
}

/// Indica si una alerta pasa el filtro por tipo (alertas-03). El filtro es la lista de tipos
/// serializados activos; vacío = pasa todo. `pub(crate)` porque `AppDesktop` la usa para mapear el
/// índice VISIBLE (el de la fila clicada) al índice REAL en `datos.alertas` antes de mutar.
pub(crate) fn pasa_filtro(a: &Alerta, filtro: &[String]) -> bool {
    filtro.is_empty() || filtro.iter().any(|t| tipo_serializado(a.tipo) == t)
}

// Anchos fijos (px) de las columnas no flexibles, espejo de los Constraint de la TUI (10/18/…/10
// chars). `detalle` es la flexible (`flex_1`).
const COL_TIPO: f32 = 170.0;
const COL_SUJETO: f32 = 200.0;
const COL_CREADA: f32 = 90.0;
const COL_ANTIGUEDAD: f32 = 96.0;

// Rojo tenue de dominio para el banner de error del broker (offline/401/otro). No es un token del
// Ethos porque el error es una condición excepcional, no una superficie normal; se mantiene sobrio
// (fondo apenas teñido, texto rojo claro) para no romper la calma de la paleta.
const ERROR_FONDO: u32 = 0x2A_11_11_FF;
const ERROR_TEXTO: u32 = 0xF1_A8_A8;

// -------------------------------------------------------------------------------------------------
// RENDER
// -------------------------------------------------------------------------------------------------

/// Punto de entrada de la pantalla (firma estable de la Fundación). Compone, dentro de una
/// `superficie_card` Ethos: cabecera (eyebrow + título + conteo), barra de filtros por tipo
/// (alertas-03), banner de error (si hubo) y tabla de alertas VISIBLES (tras filtrar). El DETALLE
/// ya NO se pinta aquí: vive en un `Modal` montado por `AppDesktop` (alertas-01).
pub fn render_alertas(datos: &EstadoPantalla) -> impl IntoElement {
    // Alertas visibles tras aplicar el filtro por tipo. Se materializa aquí (índices estables para
    // el pintado); `AppDesktop` reconstruye el mismo filtrado para resolver el índice real al clicar.
    let visibles: Vec<(usize, &Alerta)> = datos
        .alertas
        .iter()
        .enumerate()
        .filter(|(_, a)| pasa_filtro(a, &datos.alertas_filtro_tipos))
        .collect();
    let total = datos.alertas.len();
    let mostradas = visibles.len();

    // La pantalla entera vive dentro de una card elevada, con padding generoso y separación
    // amplia entre secciones (estética sobria-reverente).
    let mut tarjeta = tema::superficie_card().v_flex().size_full().gap_4().p_6();

    // Cabecera: eyebrow (label discreto) sobre título grande + conteo atenuado. El conteo refleja
    // las VISIBLES / TOTAL cuando hay filtro activo, para que el jefe sepa que está viendo un subset.
    let texto_conteo = if datos.alertas_filtro_tipos.is_empty() {
        format!("{total} vigentes")
    } else {
        format!("{mostradas} de {total} vigentes")
    };
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
                            .pb(px(3.0))
                            .child(tema::texto_terciario(texto_conteo)),
                    ),
            ),
    );

    // Barra de filtros por tipo (alertas-03): chips-toggle multi-select. Sólo tiene sentido si hay
    // alertas; con lista vacía se omite para no dejar controles sin datos.
    if total > 0 {
        tarjeta = tarjeta.child(barra_filtros(datos));
    }

    // Banner de error del broker (offline / 401 / otro) si la última operación falló. Se pinta en
    // vez de dejar la tabla muda; la lista previa se conserva igualmente debajo.
    if let Some(err) = &datos.error_alertas {
        tarjeta = tarjeta.child(banner_error(&err.to_string()));
    }

    // Cuerpo: tabla o estado vacío. Distinguimos "no hay alertas" de "el filtro las ocultó todas".
    if total == 0 {
        tarjeta = tarjeta.child(estado_vacio(
            "Sin alertas vigentes.",
            "El supervisor no ha detectado ocioso / atascado / ghosteo.",
        ));
        return tarjeta;
    }
    if mostradas == 0 {
        tarjeta = tarjeta.child(estado_vacio(
            "Ningún resultado con el filtro activo.",
            "Quita filtros de tipo para ver todas las alertas vigentes.",
        ));
        return tarjeta;
    }

    tarjeta = tarjeta.child(tabla(datos, &visibles));
    tarjeta
}

/// Barra de filtros por tipo (alertas-03): un chip "Todos" (limpia el filtro) + un chip-toggle por
/// cada `TipoAlerta`. Activo = fondo brasa tenue + borde brasa; inactivo = borde línea/humo. Cada
/// chip lleva su conteo de alertas de ese tipo (contexto ejecutivo ligero). Multi-select: pulsar
/// varios los suma. Cada chip DESPACHA su acción (`AlternarFiltroTipo` / `LimpiarFiltroTipos`).
fn barra_filtros(datos: &EstadoPantalla) -> AnyElement {
    let filtro = &datos.alertas_filtro_tipos;
    let sin_filtro = filtro.is_empty();

    let mut barra = div().h_flex().flex_wrap().w_full().gap_2().items_center();

    // Chip "Todos": activo cuando NO hay filtro (equivale a "mostrar todo").
    barra = barra.child(chip_filtro("Todos", None, sin_filtro, 0, true));

    for tipo in TIPOS_ORDEN {
        let serial = tipo_serializado(tipo);
        let activo = filtro.iter().any(|t| t == serial);
        let conteo = datos.alertas.iter().filter(|a| a.tipo == tipo).count();
        barra = barra.child(chip_filtro(
            etiqueta_alerta(tipo),
            Some(color_alerta(tipo)),
            activo,
            conteo,
            false,
        ));
    }

    barra.into_any_element()
}

/// Un chip-toggle de filtro. `punto` = color de severidad (dot) o `None` para el chip "Todos".
/// `es_todos` decide qué acción despacha (limpiar vs alternar el tipo). El texto lleva el conteo.
fn chip_filtro(
    etiqueta: &str,
    punto: Option<Rgba>,
    activo: bool,
    conteo: usize,
    es_todos: bool,
) -> impl IntoElement {
    let id = format!("filtro-{}", etiqueta.replace(' ', "-"));
    // El propio tipo serializado se recupera de la etiqueta vía el mapa inverso al despachar; para
    // no reconstruirlo, capturamos aquí la etiqueta serializada correcta buscándola en el orden.
    let serial: Option<String> = TIPOS_ORDEN
        .iter()
        .find(|t| etiqueta_alerta(**t) == etiqueta)
        .map(|t| tipo_serializado(*t).to_string());

    let etiqueta_txt = if es_todos {
        etiqueta.to_string()
    } else {
        format!("{etiqueta} · {conteo}")
    };

    let mut chip = div()
        .id(gpui::ElementId::Name(id.into()))
        .h_flex()
        .items_center()
        .gap_2()
        .px_3()
        .py(px(4.0))
        .rounded(px(tema::RADIO_PILL))
        .border_1()
        .cursor_pointer()
        .text_xs()
        .font_family(tema::FUENTE_MONO);

    // Punto de color de severidad (no en "Todos").
    if let Some(c) = punto {
        chip = chip.child(
            div()
                .w(px(7.0))
                .h(px(7.0))
                .rounded(px(999.0))
                .bg(c),
        );
    }

    let chip = chip.child(SharedString::from(etiqueta_txt.to_uppercase()));

    // Estado visual: activo = brasa tenue + borde brasa + texto papel; inactivo = borde línea + humo.
    let chip = if activo {
        chip.bg(tema::BRASA_TENUE)
            .border_color(tema::BRASA)
            .text_color(tema::PAPEL)
    } else {
        chip.border_color(tema::LINEA)
            .text_color(tema::HUMO)
            .hover(|s| s.bg(tema::TINTA2))
    };

    chip.on_click(move |_evento, window, cx| {
        if es_todos {
            window.dispatch_action(Box::new(LimpiarFiltroTipos), cx);
        } else if let Some(s) = &serial {
            window.dispatch_action(Box::new(AlternarFiltroTipo { tipo: s.clone() }), cx);
        }
    })
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

/// Estado vacío: mensaje guía en humo tenue y centrado. Sin ruido cromático. Parametrizado para
/// distinguir "no hay alertas" de "el filtro las ocultó todas".
fn estado_vacio(titulo: &str, guia: &str) -> AnyElement {
    div()
        .v_flex()
        .items_center()
        .justify_center()
        .size_full()
        .gap_2()
        .child(tema::texto_terciario(titulo.to_string()))
        .child(tema::texto_terciario(guia.to_string()))
        .into_any_element()
}

/// Tabla completa: fila de encabezado (eyebrows) + una fila por alerta VISIBLE. `div().v_flex()` por
/// la decisión de diseño documentada (no `Table` del kit). Recibe `visibles` (índice real + alerta)
/// para que cada fila despache acciones con el índice VISIBLE pero muestre los datos correctos.
fn tabla(datos: &EstadoPantalla, visibles: &[(usize, &Alerta)]) -> AnyElement {
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
        .child(celda_eyebrow("creada", COL_CREADA))
        .child(celda_eyebrow("antigüedad", COL_ANTIGUEDAD))
        // Columna reservada para la acción rápida de descarte (alertas-10); label vacío.
        .child(div().w(px(40.0)));

    // Cuerpo SCROLLABLE: con muchas alertas (p.ej. 50 vigentes) las filas desbordaban la ventana y
    // no se podían ver todas. `flex_1` + `overflow_y_scroll` hace que el cuerpo ocupe el alto
    // disponible bajo el encabezado (que queda FIJO) y las filas scrollen dentro. `min_h(0)` deja
    // que el flex encoja el cuerpo en vez de empujar el layout (requisito para que el scroll aplique).
    let mut cuerpo = div()
        .id("alertas-cuerpo-scroll")
        .v_flex()
        .w_full()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .gap_1()
        .pt_2();
    // `pos_visible` es el índice en la lista VISIBLE (el que viajan las acciones); `a` la alerta.
    for (pos_visible, (_idx_real, a)) in visibles.iter().enumerate() {
        cuerpo = cuerpo.child(fila(pos_visible, a, datos.alertas_seleccion));
    }

    // El contenedor de la tabla ocupa todo el alto (size_full/flex) para que el cuerpo tenga un
    // límite contra el cual scrollear; el encabezado fijo arriba, el cuerpo scrollable debajo.
    div()
        .v_flex()
        .w_full()
        .flex_1()
        .min_h_0()
        .child(encabezado)
        .child(cuerpo)
        .into_any_element()
}

/// Celda de encabezado de ancho fijo con un `eyebrow` (label mono/humo/mayúsculas del tema).
fn celda_eyebrow(texto: &str, ancho: f32) -> impl IntoElement {
    div().w(px(ancho)).child(tema::eyebrow(texto.to_string()))
}

/// Celda de datos de ancho fijo, con la fuente mono del tema (timestamps/sujetos alineados) y texto
/// papel. La fuente mono cae al monospace del sistema si IBM Plex Mono no está instalada.
fn celda_dato(texto: &str, ancho: f32) -> impl IntoElement {
    div()
        .w(px(ancho))
        .font_family(tema::FUENTE_MONO)
        .text_color(tema::PAPEL)
        .child(SharedString::from(texto.to_string()))
}

/// Badge de antigüedad relativa (alertas-07): pill con "hace Xm" en el color del rango (fresco humo
/// → viejo brasa/rojo). El color viaja como borde+texto (no relleno) para no competir con el chip de
/// severidad. Se pinta en su propia columna de ancho fijo.
fn badge_antiguedad(creada_en: &str) -> impl IntoElement {
    let color = color_antiguedad(creada_en);
    div().w(px(COL_ANTIGUEDAD)).child(
        div()
            .h_flex()
            .items_center()
            .px_2()
            .py(px(1.0))
            .rounded(px(999.0))
            .border_1()
            .border_color(color)
            .text_color(color)
            .text_xs()
            .font_family(tema::FUENTE_MONO)
            .child(SharedString::from(antiguedad_relativa(creada_en))),
    )
}

/// Una fila de alerta: chip de severidad (tipo), sujeto, detalle recortado, hora, badge de
/// antigüedad y un botón fantasma de descarte rápido (alertas-10) que aparece al hover. Toda la fila
/// (salvo el botón de descarte) es clicable → despacha `AbrirDetalle { indice }` (abre el modal).
/// Usa `tema::fila_seleccionable`: la seleccionada se resalta con acento brasa tenue + borde
/// izquierdo dorado; las demás dan hover a TINTA2. `pos` es el índice en la lista VISIBLE.
fn fila(pos: usize, a: &Alerta, seleccion: usize) -> impl IntoElement {
    let seleccionada = pos == seleccion;
    // Datos que la acción de descarte rápido necesita, capturados por valor para el closure.
    let tipo_str = tipo_serializado(a.tipo).to_string();
    let sujeto = a.sujeto.clone();

    tema::fila_seleccionable(format!("alerta-fila-{pos}"), seleccionada)
        // Chip de severidad como celda "tipo" (color de dominio, no de marca).
        .child(
            div()
                .w(px(COL_TIPO))
                .child(tema::chip_estado(etiqueta_alerta(a.tipo), color_alerta(a.tipo))),
        )
        .child(celda_dato(&recortar(&a.sujeto, 24), COL_SUJETO))
        // detalle: columna flexible → recorte generoso; el íntegro se ve en el modal de detalle.
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_color(tema::PAPEL)
                .child(SharedString::from(recortar(&a.detalle, 72))),
        )
        .child(celda_dato(&hora_iso(&a.creada_en), COL_CREADA))
        .child(badge_antiguedad(&a.creada_en))
        // Acción rápida de descarte (alertas-10): botón fantasma que DESPACHA `Descartar` sin abrir
        // el modal. `.occlude()` hace que el ratón NO atraviese este botón hacia el `on_click` de la
        // fila (que abriría el detalle): así el clic aquí sólo descarta. El descarte real lo hace
        // `AppDesktop` (POST + toast + recarga).
        .child(
            div()
                .id(gpui::ElementId::Name(format!("descartar-rapido-{pos}").into()))
                .occlude()
                .w(px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(tema::RADIO_CONTROL))
                .text_color(tema::HUMO)
                .cursor_pointer()
                .hover(|s| s.bg(tema::TINTA2).text_color(tema::BRASA))
                .child(SharedString::from("✕"))
                .on_click(move |_evento, window, cx| {
                    window.dispatch_action(
                        Box::new(Descartar {
                            tipo: tipo_str.clone(),
                            sujeto: sujeto.clone(),
                            indice: pos,
                        }),
                        cx,
                    );
                }),
        )
        .on_click(move |_evento, window, cx| {
            // Despacha la acción; `AppDesktop` la maneja y abre el modal. La vista no toca `cx`.
            window.dispatch_action(Box::new(AbrirDetalle { indice: pos }), cx);
        })
}

// -------------------------------------------------------------------------------------------------
// MODAL DE DETALLE (alertas-01) — la vista expone el CONTENIDO del modal como función pura; el
// overlay/backdrop/Esc los aporta el `Modal` del kit, montado por `AppDesktop` leyendo
// `datos.alerta_detalle`. Así la vista sigue pura (sin `Entity`, sin `cx`) y el foco/cierre por
// teclado los gestiona el kit. `confirmando` conmuta la barra de acciones al modo de confirmación
// de descarte en 2 pasos (alertas-09).
// -------------------------------------------------------------------------------------------------

/// Contenido del modal de detalle de una alerta (alertas-01). Devuelve el árbol que `AppDesktop`
/// mete dentro del `Modal` del kit. Muestra: chip de severidad + título, sujeto (clicable → ir al
/// sujeto, alertas-02), creada + antigüedad relativa (alertas-07), detalle íntegro con scroll, y la
/// barra de acciones (Cerrar / Ir al sujeto / Descartar). Si `confirmando` es true, la barra pasa al
/// paso 2 de la confirmación de descarte (alertas-09).
///
/// `pub` para que `AppDesktop` lo llame desde su `render` al montar el `Modal`. `idx` es el índice
/// VISIBLE (el mismo que usan las acciones de fila) para descartar la alerta correcta.
pub fn render_modal_detalle(a: &Alerta, idx: usize, confirmando: bool) -> AnyElement {
    let color = color_alerta(a.tipo);
    let tipo_str = tipo_serializado(a.tipo).to_string();
    let sujeto = a.sujeto.clone();

    div()
        .v_flex()
        .w(px(560.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        // Acento de contexto: borde izquierdo grueso con el color de la severidad de esta alerta.
        .border_l_4()
        .border_color(color)
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
                        .child(tema::chip_estado(etiqueta_alerta(a.tipo), color))
                        .child(tema::titulo("Detalle de alerta").text_size(px(18.0))),
                )
                // Botón ✕ de cierre (alertas-01): despacha `CerrarDetalleAlerta`.
                .child(
                    div()
                        .id("modal-alerta-cerrar-x")
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
                            window.dispatch_action(Box::new(CerrarDetalleAlerta), cx);
                        }),
                ),
        )
        // Sujeto clicable (alertas-02, variante "chip clicable"): navega a la pantalla del sujeto.
        .child(campo_sujeto(&sujeto, &tipo_str))
        // creada + antigüedad relativa (alertas-07) en la misma fila.
        .child(
            div()
                .h_flex()
                .items_baseline()
                .gap_3()
                .child(div().w(px(80.0)).child(tema::eyebrow("creada")))
                .child(
                    div()
                        .font_family(tema::FUENTE_MONO)
                        .text_color(tema::PAPEL)
                        .child(SharedString::from(a.creada_en.clone())),
                )
                .child(
                    div()
                        .text_color(color_antiguedad(&a.creada_en))
                        .text_sm()
                        .font_family(tema::FUENTE_MONO)
                        .child(SharedString::from(format!("· {}", antiguedad_relativa(&a.creada_en)))),
                ),
        )
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("detalle"))
                // Detalle íntegro con scroll si es largo (el modal no crece sin límite).
                .child(
                    div()
                        .id("modal-alerta-detalle-scroll")
                        .max_h(px(220.0))
                        .overflow_y_scroll()
                        .child(tema::texto_primario(a.detalle.clone())),
                ),
        )
        // Barra de acciones: modo normal vs modo confirmación de descarte (alertas-09).
        .child(barra_acciones_modal(idx, &tipo_str, &sujeto, confirmando))
        .into_any_element()
}

/// Fila "sujeto: <valor>" del modal, con el valor CLICABLE (alertas-02): subrayado brasa al hover,
/// despacha `IrAlSujeto`. El resto (etiqueta) queda discreto. Es la variante "chip clicable del
/// sujeto" de la RFC, complementaria al botón "Ir al sujeto" de la barra.
fn campo_sujeto(sujeto: &str, tipo_str: &str) -> impl IntoElement {
    let sujeto_owned = sujeto.to_string();
    let tipo_owned = tipo_str.to_string();
    div()
        .h_flex()
        .items_baseline()
        .gap_3()
        .child(div().w(px(80.0)).child(tema::eyebrow("sujeto")))
        .child(
            div()
                .id("modal-alerta-sujeto")
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::BRASA)
                .cursor_pointer()
                .hover(|s| s.underline())
                .child(SharedString::from(sujeto.to_string()))
                .on_click(move |_e, window, cx| {
                    window.dispatch_action(
                        Box::new(IrAlSujeto {
                            tipo: tipo_owned.clone(),
                            sujeto: sujeto_owned.clone(),
                        }),
                        cx,
                    );
                }),
        )
}

/// Barra de acciones del modal. En modo NORMAL: `Cerrar` (secundario) · `Ir al sujeto` (secundario,
/// alertas-02) · `Descartar` (primario → pide confirmación, alertas-09). En modo CONFIRMANDO: un
/// aviso + `Cancelar` · `Sí, descartar` (rojo). El descarte real lo dispara `Descartar`, que
/// `AppDesktop` cablea a `POST /admin/alerta-resolver` + toast + recarga.
fn barra_acciones_modal(idx: usize, tipo_str: &str, sujeto: &str, confirmando: bool) -> AnyElement {
    if confirmando {
        let tipo_owned = tipo_str.to_string();
        let sujeto_owned = sujeto.to_string();
        return div()
            .v_flex()
            .gap_2()
            .pt_2()
            .child(
                tema::texto_terciario(format!(
                    "¿Descartar la alerta de «{}»? Esta acción resuelve la alerta en el broker.",
                    recortar(sujeto, 40)
                )),
            )
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        tema::boton_secundario("alerta-cancelar-descarte", "Cancelar").on_click(
                            |_e, window, cx| {
                                window.dispatch_action(Box::new(CancelarConfirmacionDescarte), cx);
                            },
                        ),
                    )
                    .child(
                        // Botón de confirmación en rojo tenue: acción destructiva confirmada.
                        div()
                            .id("alerta-confirmar-descarte")
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_4()
                            .py_2()
                            .rounded(px(tema::RADIO_CONTROL))
                            .bg(rgb(0xC0_4A_3E))
                            .text_color(tema::PAPEL)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xD0_5A_4E)))
                            .child(SharedString::from("Sí, descartar"))
                            .on_click(move |_e, window, cx| {
                                window.dispatch_action(
                                    Box::new(Descartar {
                                        tipo: tipo_owned.clone(),
                                        sujeto: sujeto_owned.clone(),
                                        indice: idx,
                                    }),
                                    cx,
                                );
                            }),
                    ),
            )
            .into_any_element();
    }

    let tipo_ir = tipo_str.to_string();
    let sujeto_ir = sujeto.to_string();
    div()
        .h_flex()
        .gap_3()
        .pt_2()
        .child(
            tema::boton_secundario("alerta-cerrar", "Cerrar").on_click(move |_e, window, cx| {
                window.dispatch_action(Box::new(CerrarDetalleAlerta), cx);
            }),
        )
        .child(
            tema::boton_secundario("alerta-ir-sujeto", "Ir al sujeto →").on_click(
                move |_e, window, cx| {
                    window.dispatch_action(
                        Box::new(IrAlSujeto {
                            tipo: tipo_ir.clone(),
                            sujeto: sujeto_ir.clone(),
                        }),
                        cx,
                    );
                },
            ),
        )
        .child(
            // Paso 1 del descarte (alertas-09): NO descarta directo, pide confirmación en el modal.
            tema::boton_primario("alerta-descartar", "Descartar").on_click(move |_e, window, cx| {
                window.dispatch_action(Box::new(PedirConfirmacionDescarte), cx);
            }),
        )
        .into_any_element()
}
