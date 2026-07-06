//! Backend del terminal PTY embebido — RFC Lanzador **Fase 2**, Zona B (R7).
//!
//! VÍA (B) de la RFC: consume `alacritty_terminal` directo (NO el crate `terminal`/`terminal_view`
//! de Zed, que arrastra `settings`/`theme`/`task` — pesado y acoplado al resto de Zed). El render
//! de rejilla y el mapeo de teclado se implementan aquí y en `tema`/`vista::lanzador`. Patrón de
//! apertura de PTY y extracción de contenido verificado de primera mano contra el checkout local
//! del rev FIJADO en `Cargo.toml` (`alacritty_terminal` rev `fcf32fe`, vía el wrapper interno de
//! Zed `crates/terminal/src/alacritty.rs` del rev `1d217ee` como referencia de USO correcto de la
//! API — no se copia ese wrapper, sólo se replica su secuencia de llamadas).
//!
//! CICLO DE VIDA: `SesionPty::abrir(...)` compone `tty::Options` (shell + cwd + env), abre el PTY
//! (`tty::new`), crea el `Term` envuelto en `FairMutex` (estado compartido con el hilo del event
//! loop) y lanza `EventLoop::spawn()` — un hilo del SISTEMA OPERATIVO, no tokio. La comunicación de
//! vuelta al hilo de GPUI es un canal `futures::channel::mpsc::unbounded`, que SÍ es seguro dentro
//! de `cx.spawn` (no toca el runtime tokio; la trampa anti-SIGABRT documentada en el proyecto es
//! específica de `reqwest`, no de cualquier `.await`). Input: `SesionPty::escribir(bytes)` empuja al
//! PTY vía `Notifier` (no bloqueante, sólo encola en el hilo del event loop).
//!
//! NUNCA `.unwrap()`/`.expect()` en las rutas de apertura: `abrir()` devuelve `io::Result` y el
//! caller (`vista::lanzador`) degrada a banner Ethos (R7.2/R10), nunca panic.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use alacritty_terminal::event::{Event as EventoAlac, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as ConfigTerm, Term};
use alacritty_terminal::tty::{self, Options as OpcionesTty, Shell as ShellTty};
use alacritty_terminal::vte::ansi::{Color as ColorAnsi, NamedColor};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

/// Reenvía los eventos del `Term`/PTY (llegan desde el hilo del event loop, ver `EventListener`)
/// al canal `futures` que consume `cx.spawn` en GPUI. `Clone` porque `Term::new` y `EventLoop::new`
/// piden su propia copia del listener.
#[derive(Clone)]
struct Escucha(UnboundedSender<EventoPty>);

impl EventListener for Escucha {
    fn send_event(&self, evento: EventoAlac) {
        // Colapsa el evento crudo de alacritty al subconjunto que a `vista::lanzador` le interesa;
        // el resto (ClipboardLoad, ColorRequest, TextAreaSizeRequest…) no aplica sin un terminal
        // completo (fuera de alcance R7.1/R7.2) y se ignora en vez de bloquear el match.
        let mapeado = match evento {
            EventoAlac::Wakeup => Some(EventoPty::Refrescar),
            EventoAlac::Exit | EventoAlac::ChildExit(_) => Some(EventoPty::Terminado),
            EventoAlac::Bell => Some(EventoPty::Campana),
            EventoAlac::Title(t) => Some(EventoPty::Titulo(t)),
            _ => None,
        };
        if let Some(m) = mapeado {
            // Canal cerrado (sesión ya soltada del lado GPUI): no hay a quién avisar, no-op.
            let _ = self.0.unbounded_send(m);
        }
    }
}

/// Eventos que la sesión PTY empuja hacia el consumidor GPUI (ver `SesionPty::eventos`).
#[derive(Debug, Clone)]
pub enum EventoPty {
    /// Hay salida nueva: releer `contenido()` y `cx.notify()`.
    Refrescar,
    /// El proceso hijo terminó (shell salió, `exit`, Ctrl+D…). R7.1: ofrecer cerrar la pestaña.
    Terminado,
    /// Secuencia BEL recibida (`\x07`). Se ignora visualmente en v1 (sin flash/sonido).
    Campana,
    /// Cambio de título vía secuencia OSC. Informativo; no se usa aún en v1.
    Titulo(String),
}

/// Dimensiones del terminal en celdas (no en píxeles: aquí no se conoce fuente/línea). El caller
/// mide la fuente monoespaciada elegida y calcula filas/columnas antes de abrir la sesión.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionesCeldas {
    pub filas: u16,
    pub columnas: u16,
}

impl DimensionesCeldas {
    /// Tamaño de arranque razonable si el caller aún no midió el layout real (se redimensiona con
    /// `SesionPty::redimensionar` en cuanto GPUI mide el primer frame).
    pub const POR_DEFECTO: Self = Self { filas: 24, columnas: 80 };
}

/// Wrapper mínimo de `Dimensions` para pasar `DimensionesCeldas` a `Term::new`/`tty::new`, que
/// exigen ese trait (no un struct concreto). Sin scrollback propio en v1 (R7.1 básico): `total_lines
/// == screen_lines`, igual que hace Zed para el caso de sólo-resize (ver su comentario en
/// `alacritty.rs::impl Dimensions for TerminalBounds`).
struct Dims(DimensionesCeldas);

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }
    fn screen_lines(&self) -> usize {
        self.0.filas as usize
    }
    fn columns(&self) -> usize {
        self.0.columnas as usize
    }
}

fn window_size_de(dim: DimensionesCeldas) -> WindowSize {
    // cell_width/cell_height en píxeles no se conocen aquí (v1 no reenvía TIOCSWINSZ con medida de
    // fuente real); 1 es el mínimo no-cero que acepta el ioctl y no afecta al render de celdas (el
    // wrapping de línea del PTY usa num_cols, no el tamaño en píxeles).
    WindowSize {
        num_lines: dim.filas,
        num_cols: dim.columnas,
        cell_width: 1,
        cell_height: 1,
    }
}

/// Qué comando arrancar dentro del PTY. Puente con `vista::lanzador::ParamsComando`: el caller
/// decide `program`/`args`/`dir` a partir del MISMO origen de verdad que compone el preview (R6),
/// sin reparsear la cadena de shell (nota de diseño §1.2 del RFC Fase 2).
#[derive(Debug, Clone)]
pub struct ComandoPty {
    pub programa: String,
    pub args: Vec<String>,
    pub dir: Option<PathBuf>,
    /// Variables de entorno adicionales para el proceso hijo (E-05: binding `CLAUDE_PEERS_ID`).
    /// `Vec<(String, String)>` y no `HashMap` — coherente con el resto del módulo (`ParamsComando`/
    /// `argv_claude` son todo `String`/`Vec<String>` plano) y más simple de construir con un solo
    /// `push` sin la Entry API; se colecciona a `HashMap` recién en `SesionPty::abrir`, que es
    /// donde `alacritty_terminal::tty::Options` realmente lo exige.
    pub env: Vec<(String, String)>,
}

/// Sesión de terminal embebido viva: PTY + `Term` + hilo del event loop + canal de eventos hacia
/// GPUI. Una instancia = una pestaña (R7.1 construye varias de éstas).
pub struct SesionPty {
    termino: Arc<FairMutex<Term<Escucha>>>,
    envio_pty: Notifier,
    /// `None` tras el primer `.next()` que lo drena hacia GPUI — `Option` porque `UnboundedReceiver`
    /// no es `Clone` y `SesionPty` necesita construirse antes de que exista el `cx.spawn` que lo
    /// consume (ver `vista::lanzador`).
    eventos: Option<UnboundedReceiver<EventoPty>>,
    dim_actual: DimensionesCeldas,
}

impl SesionPty {
    /// Abre el PTY y arranca la sesión (camino feliz R7 Local; SSH/tmux reusan esto pasando
    /// `programa="ssh"`/`"tmux"` con los args ya compuestos por `vista::lanzador`, R4.2/R4.3).
    ///
    /// Nunca panic: cualquier fallo de `tty::new`/`EventLoop::new` (permisos, plataforma sin PTY,
    /// binario no encontrado — este último en realidad falla dentro del proceso hijo, no aquí) se
    /// propaga como `io::Error` para que el caller banner-ee (R7.2/R10).
    pub fn abrir(comando: ComandoPty, dim: DimensionesCeldas) -> io::Result<Self> {
        let opciones = OpcionesTty {
            shell: Some(ShellTty::new(comando.programa, comando.args)),
            working_directory: comando.dir,
            drain_on_exit: true,
            // E-05: env real del proceso hijo (agnóstico de shell — a diferencia de un `export`
            // en el string de shell, esto no depende de que el PTY corra un shell POSIX que lo
            // respete; alacritty_terminal lo aplica directo al `execve`, igual en Local/SSH/tmux).
            env: comando.env.into_iter().collect(),
            #[cfg(not(windows))]
            child_signal_mask: None,
            #[cfg(target_os = "windows")]
            escape_args: false,
        };

        // window_id=0: no hay ventana X11/winit real detrás (GPUI pinta la rejilla a mano); el
        // id sólo se usa en Linux para el hint `WINDOWID` del shell, que aquí no aplica.
        let pty = tty::new(&opciones, window_size_de(dim), 0)?;

        let (tx_eventos, rx_eventos) = unbounded();
        let escucha = Escucha(tx_eventos);

        let config = ConfigTerm::default();
        let term = Term::new(config, &Dims(dim), escucha.clone());
        let termino = Arc::new(FairMutex::new(term));

        let event_loop = EventLoop::new(termino.clone(), escucha, pty, opciones.drain_on_exit, false)
            .map_err(io::Error::other)?;
        let envio_pty = Notifier(event_loop.channel());
        // El hilo del event loop sobrevive mientras `envio_pty`/`termino` sigan vivos (lo dejamos
        // correr en background; se cierra solo cuando el proceso hijo termina y `drain_on_exit`
        // vacía el buffer, o explícitamente vía `cerrar()`).
        let _lector = event_loop.spawn();

        Ok(Self {
            termino,
            envio_pty,
            eventos: Some(rx_eventos),
            dim_actual: dim,
        })
    }

    /// Toma el receptor de eventos UNA vez (lo consume `cx.spawn` en un loop `while let Some(e) =
    /// rx.next().await`). Llamar dos veces devuelve `None` la segunda — el caller lo guarda, no lo
    /// vuelve a pedir.
    pub fn tomar_eventos(&mut self) -> Option<UnboundedReceiver<EventoPty>> {
        self.eventos.take()
    }

    /// Envía bytes crudos al PTY (ya traducidos a secuencia ANSI por `teclado::a_secuencia_esc`, o
    /// texto plano pegado/tecleado). No bloquea: sólo encola en el canal del event loop.
    pub fn escribir(&self, bytes: impl Into<std::borrow::Cow<'static, [u8]>>) {
        self.envio_pty.notify(bytes);
    }

    /// Redimensiona el PTY (ioctl `TIOCSWINSZ`) y el `Term` cuando el layout de GPUI cambia. Barato
    /// si `dim` no cambió respecto al último valor conocido — evita spamear el ioctl en cada frame.
    pub fn redimensionar(&mut self, dim: DimensionesCeldas) {
        if dim.filas == self.dim_actual.filas && dim.columnas == self.dim_actual.columnas {
            return;
        }
        self.dim_actual = dim;
        self.termino.lock().resize(Dims(dim));
        if let Err(e) = self
            .envio_pty
            .0
             .send(Msg::Resize(window_size_de(dim)))
        {
            tracing::warn!("no se pudo redimensionar el PTY: {e}");
        }
    }

    /// Snapshot inmutable de la rejilla actual, listo para pintar (`vista::lanzador` lo consume en
    /// cada `cx.notify()` disparado por `EventoPty::Refrescar`). Bloquea el `FairMutex` sólo durante
    /// la copia — el hilo del event loop nunca lo mantiene tomado más que para escribir bytes.
    pub fn contenido(&self) -> ContenidoPty {
        let term = self.termino.lock();
        let content = term.renderable_content();

        let mut celdas = Vec::with_capacity(content.display_iter.size_hint().0);
        celdas.extend(content.display_iter.map(|c| CeldaPty {
            fila: c.point.line.0,
            columna: c.point.column.0,
            caracter: c.c,
            fg: color_de(c.fg),
            bg: color_de(c.bg),
            negrita: c.flags.contains(alacritty_terminal::term::cell::Flags::BOLD),
            subrayado: c
                .flags
                .contains(alacritty_terminal::term::cell::Flags::UNDERLINE),
        }));

        ContenidoPty {
            celdas,
            cursor_fila: content.cursor.point.line.0,
            cursor_columna: content.cursor.point.column.0,
            cursor_visible: content.cursor.point.line.0 >= 0,
            modo: content.mode,
        }
    }

    /// Cierra explícitamente el PTY (Ctrl+D no llegó / el usuario cierra la pestaña con proceso
    /// vivo, R7.1). Idempotente: `Notifier::shutdown` sobre un canal ya cerrado no-opera.
    pub fn cerrar(&self) {
        let _ = self.envio_pty.0.send(Msg::Shutdown);
    }
}

/// Una celda ya traducida a un formato plano, sin tipos de `alacritty_terminal` — así
/// `vista::lanzador`/el render de GPUI no necesitan importar el crate del backend.
#[derive(Debug, Clone, Copy)]
pub struct CeldaPty {
    pub fila: i32,
    pub columna: usize,
    pub caracter: char,
    pub fg: ColorPty,
    pub bg: ColorPty,
    pub negrita: bool,
    pub subrayado: bool,
}

/// Snapshot renderizable de la rejilla + posición del cursor.
#[derive(Debug, Clone)]
pub struct ContenidoPty {
    pub celdas: Vec<CeldaPty>,
    pub cursor_fila: i32,
    pub cursor_columna: usize,
    pub cursor_visible: bool,
    /// Modo actual del `Term` (p.ej. `APP_CURSOR`). Lo consume `teclado::a_secuencia_esc` para
    /// decidir la secuencia de flechas/home/end — sin esto el caller no puede saber si el programa
    /// dentro del PTY activó el modo aplicación de cursor.
    pub modo: alacritty_terminal::term::TermMode,
}

/// Color de una celda, ya resuelto al subconjunto que el tema Ethos sabe pintar. v1 no hace mapeo
/// fiel de la paleta de 256 colores/RGB completa (eso es trabajo de theming, no de R7 core): los
/// colores con nombre van a los tokens Ethos más cercanos, RGB/indexado se pasa tal cual y el
/// render decide (`vista::lanzador`) si lo usa directo o cae a texto plano.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorPty {
    /// Color de texto/fondo por defecto del tema (no tocar — hereda de `tema::PAPEL`/`tema::TINTA`).
    PorDefecto,
    /// Uno de los 16 colores con nombre ANSI clásicos (rojo, verde, amarillo…), sin distinguir
    /// "bright" en v1 (se pinta igual que su base — degradación aceptada, no crashea).
    Ansi(u8),
    /// RGB explícito (secuencia truecolor `38;2;r;g;b`).
    Rgb(u8, u8, u8),
}

fn color_de(c: ColorAnsi) -> ColorPty {
    match c {
        ColorAnsi::Named(NamedColor::Foreground) | ColorAnsi::Named(NamedColor::Background) => {
            ColorPty::PorDefecto
        }
        ColorAnsi::Named(n) => ColorPty::Ansi(n as u8),
        ColorAnsi::Spec(rgb) => ColorPty::Rgb(rgb.r, rgb.g, rgb.b),
        ColorAnsi::Indexed(i) => ColorPty::Ansi(i),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensiones_por_defecto_no_son_cero() {
        // Guardrail trivial: si `Term::new`/`tty::new` reciben 0 filas o columnas, alacritty
        // puede entrar en división por cero al calcular el wrap de línea.
        assert!(DimensionesCeldas::POR_DEFECTO.filas > 0);
        assert!(DimensionesCeldas::POR_DEFECTO.columnas > 0);
    }

    #[test]
    fn color_por_defecto_colapsa_fg_y_bg() {
        assert_eq!(
            color_de(ColorAnsi::Named(NamedColor::Foreground)),
            ColorPty::PorDefecto
        );
        assert_eq!(
            color_de(ColorAnsi::Named(NamedColor::Background)),
            ColorPty::PorDefecto
        );
    }

    #[test]
    fn color_rgb_se_preserva() {
        assert_eq!(
            color_de(ColorAnsi::Spec(alacritty_terminal::vte::ansi::Rgb {
                r: 10,
                g: 20,
                b: 30
            })),
            ColorPty::Rgb(10, 20, 30)
        );
    }
}
