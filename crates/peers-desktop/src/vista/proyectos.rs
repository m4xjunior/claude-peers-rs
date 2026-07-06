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
use peers_core::Instancia;

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

/// Lanza un agente del proyecto (R9): navega al Lanzador con `id_agente` (rol@proyecto) y la ruta
/// del proyecto precargados. AppDesktop hace la navegación + precarga; Max dispara el lanzamiento.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct LanzarAgente {
    pub id_agente: String,
    /// Ruta local del proyecto (para precargar el dir del Lanzador). Vacío si la ubicación no es local.
    pub ruta: String,
}

/// Alterna el AISLAMIENTO del proyecto `id` (R12): activar/desactivar las reglas de política que
/// impiden que agentes de OTROS proyectos le hablen. `AppDesktop` lee la política del broker, aplica
/// `con_aislamiento` y la guarda. `aislar` = estado deseado (true = aislar, false = quitar).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct AlternarAislamientoProyecto {
    pub id: String,
    pub aislar: bool,
}

/// Abre el overlay de "importar peers vivos" al equipo del proyecto abierto (ficha, sub-tab Equipo).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct AbrirImportarPeers;

/// Cierra el overlay de importar sin aplicar.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct CerrarImportarPeers;

/// Marca/desmarca el peer `id` en el overlay de importar (multi-select).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct AlternarPeerImport {
    pub id: String,
}

/// Confirma: añade los peers marcados al equipo del proyecto abierto (`agregar_agentes_a_proyecto`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct ConfirmarImportarPeers;

/// Pide BORRAR el proyecto `id` (R6, acción destructiva): abre la confirmación en 2 pasos. El borrado
/// real solo ocurre al confirmar. NO toca la bitácora del broker, solo la config local.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct PedirBorrarProyecto {
    pub id: String,
}

/// Cancela la confirmación de borrado (vuelve a la ficha normal).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct CancelarBorrarProyecto;

/// Confirma el borrado del proyecto en confirmación (`Config::borrar_proyecto`) y vuelve al grid.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = proyectos, no_json)]
pub struct ConfirmarBorrarProyecto;

// --- Aislamiento de proyecto (R12): generación de reglas de política. PUROS y testeables. ---
//
// Semántica confirmada por Max: (B) SOLO ENTRADA. Aislar el proyecto X significa "nadie de afuera le
// habla a X, pero X sí puede hablar hacia afuera". Se traduce a DOS reglas, en este orden (el motor
// evalúa arriba-abajo, primera que casa gana — como un firewall):
//   1. `@X → @X : permitir`  (los agentes del propio proyecto SÍ se comunican entre ellos)
//   2. `*  → @X : bloquear`  (cualquier OTRO emisor hacia X queda bloqueado)
// El orden importa: la de permitir-dentro va ANTES que la de bloquear-todo, si no, los propios
// agentes de X quedarían bloqueados por la regla 2.

/// Las dos reglas de aislamiento (entrada) del proyecto `proyecto`, en el orden correcto.
fn reglas_aislamiento(proyecto: &str) -> Vec<peers_core::ReglaComunicacion> {
    use peers_core::{AccionPolitica, Patron, ReglaComunicacion};
    let grupo = Patron::Grupo(proyecto.to_string());
    vec![
        ReglaComunicacion {
            de: grupo.clone(),
            para: grupo.clone(),
            accion: AccionPolitica::Permitir,
            motivo: None,
        },
        ReglaComunicacion {
            de: Patron::Cualquiera,
            para: grupo,
            accion: AccionPolitica::Bloquear,
            motivo: Some(format!("proyecto '{proyecto}' aislado (necesidad de saber)")),
        },
    ]
}

/// ¿El proyecto `proyecto` está aislado en `politica`? (contiene la regla de bloqueo de entrada).
#[must_use]
pub fn proyecto_aislado(politica: &peers_core::Politica, proyecto: &str) -> bool {
    use peers_core::{AccionPolitica, Patron};
    let grupo = Patron::Grupo(proyecto.to_string());
    politica.reglas.iter().any(|r| {
        r.para == grupo && r.de == Patron::Cualquiera && r.accion == AccionPolitica::Bloquear
    })
}

/// Devuelve una NUEVA política con el aislamiento de `proyecto` en el estado `aislar`. Idempotente:
/// primero QUITA cualquier regla de aislamiento previa de ese proyecto (para no duplicar ni dejar
/// restos), y si `aislar` es true, ANTEPONE las dos reglas nuevas (deben ir arriba para ganar sobre
/// reglas más genéricas). Las reglas de OTROS proyectos y las manuales se conservan intactas.
#[must_use]
pub fn con_aislamiento(
    politica: &peers_core::Politica,
    proyecto: &str,
    aislar: bool,
) -> peers_core::Politica {
    use peers_core::Patron;
    let grupo = Patron::Grupo(proyecto.to_string());
    // Quita las reglas cuyo `para` es este grupo (las de aislamiento de ESTE proyecto), conservando
    // todo lo demás. Así togglear es idempotente y no acumula duplicados.
    let mut reglas: Vec<_> = politica
        .reglas
        .iter()
        .filter(|r| r.para != grupo)
        .cloned()
        .collect();
    if aislar {
        // Antepone (las reglas de aislamiento deben evaluarse antes que reglas `*` genéricas).
        let mut nuevas = reglas_aislamiento(proyecto);
        nuevas.extend(reglas);
        reglas = nuevas;
    }
    peers_core::Politica { reglas, accion_por_defecto: politica.accion_por_defecto }
}

// --- Filtrado por proyecto activo (R11). PUROS y reutilizables por Peers/Tareas/Jornada/Alertas. ---

/// ¿El id `id` pertenece al proyecto activo? (R11, aislamiento por convención de id). Con
/// `activo = None` ("todos") SIEMPRE pasa. Con `Some(proyecto)`, pasa si el id termina en
/// `@<proyecto>` — la convención `rol@proyecto`. Un id sin `@` NO pertenece a ningún proyecto.
#[must_use]
pub fn id_del_proyecto_activo(id: &str, activo: Option<&str>) -> bool {
    match activo {
        None => true,
        Some(p) => id.ends_with(&format!("@{p}")),
    }
}

// NOTA de diseño (R11): Peers/Tareas/Alertas NO traducen índice visible→real, sino que PINTAN solo
// las filas del proyecto activo CONSERVANDO su índice REAL en la lista completa (el `idx` que viaja
// en las acciones sigue apuntando a `datos.X`). Así se evita el patrón de traducción y su clase de
// bugs. Alertas ya tenía un `indice_real` (por el filtro de tipo) que ahora AND-ea el de proyecto.

// --- Render ---

/// Render de la pantalla. Si hay un proyecto abierto → su ficha; si no → el grid + CRUD.
pub fn render_proyectos(datos: &EstadoPantalla) -> impl IntoElement {
    // El contenido (grid o ficha) va en un cuerpo SCROLLABLE (flex_1 + min_h_0 + overflow_y_scroll),
    // así la ficha con timeline largo o el grid con muchas cards no desbordan (hallazgo UI #3).
    let contenido = match &datos.proyecto_abierto {
        Some(id) => match datos.proyectos.iter().find(|p| &p.id == id) {
            Some(p) => ficha(datos, p).into_any_element(),
            // El proyecto abierto ya no existe (borrado): cae al grid.
            None => grid(datos).into_any_element(),
        },
        None => grid(datos).into_any_element(),
    };
    let cuerpo = div()
        .id("proyectos-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(contenido);

    // Raíz `relative` (size_full por fondo_app) para que los overlays `absolute().inset_0()` se
    // posicionen respecto a ESTA vista, no al viewport. Los overlays van a nivel raíz (FUERA del
    // scroll) para cubrir la vista fija y no scrollear con el contenido.
    tema::fondo_app()
        .relative()
        .v_flex()
        .child(cuerpo)
        .when_some(datos.proyecto_form.as_ref(), |el, _| el.child(overlay_form(datos)))
        .when(datos.proyecto_import_abierto, |el| el.child(overlay_importar_peers(datos)))
        .when_some(datos.proyecto_borrar_confirmar.clone(), |el, id| el.child(overlay_borrar(&id)))
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
    // El overlay del formulario (crear/editar) se monta en la RAÍZ (render_proyectos), no aquí,
    // para que quede fuera del cuerpo scrollable y se posicione respecto a la vista.
    cont
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
    // Los overlays (importar/borrar) se montan en la RAÍZ (render_proyectos), fuera del cuerpo
    // scrollable, para que se posicionen respecto a la vista y no scrolleen con la ficha.
    div()
        .v_flex()
        .p_8()
        .gap_4()
        .child(ficha_cabecera(p))
        .child(barra_tabs(datos.proyecto_ficha_tab))
        .child(seccion_activa(datos, p))
        .child(control_aislamiento(datos, p))
}

/// Overlay de confirmación de BORRADO de proyecto (R6, destructivo, 2 pasos). Clic fuera = cancelar.
fn overlay_borrar(id: &str) -> impl IntoElement {
    let id = id.to_string();
    let tarjeta = tema::superficie_card()
        .occlude()
        .v_flex()
        .gap_3()
        .p_5()
        .w(tema::radio(440.0))
        .child(tema::titulo("¿Borrar este proyecto?"))
        .child(tema::texto_terciario(
            "Se elimina de tu config local (nombre, ubicación, equipo declarado). NO borra la \
             bitácora del broker ni afecta a los agentes vivos. No se puede deshacer.",
        ))
        .child(
            div()
                .h_flex()
                .gap_2()
                .justify_end()
                .child(
                    tema::boton_secundario("borrar-cancelar", "Cancelar").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(CancelarBorrarProyecto), cx)
                    }),
                )
                .child(
                    tema::boton_primario("borrar-confirmar", "Sí, borrar")
                        .text_color(tema::SALMO)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(ConfirmarBorrarProyecto), cx)
                        }),
                ),
        );
    div()
        .id(SharedString::from(format!("borrar-backdrop-{id}")))
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(0x00000099))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(CancelarBorrarProyecto), cx))
        .child(tarjeta)
}

/// Control de AISLAMIENTO del proyecto (R12): toggle que activa/desactiva las reglas de política
/// (`* → @X: bloquear` + `@X → @X: permitir`). Aislado = nadie de otro proyecto puede escribirle a
/// los agentes de X, ni por el canal normal ni por el chat privado (necesidad de saber). El estado
/// se lee de `proyecto_politica`; el clic despacha `AlternarAislamientoProyecto` con el nuevo valor.
fn control_aislamiento(datos: &EstadoPantalla, p: &Proyecto) -> impl IntoElement {
    let aislado = datos
        .proyecto_politica
        .as_ref()
        .map(|pol| proyecto_aislado(pol, &p.id))
        .unwrap_or(false);
    let id = p.id.clone();
    let (etiqueta, glosa) = if aislado {
        ("Aislado ✓ — quitar aislamiento", "Nadie de otros proyectos puede escribirle a este equipo (ni chat normal ni privado).")
    } else {
        ("Aislar este proyecto", "Bloquea que agentes de otros proyectos le escriban a este equipo (necesidad de saber).")
    };
    let nuevo_estado = !aislado;
    tema::superficie_card()
        .v_flex()
        .gap_2()
        .p_4()
        .child(tema::eyebrow("aislamiento (R12)"))
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_3()
                .child(
                    tema::boton_secundario("proy-aislar", etiqueta).on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(AlternarAislamientoProyecto { id: id.clone(), aislar: nuevo_estado }),
                            cx,
                        )
                    }),
                )
                .child(tema::texto_terciario(glosa)),
        )
}

/// Cabecera de la ficha: botón volver + nombre BRASA + ubicación mono.
fn ficha_cabecera(p: &Proyecto) -> impl IntoElement {
    let id_borrar = p.id.clone();
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
                .flex_1()
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
        // Borrar (R6, destructivo): abre confirmación en 2 pasos. A la derecha, separado.
        .child(
            tema::boton_secundario("proy-borrar", "Borrar")
                .text_color(tema::SALMO)
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(PedirBorrarProyecto { id: id_borrar.clone() }), cx)
                }),
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
        FichaProyectoTab::Actividad => seccion_actividad(datos).into_any_element(),
        FichaProyectoTab::Alertas => seccion_alertas(datos, &sufijo).into_any_element(),
    }
}

/// Equipo: los agentes del proyecto con su estado vivo (cruzando `instancias`).
fn seccion_equipo(datos: &EstadoPantalla, p: &Proyecto) -> impl IntoElement {
    use crate::config::Ubicacion;
    // Ruta local del proyecto (para precargar el dir del Lanzador). Solo si la ubicación es Local.
    let ruta_local = match &p.ubicacion {
        Ubicacion::Local { ruta } => ruta.clone(),
        _ => String::new(),
    };
    let mut cont = tema::superficie_card().v_flex().gap_2().p_4();
    // Equipo vacío: mensaje guía como fila (NO early-return — el botón "+ Importar" debe seguir
    // visible, que es justo el caso de uso de un proyecto recién creado sin equipo).
    if p.agentes.is_empty() {
        cont = cont.child(tema::texto_terciario(
            "Este proyecto aún no tiene equipo. Impórtale un peer vivo abajo, o lánzalo desde el \
             Lanzador con id rol@proyecto.",
        ));
    }
    for id in &p.agentes {
        let vivo = datos.instancias.iter().any(|i| &i.id == id);
        let (texto_estado, color) = if vivo {
            ("vivo", tema::BRASA)
        } else {
            ("caído", tema::HUMO)
        };
        let id_owned = id.clone();
        let ruta = ruta_local.clone();
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
                .child(tema::chip_estado(texto_estado, color))
                // "Lanzar" (R9): navega al Lanzador con este id (rol@proyecto) + la ruta precargados.
                .child(
                    tema::boton_secundario(format!("lanzar-{id}"), "Lanzar")
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(LanzarAgente { id_agente: id_owned.clone(), ruta: ruta.clone() }),
                                cx,
                            )
                        }),
                ),
        );
    }
    // Botón para importar peers vivos que NO se lanzaron desde el Lanzador con rol@proyecto.
    cont.child(
        tema::boton_secundario("proy-importar-peers", "+ Importar peers vivos").on_click(
            |_, window, cx| window.dispatch_action(Box::new(AbrirImportarPeers), cx),
        ),
    )
}

/// Overlay de "importar peers vivos" (ficha, sub-tab Equipo): checkboxes de los peers vivos que aún
/// NO están en el equipo del proyecto abierto. Confirmar → `agregar_agentes_a_proyecto`. Clic fuera
/// cierra. Solo se monta cuando `proyecto_import_abierto`.
fn overlay_importar_peers(datos: &EstadoPantalla) -> gpui::AnyElement {
    use gpui::IntoElement as _;
    if !datos.proyecto_import_abierto {
        return div().into_any_element();
    }
    // Equipo actual del proyecto abierto (para no ofrecer los que ya están).
    let ya_en_equipo: std::collections::HashSet<&str> = datos
        .proyecto_abierto
        .as_deref()
        .and_then(|id| datos.proyectos.iter().find(|p| p.id == id))
        .map(|p| p.agentes.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let candidatos: Vec<&Instancia> = datos
        .instancias
        .iter()
        .filter(|i| !ya_en_equipo.contains(i.id.as_str()))
        .collect();

    let mut lista =
        div().id("importar-peers-lista").v_flex().gap_1().max_h(tema::radio(360.0)).overflow_y_scroll();
    if candidatos.is_empty() {
        lista = lista.child(tema::texto_terciario(
            "No hay peers vivos fuera de este equipo para importar.",
        ));
    } else {
        for inst in candidatos {
            let id = inst.id.clone();
            let marcado = datos.proyecto_import_seleccion.contains(&id);
            let id_click = id.clone();
            lista = lista.child(
                tema::fila_seleccionable(SharedString::from(format!("import-{id}")), marcado)
                    .px_3()
                    .py_1()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_color(if marcado { tema::BRASA } else { tema::HUMO })
                            .child(if marcado { "☑" } else { "☐" }),
                    )
                    .child(
                        div()
                            .font_family(tema::FUENTE_MONO)
                            .text_color(tema::PAPEL)
                            .child(SharedString::from(id.clone())),
                    )
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(
                            Box::new(AlternarPeerImport { id: id_click.clone() }),
                            cx,
                        )
                    }),
            );
        }
    }

    let n = datos.proyecto_import_seleccion.len();
    let tarjeta = tema::superficie_card()
        .occlude()
        .v_flex()
        .gap_3()
        .p_5()
        .w(tema::radio(480.0))
        .child(tema::titulo("Importar peers al equipo"))
        .child(tema::texto_terciario(
            "Marca los peers vivos que pertenecen a este proyecto (aunque no se lanzaron con rol@proyecto).",
        ))
        .child(lista)
        .child(
            div()
                .h_flex()
                .gap_2()
                .justify_end()
                .child(
                    tema::boton_secundario("import-cancelar", "Cancelar").on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(CerrarImportarPeers), cx)
                    }),
                )
                .child(
                    tema::boton_primario("import-confirmar", format!("Añadir ({n})")).on_click(
                        |_, window, cx| {
                            window.dispatch_action(Box::new(ConfirmarImportarPeers), cx)
                        },
                    ),
                ),
        );

    div()
        .id("importar-peers-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(0x00000099))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(CerrarImportarPeers), cx))
        .child(tarjeta)
        .into_any_element()
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
fn seccion_actividad(datos: &EstadoPantalla) -> impl IntoElement {
    // Empalme: la actividad agregada del equipo (pares agente→acciones, cargados al abrir la ficha)
    // se cruza cronológicamente con `jornada::timeline_agregado` (componente reusable de s007).
    if datos.proyecto_actividad.is_empty() {
        return tema::superficie_card().v_flex().gap_2().p_4().child(tema::texto_terciario(
            "Sin actividad registrada para el equipo de este proyecto (o el equipo está vacío).",
        ));
    }
    tema::superficie_card()
        .v_flex()
        .gap_2()
        .p_4()
        .child(crate::vista::jornada::timeline_agregado(&datos.proyecto_actividad))
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
    use peers_core::{AccionPolitica, Patron, Politica, ReglaComunicacion};

    /// R12: `con_aislamiento` genera las 2 reglas en el orden correcto (permitir-dentro ANTES de
    /// bloquear-todo), `proyecto_aislado` las detecta, y togglear off las quita sin tocar el resto.
    #[test]
    fn aislamiento_genera_reglas_correctas_e_idempotente() {
        let vacia = Politica::default();
        assert!(!proyecto_aislado(&vacia, "web"));

        // Aislar "web": 2 reglas, permitir-dentro primero, bloquear-todo después.
        let aislada = con_aislamiento(&vacia, "web", true);
        assert!(proyecto_aislado(&aislada, "web"));
        assert_eq!(aislada.reglas.len(), 2);
        assert_eq!(aislada.reglas[0].de, Patron::Grupo("web".into()));
        assert_eq!(aislada.reglas[0].para, Patron::Grupo("web".into()));
        assert_eq!(aislada.reglas[0].accion, AccionPolitica::Permitir);
        assert_eq!(aislada.reglas[1].de, Patron::Cualquiera);
        assert_eq!(aislada.reglas[1].para, Patron::Grupo("web".into()));
        assert_eq!(aislada.reglas[1].accion, AccionPolitica::Bloquear);

        // Idempotente: volver a aislar no duplica (sigue con 2 reglas).
        let re_aislada = con_aislamiento(&aislada, "web", true);
        assert_eq!(re_aislada.reglas.len(), 2);

        // Quitar el aislamiento: vuelve a 0 reglas de "web".
        let quitada = con_aislamiento(&aislada, "web", false);
        assert!(!proyecto_aislado(&quitada, "web"));
        assert!(quitada.reglas.is_empty());
    }

    /// R12: aislar un proyecto NO toca las reglas de otro proyecto ni las manuales.
    #[test]
    fn aislamiento_no_pisa_otras_reglas() {
        // Parte con "api" ya aislado + una regla manual.
        let base = con_aislamiento(&Politica::default(), "api", true);
        let manual = ReglaComunicacion {
            de: Patron::Id("x".into()),
            para: Patron::Id("y".into()),
            accion: AccionPolitica::Bloquear,
            motivo: None,
        };
        let mut base = base;
        base.reglas.push(manual.clone());

        // Aislar "web" además: "api" y la manual siguen; "web" se suma.
        let con_web = con_aislamiento(&base, "web", true);
        assert!(proyecto_aislado(&con_web, "web"));
        assert!(proyecto_aislado(&con_web, "api"));
        assert!(con_web.reglas.contains(&manual));

        // Quitar "web" no afecta "api" ni la manual.
        let sin_web = con_aislamiento(&con_web, "web", false);
        assert!(!proyecto_aislado(&sin_web, "web"));
        assert!(proyecto_aislado(&sin_web, "api"));
        assert!(sin_web.reglas.contains(&manual));
    }

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

    /// El filtro por proyecto activo (R11) matchea por el sufijo `@proyecto` del id; `None` = todos.
    #[test]
    fn filtro_por_proyecto_activo() {
        // Sin filtro: todo pasa.
        assert!(id_del_proyecto_activo("backend@web", None));
        assert!(id_del_proyecto_activo("cualquier-id", None));
        // Con filtro: solo los del proyecto.
        assert!(id_del_proyecto_activo("backend@web", Some("web")));
        assert!(id_del_proyecto_activo("qa@web", Some("web")));
        assert!(!id_del_proyecto_activo("backend@otro", Some("web")));
        assert!(!id_del_proyecto_activo("sin-sufijo", Some("web")));
        // Cuidado con prefijos: "web" no debe matchear "@webapp".
        assert!(!id_del_proyecto_activo("rol@webapp", Some("web")));
    }

}
