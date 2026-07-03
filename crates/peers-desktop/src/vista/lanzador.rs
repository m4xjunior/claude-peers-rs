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

use gpui::{
    div, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_component::{
    input::{Input, InputState},
    StyledExt,
};
use std::path::PathBuf;

use crate::app::EstadoPantalla;
use crate::config::{Config, PlantillaPrompt};
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

/// COMPONE el comando exacto a previsualizar (R6) según los parámetros. Es la única fuente de
/// verdad del preview y de "Copiar". SIEMPRE incluye el flag de canal (R5). No ejecuta nada.
///
/// Formas por destino (verificadas contra §11.4 de la RFC):
/// - Local: `cd <dir> && claude <flags>`
/// - SSH:   `ssh -t <host> "cd <dir> && claude <flags>"`
/// - tmux:  `tmux new-session -d -s <nombre> -c <dir> "claude <flags>" && tmux attach -t <nombre>`
pub fn componer_comando(p: &ParamsComando) -> String {
    // Flags de `claude` comunes a los tres destinos, en orden estable.
    let mut flags: Vec<String> = Vec::new();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(tema::texto_terciario(
                "En Fase 1 el Lanzador previsualiza y copia; no ejecuta nada (R6).",
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
            div().v_flex().gap_3().child(preview).child(botonera).child(feedback),
        );

        // Raíz de la pantalla: fondo Ethos + cabecera + tarjetas apiladas.
        tema::fondo_app()
            .v_flex()
            .gap_5()
            .p_8()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(tema::eyebrow("Lanzador"))
                    .child(tema::titulo("Configurar sesión"))
                    .child(tema::texto_terciario(
                        "Elige directorio, prompt, tareas y destino; copia el comando de arranque.",
                    )),
            )
            .child(card_dir)
            .child(card_destino)
            .child(card_prompt)
            .child(card_tareas)
            .child(card_flags)
            .child(card_comando)
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

/// Stub de la Fundación: la pantalla Lanzador delega en su `Entity<PanelLanzador>`, creada por la
/// app al arrancar y guardada en `EstadoPantalla`. Si no está inicializada, pinta un aviso
/// tematizado en vez de crashear (nunca `.unwrap()` sobre el Option).
pub fn render_lanzador(datos: &EstadoPantalla) -> impl IntoElement {
    match &datos.panel_lanzador {
        Some(panel) => div().size_full().child(panel.clone()),
        None => tema::fondo_app()
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
}
