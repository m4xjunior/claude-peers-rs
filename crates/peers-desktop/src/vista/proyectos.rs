//! Pantalla Proyectos (RFC-proyectos): workspaces aislados de la empresa.
//!
//! Un proyecto agrupa un equipo de agentes por su ubicación (carpeta local / host SSH / tmux). El
//! aislamiento es por CONVENCIÓN de id: los agentes del proyecto `x` tienen ids `rol@x`, y la app
//! filtra por ese sufijo (v1, cero backend nuevo). Esta pantalla da el CRUD (crear/editar/archivar/
//! duplicar) sobre `config.toml` y la FICHA de cada proyecto (equipo/tablero/actividad/alertas)
//! cruzando el estado vivo del broker.
//!
//! Vista PURA (sin `cx`): despacha `Action` que `AppDesktop` maneja con `.on_action`. Los modales
//! son overlays Ethos (no el `Dialog` del kit, por el patrón "vista pura + estado" del proyecto).
//! Red del estado vivo SIEMPRE por `background_executor` + `bloquear_en` (regla anti-SIGABRT).

use gpui::{
    div, prelude::FluentBuilder, Action, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled,
};
use gpui_component::{input::Input, StyledExt};

use crate::app::EstadoPantalla;
use crate::config::Proyecto;
use crate::tema;

// --- Sub-tab de la ficha de proyecto (R8) ---

/// Secciones de la ficha de un proyecto abierto. `Default` = Equipo (la primera).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FichaProyectoTab {
    #[default]
    Equipo,
    Tablero,
    Actividad,
    Alertas,
}

impl FichaProyectoTab {
    pub const TODAS: [FichaProyectoTab; 4] = [
        FichaProyectoTab::Equipo,
        FichaProyectoTab::Tablero,
        FichaProyectoTab::Actividad,
        FichaProyectoTab::Alertas,
    ];
    pub fn titulo(self) -> &'static str {
        match self {
            FichaProyectoTab::Equipo => "Equipo",
            FichaProyectoTab::Tablero => "Tablero",
            FichaProyectoTab::Actividad => "Actividad",
            FichaProyectoTab::Alertas => "Alertas",
        }
    }
}

/// Formulario de proyecto abierto (crear o editar). El caller (AppDesktop) siembra los inputs y, al
/// confirmar, lee el nombre/ruta y decide crear vs editar según la variante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormProyecto {
    /// Crear un proyecto nuevo con la ubicación `tipo` (el picker/select elige la ruta/host).
    Crear { tipo: TipoUbicacion },
    /// Editar el proyecto `id` (nombre y/o ubicación).
    Editar { id: String, tipo: TipoUbicacion },
}

/// Qué clase de ubicación se está eligiendo en el formulario (define qué campos pinta).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoUbicacion {
    Local,
    Ssh,
}

// --- Acciones (namespace proyectos) ---

/// Abre el formulario de CREAR proyecto (con la clase de ubicación elegida por defecto Local).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct AbrirCrearProyecto;

/// Abre el formulario de EDITAR el proyecto `id`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct AbrirEditarProyecto {
    pub id: String,
}

/// Confirma el formulario activo (crea o edita según la variante). AppDesktop lee los inputs.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = proyectos, no_json)]
pub struct ConfirmarFormProyecto;

/// Cierra el formulario sin confirmar (Cancelar / Esc / clic fuera).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = proyectos, no_json)]
pub struct CerrarFormProyecto;

/// Elige la clase de ubicación en el formulario (Local vs SSH) — cambia qué campos se piden.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct ElegirTipoUbicacion {
    pub local: bool,
}

/// Abre el picker nativo de carpeta para la ubicación Local del formulario.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = proyectos, no_json)]
pub struct ElegirCarpetaProyecto;

/// Abre la ficha del proyecto `id` (sub-tabs equipo/tablero/…).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct AbrirFichaProyecto {
    pub id: String,
}

/// Cierra la ficha y vuelve al grid.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = proyectos, no_json)]
pub struct CerrarFichaProyecto;

/// Cambia el sub-tab de la ficha abierta.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct CambiarFichaTab {
    pub tab: u8, // índice en FichaProyectoTab::TODAS
}

/// Archiva o reactiva el proyecto `id`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct ArchivarProyecto {
    pub id: String,
    pub archivar: bool,
}

/// Duplica el proyecto `id` como plantilla (misma ubicación de arranque, el usuario la edita luego).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct DuplicarProyecto {
    pub id: String,
}

/// Fija (o limpia con `id=None` vía string vacío) el proyecto ACTIVO global (selector de cabecera).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct FijarProyectoActivo {
    /// id del proyecto, o "" para "todos" (limpiar el filtro global).
    pub id: String,
}

// --- Render ---

/// Render de la pantalla. Si hay un proyecto abierto → su ficha; si no → el grid + CRUD.
pub fn render_proyectos(datos: &EstadoPantalla) -> impl IntoElement {
    let raiz = tema::fondo_app().v_flex();
    match &datos.proyecto_abierto {
        Some(id) => match datos.proyectos.iter().find(|p| &p.id == id) {
            Some(p) => raiz.child(ficha(datos, p)),
            // El proyecto abierto ya no existe (borrado): cae al grid.
            None => raiz.child(grid(datos)),
        },
        None => raiz.child(grid(datos)),
    }
}

/// Grid de proyectos: cabecera con "+ Nuevo" y selector activo, luego cards activas y archivadas.
fn grid(datos: &EstadoPantalla) -> impl IntoElement {
    let activos: Vec<&Proyecto> = datos.proyectos.iter().filter(|p| !p.archivado).collect();
    let archivados: Vec<&Proyecto> = datos.proyectos.iter().filter(|p| p.archivado).collect();

    let mut cont = div().v_flex().p_8().gap_5();
    cont = cont.child(cabecera(datos));

    if activos.is_empty() {
        cont = cont.child(tema::texto_terciario(
            "No hay proyectos aún. Crea uno con «+ Nuevo proyecto».",
        ));
    } else {
        let mut fila = div().flex().flex_wrap().gap_4();
        for p in &activos {
            fila = fila.child(card_proyecto(datos, p, false));
        }
        cont = cont.child(fila);
    }

    if !archivados.is_empty() {
        cont = cont.child(tema::eyebrow("archivados"));
        let mut fila = div().flex().flex_wrap().gap_4();
        for p in &archivados {
            fila = fila.child(card_proyecto(datos, p, true));
        }
        cont = cont.child(fila);
    }

    // Overlay del formulario de crear/editar, si está abierto.
    cont.when_some(datos.proyecto_form.as_ref(), |el, _form| {
        el.child(overlay_form(datos))
    })
}

/// Cabecera del grid: título + botón "Nuevo" + selector de proyecto activo global (R11).
fn cabecera(datos: &EstadoPantalla) -> impl IntoElement {
    div()
        .v_flex()
        .gap_2()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(tema::eyebrow("Proyectos"))
                        .child(tema::titulo("Tus proyectos")),
                )
                .child(
                    tema::boton_primario("proyecto-nuevo", "+ Nuevo proyecto")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(AbrirCrearProyecto), cx)
                        }),
                ),
        )
        .child(selector_activo(datos))
}

/// Selector del proyecto ACTIVO global (R11): chips "todos" + uno por proyecto activo. El activo
/// resalta. Fija el filtro que las otras pantallas usan.
fn selector_activo(datos: &EstadoPantalla) -> impl IntoElement {
    let activo = datos.proyecto_activo.clone();
    let mut fila = div().h_flex().gap_2().flex_wrap().items_center();
    fila = fila.child(tema::texto_terciario("activo:"));
    // Chip "todos" (activo = None).
    fila = fila.child(chip_activo("(todos)", "", activo.is_none()));
    for p in datos.proyectos.iter().filter(|p| !p.archivado) {
        let es = activo.as_deref() == Some(p.id.as_str());
        fila = fila.child(chip_activo(&p.nombre, &p.id, es));
    }
    fila
}

/// Un chip del selector de proyecto activo. `id=""` = "todos".
fn chip_activo(etiqueta: &str, id: &str, activo: bool) -> impl IntoElement {
    let id_owned = id.to_string();
    tema::fila_seleccionable(SharedString::from(format!("activo-{id}")), activo)
        .px_3()
        .py_1()
        .rounded(tema::radio(tema::RADIO_PILL))
        .child(
            div()
                .text_color(if activo { tema::BRASA } else { tema::PAPEL })
                .child(SharedString::from(etiqueta.to_string())),
        )
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(FijarProyectoActivo { id: id_owned.clone() }), cx)
        })
}

/// Una card de proyecto en el grid: nombre BRASA + ubicación mono + conteo de agentes vivos.
/// Clic en la card → abre la ficha. Botones de acción (editar/archivar/duplicar) al pie.
fn card_proyecto(datos: &EstadoPantalla, p: &Proyecto, archivado: bool) -> impl IntoElement {
    let id_ficha = p.id.clone();
    let (vivos, total) = agentes_vivos(datos, p);
    tema::superficie_card()
        .v_flex()
        .gap_2()
        .p_4()
        .w(tema::radio(280.0))
        .when(archivado, |el| el.opacity(0.6))
        .child(
            div()
                .font_family(tema::FUENTE_UI)
                .text_color(tema::BRASA)
                .child(SharedString::from(p.nombre.clone())),
        )
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::HUMO)
                .child(SharedString::from(p.ubicacion.etiqueta())),
        )
        .child(tema::texto_terciario(format!("{vivos}/{total} agentes vivos")))
        .child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    tema::boton_secundario(format!("proy-abrir-{}", p.id), "Abrir")
                        .on_click({
                            let id = id_ficha.clone();
                            move |_, window, cx| {
                                window.dispatch_action(Box::new(AbrirFichaProyecto { id: id.clone() }), cx)
                            }
                        }),
                )
                .children(acciones_card(p, archivado)),
        )
}

/// Botones de acción al pie de una card (editar/archivar/reactivar/duplicar).
fn acciones_card(p: &Proyecto, archivado: bool) -> Vec<gpui::AnyElement> {
    use gpui::IntoElement as _;
    let id = p.id.clone();
    let mut v: Vec<gpui::AnyElement> = Vec::new();
    if !archivado {
        let ide = id.clone();
        v.push(
            tema::boton_secundario(format!("proy-editar-{}", p.id), "Editar")
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(AbrirEditarProyecto { id: ide.clone() }), cx)
                })
                .into_any_element(),
        );
        let idd = id.clone();
        v.push(
            tema::boton_secundario(format!("proy-dup-{}", p.id), "Duplicar")
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(DuplicarProyecto { id: idd.clone() }), cx)
                })
                .into_any_element(),
        );
    }
    let ida = id.clone();
    let etiqueta = if archivado { "Reactivar" } else { "Archivar" };
    v.push(
        tema::boton_secundario(format!("proy-arch-{}", p.id), etiqueta)
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(ArchivarProyecto { id: ida.clone(), archivar: !archivado }),
                    cx,
                )
            })
            .into_any_element(),
    );
    v
}

/// Cuenta agentes del proyecto vivos vs total, cruzando `agentes` (ids `rol@proyecto`) contra las
/// instancias vivas que trae el broker (`datos.instancias`). v1: por convención de id (R10).
fn agentes_vivos(datos: &EstadoPantalla, p: &Proyecto) -> (usize, usize) {
    let total = p.agentes.len();
    let vivos = p
        .agentes
        .iter()
        .filter(|id| datos.instancias.iter().any(|i| &i.id == *id))
        .count();
    (vivos, total)
}

// --- Ficha de un proyecto (R8): cabecera + sub-tabs + sección activa ---

/// Ficha completa de un proyecto abierto. Cabecera (nombre/ubicación + volver) + barra de sub-tabs
/// + la sección seleccionada. Cada sección cruza el estado vivo del broker filtrado por `@id`.
fn ficha(datos: &EstadoPantalla, p: &Proyecto) -> impl IntoElement {
    div()
        .v_flex()
        .p_8()
        .gap_4()
        .child(ficha_cabecera(p))
        .child(barra_tabs(datos.proyecto_ficha_tab))
        .child(seccion_activa(datos, p))
}

/// Cabecera de la ficha: botón volver + nombre BRASA + ubicación mono.
fn ficha_cabecera(p: &Proyecto) -> impl IntoElement {
    div()
        .h_flex()
        .items_center()
        .gap_3()
        .child(
            tema::boton_secundario("proy-volver", "← Proyectos")
                .on_click(|_, window, cx| window.dispatch_action(Box::new(CerrarFichaProyecto), cx)),
        )
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .text_color(tema::BRASA)
                        .child(SharedString::from(p.nombre.clone())),
                )
                .child(
                    div()
                        .font_family(tema::FUENTE_MONO)
                        .text_color(tema::HUMO)
                        .child(SharedString::from(p.ubicacion.etiqueta())),
                ),
        )
}

/// Barra de sub-tabs (Equipo/Tablero/Actividad/Alertas), el activo resaltado.
fn barra_tabs(activo: FichaProyectoTab) -> impl IntoElement {
    let mut fila = div().h_flex().gap_2();
    for (i, t) in FichaProyectoTab::TODAS.iter().enumerate() {
        let es = *t == activo;
        let idx = i as u8;
        fila = fila.child(
            tema::fila_seleccionable(SharedString::from(format!("ficha-tab-{i}")), es)
                .px_3()
                .py_1()
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .child(
                    div()
                        .text_color(if es { tema::BRASA } else { tema::PAPEL })
                        .child(SharedString::from(t.titulo())),
                )
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(CambiarFichaTab { tab: idx }), cx)
                }),
        );
    }
    fila
}

/// La sección de la ficha según el sub-tab activo. Todas cruzan el estado vivo filtrado por `@id`.
fn seccion_activa(datos: &EstadoPantalla, p: &Proyecto) -> gpui::AnyElement {
    use gpui::IntoElement as _;
    let sufijo = format!("@{}", p.id);
    match datos.proyecto_ficha_tab {
        FichaProyectoTab::Equipo => seccion_equipo(datos, p).into_any_element(),
        FichaProyectoTab::Tablero => seccion_tablero(datos, &sufijo).into_any_element(),
        FichaProyectoTab::Actividad => seccion_actividad().into_any_element(),
        FichaProyectoTab::Alertas => seccion_alertas(datos, &sufijo).into_any_element(),
    }
}

/// Equipo: los agentes del proyecto con su estado vivo (cruzando `instancias`).
fn seccion_equipo(datos: &EstadoPantalla, p: &Proyecto) -> impl IntoElement {
    let mut cont = tema::superficie_card().v_flex().gap_2().p_4();
    if p.agentes.is_empty() {
        return cont.child(tema::texto_terciario(
            "Este proyecto aún no tiene equipo. Lánzalo desde el Lanzador con id rol@proyecto.",
        ));
    }
    for id in &p.agentes {
        let vivo = datos.instancias.iter().any(|i| &i.id == id);
        let (texto_estado, color) = if vivo {
            ("vivo", tema::BRASA)
        } else {
            ("caído", tema::HUMO)
        };
        cont = cont.child(
            div()
                .h_flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .font_family(tema::FUENTE_MONO)
                        .text_color(tema::PAPEL)
                        .child(SharedString::from(id.clone())),
                )
                .child(tema::chip_estado(texto_estado, color)),
        );
    }
    cont
}

/// Tablero: conteo de tareas del proyecto por estado (filtradas por el sufijo del id dueño).
fn seccion_tablero(datos: &EstadoPantalla, sufijo: &str) -> impl IntoElement {
    use peers_core::EstadoTarea;
    let del_proyecto: Vec<&peers_core::Tarea> = datos
        .tareas
        .iter()
        .filter(|t| t.instancia_id.ends_with(sufijo))
        .collect();
    let cuenta = |e: EstadoTarea| del_proyecto.iter().filter(|t| t.estado == e).count();
    let abiertas = cuenta(EstadoTarea::Abierta) + cuenta(EstadoTarea::EnCurso);
    let bloqueadas = cuenta(EstadoTarea::Bloqueada);
    let hechas = cuenta(EstadoTarea::Hecha);
    tema::superficie_card()
        .v_flex()
        .gap_2()
        .p_4()
        .child(tema::texto_primario(format!(
            "{abiertas} en curso · {bloqueadas} bloqueadas · {hechas} hechas"
        )))
        .when(del_proyecto.is_empty(), |el| {
            el.child(tema::texto_terciario(
                "Sin tareas del proyecto (o el equipo aún no arrancó con id @proyecto).",
            ))
        })
}

/// Actividad: la bitácora del equipo. v1 = puntero (el detalle vive en Trazabilidad/Jornada por
/// peer); aquí se remite ahí en vez de duplicar la carga de acciones por-proyecto.
fn seccion_actividad() -> impl IntoElement {
    tema::superficie_card().v_flex().gap_2().p_4().child(tema::texto_terciario(
        "La actividad detallada del equipo está en Trazabilidad/Jornada por agente. (Vista agregada \
         por proyecto: v2.)",
    ))
}

/// Alertas del supervisor filtradas por el proyecto (sujeto termina en `@id`).
fn seccion_alertas(datos: &EstadoPantalla, sufijo: &str) -> impl IntoElement {
    let del_proyecto: Vec<&peers_core::Alerta> = datos
        .alertas
        .iter()
        .filter(|a| a.sujeto.ends_with(sufijo))
        .collect();
    let mut cont = tema::superficie_card().v_flex().gap_2().p_4();
    if del_proyecto.is_empty() {
        return cont.child(tema::texto_terciario("Sin alertas para este proyecto."));
    }
    for a in del_proyecto {
        cont = cont.child(
            div()
                .h_flex()
                .gap_2()
                .child(tema::chip_estado(format!("{:?}", a.tipo), tema::SALMO))
                .child(tema::texto_primario(a.detalle.clone())),
        );
    }
    cont
}

// --- Overlay del formulario de crear/editar proyecto ---

/// Overlay Ethos (backdrop + card centrada) del formulario de proyecto. Clic fuera → cerrar; el
/// contenido usa `occlude` para que el clic dentro no cierre. Esc lo maneja AppDesktop.
fn overlay_form(datos: &EstadoPantalla) -> gpui::AnyElement {
    use gpui::IntoElement as _;
    let Some(form) = &datos.proyecto_form else {
        return div().into_any_element();
    };
    let (titulo, tipo) = match form {
        FormProyecto::Crear { tipo } => ("Nuevo proyecto", *tipo),
        FormProyecto::Editar { tipo, .. } => ("Editar proyecto", *tipo),
    };
    let mut tarjeta = tema::superficie_card()
        .occlude()
        .v_flex()
        .gap_3()
        .p_6()
        .w(tema::radio(520.0))
        .child(tema::eyebrow(titulo))
        // Nombre.
        .child(tema::eyebrow("nombre"));
    if let Some(input) = &datos.input_proyecto_nombre {
        tarjeta = tarjeta.child(recuadro_input(Input::new(input)));
    }
    // Selector de clase de ubicación (Local / SSH).
    tarjeta = tarjeta.child(tema::eyebrow("ubicación")).child(
        div()
            .h_flex()
            .gap_2()
            .child(chip_tipo("Carpeta local", tipo == TipoUbicacion::Local, true))
            .child(chip_tipo("Host SSH", tipo == TipoUbicacion::Ssh, false)),
    );
    // Campo de ruta (+ botón picker si es local).
    tarjeta = tarjeta.child(tema::eyebrow(if tipo == TipoUbicacion::Local {
        "carpeta"
    } else {
        "host:ruta (ej. otus:/srv/app)"
    }));
    if let Some(input) = &datos.input_proyecto_ruta {
        let mut fila = div().h_flex().gap_2().child(div().flex_1().child(recuadro_input(Input::new(input))));
        if tipo == TipoUbicacion::Local {
            fila = fila.child(
                tema::boton_secundario("proy-elegir-carpeta", "Elegir…")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ElegirCarpetaProyecto), cx)
                    }),
            );
        }
        tarjeta = tarjeta.child(fila);
    }
    // Error de validación de frontera.
    if let Some(err) = &datos.proyecto_form_error {
        tarjeta = tarjeta.child(tema::texto_terciario(err.clone()).text_color(tema::SALMO));
    }
    // Botones.
    tarjeta = tarjeta.child(
        div()
            .h_flex()
            .gap_2()
            .child(
                tema::boton_primario("proy-form-ok", "Guardar")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ConfirmarFormProyecto), cx)
                    }),
            )
            .child(
                tema::boton_secundario("proy-form-cancelar", "Cancelar")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(CerrarFormProyecto), cx)
                    }),
            ),
    );

    // Backdrop que cierra al clicar fuera. `.id()` porque `on_click` exige un elemento con estado.
    div()
        .id("proyecto-form-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(0x00000099))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(CerrarFormProyecto), cx))
        .child(tarjeta)
        .into_any_element()
}

/// Envuelve un `Input` del kit en el recuadro Ethos (fondo TINTA + borde LINEA), como en peers.rs.
fn recuadro_input(input: gpui_component::input::Input) -> impl IntoElement {
    div()
        .w_full()
        .px_3()
        .py_1()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .bg(tema::TINTA)
        .border_1()
        .border_color(tema::LINEA)
        .child(input)
}

/// Chip de clase de ubicación (Local/SSH) en el formulario. `local` marca qué acción despacha.
fn chip_tipo(etiqueta: &str, activo: bool, es_local: bool) -> impl IntoElement {
    tema::fila_seleccionable(SharedString::from(format!("tipo-{etiqueta}")), activo)
        .px_3()
        .py_1()
        .rounded(tema::radio(tema::RADIO_PILL))
        .child(
            div()
                .text_color(if activo { tema::BRASA } else { tema::PAPEL })
                .child(SharedString::from(etiqueta.to_string())),
        )
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(ElegirTipoUbicacion { local: es_local }), cx)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Pantalla;

    /// La pantalla Proyectos está cableada en el enum con título e id propios (guardarraíl del
    /// registro: si se quita del array o se duplica un título, esto lo detecta).
    #[test]
    fn pantalla_proyectos_registrada() {
        assert!(Pantalla::TODAS.contains(&Pantalla::Proyectos));
        assert_eq!(Pantalla::Proyectos.titulo(), "Proyectos");
        let titulos: Vec<_> = Pantalla::TODAS.iter().map(|p| p.titulo()).collect();
        let unicos: std::collections::HashSet<_> = titulos.iter().collect();
        assert_eq!(titulos.len(), unicos.len(), "títulos de pantalla duplicados");
    }

    /// Los sub-tabs de la ficha cubren las 4 secciones del RFC (R8) con títulos únicos.
    #[test]
    fn ficha_tiene_cuatro_subtabs() {
        assert_eq!(FichaProyectoTab::TODAS.len(), 4);
        assert_eq!(FichaProyectoTab::default(), FichaProyectoTab::Equipo);
        let t: Vec<_> = FichaProyectoTab::TODAS.iter().map(|x| x.titulo()).collect();
        assert_eq!(t, vec!["Equipo", "Tablero", "Actividad", "Alertas"]);
    }
}
