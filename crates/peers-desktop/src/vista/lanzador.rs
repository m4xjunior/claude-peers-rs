//! Pantalla Lanzador — RFC Lanzador **Fase 1** (Zona A: configuración de sesión).
//!
//! QUÉ HACE (Fase 1): convierte a `peers-desktop` en el punto de arranque del equipo. Deja
//! elegir el directorio de trabajo (file picker NATIVO de GPUI, cero deps), escribir un system
//! prompt (con plantillas nombradas), listar tareas iniciales, y elegir el destino de ejecución
//! (Local / SSH / tmux). Con todo eso, COMPONE el comando `claude …` exacto y lo PREVISUALIZA
//! (mono, scrolleable) con un botón "Copiar". En Fase 1 NO se ejecuta nada: "Lanzar" = preview +
//! copiar (R6). El terminal PTY embebido (Zona B) y el chat privado (Zona C) son Fase 2.
//!
//! ALCANCE EXACTO (cerrado por Julio): SÍ R1 (picker), R1.1/R1.2 (recientes en config.toml),
//! R2 (system prompt), R2.1 (plantillas), **R3.2** (tareas inyectadas EN EL PROMPT — no vía
//! broker), R4 (destino cambia el comando previsualizado), R5 (flag de canal SIEMPRE + skip-perms
//! OFF por defecto), R6 (preview + copiar), R10 (degradación). NO: R3.1 (broker), R7 (PTY),
//! §7 (chat), R9 (perfiles combinados), ejecución real del comando.
//!
//! DECISIÓN DE DISEÑO (por qué un componente stateful `PanelLanzador: Render`, como `PanelConfig`):
//! el editor de system prompt es un `Input` multilínea del kit, que exige `Entity<InputState>`
//! (sólo creable con `Window`/`Context`, debe vivir en una vista). Igual el nombre de sesión tmux,
//! el host SSH y el dir remoto. Por eso la pantalla es un `Entity<PanelLanzador>` que la app crea
//! al arrancar (`nuevo_panel`) y guarda en `EstadoPantalla`; el stub `render_lanzador` (firma de la
//! Fundación intacta) sólo delega en esa entidad. El resto del estado (destino, tareas, flags,
//! recientes, plantillas) es plano y vive en el propio panel.
//!
//! DECISIÓN sobre los desplegables (por qué inline y no el `Select`/`Dropdown` del kit): la app YA
//! evita los componentes stateful del kit (Sidebar, Dialog) por estabilidad de su API entre revs
//! del git (ver cabecera de `app.rs`). Los desplegables de recientes/destino/plantilla se hacen
//! con estado booleano en el panel + una lista de `fila_seleccionable` del tema, coherente con el
//! `jornada_dropdown` que ya existe. Cero fricción, mismo look Ethos.
//!
//! ANTI-SIGABRT: el file picker devuelve un `oneshot::Receiver` (NO reqwest), así que su `.await`
//! sí puede correr en `cx.spawn` sin entrar al runtime tokio. No hay red en esta pantalla en Fase 1.

use futures::StreamExt;
use gpui::{
    div, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement, Styled, Window,
};
use gpui_component::{
    input::{Input, InputState},
    StyledExt,
};
use std::path::PathBuf;

use crate::app::EstadoPantalla;
use crate::config::{Config, PlantillaPrompt};
use crate::pty::{ColorPty, ComandoPty, DimensionesCeldas, EventoPty, SesionPty};
use crate::teclado;
use crate::tema;

/// Constante del proyecto: el flag que hace que el `<channel>` de claude-peers se renderice. La
/// app lo añade SIEMPRE al comando (R5); Max nunca lo teclea. Sin él, los peers no se ven entre sí.
const FLAG_CANAL: &str = "--dangerously-load-development-channels server:claude-peers";

/// Destino de ejecución de la sesión (R4). Sólo CAMBIA el comando previsualizado en Fase 1
/// (no se ejecuta nada). `Copy` porque es un discriminante trivial que se compara y guarda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destino {
    /// R4.1 — `claude` corre en la máquina de la app. El dir es el del picker local.
    Local,
    /// R4.2 — `ssh -t <host>` y dentro `cd <dir remoto> && claude …`. Dir remoto a mano.
    Ssh,
    /// R4.3 — `tmux new-session -d -s <nombre> -c <dir> "claude …"` + `tmux attach -t <nombre>`.
    Tmux,
}

impl Destino {
    /// Las tres opciones, en orden de aparición en el desplegable.
    const TODOS: [Destino; 3] = [Destino::Local, Destino::Ssh, Destino::Tmux];

    /// Etiqueta visible en el selector.
    fn etiqueta(self) -> &'static str {
        match self {
            Destino::Local => "Local",
            Destino::Ssh => "SSH (otra máquina)",
            Destino::Tmux => "tmux (sesión persistente)",
        }
    }

    /// Id estable para el elemento interactivo del selector.
    fn id(self) -> &'static str {
        match self {
            Destino::Local => "destino-local",
            Destino::Ssh => "destino-ssh",
            Destino::Tmux => "destino-tmux",
        }
    }
}

/// Una tarea inicial de la sesión (R3, materialización R3.2). En Fase 1 se INYECTA como texto en
/// el system prompt ("Tus tareas de hoy: 1) … 2) …"), sin tocar el broker. `estimado` es libre
/// (texto tal cual lo escribe Max, p.ej. "30m"); no se parsea porque en R3.2 sólo se pinta.
#[derive(Debug, Clone, Default)]
pub struct TareaInicial {
    /// Descripción de la tarea. Vacía no se añade.
    pub descripcion: String,
    /// Estimado opcional en texto libre (sólo informativo en R3.2).
    pub estimado: String,
}

// -------------------------------------------------------------------------------------------------
// LÓGICA PURA — composición y escapado del comando (R2/R5/R6). Aislada del render para poder
// testearla sin `Window`/`cx`. El QA prueba aquí el escapado con comillas, `$` y saltos de línea.
// -------------------------------------------------------------------------------------------------

/// Parámetros que definen el comando a componer. Struct plano y `Clone` para pasarlo a la lógica
/// pura desde el render sin arrastrar el estado del panel entero.
#[derive(Debug, Clone)]
pub struct ParamsComando {
    /// Directorio de trabajo elegido (R1). En Local/tmux es local; en SSH es la ruta remota.
    pub dir: String,
    /// System prompt ya materializado (R2 + tareas R3.2 inyectadas). Vacío = no se pasa el flag.
    pub system_prompt: String,
    /// Destino de ejecución (R4).
    pub destino: Destino,
    /// Host SSH (sólo relevante si `destino == Ssh`).
    pub host_ssh: String,
    /// Nombre de la sesión tmux (sólo relevante si `destino == Tmux`).
    pub nombre_tmux: String,
    /// Si `true`, añade `--dangerously-skip-permissions` (R5, default OFF, con aviso en la UI).
    pub skip_permisos: bool,
}

/// Escapa un valor para incrustarlo entre comillas dobles en un comando de shell POSIX. Envuelve
/// en `"…"` y escapa los cuatro metacaracteres que el shell sigue interpretando DENTRO de comillas
/// dobles: `"` (cierra la cadena), `\` (escape), `` ` `` (sustitución de comando) y `$` (expansión).
/// Un salto de línea DENTRO de comillas dobles es literal en POSIX, así que se conserva tal cual.
///
/// Por qué comillas dobles y no simples: el system prompt de Max puede contener apóstrofos; las
/// comillas simples no permiten escapar un `'` dentro sin cerrar-concatenar, lo que ensucia el
/// preview. Las dobles con estos 4 escapes son legibles y correctas para lo que aquí se genera.
pub fn escapar_shell(valor: &str) -> String {
    let mut s = String::with_capacity(valor.len() + 2);
    s.push('"');
    for c in valor.chars() {
        match c {
            '"' | '\\' | '`' | '$' => {
                s.push('\\');
                s.push(c);
            }
            otro => s.push(otro),
        }
    }
    s.push('"');
    s
}

/// Materializa el system prompt final (R2 + R3.2): al texto base le AÑADE, si hay tareas, un
/// bloque "Tus tareas de hoy:" numerado. Devuelve `String` vacía si no hay ni prompt ni tareas
/// (el caller entonces omite el flag `--append-system-prompt`).
pub fn materializar_prompt(base: &str, tareas: &[TareaInicial]) -> String {
    let base = base.trim();
    // Sólo tareas con descripción no vacía cuentan (R3.2).
    let utiles: Vec<&TareaInicial> = tareas
        .iter()
        .filter(|t| !t.descripcion.trim().is_empty())
        .collect();

    if utiles.is_empty() {
        return base.to_string();
    }

    let mut out = String::new();
    if !base.is_empty() {
        out.push_str(base);
        out.push_str("\n\n");
    }
    out.push_str("Tus tareas de hoy:");
    for (i, t) in utiles.iter().enumerate() {
        let est = t.estimado.trim();
        if est.is_empty() {
            out.push_str(&format!("\n{}) {}", i + 1, t.descripcion.trim()));
        } else {
            out.push_str(&format!("\n{}) {} ({})", i + 1, t.descripcion.trim(), est));
        }
    }
    out
}

/// Flags de `claude` comunes a los tres destinos (R2/R5), YA ESCAPADOS para incrustar en una
/// cadena de shell (`componer_comando`, R6). En orden estable. Única fuente de verdad de qué flags
/// lleva el comando — `argv_claude` (variante estructurada, R7) reusa la MISMA lógica de decisión
/// (qué flag va y cuándo) pero sin escapar, porque ahí el argumento va directo a `execve`, no a
/// través de una subshell.
fn flags_claude_shell(p: &ParamsComando) -> Vec<String> {
    let mut flags = Vec::new();
    let prompt = p.system_prompt.trim();
    if !prompt.is_empty() {
        flags.push(format!("--append-system-prompt {}", escapar_shell(prompt)));
    }
    // R5: el flag de canal SIEMPRE presente.
    flags.push(FLAG_CANAL.to_string());
    // R5: skip-permisos sólo si Max lo activó explícitamente (default OFF).
    if p.skip_permisos {
        flags.push("--dangerously-skip-permissions".to_string());
    }
    flags
}

/// COMPONE el comando exacto a previsualizar (R6) según los parámetros. Es la única fuente de
/// verdad del preview y de "Copiar". SIEMPRE incluye el flag de canal (R5). No ejecuta nada.
///
/// Formas por destino (verificadas contra §11.4 de la RFC):
/// - Local: `cd <dir> && claude <flags>`
/// - SSH:   `ssh -t <host> "cd <dir> && claude <flags>"`
/// - tmux:  `tmux new-session -d -s <nombre> -c <dir> "claude <flags>" && tmux attach -t <nombre>`
pub fn componer_comando(p: &ParamsComando) -> String {
    let flags = flags_claude_shell(p);
    let claude = if flags.is_empty() {
        "claude".to_string()
    } else {
        format!("claude {}", flags.join(" "))
    };

    let dir = p.dir.trim();
    match p.destino {
        Destino::Local => {
            if dir.is_empty() {
                claude
            } else {
                format!("cd {} && {claude}", escapar_shell(dir))
            }
        }
        Destino::Ssh => {
            let host = p.host_ssh.trim();
            let host = if host.is_empty() { "<host>" } else { host };
            let remoto = if dir.is_empty() {
                claude
            } else {
                format!("cd {} && {claude}", escapar_shell(dir))
            };
            format!("ssh -t {host} {}", escapar_shell(&remoto))
        }
        Destino::Tmux => {
            let nombre = p.nombre_tmux.trim();
            let nombre = if nombre.is_empty() { "<nombre>" } else { nombre };
            let mut new = String::from("tmux new-session -d -s ");
            new.push_str(nombre);
            if !dir.is_empty() {
                new.push_str(&format!(" -c {}", escapar_shell(dir)));
            }
            new.push(' ');
            new.push_str(&escapar_shell(&claude));
            format!("{new} && tmux attach -t {nombre}")
        }
    }
}

/// Variante ESTRUCTURADA de `componer_comando` para el PTY real (R7, Zona B): en vez de una cadena
/// de shell, devuelve `(programa, args, dir)` listos para `execve` directo — sin pasar por `sh -c`,
/// así que NO hace falta `escapar_shell` (los args van tal cual a `argv`, el kernel no reinterpreta
/// `$`/`` ` ``/comillas). Nota de diseño §1.2 del RFC Fase 2: el preview (R6) y lo que realmente
/// corre (R7) salen del MISMO origen — `flags_claude_shell` decide QUÉ flags van, esta función sólo
/// cambia CÓMO se entregan (escapados en una cadena vs. como lista de argv).
///
/// v1 SÓLO cubre `Destino::Local` (alcance de esta Fase: camino feliz). SSH/tmux devuelven `None`
/// — la UI (R7.2) cae al fallback "preparar + copiar" en vez de intentar un PTY que aún no cablea
/// esos destinos (evita reportar un R7 a medias como si fuera completo).
pub fn argv_claude(p: &ParamsComando) -> Option<(String, Vec<String>, Option<PathBuf>)> {
    if p.destino != Destino::Local {
        return None;
    }
    let mut args = Vec::new();
    let prompt = p.system_prompt.trim();
    if !prompt.is_empty() {
        args.push("--append-system-prompt".to_string());
        args.push(prompt.to_string());
    }
    // `FLAG_CANAL` es una cadena de 2 tokens de shell (`--flag valor`), NO un solo argumento de
    // argv: en `componer_comando` una subshell la retokeniza; aquí no hay subshell (execve directo),
    // así que hay que partirla a mano o el flag llegaría intacto y `claude` no lo reconocería.
    args.extend(FLAG_CANAL.split_whitespace().map(str::to_string));
    if p.skip_permisos {
        args.push("--dangerously-skip-permissions".to_string());
    }
    let dir = p.dir.trim();
    let dir = if dir.is_empty() {
        None
    } else {
        Some(PathBuf::from(dir))
    };
    Some(("claude".to_string(), args, dir))
}

/// R7 iteración 2 — extrae del contexto GPUI las medidas que `calcular_dimensiones` necesita: el
/// ancho real de una celda monoespaciada de `tema::FUENTE_MONO` (vía `text_system().advance('m')`,
/// mismo truco que usa `terminal_element.rs` de Zed para medir su rejilla) y el tamaño del
/// viewport actual. `tema::fondo_app()` fija el tamaño base de texto de la app en `px(14.0)` — se
/// usa ese mismo valor aquí para que la medición sea coherente con lo que `render_rejilla_pty`
/// realmente pinta (leer `window.text_style()` en este punto, un callback de click FUERA del árbol
/// de render, no reflejaría con fiabilidad el estilo de la card del terminal).
fn medir_dimensiones_ventana(window: &mut Window, cx: &mut App) -> DimensionesCeldas {
    const TAMANO_BASE: gpui::Pixels = gpui::px(14.0);
    // Resta el padding/chrome conocido del área del terminal (p_2 = 8px por lado, ver `card_terminal`
    // en el render) para no sobreestimar el espacio disponible y desbordar el borde de la card.
    const PADDING_AREA: gpui::Pixels = gpui::px(16.0);

    let font_id = cx.text_system().resolve_font(&gpui::font(tema::FUENTE_MONO));
    let ancho_celda = cx
        .text_system()
        .advance(font_id, TAMANO_BASE, 'm')
        .map(|s| s.width)
        // Fallback si la fuente no tiene glifo para 'm' (no debería pasar con IBM Plex Mono, pero
        // `advance` devuelve `Result` y R10 exige degradar, no `.unwrap()`): ~0.6× el tamaño de
        // fuente es la proporción típica de un monoespaciado, mejor que dividir por 0 o panicar.
        .unwrap_or(TAMANO_BASE * 0.6);
    // `line_height` del texto base de la app: mismo criterio, componentes de gpui-component suelen
    // rondar 1.2-1.4× el tamaño de fuente para line-height; usamos el `window.line_height()` real
    // si está disponible en este punto del ciclo, que ya incorpora el factor del tema activo.
    let alto_linea = window.line_height();

    let viewport = window.viewport_size();
    let ancho_disponible = (viewport.width - PADDING_AREA).max(ancho_celda);
    // Reserva vertical para el resto del panel (header fijo + otras 6 cards antes del terminal en
    // el scroll): una fracción del viewport, no todo el alto de la ventana, o el terminal pediría
    // más filas de las que realmente van a verse sin scrollear la card en sí.
    let alto_disponible = (viewport.height * 0.35 - PADDING_AREA).max(alto_linea);

    calcular_dimensiones(ancho_disponible, alto_disponible, ancho_celda, alto_linea)
}

/// R7 iteración 2 — deriva filas/columnas REALES a partir de medidas de la ventana/fuente, en vez
/// de `DimensionesCeldas::POR_DEFECTO` fijo (80x24). Función PURA (sólo aritmética con `Pixels`,
/// sin `Window`/`cx`) para poder testearla sin un contexto GPUI real — el caller (`lanzar_pty`)
/// extrae `ancho_celda`/`alto_linea` de `cx.text_system()` y `alto_disponible`/`ancho_disponible`
/// del viewport, y aquí sólo se hace la división + el `max(1)` de guardrail.
///
/// Alcance de ESTA iteración: cálculo ÚNICO al abrir la sesión (no reacciona en vivo a que Max
/// redimensione la ventana después — eso exigiría implementar `Element::prepaint` a mano, el
/// mismo trabajo pesado que el DISENO-FASE2 §1.1 descartaba para la vía B). `SesionPty::redimensionar`
/// ya existe y queda listo para cablearse a un resize en vivo en una iteración siguiente.
fn calcular_dimensiones(
    ancho_disponible: gpui::Pixels,
    alto_disponible: gpui::Pixels,
    ancho_celda: gpui::Pixels,
    alto_linea: gpui::Pixels,
) -> DimensionesCeldas {
    // `max(1)`: si la ventana es minúscula o la medida de fuente falla y devuelve 0, un terminal
    // de 0 filas/columnas haría que `Term::new`/`tty::new` (que usan esto como `Dimensions`)
    // entren en división por cero al calcular el wrap de línea — nunca dimensiones en 0 (R10).
    let columnas = ((f32::from(ancho_disponible) / f32::from(ancho_celda)).floor() as u16).max(1);
    let filas = ((f32::from(alto_disponible) / f32::from(alto_linea)).floor() as u16).max(1);
    DimensionesCeldas { filas, columnas }
}

// -------------------------------------------------------------------------------------------------
// ESTADO DEL PANEL
// -------------------------------------------------------------------------------------------------

/// Feedback efímero bajo la botonera (copiado / guardado de plantilla / error de picker).
#[derive(Debug, Clone, Default)]
enum Feedback {
    /// Sin feedback aún.
    #[default]
    Ninguno,
    /// Mensaje informativo (copiado, plantilla guardada…).
    Ok(String),
    /// Mensaje de error (picker falló en Linux, etc.). Degrada, no crashea (R10).
    Error(String),
}

/// Panel del Lanzador con estado propio. Vive como `Entity<PanelLanzador>` en `EstadoPantalla`.
pub struct PanelLanzador {
    /// Directorio elegido (R1). `None` hasta que Max elige uno en el picker o en recientes.
    dir: Option<PathBuf>,
    /// Editor del system prompt (R2), `Input` multilínea del kit.
    prompt: Entity<InputState>,
    /// Host SSH (R4.2), input de una línea.
    host_ssh: Entity<InputState>,
    /// Dir remoto para SSH (R4.2): el picker local no aplica en remoto, se escribe a mano.
    dir_remoto: Entity<InputState>,
    /// R7/E-05 — id explícito de agente (`CLAUDE_PEERS_ID`), texto libre opcional. Alcance de HOY
    /// (sin la maquinaria de Cargo/Proyecto de RFC-organigrama-roles, no implementada): si Max lo
    /// rellena, s007 lo inyecta como env var al lanzar (`ComandoPty.env`); vacío = comportamiento
    /// actual sin cambios (peers-client deriva su id de la carpeta, como siempre).
    id_agente: Entity<InputState>,
    /// Nombre de la sesión tmux (R4.3), input de una línea.
    nombre_tmux: Entity<InputState>,
    /// Nombre con el que guardar la plantilla actual (R2.1).
    nombre_plantilla: Entity<InputState>,
    /// Inputs de la tarea en composición (R3): descripción + estimado, antes de añadirla.
    nueva_tarea_desc: Entity<InputState>,
    nueva_tarea_est: Entity<InputState>,
    /// Destino de ejecución (R4).
    destino: Destino,
    /// Tareas iniciales ya añadidas (R3.2).
    tareas: Vec<TareaInicial>,
    /// R5: `--dangerously-skip-permissions`. Default OFF.
    skip_permisos: bool,
    /// Snapshot de la config del disco (recientes R1.2 + plantillas R2.1). Se recarga al guardar.
    lanzador_cfg: crate::config::ConfigLanzador,
    /// Desplegable de recientes abierto (R1.2).
    dropdown_recientes: bool,
    /// Desplegable de destino abierto (R4).
    dropdown_destino: bool,
    /// Desplegable de plantillas abierto (R2.1).
    dropdown_plantillas: bool,
    /// R7 — sesión de terminal embebido activa (Zona B, Fase 2). `None` hasta pulsar "Lanzar
    /// aquí"; sólo Local en v1 (SSH/tmux reusan `SesionPty::abrir` pero faltan por cablear la UI).
    sesion_pty: Option<SesionPty>,
    /// Último snapshot de la rejilla leído del PTY. Se cachea aquí (en vez de leer el `FairMutex`
    /// en cada `render`) porque `Render::render` puede llamarse más veces de las que hay eventos
    /// `Refrescar` reales; sólo se releen celdas cuando el event loop avisa.
    pantalla_pty: Option<crate::pty::ContenidoPty>,
    /// R7 iteración 2 — dimensión REAL con la que se abrió `sesion_pty` (medida al lanzar, ver
    /// `calcular_dimensiones`). `render_rejilla_pty` y el alto del área del terminal la usan en vez
    /// de `DimensionesCeldas::POR_DEFECTO` para que la rejilla pintada coincida con lo que el PTY
    /// realmente cree que mide (si divergieran, el wrap de línea del programa dentro del PTY no
    /// coincidiría con las columnas que GPUI pinta).
    dim_pty: DimensionesCeldas,
    /// R7.1/R7.2 — mensaje de degradación si `SesionPty::abrir` falló (banner Ethos, nunca panic).
    error_pty: Option<String>,
    /// Foco del área de la rejilla: sin esto GPUI no entrega `KeyDownEvent` a esta vista (el resto
    /// del panel usa `Input`s del kit, que gestionan su propio foco; la rejilla no es un `Input`).
    foco_pty: FocusHandle,
    /// Feedback efímero de la última acción.
    feedback: Feedback,
}

impl PanelLanzador {
    /// Construye el panel con la config del disco ya cargada (recientes + plantillas). Nunca
    /// crashea: config ausente/corrupta → `Config::default()` (misma política que `PanelConfig`).
    pub fn nuevo(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg = Config::cargar().unwrap_or_default();

        let prompt = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(4, 14)
                .placeholder("System prompt opcional. Se pasa como --append-system-prompt. Vacío = no se pasa.")
        });
        let host_ssh = cx.new(|cx| InputState::new(window, cx).placeholder("otus"));
        let dir_remoto =
            cx.new(|cx| InputState::new(window, cx).placeholder("/ruta/remota/al/proyecto"));
        let id_agente =
            cx.new(|cx| InputState::new(window, cx).placeholder("backend@proyecto-x (opcional)"));
        let nombre_tmux = cx.new(|cx| InputState::new(window, cx).placeholder("front-p2v"));
        let nombre_plantilla =
            cx.new(|cx| InputState::new(window, cx).placeholder("nombre de la plantilla"));
        let nueva_tarea_desc =
            cx.new(|cx| InputState::new(window, cx).placeholder("descripción de la tarea"));
        let nueva_tarea_est =
            cx.new(|cx| InputState::new(window, cx).placeholder("30m (opcional)"));

        Self {
            dir: None,
            prompt,
            host_ssh,
            dir_remoto,
            id_agente,
            nombre_tmux,
            nombre_plantilla,
            nueva_tarea_desc,
            nueva_tarea_est,
            destino: Destino::Local,
            tareas: Vec::new(),
            skip_permisos: false,
            lanzador_cfg: cfg.lanzador,
            dropdown_recientes: false,
            dropdown_destino: false,
            dropdown_plantillas: false,
            sesion_pty: None,
            pantalla_pty: None,
            dim_pty: DimensionesCeldas::POR_DEFECTO,
            error_pty: None,
            foco_pty: cx.focus_handle(),
            feedback: Feedback::default(),
        }
    }

    /// R1 — abre el file picker NATIVO de GPUI (cero deps) y, al elegir, guarda la ruta + la
    /// registra en recientes. El rx es un `oneshot::Receiver` (no reqwest): su `.await` corre en
    /// `cx.spawn` sin tocar el runtime tokio (patrón anti-SIGABRT). Cancelar (None) = no-op (R10).
    fn elegir_directorio(&mut self, cx: &mut Context<Self>) {
        // `cx` deref-a a `App`, donde vive `prompt_for_paths`. Sólo directorios, uno solo.
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            // Título del diálogo del sistema; `None` deja el texto por defecto del SO.
            prompt: Some(SharedString::from("Elegir directorio de la sesión")),
        });
        cx.spawn(async move |esta, cx| {
            let resultado = rx.await;
            let _ = esta.update(cx, |esta, cx| {
                match resultado {
                    // Ruta elegida: guardar la primera (multiple:false ⇒ como mucho una).
                    Ok(Ok(Some(paths))) => {
                        if let Some(dir) = paths.into_iter().next() {
                            esta.registrar_dir(dir, cx);
                        }
                    }
                    // Cancelado: no-op limpio (R10 / AC1).
                    Ok(Ok(None)) => {}
                    // Error del picker (p.ej. Linux sin portal): banner, sin panic (R10).
                    Ok(Err(e)) => {
                        esta.feedback = Feedback::Error(format!("No se pudo abrir el picker: {e}"));
                        cx.notify();
                    }
                    // El canal se cerró sin respuesta (ventana cerrada, etc.): no-op.
                    Err(_) => {}
                }
            });
        })
        .detach();
    }

    /// Fija el directorio elegido, lo registra en recientes (R1.2) y persiste la config si cambió.
    /// El guardado NO crashea: si falla el IO, se avisa por feedback y el estado en memoria queda.
    fn registrar_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let ruta = dir.to_string_lossy().to_string();
        self.dir = Some(dir);
        if self.lanzador_cfg.registrar_reciente(ruta) {
            self.persistir_lanzador();
        }
        cx.notify();
    }

    /// Precarga el panel desde un proyecto (RFC-proyectos R9 "Lanzar equipo"): fija el id de agente
    /// (`rol@proyecto`, se inyecta como CLAUDE_PEERS_ID al lanzar) y el directorio de trabajo. Lo
    /// llama `AppDesktop` al navegar aquí desde la ficha de un proyecto, con un agente elegido. El
    /// destino SSH/tmux del proyecto NO se precarga en v1 (el panel arranca en Local; Max ajusta).
    pub fn precargar_desde_proyecto(
        &mut self,
        id_agente: &str,
        dir: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.id_agente.update(cx, |s, cx| s.set_value(id_agente.to_string(), window, cx));
        if let Some(d) = dir {
            self.dir = Some(PathBuf::from(&d));
            // Registra el dir en recientes también (coherente con elegir por picker).
            if self.lanzador_cfg.registrar_reciente(d) {
                self.persistir_lanzador();
            }
        }
        cx.notify();
    }

    /// Elige un directorio de la lista de recientes (R1.2) sin reabrir el picker. Lo sube al frente.
    fn elegir_reciente(&mut self, ruta: String, cx: &mut Context<Self>) {
        self.dir = Some(PathBuf::from(&ruta));
        self.dropdown_recientes = false;
        if self.lanzador_cfg.registrar_reciente(ruta) {
            self.persistir_lanzador();
        }
        cx.notify();
    }

    /// Persiste SÓLO la sección `[lanzador]` sobre la config del disco: relee la config completa
    /// (para no pisar broker_url/token/refresh que edita otra pantalla), le mete nuestra sección
    /// y guarda. Errores de IO no crashean — se reflejan como feedback (R10).
    fn persistir_lanzador(&mut self) {
        let mut cfg = Config::cargar().unwrap_or_default();
        cfg.lanzador = self.lanzador_cfg.clone();
        if let Err(e) = cfg.guardar() {
            self.feedback = Feedback::Error(format!("No se pudo guardar la config: {e}"));
        }
    }

    /// R2.1 — guarda el system prompt actual como plantilla con el nombre tecleado. Nombre vacío
    /// se ignora con aviso. Persiste y recarga el desplegable de plantillas.
    fn guardar_plantilla(&mut self, cx: &mut Context<Self>) {
        let nombre = self.nombre_plantilla.read(cx).value().trim().to_string();
        if nombre.is_empty() {
            self.feedback = Feedback::Error("Ponle un nombre a la plantilla antes de guardar.".into());
            cx.notify();
            return;
        }
        let texto = self.prompt.read(cx).value().to_string();
        self.lanzador_cfg.guardar_plantilla(nombre.clone(), texto);
        self.persistir_lanzador();
        self.feedback = Feedback::Ok(format!("Plantilla «{nombre}» guardada."));
        cx.notify();
    }

    /// R2.1 — carga una plantilla en el editor de system prompt (reemplaza el contenido actual).
    fn cargar_plantilla(&mut self, plantilla: PlantillaPrompt, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt.update(cx, |s, cx| s.set_value(plantilla.texto, window, cx));
        self.nombre_plantilla
            .update(cx, |s, cx| s.set_value(plantilla.nombre, window, cx));
        self.dropdown_plantillas = false;
        cx.notify();
    }

    /// R3 — añade la tarea en composición a la lista si tiene descripción. Limpia los inputs.
    fn anadir_tarea(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let desc = self.nueva_tarea_desc.read(cx).value().trim().to_string();
        if desc.is_empty() {
            return;
        }
        let est = self.nueva_tarea_est.read(cx).value().trim().to_string();
        self.tareas.push(TareaInicial {
            descripcion: desc,
            estimado: est,
        });
        self.nueva_tarea_desc
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        self.nueva_tarea_est
            .update(cx, |s, cx| s.set_value(String::new(), window, cx));
        cx.notify();
    }

    /// R3 — quita la tarea `idx` de la lista (si está en rango).
    fn quitar_tarea(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.tareas.len() {
            self.tareas.remove(idx);
            cx.notify();
        }
    }

    /// R3 — reordena: sube la tarea `idx` una posición (si puede).
    fn subir_tarea(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx > 0 && idx < self.tareas.len() {
            self.tareas.swap(idx, idx - 1);
            cx.notify();
        }
    }

    /// R7/E-05 — id explícito de agente (`CLAUDE_PEERS_ID`) tecleado por Max, o `None` si el campo
    /// está vacío (comportamiento actual sin cambios). NO va en `ParamsComando`/`argv_claude`: no es
    /// parte de la cadena de shell del preview (R6, "Copiar comando" no debería filtrar el env var
    /// que se inyecta al proceso), sino algo que `lanzar_pty` pasa aparte a `ComandoPty.env`.
    pub(crate) fn id_agente(&self, cx: &App) -> Option<String> {
        let valor = self.id_agente.read(cx).value().trim().to_string();
        (!valor.is_empty()).then_some(valor)
    }

    /// Reúne los parámetros actuales del comando (R6). El dir depende del destino: en SSH es el
    /// dir remoto tecleado; en Local/tmux, el del picker.
    fn params(&self, cx: &App) -> ParamsComando {
        let dir = match self.destino {
            Destino::Ssh => self.dir_remoto.read(cx).value().to_string(),
            _ => self
                .dir
                .as_ref()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default(),
        };
        let base = self.prompt.read(cx).value().to_string();
        let system_prompt = materializar_prompt(&base, &self.tareas);
        ParamsComando {
            dir,
            system_prompt,
            destino: self.destino,
            host_ssh: self.host_ssh.read(cx).value().to_string(),
            nombre_tmux: self.nombre_tmux.read(cx).value().to_string(),
            skip_permisos: self.skip_permisos,
        }
    }

    /// R6 — copia el comando previsualizado al portapapeles del sistema (vía GPUI, sin deps).
    fn copiar_comando(&mut self, cx: &mut Context<Self>) {
        let cmd = componer_comando(&self.params(cx));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(cmd));
        self.feedback = Feedback::Ok("Comando copiado al portapapeles.".into());
        cx.notify();
    }

    /// R7 — abre el PTY con el comando derivado de `params()` (MISMO origen que el preview R6) y
    /// arranca el loop que drena sus eventos hacia esta vista. `SesionPty::abrir` es SÍNCRONO y
    /// LOCAL (abre un fd, no hace red): no necesita `cx.spawn` para la apertura en sí — sólo el
    /// drenaje de eventos posteriores corre en background (canal `futures`, no reqwest — seguro
    /// dentro de `cx.spawn` según la trampa anti-SIGABRT documentada del proyecto).
    ///
    /// R7.2: si `argv_claude` devuelve `None` (destino no-Local, fuera de alcance v1) o si
    /// `SesionPty::abrir` falla (permisos, plataforma sin PTY), banner de error — NUNCA panic.
    ///
    /// R7 iteración 2: `dim` ya NO es `POR_DEFECTO` fijo — se mide con `medir_dimensiones_ventana`
    /// (ancho de celda real de `tema::FUENTE_MONO` al tamaño base de la app + viewport actual).
    /// Cálculo ÚNICO al lanzar (no reacciona a un resize posterior de la ventana, ver nota de
    /// `calcular_dimensiones`).
    fn lanzar_pty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((programa, args, dir)) = argv_claude(&self.params(cx)) else {
            self.error_pty = Some(
                "El terminal embebido sólo soporta destino Local por ahora. Usa \"Copiar comando\" \
                 para SSH/tmux."
                    .to_string(),
            );
            cx.notify();
            return;
        };

        let dim = medir_dimensiones_ventana(window, cx);
        // E-05: si Max rellenó "id de agente", se inyecta CLAUDE_PEERS_ID en el env del proceso
        // hijo (agnóstico de shell, ver pty.rs::abrir). `None` (campo vacío) = sin env extra — el
        // client sigue derivando su id de la carpeta como hoy (compat total).
        let env = match self.id_agente(cx) {
            Some(id) => vec![("CLAUDE_PEERS_ID".to_string(), id)],
            None => Vec::new(),
        };
        let comando = ComandoPty { programa, args, dir, env };
        match SesionPty::abrir(comando, dim) {
            Ok(mut sesion) => {
                self.dim_pty = dim;
                let Some(mut eventos) = sesion.tomar_eventos() else {
                    // No debería pasar (recién abierta): `tomar_eventos` sólo devuelve `None` si ya
                    // se llamó antes. Degradar sin panic de todas formas.
                    self.error_pty = Some("La sesión PTY no expuso su canal de eventos.".into());
                    cx.notify();
                    return;
                };
                self.pantalla_pty = Some(sesion.contenido());
                self.sesion_pty = Some(sesion);
                self.error_pty = None;
                cx.notify();

                cx.spawn(async move |esta, cx| {
                    while let Some(evento) = eventos.next().await {
                        let seguir = esta
                            .update(cx, |esta, cx| {
                                match evento {
                                    EventoPty::Refrescar => {
                                        if let Some(s) = &esta.sesion_pty {
                                            esta.pantalla_pty = Some(s.contenido());
                                        }
                                    }
                                    EventoPty::Terminado => {
                                        if let Some(s) = &esta.sesion_pty {
                                            esta.pantalla_pty = Some(s.contenido());
                                        }
                                        esta.sesion_pty = None;
                                    }
                                    // Campana/Título: informativos, sin acción en v1 (R7 core).
                                    EventoPty::Campana | EventoPty::Titulo(_) => {}
                                }
                                cx.notify();
                                // Deja de drenar una vez que la sesión murió y ya no queda estado
                                // que actualizar — evita un loop infinito sobre un canal huérfano.
                                esta.sesion_pty.is_some()
                            })
                            .unwrap_or(false);
                        if !seguir {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(e) => {
                self.error_pty = Some(format!("No se pudo abrir el terminal: {e}"));
                cx.notify();
            }
        }
    }

    /// R7.1 — cierra la sesión activa (botón "Cerrar terminal" o antes de relanzar). Idempotente.
    fn cerrar_pty(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = self.sesion_pty.take() {
            s.cerrar();
        }
        self.pantalla_pty = None;
        cx.notify();
    }

    /// R7 — traduce un `KeyDownEvent` a bytes ANSI (`teclado::a_secuencia_esc`) y los envía al PTY.
    /// Si la tecla no tiene secuencia especial pero trae `key_char` (texto imprimible normal), ese
    /// texto se envía tal cual — es el camino de "escribir letras normales" en la sesión.
    fn enviar_tecla(&mut self, evento: &KeyDownEvent, _cx: &mut Context<Self>) {
        let Some(sesion) = &self.sesion_pty else {
            return;
        };
        let modo = self
            .pantalla_pty
            .as_ref()
            .map(|c| c.modo)
            .unwrap_or_default();
        if let Some(esc) = teclado::a_secuencia_esc(&evento.keystroke, modo, false) {
            sesion.escribir(esc.into_owned().into_bytes());
        } else if let Some(texto) = &evento.keystroke.key_char {
            if !texto.is_empty() {
                sesion.escribir(texto.clone().into_bytes());
            }
        }
    }

    /// Envuelve un `Input` del kit en el marco de control Ethos (mismo criterio que `PanelConfig`:
    /// el Input pinta su fondo con `cx.theme()`, así que lo metemos en un `div` tematizado con
    /// fondo TINTA para que encaje en la paleta en vez de verse con el tema por defecto del kit).
    fn control(&self, estado: &Entity<InputState>) -> impl IntoElement {
        div()
            .w_full()
            .px_3()
            .py_1()
            .rounded(tema::radio(tema::RADIO_CONTROL))
            .bg(tema::TINTA)
            .border_1()
            .border_color(tema::LINEA)
            .child(Input::new(estado).cleanable(true))
    }
}

// -------------------------------------------------------------------------------------------------
// COLOR DE FEEDBACK — coherente con la paleta cálida del tema (mismos tonos que `PanelConfig`).
// -------------------------------------------------------------------------------------------------

/// Verde salvia apagado para el feedback OK (no el verde genérico).
const VERDE_OK: gpui::Rgba = gpui::Rgba {
    r: 0x8F as f32 / 255.0,
    g: 0xB0 as f32 / 255.0,
    b: 0x7B as f32 / 255.0,
    a: 1.0,
};

/// Terracota apagado para el feedback de error (no el rojo genérico).
const ROJO_ERROR: gpui::Rgba = gpui::Rgba {
    r: 0xC9 as f32 / 255.0,
    g: 0x6A as f32 / 255.0,
    b: 0x5A as f32 / 255.0,
    a: 1.0,
};

impl Render for PanelLanzador {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // --- R1/R1.1: fila del directorio (ruta en mono BRASA + Elegir/Revelar/recientes) ---
        let dir_texto = self
            .dir
            .as_ref()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|| "(sin directorio elegido)".to_string());
        let hay_dir = self.dir.is_some();

        let ruta_mostrada = div()
            .flex_1()
            .min_w_0()
            .font_family(tema::FUENTE_MONO)
            .text_sm()
            .text_color(if hay_dir { tema::BRASA } else { tema::HUMO })
            .child(SharedString::from(dir_texto));

        let mut fila_dir = div()
            .h_flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(ruta_mostrada)
            .child(
                tema::boton_primario("lanzador-elegir-dir", "Elegir…").on_click(cx.listener(
                    |esta, _e, _w, cx| {
                        esta.elegir_directorio(cx);
                    },
                )),
            );
        // "Revelar en Finder" (R1.1) sólo si hay un dir elegido.
        if hay_dir {
            fila_dir = fila_dir.child(
                tema::boton_secundario("lanzador-revelar", "Revelar").on_click(cx.listener(
                    |esta, _e, _w, cx| {
                        if let Some(d) = esta.dir.clone() {
                            cx.reveal_path(&d);
                        }
                    },
                )),
            );
        }
        // Botón de recientes (R1.2), sólo si hay alguno guardado.
        if !self.lanzador_cfg.recientes.is_empty() {
            fila_dir = fila_dir.child(
                tema::boton_secundario("lanzador-recientes", "Recientes ▾").on_click(cx.listener(
                    |esta, _e, _w, cx| {
                        esta.dropdown_recientes = !esta.dropdown_recientes;
                        cx.notify();
                    },
                )),
            );
        }

        // Desplegable de recientes (lista de filas seleccionables del tema).
        let recientes_panel: gpui::AnyElement = if self.dropdown_recientes {
            let mut lista = div().v_flex().gap_1().mt_1().p_2();
            lista = lista
                .bg(tema::TINTA)
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .border_1()
                .border_color(tema::LINEA);
            for (i, r) in self.lanzador_cfg.recientes.clone().into_iter().enumerate() {
                let ruta = r.clone();
                lista = lista.child(
                    tema::fila_seleccionable(format!("reciente-{i}"), false)
                        .child(
                            div()
                                .font_family(tema::FUENTE_MONO)
                                .text_sm()
                                .text_color(tema::PAPEL)
                                .child(SharedString::from(r)),
                        )
                        .on_click(cx.listener(move |esta, _e, _w, cx| {
                            esta.elegir_reciente(ruta.clone(), cx);
                        })),
                );
            }
            lista.into_any_element()
        } else {
            div().into_any_element()
        };

        let card_dir = self.seccion(
            "directorio",
            "Dónde arranca la sesión (R1). El picker es nativo del sistema.",
            div().v_flex().gap_2().child(fila_dir).child(recientes_panel),
        );

        // --- R4: selector de destino (Local / SSH / tmux) + campos condicionales ---
        let destino_boton = tema::boton_secundario(
            "lanzador-destino",
            format!("{} ▾", self.destino.etiqueta()),
        )
        .on_click(cx.listener(|esta, _e, _w, cx| {
            esta.dropdown_destino = !esta.dropdown_destino;
            cx.notify();
        }));

        let destino_dropdown: gpui::AnyElement = if self.dropdown_destino {
            let mut lista = div()
                .v_flex()
                .gap_1()
                .mt_1()
                .p_2()
                .bg(tema::TINTA)
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .border_1()
                .border_color(tema::LINEA);
            for d in Destino::TODOS {
                let activa = d == self.destino;
                lista = lista.child(
                    tema::fila_seleccionable(d.id(), activa)
                        .child(tema::texto_primario(d.etiqueta()))
                        .on_click(cx.listener(move |esta, _e, _w, cx| {
                            esta.destino = d;
                            esta.dropdown_destino = false;
                            cx.notify();
                        })),
                );
            }
            lista.into_any_element()
        } else {
            div().into_any_element()
        };

        // Campos condicionales por destino: SSH (host + dir remoto), tmux (nombre).
        let campos_destino: gpui::AnyElement = match self.destino {
            Destino::Local => div().into_any_element(),
            Destino::Ssh => div()
                .v_flex()
                .gap_3()
                .mt_2()
                .child(self.campo("host ssh", "Host del ~/.ssh/config (p.ej. otus).", self.control(&self.host_ssh)))
                .child(self.campo(
                    "directorio remoto",
                    "Ruta en la máquina remota (el picker local no aplica en SSH).",
                    self.control(&self.dir_remoto),
                ))
                .into_any_element(),
            Destino::Tmux => div()
                .v_flex()
                .gap_3()
                .mt_2()
                .child(self.campo(
                    "nombre de la sesión tmux",
                    "Se crea con new-session -d y se hace attach. Si existe, harías attach directo.",
                    self.control(&self.nombre_tmux),
                ))
                .into_any_element(),
        };

        let card_destino = self.seccion(
            "destino",
            "Dónde se ejecuta (R4). En Fase 1 sólo cambia el comando previsualizado.",
            div()
                .v_flex()
                .gap_2()
                .child(destino_boton)
                .child(destino_dropdown)
                .child(campos_destino),
        );

        // --- R2/R2.1: editor de system prompt + plantillas ---
        let mut fila_plantillas = div().h_flex().items_center().gap_2().flex_wrap();
        fila_plantillas = fila_plantillas
            .child(div().flex_1().min_w_0().child(self.control(&self.nombre_plantilla)))
            .child(
                tema::boton_secundario("lanzador-guardar-plantilla", "Guardar plantilla").on_click(
                    cx.listener(|esta, _e, _w, cx| {
                        esta.guardar_plantilla(cx);
                    }),
                ),
            );
        if !self.lanzador_cfg.plantillas.is_empty() {
            fila_plantillas = fila_plantillas.child(
                tema::boton_secundario("lanzador-plantillas", "Cargar ▾").on_click(cx.listener(
                    |esta, _e, _w, cx| {
                        esta.dropdown_plantillas = !esta.dropdown_plantillas;
                        cx.notify();
                    },
                )),
            );
        }

        let plantillas_dropdown: gpui::AnyElement = if self.dropdown_plantillas {
            let mut lista = div()
                .v_flex()
                .gap_1()
                .mt_1()
                .p_2()
                .bg(tema::TINTA)
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .border_1()
                .border_color(tema::LINEA);
            for (i, p) in self.lanzador_cfg.plantillas.clone().into_iter().enumerate() {
                let plantilla = p.clone();
                lista = lista.child(
                    tema::fila_seleccionable(format!("plantilla-{i}"), false)
                        .child(tema::texto_primario(p.nombre))
                        .on_click(cx.listener(move |esta, _e, window, cx| {
                            esta.cargar_plantilla(plantilla.clone(), window, cx);
                        })),
                );
            }
            lista.into_any_element()
        } else {
            div().into_any_element()
        };

        let card_prompt = self.seccion(
            "system prompt",
            "Opcional (R2). Se pasa como --append-system-prompt. Guárdalo como plantilla (R2.1).",
            div()
                .v_flex()
                .gap_3()
                .child(self.control(&self.prompt))
                .child(fila_plantillas)
                .child(plantillas_dropdown),
        );

        // --- R3: lista editable de tareas iniciales (materialización R3.2) ---
        let fila_nueva_tarea = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().child(self.control(&self.nueva_tarea_desc)))
            .child(div().w(gpui::px(120.0)).child(self.control(&self.nueva_tarea_est)))
            .child(
                tema::boton_secundario("lanzador-add-tarea", "Añadir").on_click(cx.listener(
                    |esta, _e, window, cx| {
                        esta.anadir_tarea(window, cx);
                    },
                )),
            );

        let mut lista_tareas = div().v_flex().gap_1();
        for (i, t) in self.tareas.iter().enumerate() {
            let est = if t.estimado.trim().is_empty() {
                String::new()
            } else {
                format!("  ·  {}", t.estimado.trim())
            };
            let mut fila = div()
                .h_flex()
                .items_center()
                .gap_2()
                .w_full()
                .px_3()
                .py_2()
                .rounded(tema::radio(tema::RADIO_CONTROL))
                .bg(tema::TINTA)
                .border_1()
                .border_color(tema::LINEA)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(tema::texto_primario(format!("{}) {}{}", i + 1, t.descripcion, est))),
                );
            // Subir (reordenar) sólo si no es la primera.
            if i > 0 {
                let idx = i;
                fila = fila.child(
                    tema::boton_secundario(format!("tarea-subir-{i}"), "↑").on_click(cx.listener(
                        move |esta, _e, _w, cx| {
                            esta.subir_tarea(idx, cx);
                        },
                    )),
                );
            }
            let idx_q = i;
            fila = fila.child(
                tema::boton_secundario(format!("tarea-quitar-{i}"), "Quitar").on_click(cx.listener(
                    move |esta, _e, _w, cx| {
                        esta.quitar_tarea(idx_q, cx);
                    },
                )),
            );
            lista_tareas = lista_tareas.child(fila);
        }

        let card_tareas = self.seccion(
            "tareas iniciales",
            "Se inyectan en el system prompt al lanzar (R3.2). Añade, reordena o quita.",
            div().v_flex().gap_3().child(fila_nueva_tarea).child(lista_tareas),
        );

        // --- R5: opción avanzada de skip-permisos (default OFF, con aviso) ---
        let switch_skip = tema::fila_seleccionable("lanzador-skip", self.skip_permisos)
            .child(
                div()
                    .flex_1()
                    .child(tema::texto_primario("--dangerously-skip-permissions"))
                    .child(tema::texto_terciario(
                        "AVISADO: la sesión no pedirá confirmación de permisos. Déjalo OFF salvo que sepas por qué.",
                    )),
            )
            .child(tema::chip_estado(
                if self.skip_permisos { "ON" } else { "OFF" },
                if self.skip_permisos { ROJO_ERROR } else { tema::HUMO },
            ))
            .on_click(cx.listener(|esta, _e, _w, cx| {
                esta.skip_permisos = !esta.skip_permisos;
                cx.notify();
            }));

        let card_flags = self.seccion(
            "opciones avanzadas",
            "El flag de canal (server:claude-peers) va SIEMPRE y no se muestra aquí (R5).",
            div().v_flex().gap_2().child(switch_skip),
        );

        // --- R6: preview del comando (mono, scrolleable) + Copiar ---
        let comando = componer_comando(&self.params(cx));
        let preview = div()
            .id("lanzador-preview")
            .w_full()
            .max_h(gpui::px(160.0))
            .overflow_x_scroll()
            .overflow_y_scroll()
            .p_3()
            .rounded(tema::radio(tema::RADIO_CONTROL))
            .bg(tema::TINTA)
            .border_1()
            .border_color(tema::LINEA)
            .font_family(tema::FUENTE_MONO)
            .text_sm()
            .text_color(tema::PAPEL)
            .child(SharedString::from(comando));

        // R7: "Lanzar aquí" sólo tiene sentido con destino Local (v1 del terminal embebido); en
        // SSH/tmux el botón queda pero `lanzar_pty` degrada a banner (R7.2) explicando por qué.
        let hay_sesion = self.sesion_pty.is_some();
        let boton_pty = if hay_sesion {
            tema::boton_secundario("lanzador-cerrar-pty", "Cerrar terminal").on_click(cx.listener(
                |esta, _e, _w, cx| {
                    esta.cerrar_pty(cx);
                },
            ))
        } else {
            tema::boton_primario("lanzador-lanzar-pty", "Lanzar aquí").on_click(cx.listener(
                |esta, _e, window, cx| {
                    esta.lanzar_pty(window, cx);
                },
            ))
        };

        // R7/E-05: id explícito de agente, opcional. Sólo se usa al "Lanzar aquí" (PTY embebido);
        // "Copiar comando" NO lo incluye (no es parte de la cadena de shell del preview, es un env
        // var que se inyecta al proceso, ver `id_agente()`). Alcance de HOY: campo libre, sin
        // Cargo/Proyecto (RFC-organigrama-roles no implementada todavía).
        let campo_id_agente = self.campo(
            "id de agente (opcional)",
            "CLAUDE_PEERS_ID al lanzar con \"Lanzar aquí\" (p.ej. backend@proyecto-x). Vacío = comportamiento actual, sin cambios.",
            self.control(&self.id_agente),
        );

        let botonera = div()
            .h_flex()
            .items_center()
            .gap_3()
            .child(
                tema::boton_primario("lanzador-copiar", "Copiar comando").on_click(cx.listener(
                    |esta, _e, _w, cx| {
                        esta.copiar_comando(cx);
                    },
                )),
            )
            .child(boton_pty)
            .child(tema::texto_terciario(
                "Copia el comando o lánzalo en un terminal embebido (R7, sólo Local por ahora).",
            ));

        let feedback: gpui::AnyElement = match &self.feedback {
            Feedback::Ninguno => div().into_any_element(),
            Feedback::Ok(msg) => div()
                .text_sm()
                .text_color(VERDE_OK)
                .child(SharedString::from(format!("✓ {msg}")))
                .into_any_element(),
            Feedback::Error(msg) => div()
                .text_sm()
                .text_color(ROJO_ERROR)
                .child(SharedString::from(format!("⚠ {msg}")))
                .into_any_element(),
        };

        let card_comando = self.seccion(
            "comando",
            "Lo que se ejecutaría, exacto (R6). Revísalo antes de copiar.",
            div()
                .v_flex()
                .gap_3()
                .child(preview)
                .child(campo_id_agente)
                .child(botonera)
                .child(feedback),
        );

        // --- R7: card del terminal embebido — banner de error (R7.2), rejilla activa, o ausente.
        // `AnyElement` porque las 3 ramas devuelven tipos concretos distintos (banner vs `seccion`
        // con la rejilla vs `div()` vacío) y `seccion()` exige `impl IntoElement` homogéneo.
        let card_terminal: gpui::AnyElement = if let Some(msg) = &self.error_pty {
            self.seccion(
                "terminal",
                "R7.2 — el terminal embebido no pudo abrirse; usa \"Copiar comando\" como alternativa.",
                div()
                    .text_sm()
                    .text_color(ROJO_ERROR)
                    .child(SharedString::from(format!("⚠ {msg}"))),
            )
            .into_any_element()
        } else if let Some(contenido) = &self.pantalla_pty {
            // R7 iteración 2: `self.dim_pty` es la dimensión REAL medida al lanzar (ver
            // `medir_dimensiones_ventana`), no `POR_DEFECTO` — la rejilla pintada y el alto del área
            // usan la MISMA dimensión con la que se abrió el PTY, o divergirían del wrap real.
            let dim = self.dim_pty;
            let rejilla = render_rejilla_pty(contenido, dim);
            // Alto real = filas × line-height medido de la ventana (no el `20.0` arbitrario de v1) +
            // el padding del área (`p_2` = 8px por lado, ver más abajo).
            let alto_area = window.line_height() * dim.filas as f32 + gpui::px(16.0);
            self.seccion(
                "terminal",
                "R7 — sesión activa. Click dentro para escribir; \"Cerrar terminal\" arriba para salir.",
                div()
                    .id("lanzador-pty-area")
                    .track_focus(&self.foco_pty)
                    .key_context("LanzadorPty")
                    .on_key_down(cx.listener(|esta, evento: &KeyDownEvent, _w, cx| {
                        esta.enviar_tecla(evento, cx);
                    }))
                    .on_click(cx.listener(|esta, _e, window, cx| {
                        window.focus(&esta.foco_pty, cx);
                    }))
                    .w_full()
                    .h(alto_area)
                    .overflow_hidden()
                    .p_2()
                    .rounded(tema::radio(tema::RADIO_CONTROL))
                    .bg(tema::TINTA)
                    .border_1()
                    .border_color(tema::LINEA)
                    .child(rejilla),
            )
            .into_any_element()
        } else {
            div().into_any_element()
        };

        // Cuerpo SCROLLABLE de las 6+1 cards: con el editor de prompt (auto_grow hasta 14 líneas) +
        // tareas + terminal embebido (R7), la pantalla desborda fácilmente la altura de la ventana
        // y las últimas cards (flags/comando) quedaban inalcanzables (bug QA 03/07). Mismo patrón
        // que `alertas.rs::tabla`/`tareas.rs`: `flex_1()` + `min_h_0()` + `overflow_y_scroll()` en
        // el cuerpo, encabezado FIJO fuera de él.
        let cuerpo = div()
            .id("lanzador-cuerpo-scroll")
            .v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_5()
            .px_8()
            .pb_8()
            .child(card_dir)
            .child(card_destino)
            .child(card_prompt)
            .child(card_tareas)
            .child(card_flags)
            .child(card_comando)
            .child(card_terminal);

        // Raíz de la pantalla: fondo Ethos + cabecera FIJA + cuerpo scrollable.
        // USA `fondo_app()` (`size_full`), NO `raiz_scrollable()`. Esta pantalla tiene su PROPIO
        // cuerpo scrollable interno (`flex_1` + `min_h_0` + `overflow_y_scroll`), con la cabecera
        // FIJA fuera — el patrón "header fijo + cuerpo scroll" de `alertas.rs`/`tareas.rs`. Ese patrón
        // EXIGE que el raíz tenga alto acotado (`size_full`) para que el `flex_1` del cuerpo reparta
        // el espacio restante; con `raiz_scrollable()` (sin alto), el `flex_1` no tenía referencia y
        // el scroll interno NO se activaba (bug: las últimas cards inalcanzables). Las vistas que SÍ
        // delegan al scroll de app (broker/config/jornada…) NO tienen cuerpo scrollable propio y por
        // eso usan `raiz_scrollable()`; el Lanzador sí lo tiene, así que va como alertas.
        tema::fondo_app()
            .v_flex()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .p_8()
                    .pb_5()
                    .child(tema::eyebrow("Lanzador"))
                    .child(tema::titulo("Configurar sesión"))
                    .child(tema::texto_terciario(
                        "Elige directorio, prompt, tareas y destino; copia el comando de arranque.",
                    )),
            )
            .child(cuerpo)
    }
}

impl PanelLanzador {
    /// Envoltorio de sección: card Ethos con eyebrow + ayuda terciaria + cuerpo. Unifica el
    /// layout de las 6 tarjetas de la pantalla sin repetir el padding/flex en cada una.
    fn seccion(
        &self,
        etiqueta: &'static str,
        ayuda: &'static str,
        cuerpo: impl IntoElement,
    ) -> impl IntoElement {
        tema::superficie_card()
            .v_flex()
            .w_full()
            .gap_3()
            .p_6()
            .child(tema::eyebrow(etiqueta))
            .child(tema::texto_terciario(ayuda))
            .child(cuerpo)
    }

    /// Campo etiquetado (eyebrow + control) para los inputs condicionales de SSH/tmux.
    fn campo(
        &self,
        etiqueta: &'static str,
        ayuda: &'static str,
        control: impl IntoElement,
    ) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .child(tema::eyebrow(etiqueta))
            .child(control)
            .child(tema::texto_terciario(ayuda))
    }
}

// -------------------------------------------------------------------------------------------------
// STUB DE LA FUNDACIÓN + CONSTRUCTOR (firma intacta: `render_lanzador(&EstadoPantalla)`)
// -------------------------------------------------------------------------------------------------

/// R7 — resuelve un `ColorPty` al `Rgba` del tema Ethos más cercano. v1 (R7 core, ver `pty.rs`):
/// los 16 colores ANSI con nombre se colapsan a un subconjunto de tokens Ethos (no hay 16 tonos
/// propios en la paleta del tema — inventar 16 colores nuevos sólo para el terminal rompería la
/// coherencia visual del resto de la app); RGB truecolor se pasa literal (ahí sí hay un valor
/// exacto que respetar, típicamente lo pide el propio programa dentro del PTY, p.ej. colores de
/// sintaxis). Degradación aceptada, documentada — no es un theming completo de terminal.
fn color_pty_a_rgba(c: ColorPty, es_fondo: bool) -> gpui::Rgba {
    match c {
        ColorPty::PorDefecto => {
            if es_fondo {
                tema::TINTA
            } else {
                tema::PAPEL
            }
        }
        // Subconjunto de los 16 ANSI: rojo/verde/amarillo/azul/magenta/cian mapean a los tonos
        // cálidos más cercanos de la paleta (reusa `VERDE_OK`/`ROJO_ERROR`, ya definidos en este
        // archivo para el feedback de la card de comando — mismo criterio, no inventa tokens
        // nuevos); el resto (negro/blanco/gris/brights) cae a PAPEL/HUMO.
        ColorPty::Ansi(1) | ColorPty::Ansi(9) => ROJO_ERROR,
        ColorPty::Ansi(2) | ColorPty::Ansi(10) => VERDE_OK,
        ColorPty::Ansi(3) | ColorPty::Ansi(11) => tema::BRASA,
        ColorPty::Ansi(4) | ColorPty::Ansi(12) => tema::SALMO,
        ColorPty::Ansi(5) | ColorPty::Ansi(13) => tema::SALMO,
        ColorPty::Ansi(6) | ColorPty::Ansi(14) => tema::BRASA,
        ColorPty::Ansi(0) | ColorPty::Ansi(8) => tema::HUMO,
        ColorPty::Ansi(_) => tema::PAPEL,
        ColorPty::Rgb(r, g, b) => gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
    }
}

/// R7 — pinta el `ContenidoPty` como filas de texto monoespaciado. NO es un elemento GPUI de bajo
/// nivel celda-por-celda (eso exigiría implementar `Element` a mano: medición/layout propios,
/// fuera de alcance de R7 core) — reusa el mismo patrón de texto plano que YA usa el preview del
/// comando (R6, `font_family(tema::FUENTE_MONO)` sobre un `div` scrolleable), agrupando celdas
/// consecutivas de igual color en un solo span por corrida en vez de un `div` por celda (80
/// columnas × 24 filas = 1920 celdas/frame sería una explosión de elementos innecesaria).
///
/// Degradación v1 (documentada, no oculta): sin selección de texto, sin hyperlinks, sin negrita/
/// subrayado real todavía (se leen del backend pero no se pintan aún — `CeldaPty::negrita`/
/// `subrayado` quedan para una iteración siguiente), sin scrollback (sólo la pantalla visible).
fn render_rejilla_pty(contenido: &crate::pty::ContenidoPty, dim: DimensionesCeldas) -> impl IntoElement {
    use std::collections::BTreeMap;

    // Agrupa celdas por fila (BTreeMap para iterar en orden de línea) y dentro de cada fila arma
    // corridas contiguas de igual (fg,bg) — el span mínimo que evita un div por celda.
    let mut filas: BTreeMap<i32, Vec<&crate::pty::CeldaPty>> = BTreeMap::new();
    for celda in &contenido.celdas {
        filas.entry(celda.fila).or_default().push(celda);
    }

    let mut cuerpo = div()
        .v_flex()
        .size_full()
        .font_family(tema::FUENTE_MONO)
        .text_sm();

    // Columna del cursor en ESTA fila, si el cursor visible cae aquí — `None` en cualquier otra
    // fila. Se calcula una vez por fila (no por celda) para no repetir la comparación en el loop.
    let col_cursor_en = |fila_idx: i32| -> Option<usize> {
        (contenido.cursor_visible && contenido.cursor_fila == fila_idx)
            .then_some(contenido.cursor_columna)
    };

    for fila_idx in 0..dim.filas as i32 {
        let celdas_fila = filas.get(&fila_idx);
        let cursor_col = col_cursor_en(fila_idx);
        let mut linea = div().h_flex();

        if let Some(celdas) = celdas_fila {
            let mut ordenadas: Vec<&&crate::pty::CeldaPty> = celdas.iter().collect();
            ordenadas.sort_by_key(|c| c.columna);

            let mut i = 0;
            while i < ordenadas.len() {
                // R7 iteración 2 — cursor visual: si la celda actual ES la columna del cursor,
                // la corta como span de 1 celda con colores invertidos (bloque sólido) en vez de
                // agruparla con sus vecinas — necesita destacarse aunque comparta fg/bg con ellas.
                if cursor_col == Some(ordenadas[i].columna) {
                    let c = ordenadas[i];
                    linea = linea.child(
                        div()
                            // Invertido: el bg de la celda pasa a ser el texto, y PAPEL de fondo —
                            // mismo criterio visual que un cursor de bloque de terminal clásico.
                            .text_color(color_pty_a_rgba(c.bg, true))
                            .bg(color_pty_a_rgba(c.fg, false))
                            .child(SharedString::from(c.caracter.to_string())),
                    );
                    i += 1;
                    continue;
                }

                let fg = ordenadas[i].fg;
                let bg = ordenadas[i].bg;
                let mut texto = String::new();
                while i < ordenadas.len()
                    && ordenadas[i].fg == fg
                    && ordenadas[i].bg == bg
                    && cursor_col != Some(ordenadas[i].columna)
                {
                    texto.push(ordenadas[i].caracter);
                    i += 1;
                }
                let mut span = div().text_color(color_pty_a_rgba(fg, false));
                if bg != ColorPty::PorDefecto {
                    span = span.bg(color_pty_a_rgba(bg, true));
                }
                linea = linea.child(span.child(SharedString::from(texto)));
            }
        } else if cursor_col == Some(0) {
            // Fila vacía pero el cursor está en ella (columna 0, línea recién abierta): pintar el
            // bloque igual que si hubiera una celda — si no, el cursor "desaparece" en líneas nuevas.
            linea = linea.child(
                div()
                    .text_color(tema::TINTA)
                    .bg(tema::PAPEL)
                    .child(SharedString::from(" ")),
            );
        } else {
            // Fila vacía (sin salida aún, sin cursor): un espacio para que la altura no colapse.
            linea = linea.child(SharedString::from(" "));
        }

        cuerpo = cuerpo.child(linea);
    }

    cuerpo
}

/// Stub de la Fundación: la pantalla Lanzador delega en su `Entity<PanelLanzador>`, creada por la
/// app al arrancar y guardada en `EstadoPantalla`. Si no está inicializada, pinta un aviso
/// tematizado en vez de crashear (nunca `.unwrap()` sobre el Option).
pub fn render_lanzador(datos: &EstadoPantalla) -> impl IntoElement {
    match &datos.panel_lanzador {
        Some(panel) => div().size_full().child(panel.clone()),
        None => tema::raiz_scrollable()
            .v_flex()
            .p_8()
            .gap_2()
            .child(tema::eyebrow("Lanzador"))
            .child(tema::texto_primario("Lanzador no inicializado.")),
    }
}

/// Helper de la app para construir la `Entity<PanelLanzador>` al arrancar (mismo patrón que
/// `config::nuevo_panel`). Se expone aquí para no filtrar el tipo interno del panel a `app.rs`.
pub fn nuevo_panel(window: &mut Window, cx: &mut App) -> Entity<PanelLanzador> {
    cx.new(|cx| PanelLanzador::nuevo(window, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(desc: &str, est: &str) -> TareaInicial {
        TareaInicial {
            descripcion: desc.into(),
            estimado: est.into(),
        }
    }

    #[test]
    fn escapado_shell_metacaracteres() {
        // Comillas dobles, backslash, backtick y $ se escapan; el resto queda literal.
        assert_eq!(escapar_shell("hola"), "\"hola\"");
        assert_eq!(escapar_shell("di \"hola\""), "\"di \\\"hola\\\"\"");
        assert_eq!(escapar_shell("$HOME"), "\"\\$HOME\"");
        assert_eq!(escapar_shell("a`b`c"), "\"a\\`b\\`c\"");
        assert_eq!(escapar_shell("c:\\ruta"), "\"c:\\\\ruta\"");
        // Salto de línea: literal dentro de comillas dobles POSIX (no se toca).
        assert_eq!(escapar_shell("l1\nl2"), "\"l1\nl2\"");
    }

    #[test]
    fn materializar_prompt_solo_base() {
        assert_eq!(materializar_prompt("eres un peer", &[]), "eres un peer");
        // Tareas con descripción vacía no cuentan.
        assert_eq!(materializar_prompt("base", &[t("  ", "1h")]), "base");
    }

    #[test]
    fn materializar_prompt_con_tareas() {
        let out = materializar_prompt("Eres peer.", &[t("revisar X", "30m"), t("cerrar Y", "")]);
        assert_eq!(
            out,
            "Eres peer.\n\nTus tareas de hoy:\n1) revisar X (30m)\n2) cerrar Y"
        );
        // Sin base: arranca directo por el bloque de tareas.
        let out2 = materializar_prompt("", &[t("solo tarea", "")]);
        assert_eq!(out2, "Tus tareas de hoy:\n1) solo tarea");
    }

    fn params_base() -> ParamsComando {
        ParamsComando {
            dir: "/Users/max/proy".into(),
            system_prompt: String::new(),
            destino: Destino::Local,
            host_ssh: String::new(),
            nombre_tmux: String::new(),
            skip_permisos: false,
        }
    }

    #[test]
    fn comando_local_siempre_lleva_flag_canal() {
        let cmd = componer_comando(&params_base());
        assert_eq!(
            cmd,
            "cd \"/Users/max/proy\" && claude --dangerously-load-development-channels server:claude-peers"
        );
    }

    #[test]
    fn comando_local_con_prompt_y_skip() {
        let mut p = params_base();
        p.system_prompt = "eres $peer".into();
        p.skip_permisos = true;
        let cmd = componer_comando(&p);
        assert_eq!(
            cmd,
            "cd \"/Users/max/proy\" && claude --append-system-prompt \"eres \\$peer\" \
             --dangerously-load-development-channels server:claude-peers \
             --dangerously-skip-permissions"
        );
    }

    #[test]
    fn comando_ssh_envuelve_remoto() {
        let mut p = params_base();
        p.destino = Destino::Ssh;
        p.host_ssh = "otus".into();
        p.dir = "/remoto/proy".into();
        let cmd = componer_comando(&p);
        assert_eq!(
            cmd,
            "ssh -t otus \"cd \\\"/remoto/proy\\\" && claude \
             --dangerously-load-development-channels server:claude-peers\""
        );
    }

    #[test]
    fn comando_tmux_new_y_attach() {
        let mut p = params_base();
        p.destino = Destino::Tmux;
        p.nombre_tmux = "s1".into();
        let cmd = componer_comando(&p);
        assert_eq!(
            cmd,
            "tmux new-session -d -s s1 -c \"/Users/max/proy\" \
             \"claude --dangerously-load-development-channels server:claude-peers\" \
             && tmux attach -t s1"
        );
    }

    #[test]
    fn comando_sin_dir_no_pone_cd() {
        let mut p = params_base();
        p.dir = String::new();
        let cmd = componer_comando(&p);
        assert_eq!(
            cmd,
            "claude --dangerously-load-development-channels server:claude-peers"
        );
    }

    #[test]
    fn argv_claude_local_parte_flag_canal_en_dos_tokens() {
        // Regresión: FLAG_CANAL es "--flag valor" (2 tokens de shell). `componer_comando` lo deja
        // intacto porque una subshell lo retokeniza; `argv_claude` va directo a execve (sin
        // subshell), así que DEBE partirlo o el flag llega pegado y `claude` no lo reconoce.
        let p = params_base();
        let (programa, args, dir) = argv_claude(&p).expect("Local siempre produce Some");
        assert_eq!(programa, "claude");
        assert_eq!(
            args,
            vec![
                "--dangerously-load-development-channels".to_string(),
                "server:claude-peers".to_string(),
            ]
        );
        assert_eq!(dir, Some(PathBuf::from("/Users/max/proy")));
    }

    #[test]
    fn argv_claude_local_con_prompt_y_skip_no_escapa() {
        // A diferencia de `componer_comando` (R6, va a una subshell y escapa comillas/$/backtick),
        // `argv_claude` entrega el prompt TAL CUAL como un único elemento de argv — el kernel no
        // reinterpreta metacaracteres dentro de un argumento de `execve`, así que escaparlo aquí
        // sería incorrecto (duplicaría comillas que `claude` vería literales).
        let mut p = params_base();
        p.system_prompt = "eres $peer \"citado\"".into();
        p.skip_permisos = true;
        let (_, args, _) = argv_claude(&p).expect("Local siempre produce Some");
        assert_eq!(args[0], "--append-system-prompt");
        assert_eq!(args[1], "eres $peer \"citado\"");
        assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn argv_claude_sin_dir_devuelve_none_path() {
        let mut p = params_base();
        p.dir = String::new();
        let (_, _, dir) = argv_claude(&p).expect("Local siempre produce Some");
        assert_eq!(dir, None);
    }

    #[test]
    fn argv_claude_ssh_y_tmux_fuera_de_alcance_v1() {
        // R7 v1 sólo cubre Local (nota de diseño del RFC Fase 2, §3 paso 3 vs 4). SSH/tmux
        // devuelven `None` explícito — la UI cae al fallback de "copiar comando" (R7.2) en vez de
        // intentar abrir un PTY a medio cablear.
        let mut ssh = params_base();
        ssh.destino = Destino::Ssh;
        assert_eq!(argv_claude(&ssh), None);

        let mut tmux = params_base();
        tmux.destino = Destino::Tmux;
        assert_eq!(argv_claude(&tmux), None);
    }

    #[test]
    fn calcular_dimensiones_divide_viewport_entre_celda() {
        // 800px de ancho / 8px por celda = 100 columnas; 480px de alto / 20px de línea = 24 filas.
        let dim = calcular_dimensiones(
            gpui::px(800.0),
            gpui::px(480.0),
            gpui::px(8.0),
            gpui::px(20.0),
        );
        assert_eq!(dim.columnas, 100);
        assert_eq!(dim.filas, 24);
    }

    #[test]
    fn calcular_dimensiones_trunca_hacia_abajo_no_redondea() {
        // 85px / 8px = 10.625 → 10 columnas, NUNCA 11 (una columna de más desbordaría el ancho
        // real disponible y el wrap de línea del PTY no coincidiría con lo que GPUI pinta).
        let dim = calcular_dimensiones(gpui::px(85.0), gpui::px(100.0), gpui::px(8.0), gpui::px(20.0));
        assert_eq!(dim.columnas, 10);
    }

    #[test]
    fn calcular_dimensiones_nunca_da_cero_ni_con_viewport_minusculo() {
        // R10: 0 filas/columnas haría que `Term::new`/`tty::new` (que las usan como `Dimensions`)
        // entren en división por cero al calcular el wrap de línea — guardrail `max(1)`.
        let dim = calcular_dimensiones(gpui::px(1.0), gpui::px(1.0), gpui::px(8.0), gpui::px(20.0));
        assert_eq!(dim.columnas, 1);
        assert_eq!(dim.filas, 1);

        let dim_cero = calcular_dimensiones(gpui::px(0.0), gpui::px(0.0), gpui::px(8.0), gpui::px(20.0));
        assert_eq!(dim_cero.columnas, 1);
        assert_eq!(dim_cero.filas, 1);
    }
}
