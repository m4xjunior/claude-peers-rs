//! Tema visual de peers-desktop — traducción del Design System "Ethos" (web/Tailwind) a un
//! tema GPUI nativo.
//!
//! INTENCIÓN (por qué existe este módulo): las 9 pantallas nacieron con colores azules
//! hardcodeados (0x3b82f6…) y sin tipografía, y cada una re-definía sus propios literales de
//! estilo. Eso dejó la app "navegable pero fea" e incoherente. Este módulo centraliza LOS
//! TOKENS EXACTOS del Ethos (paleta pergamino/tinta + acento dorado "brasa") y un puñado de
//! HELPERS reutilizables, para que las pantallas compongan con vocabulario común
//! (`superficie_card()`, `titulo()`, `boton_primario()`, `fila_seleccionable()`…) y el look
//! quede uniforme sin copiar literales. Si el Ethos cambia, se toca AQUÍ y sólo aquí.
//!
//! DECISIÓN DE DISEÑO (por qué helpers-que-devuelven-`Div`, no un `Theme` del kit): las vistas
//! son PURAS (`render_x(&EstadoPantalla) -> impl IntoElement`, sin `cx`), así que no pueden leer
//! `cx.theme()`. Un helper que devuelve un `div()` ya estilizado encaja en esa firma sin fricción:
//! la vista lo llama, le encadena sus `.child(...)` y listo. Los helpers ESTÁTICOS (texto,
//! superficies) devuelven `Div`; los INTERACTIVOS (filas/botones, que necesitan `.id()` para
//! `hover`/`on_click`) devuelven `Stateful<Div>`. El caller cablea el `on_click` (despacho de
//! Action) porque el tema no conoce las acciones de cada pantalla.
//!
//! NOTA SOBRE FUENTES: `Inter` está instalada (UI). `Fraunces` (títulos) e `IBM Plex Mono`
//! (datos) NO están instaladas en la mayoría de máquinas: GPUI cae al fallback del sistema
//! (serif / monospace respectivamente). Se pide la familia igualmente para que, si el usuario la
//! instala, el render mejore sin tocar código.
//!
//! NOTA SOBRE LETTER-SPACING: el Ethos pide `letter-spacing` en los "eyebrows". GPUI (este rev de
//! zed) NO expone tracking/letter-spacing en el trait `Styled`. Se compensa con MAYÚSCULAS +
//! `text_xs` + color humo, que es el efecto visual buscado (etiqueta discreta y espaciada al ojo).

use gpui::{
    div, px, rgba, BoxShadow, Div, FontWeight, InteractiveElement, ParentElement, Pixels,
    Rgba, SharedString, Styled,
};

// -------------------------------------------------------------------------------------------------
// 1) TOKENS DE COLOR — hex EXACTOS del Ethos. `rgb()` los sube a `Rgba` opaco (alfa = FF).
//    Se exponen como `Rgba` (no `Hsla`) porque es lo que `.bg()/.text_color()/.border_color()`
//    aceptan directamente y es el tipo con el que ya trabajan las vistas (`rgb(0x…)`).
// -------------------------------------------------------------------------------------------------

/// Fondo principal de la app — negro cálido casi puro. Base de todo.
pub const TINTA: Rgba = Rgba {
    r: 0x10 as f32 / 255.0,
    g: 0x0D as f32 / 255.0,
    b: 0x0A as f32 / 255.0,
    a: 1.0,
};

/// Superficie elevada (cards, paneles) — un paso por encima de TINTA.
pub const TINTA2: Rgba = Rgba {
    r: 0x1A as f32 / 255.0,
    g: 0x16 as f32 / 255.0,
    b: 0x11 as f32 / 255.0,
    a: 1.0,
};

/// Texto principal — pergamino cálido. Sustituye al blanco puro para bajar el contraste duro.
pub const PAPEL: Rgba = Rgba {
    r: 0xEC as f32 / 255.0,
    g: 0xE5 as f32 / 255.0,
    b: 0xD7 as f32 / 255.0,
    a: 1.0,
};

/// ACENTO de marca — dorado "brasa". Úsese CON MODERACIÓN: estado activo, labels, acentos.
pub const BRASA: Rgba = Rgba {
    r: 0xC9 as f32 / 255.0,
    g: 0xA9 as f32 / 255.0,
    b: 0x6E as f32 / 255.0,
    a: 1.0,
};

/// Texto SOBRE dorado (p.ej. label de `boton_primario`) — casi negro cálido, alto contraste.
pub const SALMO: Rgba = Rgba {
    r: 0x17 as f32 / 255.0,
    g: 0x13 as f32 / 255.0,
    b: 0x09 as f32 / 255.0,
    a: 1.0,
};

/// Texto terciario / labels / placeholders — humo apagado.
pub const HUMO: Rgba = Rgba {
    r: 0x93 as f32 / 255.0,
    g: 0x8B as f32 / 255.0,
    b: 0x7B as f32 / 255.0,
    a: 1.0,
};

/// Bordes y separadores — línea tenue sobre el fondo oscuro.
pub const LINEA: Rgba = Rgba {
    r: 0x2B as f32 / 255.0,
    g: 0x27 as f32 / 255.0,
    b: 0x1F as f32 / 255.0,
    a: 1.0,
};

// -------------------------------------------------------------------------------------------------
// 2) TOKENS DE FORMA — radios y familias tipográficas del Ethos, como constantes para no dispersar
//    números mágicos. Los radios se usan vía `.rounded(px(RADIO_*))`.
// -------------------------------------------------------------------------------------------------

/// Radio de card/panel (px). Estética generosa "Apple Podcasts".
pub const RADIO_CARD: f32 = 14.0;
/// Radio de controles (botones, inputs) (px).
pub const RADIO_CONTROL: f32 = 10.0;
/// Radio "pill" (chips/badges) — valor alto para que sea totalmente redondeado.
pub const RADIO_PILL: f32 = 999.0;

/// Familia de UI (instalada). Cuerpo, controles, tablas.
pub const FUENTE_UI: &str = "Inter";
/// Familia de títulos (NO instalada → fallback serif del sistema).
pub const FUENTE_DISPLAY: &str = "Fraunces";
/// Familia de datos/timestamps (NO instalada → fallback monospace del sistema).
pub const FUENTE_MONO: &str = "IBM Plex Mono";

/// Acento dorado TENUE para resaltar la fila activa sin gritar (brasa al ~14% de alfa sobre el
/// fondo). Se define como `rgba` con alfa bajo para que el fondo de la card se transparente.
/// `pub`: lo reutiliza el sidebar de `AppDesktop` para marcar el ítem de la pantalla activa con el
/// mismo lenguaje que las filas seleccionables (coherencia visual).
pub const BRASA_TENUE: Rgba = Rgba {
    r: 0xC9 as f32 / 255.0,
    g: 0xA9 as f32 / 255.0,
    b: 0x6E as f32 / 255.0,
    a: 0.14,
};

// -------------------------------------------------------------------------------------------------
// 3) SOMBRAS — sutiles, no "glow". El Ethos pide profundidad discreta sobre fondo oscuro, así que
//    la sombra es negra semi-transparente con blur moderado y sin spread (nada de halos de color).
// -------------------------------------------------------------------------------------------------

/// Sombra sutil para superficies elevadas (cards/paneles). Vector de un solo `BoxShadow` porque
/// `.shadow(Vec<BoxShadow>)` es la API de GPUI (no hay preset dorado; los presets `shadow_sm/md`
/// del kit usan negro genérico y sirven, pero aquí se calibra el alfa para el fondo cálido).
pub fn sombra_card() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: rgba(0x00000059).into(), // negro ~35% alfa
        offset: gpui::point(px(0.0), px(6.0)),
        blur_radius: px(18.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

// -------------------------------------------------------------------------------------------------
// 4) HELPERS DE SUPERFICIE
// -------------------------------------------------------------------------------------------------

/// Fondo raíz de la app: ocupa todo, pinta TINTA, texto PAPEL por defecto y fija la fuente UI para
/// todo el subárbol (los hijos heredan la familia salvo que la pisen). Punto de partida del render
/// raíz de `AppDesktop`.
pub fn fondo_app() -> Div {
    div()
        .size_full()
        .bg(TINTA)
        .text_color(PAPEL)
        .font_family(FUENTE_UI)
        .text_size(px(14.0))
}

/// Superficie elevada (card/panel): fondo TINTA2, esquinas de card, borde LINEA y sombra sutil.
/// El caller le encadena padding/flex y sus hijos. Es el contenedor canónico de las secciones.
pub fn superficie_card() -> Div {
    div()
        .bg(TINTA2)
        .rounded(px(RADIO_CARD))
        .border_1()
        .border_color(LINEA)
        .shadow(sombra_card())
}

// -------------------------------------------------------------------------------------------------
// 5) HELPERS DE TEXTO — cada uno recibe el texto y devuelve un `Div` ya estilizado con el rol
//    tipográfico correspondiente. Devolver `Div` (no `impl IntoElement`) permite que el caller,
//    si lo necesita, siga encadenando estilos (p.ej. `.mb_2()`).
// -------------------------------------------------------------------------------------------------

/// Título de sección — Fraunces (serif fallback), PAPEL, grande y con peso semibold. Encabeza cada
/// pantalla. `impl Into<SharedString>` para aceptar `&str`, `String` o `SharedString` sin fricción.
pub fn titulo(texto: impl Into<SharedString>) -> Div {
    div()
        .font_family(FUENTE_DISPLAY)
        .text_size(px(26.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(PAPEL)
        .child(texto.into())
}

/// "Eyebrow" — etiqueta discreta sobre un título/sección. Mono (fallback), HUMO, MAYÚSCULAS y
/// pequeña. Compensa la falta de letter-spacing de GPUI con el uppercase (mismo efecto al ojo).
pub fn eyebrow(texto: impl Into<SharedString>) -> Div {
    let t: SharedString = texto.into();
    div()
        .font_family(FUENTE_MONO)
        .text_xs()
        .text_color(HUMO)
        .child(SharedString::from(t.to_uppercase()))
}

/// Texto de cuerpo/primario — PAPEL, tamaño base UI. Para valores y contenido normal.
pub fn texto_primario(texto: impl Into<SharedString>) -> Div {
    div().text_color(PAPEL).text_size(px(14.0)).child(texto.into())
}

/// Texto terciario — HUMO, pequeño. Para labels de campo, metadatos, timestamps atenuados.
pub fn texto_terciario(texto: impl Into<SharedString>) -> Div {
    div().text_color(HUMO).text_sm().child(texto.into())
}

// -------------------------------------------------------------------------------------------------
// 6) BOTONES — devuelven `Stateful<Div>` (llevan `.id()` para hover/on_click). El caller SÓLO
//    encadena su `.on_click(...)` (que despacha la Action de su pantalla). No se usa el `Button`
//    del kit para no atarse a su tema y mantener el look Ethos exacto.
// -------------------------------------------------------------------------------------------------

/// Botón primario (acción principal): fondo BRASA, texto SALMO, radio de control, hover que
/// aclara ligeramente el dorado. `id` único obligatorio (requisito de elementos interactivos).
pub fn boton_primario(id: impl Into<SharedString>, label: impl Into<SharedString>) -> gpui::Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .py_2()
        .rounded(px(RADIO_CONTROL))
        .bg(BRASA)
        .text_color(SALMO)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.bg(rgba(0xD9BC85FF))) // brasa un punto más claro
        .child(label.into())
}

/// Botón secundario (acción alternativa/cancelar): sin relleno de color, borde LINEA y texto
/// PAPEL. Hover pinta la superficie TINTA2 para dar feedback sin competir con el primario.
pub fn boton_secundario(id: impl Into<SharedString>, label: impl Into<SharedString>) -> gpui::Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .py_2()
        .rounded(px(RADIO_CONTROL))
        .border_1()
        .border_color(LINEA)
        .text_color(PAPEL)
        .cursor_pointer()
        .hover(|s| s.bg(TINTA2))
        .child(label.into())
}

// -------------------------------------------------------------------------------------------------
// 7) FILA SELECCIONABLE — el corazón de "operable como la TUI". Devuelve un `Stateful<Div>`
//    (contenedor h_flex vacío) listo para que el caller le meta las celdas y el `on_click`.
// -------------------------------------------------------------------------------------------------

/// Fila de tabla/lista seleccionable. `activa=true` → resalta con acento brasa tenue + borde
/// izquierdo dorado (marcador de selección claro, estilo lista de reproducción). En reposo es
/// transparente y el hover pinta TINTA2 (feedback de "puedo clicar aquí"). El caller añade las
/// celdas con `.child(...)` y cablea `.on_click(...)` para despachar su Action de selección.
///
/// Detalle de layout: se usa `border_l_2` SIEMPRE (transparente cuando inactiva) para que la fila
/// no "salte" 2px al seleccionarse — sólo cambia el color del borde, no su presencia.
pub fn fila_seleccionable(id: impl Into<SharedString>, activa: bool) -> gpui::Stateful<Div> {
    let base = div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .px_3()
        .py_2()
        .gap_2()
        .rounded(px(RADIO_CONTROL))
        .border_l_2()
        .cursor_pointer();

    if activa {
        // Seleccionada: fondo dorado tenue + borde izquierdo BRASA sólido.
        base.bg(BRASA_TENUE).border_color(BRASA)
    } else {
        // En reposo: borde izquierdo transparente (reserva el espacio) y hover a TINTA2.
        base.border_color(rgba(0x00000000)).hover(|s| s.bg(TINTA2))
    }
}

// -------------------------------------------------------------------------------------------------
// 8) CHIP DE ESTADO — píldora de color para estados de tarea/alerta/peer. El `color` lo decide la
//    pantalla (mapea su enum de estado → color), así el tema no conoce los estados de dominio.
// -------------------------------------------------------------------------------------------------

/// Chip "pill" con `texto` sobre fondo `color`. Radio pill, padding compacto, texto pequeño y
/// oscuro (SALMO) para contraste sobre colores claros/saturados. `color` es `Rgba` para aceptar
/// tanto los tokens del tema (BRASA…) como literales de severidad de cada pantalla.
pub fn chip_estado(texto: impl Into<SharedString>, color: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .px_2()
        .py(px(2.0))
        .rounded(px(RADIO_PILL))
        .bg(color)
        .text_color(SALMO)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .child(texto.into())
}

// -------------------------------------------------------------------------------------------------
// Helper interno reservado para consumidores del tema que necesiten componer paddings uniformes.
// Se deja documentado el tipo `Pixels` importado para radios/offsets calculados por las vistas.
// -------------------------------------------------------------------------------------------------

/// Convierte un radio del Ethos (f32) a `Pixels` de GPUI. Azúcar para las vistas que quieran
/// `.rounded(tema::radio(tema::RADIO_CARD))` sin importar `px` ellas mismas.
pub fn radio(valor: f32) -> Pixels {
    px(valor)
}
