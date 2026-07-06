//! Estado de la aplicación TUI: pantalla activa, datos refrescados del broker, modo de input
//! y banner de estado de red. Aquí vive la lógica PURA (formato de filas, recorte de columnas,
//! navegación de selección) — testeable sin terminal ni red.

use crate::cliente::ErrorBroker;
use peers_core::{
    AccionRegistrada, Alerta, EstadoMensaje, EstadoTarea, FactorEstimacion, Instancia, Mensaje,
    RespuestaAdminInfo, RespuestaAdminRedis, RespuestaJornada, RespuestaSalud, Tarea, TipoAlerta,
};

/// Las 9 pantallas del panel, conmutables con Tab o las teclas 1-9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pantalla {
    Peers,
    Acceso,
    Redis,
    Broker,
    Config,
    Trazabilidad,
    Jornada,
    Tareas,
    Alertas,
}

impl Pantalla {
    /// Orden de ciclo para Tab (y Shift-Tab al revés).
    pub const TODAS: [Pantalla; 9] = [
        Pantalla::Peers,
        Pantalla::Acceso,
        Pantalla::Redis,
        Pantalla::Broker,
        Pantalla::Config,
        Pantalla::Trazabilidad,
        Pantalla::Jornada,
        Pantalla::Tareas,
        Pantalla::Alertas,
    ];

    pub fn titulo(self) -> &'static str {
        match self {
            Pantalla::Peers => "1 Peers",
            Pantalla::Acceso => "2 Red/Acceso",
            Pantalla::Redis => "3 Redis",
            Pantalla::Broker => "4 Broker",
            Pantalla::Config => "5 Config",
            Pantalla::Trazabilidad => "6 Trazabilidad",
            Pantalla::Jornada => "7 Jornada",
            Pantalla::Tareas => "8 Tareas",
            Pantalla::Alertas => "9 Alertas",
        }
    }

    /// Índice en `TODAS` (para el widget Tabs).
    pub fn indice(self) -> usize {
        Pantalla::TODAS.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// Siguiente pantalla en el ciclo (Tab).
    pub fn siguiente(self) -> Pantalla {
        let i = (self.indice() + 1) % Pantalla::TODAS.len();
        Pantalla::TODAS[i]
    }

    /// Pantalla anterior en el ciclo (Shift-Tab).
    pub fn anterior(self) -> Pantalla {
        let n = Pantalla::TODAS.len();
        let i = (self.indice() + n - 1) % n;
        Pantalla::TODAS[i]
    }

    /// Mapeo de las teclas numéricas 1-9 a pantalla.
    pub fn desde_tecla(c: char) -> Option<Pantalla> {
        match c {
            '1' => Some(Pantalla::Peers),
            '2' => Some(Pantalla::Acceso),
            '3' => Some(Pantalla::Redis),
            '4' => Some(Pantalla::Broker),
            '5' => Some(Pantalla::Config),
            '6' => Some(Pantalla::Trazabilidad),
            '7' => Some(Pantalla::Jornada),
            '8' => Some(Pantalla::Tareas),
            '9' => Some(Pantalla::Alertas),
            _ => None,
        }
    }

    /// Mapea una coordenada `x` (columna del terminal) dentro de la barra de pestañas a la
    /// pantalla clicada. Reproduce el layout del widget `Tabs` de ratatui: borde izquierdo
    /// (1 col), luego cada título pintado como `" titulo "` y separado del siguiente por el
    /// divisor `│` (1 col). El primer tab NO lleva espacio inicial extra de divisor.
    ///
    /// Devuelve `None` si el click cae sobre el borde o fuera de cualquier título — así un
    /// click en zona muerta no cambia de pantalla (robustez: nunca navega "por accidente").
    ///
    /// NOTA: este cálculo es PURO y debe mantenerse en sync con `ui::dibujar_tabs`. Si el
    /// `.divider()` o el padding del widget cambian, ajustar aquí. Testeado abajo.
    pub fn pantalla_en_x_tabs(x: u16) -> Option<Pantalla> {
        // ratatui::Tabs pinta cada título con un espacio a cada lado: " titulo ".
        // Entre títulos hay un divisor de 1 columna. El bloque tiene borde de 1 col a la izq.
        // Layout por tab i (0-indexado), arrancando en col 1 (tras el borde):
        //   tab0: [1 .. 1+w0)
        //   div:  [1+w0]                      (1 col)
        //   tab1: [1+w0+1 .. 1+w0+1+w1)
        //   ...
        // donde wi = len(titulo_i) + 2 (los dos espacios de padding).
        let mut col: u16 = 1; // tras el borde izquierdo del bloque
        for p in Pantalla::TODAS {
            let ancho = p.titulo().chars().count() as u16 + 2; // " titulo "
            if x >= col && x < col + ancho {
                return Some(p);
            }
            col += ancho + 1; // +1 por el divisor "│"
        }
        None
    }
}

/// Qué está editando el usuario cuando hay un input abierto. La pantalla Config reusa el
/// mismo modal de input para sus tres campos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Sin input abierto: las teclas son comandos.
    Ninguno,
    /// Escribiendo un mensaje para el peer seleccionado (acción 'm' en Peers).
    Mensaje { para_id: String },
    /// Editando el resumen del peer seleccionado (acción 'r' en Peers).
    Resumen { id: String },
    /// Editando el broker_url en Config (tecla 'e' sobre el campo url).
    ConfigUrl,
    /// Editando el token en Config.
    ConfigToken,
    /// Editando el refresh_ms en Config.
    ConfigRefresh,
    /// Editando la descripción de la tarea seleccionada (acción 'e' en Tareas, paso 1/2).
    /// Tras confirmar la descripción se encadena `TareaEditarEstimado` para el estimado.
    TareaEditarDescripcion { tarea_id: String },
    /// Editando el estimado (en minutos) de la tarea seleccionada (acción 'e', paso 2/2).
    /// `descripcion` ya capturada en el paso anterior; se manda todo junto en `/tarea/editar`.
    TareaEditarEstimado { tarea_id: String, descripcion: String },
    /// Ampliando el estimado de la tarea: minutos a SUMAR al estimado actual (acción '+').
    /// El handler lee el estimado vigente y suma; reusa `/tarea/editar`.
    TareaAmpliarEstimado { tarea_id: String, estimado_actual_seg: i64 },
    /// Capturando el motivo del bloqueo al pasar a Bloqueada (acción 'b').
    TareaBloquear { tarea_id: String },
    /// Nueva tarea asignada al peer enfocado: descripción (acción 'n', paso 1/2).
    TareaNuevaDescripcion { instancia_id: String },
    /// Nueva tarea: estimado en minutos (acción 'n', paso 2/2). `instancia_id`/`descripcion`
    /// ya capturados; se manda todo junto en `/tarea/asignar`.
    TareaNuevaEstimado { instancia_id: String, descripcion: String },
}

impl Input {
    pub fn esta_activo(&self) -> bool {
        !matches!(self, Input::Ninguno)
    }

    /// Etiqueta del modal de input, según qué se está editando.
    pub fn etiqueta(&self) -> String {
        match self {
            Input::Ninguno => String::new(),
            Input::Mensaje { para_id } => format!("Mensaje para '{para_id}'"),
            Input::Resumen { id } => format!("Nuevo resumen de '{id}'"),
            Input::ConfigUrl => "broker_url".to_string(),
            Input::ConfigToken => "token".to_string(),
            Input::ConfigRefresh => "refresh_ms".to_string(),
            Input::TareaEditarDescripcion { .. } => "Editar descripción".to_string(),
            Input::TareaEditarEstimado { .. } => "Editar estimado (minutos)".to_string(),
            Input::TareaAmpliarEstimado { .. } => "Ampliar estimado (+minutos)".to_string(),
            Input::TareaBloquear { .. } => "Motivo del bloqueo".to_string(),
            Input::TareaNuevaDescripcion { instancia_id } => {
                format!("Nueva tarea para '{instancia_id}' (descripción)")
            }
            Input::TareaNuevaEstimado { .. } => "Estimado de la tarea (minutos)".to_string(),
        }
    }
}

/// Estado de la conexión con el broker, para pintar el banner superior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstadoRed {
    /// Aún no se hizo la primera petición.
    Desconocido,
    /// Última petición OK.
    Ok,
    /// Banner de error persistente (offline / 401 / otro). La TUI sigue reintentando.
    Error(String),
}

/// Datos vivos cacheados de la última respuesta exitosa de cada pantalla. Si una petición
/// falla, se conserva el último dato bueno y se levanta el banner — NO se borra la tabla.
#[derive(Debug, Default, Clone)]
pub struct Datos {
    pub peers: Vec<Instancia>,
    pub redis: Option<RespuestaAdminRedis>,
    pub info: Option<RespuestaAdminInfo>,
    pub salud: Option<RespuestaSalud>,
    /// Factor de corrección de estimación aprendido por el broker (GET /factor-estimacion).
    /// Mostrado en la pantalla Broker. `None` hasta la primera respuesta exitosa.
    pub factor: Option<FactorEstimacion>,
    /// Historial durable del peer enfocado en la pantalla Trazabilidad (orden cronológico
    /// ascendente, tal como lo devuelve `GET /admin/historial`). Vacío si no hay foco o si
    /// el peer no tiene mensajes.
    pub historial: Vec<Mensaje>,
    /// Jornada (sesiones + tareas) del peer enfocado en la pantalla Jornada (POST /jornada).
    /// `None` hasta la primera respuesta exitosa. Es el "fichaje" visible del peer.
    pub jornada: Option<RespuestaJornada>,
    /// Bitácora de acciones del peer enfocado (GET /acciones, registro-acciones R12): las
    /// últimas N, más reciente primero. Vacía si el broker no expone la bitácora (compat).
    pub jornada_acciones: Vec<AccionRegistrada>,
    /// Tareas del peer enfocado en la pantalla Tareas (POST /listar-tareas). Estimado vs real.
    /// Vacío si no hay foco o si el peer no abrió tareas.
    pub tareas: Vec<Tarea>,
    /// Alertas vigentes del supervisor (GET /admin/alertas), orden tal cual lo devuelve el
    /// broker. Solo lectura — la pantalla Alertas las pinta coloreadas por tipo.
    pub alertas: Vec<Alerta>,
}

/// Estado completo de la aplicación.
pub struct App {
    pub pantalla: Pantalla,
    pub datos: Datos,
    pub red: EstadoRed,
    pub input: Input,
    /// Buffer del texto que se está editando (válido solo si `input.esta_activo()`).
    pub buffer: String,
    /// Fila seleccionada en la pantalla actual (Peers / Redis).
    pub seleccion: usize,
    /// Bandera de salida: el loop principal sale cuando es true.
    pub salir: bool,
    /// Config en memoria (editable desde la pantalla Config). El cliente se reconstruye al guardar.
    pub config: crate::config::Config,
    /// Campo activo en la pantalla Config (0=url, 1=token, 2=refresh_ms).
    pub config_campo: usize,
    /// Mensaje efímero de estado (ej. "mensaje enviado", "config guardada").
    pub flash: Option<String>,
    /// Si está abierto el timeline detallado de un mensaje (modal por Enter).
    pub traza_timeline: bool,
    /// Índice del peer ENFOCADO en las pantallas Jornada/Trazabilidad/Tareas (por las que se
    /// cicla con `[`/`]`). Independiente de `seleccion` (que ahí indexa filas de datos, no peers).
    /// Permite ver la jornada/trazabilidad/tareas de CUALQUIER peer, no solo el de Peers.
    pub peer_foco: usize,
    /// Tarea cuyo modal DETALLE está abierto (acción Enter en Tareas). `None` = sin modal.
    /// Se guarda una copia (no un índice) para que el modal sobreviva a un refresco que
    /// reordene/encoja la lista. Los reportes asociados viven en `tarea_reportes`.
    pub tarea_detalle: Option<Tarea>,
    /// Alerta abierta en el modal DETALLE (Enter en pantalla Alertas). Copia, no índice, para
    /// sobrevivir a un refresco que reordene/encoja la lista (igual criterio que `tarea_detalle`).
    pub alerta_detalle: Option<Alerta>,
    /// Offset de scroll VERTICAL del modal de detalle abierto (tarea/alerta/timeline). Se mueve
    /// con PageDown/PageUp para leer contenido que excede el alto fijo del modal (descripciones,
    /// reportes o mensajes largos). Se resetea a 0 al abrir y al cerrar cualquier modal.
    pub modal_scroll: u16,
    /// Reportes de progreso de la tarea del modal DETALLE (`GET /tarea/reportes`). Se cargan al
    /// abrir el modal; vacío si la tarea no tiene reportes o el broker no respondió.
    pub tarea_reportes: Vec<String>,
    /// Índice del peer destino para REASIGNAR (acción 'a'): se cicla con pulsaciones repetidas
    /// de 'a' sobre la misma tarea. Se aplica al confirmar con Enter. Independiente de `peer_foco`.
    pub reasignar_destino: usize,
    /// VISTA GLOBAL de tareas (#14/R2): si `true`, la pantalla Tareas muestra las tareas de TODOS
    /// los peers (`GET /admin/tareas`, con columna PEER) en vez de las del peer enfocado por `[`/`]`.
    /// Se alterna con la tecla `g`. En vista global, `[`/`]` no aplica y `1`-`5`/`0` filtran por estado.
    pub vista_global: bool,
    /// Filtro de estado activo en la VISTA GLOBAL (#14/R3): `None` = todas. Se fija con `1`-`5`
    /// (Abierta/EnCurso/Bloqueada/Hecha/Cancelada) y se limpia con `0`. Solo aplica en vista global.
    pub filtro_estado: Option<EstadoTarea>,
    /// "Ahora" en RFC3339 (UTC), refrescado por el loop en cada tick. Lo usa el orden
    /// overrun-primero de la vista global para medir el transcurrido SIN reloj propio en cada
    /// helper puro (el reloj se timbra una vez por frame en el loop). Vacío = aún sin timbrar.
    pub ahora: String,
}

impl App {
    pub fn nueva(config: crate::config::Config) -> Self {
        Self {
            pantalla: Pantalla::Peers,
            datos: Datos::default(),
            red: EstadoRed::Desconocido,
            input: Input::Ninguno,
            buffer: String::new(),
            seleccion: 0,
            salir: false,
            config,
            config_campo: 0,
            flash: None,
            traza_timeline: false,
            peer_foco: 0,
            tarea_detalle: None,
            alerta_detalle: None,
            modal_scroll: 0,
            tarea_reportes: Vec::new(),
            reasignar_destino: 0,
            vista_global: false,
            filtro_estado: None,
            ahora: String::new(),
        }
    }

    /// Tarea seleccionada en la pantalla Tareas (por la fila `seleccion`), si la hay. Resuelve
    /// sobre la lista REALMENTE mostrada (`tareas_visibles`): así las acciones de gestión
    /// (e/f/h/c/a…) operan sobre la fila correcta tanto en vista peer como en vista global, sea
    /// cual sea el filtro/orden aplicado (#14/R4/AC2 — cada `Tarea` ya trae su `instancia_id`).
    pub fn tarea_seleccionada(&self) -> Option<&Tarea> {
        self.tareas_ordenadas(&self.ahora).get(self.seleccion).copied()
    }

    /// Lista de tareas que la pantalla Tareas debe PINTAR, en el mismo orden que la tabla:
    /// en vista global, aplica `filtro_estado` y ordena overrun/atascadas primero (R3); en vista
    /// peer, devuelve `datos.tareas` tal cual (orden del broker). Devuelve referencias (sin clonar).
    ///
    /// El orden overrun-primero usa `peers_core::supera_umbral(inicio, ahora, estimado)` para
    /// detectar las tareas vivas que ya pasaron su estimado, evitando depender de un reloj aquí:
    /// el `ahora` lo aporta el llamador (`tareas_ordenadas_global`). Esta variante SIN reloj se
    /// usa cuando solo importa el filtrado (p.ej. contar) y mantiene el orden de origen.
    pub fn tareas_visibles(&self) -> Vec<&Tarea> {
        if !self.vista_global {
            return self.datos.tareas.iter().collect();
        }
        self.datos
            .tareas
            .iter()
            .filter(|t| self.filtro_estado.is_none_or(|e| t.estado == e))
            .collect()
    }

    /// Igual que `tareas_visibles` pero ORDENADA con las overrun/atascadas primero (R3), usando
    /// `ahora_iso` como reloj para medir el transcurrido de las tareas vivas. El orden es estable:
    /// dentro de cada grupo (overrun / no overrun) se conserva el orden de origen (inicio desc del
    /// broker). En vista peer no reordena (devuelve el orden del broker). Función pura — testeada.
    pub fn tareas_ordenadas(&self, ahora_iso: &str) -> Vec<&Tarea> {
        let mut visibles = self.tareas_visibles();
        if self.vista_global {
            // `sort_by_key` de Rust es ESTABLE: las overrun (clave 0) van antes que el resto
            // (clave 1) conservando el orden relativo previo. Sin reloj inventado: lo trae el caller.
            visibles.sort_by_key(|t| u8::from(!Self::tarea_overrun(t, ahora_iso)));
        }
        visibles
    }

    /// ¿La tarea `t` está en OVERRUN/atascada? = está VIVA (Abierta/EnCurso) y ya consumió más
    /// tiempo del estimado desde su `inicio`. Reusa `peers_core::supera_umbral` (parseo robusto de
    /// ISO; degrada a `false` si algo no parsea). Las terminales/bloqueadas nunca son overrun aquí.
    pub fn tarea_overrun(t: &Tarea, ahora_iso: &str) -> bool {
        let viva = matches!(t.estado, EstadoTarea::Abierta | EstadoTarea::EnCurso);
        if !viva {
            return false;
        }
        match t.estimado_seg {
            Some(est) if est > 0 => peers_core::supera_umbral(&t.inicio, ahora_iso, est),
            _ => false,
        }
    }

    /// Mapea las teclas `1`-`5` a un estado para el filtro de la VISTA GLOBAL (R3); cualquier otra
    /// tecla → `None` (que el llamador interpreta como "0 = todas"). Función pura — testeada.
    pub fn filtro_desde_tecla(c: char) -> Option<EstadoTarea> {
        match c {
            '1' => Some(EstadoTarea::Abierta),
            '2' => Some(EstadoTarea::EnCurso),
            '3' => Some(EstadoTarea::Bloqueada),
            '4' => Some(EstadoTarea::Hecha),
            '5' => Some(EstadoTarea::Cancelada),
            _ => None,
        }
    }

    /// Etiqueta del filtro activo para el título de la pantalla Tareas en vista global.
    pub fn etiqueta_filtro(&self) -> &'static str {
        match self.filtro_estado {
            None => "todas",
            Some(e) => etiqueta_estado_tarea(e),
        }
    }

    /// Cierra el modal DETALLE de tarea y limpia sus reportes cacheados.
    pub fn cerrar_detalle_tarea(&mut self) {
        self.tarea_detalle = None;
        self.tarea_reportes.clear();
        self.modal_scroll = 0;
    }

    /// Baja el scroll del modal de detalle abierto (PageDown). Acotado por el llamador según las
    /// líneas visibles; aquí solo incrementa de forma segura (paso de media página = 6 líneas).
    pub fn modal_scroll_abajo(&mut self) {
        self.modal_scroll = self.modal_scroll.saturating_add(6);
    }

    /// Sube el scroll del modal de detalle abierto (PageUp), sin pasar de 0.
    pub fn modal_scroll_arriba(&mut self) {
        self.modal_scroll = self.modal_scroll.saturating_sub(6);
    }

    /// Resuelve qué peer enfoca las pantallas Jornada/Trazabilidad/Tareas: el peer en `peer_foco`
    /// (que se cicla con `[`/`]`), acotado a la lista actual. `None` solo si no hay peers.
    pub fn traza_peer_actual(&self) -> Option<String> {
        if self.datos.peers.is_empty() {
            return None;
        }
        let idx = self.peer_foco.min(self.datos.peers.len() - 1);
        self.datos.peers.get(idx).map(|p| p.id.clone())
    }

    /// Cicla el peer enfocado en Jornada/Trazabilidad/Tareas. `dir`: +1 siguiente, -1 anterior.
    /// Envuelve en los extremos. Resetea la selección de filas (datos del nuevo peer).
    pub fn ciclar_peer_foco(&mut self, dir: i32) {
        let n = self.datos.peers.len();
        if n == 0 {
            return;
        }
        let actual = self.peer_foco.min(n - 1) as i32;
        let nuevo = (actual + dir).rem_euclid(n as i32) as usize;
        self.peer_foco = nuevo;
        self.seleccion = 0; // las filas son del nuevo peer
    }

    /// Mensaje seleccionado en la tabla de Trazabilidad (por la fila `seleccion`).
    pub fn traza_mensaje_seleccionado(&self) -> Option<&Mensaje> {
        self.datos.historial.get(self.seleccion)
    }

    /// Aplica el resultado de la última petición al estado de red (banner).
    pub fn marcar_resultado<T>(&mut self, r: &Result<T, ErrorBroker>) {
        self.red = match r {
            Ok(_) => EstadoRed::Ok,
            Err(e) => EstadoRed::Error(e.to_string()),
        };
    }

    /// Mueve la selección hacia abajo, acotada al número de filas visibles.
    pub fn seleccion_abajo(&mut self, total: usize) {
        if total == 0 {
            self.seleccion = 0;
        } else if self.seleccion + 1 < total {
            self.seleccion += 1;
        }
    }

    /// Mueve la selección hacia arriba.
    pub fn seleccion_arriba(&mut self) {
        self.seleccion = self.seleccion.saturating_sub(1);
    }

    /// Devuelve el peer actualmente seleccionado en la pantalla Peers, si lo hay.
    pub fn peer_seleccionado(&self) -> Option<&Instancia> {
        self.datos.peers.get(self.seleccion)
    }

    /// Id de la cola seleccionada en la pantalla Redis (para purgar), si la hay.
    pub fn cola_redis_seleccionada(&self) -> Option<String> {
        self.datos
            .redis
            .as_ref()
            .and_then(|r| r.colas.get(self.seleccion))
            .map(|c| c.id.clone())
    }

    /// Abre un input limpiando el buffer.
    pub fn abrir_input(&mut self, input: Input, valor_inicial: String) {
        self.input = input;
        self.buffer = valor_inicial;
    }

    /// Cierra el input descartando el buffer.
    pub fn cerrar_input(&mut self) {
        self.input = Input::Ninguno;
        self.buffer.clear();
    }
}

/// Mapea una coordenada `y` (fila del terminal) dentro del área del CUERPO a un índice de fila
/// de la tabla. Las tablas de las pantallas se pintan con `Block::borders(ALL)` + `header`, así
/// que dentro del área del cuerpo: `y_rel = 0` es el borde superior, `y_rel = 1` el encabezado,
/// y `y_rel = 2` la primera fila de datos (índice 0). Devuelve `Some(idx)` solo si cae sobre una
/// fila de datos válida (`idx < total`); `None` si pega en el borde, el header o el vacío.
///
/// `area_cuerpo_y` es la coordenada absoluta del borde superior del cuerpo (areas[2].y).
/// Función pura — testeada. Mantener en sync con el layout de las tablas (peers/redis/traza).
pub fn fila_en_y_cuerpo(y: u16, area_cuerpo_y: u16, total: usize) -> Option<usize> {
    // Borde superior (1) + encabezado (1) = 2 filas antes de la primera fila de datos.
    let primera_fila = area_cuerpo_y.saturating_add(2);
    if y < primera_fila {
        return None;
    }
    let idx = (y - primera_fila) as usize;
    if idx < total {
        Some(idx)
    } else {
        None
    }
}

/// Recorta un texto a `max` caracteres añadiendo `…` si se pasa. Pensado para celdas de tabla.
/// Función pura — testeada.
pub fn recortar(texto: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let n = texto.chars().count();
    if n <= max {
        return texto.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let recortado: String = texto.chars().take(max - 1).collect();
    format!("{recortado}…")
}

/// Extrae solo la parte HH:MM:SS de un timestamp ISO 8601 (`visto_en`), para la columna "visto".
/// Si no encuentra el patrón, devuelve los últimos caracteres tal cual. Función pura — testeada.
pub fn hora_iso(iso: &str) -> String {
    // Formato típico: "2026-06-29T12:34:56.789Z" → buscamos tras la 'T'.
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        // Cortamos en el primer '.' o 'Z' o '+' para quedarnos con HH:MM:SS.
        let fin = resto
            .find(['.', 'Z', '+'])
            .unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

/// Símbolo + color de un estado para la columna "estado" de la pantalla Trazabilidad.
/// Mapeo (spec R2.4): ○ enviado/gris, ◑ entregado-leído/amarillo, ● procesado/verde,
/// ✕ fallido-deadletter/rojo. Función pura — testeada.
pub fn estilo_estado(estado: EstadoMensaje) -> (&'static str, ratatui::style::Color) {
    use ratatui::style::Color;
    match estado {
        EstadoMensaje::Enviado => ("○ enviado", Color::Gray),
        EstadoMensaje::Entregado => ("◑ entregado", Color::Yellow),
        EstadoMensaje::Leido => ("◑ leído", Color::Yellow),
        EstadoMensaje::Procesado => ("● procesado", Color::Green),
        EstadoMensaje::Fallido => ("✕ fallido", Color::Red),
        EstadoMensaje::DeadLetter => ("✕ dead-letter", Color::Red),
    }
}

/// Construye las 4 celdas de una fila de la tabla Trazabilidad: `id` (msg), `de_id`,
/// texto recortado y hora ISO del envío. El estado se pinta aparte (coloreado) con
/// `estilo_estado`. Función pura — testeada.
pub fn fila_traza(m: &Mensaje) -> [String; 4] {
    [
        m.id.to_string(),
        recortar(&m.de_id, 14),
        // El texto NO se recorta aquí: es columna flexible (Min) → el render lo recorta al ancho
        // real de la columna con `ancho_columna_flexible` (fix "textos cortados" 2026-07-01).
        m.texto.clone(),
        hora_iso(&m.enviado_en),
    ]
}

/// Construye las 4 celdas (id, dir, resumen, visto) de una fila de la tabla Peers, ya recortadas
/// a anchos sensatos. Función pura — testeada (formato de filas).
pub fn fila_peer(i: &Instancia) -> [String; 4] {
    [
        recortar(&i.id, 16),
        recortar(&i.directorio, 32),
        // resumen: columna flexible (Min) → recorte al ancho real en el render, no aquí.
        i.resumen.clone(),
        hora_iso(&i.visto_en),
    ]
}

/// Formatea una duración en segundos como `HhMM` / `MMmin` / `SSs` legible. `None` → "—"
/// (sesión/tarea abierta: el broker aún no timbró el fin). Función pura — testeada.
pub fn formatear_duracion(seg: Option<i64>) -> String {
    let s = match seg {
        Some(s) if s >= 0 => s,
        Some(_) => return "—".to_string(), // negativo no tiene sentido: lo tratamos como sin dato
        None => return "—".to_string(),
    };
    if s < 60 {
        return format!("{s}s");
    }
    let min = s / 60;
    if min < 60 {
        return format!("{min}min");
    }
    let horas = min / 60;
    let resto_min = min % 60;
    format!("{horas}h{resto_min:02}")
}

/// Construye las 3 celdas (inicio, fin, duración) de una fila de la tabla de sesiones de la
/// pantalla Jornada. Una sesión sin `fin` está ABIERTA (en curso). Función pura — testeada.
pub fn fila_sesion(s: &peers_core::Sesion) -> [String; 3] {
    let fin = match &s.fin {
        Some(f) if !f.is_empty() => hora_iso(f),
        _ => "(abierta)".to_string(),
    };
    [hora_iso(&s.inicio), fin, formatear_duracion(s.duracion_seg)]
}

/// Construye las 4 celdas (descripción, estimado, real, estado) de una fila de la tabla de
/// tareas (pantallas Jornada/Tareas). `estado` = "fin" si la tarea se cerró, "abierta" si sigue
/// en curso. Muestra estimado (de la IA) vs real (timbrado por el broker). Función pura — testeada.
pub fn fila_tarea(t: &Tarea) -> [String; 4] {
    let estado = if t.fin.as_deref().map(|f| !f.is_empty()).unwrap_or(false) {
        "fin".to_string()
    } else {
        "abierta".to_string()
    };
    [
        // descripción: columna flexible (Min) → recorte al ancho real en el render, no aquí.
        t.descripcion.clone(),
        formatear_duracion(t.estimado_seg),
        formatear_duracion(t.duracion_seg),
        estado,
    ]
}

/// Color del estado de una tarea (R11): Abierta/gris, EnCurso/cian, Bloqueada/naranja,
/// Hecha/verde, Cancelada/rojo. Naranja no es `Color` nombrado → RGB. Función pura — testeada.
pub fn color_estado_tarea(estado: EstadoTarea) -> ratatui::style::Color {
    use ratatui::style::Color;
    match estado {
        EstadoTarea::Abierta => Color::Gray,
        EstadoTarea::EnCurso => Color::Cyan,
        EstadoTarea::Bloqueada => Color::Rgb(255, 140, 0), // naranja
        EstadoTarea::Hecha => Color::Green,
        EstadoTarea::Cancelada => Color::Red,
    }
}

/// Etiqueta legible del estado de una tarea para la columna "estado" y el modal. Función pura.
pub fn etiqueta_estado_tarea(estado: EstadoTarea) -> &'static str {
    match estado {
        EstadoTarea::Abierta => "abierta",
        EstadoTarea::EnCurso => "en curso",
        EstadoTarea::Bloqueada => "bloqueada",
        EstadoTarea::Hecha => "hecha",
        EstadoTarea::Cancelada => "cancelada",
    }
}

/// Color de una alerta según su tipo (R6): ocioso/amarillo, atascado/naranja, ghosteo/rojo.
/// Naranja no existe como `Color` nombrado → usamos RGB. Función pura — testeada.
pub fn color_alerta(tipo: TipoAlerta) -> ratatui::style::Color {
    use ratatui::style::Color;
    match tipo {
        TipoAlerta::Ocioso => Color::Yellow,
        TipoAlerta::Atascado => Color::Rgb(255, 140, 0), // naranja
        TipoAlerta::Ghosteo => Color::Red,
        // Las dos de accountability (honestidad del peer) son graves → magenta para destacar.
        TipoAlerta::CierreSospechoso => Color::Magenta,
        TipoAlerta::CancelacionExcesiva => Color::Magenta,
    }
}

/// Etiqueta corta del tipo de alerta para la columna "tipo". Función pura — testeada.
pub fn etiqueta_alerta(tipo: TipoAlerta) -> &'static str {
    match tipo {
        TipoAlerta::Ocioso => "ocioso",
        TipoAlerta::Atascado => "atascado",
        TipoAlerta::Ghosteo => "ghosteo",
        TipoAlerta::CierreSospechoso => "cierre sospechoso",
        TipoAlerta::CancelacionExcesiva => "cancelación excesiva",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciclo_de_pantallas_tab() {
        assert_eq!(Pantalla::Peers.siguiente(), Pantalla::Acceso);
        assert_eq!(Pantalla::Config.siguiente(), Pantalla::Trazabilidad);
        assert_eq!(Pantalla::Trazabilidad.siguiente(), Pantalla::Jornada);
        assert_eq!(Pantalla::Alertas.siguiente(), Pantalla::Peers); // da la vuelta
        assert_eq!(Pantalla::Peers.anterior(), Pantalla::Alertas);
        assert_eq!(Pantalla::Acceso.anterior(), Pantalla::Peers);
    }

    #[test]
    fn ciclar_peer_foco_envuelve_y_resetea_seleccion() {
        let mut app = App::nueva(crate::config::Config::default());
        let mk = |id: &str| Instancia {
            id: id.into(), pid: 1, hostname: String::new(), directorio: "/x".into(), repo_git: None,
            repo_github: None, tty: None, resumen: String::new(),
            registrada_en: String::new(), visto_en: String::new(), ultima_actividad_en: String::new(),
            secreto: None,
        };
        app.datos.peers = vec![mk("a"), mk("b"), mk("c")];
        app.seleccion = 2;
        assert_eq!(app.traza_peer_actual().as_deref(), Some("a")); // foco inicial = 0
        app.ciclar_peer_foco(1);
        assert_eq!(app.traza_peer_actual().as_deref(), Some("b"));
        assert_eq!(app.seleccion, 0, "cambiar de peer resetea la selección de filas");
        app.ciclar_peer_foco(1);
        app.ciclar_peer_foco(1); // c → vuelve a a (envuelve)
        assert_eq!(app.traza_peer_actual().as_deref(), Some("a"));
        app.ciclar_peer_foco(-1); // a → c (envuelve hacia atrás)
        assert_eq!(app.traza_peer_actual().as_deref(), Some("c"));
    }

    #[test]
    fn click_en_tabs_mapea_pantalla() {
        // borde en x=0 → nada; primer tab "1 Peers" (7 chars) ocupa " 1 Peers " = 9 cols: [1..10).
        assert_eq!(Pantalla::pantalla_en_x_tabs(0), None); // borde izquierdo
        assert_eq!(Pantalla::pantalla_en_x_tabs(1), Some(Pantalla::Peers));
        assert_eq!(Pantalla::pantalla_en_x_tabs(9), Some(Pantalla::Peers));
        assert_eq!(Pantalla::pantalla_en_x_tabs(10), None); // divisor "│"
        // segundo tab "2 Red/Acceso" empieza en col 11.
        assert_eq!(Pantalla::pantalla_en_x_tabs(11), Some(Pantalla::Acceso));
        // muy a la derecha, fuera de todos los títulos → None.
        assert_eq!(Pantalla::pantalla_en_x_tabs(250), None);
    }

    #[test]
    fn click_en_cuerpo_mapea_fila() {
        // cuerpo arranca en y=4 (tras tabs y=0..3 + banner y=3). primera fila datos en y=6.
        assert_eq!(fila_en_y_cuerpo(4, 4, 5), None); // borde superior
        assert_eq!(fila_en_y_cuerpo(5, 4, 5), None); // encabezado
        assert_eq!(fila_en_y_cuerpo(6, 4, 5), Some(0)); // primera fila de datos
        assert_eq!(fila_en_y_cuerpo(8, 4, 5), Some(2));
        assert_eq!(fila_en_y_cuerpo(10, 4, 5), Some(4)); // última (total=5)
        assert_eq!(fila_en_y_cuerpo(11, 4, 5), None); // fuera del rango de datos
        assert_eq!(fila_en_y_cuerpo(6, 4, 0), None); // tabla vacía
    }

    #[test]
    fn teclas_numericas() {
        assert_eq!(Pantalla::desde_tecla('3'), Some(Pantalla::Redis));
        assert_eq!(Pantalla::desde_tecla('5'), Some(Pantalla::Config));
        assert_eq!(Pantalla::desde_tecla('6'), Some(Pantalla::Trazabilidad));
        assert_eq!(Pantalla::desde_tecla('7'), Some(Pantalla::Jornada));
        assert_eq!(Pantalla::desde_tecla('8'), Some(Pantalla::Tareas));
        assert_eq!(Pantalla::desde_tecla('9'), Some(Pantalla::Alertas));
        assert_eq!(Pantalla::desde_tecla('0'), None);
    }

    #[test]
    fn estilo_estado_mapea_simbolo_y_color() {
        use ratatui::style::Color;
        assert_eq!(estilo_estado(EstadoMensaje::Enviado), ("○ enviado", Color::Gray));
        assert_eq!(estilo_estado(EstadoMensaje::Entregado).1, Color::Yellow);
        assert_eq!(estilo_estado(EstadoMensaje::Leido).1, Color::Yellow);
        assert_eq!(estilo_estado(EstadoMensaje::Procesado), ("● procesado", Color::Green));
        assert_eq!(estilo_estado(EstadoMensaje::Fallido).1, Color::Red);
        assert_eq!(estilo_estado(EstadoMensaje::DeadLetter).1, Color::Red);
    }

    #[test]
    fn fila_traza_formatea_4_celdas() {
        let m = Mensaje {
            id: 42,
            de_id: "claudia".to_string(),
            para_id: "max".to_string(),
            texto: "hola que tal".to_string(),
            enviado_en: "2026-06-29T12:34:56.000Z".to_string(),
            estado: EstadoMensaje::Procesado,
            entregado_en: None,
            leido_en: None,
            procesado_en: None,
            intentos: 0,
            reenviado_de: None,
            reenvios: 0,
        };
        let fila = fila_traza(&m);
        assert_eq!(fila[0], "42");
        assert_eq!(fila[1], "claudia");
        assert_eq!(fila[2], "hola que tal");
        assert_eq!(fila[3], "12:34:56");
    }

    #[test]
    fn fila_traza_no_recorta_texto_es_columna_flexible() {
        // Desde el fix "textos cortados" (2026-07-01) la función pura NO recorta el texto:
        // es columna flexible y el recorte lo hace el render según el ancho real. Aquí se
        // verifica que devuelve el texto ÍNTEGRO (sin `…`).
        let m = Mensaje {
            id: 1,
            de_id: "p".to_string(),
            para_id: "q".to_string(),
            texto: "x".repeat(100),
            enviado_en: String::new(),
            estado: EstadoMensaje::Enviado,
            entregado_en: None,
            leido_en: None,
            procesado_en: None,
            intentos: 0,
            reenviado_de: None,
            reenvios: 0,
        };
        let fila = fila_traza(&m);
        assert_eq!(fila[2].chars().count(), 100);
        assert!(!fila[2].ends_with('…'));
    }

    #[test]
    fn recortar_respeta_max_y_anade_elipsis() {
        assert_eq!(recortar("hola", 10), "hola");
        assert_eq!(recortar("hola mundo", 5), "hola…");
        assert_eq!(recortar("abc", 0), "");
        assert_eq!(recortar("abc", 1), "…");
        // Multibyte: no debe panicar cortando en medio de un char.
        assert_eq!(recortar("áéíóú", 3), "áé…");
    }

    #[test]
    fn hora_iso_extrae_hms() {
        assert_eq!(hora_iso("2026-06-29T12:34:56.789Z"), "12:34:56");
        assert_eq!(hora_iso("2026-06-29T08:00:00Z"), "08:00:00");
        assert_eq!(hora_iso("2026-06-29T08:00:00+02:00"), "08:00:00");
        assert_eq!(hora_iso("sin-formato"), "sin-formato");
    }

    #[test]
    fn fila_peer_formatea_4_celdas() {
        let i = Instancia {
            id: "claudia".to_string(),
            pid: 123,
            hostname: String::new(),
            directorio: "/Users/max/proyecto".to_string(),
            repo_git: None,
            repo_github: None,
            tty: None,
            resumen: "construyendo la TUI".to_string(),
            registrada_en: "2026-06-29T10:00:00Z".to_string(),
            visto_en: "2026-06-29T12:34:56.000Z".to_string(),
            ultima_actividad_en: String::new(),
            secreto: None,
        };
        let fila = fila_peer(&i);
        assert_eq!(fila[0], "claudia");
        assert_eq!(fila[1], "/Users/max/proyecto");
        assert_eq!(fila[2], "construyendo la TUI");
        assert_eq!(fila[3], "12:34:56");
    }

    #[test]
    fn fila_peer_no_recorta_resumen_es_columna_flexible() {
        // Igual que fila_traza: el resumen es columna flexible, la función pura NO lo recorta
        // (lo hace el render con el ancho real). Verifica que devuelve el resumen íntegro.
        let largo = "x".repeat(100);
        let i = Instancia {
            id: "p".to_string(),
            pid: 1,
            hostname: String::new(),
            directorio: String::new(),
            repo_git: None,
            repo_github: None,
            tty: None,
            resumen: largo,
            registrada_en: String::new(),
            visto_en: String::new(),
            ultima_actividad_en: String::new(),
            secreto: None,
        };
        let fila = fila_peer(&i);
        assert_eq!(fila[2].chars().count(), 100);
        assert!(!fila[2].ends_with('…'));
    }

    #[test]
    fn formatear_duracion_legible() {
        assert_eq!(formatear_duracion(None), "—");
        assert_eq!(formatear_duracion(Some(-5)), "—");
        assert_eq!(formatear_duracion(Some(0)), "0s");
        assert_eq!(formatear_duracion(Some(45)), "45s");
        assert_eq!(formatear_duracion(Some(90)), "1min");
        assert_eq!(formatear_duracion(Some(600)), "10min");
        assert_eq!(formatear_duracion(Some(3600)), "1h00");
        assert_eq!(formatear_duracion(Some(3660)), "1h01");
        assert_eq!(formatear_duracion(Some(8130)), "2h15");
    }

    #[test]
    fn fila_sesion_abierta_y_cerrada() {
        let abierta = peers_core::Sesion {
            id: "s1".to_string(),
            instancia_id: "claudia".to_string(),
            inicio: "2026-06-29T08:00:00Z".to_string(),
            fin: None,
            duracion_seg: None,
        };
        let f = fila_sesion(&abierta);
        assert_eq!(f[0], "08:00:00");
        assert_eq!(f[1], "(abierta)");
        assert_eq!(f[2], "—");

        let cerrada = peers_core::Sesion {
            id: "s2".to_string(),
            instancia_id: "claudia".to_string(),
            inicio: "2026-06-29T08:00:00Z".to_string(),
            fin: Some("2026-06-29T10:30:00Z".to_string()),
            duracion_seg: Some(9000),
        };
        let f = fila_sesion(&cerrada);
        assert_eq!(f[1], "10:30:00");
        assert_eq!(f[2], "2h30");
    }

    #[test]
    fn fila_tarea_estimado_vs_real() {
        let t = Tarea {
            id: "t1".to_string(),
            instancia_id: "claudia".to_string(),
            sesion_id: "s1".to_string(),
            descripcion: "implementar pantallas".to_string(),
            inicio: "2026-06-29T08:00:00Z".to_string(),
            fin: Some("2026-06-29T09:00:00Z".to_string()),
            duracion_seg: Some(3600),
            estimado_seg: Some(600),
            estado: EstadoTarea::Hecha,
            bloqueo_motivo: None,
            issue_number: None,
            factor_aprendido: false,
            evidencia: None,
        };
        let f = fila_tarea(&t);
        assert_eq!(f[0], "implementar pantallas");
        assert_eq!(f[1], "10min"); // estimado de la IA
        assert_eq!(f[2], "1h00"); // real timbrado por el broker
        assert_eq!(f[3], "fin");

        let abierta = Tarea {
            fin: None,
            duracion_seg: None,
            estimado_seg: None,
            ..t
        };
        let f = fila_tarea(&abierta);
        assert_eq!(f[1], "—");
        assert_eq!(f[2], "—");
        assert_eq!(f[3], "abierta");
    }

    #[test]
    fn alerta_color_y_etiqueta() {
        use ratatui::style::Color;
        assert_eq!(color_alerta(TipoAlerta::Ocioso), Color::Yellow);
        assert_eq!(color_alerta(TipoAlerta::Atascado), Color::Rgb(255, 140, 0));
        assert_eq!(color_alerta(TipoAlerta::Ghosteo), Color::Red);
        assert_eq!(etiqueta_alerta(TipoAlerta::Ocioso), "ocioso");
        assert_eq!(etiqueta_alerta(TipoAlerta::Atascado), "atascado");
        assert_eq!(etiqueta_alerta(TipoAlerta::Ghosteo), "ghosteo");
    }

    #[test]
    fn estado_tarea_color_y_etiqueta() {
        use ratatui::style::Color;
        assert_eq!(color_estado_tarea(EstadoTarea::Abierta), Color::Gray);
        assert_eq!(color_estado_tarea(EstadoTarea::EnCurso), Color::Cyan);
        assert_eq!(color_estado_tarea(EstadoTarea::Bloqueada), Color::Rgb(255, 140, 0));
        assert_eq!(color_estado_tarea(EstadoTarea::Hecha), Color::Green);
        assert_eq!(color_estado_tarea(EstadoTarea::Cancelada), Color::Red);
        assert_eq!(etiqueta_estado_tarea(EstadoTarea::Abierta), "abierta");
        assert_eq!(etiqueta_estado_tarea(EstadoTarea::EnCurso), "en curso");
        assert_eq!(etiqueta_estado_tarea(EstadoTarea::Bloqueada), "bloqueada");
        assert_eq!(etiqueta_estado_tarea(EstadoTarea::Hecha), "hecha");
        assert_eq!(etiqueta_estado_tarea(EstadoTarea::Cancelada), "cancelada");
    }

    /// Constructor mínimo de tarea para los tests de la vista global.
    fn mk_tarea(id: &str, inst: &str, estado: EstadoTarea, inicio: &str, estimado_seg: Option<i64>) -> Tarea {
        Tarea {
            id: id.into(),
            instancia_id: inst.into(),
            sesion_id: "s".into(),
            descripcion: "x".into(),
            inicio: inicio.into(),
            fin: None,
            duracion_seg: None,
            estimado_seg,
            estado,
            bloqueo_motivo: None,
            issue_number: None,
            factor_aprendido: false,
            evidencia: None,
        }
    }

    #[test]
    fn filtro_desde_tecla_mapea_estados() {
        assert_eq!(App::filtro_desde_tecla('1'), Some(EstadoTarea::Abierta));
        assert_eq!(App::filtro_desde_tecla('2'), Some(EstadoTarea::EnCurso));
        assert_eq!(App::filtro_desde_tecla('3'), Some(EstadoTarea::Bloqueada));
        assert_eq!(App::filtro_desde_tecla('4'), Some(EstadoTarea::Hecha));
        assert_eq!(App::filtro_desde_tecla('5'), Some(EstadoTarea::Cancelada));
        assert_eq!(App::filtro_desde_tecla('0'), None);
        assert_eq!(App::filtro_desde_tecla('9'), None);
    }

    #[test]
    fn tarea_overrun_solo_vivas_y_pasadas_de_estimado() {
        let ahora = "2026-06-29T12:00:00Z";
        // Viva (EnCurso), empezó hace 2h con estimado 1h → overrun.
        let viva_tarde = mk_tarea("a", "p1", EstadoTarea::EnCurso, "2026-06-29T10:00:00Z", Some(3600));
        assert!(App::tarea_overrun(&viva_tarde, ahora));
        // Viva pero dentro del estimado (empezó hace 30min, estimado 1h) → no overrun.
        let viva_ok = mk_tarea("b", "p1", EstadoTarea::Abierta, "2026-06-29T11:30:00Z", Some(3600));
        assert!(!App::tarea_overrun(&viva_ok, ahora));
        // Hecha aunque pasada de tiempo → nunca overrun (no está viva).
        let hecha = mk_tarea("c", "p1", EstadoTarea::Hecha, "2026-06-29T08:00:00Z", Some(3600));
        assert!(!App::tarea_overrun(&hecha, ahora));
        // Sin estimado válido → no overrun (degrada).
        let sin_est = mk_tarea("d", "p1", EstadoTarea::EnCurso, "2026-06-29T08:00:00Z", None);
        assert!(!App::tarea_overrun(&sin_est, ahora));
    }

    #[test]
    fn vista_global_filtra_y_ordena_overrun_primero() {
        let cfg = crate::config::Config::default();
        let mut app = App::nueva(cfg);
        let ahora = "2026-06-29T12:00:00Z";
        app.ahora = ahora.to_string();
        // 3 tareas: una hecha, una en curso a tiempo, una en curso en overrun.
        app.datos.tareas = vec![
            mk_tarea("hecha", "p1", EstadoTarea::Hecha, "2026-06-29T08:00:00Z", Some(3600)),
            mk_tarea("ok", "p2", EstadoTarea::EnCurso, "2026-06-29T11:50:00Z", Some(3600)),
            mk_tarea("overrun", "p3", EstadoTarea::EnCurso, "2026-06-29T09:00:00Z", Some(3600)),
        ];

        // Vista peer (default): sin reordenar ni filtrar → orden de origen.
        let peer = app.tareas_ordenadas(ahora);
        assert_eq!(peer.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["hecha", "ok", "overrun"]);

        // Vista global sin filtro: la overrun sube al principio (orden estable en el resto).
        app.vista_global = true;
        let glob = app.tareas_ordenadas(ahora);
        assert_eq!(glob[0].id, "overrun", "la atascada va primero");
        assert_eq!(glob.len(), 3);

        // Filtro por EnCurso: solo las 2 en curso, overrun primero.
        app.filtro_estado = Some(EstadoTarea::EnCurso);
        let enc = app.tareas_ordenadas(ahora);
        assert_eq!(enc.len(), 2);
        assert_eq!(enc[0].id, "overrun");
        assert_eq!(enc[1].id, "ok");

        // Filtro por Hecha: solo la hecha.
        app.filtro_estado = Some(EstadoTarea::Hecha);
        let h = app.tareas_ordenadas(ahora);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].id, "hecha");
    }

    #[test]
    fn tarea_seleccionada_sigue_el_orden_mostrado_en_vista_global() {
        let mut app = App::nueva(crate::config::Config::default());
        let ahora = "2026-06-29T12:00:00Z";
        app.ahora = ahora.to_string();
        app.vista_global = true;
        app.datos.tareas = vec![
            mk_tarea("hecha", "p1", EstadoTarea::Hecha, "2026-06-29T08:00:00Z", Some(3600)),
            mk_tarea("overrun", "p3", EstadoTarea::EnCurso, "2026-06-29T09:00:00Z", Some(3600)),
        ];
        // En vista global la overrun va primero: seleccion 0 debe apuntar a "overrun".
        app.seleccion = 0;
        assert_eq!(app.tarea_seleccionada().map(|t| t.id.as_str()), Some("overrun"));
        app.seleccion = 1;
        assert_eq!(app.tarea_seleccionada().map(|t| t.id.as_str()), Some("hecha"));
    }

    #[test]
    fn seleccion_acotada() {
        let cfg = crate::config::Config::default();
        let mut app = App::nueva(cfg);
        app.seleccion_abajo(0); // sin filas → queda en 0
        assert_eq!(app.seleccion, 0);
        app.seleccion_abajo(3);
        app.seleccion_abajo(3);
        app.seleccion_abajo(3);
        app.seleccion_abajo(3); // tope en total-1
        assert_eq!(app.seleccion, 2);
        app.seleccion_arriba();
        assert_eq!(app.seleccion, 1);
    }
}
