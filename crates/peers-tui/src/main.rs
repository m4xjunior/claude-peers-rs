//! peers-tui — panel de control de la red claude-peers.
//!
//! Habla con el broker SOLO por HTTP (cliente reqwest con X-Peers-Token); NUNCA toca Redis.
//! 9 pantallas (Peers, Red/Acceso, Redis, Broker, Config, Trazabilidad, Jornada, Tareas,
//! Alertas) conmutables con Tab o 1-9, todas con refresco asíncrono según `refresh_ms`.
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
use peers_core::EstadoTarea;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use std::io::stdout;
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
        let terminal = ratatui::init();
        // ratatui::init NO habilita la captura de mouse: lo hacemos aquí explícitamente.
        // Si falla (terminal sin soporte), seguimos solo-teclado — NUNCA crasheamos por esto.
        if let Err(e) = execute!(stdout(), EnableMouseCapture) {
            eprintln!("aviso: no se pudo habilitar el mouse ({e}); solo-teclado");
        }
        // Con panic=abort el Drop NO corre, y el panic hook de ratatui::init restaura la
        // pantalla pero NO deshabilita el mouse → la terminal quedaría capturando mouse tras
        // un panic. Encadenamos ANTES del hook actual un DisableMouseCapture para cerrar ese gap.
        let hook_previo = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(stdout(), DisableMouseCapture);
            hook_previo(info);
        }));
        Self { terminal }
    }
}

impl Drop for GuardaTerminal {
    fn drop(&mut self) {
        // Deshabilitamos la captura de mouse ANTES de restaurar (orden inverso al setup).
        // Ignoramos el error: si el stdout ya está roto, igualmente vamos a restaurar.
        let _ = execute!(stdout(), DisableMouseCapture);
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
        // Timbramos el "ahora" UTC una vez por frame: lo usa el orden overrun-primero de la
        // vista global (#14/R3) sin que cada helper puro necesite su propio reloj. Si el formateo
        // fallara (no debería con Rfc3339), conservamos el último valor — nunca crasheamos.
        if let Ok(s) = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339) {
            app.ahora = s;
        }
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
            match event::read()? {
                Event::Key(tecla) => {
                    // En Windows llegan eventos Press y Release; solo procesamos Press.
                    if tecla.kind == KeyEventKind::Press {
                        manejar_tecla(tecla, &cliente, &mut app, &mut flash_desde).await;
                        // Si se editó la config (al guardar), reconstruimos el cliente con la nueva URL/token.
                        if app.pantalla == Pantalla::Config {
                            cliente = ClienteAdmin::nuevo(
                                app.config.broker_url.clone(),
                                app.config.token.clone(),
                            );
                        }
                    }
                }
                Event::Mouse(evento) => {
                    manejar_mouse(evento, &mut app);
                }
                _ => {}
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
        Pantalla::Jornada => {
            // Foco = peer seleccionado en Peers. Pedimos peers primero (barato) para resolverlo.
            let lp = cliente.listar().await;
            if let Ok(peers) = &lp {
                app.datos.peers = peers.clone();
            }
            match app.traza_peer_actual() {
                Some(id) => {
                    let r = cliente.jornada(&id).await;
                    app.marcar_resultado(&r);
                    if let Ok(j) = r {
                        app.datos.jornada = Some(j);
                    }
                    // Bitácora (registro-acciones R12): best-effort SIN tocar el banner — un
                    // broker viejo sin /acciones no es un fallo de la TUI; sección vacía.
                    match cliente.acciones(&id).await {
                        Ok(a) => app.datos.jornada_acciones = a,
                        Err(_) => app.datos.jornada_acciones.clear(),
                    }
                }
                None => {
                    app.marcar_resultado(&lp);
                    app.datos.jornada = None;
                    app.datos.jornada_acciones.clear();
                }
            }
        }
        Pantalla::Tareas => {
            // La lista de peers se necesita en ambas vistas: en vista peer para resolver el foco
            // ([ ]) y en vista global para reasignar (a) sobre la fila seleccionada (R4).
            let lp = cliente.listar().await;
            if let Ok(peers) = &lp {
                app.datos.peers = peers.clone();
            }
            if app.vista_global {
                // VISTA GLOBAL (#14/R1): tareas de TODOS los peers vía GET /admin/tareas.
                // `[`/`]` no aplica aquí; el filtrado/orden se hace en cliente (tareas_ordenadas).
                let r = cliente.admin_tareas().await;
                app.marcar_resultado(&r);
                if let Ok(t) = r {
                    app.datos.tareas = t;
                    acotar_seleccion_tareas(app);
                }
            } else {
                // VISTA PEER (actual): solo las tareas del peer enfocado por `[`/`]`.
                match app.traza_peer_actual() {
                    Some(id) => {
                        let r = cliente.listar_tareas(&id).await;
                        app.marcar_resultado(&r);
                        if let Ok(t) = r {
                            app.datos.tareas = t;
                            acotar_seleccion_tareas(app);
                        }
                    }
                    None => {
                        app.marcar_resultado(&lp);
                        app.datos.tareas.clear();
                    }
                }
            }
        }
        Pantalla::Alertas => {
            let r = cliente.alertas().await;
            app.marcar_resultado(&r);
            if let Ok(a) = r {
                let n = a.len();
                app.datos.alertas = a;
                if app.seleccion >= n {
                    app.seleccion = n.saturating_sub(1);
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
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                app.traza_timeline = false;
                app.modal_scroll = 0;
            }
            KeyCode::PageDown => app.modal_scroll_abajo(),
            KeyCode::PageUp => app.modal_scroll_arriba(),
            KeyCode::Char('r') => reenviar_seleccionado(cliente, app, flash_desde).await,
            _ => {}
        }
        return;
    }

    // Con el modal DETALLE de tarea abierto: Esc/Enter/q lo cierran; las teclas de acción
    // (e/+/f/b/h/c/R) operan sobre la tarea del modal sin cerrarlo (salvo las que cambian de
    // contexto). El resto se ignora — no se navega ni se sale de la app por accidente.
    if app.pantalla == Pantalla::Tareas && app.tarea_detalle.is_some() {
        match tecla.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.cerrar_detalle_tarea(),
            KeyCode::PageDown => app.modal_scroll_abajo(),
            KeyCode::PageUp => app.modal_scroll_arriba(),
            KeyCode::Char('e') | KeyCode::Char('+') | KeyCode::Char('f')
            | KeyCode::Char('b') | KeyCode::Char('h') | KeyCode::Char('c')
            | KeyCode::Char('R') => {
                accion_tarea(tecla, cliente, app, flash_desde).await;
            }
            _ => {}
        }
        return;
    }

    // Con el modal DETALLE de alerta abierto: Esc/Enter/q cierran; `d` descarta la alerta (la
    // resuelve en el broker) y cierra; `g` salta al sujeto en su pantalla. El resto se ignora.
    if app.pantalla == Pantalla::Alertas && app.alerta_detalle.is_some() {
        match tecla.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                app.alerta_detalle = None;
                app.modal_scroll = 0;
            }
            KeyCode::PageDown => app.modal_scroll_abajo(),
            KeyCode::PageUp => app.modal_scroll_arriba(),
            KeyCode::Char('d') => accion_descartar_alerta(cliente, app, flash_desde).await,
            KeyCode::Char('g') => saltar_al_sujeto_alerta(app),
            _ => {}
        }
        return;
    }

    // VISTA GLOBAL de Tareas (#14): la tecla `g` la alterna en cualquier momento dentro de la
    // pantalla Tareas. Resetea la selección porque la lista mostrada cambia por completo.
    if app.pantalla == Pantalla::Tareas && tecla.code == KeyCode::Char('g') {
        app.vista_global = !app.vista_global;
        app.seleccion = 0;
        return;
    }
    // En pantalla Tareas CON vista global activa, las teclas `1`-`5`/`0` se REINTERPRETAN como
    // FILTRO por estado (R3), NO como cambio de pantalla. Para cambiar de pantalla aquí se usa Tab
    // (documentado en la ayuda). Fuera de esta condición, `1`-`9` siguen cambiando de pantalla.
    if app.pantalla == Pantalla::Tareas && app.vista_global {
        if let KeyCode::Char(c @ ('0'..='5')) = tecla.code {
            app.filtro_estado = App::filtro_desde_tecla(c); // '0' o no-1..5 → None (todas)
            acotar_seleccion_tareas(app);
            return;
        }
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
        KeyCode::Char(c @ '1'..='9') => {
            if let Some(p) = Pantalla::desde_tecla(c) {
                app.pantalla = p;
                app.seleccion = 0;
            }
        }
        // [ / ] ciclan el PEER enfocado en Jornada/Trazabilidad/Tareas (ver la jornada/
        // trazabilidad/tareas de CUALQUIER peer, no solo el seleccionado en Peers).
        KeyCode::Char('[') | KeyCode::Char(']')
            if matches!(
                app.pantalla,
                Pantalla::Jornada | Pantalla::Trazabilidad | Pantalla::Tareas
            ) =>
        {
            let dir = if tecla.code == KeyCode::Char(']') { 1 } else { -1 };
            app.ciclar_peer_foco(dir);
        }
        KeyCode::Down | KeyCode::Char('j') => match app.pantalla {
            Pantalla::Peers => app.seleccion_abajo(app.datos.peers.len()),
            Pantalla::Redis => {
                let n = app.datos.redis.as_ref().map(|r| r.colas.len()).unwrap_or(0);
                app.seleccion_abajo(n);
            }
            Pantalla::Trazabilidad => app.seleccion_abajo(app.datos.historial.len()),
            Pantalla::Tareas => app.seleccion_abajo(app.datos.tareas.len()),
            Pantalla::Alertas => app.seleccion_abajo(app.datos.alertas.len()),
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

/// Despacha un evento de MOUSE. Navegación complementaria al teclado (que sigue 100% activo).
///
/// Mapeo de clicks (no rompe la robustez: offline/401/RAII intactos — esto solo muta estado local):
///   (a) Click izquierdo en la barra de pestañas (y < 3) → cambia de pantalla según la columna x,
///       reproduciendo el layout del widget Tabs (ver `Pantalla::pantalla_en_x_tabs`). Resetea
///       la selección, igual que Tab / teclas 1-6.
///   (b) Click izquierdo en una fila del cuerpo (tabla) → selecciona esa fila (índice por la y
///       relativa al área del cuerpo, descontando borde + encabezado). Solo en pantallas con
///       tabla seleccionable (Peers / Redis / Trazabilidad); clicks en zona muerta se ignoran.
///   (c) ScrollUp/ScrollDown en cualquier parte → mueve la selección como ↑/↓.
///
/// Con un input modal abierto, ignoramos el mouse: el modal se cierra con teclado (Esc/Enter),
/// para no crear estados ambiguos. El cuerpo siempre arranca en y=4 (3 filas de tabs + 1 banner).
fn manejar_mouse(evento: MouseEvent, app: &mut App) {
    // Con un input/modal abierto, el mouse no actúa (la edición es solo-teclado).
    if app.input.esta_activo() || app.traza_timeline || app.tarea_detalle.is_some() {
        return;
    }

    // Coordenada absoluta del borde superior del cuerpo (tras tabs[3] + banner[1]).
    const CUERPO_Y: u16 = 4;
    // Altura de la barra de pestañas (Constraint::Length(3) → filas y=0,1,2).
    const TABS_ALTO: u16 = 3;

    match evento.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if evento.row < TABS_ALTO {
                // (a) Click sobre la barra de pestañas.
                if let Some(p) = Pantalla::pantalla_en_x_tabs(evento.column) {
                    app.pantalla = p;
                    app.seleccion = 0;
                }
            } else {
                // (b) Click sobre el cuerpo: seleccionar fila si la pantalla tiene tabla.
                let total = filas_pantalla(app);
                if total > 0 {
                    if let Some(idx) = app::fila_en_y_cuerpo(evento.row, CUERPO_Y, total) {
                        app.seleccion = idx;
                    }
                }
            }
        }
        // (c) Scroll → mover selección como flechas, acotado al total de la pantalla actual.
        MouseEventKind::ScrollDown => {
            let total = filas_pantalla(app);
            app.seleccion_abajo(total);
        }
        MouseEventKind::ScrollUp => {
            app.seleccion_arriba();
        }
        _ => {}
    }
}

/// Número de filas seleccionables de la pantalla activa (las que tienen tabla navegable).
/// Centraliza la cuenta que el teclado hacía inline, para reusarla en el mouse. En Tareas usa la
/// lista VISIBLE (filtrada en vista global), que es la que realmente se pinta y se puede seleccionar.
fn filas_pantalla(app: &App) -> usize {
    match app.pantalla {
        Pantalla::Peers => app.datos.peers.len(),
        Pantalla::Redis => app.datos.redis.as_ref().map(|r| r.colas.len()).unwrap_or(0),
        Pantalla::Trazabilidad => app.datos.historial.len(),
        Pantalla::Tareas => app.tareas_visibles().len(),
        Pantalla::Alertas => app.datos.alertas.len(),
        _ => 0,
    }
}

/// Acota la selección de la pantalla Tareas al número de filas VISIBLES (tras filtro en vista
/// global). Evita que la fila seleccionada quede fuera de rango al cambiar de vista/filtro o al
/// encoger la lista tras un refresco.
fn acotar_seleccion_tareas(app: &mut App) {
    let n = app.tareas_visibles().len();
    if app.seleccion >= n {
        app.seleccion = n.saturating_sub(1);
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
                app.modal_scroll = 0;
            }
        }
        (Pantalla::Trazabilidad, KeyCode::Char('r')) => {
            reenviar_seleccionado(cliente, app, flash_desde).await;
        }
        // --- Alertas ---
        (Pantalla::Alertas, KeyCode::Enter) => {
            // Abre el modal DETALLE de la alerta seleccionada (detalle completo, sin recorte).
            if let Some(a) = app.datos.alertas.get(app.seleccion) {
                app.alerta_detalle = Some(a.clone());
                app.modal_scroll = 0;
            }
        }
        (Pantalla::Alertas, KeyCode::Char('d')) => {
            // Descarta la alerta seleccionada (sin abrir el modal): resuelve en el broker.
            accion_descartar_alerta(cliente, app, flash_desde).await;
        }
        (Pantalla::Alertas, KeyCode::Char('g')) => {
            // Salta al sujeto de la alerta seleccionada en su pantalla (Peers/Tareas/Trazabilidad).
            if app.datos.alertas.get(app.seleccion).is_some() {
                app.alerta_detalle = app.datos.alertas.get(app.seleccion).cloned();
                saltar_al_sujeto_alerta(app);
            }
        }
        // --- Tareas ---
        (Pantalla::Tareas, KeyCode::Enter) => {
            // Abre el modal DETALLE de la tarea seleccionada y carga sus reportes (AC5).
            abrir_detalle_tarea(cliente, app).await;
        }
        (Pantalla::Tareas, KeyCode::Char('n')) => {
            // Nueva tarea asignada al PEER ENFOCADO (el de [ ]); arranca el input de descripción.
            if let Some(id) = app.traza_peer_actual() {
                app.abrir_input(Input::TareaNuevaDescripcion { instancia_id: id }, String::new());
            }
        }
        (Pantalla::Tareas, KeyCode::Char('a')) => {
            // Reasigna la tarea seleccionada al siguiente peer (cicla el destino con cada 'a').
            reasignar_tarea(cliente, app, flash_desde).await;
        }
        (
            Pantalla::Tareas,
            KeyCode::Char('e') | KeyCode::Char('+') | KeyCode::Char('f') | KeyCode::Char('b')
            | KeyCode::Char('h') | KeyCode::Char('c') | KeyCode::Char('R'),
        ) => {
            accion_tarea(tecla, cliente, app, flash_desde).await;
        }
        _ => {}
    }
}

/// Abre el modal DETALLE de la tarea seleccionada y carga sus reportes desde el broker.
/// Si la carga de reportes falla (offline/401), el modal igual se abre con la lista vacía y el
/// banner refleja el error — degradación graciosa (R13/AC7).
async fn abrir_detalle_tarea(cliente: &ClienteAdmin, app: &mut App) {
    let Some(tarea) = app.tarea_seleccionada().cloned() else {
        return;
    };
    let r = cliente.tarea_reportes(&tarea.id).await;
    app.marcar_resultado(&r);
    app.tarea_reportes = r.unwrap_or_default();
    app.tarea_detalle = Some(tarea);
    app.modal_scroll = 0;
}

/// Tipo de alerta → string del wire (lo que el broker guarda y `alerta_resolver` espera).
/// Reusa la serialización serde (lowercase + renames) para no duplicar el mapeo.
fn tipo_alerta_str(tipo: peers_core::TipoAlerta) -> String {
    serde_json::to_value(tipo)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Descarta (resuelve a mano) la alerta OBJETIVO: la del modal si está abierto, o la
/// seleccionada en la tabla. Llama al broker (`/admin/alerta-resolver`) y cierra el modal.
/// Degrada offline/401 vía `marcar_resultado` (no crashea, muestra flash).
async fn accion_descartar_alerta(
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
) {
    let objetivo = app
        .alerta_detalle
        .clone()
        .or_else(|| app.datos.alertas.get(app.seleccion).cloned());
    let Some(a) = objetivo else {
        return;
    };
    let r = cliente.alerta_resolver(&tipo_alerta_str(a.tipo), &a.sujeto).await;
    app.marcar_resultado(&r);
    match r {
        Ok(_) => poner_flash(app, flash_desde, format!("alerta '{}' descartada", a.sujeto)),
        Err(e) => poner_flash(app, flash_desde, format!("no se pudo descartar: {e}")),
    }
    app.alerta_detalle = None;
}

/// Salta al SUJETO de la alerta del modal en su pantalla natural para actuar ahí:
///   - Ocioso / CancelacionExcesiva → Peers (sujeto = id de peer).
///   - Atascado / CierreSospechoso  → Tareas (sujeto = id de tarea).
///   - Ghosteo                       → Trazabilidad (sujeto = "msg:<id>").
/// Cierra el modal y deja al jefe en la pantalla del sujeto. No selecciona la fila exacta
/// (las listas se refrescan async); cambiar de pantalla ya es el atajo útil pedido.
fn saltar_al_sujeto_alerta(app: &mut App) {
    use peers_core::TipoAlerta::*;
    if let Some(a) = app.alerta_detalle.take() {
        app.pantalla = match a.tipo {
            Ocioso | CancelacionExcesiva => Pantalla::Peers,
            Atascado | CierreSospechoso => Pantalla::Tareas,
            Ghosteo => Pantalla::Trazabilidad,
        };
        app.seleccion = 0;
    }
}

/// Despacha las acciones que operan sobre la TAREA OBJETIVO (la del modal si está abierto, o la
/// seleccionada en la tabla si no). Centraliza editar/ampliar/forzar/estado para que funcionen
/// igual desde la tabla y desde el modal. Todas degradan offline/401 vía `marcar_resultado`.
async fn accion_tarea(
    tecla: KeyEvent,
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
) {
    // La tarea objetivo: la del modal (copia) tiene prioridad; si no, la fila seleccionada.
    let Some(tarea) = app
        .tarea_detalle
        .clone()
        .or_else(|| app.tarea_seleccionada().cloned())
    else {
        return;
    };

    match tecla.code {
        KeyCode::Char('e') => {
            // Editar: paso 1/2 = descripción (precargada con la actual).
            app.abrir_input(
                Input::TareaEditarDescripcion { tarea_id: tarea.id.clone() },
                tarea.descripcion.clone(),
            );
        }
        KeyCode::Char('+') => {
            // Ampliar estimado: input de minutos a SUMAR; el handler lee el estimado vigente.
            app.abrir_input(
                Input::TareaAmpliarEstimado {
                    tarea_id: tarea.id.clone(),
                    estimado_actual_seg: tarea.estimado_seg.unwrap_or(0),
                },
                String::new(),
            );
        }
        KeyCode::Char('f') => {
            // Forzar: "tócale el hombro". ok=false si el peer no está vivo (no es error).
            let r = cliente.tarea_forzar(&tarea.id).await;
            app.marcar_resultado(&r);
            match r {
                Ok(resp) if resp.ok => {
                    poner_flash(app, flash_desde, format!("recordatorio enviado a '{}'", tarea.instancia_id));
                }
                Ok(_) => {
                    poner_flash(app, flash_desde, format!("'{}' no está vivo: no se pudo forzar", tarea.instancia_id));
                }
                Err(_) => {}
            }
        }
        KeyCode::Char('b') => {
            // Bloquear: pide el motivo (input). El cambio de estado se hace al confirmar.
            app.abrir_input(Input::TareaBloquear { tarea_id: tarea.id.clone() }, String::new());
        }
        KeyCode::Char('h') => {
            cambiar_estado_tarea(cliente, app, flash_desde, &tarea.id, EstadoTarea::Hecha, None).await;
        }
        KeyCode::Char('c') => {
            cambiar_estado_tarea(cliente, app, flash_desde, &tarea.id, EstadoTarea::Cancelada, None).await;
        }
        KeyCode::Char('R') => {
            // Reabrir = volver a Abierta (el broker valida la transición).
            cambiar_estado_tarea(cliente, app, flash_desde, &tarea.id, EstadoTarea::Abierta, None).await;
        }
        _ => {}
    }
}

/// Aplica una transición de estado vía `POST /tarea/estado` y refleja el resultado. Si el modal
/// detalle está abierto sobre esta tarea, lo actualiza con la `Tarea` devuelta (estado/motivo
/// frescos) para que el cambio se vea al instante sin esperar al refresco. AC2.
async fn cambiar_estado_tarea(
    cliente: &ClienteAdmin,
    app: &mut App,
    flash_desde: &mut Option<Instant>,
    tarea_id: &str,
    estado: EstadoTarea,
    motivo: Option<String>,
) {
    let r = cliente.tarea_estado(tarea_id, estado, motivo).await;
    app.marcar_resultado(&r);
    if let Ok(tarea) = r {
        let etiqueta = app::etiqueta_estado_tarea(estado);
        poner_flash(app, flash_desde, format!("tarea → {etiqueta}"));
        // Si el modal mira esta tarea, refréscalo con el estado nuevo.
        if app.tarea_detalle.as_ref().map(|t| t.id == tarea.id).unwrap_or(false) {
            app.tarea_detalle = Some(tarea);
        }
    }
}

/// Reasigna la tarea seleccionada al SIGUIENTE peer de la lista (cicla con cada 'a'). El destino
/// arranca tras el dueño actual y se mantiene en `reasignar_destino`. Notifica al nuevo dueño
/// (lo hace el broker). Si no hay otro peer al que reasignar, no hace nada.
async fn reasignar_tarea(cliente: &ClienteAdmin, app: &mut App, flash_desde: &mut Option<Instant>) {
    let Some(tarea) = app.tarea_seleccionada().cloned() else {
        return;
    };
    let peers = &app.datos.peers;
    if peers.is_empty() {
        return;
    }
    // Cicla el destino y evita reasignar al mismo dueño actual (salta una posición si coincide).
    app.reasignar_destino = (app.reasignar_destino + 1) % peers.len();
    let mut idx = app.reasignar_destino;
    if peers[idx].id == tarea.instancia_id && peers.len() > 1 {
        idx = (idx + 1) % peers.len();
        app.reasignar_destino = idx;
    }
    let nuevo = peers[idx].id.clone();
    if nuevo == tarea.instancia_id {
        return; // único peer y es el dueño: nada que hacer.
    }
    let r = cliente.tarea_reasignar(&tarea.id, &nuevo).await;
    app.marcar_resultado(&r);
    if r.is_ok() {
        poner_flash(app, flash_desde, format!("tarea reasignada a '{nuevo}'"));
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
        // --- Tareas: editar descripción (paso 1/2) → encadena el estimado (paso 2/2) ---
        Input::TareaEditarDescripcion { tarea_id } => {
            let desc = valor.trim().to_string();
            // Encadena el segundo paso: el estimado. Si la descripción quedó vacía, igual la
            // mandamos como `None` más abajo (no se toca). El estimado se precarga vacío.
            app.abrir_input(
                Input::TareaEditarEstimado { tarea_id, descripcion: desc },
                String::new(),
            );
        }
        // --- Tareas: editar estimado (paso 2/2) → manda /tarea/editar con ambos campos ---
        Input::TareaEditarEstimado { tarea_id, descripcion } => {
            let descripcion = if descripcion.is_empty() { None } else { Some(descripcion) };
            let estimado_seg = minutos_a_segundos(&valor);
            // Si ambos son None, no hay nada que editar.
            if descripcion.is_some() || estimado_seg.is_some() {
                let r = cliente.tarea_editar(&tarea_id, descripcion, estimado_seg).await;
                app.marcar_resultado(&r);
                if let Ok(t) = r {
                    sincronizar_detalle(app, t);
                    poner_flash(app, flash_desde, "tarea editada".to_string());
                }
            }
        }
        // --- Tareas: ampliar estimado (suma minutos al estimado vigente) ---
        Input::TareaAmpliarEstimado { tarea_id, estimado_actual_seg } => {
            if let Some(extra) = minutos_a_segundos(&valor) {
                let nuevo = estimado_actual_seg.saturating_add(extra);
                let r = cliente.tarea_editar(&tarea_id, None, Some(nuevo)).await;
                app.marcar_resultado(&r);
                if let Ok(t) = r {
                    sincronizar_detalle(app, t);
                    poner_flash(app, flash_desde, "estimado ampliado".to_string());
                }
            }
        }
        // --- Tareas: bloquear con motivo ---
        Input::TareaBloquear { tarea_id } => {
            let motivo = if valor.trim().is_empty() { None } else { Some(valor.trim().to_string()) };
            cambiar_estado_tarea(cliente, app, flash_desde, &tarea_id, EstadoTarea::Bloqueada, motivo).await;
        }
        // --- Tareas: nueva (paso 1/2) → encadena el estimado (paso 2/2) ---
        Input::TareaNuevaDescripcion { instancia_id } => {
            let desc = valor.trim().to_string();
            if !desc.is_empty() {
                app.abrir_input(
                    Input::TareaNuevaEstimado { instancia_id, descripcion: desc },
                    String::new(),
                );
            }
        }
        // --- Tareas: nueva (paso 2/2) → manda /tarea/asignar ---
        Input::TareaNuevaEstimado { instancia_id, descripcion } => {
            let estimado_seg = minutos_a_segundos(&valor);
            let r = cliente.tarea_asignar(&instancia_id, &descripcion, estimado_seg).await;
            app.marcar_resultado(&r);
            if let Ok(resp) = r {
                let id = resp.tarea_id.unwrap_or_default();
                let sufijo = if id.is_empty() { String::new() } else { format!(" (#{id})") };
                let estado = if resp.ok { "asignada" } else { "creada" };
                poner_flash(app, flash_desde, format!("tarea {estado} a '{instancia_id}'{sufijo}"));
            }
        }
        Input::Ninguno => {}
    }
}

/// Parsea minutos (texto del input) a segundos. Vacío o inválido → `None` (no toca el campo).
/// Acepta enteros positivos; negativos/no-numéricos se descartan en silencio (sin crash).
fn minutos_a_segundos(valor: &str) -> Option<i64> {
    valor.trim().parse::<i64>().ok().filter(|m| *m >= 0).map(|m| m * 60)
}

/// Si el modal DETALLE está abierto sobre la tarea `t`, lo actualiza con la versión fresca
/// (descripción/estimado recién editados) para que el cambio se vea al instante.
fn sincronizar_detalle(app: &mut App, t: peers_core::Tarea) {
    if app.tarea_detalle.as_ref().map(|d| d.id == t.id).unwrap_or(false) {
        app.tarea_detalle = Some(t);
    }
}

/// Establece un mensaje flash efímero y marca su instante de nacimiento.
fn poner_flash(app: &mut App, flash_desde: &mut Option<Instant>, texto: String) {
    app.flash = Some(texto);
    *flash_desde = Some(Instant::now());
}
