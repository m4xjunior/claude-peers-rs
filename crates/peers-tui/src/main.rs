//! peers-tui — panel de control de la red claude-peers.
//!
//! Habla con el broker SOLO por HTTP (cliente reqwest con X-Peers-Token); NUNCA toca Redis.
//! 5 pantallas (Peers, Red/Acceso, Redis, Broker, Config) conmutables con Tab o 1-5, todas
//! con refresco asíncrono según `refresh_ms`.
//!
//! ROBUSTEZ (criterio de diseño): si el broker cae o devuelve 401, la TUI NO crashea —
//! levanta un banner ("broker offline" / "token inválido (401)") y sigue reintentando. No hay
//! `.unwrap()` sobre respuestas de red. La terminal se restaura siempre: en la salida normal
//! por el Drop de `GuardaTerminal`; ante un PANIC, por el panic hook que instala
//! `ratatui::init()` (necesario porque el perfil release usa `panic = "abort"`, y con abort
//! el Drop NO se ejecuta durante el panic). Entre ambos, el shell nunca queda en raw mode.

mod app;
mod cliente;
mod config;
mod ui;

use anyhow::Result;
use app::{App, Input, Pantalla};
use cliente::ClienteAdmin;
use config::Config;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};

/// Guarda RAII de la terminal: al construirse entra en raw mode + pantalla alternativa
/// (vía `ratatui::init`), y al destruirse (Drop) restaura — esto cubre la SALIDA NORMAL.
/// El caso PANIC NO lo cubre este Drop (release usa `panic = "abort"` → sin unwind, el Drop
/// no corre): lo cubre el panic hook que `ratatui::init()` instala, que restaura la terminal
/// antes de abortar. Entre el Drop (salida normal) y el hook (panic), el shell nunca queda roto.
struct GuardaTerminal {
    terminal: ratatui::DefaultTerminal,
}

impl GuardaTerminal {
    fn nueva() -> Self {
        Self { terminal: ratatui::init() }
    }
}

impl Drop for GuardaTerminal {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Carga de config tolerante a fallos: si el TOML está corrupto, arranca con defaults y
    // deja constancia por stderr (no crashea). El archivo ausente ya da default sin error.
    let config = match Config::cargar() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aviso: config inválida ({e:#}); usando valores por defecto");
            Config::default()
        }
    };

    let mut guarda = GuardaTerminal::nueva();
    // Ejecutamos el loop dentro de un wrapper que NO usa `?` antes de restaurar: el Drop de
    // `guarda` restaura la terminal aunque `correr` devuelva Err.
    let resultado = correr(&mut guarda.terminal, config).await;
    drop(guarda); // restaura explícitamente antes de imprimir cualquier error.
    resultado
}

/// Bucle principal: refresca los datos de la pantalla activa cada `refresh_ms`, dibuja, y
/// procesa eventos de teclado con un poll de timeout corto (no bloquea la salida).
async fn correr(terminal: &mut ratatui::DefaultTerminal, config: Config) -> Result<()> {
    let mut cliente = ClienteAdmin::nuevo(config.broker_url.clone(), config.token.clone());
    let mut app = App::nueva(config);

    // Primer refresco inmediato para no mostrar la UI vacía mientras llega el primer tick.
    refrescar(&cliente, &mut app).await;

    let mut ultimo_refresco = Instant::now();
    // Tiempo de vida del flash efímero (mensaje "config guardada", etc.).
    let mut flash_desde: Option<Instant> = None;

    while !app.salir {
        terminal.draw(|f| ui::dibujar(f, &app))?;

        // Caduca el flash tras 2s.
        if let Some(t) = flash_desde {
            if t.elapsed() >= Duration::from_secs(2) {
                app.flash = None;
                flash_desde = None;
            }
        }

        // Poll de eventos con timeout corto: mantiene la UI responsiva sin busy-loop.
        // `event::poll` es síncrono; el timeout breve (120ms) acota el bloqueo del worker.
        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(tecla) = event::read()? {
                // En Windows llegan eventos Press y Release; solo procesamos Press.
                if tecla.kind == KeyEventKind::Press {
                    manejar_tecla(tecla, &cliente, &mut app, &mut flash_desde).await;
                    // Si se editó la config (al guardar), reconstruimos el cliente con la nueva URL/token.
                    if app.pantalla == Pantalla::Config {
                        cliente = ClienteAdmin::nuevo(app.config.broker_url.clone(), app.config.token.clone());
                    }
                }
            }
        }

        // Tick de refresco según refresh_ms (mínimo 100ms para no martillar el broker).
        let periodo = Duration::from_millis(app.config.refresh_ms.max(100));
        if ultimo_refresco.elapsed() >= periodo {
            refrescar(&cliente, &mut app).await;
            ultimo_refresco = Instant::now();
        }
    }
    Ok(())
}

/// Refresca los datos de la pantalla activa. Solo pide lo que esa pantalla necesita (menos
/// tráfico). Cada llamada actualiza el banner de red según éxito/fallo. NUNCA propaga error.
async fn refrescar(cliente: &ClienteAdmin, app: &mut App) {
    match app.pantalla {
        Pantalla::Peers => {
            let r = cliente.listar().await;
            app.marcar_resultado(&r);
            if let Ok(peers) = r {
                app.datos.peers = peers;
                // Acota la selección si la lista encogió.
                if app.seleccion >= app.datos.peers.len() {
                    app.seleccion = app.datos.peers.len().saturating_sub(1);
                }
            }
        }
        Pantalla::Redis => {
            let r = cliente.admin_redis().await;
            app.marcar_resultado(&r);
            if let Ok(redis) = r {
                let n_colas = redis.colas.len();
                app.datos.redis = Some(redis);
                if app.seleccion >= n_colas {
                    app.seleccion = n_colas.saturating_sub(1);
                }
            }
        }
        Pantalla::Broker => {
            // Dos llamadas independientes; el banner refleja la primera que falle.
            let info = cliente.admin_info().await;
            app.marcar_resultado(&info);
            if let Ok(i) = info {
                app.datos.info = Some(i);
            }
            let salud = cliente.salud().await;
            if salud.is_ok() {
                // Solo degradamos el banner; no lo subimos a Ok si info ya falló.
                app.marcar_resultado(&salud);
            }
            if let Ok(s) = salud {
                app.datos.salud = Some(s);
            }
            // Factor aprendido: degrada igual que el resto. Si falla, no subimos el banner a
            // Ok; conservamos el último factor bueno. Si va bien, refrescamos.
            let factor = cliente.factor_estimacion().await;
            if factor.is_ok() {
                app.marcar_resultado(&factor);
            }
            if let Ok(fe) = factor {
                app.datos.factor = Some(fe);
            }
        }
        Pantalla::Trazabilidad => {
            // El foco necesita la lista de peers (para resolver el peer seleccionado por
            // defecto). Pedimos peers primero (barato) y luego el historial del foco.
            let lp = cliente.listar().await;
            if let Ok(peers) = &lp {
                app.datos.peers = peers.clone();
            }
            match app.traza_peer_actual() {
                Some(id) => {
                    let r = cliente.historial(&id).await;
                    app.marcar_resultado(&r);
                    if let Ok(h) = r {
                        let n = h.len();
                        app.datos.historial = h;
                        if app.seleccion >= n {
                            app.seleccion = n.saturating_sub(1);
                        }
                    }
                }
                None => {
                    // Sin peers: el banner refleja el resultado del listar; historial vacío.
                    app.marcar_resultado(&lp);
                    app.datos.historial.clear();
                }
            }
        }
        // Acceso y Config son locales (no piden red); aun así verificamos vida del broker
        // para que el banner siga reflejando el estado real.
        Pantalla::Acceso | Pantalla::Config => {
            let r = cliente.salud().await;
            app.marcar_resultado(&r);
            if let Ok(s) = r {
                app.datos.salud = Some(s);
            }
        }
    }
}

/// Despacha una pulsación de tecla. Si hay un input activo, las teclas alimentan el buffer;
/// si no, son comandos de navegación/acción. `flash_desde` marca cuándo nació un flash efímero.
async fn manejar_tecla(
    tecla: KeyEvent,
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
) {
    if app.input.esta_activo() {
        manejar_input(tecla, cliente, app, flash_desde).await;
        return;
    }

    // Con el timeline de Trazabilidad abierto, Esc/Enter solo lo cierran (no salen de la app);
    // 'r' reenvía el mensaje del timeline; el resto de teclas se ignora hasta cerrarlo.
    if app.pantalla == Pantalla::Trazabilidad && app.traza_timeline {
        match tecla.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.traza_timeline = false,
            KeyCode::Char('r') => reenviar_seleccionado(cliente, app, flash_desde).await,
            _ => {}
        }
        return;
    }

    match tecla.code {
        KeyCode::Char('q') | KeyCode::Esc => app.salir = true,
        KeyCode::Tab => {
            app.pantalla = app.pantalla.siguiente();
            app.seleccion = 0;
        }
        KeyCode::BackTab => {
            app.pantalla = app.pantalla.anterior();
            app.seleccion = 0;
        }
        KeyCode::Char(c @ '1'..='6') => {
            if let Some(p) = Pantalla::desde_tecla(c) {
                app.pantalla = p;
                app.seleccion = 0;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => match app.pantalla {
            Pantalla::Peers => app.seleccion_abajo(app.datos.peers.len()),
            Pantalla::Redis => {
                let n = app.datos.redis.as_ref().map(|r| r.colas.len()).unwrap_or(0);
                app.seleccion_abajo(n);
            }
            Pantalla::Trazabilidad => app.seleccion_abajo(app.datos.historial.len()),
            Pantalla::Config => {
                app.config_campo = (app.config_campo + 1).min(2);
            }
            _ => {}
        },
        KeyCode::Up | KeyCode::Char('k') if app.pantalla != Pantalla::Peers => {
            // 'k' es kick SOLO en Peers; en el resto sube selección/campo.
            match app.pantalla {
                Pantalla::Redis => app.seleccion_arriba(),
                Pantalla::Config => app.config_campo = app.config_campo.saturating_sub(1),
                _ => app.seleccion_arriba(),
            }
        }
        KeyCode::Up => app.seleccion_arriba(),
        _ => manejar_accion(tecla, cliente, app, flash_desde).await,
    }
}

/// Acciones específicas de pantalla (m/k/r en Peers, p en Redis, e/s en Config).
async fn manejar_accion(
    tecla: KeyEvent,
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
) {
    match (app.pantalla, tecla.code) {
        // --- Peers ---
        (Pantalla::Peers, KeyCode::Char('m')) => {
            if let Some(p) = app.peer_seleccionado() {
                let id = p.id.clone();
                app.abrir_input(Input::Mensaje { para_id: id }, String::new());
            }
        }
        (Pantalla::Peers, KeyCode::Char('k')) => {
            if let Some(p) = app.peer_seleccionado() {
                let id = p.id.clone();
                let r = cliente.salir(&id).await;
                app.marcar_resultado(&r);
                if r.is_ok() {
                    poner_flash(app, flash_desde, format!("peer '{id}' expulsado"));
                }
            }
        }
        (Pantalla::Peers, KeyCode::Char('r')) => {
            if let Some(p) = app.peer_seleccionado() {
                let id = p.id.clone();
                let actual = p.resumen.clone();
                app.abrir_input(Input::Resumen { id }, actual);
            }
        }
        // --- Redis ---
        (Pantalla::Redis, KeyCode::Char('p')) => {
            if let Some(id) = app.cola_redis_seleccionada() {
                let r = cliente.purgar(&id).await;
                app.marcar_resultado(&r);
                if r.is_ok() {
                    poner_flash(app, flash_desde, format!("cola de '{id}' purgada"));
                }
            }
        }
        // --- Config ---
        (Pantalla::Config, KeyCode::Char('e')) => {
            let (input, inicial) = match app.config_campo {
                0 => (Input::ConfigUrl, app.config.broker_url.clone()),
                1 => (Input::ConfigToken, app.config.token.clone().unwrap_or_default()),
                _ => (Input::ConfigRefresh, app.config.refresh_ms.to_string()),
            };
            app.abrir_input(input, inicial);
        }
        (Pantalla::Config, KeyCode::Char('s')) => match app.config.guardar() {
            Ok(()) => poner_flash(app, flash_desde, "config guardada".to_string()),
            Err(e) => poner_flash(app, flash_desde, format!("error al guardar: {e}")),
        },
        // --- Trazabilidad ---
        (Pantalla::Trazabilidad, KeyCode::Enter) => {
            // Abre el timeline del mensaje seleccionado (si hay alguno).
            if app.traza_mensaje_seleccionado().is_some() {
                app.traza_timeline = true;
            }
        }
        (Pantalla::Trazabilidad, KeyCode::Char('r')) => {
            reenviar_seleccionado(cliente, app, flash_desde).await;
        }
        _ => {}
    }
}

/// Reenvía el mensaje seleccionado en la pantalla Trazabilidad vía `POST /admin/reenviar`.
/// Maneja los tres desenlaces sin crashear: reenviado (flash con el nuevo id), no-existe
/// (flash con el motivo), o error de red/401 (banner vía `marcar_resultado`).
async fn reenviar_seleccionado(
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
) {
    let Some(msg_id) = app.traza_mensaje_seleccionado().map(|m| m.id) else {
        return;
    };
    let r = cliente.reenviar(msg_id).await;
    app.marcar_resultado(&r);
    match r {
        Ok(resp) if resp.ok => {
            let nuevo = resp.msg_id.map(|n| n.to_string()).unwrap_or_else(|| "?".to_string());
            poner_flash(app, flash_desde, format!("msg #{msg_id} reenviado → #{nuevo}"));
        }
        Ok(resp) => {
            let motivo = resp.error.unwrap_or_else(|| "no se pudo reenviar".to_string());
            poner_flash(app, flash_desde, motivo);
        }
        // Err ya quedó reflejado en el banner por `marcar_resultado`.
        Err(_) => {}
    }
}

/// Teclas mientras un input está abierto: edición del buffer, Enter confirma, Esc cancela.
async fn manejar_input(
    tecla: KeyEvent,
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
) {
    match tecla.code {
        KeyCode::Esc => app.cerrar_input(),
        KeyCode::Enter => confirmar_input(cliente, app, flash_desde).await,
        KeyCode::Backspace => {
            app.buffer.pop();
        }
        KeyCode::Char(c) => {
            // Ignora combinaciones con Ctrl (p.ej. Ctrl-C lo gestiona la terminal).
            if !tecla.modifiers.contains(KeyModifiers::CONTROL) {
                app.buffer.push(c);
            }
        }
        _ => {}
    }
}

/// Aplica el contenido del buffer según qué se estaba editando, y cierra el input.
async fn confirmar_input(cliente: &ClienteAdmin, app: &mut App, flash_desde: &mut Option<Instant>) {
    let input = app.input.clone();
    let valor = app.buffer.clone();
    app.cerrar_input();

    match input {
        Input::Mensaje { para_id } => {
            if !valor.trim().is_empty() {
                let r = cliente.enviar(&para_id, &valor).await;
                app.marcar_resultado(&r);
                if r.is_ok() {
                    poner_flash(app, flash_desde, format!("mensaje enviado a '{para_id}'"));
                }
            }
        }
        Input::Resumen { id } => {
            let r = cliente.definir_resumen(&id, &valor).await;
            app.marcar_resultado(&r);
            if r.is_ok() {
                poner_flash(app, flash_desde, format!("resumen de '{id}' actualizado"));
            }
        }
        Input::ConfigUrl => app.config.broker_url = valor,
        Input::ConfigToken => {
            // Token vacío → None (broker sin token).
            app.config.token = if valor.trim().is_empty() { None } else { Some(valor) };
        }
        Input::ConfigRefresh => {
            // Valor inválido → se ignora silenciosamente (no rompe la config en memoria).
            if let Ok(ms) = valor.trim().parse::<u64>() {
                app.config.refresh_ms = ms.max(100);
            }
        }
        Input::Ninguno => {}
    }
}

/// Establece un mensaje flash efímero y marca su instante de nacimiento.
fn poner_flash(app: &mut App, flash_desde: &mut Option<Instant>, texto: String) {
    app.flash = Some(texto);
    *flash_desde = Some(Instant::now());
}
