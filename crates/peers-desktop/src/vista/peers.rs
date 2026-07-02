//! Pantalla Peers — tabla viva de instancias de la red claude-peers.
//!
//! INTENCIÓN: porta la pantalla 1 de la TUI (`peers-tui/src/ui/peers.rs`), que pinta una tabla
//! `(id, directorio, resumen, visto)` desde `POST /listar`. Aquí se AÑADE (feature documentada)
//! una columna de ESTADO por peer — ocioso / atascado / trabajando — derivada cruzando las
//! alertas vigentes del supervisor con cada peer por `sujeto == id`. Es información que la TUI ya
//! tenía repartida en la pantalla Alertas; aquí se consolida junto al peer para verla de un vistazo.
//!
//! DECISIÓN DE DISEÑO (por qué NO uso `Table` ni `Badge` del kit): coherente con el resto de las
//! vistas (alertas/redis/trazabilidad) y con la Fundación (que evitó `Sidebar`). El `Table` del kit
//! es STATEFUL (exige `Entity<TableState>` + `TableDelegate` + `Context`), incompatible con la firma
//! pura `render_peers(&EstadoPantalla) -> impl IntoElement`. Se construye con `div()` en filas flex
//! (mismos helpers de ancho fijo `w(px)` + `flex_1` que ya compilan en las otras pantallas). El chip
//! de estado es un `div()` estilizado (píldora con color literal), no `Badge` (que es un decorador
//! de esquina con color de tema). Migrable a `Table`/`Badge` cuando se promueva el render a método
//! con `Context`.

use gpui::{div, px, rgb, IntoElement, ParentElement, SharedString, Styled};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;

use peers_core::{Alerta, Instancia, TipoAlerta};

use crate::app::EstadoPantalla;

// Anchos fijos (px) de las columnas no flexibles, espejo de los Constraint de la TUI. `resumen` es
// la flexible (`flex_1`); las demás llevan ancho fijo para que los chips de estado queden alineados.
const COL_ID: f32 = 140.0;
const COL_DIR: f32 = 260.0;
const COL_VISTO: f32 = 90.0;
const COL_ESTADO: f32 = 110.0;

/// Estado operativo de un peer, derivado de las alertas vigentes. Es un enum cerrado (no
/// "stringly typed") para que la etiqueta y el color salgan de un único match y no se
/// desincronicen. `Trabajando` es el caso por defecto: un peer listado que no tiene alerta viva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EstadoPeer {
    /// Peer vivo sin tarea en curso pasado el umbral (alerta `TipoAlerta::Ocioso`).
    Ocioso,
    /// Peer con tarea abierta sin reporte pasado el umbral (alerta `TipoAlerta::Atascado`).
    Atascado,
    /// Sin alerta viva asociada: se asume trabajando normalmente. Caso por defecto.
    Trabajando,
}

impl EstadoPeer {
    /// Etiqueta corta para el chip. Espejo del vocabulario del supervisor (R2/R3).
    fn etiqueta(self) -> &'static str {
        match self {
            EstadoPeer::Ocioso => "ocioso",
            EstadoPeer::Atascado => "atascado",
            EstadoPeer::Trabajando => "trabajando",
        }
    }

    /// Color del chip (RGB literal, sin depender de `cx.theme()` — la firma pura no tiene `cx`).
    /// Mapeo espejo de la TUI: ocioso/amarillo, atascado/naranja, trabajando/verde.
    fn color(self) -> u32 {
        match self {
            EstadoPeer::Ocioso => 0xd9a021,     // amarillo ámbar
            EstadoPeer::Atascado => 0xd9542b,   // naranja-rojo (más severo que ocioso)
            EstadoPeer::Trabajando => 0x3f9d5c, // verde
        }
    }
}

/// Deriva el estado de un peer cruzando su `id` con las alertas vigentes. Función PURA (sin reloj
/// ni red): el broker ya evaluó los umbrales temporales y emitió las alertas; aquí sólo se lee su
/// veredicto. `Atascado` tiene prioridad sobre `Ocioso` (es la condición más severa) si por alguna
/// razón coexistieran. El resto de tipos de alerta (ghosteo, cierre/cancelación sospechosos) no
/// describen el estado operativo del peer en esta tabla, así que no cambian esta columna.
fn estado_peer(id: &str, alertas: &[Alerta]) -> EstadoPeer {
    let mut estado = EstadoPeer::Trabajando;
    for a in alertas.iter().filter(|a| a.sujeto == id) {
        match a.tipo {
            TipoAlerta::Atascado => return EstadoPeer::Atascado, // el más severo: corta ya
            TipoAlerta::Ocioso => estado = EstadoPeer::Ocioso,
            _ => {}
        }
    }
    estado
}

/// Chip de estado: píldora de color con la etiqueta. Un `div()` estilizado (no `Badge`), estable
/// entre revisiones del kit.
fn chip_estado(estado: EstadoPeer) -> impl IntoElement {
    div()
        .px_2()
        .py(px(1.0))
        .rounded_md()
        .text_xs()
        .bg(rgb(estado.color()))
        .text_color(rgb(0xffffff))
        .child(SharedString::from(estado.etiqueta()))
}

/// Celda de texto de ancho fijo. `flex_celda` (abajo) es la flexible para `resumen`.
fn celda(texto: impl Into<SharedString>, ancho: f32) -> impl IntoElement {
    div().w(px(ancho)).px_1().text_sm().child(texto.into())
}

/// Celda flexible (ocupa el espacio sobrante). Se usa para la columna `resumen`.
fn celda_flex(texto: impl Into<SharedString>) -> impl IntoElement {
    div().flex_1().overflow_hidden().px_1().text_sm().child(texto.into())
}

/// Fila de encabezado de la tabla. Los anchos deben cuadrar con los de `fila`.
fn encabezado() -> impl IntoElement {
    div()
        .h_flex()
        .w_full()
        .py_1()
        .gap_2()
        .border_b_1()
        .border_color(rgb(0x3a3a3a))
        .text_color(rgb(0xd9a021))
        .child(celda("id", COL_ID))
        .child(celda("directorio", COL_DIR))
        .child(celda_flex("resumen"))
        .child(celda("visto", COL_VISTO))
        // La columna añadida de estado va a la derecha, con ancho fijo para alinear los chips.
        .child(div().w(px(COL_ESTADO)).px_1().child(SharedString::from("estado")))
}

/// Una fila de peer: 4 celdas de la TUI + el chip de estado (columna añadida). `alterna` pinta un
/// fondo tenue en filas impares para legibilidad (zebra), como la selección resaltada de la TUI.
fn fila(inst: &Instancia, estado: EstadoPeer, alterna: bool) -> impl IntoElement {
    // Espejo de `fila_peer` de la TUI: mismos campos y misma extracción de la hora del `visto_en`.
    // No se recorta el texto aquí: el ancho fijo/flex de cada celda lo acota al pintar.
    let visto = hora_iso(&inst.visto_en);
    let repo = inst.repo_git.as_deref().unwrap_or("");

    // El directorio se muestra tal cual; si hay repo git, se anota entre paréntesis como pista.
    let dir = if repo.is_empty() {
        inst.directorio.clone()
    } else {
        format!("{} ({repo})", inst.directorio)
    };

    let base = div()
        .h_flex()
        .w_full()
        .items_center()
        .py_1()
        .gap_2();
    let base = if alterna { base.bg(rgb(0x1c1c1c)) } else { base };

    base.child(celda(inst.id.clone(), COL_ID))
        .child(celda(dir, COL_DIR))
        .child(celda_flex(inst.resumen.clone()))
        .child(celda(visto, COL_VISTO))
        .child(div().w(px(COL_ESTADO)).px_1().child(chip_estado(estado)))
}

/// Banner de error: se pinta cuando la última carga contra el broker falló, para que la tabla
/// vacía no se confunda con "no hay peers". El texto lo aporta `ErrorBroker::Display`.
fn banner_error(err: &crate::cliente::ErrorBroker) -> impl IntoElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x5a1f1f))
        .text_color(rgb(0xffd7d7))
        .text_sm()
        .child(SharedString::from(format!("Broker: {err}")))
}

/// Render de la pantalla Peers. Consume `instancias` y `alertas` ya cargadas por la app.
pub fn render_peers(datos: &EstadoPantalla) -> impl IntoElement {
    let total = datos.instancias.len();

    // Cuerpo: título con el conteo, banner de error (si lo hay), encabezado y filas.
    let mut columna = div()
        .v_flex()
        .size_full()
        .gap_2()
        .p_4()
        .child(div().text_xl().child(SharedString::from(format!("Peers ({total})"))));

    if let Some(err) = &datos.error_peers {
        columna = columna.child(banner_error(err));
    }

    if total == 0 && datos.error_peers.is_none() {
        // Sin datos y sin error: aún no llegó la primera respuesta o no hay peers vivos.
        return columna.child(
            div()
                .text_sm()
                .text_color(rgb(0x888888))
                .child(SharedString::from("(sin peers vivos)")),
        );
    }

    // Filas zebra: se derivan aquí a partir de las alertas cacheadas (cruce por sujeto == id).
    let filas = datos
        .instancias
        .iter()
        .enumerate()
        .map(|(idx, inst)| {
            let estado = estado_peer(&inst.id, &datos.alertas);
            fila(inst, estado, idx % 2 == 1)
        })
        .collect::<Vec<_>>();

    let tabla = div().v_flex().w_full().child(encabezado()).children(filas);

    columna.child(tabla)
}

/// Extrae `HH:MM:SS` de un ISO 8601 para la columna "visto". Espejo de `hora_iso` de la TUI
/// (`peers-tui/src/app.rs`), replicado local porque `peers-core` no lo expone como función pública
/// y duplicar 6 líneas es más barato que acoplar crates. Sin patrón `T` → cadena tal cual.
fn hora_iso(iso: &str) -> String {
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        let fin = resto.find(['.', 'Z', '+']).unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alerta(tipo: TipoAlerta, sujeto: &str) -> Alerta {
        Alerta {
            tipo,
            sujeto: sujeto.to_string(),
            detalle: String::new(),
            creada_en: String::new(),
        }
    }

    #[test]
    fn sin_alertas_el_peer_esta_trabajando() {
        assert_eq!(estado_peer("claudia", &[]), EstadoPeer::Trabajando);
    }

    #[test]
    fn alerta_ocioso_marca_ocioso() {
        let a = [alerta(TipoAlerta::Ocioso, "claudia")];
        assert_eq!(estado_peer("claudia", &a), EstadoPeer::Ocioso);
    }

    #[test]
    fn alerta_atascado_tiene_prioridad_sobre_ocioso() {
        let a = [
            alerta(TipoAlerta::Ocioso, "claudia"),
            alerta(TipoAlerta::Atascado, "claudia"),
        ];
        assert_eq!(estado_peer("claudia", &a), EstadoPeer::Atascado);
    }

    #[test]
    fn alerta_de_otro_sujeto_no_afecta() {
        let a = [alerta(TipoAlerta::Ocioso, "otro")];
        assert_eq!(estado_peer("claudia", &a), EstadoPeer::Trabajando);
    }

    #[test]
    fn otros_tipos_de_alerta_no_cambian_el_estado() {
        let a = [alerta(TipoAlerta::Ghosteo, "claudia")];
        assert_eq!(estado_peer("claudia", &a), EstadoPeer::Trabajando);
    }

    #[test]
    fn hora_iso_extrae_hh_mm_ss() {
        assert_eq!(hora_iso("2026-07-01T17:30:45.123Z"), "17:30:45");
        assert_eq!(hora_iso("sin-t"), "sin-t");
    }
}
