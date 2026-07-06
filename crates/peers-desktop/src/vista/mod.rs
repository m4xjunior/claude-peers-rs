//! Módulo de vistas: una por pantalla, espejo de las 9 pantallas de la TUI.
//!
//! INTENCIÓN (Fase 1 — Fundación): cada `render_<pantalla>` es un STUB que devuelve el título
//! de su pantalla. Las fases siguientes llenarán estos cuerpos con Table/List/etc. de
//! gpui-component consumiendo los datos del `ClienteBroker`. La firma se fija AHORA para que
//! los agentes de Fase 2 encajen sin tocar `app.rs` ni el enrutado del sidebar.
//!
//! Contrato de firma (estable): cada stub recibe `&EstadoPantalla` (datos ya cargados por la
//! app; en Fase 1 está vacío) y devuelve `impl IntoElement`. Al rellenar un stub, se leen los
//! campos de `EstadoPantalla` correspondientes; no cambia la firma.

use gpui::{div, IntoElement, ParentElement, SharedString, Styled};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;

pub mod acceso;
pub mod alertas;
pub mod broker;
pub mod chat_privado;
pub mod config;
pub mod jornada;
pub mod lanzador;
pub mod peers;
pub mod proyectos;
pub mod redis;
pub mod tareas;
pub mod trazabilidad;

use crate::app::EstadoPantalla;

/// Encabezado común de una pantalla placeholder: un título grande centrado. Se comparte entre
/// todos los stubs para que la Fundación se vea coherente y para no duplicar el layout base.
pub(crate) fn placeholder(titulo: impl Into<SharedString>) -> impl IntoElement {
    div()
        .v_flex()
        .size_full()
        .gap_2()
        .p_6()
        .child(div().text_xl().child(titulo.into()))
        .child(div().text_sm().child(SharedString::from("(pantalla en construcción)")))
}

/// Reexporta los stubs por pantalla con el nombre canónico `render_<pantalla>` que consumen
/// `app.rs` y, más adelante, los agentes de Fase 2.
pub use acceso::render_acceso;
pub use alertas::render_alertas;
pub use broker::render_broker;
pub use chat_privado::render_chat_privado;
pub use config::render_config;
pub use jornada::render_jornada;
pub use lanzador::render_lanzador;
pub use peers::render_peers;
pub use proyectos::render_proyectos;
pub use redis::render_redis;
pub use tareas::render_tareas;
pub use trazabilidad::render_trazabilidad;

/// Firma de referencia (documentación): todos los stubs comparten esta forma.
/// `pub fn render_<pantalla>(datos: &EstadoPantalla) -> impl IntoElement`.
#[allow(dead_code)]
fn _referencia_de_firma(datos: &EstadoPantalla) -> impl IntoElement {
    render_peers(datos)
}
