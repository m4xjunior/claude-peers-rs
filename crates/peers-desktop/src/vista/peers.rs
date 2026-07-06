//! Pantalla Peers — tabla viva de instancias de la red claude-peers, rediseñada con el tema Ethos.
//!
//! INTENCIÓN: porta la pantalla 1 de la TUI (`peers-tui/src/ui/peers.rs`), que pinta una tabla
//! `(id, directorio, resumen, visto)` desde `POST /listar`. Aquí se AÑADE (feature documentada)
//! una columna de ESTADO por peer — ocioso / atascado / trabajando — derivada cruzando las
//! alertas vigentes del supervisor con cada peer por `sujeto == id`. Es información que la TUI ya
//! tenía repartida en la pantalla Alertas; aquí se consolida junto al peer para verla de un vistazo.
//!
//! REDISEÑO ETHOS (por qué se reescribió): la versión previa usaba literales azules/genéricos
//! (`0x3a3a3a`, `0x1c1c1c`…) y no era operable (filas no seleccionables, sin acciones). Ahora:
//!   1) TODO el color sale del `tema` (TINTA/PAPEL/BRASA/LINEA…), sin literales sueltos salvo los
//!      colores de severidad de estado (que son semántica de dominio, no del tema).
//!   2) La tabla vive dentro de una `superficie_card`, con `eyebrow` para los labels de columna
//!      (mono/humo/mayúsculas) y tipografía Inter para los datos.
//!   3) Cada fila es SELECCIONABLE (`tema::fila_seleccionable`): al clicar se marca el peer y se
//!      despacha `SeleccionarPeer`. Con un peer marcado aparece una barra de acciones: "Enviar
//!      mensaje" y "Ver jornada" (trazabilidad), que despachan sus Action.
//!
//! DECISIÓN DE DISEÑO (por qué NO uso `Table`/`Badge`/`Button` del kit): coherente con el resto de
//! las vistas y con la Fundación. El `Table` del kit es STATEFUL (exige `Entity<TableState>` +
//! `TableDelegate` + `Context`), incompatible con la firma pura `render_peers(&EstadoPantalla)`. La
//! tabla se construye con filas flex (`tema::fila_seleccionable`, que ya trae `.id()` y hover). Los
//! botones de acción son los `tema::boton_*` (Stateful<Div>), no el `Button` del kit, para mantener
//! el look Ethos exacto. La vista es PURA: sólo LEE `EstadoPantalla` y DESPACHA acciones; toda
//! mutación (marcar peer, disparar fetch, navegar a trazabilidad) vive en `AppDesktop` (Fase 3).

use gpui::{
    div, actions, px, rgb, rgba, Action, AnyElement, InteractiveElement, IntoElement,
    ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled,
};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::{input::Input, StyledExt};

use peers_core::{Alerta, Instancia, TipoAlerta};

use crate::app::EstadoPantalla;
use crate::tema;

// -------------------------------------------------------------------------------------------------
// ACCIONES — la vista es pura (sin `cx`), así que DESPACHA acciones que `AppDesktop` maneja con
// `.on_action(cx.listener(...))` en su contenedor raíz (mismo patrón que `vista/alertas.rs`). GPUI
// las hace burbujear por el árbol desde el elemento clicado hasta ese manejador. El payload lleva
// lo mínimo para que el manejador actúe sin volver a mirar el estado de la vista. Namespace `peers`
// para no colisionar con las de otras pantallas.
//
// `no_json`: estas acciones SÓLO se despachan por código (`window.dispatch_action`), nunca desde un
// keymap, así que no arrastran `serde::Deserialize` + `schemars::JsonSchema` (basta Clone+PartialEq).
// -------------------------------------------------------------------------------------------------

/// Seleccionar el peer de la fila `indice` (click en la fila). Índice en `datos.instancias`.
/// `AppDesktop` guarda la selección (campo NUEVO `peers_seleccion`, ver notas de Fase 3) y resalta
/// la fila. Es la acción base que habilita la barra de acciones del peer marcado.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = peers, no_json)]
pub struct SeleccionarPeer {
    pub indice: usize,
}

/// Enviar un mensaje al peer `id` (tecla `m` de la TUI / botón "Enviar mensaje"). `AppDesktop`
/// abrirá el flujo de composición y, al confirmar, hará `POST /enviar` vía `ClienteBroker::reenviar`
/// (o el método de envío directo del cliente). Lleva el `id` completo para no depender del índice
/// (que puede reordenarse entre recargas). El campo `id` es la identidad estable del peer.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = peers, no_json)]
pub struct EnviarMensajePeer {
    pub id: String,
}

/// Ver la jornada/trazabilidad del peer `id` (botón "Ver jornada"). `AppDesktop` fija
/// `traza_peer = Some(id)`, dispara `ClienteBroker::historial` para poblar `historial` y navega a
/// la pantalla Trazabilidad. Lleva el `id` completo por la misma razón que arriba.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = peers, no_json)]
pub struct VerJornadaPeer {
    pub id: String,
}

// Acción sin datos: limpiar la selección de peer (botón "Cerrar" de la barra de acciones).
actions!(peers, [DeseleccionarPeer]);

// Acciones sin datos del detalle y los formularios de peers (peers-01/02/03): cerrar el pop-up de
// detalle, cerrar el formulario activo (composer/kick) sin confirmar, y confirmarlo (enviar/kick).
actions!(peers, [CerrarDetallePeer, CerrarFormPeers, ConfirmarFormPeers]);

/// Abrir el pop-up de DETALLE del peer `indice` (peers-01): doble-click en la fila, botón "Abrir"
/// o Enter. `AppDesktop` guarda `Some(indice)` en `peer_detalle` y monta el overlay en su render
/// raíz (patrón idéntico a `tareas::AbrirDetalleTarea` → `overlay_tarea`).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = peers, no_json)]
pub struct AbrirDetallePeer {
    pub indice: usize,
}

/// Pedir la EXPULSIÓN del peer `id` (peers-03): abre el mini-modal de confirmación (acción
/// destructiva — cierra la presencia del peer en la red). El POST real (`/salir`) sólo se dispara
/// al confirmar con `ConfirmarFormPeers`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = peers, no_json)]
pub struct PedirKickPeer {
    pub id: String,
}

/// Qué formulario de la pestaña Peers está abierto (peers-02/03). Vive en
/// `EstadoPantalla.peers_form` (`None` = ninguno). Cada variante captura el `id` del peer al
/// abrir para no depender del índice (la lista puede reordenarse entre recargas).
#[derive(Clone, PartialEq)]
pub enum FormPeers {
    /// peers-02: componer y enviar un mensaje al peer (el texto vive en `input_mensaje_peer`).
    Mensaje { id: String },
    /// peers-03: confirmación de expulsión (destructiva, 2 pasos).
    Kick { id: String },
}

// -------------------------------------------------------------------------------------------------
// ESTADO OPERATIVO DEL PEER — enum cerrado (no "stringly typed") para que etiqueta y color salgan
// de un único match y no se desincronicen. `Trabajando` es el caso por defecto.
// -------------------------------------------------------------------------------------------------

/// Estado operativo de un peer, derivado de las alertas vigentes.
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

    /// Color del chip. Es SEMÁNTICA DE DOMINIO (severidad del estado), no un token del tema, por eso
    /// vive aquí y no en `tema`. Se afinan al fondo cálido del Ethos: ámbar/naranja/verde apagados
    /// que armonizan con el dorado brasa sin gritar. El texto del chip es SALMO (lo pone `tema`).
    fn color(self) -> Rgba {
        match self {
            EstadoPeer::Ocioso => rgba(0xD9A021FF),     // ámbar (atención suave)
            EstadoPeer::Atascado => rgba(0xD9542BFF),   // naranja-rojo (más severo)
            EstadoPeer::Trabajando => rgba(0x7FA86AFF), // verde apagado (normal, no compite con brasa)
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

// -------------------------------------------------------------------------------------------------
// LAYOUT — anchos fijos (px) de columnas no flexibles, espejo de los Constraint de la TUI. `resumen`
// es la flexible (`flex_1`); las demás llevan ancho fijo para que los chips de estado se alineen.
// -------------------------------------------------------------------------------------------------

const COL_ID: f32 = 150.0;
const COL_DIR: f32 = 260.0;
const COL_VISTO: f32 = 96.0;
const COL_ESTADO: f32 = 120.0;

// -------------------------------------------------------------------------------------------------
// HELPERS DE CELDA — reparten con el vocabulario del tema. Los datos van en Inter (heredado del
// fondo); los timestamps en mono (`tema::FUENTE_MONO`) para alineación de dígitos.
// -------------------------------------------------------------------------------------------------

/// Celda de texto de ancho fijo (directorio). Texto PAPEL, tamaño base. `.truncate()` (craft P2):
/// mismo fix que la celda de id — un directorio/anotación de repo largo no debe partirse por wrap.
fn celda(texto: impl Into<SharedString>, ancho: f32) -> impl IntoElement {
    div()
        .w(px(ancho))
        .truncate()
        .px_1()
        .text_color(tema::PAPEL)
        .child(texto.into())
}

/// Celda flexible (ocupa el espacio sobrante) para `resumen`. Se atenúa a HUMO por ser metadato
/// secundario, y recorta con `overflow_hidden` para no romper el layout con resúmenes largos.
fn celda_flex(texto: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .overflow_hidden()
        .px_1()
        .text_color(tema::HUMO)
        .child(texto.into())
}

/// Celda de timestamp: mono para que los dígitos alineen, atenuada a HUMO.
fn celda_hora(texto: impl Into<SharedString>) -> impl IntoElement {
    div()
        .w(px(COL_VISTO))
        .px_1()
        .font_family(tema::FUENTE_MONO)
        .text_color(tema::HUMO)
        .text_xs()
        .child(texto.into())
}

/// Fila de encabezado: labels de columna como `eyebrow` (mono/humo/mayúsculas), separada por LINEA.
fn encabezado() -> impl IntoElement {
    div()
        .h_flex()
        .w_full()
        .items_center()
        .px_3()
        .py_2()
        .gap_2()
        .border_b_1()
        .border_color(tema::LINEA)
        .child(div().w(px(COL_ID)).px_1().child(tema::eyebrow("id")))
        .child(div().w(px(COL_DIR)).px_1().child(tema::eyebrow("directorio")))
        .child(div().flex_1().px_1().child(tema::eyebrow("resumen")))
        .child(div().w(px(COL_VISTO)).px_1().child(tema::eyebrow("visto")))
        .child(div().w(px(COL_ESTADO)).px_1().child(tema::eyebrow("estado")))
}

/// Una fila de peer: seleccionable (marca el peer al clicar). Compone `tema::fila_seleccionable`
/// (que trae `.id()`, hover, borde de selección) + las 4 celdas de la TUI + el chip de estado.
/// `indice` viaja en la Action para que `AppDesktop` sepa qué peer marcar sin re-buscar por id.
fn fila(indice: usize, inst: &Instancia, estado: EstadoPeer, activa: bool) -> impl IntoElement {
    // Espejo de `fila_peer` de la TUI: mismos campos y misma extracción de la hora del `visto_en`.
    let visto = hora_iso(&inst.visto_en);
    let repo = inst.repo_git.as_deref().unwrap_or("");

    // El directorio se muestra tal cual; si hay repo git DISTINTO del directorio, se anota entre
    // paréntesis como pista (craft P2, s005): `repo_git` es `git rev-parse --show-toplevel` — si
    // el peer arrancó en el ROOT del repo (el caso común), `repo == directorio` y el paréntesis
    // sólo repetía la misma ruta ("…/claude-peers-rs (…/claude-peers-rs)"). Sólo aporta cuando el
    // peer arrancó en un SUBDIRECTORIO del repo (`directorio` != `repo_git`).
    let dir = if repo.is_empty() || repo == inst.directorio {
        inst.directorio.clone()
    } else {
        format!("{} ({repo})", inst.directorio)
    };

    tema::fila_seleccionable(SharedString::from(format!("peer-fila-{indice}")), activa)
        // El id del peer se destaca en dorado brasa: es la identidad, el ancla de la fila.
        // `.truncate()` (craft P2, s005): un id largo partía la palabra a media sílaba por wrap
        // ("app-planificacion-servi\ndor") — nunca se debe romper un identificador. El id íntegro
        // sigue disponible en el modal de detalle y en "Copiar id" (bug #4).
        .child(
            div()
                .w(px(COL_ID))
                .truncate()
                .px_1()
                .text_color(tema::BRASA)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(SharedString::from(inst.id.clone())),
        )
        .child(celda(dir, COL_DIR))
        .child(celda_flex(inst.resumen.clone()))
        .child(celda_hora(visto))
        .child(
            div()
                .w(px(COL_ESTADO))
                .px_1()
                .child(tema::chip_estado(estado.etiqueta(), estado.color())),
        )
        .on_click(move |evento, window, cx| {
            // Doble-click abre el pop-up de detalle (peers-01); click simple sólo selecciona.
            // La vista no toca `cx`: despacha y `AppDesktop` muta el estado.
            if evento.click_count() >= 2 {
                window.dispatch_action(Box::new(AbrirDetallePeer { indice }), cx);
            } else {
                window.dispatch_action(Box::new(SeleccionarPeer { indice }), cx);
            }
        })
}

/// Indicador de salud del broker junto al título (peers-18): punto de color + etiqueta corta.
/// Deriva SÓLO de `datos.salud`/`datos.error_acceso` (ya cargados por `cargar_broker` para la
/// pestaña Broker) — cero llamada HTTP propia de Peers. Tres estados: vivo (verde salvia), caído/
/// degradado (terracota) y "—" mientras no ha llegado la primera respuesta (humo, ni ok ni error).
fn indicador_salud_broker(datos: &EstadoPantalla) -> impl IntoElement {
    const VERDE_OK: gpui::Rgba = gpui::Rgba { r: 0x8F as f32 / 255.0, g: 0xB0 as f32 / 255.0, b: 0x7B as f32 / 255.0, a: 1.0 };
    const ROJO_ERROR: gpui::Rgba = gpui::Rgba { r: 0xC9 as f32 / 255.0, g: 0x6A as f32 / 255.0, b: 0x5A as f32 / 255.0, a: 1.0 };

    let (color, etiqueta) = match (&datos.error_acceso, &datos.salud) {
        (Some(_), _) => (ROJO_ERROR, "broker caído".to_string()),
        (None, Some(s)) => (VERDE_OK, format!("vivo · {} instancia(s)", s.instancias)),
        (None, None) => (tema::HUMO, "—".to_string()),
    };

    div()
        .h_flex()
        .items_center()
        .gap_2()
        .child(div().w(px(8.0)).h(px(8.0)).rounded(px(999.0)).bg(color))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_xs()
                .text_color(tema::HUMO)
                .child(SharedString::from(etiqueta)),
        )
}

/// Banner de error: se pinta cuando la última carga contra el broker falló, para que la tabla
/// vacía no se confunda con "no hay peers". El texto lo aporta `ErrorBroker::Display`. Colores de
/// severidad (rojo tenue) — semántica de error, no token del tema.
fn banner_error(err: &crate::cliente::ErrorBroker) -> impl IntoElement {
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .border_1()
        .border_color(rgba(0x7F1D1DFF))
        .bg(rgba(0x7F1D1D33))
        .text_color(rgba(0xFEE2E2FF))
        .child(SharedString::from(format!("⚠ Broker: {err}")))
}

/// Compone el identificador de una línea de un peer para copiar al portapapeles (bug #4 de Max:
/// "conectar visualmente una tarea con el agente que la ejecuta" — copiar el id + contexto en un
/// clic y pegarlo donde Max lo necesite, ej. junto a la tarea que ese peer ejecuta).
///
/// DECISIÓN sobre "jornada" (el pedido de Max era "id + jornada + descripción + PID"): `Instancia`
/// NO trae un campo de jornada — eso es `GET /jornada` (sesiones+tareas completas, una llamada HTTP
/// aparte, pensada para la pantalla Jornada, no para un identificador de un clic). Pedirla por cada
/// fila para armar un string a copiar sería una llamada de red innecesaria para algo que debe ser
/// INSTANTÁNEO. Se usa `visto_en` (último latido, YA en caché local) como el dato de "actividad/
/// jornada" más cercano sin red extra — es lo que la tabla y el resto de la app ya muestran como
/// pulso del peer. Si Max quiere el detalle completo de sesiones, ya existe el botón "Ver jornada".
fn identificador_peer(inst: &Instancia) -> String {
    format!(
        "{} · pid {} · {} · {}",
        inst.id,
        inst.pid,
        if inst.resumen.is_empty() {
            "(sin resumen)"
        } else {
            &inst.resumen
        },
        hora_iso(&inst.visto_en),
    )
}

/// Barra de acciones del peer seleccionado: aparece bajo la tabla cuando hay un peer marcado.
/// Muestra el id del peer en foco y los botones Abrir (detalle, peers-01), Enviar mensaje
/// (composer, peers-02), Ver jornada, Copiar id (bug #4), Expulsar (kick con confirmación,
/// peers-03) y "Cerrar" para soltar la selección. Espejo de las teclas de la TUI (`m`/`k`)
/// elevadas a botones visibles. `indice` viaja en `AbrirDetallePeer` (posición en `datos.instancias`).
///
/// PISTA DE ATAJOS (peers-18, variante B de la RFC): línea mono/humo bajo la barra con los atajos
/// de teclado disponibles sobre el peer seleccionado. NO son tooltips reales (variante A) porque
/// `gpui_component::tooltip::ManagedTooltipExt` (el mecanismo para adjuntar un `Tooltip` a un
/// elemento) es `pub(crate)` — invisible fuera del crate del kit. Sólo el `Button` propio del kit
/// expone `.tooltip(...)`, y esta pantalla usa deliberadamente `tema::boton_*` (divs planos) para
/// mantener el look Ethos exacto, no el `Button` del kit. Reconstruir el overlay de tooltip a mano
/// (delay/posicionamiento/animación) sería sobre-ingeniería para algo que la propia RFC no marca
/// como bloqueante — la pista visible cubre la misma necesidad de descubribilidad.
fn barra_acciones(inst: &Instancia, indice: usize) -> impl IntoElement {
    let id_msg = inst.id.clone();
    let id_jornada = inst.id.clone();
    let id_kick = inst.id.clone();
    let id_copiar = identificador_peer(inst);

    let barra = tema::superficie_card()
        .h_flex()
        .items_center()
        .w_full()
        .px_4()
        .py_3()
        .gap_3()
        .child(tema::eyebrow("peer"))
        .child(
            div()
                .text_color(tema::BRASA)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(SharedString::from(inst.id.clone())),
        )
        // Empuja los botones a la derecha.
        .child(div().flex_1())
        .child(
            tema::boton_primario("peer-accion-abrir", "Abrir").on_click(move |_e, window, cx| {
                window.dispatch_action(Box::new(AbrirDetallePeer { indice }), cx);
            }),
        )
        .child(
            tema::boton_secundario("peer-accion-mensaje", "Enviar mensaje").on_click(
                move |_e, window, cx| {
                    window.dispatch_action(Box::new(EnviarMensajePeer { id: id_msg.clone() }), cx);
                },
            ),
        )
        .child(
            tema::boton_secundario("peer-accion-jornada", "Ver jornada").on_click(
                move |_e, window, cx| {
                    window.dispatch_action(Box::new(VerJornadaPeer { id: id_jornada.clone() }), cx);
                },
            ),
        )
        // Copiar id (bug #4 de Max): compone el identificador de una línea y lo escribe al
        // portapapeles del sistema vía GPUI (cero deps, mismo mecanismo que `lanzador.rs`). Al ser
        // una acción puramente local (no toca el broker), NO se despacha por Action: se resuelve
        // aquí mismo con `cx.write_to_clipboard`, igual que cualquier otro `on_click` de esta vista
        // pura (el closure ya recibe `cx` sin necesitar `Entity`/estado propio).
        .child(
            tema::boton_secundario("peer-accion-copiar-id", "Copiar id").on_click(
                move |_e, _window, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(id_copiar.clone()));
                },
            ),
        )
        // Kick (peers-03): botón PELIGRO (rojo terroso, no brasa) — la confirmación la abre
        // `PedirKickPeer`; el POST sólo sale al confirmar.
        .child(boton_peligro("peer-accion-kick", "Expulsar").on_click(move |_e, window, cx| {
            window.dispatch_action(Box::new(PedirKickPeer { id: id_kick.clone() }), cx);
        }))
        .child(
            tema::boton_secundario("peer-accion-cerrar", "Cerrar").on_click(|_e, window, cx| {
                window.dispatch_action(Box::new(DeseleccionarPeer), cx);
            }),
        );

    div()
        .v_flex()
        .w_full()
        .gap_1()
        .child(barra)
        .child(
            div()
                .px_1()
                .font_family(tema::FUENTE_MONO)
                .text_xs()
                .text_color(tema::HUMO)
                .child(SharedString::from(
                    "m mensaje · k expulsar · r jornada · ↵ detalle",
                )),
        )
}

/// Render de la pantalla Peers. Consume `instancias` y `alertas` ya cargadas por la app, más la
/// selección (`peers_seleccion`, campo NUEVO que Fase 3 añade a `EstadoPantalla`).
pub fn render_peers(datos: &EstadoPantalla) -> impl IntoElement {
    // R11: filtro por proyecto activo (aislamiento por convención de id). Se conserva el ÍNDICE REAL
    // de cada instancia (posición en `datos.instancias`) en la fila pintada, así las acciones que
    // viajan con `indice` siguen apuntando a la lista completa — sin traducción, sin bug índice-real.
    let activo = datos.proyecto_activo.as_deref();
    let total = datos
        .instancias
        .iter()
        .filter(|i| crate::vista::proyectos::id_del_proyecto_activo(&i.id, activo))
        .count();

    // Raíz: fondo/tipografía heredados del contenedor raíz de la app; aquí sólo el espaciado y la
    // cabecera. Se compone dentro del `fondo_app` del `AppDesktop`, así que no repite `.bg(TINTA)`.
    let mut raiz = div()
        .v_flex()
        .size_full()
        .gap_4()
        .p_6();

    // Cabecera: eyebrow + título Ethos con el conteo + indicador de salud del broker (peers-18,
    // variante 3 de la RFC: "punto verde/rojo junto al título"). Reusa `datos.salud` (GET /salud,
    // ya cargado globalmente por `cargar_broker`) — sin llamada HTTP propia de esta pantalla.
    raiz = raiz.child(
        div()
            .v_flex()
            .gap_1()
            .child(tema::eyebrow("red claude-peers"))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(tema::titulo(format!("Peers ({total})")))
                    .child(indicador_salud_broker(datos)),
            ),
    );

    if let Some(err) = &datos.error_peers {
        raiz = raiz.child(banner_error(err));
    }

    if total == 0 && datos.error_peers.is_none() {
        // Sin datos y sin error: aún no llegó la primera respuesta o no hay peers vivos.
        return raiz.child(
            tema::superficie_card()
                .v_flex()
                .items_center()
                .justify_center()
                .w_full()
                .py_8()
                .child(tema::texto_terciario("Sin peers vivos en esta máquina.")),
        );
    }

    // Índice seleccionado (si Fase 3 lo dejó dentro del rango). `peers_seleccion` es `Option<usize>`.
    let seleccion = datos.peers_seleccion;

    // Filas seleccionables: cada una deriva su estado de las alertas cacheadas (cruce sujeto == id).
    // R11: se PINTAN solo las del proyecto activo, pero conservando `idx` REAL (posición en
    // `datos.instancias`) — así `SeleccionarPeer{indice}` y la barra de acciones siguen correctos.
    let filas = datos
        .instancias
        .iter()
        .enumerate()
        .filter(|(_, inst)| crate::vista::proyectos::id_del_proyecto_activo(&inst.id, activo))
        .map(|(idx, inst)| {
            let estado = estado_peer(&inst.id, &datos.alertas);
            let activa = seleccion == Some(idx);
            fila(idx, inst, estado, activa)
        })
        .collect::<Vec<_>>();

    // La tabla va dentro de una superficie_card: encabezado (eyebrows) + cuerpo de filas.
    let tabla = tema::superficie_card()
        .v_flex()
        .w_full()
        .p_2()
        .child(encabezado())
        .child(div().v_flex().w_full().py_1().children(filas));

    raiz = raiz.child(tabla);

    // Barra de acciones del peer marcado (si la selección apunta a una fila válida Y VISIBLE bajo el
    // filtro de proyecto activo — un peer seleccionado que el filtro oculta no muestra su barra).
    if let Some(idx) = seleccion {
        if let Some(inst) = datos.instancias.get(idx) {
            if crate::vista::proyectos::id_del_proyecto_activo(&inst.id, activo) {
                raiz = raiz.child(barra_acciones(inst, idx));
            }
        }
    }

    raiz
}

// -------------------------------------------------------------------------------------------------
// MODALES (peers-01/02/03) — contenido PURO de los overlays que `AppDesktop` monta en su render
// raíz leyendo `peer_detalle` / `peers_form` (patrón idéntico a los modales de Tareas). El overlay
// (backdrop, clic-fuera, Esc) lo aporta la app; aquí sólo se compone el interior Ethos.
// -------------------------------------------------------------------------------------------------

/// Rojo terroso para las acciones DESTRUCTIVAS (kick). Mismo tono que el confirmar destructivo de
/// Tareas/Alertas: semántica de peligro calibrada a la paleta cálida, nunca el dorado de marca.
const ROJO_PELIGRO: u32 = 0xC0_4A_3E;
const ROJO_PELIGRO_HOVER: u32 = 0xD0_5A_4E;

/// Botón de acción PELIGROSA (variante roja apagada de la RFC): mismo formato que los botones del
/// tema pero en rojo terroso. Vive aquí (no en `tema`) hasta que otra pestaña lo necesite.
fn boton_peligro(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .py_2()
        .rounded(px(tema::RADIO_CONTROL))
        .bg(rgb(ROJO_PELIGRO))
        .text_color(tema::PAPEL)
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.bg(rgb(ROJO_PELIGRO_HOVER)))
        .child(label.into())
}

/// Fila "eyebrow humo + valor papel" del detalle, con el valor en MONO (ids, rutas, pids, horas).
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

/// Fila de una métrica CONTABLE del modal (craft P2, s005): columna "métricas" (ancho fijo, como
/// el resto de campos) + `n <descripción>` con singular/plural real (nunca "alerta(s)") y color
/// BRASA si `n > 0` (llama la atención sobre lo pendiente), HUMO si es 0 (nada que ver, no compite
/// visualmente). `plural`/`singular` se pasan explícitos (no un pluralizador genérico) porque son
/// sólo 2 casos reales en este modal — más simple y correcto que reglas gramaticales de más.
fn campo_metrica(columna: &'static str, singular: &str, plural: &str, n: usize) -> impl IntoElement {
    let color = if n > 0 { tema::BRASA } else { tema::HUMO };
    let texto = if n == 1 {
        format!("1 {singular}")
    } else {
        format!("{n} {plural}")
    };
    div()
        .h_flex()
        .items_baseline()
        .gap_3()
        .child(div().w(px(110.0)).flex_shrink_0().child(tema::eyebrow(columna)))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(color)
                .child(SharedString::from(texto)),
        )
}

/// Contenido del pop-up "Detalle del peer" (peers-01). Consolida TODO lo que la fila recorta:
/// identidad (id + directorio + repos + tty/pid/hostname), resumen íntegro, estado operativo
/// (chip derivado de alertas), señales (`registrada_en`/`visto_en` mono) y métricas ligeras
/// (nº alertas vivas del peer y nº tareas abiertas, cruzadas de las cachés). Al pie, la barra de
/// acciones (mensaje / jornada / expulsar) — el hub que la RFC pide como shell de las siguientes.
pub fn render_modal_detalle_peer(inst: &Instancia, datos: &EstadoPantalla) -> AnyElement {
    let estado = estado_peer(&inst.id, &datos.alertas);

    // Métricas cruzadas de las cachés ya cargadas (sin red aquí: la vista es pura). `AppDesktop`
    // refresca `tareas` al abrir el detalle para que estos números no estén rancios.
    let alertas_vivas = datos.alertas.iter().filter(|a| a.sujeto == inst.id).count();
    let tareas_abiertas = datos
        .tareas
        .iter()
        .filter(|t| t.instancia_id == inst.id && !t.estado.es_terminal())
        .count();

    let id_msg = inst.id.clone();
    let id_jornada = inst.id.clone();
    let id_kick = inst.id.clone();
    let id_copiar = identificador_peer(inst);

    div()
        .v_flex()
        .w(px(560.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        // Cabecera: chip de estado + título display + ✕ de cierre.
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
                        .child(tema::chip_estado(estado.etiqueta(), estado.color()))
                        .child(tema::titulo("Detalle del peer").text_size(px(18.0))),
                )
                .child(
                    div()
                        .id("modal-peer-cerrar-x")
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
                            window.dispatch_action(Box::new(CerrarDetallePeer), cx);
                        }),
                ),
        )
        // Identidad: el id en BRASA grande (el ancla), luego las rutas en mono.
        .child(
            div()
                .text_color(tema::BRASA)
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(SharedString::from(inst.id.clone())),
        )
        .child(campo_mono("directorio", inst.directorio.clone()))
        .child(campo_mono(
            "repo git",
            inst.repo_git.clone().unwrap_or_else(|| "—".to_string()),
        ))
        .child(campo_mono(
            "github",
            inst.repo_github.clone().unwrap_or_else(|| "—".to_string()),
        ))
        .child(campo_mono(
            "proceso",
            format!(
                "pid {} · {} · {}",
                inst.pid,
                if inst.hostname.is_empty() { "host —" } else { &inst.hostname },
                inst.tty.as_deref().unwrap_or("tty —")
            ),
        ))
        // Resumen ÍNTEGRO (la celda de la tabla lo recorta): texto humano, con scroll acotado.
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(tema::eyebrow("resumen"))
                .child(
                    div()
                        .id("modal-peer-resumen-scroll")
                        .max_h(px(140.0))
                        .overflow_y_scroll()
                        .child(tema::texto_primario(if inst.resumen.is_empty() {
                            "(sin resumen)".to_string()
                        } else {
                            inst.resumen.clone()
                        })),
                ),
        )
        // Señales de vida: registro y último latido, en mono (timestamps). Antes mostraban el ISO
        // CRUDO con microsegundos ("2026-07-03T07:18:43.453626Z") — Max pidió fecha uniforme
        // dd/mm/aaaa hh:mm:ss en toda la GPUI (#10, hallazgo de crítica de diseño confirmado).
        // "(UTC)": el broker timbra en UTC (no se convierte a hora local España hoy, ver nota de
        // Julio sobre `time`/`current_local_offset` frágil en multi-thread).
        .child(campo_mono("registrada (UTC)", tema::formatear_fecha(&inst.registrada_en)))
        .child(campo_mono("visto (UTC)", tema::formatear_fecha(&inst.visto_en)))
        // Métricas ligeras de las cachés (alertas del peer + tareas abiertas). Craft P2 (s005):
        // "1 alerta(s) viva(s) · 0 tarea(s) abierta(s)" era pluralización literal fea; ahora
        // singular/plural real + color BRASA si >0 (llama la atención cuando hay algo pendiente),
        // HUMO neutro en 0 (nada que ver aquí).
        .child(campo_metrica("alertas", "alerta viva", "alertas vivas", alertas_vivas))
        .child(campo_metrica("tareas", "tarea abierta", "tareas abiertas", tareas_abiertas))
        // Pie: acciones del peer (el detalle es el hub). Cerrar + mensaje + jornada + expulsar.
        .child(
            div()
                .h_flex()
                .flex_wrap()
                .gap_3()
                .pt_2()
                .child(
                    tema::boton_secundario("modal-peer-cerrar", "Cerrar").on_click(
                        |_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarDetallePeer), cx);
                        },
                    ),
                )
                .child(
                    tema::boton_primario("modal-peer-mensaje", "Enviar mensaje").on_click(
                        move |_e, window, cx| {
                            window.dispatch_action(
                                Box::new(EnviarMensajePeer { id: id_msg.clone() }),
                                cx,
                            );
                        },
                    ),
                )
                .child(
                    tema::boton_secundario("modal-peer-jornada", "Ver jornada").on_click(
                        move |_e, window, cx| {
                            window.dispatch_action(
                                Box::new(VerJornadaPeer { id: id_jornada.clone() }),
                                cx,
                            );
                        },
                    ),
                )
                // Copiar id (bug #4): mismo mecanismo que `barra_acciones` — acción local, sin
                // Action/broker, resuelta directo en el `on_click` con `cx.write_to_clipboard`.
                .child(
                    tema::boton_secundario("modal-peer-copiar-id", "Copiar id").on_click(
                        move |_e, _window, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                id_copiar.clone(),
                            ));
                        },
                    ),
                )
                .child(boton_peligro("modal-peer-kick", "Expulsar").on_click(
                    move |_e, window, cx| {
                        window.dispatch_action(Box::new(PedirKickPeer { id: id_kick.clone() }), cx);
                    },
                )),
        )
        .into_any_element()
}

/// Contenido del modal del formulario activo de Peers (peers-02/03): el COMPOSER de mensaje
/// (Input real + Para: <id> en brasa) o la CONFIRMACIÓN de expulsión (destructiva, botón rojo).
/// Confirmar despacha `ConfirmarFormPeers` sin payload: el texto lo lee `AppDesktop` del Input.
pub fn render_modal_form_peers(form: &FormPeers, datos: &EstadoPantalla) -> AnyElement {
    let titulo = match form {
        FormPeers::Mensaje { .. } => "Enviar mensaje",
        FormPeers::Kick { .. } => "Expulsar peer",
    };

    let mut modal = div()
        .v_flex()
        .w(px(520.0))
        .gap_3()
        .p_5()
        .rounded(px(tema::RADIO_CARD))
        .bg(tema::TINTA2)
        .border_1()
        .border_color(tema::LINEA)
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(tema::titulo(titulo).text_size(px(18.0)))
                .child(
                    div()
                        .id("modal-form-peers-cerrar-x")
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
                            window.dispatch_action(Box::new(CerrarFormPeers), cx);
                        }),
                ),
        );

    match form {
        FormPeers::Mensaje { id } => {
            // Cabecera "Para: <id>" en brasa (la RFC lo pide explícito: ver a quién se manda).
            modal = modal.child(
                div()
                    .h_flex()
                    .items_baseline()
                    .gap_3()
                    .child(div().w(px(60.0)).flex_shrink_0().child(tema::eyebrow("para")))
                    .child(
                        div()
                            .font_family(tema::FUENTE_MONO)
                            .text_color(tema::BRASA)
                            .child(SharedString::from(id.clone())),
                    ),
            );
            // Input real del mensaje (Entity creada por la app; la vista sólo lo pinta).
            if let Some(input) = &datos.input_mensaje_peer {
                modal = modal.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(tema::eyebrow("mensaje"))
                        .child(
                            div()
                                .w_full()
                                .px_3()
                                .py_1()
                                .rounded(tema::radio(tema::RADIO_CONTROL))
                                .bg(tema::TINTA)
                                .border_1()
                                .border_color(tema::LINEA)
                                .child(Input::new(input).cleanable(true)),
                        ),
                );
            } else {
                modal = modal.child(tema::texto_terciario(
                    "El campo de mensaje no está inicializado; reinicia la app.",
                ));
            }
        }
        FormPeers::Kick { id } => {
            modal = modal
                .child(tema::texto_primario(format!(
                    "¿Expulsar a «{id}»? Se cerrará su presencia en la red."
                )))
                .child(tema::texto_terciario(
                    "El peer desaparece del roster; si su sesión sigue viva, volverá a registrarse \
                     con el siguiente latido.",
                ));
        }
    }

    // Error de validación de frontera (mensaje vacío) que dejó `AppDesktop`.
    if let Some(err) = &datos.peers_form_error {
        modal = modal.child(
            div()
                .text_sm()
                .text_color(rgb(0xF1_A8_A8))
                .child(SharedString::from(format!("⚠ {err}"))),
        );
    }

    // Botonera: Cancelar + confirmar (dorado para enviar, rojo para expulsar).
    let confirmar: AnyElement = match form {
        FormPeers::Mensaje { .. } => {
            tema::boton_primario("modal-form-peers-enviar", "Enviar")
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(ConfirmarFormPeers), cx);
                })
                .into_any_element()
        }
        FormPeers::Kick { .. } => boton_peligro("modal-form-peers-kick", "Sí, expulsar")
            .on_click(|_e, window, cx| {
                window.dispatch_action(Box::new(ConfirmarFormPeers), cx);
            })
            .into_any_element(),
    };

    modal
        .child(
            div()
                .h_flex()
                .gap_3()
                .pt_2()
                .child(
                    tema::boton_secundario("modal-form-peers-cancelar", "Cancelar").on_click(
                        |_e, window, cx| {
                            window.dispatch_action(Box::new(CerrarFormPeers), cx);
                        },
                    ),
                )
                .child(confirmar),
        )
        .into_any_element()
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
