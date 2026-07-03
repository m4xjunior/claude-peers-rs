//! `AppDesktop`: estado raíz y layout principal de la app desktop.
//!
//! INTENCIÓN: es el `View` que envuelve `Root`. Mantiene qué pantalla está activa y dibuja el
//! layout de dos columnas — sidebar navegable a la izquierda (las 9 pantallas espejo de la TUI)
//! y área de contenido a la derecha que delega en el `render_<pantalla>` correspondiente.
//!
//! Decisión de Fundación (por qué): la navegación se implementa con `Button` en un `v_flex`,
//! no con el componente `Sidebar` del kit. Motivo: la API de `Sidebar` varía entre commits del
//! git de gpui-component y podría romper la compilación de la Fundación; un `v_flex` de botones
//! es estable, navega igual (click cambia la activa) y no bloquea a los agentes de Fase 2, que
//! pueden migrarlo a `Sidebar` cuando fijen la revisión del kit. El cableado de datos y el
//! contrato de `EstadoPantalla` quedan idénticos.

use gpui::{
    div, prelude::FluentBuilder, AppContext, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;

use crate::cliente::ClienteBroker;
use crate::tema;
use crate::vista;
use crate::vista::config::PanelConfig;
use crate::vista::lanzador::PanelLanzador;

/// Las 9 pantallas de la app, espejo 1:1 de las pantallas de la TUI. `Copy` porque es un
/// discriminante trivial que se compara y se guarda en el estado sin coste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pantalla {
    Peers,
    Alertas,
    Broker,
    Config,
    Jornada,
    Redis,
    Tareas,
    Trazabilidad,
    Acceso,
    /// 10ª pestaña — RFC Lanzador Fase 1 (Zona A: configuración de sesión + preview del comando).
    Lanzador,
}

impl Pantalla {
    /// Orden de aparición en el sidebar. Fuente única para pintar la navegación y para
    /// no repetir la lista en varios sitios (añadir/quitar pantalla = tocar sólo aquí).
    pub const TODAS: [Pantalla; 10] = [
        Pantalla::Peers,
        Pantalla::Alertas,
        Pantalla::Broker,
        Pantalla::Config,
        Pantalla::Jornada,
        Pantalla::Redis,
        Pantalla::Tareas,
        Pantalla::Trazabilidad,
        Pantalla::Acceso,
        Pantalla::Lanzador,
    ];

    /// Etiqueta visible en el sidebar y en el título del contenido.
    pub fn titulo(self) -> &'static str {
        match self {
            Pantalla::Peers => "Peers",
            Pantalla::Alertas => "Alertas",
            Pantalla::Broker => "Broker",
            Pantalla::Config => "Config",
            Pantalla::Jornada => "Jornada",
            Pantalla::Redis => "Redis",
            Pantalla::Tareas => "Tareas",
            Pantalla::Trazabilidad => "Trazabilidad",
            Pantalla::Acceso => "Acceso",
            Pantalla::Lanzador => "Lanzador",
        }
    }

    /// Identificador estable para el `id` del elemento interactivo (botón del sidebar). GPUI
    /// exige un id único por elemento con estado; derivarlo del enum evita colisiones.
    fn id(self) -> &'static str {
        // Reusa el título en minúsculas conceptualmente; como necesitamos `&'static str`,
        // devolvemos literales fijos por variante.
        match self {
            Pantalla::Peers => "nav-peers",
            Pantalla::Alertas => "nav-alertas",
            Pantalla::Broker => "nav-broker",
            Pantalla::Config => "nav-config",
            Pantalla::Jornada => "nav-jornada",
            Pantalla::Redis => "nav-redis",
            Pantalla::Tareas => "nav-tareas",
            Pantalla::Trazabilidad => "nav-trazabilidad",
            Pantalla::Acceso => "nav-acceso",
            Pantalla::Lanzador => "nav-lanzador",
        }
    }
}

/// Datos que las pantallas consumen para pintarse. En la Fundación está vacío (los stubs no
/// leen nada); existe AHORA para fijar el contrato de firma `render_<pantalla>(&EstadoPantalla)`
/// y que la Fase 2 sólo tenga que añadir campos (p. ej. `instancias`, `tareas`, `alertas`…),
/// no cambiar firmas ni el enrutado del sidebar.
#[derive(Default)]
pub struct EstadoPantalla {
    // Fase 2 rellenará aquí las cachés por pantalla, p. ej.:
    // pub tareas: Vec<peers_core::Tarea>,
    // ...

    // --- Pantalla Peers (Fase 2) ---
    /// Peers vivos de la máquina (`POST /listar`). Alimenta la tabla de la pantalla Peers.
    pub instancias: Vec<peers_core::Instancia>,
    /// Alertas vigentes del supervisor (`GET /admin/alertas`). La pantalla Peers las cruza por
    /// `sujeto == instancia.id` para derivar el estado ocioso/atascado por peer (columna añadida
    /// respecto a la TUI). Compartida también con la pantalla Alertas.
    pub alertas: Vec<peers_core::Alerta>,
    /// Último error al hablar con el broker en la carga de Peers, si lo hubo. La pantalla lo pinta
    /// como banner en vez de dejar la tabla vacía sin explicación (offline vs 401 vs otro).
    pub error_peers: Option<crate::cliente::ErrorBroker>,
    /// Peer seleccionado en la tabla de Peers (`Some(indice)` sobre `instancias`), o `None` si no
    /// hay ninguno marcado. Habilita la barra de acciones del peer (enviar/jornada). Espejo del
    /// cursor de la TUI; se acota/limpia al recargar `instancias` (ver `cargar_peers`).
    pub peers_seleccion: Option<usize>,
    /// Si `Some(i)`, se muestra el POP-UP de detalle (peers-01) del peer `instancias[i]`:
    /// identidad, rutas, resumen íntegro, señales de vida y métricas. Espejo de `tarea_detalle`;
    /// se cierra si el índice queda fuera de rango al recargar el roster.
    pub peer_detalle: Option<usize>,
    /// Formulario de la pestaña Peers abierto (peers-02 composer / peers-03 kick), o `None`.
    pub peers_form: Option<crate::vista::peers::FormPeers>,
    /// Error de validación de frontera del formulario de Peers (mensaje vacío). Se pinta DENTRO
    /// del modal. Espejo de `tareas_form_error`.
    pub peers_form_error: Option<String>,
    /// Input del composer de mensaje (peers-02). `Entity<InputState>` del kit creada al arrancar;
    /// `None` sólo si no llegó a construirse (el modal pinta un aviso; nunca `.unwrap()`).
    pub input_mensaje_peer: Option<Entity<gpui_component::input::InputState>>,

    // --- Pantalla Trazabilidad (Fase 2) ---
    /// Peer en foco cuyo historial se muestra (espejo de `traza_peer_actual` de la TUI).
    /// `None` = aún no hay peer seleccionado en la pantalla Peers.
    pub traza_peer: Option<String>,
    /// Historial durable de la cola del peer en foco (`GET /admin/historial`), en orden
    /// cronológico ascendente tal como lo devuelve el broker. Vacío si no hay foco o datos.
    pub historial: Vec<peers_core::Mensaje>,
    /// Índice de la fila seleccionada en la tabla de trazabilidad. Sirve para resaltar la fila
    /// y decidir qué mensaje abre su timeline. Se mantiene dentro de `historial`.
    pub traza_seleccion: usize,
    /// Si `true`, se muestra el MODAL de timeline (trazabilidad-01) del mensaje seleccionado —
    /// transiciones + timestamps timbrados por el broker. Espejo del modal `Enter` de la TUI.
    /// (Antes abría un panel inline; ahora abre el overlay `overlay_mensaje`.)
    pub traza_timeline: bool,
    /// Filtro por estado del historial (trazabilidad-02). `Some(estado)` recarga el historial
    /// acotado EN EL BROKER (`GET /admin/historial?estado=`); `None` = chip "Todos". Se conserva
    /// entre recargas y al cambiar de peer en foco.
    pub traza_filtro_estado: Option<peers_core::EstadoMensaje>,
    /// Si `Some(msg_id)`, está abierto el mini-modal de CONFIRMACIÓN de reenvío (trazabilidad-05)
    /// para ese mensaje. El POST `/admin/reenviar` sólo sale al confirmar.
    pub traza_confirmar_reenvio: Option<i64>,

    // --- Pantalla Broker (Fase 2) ---
    /// Datos de arranque del broker (`GET /admin/info`): host, puerto, versión, nº instancias.
    /// `None` mientras no haya llegado la primera respuesta (o si el broker está offline).
    pub info: Option<peers_core::RespuestaAdminInfo>,
    /// Estado de salud del broker (`GET /salud`): "ok"/otro y nº de instancias vivas.
    /// `None` mientras no haya datos; la pantalla pinta el aviso de "sin datos todavía".
    pub salud: Option<peers_core::RespuestaSalud>,
    /// Factor de corrección de estimación aprendido (`GET /factor-estimacion`). Sólo lectura:
    /// lo aprende el broker del tiempo real. `None` si aún no llegó la respuesta.
    pub factor: Option<peers_core::FactorEstimacion>,

    // --- Pantalla Alertas (Fase 2) ---
    /// Índice de la fila seleccionada en la tabla de alertas. Resalta la fila y decide qué
    /// alerta expande su detalle. Se mantiene dentro de `alertas` (se acota al recargar).
    pub alertas_seleccion: usize,
    /// Si `Some(i)`, se muestra el MODAL de detalle (alertas-01) de la alerta `alertas[i]` con su
    /// texto íntegro (la celda de la tabla lo recorta). `i` es el índice REAL en `alertas` (no el
    /// visible): al abrir desde una fila se traduce el índice visible→real aplicando el filtro, para
    /// que el modal apunte a la alerta correcta aunque haya filtro activo. `None` = sin modal.
    pub alerta_detalle: Option<usize>,
    /// Último error al hablar con el broker desde la pantalla Alertas (cargar lista o descartar).
    /// Se pinta como banner en vez de dejar la tabla muda. Separado de `error_peers` a propósito.
    pub error_alertas: Option<crate::cliente::ErrorBroker>,
    /// Filtro por tipo de alerta activo (alertas-03), como tipos SERIALIZADOS ("ocioso",
    /// "cierre_sospechoso"…). Vacío = mostrar TODAS. Multi-select: varios tipos suman (unión). La
    /// vista pinta sólo las alertas cuyo tipo está en el conjunto (o todas si vacío) y el conteo de
    /// la cabecera se recalcula. Se conserva entre recargas (no se toca en `cargar_alertas`).
    pub alertas_filtro_tipos: Vec<String>,
    /// Si `true`, el modal de detalle muestra el paso 2 de la confirmación de descarte (alertas-09):
    /// "¿Seguro? [Sí, descartar]" en vez del botón normal. Se pone a `true` con
    /// `PedirConfirmacionDescarte`; vuelve a `false` al cancelar, cerrar el modal o descartar.
    pub alerta_confirmar_descarte: bool,

    // --- Pantalla Acceso (Fase 2) ---
    // host/puerto/versión salen de `info` (GET /admin/info) y el estado de salud de `salud`
    // (GET /salud); ambos ya están arriba y se reutilizan (no se duplican). La URL base y el token
    // enmascarado se cachean aquí porque la vista sólo recibe `&EstadoPantalla` (no el cliente);
    // la app los rellena una vez desde `ClienteBroker::base`/`token_enmascarado` al construir.
    /// URL base del broker a la que apunta la desktop (ej. `http://127.0.0.1:7899`). Copia de
    /// `ClienteBroker::base` fijada al arrancar; la pantalla Acceso la muestra tal cual.
    pub acceso_url: String,
    /// Token enmascarado (ej. `lexus…2026`) para mostrarlo sin revelarlo. Copia de
    /// `ClienteBroker::token_enmascarado` fijada al arrancar.
    pub acceso_token: String,
    /// `true` si el cliente corre SIN token (modo anónimo). Copia tipada de `ClienteBroker::sin_token`
    /// fijada al arrancar y en cada `aplicar_conexion`. La usa la pantalla Acceso (acceso-09) para el
    /// aviso "broker expuesto sin token" en vez de comparar el string enmascarado (parse, don't
    /// validate): el hecho "no hay token" es un booleano de dominio, no una coincidencia de texto.
    pub acceso_sin_token: bool,
    /// Último error al comprobar el acceso al broker (cargar `/salud` o `/admin/info`). La pantalla
    /// lo pinta como banner (offline vs 401 vs otro) para explicar por qué faltan datos.
    pub error_acceso: Option<crate::cliente::ErrorBroker>,
    /// Panel stateful de la pantalla Acceso: Inputs editables de url/token (acceso-01/02), resultado
    /// de "Probar conexión" (acceso-05) y diagnóstico del último error (acceso-06). Es una
    /// `Entity<PanelAcceso>` porque los `Input` del kit exigen estado que no cabe en una vista pura;
    /// la app la construye al arrancar. `None` sólo si la construcción no llegó a hacerse: la vista
    /// pinta la parte de solo-lectura igualmente (nunca `.unwrap()`).
    pub panel_acceso: Option<Entity<crate::vista::acceso::PanelAcceso>>,

    // --- Pantalla Redis (Fase 2) ---
    /// Colas de mensajes + outbox pendientes por peer (`GET /admin/redis`). `None` mientras no
    /// haya llegado la primera respuesta (o si el broker está offline); la pantalla pinta el
    /// aviso "sin datos todavía" en ese caso, no dos tablas vacías sin explicación.
    pub redis: Option<peers_core::RespuestaAdminRedis>,
    /// Último error al hablar con el broker desde la pantalla Redis (cargar colas o purgar). Se
    /// pinta como banner (offline vs 401 vs otro) en vez de dejar las tablas mudas. Separado del
    /// resto de `error_*` a propósito: cada pantalla explica su propio fallo.
    pub error_redis: Option<crate::cliente::ErrorBroker>,
    /// Cola seleccionada en la tabla de colas de Redis (`Some(indice)` sobre `redis.colas`), o
    /// `None`. Alimenta el botón "Purgar" (que actúa sobre el peer de esa fila) y resalta la fila.
    /// Espejo del cursor + tecla `p` de la TUI. Tras purgar se pone a `None` (ese peer desaparece).
    pub redis_seleccion_cola: Option<usize>,
    /// Si `Some(id)`, está abierto el pop-up de INSPECCIÓN de la cola de ese peer (redis-01).
    pub redis_cola_detalle: Option<String>,
    /// Mensajes pendientes de la cola inspeccionada (historial `estado=enviado` del peer). Se
    /// cargan al abrir el pop-up y se refrescan tras un reenvío desde él.
    pub redis_cola_mensajes: Vec<peers_core::Mensaje>,
    /// `true` mientras el fetch del contenido de la cola está en vuelo (el modal pinta "Cargando…"
    /// en vez de confundir "aún no llegó" con "no hay pendientes").
    pub redis_cola_cargando: bool,
    /// Si `Some(id)`, está abierto el diálogo de CONFIRMACIÓN de purga (redis-04) para ese peer.
    /// El POST `/admin/purgar` sólo sale al confirmar.
    pub redis_confirmar_purga: Option<String>,
    /// Instante MONOTÓNICO local de la última respuesta OK de `/admin/redis` (redis-05): alimenta
    /// el sello "actualizado hace Xs". Es frescura de la UI, no un tiempo de trabajo (esos los
    /// timbra siempre el broker).
    pub redis_actualizado_en: Option<std::time::Instant>,

    // --- Pantalla Tareas (Fase 2) ---
    /// TODAS las tareas de TODOS los peers (`GET /admin/tareas`), cada `Tarea` con su `instancia_id`,
    /// ordenadas por el broker (inicio desc). Alimentan la tabla global de la pantalla Tareas con el
    /// estado coloreado, estimado vs real y la columna dueño. Vacío = sin tareas o aún sin cargar.
    pub tareas: Vec<peers_core::Tarea>,
    /// Índice de la fila seleccionada en la tabla de Tareas. Resalta la fila (acento brasa) y decide
    /// sobre qué tarea opera la barra de acciones (estado/reasignar/forzar). Se acota a `tareas` al
    /// recargar. Espejo de `alertas_seleccion`. Lo mueve `SeleccionarTarea{indice}` (Fase 3).
    pub tareas_seleccion: usize,
    /// Último error al hablar con el broker desde la pantalla Tareas (cargar lista o ejecutar una
    /// acción). Se pinta como banner (offline vs 401 vs otro) en vez de dejar la tabla muda.
    pub error_tareas: Option<crate::cliente::ErrorBroker>,
    /// Si `Some(i)`, se muestra el POP-UP de detalle (tareas-01) de la tarea `tareas[i]`: descripción
    /// íntegra, dueño, estimado vs real, motivo de bloqueo, evidencia e issue. `i` es el índice REAL
    /// en `tareas` (la vista global no filtra, así que fila pintada == índice real). `None` = sin
    /// pop-up. Espejo exacto de `alerta_detalle`; se acota/cierra al recargar la lista.
    pub tarea_detalle: Option<usize>,
    /// Formulario del CRUD de tareas abierto (tareas-03..08), o `None`. Cada variante lleva su
    /// contexto capturado al abrir (tarea_id, dueño, estimado vigente) — ver `FormTareas`.
    pub tareas_form: Option<crate::vista::tareas::FormTareas>,
    /// Peer elegido en el selector del formulario activo (asignar/reasignar). Se limpia al abrir.
    pub tareas_form_peer: Option<String>,
    /// Error de VALIDACIÓN DE FRONTERA del formulario activo (campo obligatorio vacío, número
    /// ilegible…). Se pinta DENTRO del modal (el banner de la pantalla queda detrás del overlay).
    pub tareas_form_error: Option<String>,
    /// Las tres entradas de texto compartidas por los formularios del CRUD (descripcion/estimado/
    /// motivo). Son `Entity<InputState>` del kit — estado que no cabe en la vista pura — creadas
    /// por la app al arrancar. `None` sólo si la construcción no llegó a hacerse (nunca `.unwrap()`;
    /// el modal pinta un aviso).
    pub inputs_tareas: Option<crate::vista::tareas::InputsTareas>,

    // --- Pantalla Jornada (Fase 2) ---
    /// Jornada del peer enfocado (sesiones + tareas timbradas por el broker). `None` hasta que
    /// Peers fije un foco y se cargue vía `ClienteBroker::jornada`. La pantalla Jornada lo pinta;
    /// si es `None` muestra el texto guía "selecciona un peer".
    pub jornada: Option<peers_core::RespuestaJornada>,
    /// Bitácora de acciones del peer enfocado en Jornada (`GET /acciones`, registro-acciones
    /// R10): timeline "quién hizo qué, cuándo". Se recarga junto a la jornada; vacía si el
    /// broker no tiene bitácora activa o aún no expone el endpoint (degradación sin banner).
    pub jornada_acciones: Vec<peers_core::AccionRegistrada>,
    /// Id del peer cuya jornada está cacheada (para el título "Jornada · <id>"). `None` = sin foco.
    pub jornada_peer: Option<String>,
    /// Índice de la sesión seleccionada en la tabla de sesiones de Jornada (feedback de cursor,
    /// espejo de la TUI). Sólo UI; se acota al recargar `jornada`.
    pub jornada_ses_sel: usize,
    /// Índice de la tarea seleccionada en la tabla de tareas de Jornada. Sólo UI.
    pub jornada_tar_sel: usize,
    /// Si `Some(i)`, está abierto el pop-up de detalle (jornada-01) de la tarea `jornada.tareas[i]`.
    /// El contenido REUTILIZA el modal de la fase 1 (`tareas::render_modal_detalle_tarea`) más la
    /// card de transiciones (jornada-04). Se cierra si queda fuera de rango al recargar.
    pub jornada_tarea_detalle: Option<usize>,
    /// Si `Some(i)`, está abierto el pop-up de detalle de sesión (jornada-02) de
    /// `jornada.sesiones[i]`, con las tareas correlacionadas por `sesion_id`.
    pub jornada_sesion_detalle: Option<usize>,
    /// Si `true`, el DROPDOWN del selector de peer (jornada-03, decisión A+B de Max) está
    /// desplegado mostrando los peers vivos.
    pub jornada_dropdown: bool,
    /// Colapsado/expandido de las 3 tablas de Jornada (bug #1: peers con muchas sesiones/tareas/
    /// acciones desbordaban la pantalla y sepultaban el resto). `false` = expandida por defecto
    /// (comportamiento actual sin cambios si Max nunca toca el chevron). Efímero, no se persiste.
    pub jornada_ses_colapsada: bool,
    pub jornada_tar_colapsada: bool,
    pub jornada_acc_colapsada: bool,

    // --- Pantalla Config (Fase 2) ---
    /// Panel de configuración con estado propio (Inputs editables + feedback de guardado). Es una
    /// `Entity<PanelConfig>` porque los `Input` del kit exigen estado que no cabe en una función
    /// pura; la app la construye al arrancar. `None` sólo si la construcción no llegó a hacerse:
    /// la vista pinta "Config no inicializada" en ese caso (nunca `.unwrap()`).
    pub panel_config: Option<Entity<PanelConfig>>,

    // --- Pantalla Lanzador (RFC Lanzador Fase 1) ---
    /// Panel del Lanzador con estado propio (Inputs del kit para el system prompt, host SSH,
    /// nombre tmux; destino, tareas, flags). Igual que `panel_config`, es una `Entity` porque los
    /// `Input` del kit exigen estado; la app la construye al arrancar. `None` sólo si no se llegó
    /// a construir: la vista pinta "Lanzador no inicializado" (nunca `.unwrap()`).
    pub panel_lanzador: Option<Entity<PanelLanzador>>,
}

/// Estado raíz de la app: cliente del broker, pantalla activa y datos cacheados.
pub struct AppDesktop {
    /// Cliente HTTP hacia el broker. Se guarda para que la Fase 2 dispare recargas con `cx.spawn`.
    cliente: ClienteBroker,
    /// Pantalla actualmente visible en el área de contenido.
    activa: Pantalla,
    /// Datos que consumen las pantallas (vacío en Fundación).
    datos: EstadoPantalla,
    /// Foco del contenedor raíz. INTENCIÓN: sin un `FocusHandle` rastreado por `track_focus` en el
    /// div raíz, GPUI NO entrega los `KeyDownEvent` a la app → la navegación por teclado (↑/↓/j/k,
    /// números, Enter, letras de acción, espejo de la TUI) no funcionaría. Se crea al arrancar y se
    /// enfoca una vez (`window.focus`) para que la ventana capture las teclas desde el primer frame.
    foco: gpui::FocusHandle,
}

impl AppDesktop {
    /// Constructor invocado dentro de `open_window`. Arranca en la pantalla Peers (igual que
    /// la TUI) con el cliente configurado desde el entorno.
    pub fn nueva(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cliente = ClienteBroker::default();
        // El panel de Config es una entidad con estado (Inputs del kit): se construye AQUÍ, donde
        // hay `window`/`cx`, y se guarda en `datos` para que la pantalla Config delegue en él. Si no
        // se construyera, la vista pinta "Config no inicializada" (nunca crashea).
        let panel_config = Some(crate::vista::config::nuevo_panel(window, cx));
        // El panel de Acceso también es stateful (Inputs editables de url/token): se construye AQUÍ,
        // sembrado con la config del disco, para que la pantalla Acceso delegue en él la edición.
        let panel_acceso = Some(crate::vista::acceso::nuevo_panel(window, cx));
        // El panel del Lanzador es stateful (Input multilínea del system prompt + inputs de host/
        // tmux/tarea): se construye AQUÍ, sembrado con la config del disco (recientes + plantillas).
        let panel_lanzador = Some(crate::vista::lanzador::nuevo_panel(window, cx));
        // La URL base y el token enmascarado son config fija del cliente: se copian una vez al
        // estado para que la pantalla Acceso (que sólo recibe `&EstadoPantalla`) pueda pintarlos
        // sin acceso al cliente. Nunca se guarda el token en claro, sólo su versión enmascarada.
        // Las entradas del CRUD de tareas (tareas-03..08) también son stateful (InputState del
        // kit): se crean aquí una vez y los formularios las siembran/leen al abrir/confirmar.
        let inputs_tareas = Some(crate::vista::tareas::nuevos_inputs(window, cx));
        // Input del composer de mensaje a peer (peers-02), mismo criterio.
        let input_mensaje_peer = Some(cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder("Escribe el mensaje…")
        }));
        let datos = EstadoPantalla {
            acceso_url: cliente.base().to_string(),
            acceso_token: cliente.token_enmascarado(),
            acceso_sin_token: cliente.sin_token(),
            panel_config,
            panel_acceso,
            panel_lanzador,
            inputs_tareas,
            input_mensaje_peer,
            ..EstadoPantalla::default()
        };
        // Arranca la carga inicial + el refresco periódico. El `cx` aquí es `Context<Self>` (viene
        // de `cx.new(|cx| AppDesktop::nueva(...))`), así que `cx.spawn` captura la entidad ya creada.
        Self::arrancar_refresco(cx);
        // Foco del contenedor raíz + enfoque inicial: habilita la captura de teclado desde el
        // primer frame (patrón de los ejemplos de gpui, p.ej. `tab_stop`).
        let foco = cx.focus_handle();
        window.focus(&foco, cx);
        Self {
            cliente,
            activa: Pantalla::Peers,
            datos,
            foco,
        }
    }

    /// Cambia la pantalla activa, dispara la carga de sus datos y solicita re-render. Es el único
    /// punto que muta la navegación. La carga inmediata (T4/R2) evita que el usuario vea la pantalla
    /// vacía hasta el siguiente tick del refresco periódico.
    fn ir_a(&mut self, pantalla: Pantalla, cx: &mut Context<Self>) {
        if self.activa != pantalla {
            self.activa = pantalla;
            self.cargar_pantalla_activa(cx);
            cx.notify();
        }
    }

    /// Navega a `pantalla` y FUERZA la recarga de sus datos, aunque ya sea la activa. INTENCIÓN:
    /// `ir_a` no recarga si la pantalla no cambia (evita refetch en clicks del sidebar), pero cuando
    /// cambia el FOCO dentro de la misma pantalla (p.ej. elegir otro peer en el selector de
    /// Trazabilidad, ya estando en Trazabilidad), sí hay que recargar. Este helper cubre ese caso.
    fn navegar_y_cargar(&mut self, pantalla: Pantalla, cx: &mut Context<Self>) {
        self.activa = pantalla;
        self.cargar_pantalla_activa(cx);
        cx.notify();
    }

    // --- Carga de datos del broker (feature desktop-carga-datos) ---
    // Cada `cargar_*` sigue el patrón verificado de `descartar_alerta`: clona el cliente, hace el
    // fetch en `cx.spawn` (no bloquea el hilo de UI) y muta `datos` al volver con `esta.update`.
    // El `Result` del cliente se maneja con match (nunca `.unwrap()`): Ok puebla la caché y limpia
    // el `error_*`; Err guarda el motivo para que la vista pinte el banner. `cx.notify()` repinta.

    /// Dispatcher: carga los datos de la pantalla ACTIVA. Lo llaman la carga inicial, el cambio de
    /// pantalla (`ir_a`) y el refresco periódico. Config no hace fetch (edita config local).
    fn cargar_pantalla_activa(&mut self, cx: &mut Context<Self>) {
        match self.activa {
            Pantalla::Peers => self.cargar_peers(cx),
            Pantalla::Alertas => self.cargar_alertas(cx),
            Pantalla::Broker | Pantalla::Acceso => self.cargar_broker(cx),
            Pantalla::Redis => self.cargar_redis(cx),
            Pantalla::Tareas => self.cargar_tareas(cx),
            Pantalla::Trazabilidad => self.cargar_trazabilidad(cx),
            Pantalla::Jornada => self.cargar_jornada(cx),
            // Config y Lanzador editan estado local (no consultan el broker en Fase 1).
            Pantalla::Config | Pantalla::Lanzador => {}
        }
    }

    /// Peers: `POST /listar` + `GET /admin/alertas` (la tabla cruza alertas por sujeto para la
    /// columna de estado ocioso/atascado). Ambas cachés se rellenan; el error va a `error_peers`.
    fn cargar_peers(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        // El fetch corre dentro del executor de fondo de GPUI (`background_executor().spawn`), que
        // devuelve una Task awaitable. Ahí `bloquear_en` entra al runtime tokio del cliente (reqwest
        // exige reactor tokio; el executor de GPUI no lo es). Al volver al `cx.spawn` (foreground)
        // se muta el estado con `esta.update`. Así ni el hilo de UI se bloquea ni reqwez paniquea.
        let fondo = cx.background_executor().spawn(async move {
            let peers = cliente.bloquear_en(cliente.listar_instancias());
            let alertas = cliente.bloquear_en(cliente.admin_alertas());
            (peers, alertas)
        });
        cx.spawn(async move |esta, cx| {
            let (peers, alertas) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                match peers {
                    Ok(lista) => {
                        esta.datos.instancias = lista;
                        esta.datos.error_peers = None;
                        // Acota la selección de peer a la nueva longitud: si la lista encogió y el
                        // índice quedó fuera de rango, se limpia (evita barra de acciones huérfana).
                        if let Some(i) = esta.datos.peers_seleccion {
                            if i >= esta.datos.instancias.len() {
                                esta.datos.peers_seleccion = None;
                            }
                        }
                        // Mismo criterio para el pop-up de detalle (peers-01): fuera de rango → cerrar.
                        if let Some(i) = esta.datos.peer_detalle {
                            if i >= esta.datos.instancias.len() {
                                esta.datos.peer_detalle = None;
                            }
                        }
                    }
                    Err(e) => esta.datos.error_peers = Some(e),
                }
                if let Ok(a) = alertas {
                    esta.datos.alertas = a;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Alertas: `GET /admin/alertas`. Acota la selección a la nueva longitud.
    fn cargar_alertas(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.admin_alertas()) });
        cx.spawn(async move |esta, cx| {
            let r = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                match r {
                    Ok(lista) => {
                        esta.datos.alertas = lista;
                        esta.datos.error_alertas = None;
                        // FIX (mismo bug de índice-visible-vs-total que mover_seleccion/tecla `d`):
                        // acotaba contra `alertas.len()` (TOTAL), pero `alertas_seleccion` es un
                        // índice sobre la lista VISIBLE post-filtro. Reusa el helper ya correcto.
                        esta.acotar_seleccion_alertas_visibles();
                    }
                    Err(e) => esta.datos.error_alertas = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Broker/Acceso: `GET /admin/info` + `GET /salud` + `GET /factor-estimacion`. Comparten estado
    /// (info/salud alimentan ambas pantallas). El error va a `error_acceso` (Broker lee los mismos
    /// campos; si están en None la vista pinta "sin datos").
    fn cargar_broker(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let fondo = cx.background_executor().spawn(async move {
            let info = cliente.bloquear_en(cliente.admin_info());
            let salud = cliente.bloquear_en(cliente.salud());
            let factor = cliente.bloquear_en(cliente.factor_estimacion());
            (info, salud, factor)
        });
        cx.spawn(async move |esta, cx| {
            let (info, salud, factor) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                // El primer error real (offline/401) se guarda para el banner; los Ok pueblan.
                let mut err = None;
                match info {
                    Ok(v) => esta.datos.info = Some(v),
                    Err(e) => err = Some(e),
                }
                match salud {
                    Ok(v) => esta.datos.salud = Some(v),
                    Err(e) => err = err.or(Some(e)),
                }
                if let Ok(v) = factor {
                    esta.datos.factor = Some(v);
                }
                esta.datos.error_acceso = err.clone();
                // acceso-06: inyecta el diagnóstico (o `None` si fue OK) en el panel de Acceso para
                // su banner tipado. El panel no habla con el broker; refleja lo que la app le pasa.
                if let Some(panel) = &esta.datos.panel_acceso {
                    panel.update(cx, |p, cx| p.set_diagnostico(err, cx));
                }
                // config-06: MISMO `info` que acaba de poblar la pestaña Broker, reflejado también
                // en el panel de Config (card "Broker conectado") — sin segunda llamada HTTP.
                if let Some(panel) = &esta.datos.panel_config {
                    let info = esta.datos.info.clone();
                    panel.update(cx, |p, cx| p.set_info(info, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Redis: `GET /admin/redis` (colas + outbox por peer). Error a `error_redis`.
    fn cargar_redis(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.admin_redis()) });
        cx.spawn(async move |esta, cx| {
            let r = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                match r {
                    Ok(v) => {
                        esta.datos.redis = Some(v);
                        esta.datos.error_redis = None;
                        // redis-05: timbra la frescura del snapshot (reloj monotónico local).
                        esta.datos.redis_actualizado_en = Some(std::time::Instant::now());
                    }
                    Err(e) => esta.datos.error_redis = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Tareas: `GET /admin/tareas` (todas las de todos los peers). Error a `error_tareas`.
    fn cargar_tareas(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.admin_tareas()) });
        cx.spawn(async move |esta, cx| {
            let r = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                match r {
                    Ok(lista) => {
                        esta.datos.tareas = lista;
                        esta.datos.error_tareas = None;
                        // Si el pop-up de detalle apunta fuera de la nueva lista, se cierra (mejor
                        // que dejarlo abierto sobre una tarea que ya no existe en esa posición).
                        if let Some(i) = esta.datos.tarea_detalle {
                            if i >= esta.datos.tareas.len() {
                                esta.datos.tarea_detalle = None;
                            }
                        }
                    }
                    Err(e) => esta.datos.error_tareas = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Trazabilidad: `GET /admin/historial` del peer en foco. Sin foco → no hay fetch (la vista
    /// pinta el texto guía "selecciona un peer"). El historial no tiene campo de error propio:
    /// un fallo deja el historial como estaba (mejor que vaciarlo).
    fn cargar_trazabilidad(&mut self, cx: &mut Context<Self>) {
        let Some(peer) = self.datos.traza_peer.clone() else {
            return;
        };
        // El filtro por estado (trazabilidad-02) viaja al broker: la criba la hace el backend.
        let filtro = self.datos.traza_filtro_estado;
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.historial(&peer, filtro)) });
        cx.spawn(async move |esta, cx| {
            let r = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Ok(lista) = r {
                    let n = lista.len();
                    esta.datos.historial = lista;
                    if n == 0 {
                        esta.datos.traza_seleccion = 0;
                    } else if esta.datos.traza_seleccion >= n {
                        esta.datos.traza_seleccion = n - 1;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Jornada: asegura el roster (`POST /listar`) para poder elegir el peer en foco y, si hay un
    /// peer enfocado (`jornada_peer`), carga su jornada detallada (`POST /jornada`) con sesiones y
    /// tareas timbradas por el broker. Sin foco no hay fetch de jornada (la vista pinta el texto
    /// guía "selecciona un peer"). Al recargar se acotan las selecciones de fila a las nuevas
    /// longitudes para no apuntar fuera de rango tras un cambio de tamaño.
    fn cargar_jornada(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let foco = self.datos.jornada_peer.clone();
        let fondo = cx.background_executor().spawn(async move {
            let peers = cliente.bloquear_en(cliente.listar_instancias());
            // Sólo pedimos la jornada si hay un peer enfocado; si no, `None` (nada que cargar).
            let jornada = foco
                .as_deref()
                .map(|id| cliente.bloquear_en(cliente.jornada(id)));
            // Bitácora del peer (registro-acciones R10), en el MISMO viaje de fondo.
            let acciones = foco
                .as_deref()
                .map(|id| cliente.bloquear_en(cliente.acciones(id)));
            (peers, jornada, acciones)
        });
        cx.spawn(async move |esta, cx| {
            let (peers, jornada, acciones) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Ok(lista) = peers {
                    esta.datos.instancias = lista;
                }
                // Bitácora: en éxito se refresca; en fallo (broker viejo sin /acciones, error
                // puntual) o sin foco se VACÍA — mejor sección vacía que el timeline de OTRO
                // peer colgado en pantalla. Sin banner: es observabilidad, no negocio.
                match acciones {
                    Some(Ok(a)) => esta.datos.jornada_acciones = a,
                    _ => esta.datos.jornada_acciones.clear(),
                }
                // Un fallo al cargar la jornada deja la caché como estaba (mejor que vaciarla).
                if let Some(Ok(j)) = jornada {
                    // Acota las selecciones de fila a las nuevas longitudes (defensivo).
                    if esta.datos.jornada_ses_sel >= j.sesiones.len() {
                        esta.datos.jornada_ses_sel = 0;
                    }
                    if esta.datos.jornada_tar_sel >= j.tareas.len() {
                        esta.datos.jornada_tar_sel = 0;
                    }
                    // Detalles abiertos fuera de rango → cerrar (la lista pudo encoger).
                    if let Some(i) = esta.datos.jornada_tarea_detalle {
                        if i >= j.tareas.len() {
                            esta.datos.jornada_tarea_detalle = None;
                        }
                    }
                    if let Some(i) = esta.datos.jornada_sesion_detalle {
                        if i >= j.sesiones.len() {
                            esta.datos.jornada_sesion_detalle = None;
                        }
                    }
                    esta.datos.jornada = Some(j);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Lanza la carga inicial (R1) + el refresco periódico de la pantalla activa (cada 2s, R5). Un
    /// solo timer para toda la app: sólo refresca lo que el usuario mira (sin martillar el broker).
    /// Se llama desde `nueva` (donde el `cx` es `Context<Self>` y permite `cx.spawn`). El primer
    /// tick es inmediato (sin esperar los 2s) para que la primera pantalla no arranque vacía. El
    /// loop se corta solo cuando la entidad muere (cierre de ventana) → `update` devuelve Err.
    fn arrancar_refresco(cx: &mut Context<Self>) {
        cx.spawn(async move |esta, cx| {
            // Carga inicial inmediata (R1): antes del primer sleep.
            if esta
                .update(cx, |esta, cx| esta.cargar_pantalla_activa(cx))
                .is_err()
            {
                return;
            }
            loop {
                // Timer del executor de GPUI (no depende de un runtime tokio en este hilo).
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                if esta
                    .update(cx, |esta, cx| esta.cargar_pantalla_activa(cx))
                    .is_err()
                {
                    break; // la entidad ya no existe: la ventana se cerró.
                }
            }
        })
        .detach();
    }

    // --- Manejadores de la pantalla Alertas ---
    // La vista es pura y despacha acciones; aquí se mutan `datos` y se dispara la escritura al
    // broker. Se registran con `.on_action(cx.listener(...))` en el contenedor raíz de `render`.

    /// Traduce un índice de la lista VISIBLE de alertas (el que viaja en las acciones de fila, tras
    /// aplicar el filtro por tipo) al índice REAL en `datos.alertas`. Reconstruye el mismo filtrado
    /// que la vista (`pasa_filtro`) y toma el n-ésimo que pasa. `None` si el índice visible ya no
    /// existe (la lista cambió entre el pintado y el click). Centraliza el mapeo para las acciones
    /// de alertas que llegan con índice visible (abrir detalle, descarte rápido de fila).
    fn indice_real_alerta(&self, indice_visible: usize) -> Option<usize> {
        crate::vista::alertas::indice_real(
            &self.datos.alertas,
            &self.datos.alertas_filtro_tipos,
            indice_visible,
        )
    }

    /// Acota `alertas_seleccion` al número de alertas VISIBLES (post-filtro), saturando al último.
    /// El cursor de selección se indexa sobre la lista visible (igual que las filas pintadas).
    fn acotar_seleccion_alertas_visibles(&mut self) {
        use crate::vista::alertas::pasa_filtro;
        let visibles = self
            .datos
            .alertas
            .iter()
            .filter(|a| pasa_filtro(a, &self.datos.alertas_filtro_tipos))
            .count();
        if visibles == 0 {
            self.datos.alertas_seleccion = 0;
        } else if self.datos.alertas_seleccion >= visibles {
            self.datos.alertas_seleccion = visibles - 1;
        }
    }

    /// Abre el MODAL de detalle (alertas-01) de la fila pulsada y la marca como seleccionada. El
    /// `indice` llega en coordenadas VISIBLES (post-filtro); se traduce a real para que el modal
    /// —que `AppDesktop` monta leyendo `datos.alertas[real]`— apunte a la alerta correcta aunque
    /// haya filtro activo. Resetea la confirmación de descarte (modal recién abierto = paso 1).
    fn abrir_detalle_alerta(&mut self, indice: usize, cx: &mut Context<Self>) {
        let Some(real) = self.indice_real_alerta(indice) else {
            return; // el índice visible ya no existe (la lista cambió): no abrir nada
        };
        self.datos.alertas_seleccion = indice;
        self.datos.alerta_detalle = Some(real);
        self.datos.alerta_confirmar_descarte = false;
        cx.notify();
    }

    /// Cierra el modal de detalle y resetea la confirmación de descarte (deja la selección donde
    /// estaba). Es el destino de `CerrarDetalleAlerta`, del ✕, del clic fuera y de `Esc`.
    fn cerrar_detalle_alerta(&mut self, cx: &mut Context<Self>) {
        if self.datos.alerta_detalle.is_some() || self.datos.alerta_confirmar_descarte {
            self.datos.alerta_detalle = None;
            self.datos.alerta_confirmar_descarte = false;
            cx.notify();
        }
    }

    /// Alterna un tipo en el filtro por tipo (alertas-03). `tipo` es la cadena serializada; se
    /// mete/saca del conjunto `alertas_filtro_tipos` (multi-select, unión). Acota la selección de
    /// fila a la nueva lista visible para que el cursor no apunte fuera de rango tras filtrar.
    fn alternar_filtro_tipo(&mut self, tipo: String, cx: &mut Context<Self>) {
        if let Some(pos) = self.datos.alertas_filtro_tipos.iter().position(|t| *t == tipo) {
            self.datos.alertas_filtro_tipos.remove(pos);
        } else {
            self.datos.alertas_filtro_tipos.push(tipo);
        }
        self.acotar_seleccion_alertas_visibles();
        cx.notify();
    }

    /// Limpia todos los filtros de tipo (chip "Todos"): vuelve a mostrar TODAS las alertas.
    fn limpiar_filtro_tipos(&mut self, cx: &mut Context<Self>) {
        if !self.datos.alertas_filtro_tipos.is_empty() {
            self.datos.alertas_filtro_tipos.clear();
            self.acotar_seleccion_alertas_visibles();
            cx.notify();
        }
    }

    /// Paso 1 → 2 de la confirmación de descarte (alertas-09): el botón "Descartar" del modal no
    /// resuelve directo, pide confirmación. Sólo tiene efecto con el modal abierto.
    fn pedir_confirmacion_descarte(&mut self, cx: &mut Context<Self>) {
        if self.datos.alerta_detalle.is_some() {
            self.datos.alerta_confirmar_descarte = true;
            cx.notify();
        }
    }

    /// Cancela la confirmación de descarte y vuelve al modal en su estado normal (paso 2 → 1).
    fn cancelar_confirmacion_descarte(&mut self, cx: &mut Context<Self>) {
        if self.datos.alerta_confirmar_descarte {
            self.datos.alerta_confirmar_descarte = false;
            cx.notify();
        }
    }

    /// "Ir al sujeto" (alertas-02, portando la tecla `g` de la TUI): cierra el modal y navega a la
    /// pantalla natural del sujeto según el tipo, preseleccionando/enfocando por `sujeto`:
    ///   - Ocioso / CancelaciónExcesiva → Peers (marca la fila del peer `sujeto`).
    ///   - Atascado / CierreSospechoso  → Tareas (marca la tarea `sujeto`, o la 1ª de ese peer).
    ///   - Ghosteo                      → Trazabilidad (enfoca el peer `sujeto` y carga su historial).
    /// Si el sujeto no se encuentra en la pantalla destino, navega igualmente (la recarga lo mostrará).
    /// `tipo` llega serializado; una cadena desconocida se ignora (no navega).
    fn ir_al_sujeto_alerta(&mut self, tipo: String, sujeto: String, cx: &mut Context<Self>) {
        use crate::vista::alertas::tipo_desde_serializado;
        let Some(tipo) = tipo_desde_serializado(&tipo) else {
            return;
        };
        // Cierra el modal antes de saltar (el foco se va a otra pantalla).
        self.datos.alerta_detalle = None;
        self.datos.alerta_confirmar_descarte = false;

        match tipo {
            peers_core::TipoAlerta::Ocioso | peers_core::TipoAlerta::CancelacionExcesiva => {
                // Peers: preselecciona la fila cuyo `id` coincide con el sujeto (si está en el roster).
                if let Some(pos) = self.datos.instancias.iter().position(|i| i.id == sujeto) {
                    self.datos.peers_seleccion = Some(pos);
                }
                self.ir_a(Pantalla::Peers, cx);
            }
            peers_core::TipoAlerta::Atascado | peers_core::TipoAlerta::CierreSospechoso => {
                // Tareas: el sujeto de estas alertas es el id de la tarea; si no casa, se prueba como
                // dueño (instancia_id) para al menos enfocar una tarea de ese peer.
                let pos = self
                    .datos
                    .tareas
                    .iter()
                    .position(|t| t.id == sujeto)
                    .or_else(|| self.datos.tareas.iter().position(|t| t.instancia_id == sujeto));
                if let Some(pos) = pos {
                    self.datos.tareas_seleccion = pos;
                }
                self.ir_a(Pantalla::Tareas, cx);
            }
            peers_core::TipoAlerta::Ghosteo => {
                // Trazabilidad: el sujeto es el peer cuyo mensaje se ghosteó. `enfocar_traza` fija el
                // foco, navega y dispara la carga del historial (reutiliza el manejador de Peers).
                self.enfocar_traza(sujeto, cx);
            }
        }
    }

    /// Descarta una alerta: `POST /admin/alerta-resolver` y, si va bien, recarga la lista + emite un
    /// toast de éxito/fallo (alertas-09). La escritura corre en un `cx.spawn` (no bloquea el hilo de
    /// UI); al volver se re-lee `admin_alertas` para que la tabla refleje el descarte sin depender
    /// del refresco periódico. Cualquier fallo se guarda en `error_alertas` (banner) y en el toast;
    /// nunca hace panic. El `Notification` se empuja a la ventana activa desde el spawn vía su
    /// `AnyWindowHandle` (capturado de `cx.active_window()`), ya que allí no hay `&mut Window` directo.
    fn descartar_alerta(&mut self, tipo: String, sujeto: String, cx: &mut Context<Self>) {
        // Cierra el modal/confirmación de inmediato: la acción ya está en marcha, la UI no debe
        // quedar abierta sobre una alerta que va a desaparecer. La recarga posterior repinta la tabla.
        self.datos.alerta_detalle = None;
        self.datos.alerta_confirmar_descarte = false;
        cx.notify();

        // Handle de la ventana activa (Copy) para empujar el toast desde el spawn async. `Context`
        // deref a `App`, que expone `active_window()`. Si no hubiera ventana activa, no hay toast.
        let ventana = cx.active_window();
        let sujeto_toast = sujeto.clone();

        let cliente = self.cliente.clone();
        // Red en el hilo de FONDO con `bloquear_en` (runtime tokio del cliente): el `.await`
        // directo en `cx.spawn` abortaba con "no reactor running" (fix transversal del crash).
        let fondo = cx.background_executor().spawn(async move {
            let resultado = cliente.bloquear_en(cliente.alerta_resolver(&tipo, &sujeto));
            // Tras descartar, re-lee la lista para reflejar el estado real del broker.
            let recarga = match &resultado {
                Ok(_) => Some(cliente.bloquear_en(cliente.admin_alertas())),
                Err(_) => None,
            };
            (resultado, recarga)
        });
        cx.spawn(async move |esta, cx| {
            let (resultado, recarga) = fondo.await;
            // Éxito/motivo para el toast (se leen antes de mover `resultado` al match).
            let ok = resultado.is_ok();
            let motivo_fallo = resultado.as_ref().err().map(|e| e.to_string());
            // Vuelve al hilo de la entidad para mutar el estado con el resultado.
            let _ = esta.update(cx, |esta, cx| {
                match resultado {
                    Ok(_) => {
                        esta.datos.error_alertas = None;
                        if let Some(Ok(lista)) = recarga {
                            esta.datos.alertas = lista;
                            esta.acotar_seleccion_alertas_visibles();
                        } else if let Some(Err(e)) = recarga {
                            // El descarte fue OK pero la recarga falló: informa sin romper la lista.
                            esta.datos.error_alertas = Some(e);
                        }
                    }
                    Err(e) => {
                        // El descarte falló (offline/401/otro): muestra el motivo, conserva la lista.
                        esta.datos.error_alertas = Some(e);
                    }
                }
                cx.notify();
            });
            // Toast de feedback (alertas-09): se empuja a través del handle de la ventana, que da un
            // `&mut Window` + `&mut App` aun estando en el spawn. Si la ventana ya se cerró, `update`
            // devuelve Err y se ignora (no hay a quién notificar).
            if let Some(ventana) = ventana {
                let _ = ventana.update(cx, |_raiz, win, app| {
                    use gpui_component::notification::Notification;
                    use gpui_component::WindowExt as _;
                    let note = if ok {
                        Notification::success(format!("Alerta «{sujeto_toast}» descartada"))
                    } else {
                        let motivo =
                            motivo_fallo.unwrap_or_else(|| "error desconocido".to_string());
                        Notification::error(format!("No se pudo descartar: {motivo}"))
                    };
                    win.push_notification(note, app);
                });
            }
        })
        .detach();
    }

    // --- Manejadores de la pantalla Acceso (acceso-01/02/05) ---
    // La vista/panel despacha estas acciones; aquí se muta la config, se reconstruye el cliente y se
    // dispara la recarga o el check. El cliente vive SÓLO aquí, de ahí que estas acciones existan.

    /// Aplica una nueva conexión (acceso-01 + acceso-02): persiste `broker_url`/`token` en
    /// `config.toml` (0600), reconfigura el `ClienteBroker` EN CALIENTE y recarga el estado contra el
    /// nuevo destino. `token` vacío → `None` (broker sin token, modo anónimo). No hace `.unwrap()`:
    /// un fallo al guardar el TOML va al banner (`error_acceso`) sin abortar la reconexión (la
    /// conexión en caliente igual se aplica; sólo no queda persistida).
    fn aplicar_conexion(
        &mut self,
        broker_url: String,
        token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Normaliza la URL (sin barra final) para consistencia entre config, cliente y lo que se pinta.
        let broker_url = broker_url.trim().trim_end_matches('/').to_string();
        // Token vacío = sin token (no se persiste una cadena vacía; el broker corre anónimo).
        let token_opt = if token.trim().is_empty() {
            None
        } else {
            Some(token.trim().to_string())
        };

        // 1) Persistir en config.toml (permisos 0600). Se conserva `refresh_ms` actual del disco para
        //    no pisarlo (Acceso sólo gestiona url/token; el refresco lo lleva Config).
        let mut cfg = crate::config::Config::cargar().unwrap_or_default();
        cfg.broker_url = broker_url.clone();
        cfg.token = token_opt.clone();
        if let Err(e) = cfg.guardar() {
            // No abortamos: reconfiguramos igual en caliente; sólo avisamos que no quedó persistido.
            self.datos.error_acceso = Some(crate::cliente::ErrorBroker::Otro(format!(
                "conexión aplicada, pero no se pudo guardar config.toml: {e}"
            )));
        }

        // 2) Reconfigurar el cliente EN CALIENTE (reusa runtime tokio + pool reqwest).
        self.cliente.reconfigurar(broker_url, token_opt);
        // Refresca la copia cacheada que la vista pura muestra (url + token enmascarado + flag anónimo).
        self.datos.acceso_url = self.cliente.base().to_string();
        self.datos.acceso_token = self.cliente.token_enmascarado();
        self.datos.acceso_sin_token = self.cliente.sin_token();

        // 3) Re-sembrar los Inputs del panel desde el disco: refleja la URL ya NORMALIZADA (sin barra
        //    final, token trim) en los campos editables, para que lo que ve el jefe coincida con lo
        //    persistido y con el destino real del cliente (evita "escribí http://x/ pero se guardó
        //    http://x"). Sólo si el panel existe; nunca `.unwrap()`.
        if let Some(panel) = &self.datos.panel_acceso {
            panel.update(cx, |p, cx| p.resembrar_desde_disco(window, cx));
        }

        // 4) Recargar el estado contra el nuevo broker (poblará info/salud + diagnóstico en el panel).
        self.cargar_broker(cx);

        // acceso-13: registra el evento "Aplicar" en el historial local. `cargar_broker` es
        // fire-and-forget (no expone su resultado síncronamente), así que se hace un `GET /salud`
        // propio y LIGERO —igual que el primer paso de `probar_conexion`— sólo para saber si
        // "aplicar" terminó viendo el broker vivo o no; el resultado se pasa YA CALCULADO a
        // `registrar_evento_acceso` (que no repite ninguna llamada de red). Se lee `self.cliente.base()`
        // (ya normalizada tras `reconfigurar`) en vez de la variable `broker_url` local, que
        // `reconfigurar` movió en el paso 2.
        let cliente = self.cliente.clone();
        let broker_url_evento = self.cliente.base().to_string();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.salud()) });
        cx.spawn(async move |esta, cx| {
            let salud = fondo.await;
            let (ok, detalle) = match salud {
                Ok(s) => (true, format!("{} · {} instancias", s.estado, s.instancias)),
                Err(e) => (false, e.to_string()),
            };
            let _ = esta.update(cx, |esta, cx| {
                esta.registrar_evento_acceso(
                    crate::config::TipoEventoConexion::Aplicar,
                    broker_url_evento,
                    ok,
                    detalle,
                    cx,
                );
            });
        })
        .detach();
    }

    /// Persiste un `EventoConexion` en `config.toml` (acceso-13) con un resultado YA CALCULADO por
    /// el caller (no repite ninguna llamada de red: `probar_conexion` ya hizo su propio `/salud` y
    /// `aplicar_conexion` el suyo; duplicar el check aquí habría significado un segundo hit de red
    /// por acción y, peor, un resultado que podría diferir del que el jefe ya vio en pantalla).
    /// Refresca el timeline del panel tras guardar. Se ejecuta en el `background_executor` (la
    /// escritura a disco es síncrona pero barata; no vale la pena bloquear el hilo de UI).
    fn registrar_evento_acceso(
        &mut self,
        tipo: crate::config::TipoEventoConexion,
        broker_url: String,
        ok: bool,
        detalle: String,
        cx: &mut Context<Self>,
    ) {
        let fondo = cx.background_executor().spawn(async move {
            // `time` ya es dependencia (usado por alertas para `creada_en`/antigüedad relativa);
            // se reusa el mismo formato RFC 3339 UTC en vez de introducir otra convención de hora.
            use time::format_description::well_known::Rfc3339;
            let cuando = time::OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_else(|_| String::new());

            // Cargar+mutar+guardar puede perder una escritura concurrente rara vez (dos eventos
            // casi simultáneos), aceptable para un historial de auditoría local, no un contador
            // crítico — mismo criterio de tolerancia que el resto de `Config`.
            let mut cfg = crate::config::Config::cargar().unwrap_or_default();
            cfg.acceso.registrar_evento(crate::config::EventoConexion {
                cuando,
                tipo,
                broker_url,
                ok,
                detalle,
            });
            let guardado = cfg.guardar();
            (guardado, cfg.acceso.historial)
        });
        cx.spawn(async move |esta, cx| {
            let (guardado, historial) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Err(e) = guardado {
                    eprintln!("acceso-13: no se pudo persistir el historial de conexión: {e}");
                }
                if let Some(panel) = &esta.datos.panel_acceso {
                    panel.update(cx, |p, cx| p.set_historial(historial, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Probar conexión" (acceso-05 y config-01): check DEDICADO y ligero, SIN recargar el resto.
    /// Dos pasos independientes para separar "broker vivo" de "token válido":
    ///   ① `GET /salud`  (ruta EXENTA de token) → ¿alcanzable y vivo?
    ///   ② `POST /listar` (ruta PROTEGIDA)       → ¿el token autentica? (401 = rechazado)
    /// El resultado se inyecta en el panel que lo pidió (`hacia_config` decide: PanelConfig o
    /// PanelAcceso) con `set_resultado_prueba` — MISMO check, dos consumidores. Nunca `.unwrap()`.
    fn probar_conexion(&mut self, hacia_config: bool, cx: &mut Context<Self>) {
        use crate::vista::acceso::ResultadoPrueba;
        let cliente = self.cliente.clone();
        // El check corre en el executor de fondo (reqwest necesita su runtime tokio, no el de GPUI).
        let fondo = cx.background_executor().spawn(async move {
            // acceso-12: RTT medido AQUÍ (dentro del executor de fondo, alrededor de la llamada
            // bloqueante real) para no incluir el overhead de scheduling entre el background_executor
            // y el cx.spawn async — eso mediría "cuánto tarda GPUI en despertar", no la red.
            let inicio = std::time::Instant::now();
            let salud = cliente.bloquear_en(cliente.salud());
            let latencia_ms = if salud.is_ok() {
                Some(inicio.elapsed().as_millis() as u64)
            } else {
                // Offline/error: el tiempo transcurrido es el del timeout/rechazo, no un RTT real.
                None
            };
            // Sólo probamos el token si el broker respondió a /salud (si está caído, el 2º paso no
            // aporta: no hay a quién autenticar). `None` = no se llegó a probar.
            let auth = if salud.is_ok() {
                Some(cliente.bloquear_en(cliente.listar_instancias()))
            } else {
                None
            };
            (salud, auth, latencia_ms)
        });
        cx.spawn(async move |esta, cx| {
            let (salud, auth, latencia_ms) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                let salud_ok = salud.is_ok();
                // Distingue token válido (200) de rechazado (401) de "no probado" (broker caído).
                let token_ok = match &auth {
                    Some(Ok(_)) => Some(true),
                    Some(Err(crate::cliente::ErrorBroker::NoAutorizado)) => Some(false),
                    Some(Err(_)) => None, // otro error tras salud ok: no concluyente sobre el token
                    None => None,         // no se probó (broker no respondió a /salud)
                };
                // Resumen legible del intento para el pie del bloque de prueba.
                let resumen = match (salud_ok, token_ok) {
                    (false, _) => match &salud {
                        Err(e) => format!("broker inalcanzable: {e}"),
                        Ok(_) => "broker inalcanzable".to_string(),
                    },
                    (true, Some(true)) => "broker vivo · token válido".to_string(),
                    (true, Some(false)) => "broker vivo · token RECHAZADO (401)".to_string(),
                    (true, None) => "broker vivo · validación de token no concluyente".to_string(),
                };
                let resultado = ResultadoPrueba {
                    salud_ok: Some(salud_ok),
                    token_ok,
                    resumen,
                    en_curso: false,
                    latencia_ms,
                };
                // El resultado vuelve al panel que pidió la prueba (config-01 vs acceso-05).
                if hacia_config {
                    if let Some(panel) = &esta.datos.panel_config {
                        panel.update(cx, |p, cx| p.set_resultado_prueba(resultado.clone(), cx));
                    }
                } else if let Some(panel) = &esta.datos.panel_acceso {
                    panel.update(cx, |p, cx| p.set_resultado_prueba(resultado.clone(), cx));
                }
                cx.notify();

                // acceso-13: registra el evento "Probar" en el historial local (sólo desde la
                // pestaña Acceso — config-01 comparte el MISMO check pero su propio historial no
                // está en el alcance de esta RFC; evita mezclar el timeline de Acceso con acciones
                // disparadas desde Config). Reusa `salud_ok`/`resumen` YA calculados: sin 2º hit.
                if !hacia_config {
                    esta.registrar_evento_acceso(
                        crate::config::TipoEventoConexion::Probar,
                        esta.cliente.base().to_string(),
                        resultado.salud_ok.unwrap_or(false),
                        resultado.resumen.clone(),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    // --- Manejadores de la pantalla Peers ---
    // La barra de acciones del peer marcado despacha estas acciones; aquí se mutan `datos` y, en el
    // caso de "ver jornada", se dispara un fetch (jornada del peer) + navegación a esa pantalla.

    /// Marca el peer de la fila `indice` (o lo limpia si el índice quedó fuera de rango tras una
    /// recarga). Sólo muta estado local; habilita la barra de acciones del peer.
    fn seleccionar_peer(&mut self, indice: usize, cx: &mut Context<Self>) {
        self.datos.peers_seleccion = if indice < self.datos.instancias.len() {
            Some(indice)
        } else {
            None
        };
        cx.notify();
    }

    /// Suelta la selección de peer (botón "Cerrar" de la barra de acciones).
    fn deseleccionar_peer(&mut self, cx: &mut Context<Self>) {
        self.datos.peers_seleccion = None;
        cx.notify();
    }

    /// "Ver jornada" del peer `id`: fija el foco de jornada, resetea las selecciones de fila de esa
    /// pantalla, NAVEGA a Jornada y dispara la carga (que ahora sí pide `POST /jornada`). Reutiliza
    /// `ir_a`, que ya llama a `cargar_pantalla_activa` (→ `cargar_jornada`) al cambiar de pantalla.
    fn ver_jornada_peer(&mut self, id: String, cx: &mut Context<Self>) {
        self.datos.jornada_peer = Some(id);
        self.datos.jornada = None; // limpia la jornada previa para no mostrar la de otro peer
        self.datos.jornada_ses_sel = 0;
        self.datos.jornada_tar_sel = 0;
        // `navegar_y_cargar` (no `ir_a`): fuerza el fetch aunque ya estuviéramos en Jornada, para
        // reflejar el peer recién elegido en vez de dejar la jornada anterior/ vacía.
        self.navegar_y_cargar(Pantalla::Jornada, cx);
    }

    /// "Enviar mensaje" al peer `id` (peers-02): abre el COMPOSER real (Input + Para: <id>). El
    /// comportamiento anterior (saltar a Trazabilidad) era un apaño hasta tener composición; ahora
    /// el cliente expone `enviar` (`POST /enviar`) y el flujo es el de la TUI (tecla `m`).
    fn enviar_mensaje_peer(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.abrir_form_peers(crate::vista::peers::FormPeers::Mensaje { id }, window, cx);
    }

    // --- Detalle y formularios de la pestaña Peers (peers-01/02/03) ---
    // Mismo esquema que Tareas: un pop-up de detalle (`peer_detalle`) + un único formulario a la
    // vez (`peers_form`), montados como overlays por el render raíz.

    /// Abre el pop-up de detalle (peers-01) del peer `indice` y lo marca como seleccionado.
    /// Refresca la caché de tareas en segundo plano para que la métrica "N tarea(s) abierta(s)"
    /// del detalle no esté rancia (en esta pantalla `tareas` no se recarga sola).
    fn abrir_detalle_peer(&mut self, indice: usize, cx: &mut Context<Self>) {
        if indice >= self.datos.instancias.len() {
            return;
        }
        self.datos.peers_seleccion = Some(indice);
        self.datos.peer_detalle = Some(indice);
        self.cargar_tareas(cx);
        cx.notify();
    }

    /// Cierra el pop-up de detalle del peer (✕, Cerrar, clic fuera, Esc).
    fn cerrar_detalle_peer(&mut self, cx: &mut Context<Self>) {
        if self.datos.peer_detalle.is_some() {
            self.datos.peer_detalle = None;
            cx.notify();
        }
    }

    /// Abre un formulario de Peers (composer de mensaje o confirmación de kick). Siembra el Input
    /// del mensaje VACÍO (no hereda texto de un envío anterior), limpia el error y cierra el
    /// detalle (un solo overlay a la vez, coherente con Tareas).
    fn abrir_form_peers(
        &mut self,
        form: crate::vista::peers::FormPeers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = &self.datos.input_mensaje_peer {
            input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
        }
        self.datos.peer_detalle = None;
        self.datos.peers_form_error = None;
        self.datos.peers_form = Some(form);
        cx.notify();
    }

    /// Cierra el formulario de Peers sin confirmar (Cancelar, ✕, clic fuera, Esc).
    fn cerrar_form_peers(&mut self, cx: &mut Context<Self>) {
        if self.datos.peers_form.is_some() {
            self.datos.peers_form = None;
            self.datos.peers_form_error = None;
            cx.notify();
        }
    }

    /// Confirma el formulario de Peers activo: valida la frontera y dispara el POST vía
    /// `mutar_peer` (red en fondo + recarga del roster + toast). Mensaje vacío → error en el
    /// modal (sigue abierto para corregir).
    fn confirmar_form_peers(&mut self, cx: &mut Context<Self>) {
        use crate::vista::peers::FormPeers;
        let Some(form) = self.datos.peers_form.clone() else {
            return;
        };
        match form {
            FormPeers::Mensaje { id } => {
                let texto = self
                    .datos
                    .input_mensaje_peer
                    .as_ref()
                    .map(|i| i.read(cx).value().trim().to_string())
                    .unwrap_or_default();
                if texto.is_empty() {
                    self.datos.peers_form_error = Some("el mensaje no puede ir vacío".to_string());
                    cx.notify();
                    return;
                }
                self.cerrar_form_peers(cx);
                let exito = format!("Mensaje enviado a {id}");
                self.mutar_peer(
                    exito,
                    move |c| c.bloquear_en(c.enviar(&id, &texto)).map(|_| ()),
                    cx,
                );
            }
            FormPeers::Kick { id } => {
                self.cerrar_form_peers(cx);
                // Si el peer expulsado estaba seleccionado, suelta la selección (va a desaparecer).
                self.datos.peers_seleccion = None;
                let exito = format!("Peer «{id}» expulsado");
                self.mutar_peer(
                    exito,
                    move |c| c.bloquear_en(c.salir(&id)).map(|_| ()),
                    cx,
                );
            }
        }
    }

    /// Ejecuta una MUTACIÓN de peer en segundo plano y aplica el post-proceso común: recarga el
    /// roster (`POST /listar`) si fue OK, guarda el error en `error_peers` si falló, y emite un
    /// toast de éxito/fallo. Análogo exacto de `mutar_tarea` (red SIEMPRE vía `bloquear_en` en el
    /// executor de fondo — el `.await` directo en `cx.spawn` era el crash SIGABRT).
    fn mutar_peer(
        &mut self,
        exito: String,
        hacer: impl FnOnce(&ClienteBroker) -> Result<(), crate::cliente::ErrorBroker> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let ventana = cx.active_window();
        let cliente = self.cliente.clone();
        let fondo = cx.background_executor().spawn(async move {
            let resultado = hacer(&cliente);
            let recarga = match &resultado {
                Ok(_) => Some(cliente.bloquear_en(cliente.listar_instancias())),
                Err(_) => None,
            };
            (resultado, recarga)
        });
        cx.spawn(async move |esta, cx| {
            let (resultado, recarga) = fondo.await;
            let ok = resultado.is_ok();
            let motivo_fallo = resultado.as_ref().err().map(|e| e.to_string());
            let _ = esta.update(cx, |esta, cx| {
                match resultado {
                    Ok(_) => {
                        esta.datos.error_peers = None;
                        if let Some(Ok(lista)) = recarga {
                            esta.datos.instancias = lista;
                            // Acota selección y detalle a la nueva longitud del roster.
                            let n = esta.datos.instancias.len();
                            if let Some(i) = esta.datos.peers_seleccion {
                                if i >= n {
                                    esta.datos.peers_seleccion = None;
                                }
                            }
                            if let Some(i) = esta.datos.peer_detalle {
                                if i >= n {
                                    esta.datos.peer_detalle = None;
                                }
                            }
                        } else if let Some(Err(e)) = recarga {
                            esta.datos.error_peers = Some(e);
                        }
                    }
                    Err(e) => esta.datos.error_peers = Some(e),
                }
                cx.notify();
            });
            if let Some(ventana) = ventana {
                let _ = ventana.update(cx, |_raiz, win, app| {
                    use gpui_component::notification::Notification;
                    use gpui_component::WindowExt as _;
                    let note = if ok {
                        Notification::success(exito.clone())
                    } else {
                        let motivo =
                            motivo_fallo.unwrap_or_else(|| "error desconocido".to_string());
                        Notification::error(format!("No se pudo aplicar: {motivo}"))
                    };
                    win.push_notification(note, app);
                });
            }
        })
        .detach();
    }

    // --- Manejadores de la pantalla Trazabilidad ---

    /// Enfoca un peer en Trazabilidad: fija `traza_peer`, resetea selección/timeline, navega a la
    /// pantalla y dispara la carga del historial (vía `ir_a` → `cargar_trazabilidad`).
    fn enfocar_traza(&mut self, id: String, cx: &mut Context<Self>) {
        self.datos.traza_peer = Some(id);
        self.datos.traza_seleccion = 0;
        self.datos.traza_timeline = false;
        self.datos.historial.clear(); // evita mostrar el historial del peer anterior mientras carga
        // `navegar_y_cargar` (no `ir_a`): el selector de peer de Trazabilidad se usa ESTANDO ya en
        // Trazabilidad; con `ir_a` (que no recarga si la pantalla no cambia) el historial no se
        // refrescaría al cambiar de peer. Aquí forzamos el fetch del nuevo peer.
        self.navegar_y_cargar(Pantalla::Trazabilidad, cx);
    }

    /// Selecciona la fila `indice` del historial (click simple). Desde trazabilidad-01 SOLO mueve
    /// el cursor: el modal de timeline se abre con doble-click/Enter (`abrir_mensaje`).
    fn seleccionar_mensaje(&mut self, indice: usize, cx: &mut Context<Self>) {
        if indice >= self.datos.historial.len() {
            return;
        }
        self.datos.traza_seleccion = indice;
        cx.notify();
    }

    /// Abre el MODAL de timeline (trazabilidad-01) del mensaje `indice` y lo marca seleccionado.
    fn abrir_mensaje(&mut self, indice: usize, cx: &mut Context<Self>) {
        if indice >= self.datos.historial.len() {
            return;
        }
        self.datos.traza_seleccion = indice;
        self.datos.traza_timeline = true;
        cx.notify();
    }

    /// Cierra el modal de timeline (✕, Cerrar, clic fuera, Esc).
    fn cerrar_timeline(&mut self, cx: &mut Context<Self>) {
        if self.datos.traza_timeline {
            self.datos.traza_timeline = false;
            cx.notify();
        }
    }

    /// Abre el mini-modal de CONFIRMACIÓN de reenvío (trazabilidad-05) para `msg_id`.
    fn pedir_reenvio(&mut self, msg_id: i64, cx: &mut Context<Self>) {
        self.datos.traza_confirmar_reenvio = Some(msg_id);
        cx.notify();
    }

    /// Cancela la confirmación de reenvío pendiente (Cancelar / clic fuera / Esc).
    fn cancelar_reenvio(&mut self, cx: &mut Context<Self>) {
        if self.datos.traza_confirmar_reenvio.is_some() {
            self.datos.traza_confirmar_reenvio = None;
            cx.notify();
        }
    }

    /// Confirma el reenvío pendiente: cierra la confirmación y dispara el POST real.
    fn confirmar_reenvio(&mut self, cx: &mut Context<Self>) {
        let Some(msg_id) = self.datos.traza_confirmar_reenvio.take() else {
            return;
        };
        cx.notify();
        self.reenviar_mensaje(msg_id, cx);
    }

    /// Reenvío disparado desde el pop-up de cola de Redis (redis-03): mismo POST que el flujo de
    /// Trazabilidad, pero la recarga es la de la COLA inspeccionada (`estado=enviado`) + el
    /// snapshot de contadores (`/admin/redis`). Toast tipado igual que allí. Red en fondo con
    /// `bloquear_en`, sin `.unwrap()`.
    fn reenviar_desde_redis(&mut self, msg_id: i64, peer: String, cx: &mut Context<Self>) {
        let ventana = cx.active_window();
        let cliente = self.cliente.clone();
        let fondo = cx.background_executor().spawn(async move {
            let resultado = cliente.bloquear_en(cliente.reenviar(msg_id));
            let recarga = match &resultado {
                Ok(r) if r.ok => Some((
                    cliente.bloquear_en(
                        cliente.historial(&peer, Some(peers_core::EstadoMensaje::Enviado)),
                    ),
                    cliente.bloquear_en(cliente.admin_redis()),
                )),
                _ => None,
            };
            (resultado, recarga)
        });
        cx.spawn(async move |esta, cx| {
            let (resultado, recarga) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Some((cola, snapshot)) = recarga {
                    if let Ok(lista) = cola {
                        esta.datos.redis_cola_mensajes = lista;
                    }
                    if let Ok(v) = snapshot {
                        esta.datos.redis = Some(v);
                        esta.datos.redis_actualizado_en = Some(std::time::Instant::now());
                    }
                }
                cx.notify();
            });
            if let Some(ventana) = ventana {
                let _ = ventana.update(cx, |_raiz, win, app| {
                    use gpui_component::notification::Notification;
                    use gpui_component::WindowExt as _;
                    let note = match &resultado {
                        Ok(r) if r.ok => match r.msg_id {
                            Some(nuevo) => Notification::success(format!(
                                "Mensaje #{msg_id} reenviado como #{nuevo}"
                            )),
                            None => Notification::success(format!("Mensaje #{msg_id} reenviado")),
                        },
                        Ok(r) => {
                            let motivo = r
                                .error
                                .clone()
                                .unwrap_or_else(|| "el broker lo rechazó".to_string());
                            Notification::error(format!("No se pudo reenviar: {motivo}"))
                        }
                        Err(e) => Notification::error(format!("No se pudo reenviar: {e}")),
                    };
                    win.push_notification(note, app);
                });
            }
        })
        .detach();
    }

    /// Cambia el filtro por estado del historial (trazabilidad-02) y recarga contra el broker.
    /// `None` = "Todos". Resetea el cursor de fila (la lista cambia de composición).
    fn filtrar_estado_traza(
        &mut self,
        estado: Option<peers_core::EstadoMensaje>,
        cx: &mut Context<Self>,
    ) {
        if self.datos.traza_filtro_estado == estado {
            return; // chip ya activo: nada que recargar
        }
        self.datos.traza_filtro_estado = estado;
        self.datos.traza_seleccion = 0;
        self.datos.traza_timeline = false;
        self.cargar_trazabilidad(cx);
        cx.notify();
    }

    /// Reenvía el mensaje `msg_id` (`POST /admin/reenviar`), recarga el historial del peer en foco
    /// (conservando el filtro por estado activo) y emite un TOAST con el resultado — "Reenviado
    /// como #N" si el broker devolvió el id nuevo (trazabilidad-05). Red SIEMPRE en fondo con
    /// `bloquear_en` (fix anti-SIGABRT), sin `.unwrap()`.
    fn reenviar_mensaje(&mut self, msg_id: i64, cx: &mut Context<Self>) {
        // CONTEXTO REDIS (redis-03): si el pop-up de inspección de cola está abierto, el reenvío
        // viene de ahí — la recarga correcta es la de ESA cola (+ los contadores del snapshot),
        // no el historial de Trazabilidad.
        if let Some(peer) = self.datos.redis_cola_detalle.clone() {
            self.reenviar_desde_redis(msg_id, peer, cx);
            return;
        }
        let Some(peer) = self.datos.traza_peer.clone() else {
            return; // sin peer en foco no hay historial que refrescar
        };
        let filtro = self.datos.traza_filtro_estado;
        let ventana = cx.active_window();
        let cliente = self.cliente.clone();
        let fondo = cx.background_executor().spawn(async move {
            let resultado = cliente.bloquear_en(cliente.reenviar(msg_id));
            // Si el reenvío fue aceptado, re-lee el historial para reflejar el nuevo mensaje.
            let recarga = match &resultado {
                Ok(r) if r.ok => Some(cliente.bloquear_en(cliente.historial(&peer, filtro))),
                _ => None,
            };
            (resultado, recarga)
        });
        cx.spawn(async move |esta, cx| {
            let (resultado, recarga) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Some(Ok(lista)) = recarga {
                    let n = lista.len();
                    esta.datos.historial = lista;
                    if esta.datos.traza_seleccion >= n {
                        esta.datos.traza_seleccion = n.saturating_sub(1);
                    }
                }
                cx.notify();
            });
            // Toast con el veredicto tipado del broker: reenviado (con el id nuevo), rechazado
            // (`ok=false`, mensaje inexistente) o error de transporte.
            if let Some(ventana) = ventana {
                let _ = ventana.update(cx, |_raiz, win, app| {
                    use gpui_component::notification::Notification;
                    use gpui_component::WindowExt as _;
                    let note = match &resultado {
                        Ok(r) if r.ok => match r.msg_id {
                            Some(nuevo) => Notification::success(format!(
                                "Mensaje #{msg_id} reenviado como #{nuevo}"
                            )),
                            None => Notification::success(format!("Mensaje #{msg_id} reenviado")),
                        },
                        Ok(r) => {
                            let motivo = r
                                .error
                                .clone()
                                .unwrap_or_else(|| "el broker lo rechazó".to_string());
                            Notification::error(format!("No se pudo reenviar: {motivo}"))
                        }
                        Err(e) => Notification::error(format!("No se pudo reenviar: {e}")),
                    };
                    win.push_notification(note, app);
                });
            }
        })
        .detach();
    }

    // --- Manejadores de la pantalla Tareas ---
    // Cada acción de escritura sigue el patrón de `descartar_alerta`: POST en `cx.spawn`, y al
    // volver se recarga `admin_tareas` para reflejar el estado real (el broker valida transiciones).

    /// Marca la fila `indice` de la tabla de tareas (sólo UI; la barra de acciones opera sobre ella).
    fn seleccionar_tarea(&mut self, indice: usize, cx: &mut Context<Self>) {
        self.datos.tareas_seleccion = indice;
        cx.notify();
    }

    /// Abre el POP-UP de detalle (tareas-01) de la tarea `indice` y la marca como seleccionada.
    /// Índice fuera de rango (la lista cambió entre el pintado y el click) → no abre nada.
    fn abrir_detalle_tarea(&mut self, indice: usize, cx: &mut Context<Self>) {
        if indice >= self.datos.tareas.len() {
            return;
        }
        self.datos.tareas_seleccion = indice;
        self.datos.tarea_detalle = Some(indice);
        cx.notify();
    }

    /// Cierra el pop-up de detalle de tarea (deja la selección donde estaba). Destino de
    /// `CerrarDetalleTarea` (✕, botón Cerrar, clic en el backdrop) y de `Esc` en la pantalla
    /// Tareas. También cierra el detalle de tarea de JORNADA (jornada-01): ese overlay reutiliza
    /// el mismo modal de la fase 1, así que sus ✕/Cerrar despachan esta misma acción.
    fn cerrar_detalle_tarea(&mut self, cx: &mut Context<Self>) {
        if self.datos.tarea_detalle.is_some() || self.datos.jornada_tarea_detalle.is_some() {
            self.datos.tarea_detalle = None;
            self.datos.jornada_tarea_detalle = None;
            cx.notify();
        }
    }

    // --- Formularios del CRUD de tareas (tareas-03..08) ---
    // Un único formulario abierto a la vez (`tareas_form`). Abrir SIEMBRA los Inputs compartidos
    // (nunca heredan texto del formulario anterior), limpia el peer elegido y el error, y cierra
    // el detalle (un solo overlay en pantalla). Confirmar valida la frontera y hace el POST.

    /// Abre el formulario `form`, sembrando las tres entradas con los valores indicados. Punto
    /// único de apertura: garantiza el estado limpio (peer/error) y el cierre del detalle.
    fn abrir_form_tareas(
        &mut self,
        form: crate::vista::tareas::FormTareas,
        descripcion: &str,
        estimado: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::vista::tareas::FormTareas;
        if let Some(inputs) = &self.datos.inputs_tareas {
            inputs.sembrar(descripcion, estimado, "", window, cx);
        }
        // ASIGNAR/REASIGNAR necesitan el roster fresco para el selector de peer: la pantalla
        // Tareas no lo recarga por sí sola (sólo Peers/Jornada lo hacen).
        if matches!(form, FormTareas::Asignar | FormTareas::Reasignar { .. }) {
            self.recargar_roster(cx);
        }
        self.datos.tarea_detalle = None;
        self.datos.tareas_form_peer = None;
        self.datos.tareas_form_error = None;
        self.datos.tareas_form = Some(form);
        cx.notify();
    }

    /// Cierra el formulario activo sin confirmar (Cancelar, ✕, clic fuera, Esc).
    fn cerrar_form_tareas(&mut self, cx: &mut Context<Self>) {
        if self.datos.tareas_form.is_some() {
            self.datos.tareas_form = None;
            self.datos.tareas_form_peer = None;
            self.datos.tareas_form_error = None;
            cx.notify();
        }
    }

    /// Fija el peer elegido en el selector del formulario activo (asignar/reasignar).
    fn elegir_peer_form(&mut self, id: String, cx: &mut Context<Self>) {
        self.datos.tareas_form_peer = Some(id);
        // Elegir un peer resuelve el error "elige un peer destino" si estaba en pantalla.
        self.datos.tareas_form_error = None;
        cx.notify();
    }

    /// Abre REASIGNAR (tareas-04) buscando el dueño actual de la tarea (para excluirlo del
    /// selector). Si la tarea ya no está (la lista cambió), no abre nada.
    fn abrir_form_reasignar(&mut self, tarea_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dueno) = self
            .datos
            .tareas
            .iter()
            .find(|t| t.id == tarea_id)
            .map(|t| t.instancia_id.clone())
        else {
            return;
        };
        self.abrir_form_tareas(
            crate::vista::tareas::FormTareas::Reasignar { tarea_id, dueno },
            "",
            "",
            window,
            cx,
        );
    }

    /// Abre EDITAR descripción (tareas-05) PRECARGANDO el Input con la descripción actual.
    fn abrir_form_editar(&mut self, tarea_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(descripcion) = self
            .datos
            .tareas
            .iter()
            .find(|t| t.id == tarea_id)
            .map(|t| t.descripcion.clone())
        else {
            return;
        };
        self.abrir_form_tareas(
            crate::vista::tareas::FormTareas::Editar { tarea_id },
            &descripcion,
            "",
            window,
            cx,
        );
    }

    /// Abre AMPLIAR estimado (tareas-06) capturando el estimado VIGENTE (para el hint y la suma).
    fn abrir_form_ampliar(&mut self, tarea_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(estimado_actual) = self
            .datos
            .tareas
            .iter()
            .find(|t| t.id == tarea_id)
            .map(|t| t.estimado_seg)
        else {
            return;
        };
        self.abrir_form_tareas(
            crate::vista::tareas::FormTareas::Ampliar {
                tarea_id,
                estimado_actual,
            },
            "",
            "",
            window,
            cx,
        );
    }

    /// Abre el mini-modal de CONFIRMACIÓN destructiva (tareas-08) capturando la descripción para
    /// el texto "¿Seguro? «…»". Cancelar y Reabrir pasan por aquí; el resto transiciona directo.
    fn pedir_confirm_estado(
        &mut self,
        tarea_id: String,
        estado: peers_core::EstadoTarea,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(descripcion) = self
            .datos
            .tareas
            .iter()
            .find(|t| t.id == tarea_id)
            .map(|t| t.descripcion.clone())
        else {
            return;
        };
        self.abrir_form_tareas(
            crate::vista::tareas::FormTareas::Confirmar {
                tarea_id,
                estado,
                descripcion,
            },
            "",
            "",
            window,
            cx,
        );
    }

    /// Confirma el formulario activo: valida la FRONTERA (campos obligatorios, números legibles) y
    /// dispara el POST correspondiente vía `mutar_tarea` (que recarga + toast). Un error de
    /// validación se queda en `tareas_form_error` (el modal sigue abierto para corregir); una
    /// validación OK cierra el formulario de inmediato (la mutación ya está en marcha).
    fn confirmar_form_tareas(&mut self, cx: &mut Context<Self>) {
        use crate::vista::tareas::FormTareas;
        let Some(form) = self.datos.tareas_form.clone() else {
            return;
        };

        // Lecturas de los Inputs compartidos (trim en frontera; parse, don't validate).
        let (descripcion, estimado_txt, motivo) = match &self.datos.inputs_tareas {
            Some(inputs) => (
                inputs.descripcion.read(cx).value().trim().to_string(),
                inputs.estimado.read(cx).value().trim().to_string(),
                inputs.motivo.read(cx).value().trim().to_string(),
            ),
            None => (String::new(), String::new(), String::new()),
        };

        // Cada rama valida y produce (mensaje de éxito, POST como closure sync sobre el cliente).
        // El closure corre en el hilo de fondo con `bloquear_en` (reqwest exige su runtime tokio).
        match form {
            FormTareas::Asignar => {
                let Some(peer) = self.datos.tareas_form_peer.clone() else {
                    self.datos.tareas_form_error = Some("elige un peer destino".to_string());
                    cx.notify();
                    return;
                };
                if descripcion.is_empty() {
                    self.datos.tareas_form_error = Some("la descripción es obligatoria".to_string());
                    cx.notify();
                    return;
                }
                // Estimado opcional: vacío = sin estimado; con texto, debe ser minutos legibles.
                let estimado_seg = if estimado_txt.is_empty() {
                    None
                } else {
                    match estimado_txt.parse::<i64>() {
                        Ok(min) if min > 0 => Some(min * 60),
                        _ => {
                            self.datos.tareas_form_error =
                                Some("el estimado debe ser un número de minutos > 0".to_string());
                            cx.notify();
                            return;
                        }
                    }
                };
                self.cerrar_form_tareas(cx);
                let exito = format!("Tarea asignada a {peer}");
                self.mutar_tarea(
                    exito,
                    move |c| {
                        c.bloquear_en(c.tarea_asignar(&peer, &descripcion, estimado_seg))
                            .map(|_| ())
                    },
                    cx,
                );
            }
            FormTareas::Reasignar { tarea_id, .. } => {
                let Some(destino) = self.datos.tareas_form_peer.clone() else {
                    self.datos.tareas_form_error = Some("elige el peer destino".to_string());
                    cx.notify();
                    return;
                };
                self.cerrar_form_tareas(cx);
                let exito = format!("Tarea reasignada a {destino}");
                self.mutar_tarea(
                    exito,
                    move |c| {
                        c.bloquear_en(c.tarea_reasignar(&tarea_id, &destino))
                            .map(|_| ())
                    },
                    cx,
                );
            }
            FormTareas::Editar { tarea_id } => {
                if descripcion.is_empty() {
                    self.datos.tareas_form_error = Some("la descripción es obligatoria".to_string());
                    cx.notify();
                    return;
                }
                self.cerrar_form_tareas(cx);
                self.mutar_tarea(
                    "Descripción actualizada".to_string(),
                    move |c| {
                        c.bloquear_en(c.tarea_editar(&tarea_id, Some(descripcion), None))
                            .map(|_| ())
                    },
                    cx,
                );
            }
            FormTareas::Ampliar {
                tarea_id,
                estimado_actual,
            } => {
                let minutos = match estimado_txt.parse::<i64>() {
                    Ok(min) if min > 0 => min,
                    _ => {
                        self.datos.tareas_form_error =
                            Some("indica los minutos a sumar (número > 0)".to_string());
                        cx.notify();
                        return;
                    }
                };
                // Suma sobre el vigente capturado al abrir; el broker valida el rango plausible.
                let nuevo = estimado_actual.unwrap_or(0) + minutos * 60;
                self.cerrar_form_tareas(cx);
                let exito = format!("Estimado ampliado +{minutos}min");
                self.mutar_tarea(
                    exito,
                    move |c| {
                        c.bloquear_en(c.tarea_editar(&tarea_id, None, Some(nuevo)))
                            .map(|_| ())
                    },
                    cx,
                );
            }
            FormTareas::Bloquear { tarea_id } => {
                // tareas-07: el motivo es OBLIGATORIO — bloquear sin razón era el defecto a matar.
                if motivo.is_empty() {
                    self.datos.tareas_form_error =
                        Some("el motivo de bloqueo es obligatorio".to_string());
                    cx.notify();
                    return;
                }
                self.cerrar_form_tareas(cx);
                self.mutar_tarea(
                    "Tarea bloqueada (motivo registrado)".to_string(),
                    move |c| {
                        c.bloquear_en(c.tarea_estado(
                            &tarea_id,
                            peers_core::EstadoTarea::Bloqueada,
                            Some(motivo),
                            None,
                        ))
                        .map(|_| ())
                    },
                    cx,
                );
            }
            FormTareas::PedirEvidencia { tarea_id } => {
                // #7 (paquete de bugs Max): la evidencia es OBLIGATORIA para llegar a Hecha desde
                // este flujo — nunca se dispara el cambio de estado sin ella (mismo criterio que
                // el motivo de Bloquear arriba). Reusa el Input `motivo` (un sólo formulario abierto
                // a la vez); el valor tecleado se envía como `evidencia`, no como `motivo`, al POST.
                if motivo.is_empty() {
                    self.datos.tareas_form_error =
                        Some("la evidencia es obligatoria para marcar Hecha".to_string());
                    cx.notify();
                    return;
                }
                self.cerrar_form_tareas(cx);
                self.mutar_tarea(
                    "Tarea marcada como hecha (evidencia registrada)".to_string(),
                    move |c| {
                        c.bloquear_en(c.tarea_estado(
                            &tarea_id,
                            peers_core::EstadoTarea::Hecha,
                            None,
                            Some(motivo),
                        ))
                        .map(|_| ())
                    },
                    cx,
                );
            }
            FormTareas::Confirmar {
                tarea_id, estado, ..
            } => {
                self.cerrar_form_tareas(cx);
                let exito = match estado {
                    peers_core::EstadoTarea::Cancelada => "Tarea cancelada".to_string(),
                    peers_core::EstadoTarea::Abierta => "Tarea reabierta".to_string(),
                    _ => "Estado actualizado".to_string(),
                };
                self.mutar_tarea(
                    exito,
                    move |c| {
                        c.bloquear_en(c.tarea_estado(&tarea_id, estado, None, None))
                            .map(|_| ())
                    },
                    cx,
                );
            }
        }
    }

    /// Ejecuta una MUTACIÓN de tarea en segundo plano y aplica el post-proceso común: recarga
    /// `admin_tareas` si fue OK, guarda el error en el banner si falló, y emite un toast de
    /// éxito/fallo (tareas-17 aplicado a las mutaciones del CRUD; patrón de `descartar_alerta`).
    /// `hacer` es un closure SÍNCRONO que corre en el executor de fondo y usa `bloquear_en` para
    /// entrar al runtime tokio del cliente (reqwest lo exige; el executor de GPUI no lo es).
    fn mutar_tarea(
        &mut self,
        exito: String,
        hacer: impl FnOnce(&ClienteBroker) -> Result<(), crate::cliente::ErrorBroker> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let ventana = cx.active_window();
        let cliente = self.cliente.clone();
        let fondo = cx.background_executor().spawn(async move {
            let resultado = hacer(&cliente);
            // Tras mutar OK, re-lee la lista para reflejar el estado real del broker.
            let recarga = match &resultado {
                Ok(_) => Some(cliente.bloquear_en(cliente.admin_tareas())),
                Err(_) => None,
            };
            (resultado, recarga)
        });
        cx.spawn(async move |esta, cx| {
            let (resultado, recarga) = fondo.await;
            let ok = resultado.is_ok();
            let motivo_fallo = resultado.as_ref().err().map(|e| e.to_string());
            let _ = esta.update(cx, |esta, cx| {
                Self::aplicar_recarga_tareas(esta, resultado.err(), recarga, cx);
            });
            // Toast de feedback vía el handle de la ventana (no hay `&mut Window` en el spawn).
            if let Some(ventana) = ventana {
                let _ = ventana.update(cx, |_raiz, win, app| {
                    use gpui_component::notification::Notification;
                    use gpui_component::WindowExt as _;
                    let note = if ok {
                        Notification::success(exito.clone())
                    } else {
                        let motivo =
                            motivo_fallo.unwrap_or_else(|| "error desconocido".to_string());
                        Notification::error(format!("No se pudo aplicar: {motivo}"))
                    };
                    win.push_notification(note, app);
                });
            }
        })
        .detach();
    }

    /// Recarga SÓLO el roster de peers vivos (`POST /listar`) para poblar el selector de peer de
    /// los formularios de asignar/reasignar. No toca `error_peers` (no es la pantalla Peers): un
    /// fallo deja el roster como estaba y el selector avisa si quedó vacío.
    fn recargar_roster(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.listar_instancias()) });
        cx.spawn(async move |esta, cx| {
            let r = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Ok(lista) = r {
                    esta.datos.instancias = lista;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Transiciona el estado de una tarea (`POST /tarea/estado`) y recarga la lista. El broker
    /// valida la transición; si la rechaza, el error se pinta en el banner (`error_tareas`).
    ///
    /// CORRECCIÓN (crash SIGABRT): la versión anterior hacía `cliente.tarea_estado(...).await`
    /// DIRECTO en `cx.spawn` — el executor de GPUI no es tokio y reqwest abortaba con "there is no
    /// reactor running". Ahora delega en `mutar_tarea`, que ejecuta la red en el hilo de fondo con
    /// `bloquear_en` (el runtime tokio del cliente), igual que `cargar_peers`.
    fn cambiar_estado_tarea(
        &mut self,
        tarea_id: String,
        estado: peers_core::EstadoTarea,
        cx: &mut Context<Self>,
    ) {
        use peers_core::EstadoTarea;
        let exito = match estado {
            EstadoTarea::EnCurso => "Tarea en curso",
            EstadoTarea::Hecha => "Tarea marcada como hecha",
            EstadoTarea::Cancelada => "Tarea cancelada",
            EstadoTarea::Abierta => "Tarea reabierta",
            EstadoTarea::Bloqueada => "Tarea bloqueada",
        };
        self.mutar_tarea(
            exito.to_string(),
            move |c| {
                c.bloquear_en(c.tarea_estado(&tarea_id, estado, None, None))
                    .map(|_| ())
            },
            cx,
        );
    }

    /// Reasigna una tarea a otro peer (`POST /tarea/reasignar`) y recarga. Si `nuevo_instancia_id`
    /// llega vacío (la vista no tiene selector de peer todavía), se resuelve una ruta por defecto:
    /// el SIGUIENTE peer vivo de `instancias` distinto del dueño actual (rota el trabajo, espejo del
    /// ciclado con la tecla `a` de la TUI). Si no hay otro peer, se ignora (no hay a quién reasignar).
    fn reasignar_tarea(&mut self, tarea_id: String, nuevo_instancia_id: String, cx: &mut Context<Self>) {
        let destino = if !nuevo_instancia_id.is_empty() {
            Some(nuevo_instancia_id)
        } else {
            self.siguiente_peer_para_tarea(&tarea_id)
        };
        let Some(destino) = destino else {
            // No hay otro peer vivo al que reasignar: avisa en el banner y no llama al broker.
            self.datos.error_tareas = Some(crate::cliente::ErrorBroker::Otro(
                "no hay otro peer vivo al que reasignar la tarea".to_string(),
            ));
            cx.notify();
            return;
        };
        // Red SIEMPRE vía `mutar_tarea` (fondo + `bloquear_en`): el `.await` directo en `cx.spawn`
        // abortaba con "no reactor running" (mismo fix que `cambiar_estado_tarea`).
        let exito = format!("Tarea reasignada a {destino}");
        self.mutar_tarea(
            exito,
            move |c| {
                c.bloquear_en(c.tarea_reasignar(&tarea_id, &destino))
                    .map(|_| ())
            },
            cx,
        );
    }

    /// "Tócale el hombro": `POST /tarea/forzar`. No cambia el estado; recarga por consistencia.
    fn forzar_tarea(&mut self, tarea_id: String, cx: &mut Context<Self>) {
        self.mutar_tarea(
            "Recordatorio enviado al dueño".to_string(),
            move |c| c.bloquear_en(c.tarea_forzar(&tarea_id)).map(|_| ()),
            cx,
        );
    }

    /// Crea una tarea nueva (`POST /tarea/asignar`) y recarga. Si `instancia_id`/`descripcion` van
    /// vacíos (la vista no tiene formulario de captura todavía), se resuelve una ruta por defecto:
    /// asignar al PRIMER peer vivo una tarea con descripción marcador, para que el flujo end-to-end
    /// funcione y quede visible en la tabla. Documentado como comportamiento provisional hasta que
    /// exista un diálogo de captura (peer + descripción + estimado).
    fn asignar_tarea(
        &mut self,
        instancia_id: String,
        descripcion: String,
        estimado_seg: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        // Peer destino: el indicado, o el primer peer vivo como ruta por defecto.
        let destino = if !instancia_id.is_empty() {
            Some(instancia_id)
        } else {
            self.datos.instancias.first().map(|i| i.id.clone())
        };
        let Some(destino) = destino else {
            self.datos.error_tareas = Some(crate::cliente::ErrorBroker::Otro(
                "no hay ningún peer vivo al que asignar una tarea".to_string(),
            ));
            cx.notify();
            return;
        };
        // Descripción: la indicada, o un marcador provisional (hasta que haya formulario).
        let descripcion = if descripcion.is_empty() {
            "(tarea nueva — editar descripción)".to_string()
        } else {
            descripcion
        };
        // Red SIEMPRE vía `mutar_tarea` (fondo + `bloquear_en`), mismo fix anti-SIGABRT.
        let exito = format!("Tarea asignada a {destino}");
        self.mutar_tarea(
            exito,
            move |c| {
                c.bloquear_en(c.tarea_asignar(&destino, &descripcion, estimado_seg))
                    .map(|_| ())
            },
            cx,
        );
    }

    /// Aplica el resultado de una acción sobre tareas: guarda el error (si lo hubo) en el banner, o
    /// refresca `tareas` con la recarga y acota `tareas_seleccion` a la nueva longitud. Centraliza
    /// el post-proceso común de las 4 acciones de escritura de tareas para no repetirlo.
    fn aplicar_recarga_tareas(
        esta: &mut Self,
        error: Option<crate::cliente::ErrorBroker>,
        recarga: Option<crate::cliente::ResultadoBroker<Vec<peers_core::Tarea>>>,
        cx: &mut Context<Self>,
    ) {
        match error {
            Some(e) => esta.datos.error_tareas = Some(e), // la acción falló: conserva la lista previa
            None => {
                esta.datos.error_tareas = None;
                match recarga {
                    Some(Ok(lista)) => {
                        esta.datos.tareas = lista;
                        let n = esta.datos.tareas.len();
                        if n == 0 {
                            esta.datos.tareas_seleccion = 0;
                        } else if esta.datos.tareas_seleccion >= n {
                            esta.datos.tareas_seleccion = n - 1;
                        }
                        // Mismo criterio que `cargar_tareas`: detalle fuera de rango → cerrar.
                        if let Some(i) = esta.datos.tarea_detalle {
                            if i >= n {
                                esta.datos.tarea_detalle = None;
                            }
                        }
                    }
                    // La acción fue OK pero la recarga falló: informa sin romper la lista.
                    Some(Err(e)) => esta.datos.error_tareas = Some(e),
                    None => {}
                }
            }
        }
        cx.notify();
    }

    /// Devuelve el siguiente peer vivo distinto del dueño actual de la tarea `tarea_id`, para la
    /// reasignación por defecto (rotación). `None` si la tarea no está o no hay otro peer.
    fn siguiente_peer_para_tarea(&self, tarea_id: &str) -> Option<String> {
        let dueño = self
            .datos
            .tareas
            .iter()
            .find(|t| t.id == tarea_id)
            .map(|t| t.instancia_id.as_str())?;
        let n = self.datos.instancias.len();
        if n == 0 {
            return None;
        }
        // Posición del dueño en el roster (si está); rota al siguiente índice.
        let pos = self.datos.instancias.iter().position(|i| i.id == dueño);
        let inicio = pos.map(|p| p + 1).unwrap_or(0);
        for offset in 0..n {
            let cand = &self.datos.instancias[(inicio + offset) % n];
            if cand.id != dueño {
                return Some(cand.id.clone());
            }
        }
        None
    }

    // --- Manejadores de la pantalla Redis ---

    /// Marca la cola de la fila `indice` (o la limpia si quedó fuera de rango). Alimenta "Purgar".
    fn seleccionar_cola_redis(&mut self, indice: usize, cx: &mut Context<Self>) {
        let total = self
            .datos
            .redis
            .as_ref()
            .map(|r| r.colas.len())
            .unwrap_or(0);
        self.datos.redis_seleccion_cola = if indice < total { Some(indice) } else { None };
        cx.notify();
    }

    /// Purga cola + outbox del peer `id` (`POST /admin/purgar`) y recarga `admin_redis`. Como ese
    /// peer desaparece de la lista, tras la recarga se limpia la selección (evita índice colgado).
    fn purgar_peer(&mut self, id: String, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        // Mismo fix anti-SIGABRT que el resto de escrituras: la red va al fondo con `bloquear_en`.
        let fondo = cx.background_executor().spawn(async move {
            let resultado = cliente.bloquear_en(cliente.purgar(&id));
            let recarga = match &resultado {
                Ok(_) => Some(cliente.bloquear_en(cliente.admin_redis())),
                Err(_) => None,
            };
            (resultado, recarga)
        });
        cx.spawn(async move |esta, cx| {
            let (resultado, recarga) = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                match resultado {
                    Ok(_) => {
                        esta.datos.error_redis = None;
                        if let Some(Ok(v)) = recarga {
                            esta.datos.redis = Some(v);
                        } else if let Some(Err(e)) = recarga {
                            esta.datos.error_redis = Some(e);
                        }
                        // El peer purgado ya no está: soltar la selección (clave era el id, no el índice).
                        esta.datos.redis_seleccion_cola = None;
                    }
                    Err(e) => esta.datos.error_redis = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Abre el pop-up de INSPECCIÓN de cola (redis-01) y carga sus mensajes pendientes vía
    /// `GET /admin/historial?id=&estado=enviado` (aproximación con endpoint EXISTENTE — el
    /// contenido crudo exacto exigiría `/admin/bandeja`, pendiente de backend, y no se inventa).
    fn abrir_cola_redis(&mut self, id: String, cx: &mut Context<Self>) {
        self.datos.redis_cola_detalle = Some(id.clone());
        self.datos.redis_cola_mensajes.clear();
        self.datos.redis_cola_cargando = true;
        cx.notify();

        let cliente = self.cliente.clone();
        let fondo = cx.background_executor().spawn(async move {
            cliente.bloquear_en(cliente.historial(&id, Some(peers_core::EstadoMensaje::Enviado)))
        });
        cx.spawn(async move |esta, cx| {
            let r = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                esta.datos.redis_cola_cargando = false;
                match r {
                    Ok(lista) => esta.datos.redis_cola_mensajes = lista,
                    // Un fallo deja la lista vacía y el motivo en el banner de la pantalla.
                    Err(e) => esta.datos.error_redis = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Cierra el pop-up de inspección de cola (✕, Cerrar, clic fuera, Esc).
    fn cerrar_cola_redis(&mut self, cx: &mut Context<Self>) {
        if self.datos.redis_cola_detalle.is_some() {
            self.datos.redis_cola_detalle = None;
            self.datos.redis_cola_mensajes.clear();
            self.datos.redis_cola_cargando = false;
            cx.notify();
        }
    }

    /// Abre la CONFIRMACIÓN de purga (redis-04) para el peer `id`.
    fn pedir_purga(&mut self, id: String, cx: &mut Context<Self>) {
        self.datos.redis_confirmar_purga = Some(id);
        cx.notify();
    }

    /// Cancela la purga pendiente (Cancelar / clic fuera / Esc): no toca nada.
    fn cancelar_purga(&mut self, cx: &mut Context<Self>) {
        if self.datos.redis_confirmar_purga.is_some() {
            self.datos.redis_confirmar_purga = None;
            cx.notify();
        }
    }

    /// Confirma la purga pendiente: cierra el diálogo y dispara el POST real (`purgar_peer`).
    fn confirmar_purga(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.datos.redis_confirmar_purga.take() else {
            return;
        };
        cx.notify();
        self.purgar_peer(id, cx);
    }

    // --- Manejadores de Jornada (sólo UI) ---

    /// Marca la fila `indice` de la tabla de SESIONES de Jornada (sólo feedback visual).
    fn seleccionar_sesion_jornada(&mut self, indice: usize, cx: &mut Context<Self>) {
        self.datos.jornada_ses_sel = indice;
        cx.notify();
    }

    /// Marca la fila `indice` de la tabla de TAREAS de Jornada (sólo feedback visual).
    fn seleccionar_tarea_jornada(&mut self, indice: usize, cx: &mut Context<Self>) {
        self.datos.jornada_tar_sel = indice;
        cx.notify();
    }

    /// Abre el pop-up de detalle (jornada-01) de la tarea `indice` de la jornada. Además refresca
    /// la caché GLOBAL de tareas: las acciones colgadas del modal reutilizado (Editar/Ampliar,
    /// fase 2) buscan la tarea por id en `datos.tareas`, y en esta pantalla esa caché no se
    /// recarga sola.
    fn abrir_tarea_jornada(&mut self, indice: usize, cx: &mut Context<Self>) {
        let total = self.datos.jornada.as_ref().map(|j| j.tareas.len()).unwrap_or(0);
        if indice >= total {
            return;
        }
        self.datos.jornada_tar_sel = indice;
        self.datos.jornada_tarea_detalle = Some(indice);
        self.cargar_tareas(cx);
        cx.notify();
    }

    /// Abre el pop-up de detalle de sesión (jornada-02) de `jornada.sesiones[indice]`.
    fn abrir_sesion_jornada(&mut self, indice: usize, cx: &mut Context<Self>) {
        let total = self.datos.jornada.as_ref().map(|j| j.sesiones.len()).unwrap_or(0);
        if indice >= total {
            return;
        }
        self.datos.jornada_ses_sel = indice;
        self.datos.jornada_sesion_detalle = Some(indice);
        cx.notify();
    }

    /// Cierra el pop-up de detalle de sesión (✕, Cerrar, clic fuera, Esc).
    fn cerrar_sesion_jornada(&mut self, cx: &mut Context<Self>) {
        if self.datos.jornada_sesion_detalle.is_some() {
            self.datos.jornada_sesion_detalle = None;
            cx.notify();
        }
    }

    /// Alterna el DROPDOWN del selector de peer (jornada-03). Al abrirlo refresca el roster para
    /// que la lista de peers vivos no esté rancia (en esta pantalla ya se recarga con la jornada,
    /// pero el desplegable debe reflejar el ahora).
    fn alternar_dropdown_jornada(&mut self, cx: &mut Context<Self>) {
        self.datos.jornada_dropdown = !self.datos.jornada_dropdown;
        if self.datos.jornada_dropdown {
            self.recargar_roster(cx);
        }
        cx.notify();
    }

    /// Alterna colapsado/expandido de UNA de las 3 tablas de Jornada (bug #1: peers con muchas
    /// sesiones/tareas/acciones desbordaban la pantalla). Un solo manejador para las 3 — el `match`
    /// sólo decide QUÉ flag tocar, la lógica de alternar es idéntica.
    fn alternar_seccion_jornada(
        &mut self,
        seccion: crate::vista::jornada::SeccionJornada,
        cx: &mut Context<Self>,
    ) {
        use crate::vista::jornada::SeccionJornada;
        let flag = match seccion {
            SeccionJornada::Sesiones => &mut self.datos.jornada_ses_colapsada,
            SeccionJornada::Tareas => &mut self.datos.jornada_tar_colapsada,
            SeccionJornada::Acciones => &mut self.datos.jornada_acc_colapsada,
        };
        *flag = !*flag;
        cx.notify();
    }

    /// Salta al peer `id` desde el dropdown (jornada-03 A): cierra el desplegable, fija el foco y
    /// recarga la jornada (reutiliza `ver_jornada_peer`, que resetea selecciones y fuerza fetch).
    fn elegir_peer_jornada(&mut self, id: String, cx: &mut Context<Self>) {
        self.datos.jornada_dropdown = false;
        self.datos.jornada_tarea_detalle = None;
        self.datos.jornada_sesion_detalle = None;
        self.ver_jornada_peer(id, cx);
    }

    /// Cicla al peer anterior/siguiente con los chevrones ‹ › (jornada-03 B, espejo de `[`/`]` de
    /// la TUI). Rota circularmente sobre el roster; sin foco actual, `›` va al primero y `‹` al
    /// último. Sin peers vivos no hace nada.
    fn ciclar_peer_jornada(&mut self, delta: i32, cx: &mut Context<Self>) {
        let n = self.datos.instancias.len();
        if n == 0 {
            return;
        }
        let pos = self
            .datos
            .jornada_peer
            .as_deref()
            .and_then(|id| self.datos.instancias.iter().position(|i| i.id == id));
        let idx = match pos {
            Some(p) => ((p as i64 + delta as i64).rem_euclid(n as i64)) as usize,
            None if delta >= 0 => 0,
            None => n - 1,
        };
        let Some(inst) = self.datos.instancias.get(idx) else {
            return;
        };
        let id = inst.id.clone();
        self.datos.jornada_dropdown = false;
        self.datos.jornada_tarea_detalle = None;
        self.datos.jornada_sesion_detalle = None;
        self.ver_jornada_peer(id, cx);
    }

    /// Transición de estado desde el detalle de jornada (jornada-04). Las DESTRUCTIVAS
    /// (Cancelar/Reabrir) pasan por la confirmación de la fase 2 (`FormTareas::Confirmar`, con la
    /// descripción sacada de la PROPIA jornada para no depender de la caché global); Hecha va
    /// directa vía `cambiar_estado_tarea` (fase 2: `mutar_tarea` + validación del broker + toast).
    /// En ambos casos se cierra el detalle y se re-pide la jornada para reflejar el cambio.
    fn cambiar_estado_tarea_jornada(
        &mut self,
        tarea_id: String,
        estado: peers_core::EstadoTarea,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use peers_core::EstadoTarea;
        self.datos.jornada_tarea_detalle = None;
        match estado {
            EstadoTarea::Cancelada | EstadoTarea::Abierta => {
                let descripcion = self
                    .datos
                    .jornada
                    .as_ref()
                    .and_then(|j| j.tareas.iter().find(|t| t.id == tarea_id))
                    .map(|t| t.descripcion.clone())
                    .unwrap_or_default();
                self.abrir_form_tareas(
                    crate::vista::tareas::FormTareas::Confirmar {
                        tarea_id,
                        estado,
                        descripcion,
                    },
                    "",
                    "",
                    window,
                    cx,
                );
            }
            // #7 (paquete de bugs Max): MISMA función pura `necesita_pedir_evidencia` que
            // `boton_estado`/atajo "h" de Tareas — no una condición divergente escrita a mano. Este
            // es el TERCER camino hacia Hecha (modal de detalle de tarea desde Jornada) que hubiera
            // seguido siendo un bypass si sólo arreglaba `boton_estado` y el atajo de teclado.
            // Si la tarea no aparece en la caché de jornada (no debería pasar, pero R10 exige
            // degradar hacia el lado SEGURO), `and_then` encadena a `None` → `necesita_pedir_evidencia`
            // ve `evidencia_existente: None` → pide evidencia igual, nunca al revés.
            EstadoTarea::Hecha
                if crate::vista::tareas::necesita_pedir_evidencia(
                    estado,
                    self.datos
                        .jornada
                        .as_ref()
                        .and_then(|j| j.tareas.iter().find(|t| t.id == tarea_id))
                        .and_then(|t| t.evidencia.as_ref()),
                ) =>
            {
                self.abrir_form_tareas(
                    crate::vista::tareas::FormTareas::PedirEvidencia { tarea_id },
                    "",
                    "",
                    window,
                    cx,
                );
            }
            _ => {
                self.cambiar_estado_tarea(tarea_id, estado, cx);
                // La tabla de la jornada no se recarga con `admin_tareas`: re-pide `/jornada` ya
                // (el timer de 2s también lo haría, pero el feedback inmediato es mejor).
                self.cargar_jornada(cx);
            }
        }
        cx.notify();
    }

    // --- Navegación por teclado (espejo de la TUI) ---
    // Se maneja en el `on_key_down` del contenedor raíz (que rastrea el foco). Documentación del
    // mapa completo en el comentario de `manejar_tecla`.

    /// Mueve la selección de fila de la PANTALLA ACTIVA en `delta` (+1 abajo, -1 arriba), acotando a
    /// los límites (sin envolver). Cada pantalla tiene su propio cursor; esta función enruta al que
    /// corresponde. Las pantallas sin tabla (Broker/Config/Acceso) no hacen nada.
    fn mover_seleccion(&mut self, delta: i32, cx: &mut Context<Self>) {
        // Helper local: aplica delta a un `usize` dentro de `[0, len)`, saturando en los extremos.
        fn mover(actual: usize, delta: i32, len: usize) -> usize {
            if len == 0 {
                return 0;
            }
            let nuevo = actual as i64 + delta as i64;
            nuevo.clamp(0, len as i64 - 1) as usize
        }
        match self.activa {
            Pantalla::Peers => {
                let len = self.datos.instancias.len();
                if len == 0 {
                    return;
                }
                let actual = self.datos.peers_seleccion.unwrap_or(0);
                self.datos.peers_seleccion = Some(mover(actual, delta, len));
            }
            Pantalla::Alertas => {
                // FIX (bug real de correctitud, peers-18-style): `alertas_seleccion` es un índice
                // sobre la lista VISIBLE post-filtro (mismo contrato que `indice_real_alerta` /
                // `acotar_seleccion_alertas_visibles`), NO sobre `datos.alertas` completo. Clampar
                // contra el TOTAL permitía que ↑/↓ moviera la selección más allá de las filas
                // realmente pintadas cuando había un filtro de tipo activo (alertas-03).
                use crate::vista::alertas::pasa_filtro;
                let visibles = self
                    .datos
                    .alertas
                    .iter()
                    .filter(|a| pasa_filtro(a, &self.datos.alertas_filtro_tipos))
                    .count();
                self.datos.alertas_seleccion = mover(self.datos.alertas_seleccion, delta, visibles);
            }
            Pantalla::Tareas => {
                self.datos.tareas_seleccion =
                    mover(self.datos.tareas_seleccion, delta, self.datos.tareas.len());
            }
            Pantalla::Trazabilidad => {
                self.datos.traza_seleccion =
                    mover(self.datos.traza_seleccion, delta, self.datos.historial.len());
            }
            Pantalla::Redis => {
                let len = self
                    .datos
                    .redis
                    .as_ref()
                    .map(|r| r.colas.len())
                    .unwrap_or(0);
                if len == 0 {
                    return;
                }
                let actual = self.datos.redis_seleccion_cola.unwrap_or(0);
                self.datos.redis_seleccion_cola = Some(mover(actual, delta, len));
            }
            Pantalla::Jornada => {
                // La jornada tiene dos tablas; con teclado movemos la de sesiones (la primera).
                if let Some(j) = &self.datos.jornada {
                    self.datos.jornada_ses_sel =
                        mover(self.datos.jornada_ses_sel, delta, j.sesiones.len());
                }
            }
            Pantalla::Broker | Pantalla::Config | Pantalla::Acceso | Pantalla::Lanzador => {}
        }
        cx.notify();
    }

    /// Abre el detalle/timeline de la fila seleccionada en la pantalla activa (tecla Enter, espejo
    /// del modal `Enter` de la TUI). En Alertas abre el panel de detalle; en Trazabilidad abre el
    /// timeline; en el resto no aplica.
    fn abrir_seleccion(&mut self, cx: &mut Context<Self>) {
        match self.activa {
            Pantalla::Alertas => {
                let idx = self.datos.alertas_seleccion;
                self.abrir_detalle_alerta(idx, cx);
            }
            Pantalla::Tareas => {
                // Paridad con el Enter de la TUI: abre el pop-up de detalle (tareas-01).
                let idx = self.datos.tareas_seleccion;
                self.abrir_detalle_tarea(idx, cx);
            }
            Pantalla::Peers => {
                // peers-01: Enter abre el detalle del peer seleccionado (o del primero si no hay).
                let idx = self.datos.peers_seleccion.unwrap_or(0);
                self.abrir_detalle_peer(idx, cx);
            }
            Pantalla::Redis => {
                // redis-01: Enter abre la inspección de la cola seleccionada.
                let id = self
                    .datos
                    .redis_seleccion_cola
                    .and_then(|i| self.datos.redis.as_ref().and_then(|r| r.colas.get(i)))
                    .map(|c| c.id.clone());
                if let Some(id) = id {
                    self.abrir_cola_redis(id, cx);
                }
            }
            Pantalla::Trazabilidad => {
                // trazabilidad-01: Enter abre el MODAL de timeline del mensaje seleccionado.
                let idx = self.datos.traza_seleccion;
                self.abrir_mensaje(idx, cx);
            }
            _ => {}
        }
    }

    /// Ejecuta la letra de acción del TUI sobre la fila seleccionada de la pantalla activa. Mapa:
    ///   - Tareas:  c=en curso, b=bloquear (form de motivo), h=hecha, x=cancelar (confirmación),
    ///              a=reasignar (selector de peer), f=forzar
    ///   - Alertas: d=descartar (la seleccionada)
    ///   - Redis:   p=purgar (la cola seleccionada)
    ///   - Trazab.: r=reenviar (el mensaje seleccionado)
    /// Devuelve `true` si la tecla correspondía a una acción de la pantalla activa (para consumirla).
    /// `window` es necesario para SEMBRAR los Inputs al abrir un formulario (tareas-04/07/08).
    fn ejecutar_letra_accion(
        &mut self,
        tecla: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use peers_core::EstadoTarea;
        match self.activa {
            Pantalla::Tareas => {
                let Some(t) = self.datos.tareas.get(self.datos.tareas_seleccion) else {
                    return false;
                };
                let tarea_id = t.id.clone();
                match tecla {
                    "c" => self.cambiar_estado_tarea(tarea_id, EstadoTarea::EnCurso, cx),
                    // Paridad con los botones del CRUD: bloquear pide MOTIVO (tareas-07), cancelar
                    // pide CONFIRMACIÓN (tareas-08) y reasignar abre el selector (tareas-04).
                    "b" => self.abrir_form_tareas(
                        crate::vista::tareas::FormTareas::Bloquear { tarea_id },
                        "",
                        "",
                        window,
                        cx,
                    ),
                    // "h" (Hecha) MISMO criterio de #7 que `boton_estado` — reusa la MISMA función
                    // pura `necesita_pedir_evidencia`, no una condición divergente escrita a mano
                    // (evita el bypass sutil de un tercer camino con su propia lógica ligeramente
                    // distinta a los otros dos).
                    "h" if crate::vista::tareas::necesita_pedir_evidencia(
                        EstadoTarea::Hecha,
                        t.evidencia.as_ref(),
                    ) =>
                    {
                        self.abrir_form_tareas(
                            crate::vista::tareas::FormTareas::PedirEvidencia { tarea_id },
                            "",
                            "",
                            window,
                            cx,
                        )
                    }
                    "h" => self.cambiar_estado_tarea(tarea_id, EstadoTarea::Hecha, cx),
                    "x" => self.pedir_confirm_estado(tarea_id, EstadoTarea::Cancelada, window, cx),
                    "a" => self.abrir_form_reasignar(tarea_id, window, cx),
                    "f" => self.forzar_tarea(tarea_id, cx),
                    _ => return false,
                }
                true
            }
            Pantalla::Peers => {
                // Paridad con la TUI: `m` = componer mensaje (peers-02), `k` = kick con
                // confirmación (peers-03), `r` = ver jornada/trazabilidad (peers-18: la RFC lista
                // `r` junto a `m`/`k` como paridad de teclado pendiente). Operan sobre el peer
                // seleccionado.
                let Some(inst) = self
                    .datos
                    .peers_seleccion
                    .and_then(|i| self.datos.instancias.get(i))
                else {
                    return false;
                };
                let id = inst.id.clone();
                match tecla {
                    "m" => self.enviar_mensaje_peer(id, window, cx),
                    "k" => self.abrir_form_peers(
                        crate::vista::peers::FormPeers::Kick { id },
                        window,
                        cx,
                    ),
                    "r" => self.ver_jornada_peer(id, cx),
                    _ => return false,
                }
                true
            }
            Pantalla::Alertas => {
                // `d` descarta, `g` va al sujeto (alertas-14, paridad con la TUI). Ambos operan
                // sobre la alerta RESALTADA en pantalla: `alertas_seleccion` es un índice VISIBLE
                // (post-filtro alertas-03), así que SIEMPRE se traduce con `indice_real_alerta`
                // antes de leer `datos.alertas` — el mismo bug que tenía `d` (descartaba la alerta
                // de esa posición en la lista TOTAL, no la resaltada, con un filtro activo) se
                // habría repetido en `g` si se le hubiera aplicado el patrón viejo por copia.
                if tecla != "d" && tecla != "g" {
                    return false;
                }
                let Some(real) = self.indice_real_alerta(self.datos.alertas_seleccion) else {
                    return false;
                };
                let Some(a) = self.datos.alertas.get(real) else {
                    return false;
                };
                let tipo = crate::vista::alertas::tipo_serializado(a.tipo).to_string();
                let sujeto = a.sujeto.clone();
                if tecla == "d" {
                    self.descartar_alerta(tipo, sujeto, cx);
                } else {
                    self.ir_al_sujeto_alerta(tipo, sujeto, cx);
                }
                true
            }
            Pantalla::Redis => {
                if tecla != "p" {
                    return false;
                }
                let id = self
                    .datos
                    .redis_seleccion_cola
                    .and_then(|i| self.datos.redis.as_ref().and_then(|r| r.colas.get(i)))
                    .map(|c| c.id.clone());
                if let Some(id) = id {
                    // redis-04: la tecla `p` también pasa por la confirmación destructiva.
                    self.pedir_purga(id, cx);
                    true
                } else {
                    false
                }
            }
            Pantalla::Trazabilidad => {
                if tecla != "r" {
                    return false;
                }
                let msg_id = self
                    .datos
                    .historial
                    .get(self.datos.traza_seleccion)
                    .map(|m| m.id);
                if let Some(msg_id) = msg_id {
                    // trazabilidad-05: la tecla `r` también pasa por la confirmación.
                    self.pedir_reenvio(msg_id, cx);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Punto único de la navegación por teclado, espejo de la TUI. Se llama desde el `on_key_down`
    /// del contenedor raíz. MAPA DE TECLAS:
    ///   - `1`..`9`            → cambia a la pantalla N (orden del sidebar `Pantalla::TODAS`).
    ///   - `tab`               → siguiente pantalla; `shift-tab` → anterior (cíclico).
    ///   - `down` / `j`        → baja la selección de fila; `up` / `k` → sube (pantalla activa).
    ///   - `enter`             → abre detalle (Alertas) / timeline (Trazabilidad) de la fila.
    ///   - `escape`            → cierra detalle/timeline o suelta selección de peer, según pantalla.
    ///   - letras de acción    → c/b/h/x/a/f (Tareas), d (Alertas), p (Redis), r (Trazabilidad).
    /// Se ignoran las pulsaciones con Cmd/Ctrl/Alt (reservadas al SO / atajos globales del kit) para
    /// no pisar copiar/pegar ni cerrar ventana.
    fn manejar_tecla(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        // No interceptamos combinaciones con modificadores de sistema (salvo shift, que usamos para
        // shift-tab): dejar pasar Cmd+C, Ctrl+…, etc.
        if m.platform || m.control || m.alt || m.function {
            return;
        }
        // GUARD GENÉRICO DE ESCRITURA (fix del bug de teclado, QA s005): si el foco lo tiene
        // CUALQUIER otro elemento — un Input del kit en Config/Acceso/Tareas/Peers o cualquier
        // pantalla futura — el usuario está ESCRIBIENDO: ninguna tecla es atajo de navegación
        // (teclear "7899" en BROKER_URL saltaba de pantalla con el guard viejo, que era una
        // lista hardcodeada de formularios). EL FOCO DECIDE, no una lista. Única excepción:
        // Escape, el "volver atrás" contextual — cierra el formulario/modal abierto aunque el
        // Input retenga el foco, y DEVUELVE el foco a la raíz para que la navegación siga viva
        // (si no, cerrar un modal con el Input enfocado dejaba el teclado muerto hasta un clic).
        let otro_con_foco =
            window.focused(cx).is_some() && !self.foco.is_focused(window);
        if otro_con_foco {
            if ks.key.as_str() == "escape" {
                if self.datos.tareas_form.is_some() {
                    self.cerrar_form_tareas(cx);
                } else if self.datos.peers_form.is_some() {
                    self.cerrar_form_peers(cx);
                } else {
                    self.manejar_escape(cx);
                }
                window.focus(&self.foco, cx);
            }
            return;
        }
        // Con un FORMULARIO abierto (CRUD de tareas o composer/kick de peers) pero el foco aún
        // en la raíz (todavía no se clicó ningún Input), las letras pertenecen al formulario:
        // sólo se intercepta Escape (cerrar). Sin este guard, teclear 'b'/'x'/'a'/'m'/'k' con
        // el modal abierto dispararía acciones de pantalla debajo del modal.
        if self.datos.tareas_form.is_some() {
            if ks.key.as_str() == "escape" {
                self.cerrar_form_tareas(cx);
            }
            return;
        }
        if self.datos.peers_form.is_some() {
            if ks.key.as_str() == "escape" {
                self.cerrar_form_peers(cx);
            }
            return;
        }
        let tecla = ks.key.as_str();
        match tecla {
            // Números 1..9 → pantallas 1ª..9ª; `0` → 10ª (Lanzador), espejo del número del sidebar.
            d @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0") => {
                if let Some(n) = d.chars().next().and_then(|c| c.to_digit(10)) {
                    // `0` mapea a la 10ª posición (índice 9); 1..9 a los índices 0..8.
                    let idx = if n == 0 { 9 } else { (n as usize) - 1 };
                    if let Some(p) = Pantalla::TODAS.get(idx) {
                        self.ir_a(*p, cx);
                    }
                }
            }
            "tab" => {
                let siguiente = if m.shift {
                    self.pantalla_relativa(-1)
                } else {
                    self.pantalla_relativa(1)
                };
                self.ir_a(siguiente, cx);
            }
            "down" | "j" => self.mover_seleccion(1, cx),
            // peers-18 (fix de paridad con la TUI, `peers_tui::main.rs:438`): `k` es AMBIGUO —
            // en el resto de pantallas sube la selección, pero en Peers es el atajo de kick
            // (peers-03). La flecha `up` siempre sube (nunca ambigua); la LETRA `k` sólo sube si
            // la pantalla activa NO es Peers — si lo es, cae a `ejecutar_letra_accion`, que ya
            // tiene el brazo `"k" => abrir_form_peers(Kick)` documentado pero antes INALCANZABLE
            // (este mismo match lo interceptaba primero). Sin este guard, el atajo de kick de
            // teclado nunca disparaba pese a estar implementado.
            "up" => self.mover_seleccion(-1, cx),
            "k" if self.activa != Pantalla::Peers => self.mover_seleccion(-1, cx),
            "enter" => self.abrir_seleccion(cx),
            "escape" => self.manejar_escape(cx),
            // Letras de acción del TUI (dependen de la pantalla activa).
            otra => {
                self.ejecutar_letra_accion(otra, window, cx);
            }
        }
    }

    /// Cierra el panel/timeline abierto o suelta la selección de peer, según la pantalla activa
    /// (tecla Escape). Es el "volver atrás" contextual, espejo del Esc de la TUI.
    fn manejar_escape(&mut self, cx: &mut Context<Self>) {
        match self.activa {
            Pantalla::Alertas => self.cerrar_detalle_alerta(cx),
            Pantalla::Tareas => {
                // El formulario del CRUD tiene prioridad (está por encima del detalle).
                if self.datos.tareas_form.is_some() {
                    self.cerrar_form_tareas(cx);
                } else {
                    self.cerrar_detalle_tarea(cx);
                }
            }
            Pantalla::Trazabilidad => {
                // Prioridad: confirmación de reenvío > modal de timeline (volver atrás contextual).
                if self.datos.traza_confirmar_reenvio.is_some() {
                    self.cancelar_reenvio(cx);
                } else {
                    self.cerrar_timeline(cx);
                }
            }
            Pantalla::Peers => {
                // Prioridad: formulario > detalle > selección (el "volver atrás" contextual).
                if self.datos.peers_form.is_some() {
                    self.cerrar_form_peers(cx);
                } else if self.datos.peer_detalle.is_some() {
                    self.cerrar_detalle_peer(cx);
                } else {
                    self.deseleccionar_peer(cx);
                }
            }
            Pantalla::Jornada => {
                // Prioridad: dropdown > detalle de tarea > detalle de sesión.
                if self.datos.jornada_dropdown {
                    self.datos.jornada_dropdown = false;
                    cx.notify();
                } else if self.datos.jornada_tarea_detalle.is_some() {
                    self.cerrar_detalle_tarea(cx);
                } else {
                    self.cerrar_sesion_jornada(cx);
                }
            }
            Pantalla::Redis => {
                // Prioridad: confirmación de reenvío > confirmación de purga > pop-up de cola.
                if self.datos.traza_confirmar_reenvio.is_some() {
                    self.cancelar_reenvio(cx);
                } else if self.datos.redis_confirmar_purga.is_some() {
                    self.cancelar_purga(cx);
                } else {
                    self.cerrar_cola_redis(cx);
                }
            }
            _ => {}
        }
    }

    /// Pantalla situada `delta` posiciones respecto a la activa en `Pantalla::TODAS`, de forma
    /// cíclica (para Tab / Shift-Tab). `delta` puede ser negativo.
    fn pantalla_relativa(&self, delta: i32) -> Pantalla {
        let todas = Pantalla::TODAS;
        let n = todas.len() as i32;
        let pos = todas.iter().position(|p| *p == self.activa).unwrap_or(0) as i32;
        let idx = (((pos + delta) % n) + n) % n; // módulo positivo
        todas[idx as usize]
    }

    /// Construye un ítem del sidebar con el tema Ethos: la pantalla activa se marca con acento brasa
    /// (fondo dorado tenue + borde izquierdo dorado + texto papel), las inactivas van en humo con
    /// hover a TINTA2. Un número de atajo (1..9) a la izquierda, espejo de la navegación por teclado.
    /// Click → navega (`cx.listener` para poder mutar `self`).
    fn item_nav(&self, pantalla: Pantalla, indice: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let activa = self.activa == pantalla;
        // Número de atajo alineado a la izquierda, en mono/humo (o brasa si activa). Las 9 primeras
        // van 1..9; la 10ª muestra "0" (su atajo de teclado es la tecla 0, no "10" de dos dígitos).
        let etiqueta_atajo = if indice == 9 {
            "0".to_string()
        } else {
            (indice + 1).to_string()
        };
        let numero = div()
            .w(gpui::px(18.0))
            .font_family(tema::FUENTE_MONO)
            .text_xs()
            .text_color(if activa { tema::BRASA } else { tema::HUMO })
            .child(SharedString::from(etiqueta_atajo));

        let etiqueta = div()
            .flex_1()
            .text_color(if activa { tema::PAPEL } else { tema::HUMO })
            .when(activa, |d| d.font_weight(FontWeight::MEDIUM))
            .child(SharedString::from(pantalla.titulo()));

        let base = div()
            .id(pantalla.id())
            .h_flex()
            .items_center()
            .w_full()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(tema::radio(tema::RADIO_CONTROL))
            // Borde izquierdo SIEMPRE presente (transparente en reposo) para que el ítem no salte
            // 2px al activarse; sólo cambia el color — mismo criterio que `tema::fila_seleccionable`.
            .border_l_2()
            .cursor_pointer();

        let base = if activa {
            base.bg(tema::BRASA_TENUE).border_color(tema::BRASA)
        } else {
            base.border_color(gpui::rgba(0x00000000))
                .hover(|s| s.bg(tema::TINTA2))
        };

        base.child(numero)
            .child(etiqueta)
            .on_click(cx.listener(move |esta, _evento, _window, cx| {
                esta.ir_a(pantalla, cx);
            }))
    }

    /// Delega el render del área de contenido en el stub de la pantalla activa. Cada rama
    /// devuelve un tipo `impl IntoElement` distinto, por eso se envuelve en `.into_any_element()`.
    fn contenido(&self) -> gpui::AnyElement {
        use gpui::IntoElement as _;
        match self.activa {
            Pantalla::Peers => vista::render_peers(&self.datos).into_any_element(),
            Pantalla::Alertas => vista::render_alertas(&self.datos).into_any_element(),
            Pantalla::Broker => vista::render_broker(&self.datos).into_any_element(),
            Pantalla::Config => vista::render_config(&self.datos).into_any_element(),
            Pantalla::Jornada => vista::render_jornada(&self.datos).into_any_element(),
            Pantalla::Redis => vista::render_redis(&self.datos).into_any_element(),
            Pantalla::Tareas => vista::render_tareas(&self.datos).into_any_element(),
            Pantalla::Trazabilidad => vista::render_trazabilidad(&self.datos).into_any_element(),
            Pantalla::Acceso => vista::render_acceso(&self.datos).into_any_element(),
            Pantalla::Lanzador => vista::render_lanzador(&self.datos).into_any_element(),
        }
    }

    /// Overlay del MODAL de detalle de alerta (alertas-01), si `alerta_detalle` apunta a una alerta
    /// válida. Devuelve un backdrop `absolute inset_0` (tinta translúcido) que centra el contenido
    /// puro `render_modal_detalle`. Clic en el backdrop → `CerrarDetalleAlerta` (clic fuera cierra);
    /// el contenido usa `occlude` para que el clic dentro NO cierre. El `Esc` ya lo cubre
    /// `manejar_escape` → `cerrar_detalle_alerta`. Se monta como último hijo del render raíz para
    /// quedar POR ENCIMA del layout de dos columnas.
    ///
    /// DECISIÓN DE DISEÑO (por qué overlay propio y no el `Dialog` del kit): igual que la Fundación
    /// evitó el `Sidebar` stateful, el `Dialog` del kit exige una `Entity` + `window.open_dialog`,
    /// lo que rompería el patrón "vista pura + estado en `EstadoPantalla`" que respeta toda la app.
    /// Un overlay `absolute` con backdrop da el mismo resultado (foco visual, backdrop, Esc, clic
    /// fuera) sin `Entity`, leyendo `datos.alerta_detalle` como el resto de modales inline previos.
    fn overlay_alerta(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let idx = self.datos.alerta_detalle?;
        let alerta = self.datos.alertas.get(idx)?;
        let contenido = crate::vista::alertas::render_modal_detalle(
            alerta,
            self.datos.alertas_seleccion,
            self.datos.alerta_confirmar_descarte,
        );

        Some(
            div()
                .id("overlay-alerta")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                // Backdrop tinta translúcido: atenúa el fondo y capta el clic-fuera.
                .bg(gpui::rgba(0x0A_08_06_CC))
                // Clic en el backdrop cierra el modal (clic fuera).
                .on_click(|_e, window, cx| {
                    window.dispatch_action(
                        Box::new(crate::vista::alertas::CerrarDetalleAlerta),
                        cx,
                    );
                })
                // El contenido ocluye el ratón para que un clic DENTRO del modal no burbujee al
                // backdrop (no cierre). `occlude` es la vía del propio kit para esto.
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del POP-UP de detalle de tarea (tareas-01), si `tarea_detalle` apunta a una tarea
    /// válida. Calco de `overlay_alerta` (misma decisión de diseño: overlay `absolute` propio en vez
    /// del `Dialog` stateful del kit, para conservar el patrón "vista pura + estado en
    /// `EstadoPantalla`"). Backdrop tinta translúcido que centra `render_modal_detalle_tarea`;
    /// clic en el backdrop → `CerrarDetalleTarea`; el contenido `occlude` para que el clic dentro
    /// no cierre. El `Esc` lo cubre `manejar_escape` → `cerrar_detalle_tarea`.
    fn overlay_tarea(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let idx = self.datos.tarea_detalle?;
        let tarea = self.datos.tareas.get(idx)?;
        let contenido = crate::vista::tareas::render_modal_detalle_tarea(tarea);

        Some(
            div()
                .id("overlay-tarea")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(
                        Box::new(crate::vista::tareas::CerrarDetalleTarea),
                        cx,
                    );
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del POP-UP de detalle de peer (peers-01), si `peer_detalle` apunta a un peer
    /// válido. Calco del patrón `overlay_tarea` (backdrop, clic-fuera cierra, contenido occlude).
    fn overlay_peer(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let idx = self.datos.peer_detalle?;
        let inst = self.datos.instancias.get(idx)?;
        let contenido = crate::vista::peers::render_modal_detalle_peer(inst, &self.datos);

        Some(
            div()
                .id("overlay-peer")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::peers::CerrarDetallePeer), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del detalle de TAREA de la jornada (jornada-01/04): REUTILIZA íntegro el modal de
    /// la fase 1 (`tareas::render_modal_detalle_tarea`) y le añade debajo, en la misma columna, la
    /// card de TRANSICIONES (jornada-04). Cerrar despacha `tareas::CerrarDetalleTarea` (la misma
    /// acción que usan el ✕/Cerrar del modal reutilizado).
    fn overlay_tarea_jornada(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let idx = self.datos.jornada_tarea_detalle?;
        let tarea = self.datos.jornada.as_ref()?.tareas.get(idx)?;
        let detalle = crate::vista::tareas::render_modal_detalle_tarea(tarea);
        let acciones = crate::vista::jornada::render_acciones_tarea_jornada(tarea);

        Some(
            div()
                .id("overlay-tarea-jornada")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::tareas::CerrarDetalleTarea), cx);
                })
                .child(
                    div()
                        .occlude()
                        .v_flex()
                        .gap_3()
                        .child(detalle)
                        .child(acciones),
                )
                .into_any_element(),
        )
    }

    /// Overlay del detalle de SESIÓN (jornada-02), con las tareas correlacionadas por `sesion_id`.
    fn overlay_sesion_jornada(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let idx = self.datos.jornada_sesion_detalle?;
        let jornada = self.datos.jornada.as_ref()?;
        let sesion = jornada.sesiones.get(idx)?;
        let contenido = crate::vista::jornada::render_modal_sesion(sesion, &jornada.tareas);

        Some(
            div()
                .id("overlay-sesion-jornada")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(
                        Box::new(crate::vista::jornada::CerrarSesionJornada),
                        cx,
                    );
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del MODAL de timeline de mensaje (trazabilidad-01/04), si `traza_timeline` está
    /// activo y la selección apunta a un mensaje válido. Mismo patrón que el resto de overlays.
    fn overlay_mensaje(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        if !self.datos.traza_timeline {
            return None;
        }
        let sel = self
            .datos
            .traza_seleccion
            .min(self.datos.historial.len().checked_sub(1)?);
        let mensaje = self.datos.historial.get(sel)?;
        let contenido = crate::vista::trazabilidad::render_modal_timeline(mensaje);

        Some(
            div()
                .id("overlay-mensaje")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::trazabilidad::CerrarTimeline), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del pop-up de INSPECCIÓN de cola de Redis (redis-01/03).
    fn overlay_cola_redis(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let id = self.datos.redis_cola_detalle.as_deref()?;
        let contenido = crate::vista::redis::render_modal_cola(
            id,
            &self.datos.redis_cola_mensajes,
            self.datos.redis_cola_cargando,
        );

        Some(
            div()
                .id("overlay-cola-redis")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::redis::CerrarColaRedis), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del diálogo de CONFIRMACIÓN de purga (redis-04), con los recuentos exactos sacados
    /// del snapshot ya cargado. Clic fuera = cancelar (no purga).
    fn overlay_purga_redis(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let id = self.datos.redis_confirmar_purga.as_deref()?;
        // Recuentos exactos del snapshot (0 si el peer ya no aparece en él).
        let contar = |lista: &[peers_core::ColaResumen]| {
            lista
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.pendientes)
                .unwrap_or(0)
        };
        let (en_cola, en_outbox) = match &self.datos.redis {
            Some(r) => (contar(&r.colas), contar(&r.outbox)),
            None => (0, 0),
        };
        let contenido = crate::vista::redis::render_modal_purga(id, en_cola, en_outbox);

        Some(
            div()
                .id("overlay-purga-redis")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::redis::CancelarPurga), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del mini-modal de CONFIRMACIÓN de reenvío (trazabilidad-05), por encima del modal
    /// de timeline si ambos están abiertos. Clic fuera = cancelar (no reenvía).
    fn overlay_confirmar_reenvio(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let msg_id = self.datos.traza_confirmar_reenvio?;
        // El mensaje original, si sigue en alguna caché, da contexto al texto de confirmación
        // (historial de Trazabilidad o la cola inspeccionada de Redis, redis-03).
        let mensaje = self
            .datos
            .historial
            .iter()
            .find(|m| m.id == msg_id)
            .or_else(|| self.datos.redis_cola_mensajes.iter().find(|m| m.id == msg_id));
        let contenido =
            crate::vista::trazabilidad::render_modal_confirmar_reenvio(msg_id, mensaje);

        Some(
            div()
                .id("overlay-confirmar-reenvio")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::trazabilidad::CancelarReenvio), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del FORMULARIO de Peers (composer de mensaje / confirmación de kick), si hay uno
    /// abierto. Mismo patrón; se monta por encima del detalle.
    fn overlay_form_peers(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let form = self.datos.peers_form.as_ref()?;
        let contenido = crate::vista::peers::render_modal_form_peers(form, &self.datos);

        Some(
            div()
                .id("overlay-form-peers")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::peers::CerrarFormPeers), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }

    /// Overlay del FORMULARIO del CRUD de tareas (tareas-03..08), si hay uno abierto. Mismo patrón
    /// que `overlay_tarea`/`overlay_alerta`: backdrop translúcido, clic fuera cierra (sin
    /// confirmar), contenido `occlude`. Se monta el ÚLTIMO (por encima del detalle).
    fn overlay_form_tareas(&self) -> Option<gpui::AnyElement> {
        use gpui::IntoElement as _;
        let form = self.datos.tareas_form.as_ref()?;
        let contenido = crate::vista::tareas::render_modal_form(form, &self.datos);

        Some(
            div()
                .id("overlay-form-tareas")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0A_08_06_CC))
                .on_click(|_e, window, cx| {
                    window.dispatch_action(Box::new(crate::vista::tareas::CerrarFormTareas), cx);
                })
                .child(div().occlude().child(contenido))
                .into_any_element(),
        )
    }
}

impl Render for AppDesktop {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Sidebar navegable (tema Ethos): cabecera con marca + un ítem por pantalla, en orden
        // canónico. Cada ítem lleva su número de atajo (1..9) espejo de la navegación por teclado.
        let items = Pantalla::TODAS
            .iter()
            .enumerate()
            .map(|(i, p)| self.item_nav(*p, i, cx))
            .collect::<Vec<_>>();

        // Cabecera del sidebar: eyebrow discreto + título de marca en la fuente display (Fraunces
        // fallback). Reemplaza el `text_lg` plano previo.
        let cabecera_sidebar = div()
            .v_flex()
            .gap_1()
            .px_3()
            .py_2()
            .pb_4()
            .child(tema::eyebrow("red"))
            .child(tema::titulo("claude-peers").text_size(gpui::px(20.0)));

        let sidebar = div()
            .v_flex()
            .h_full()
            .w(gpui::px(232.0))
            .flex_shrink_0()
            .gap_1()
            .p_3()
            // Fondo del sidebar un paso por encima de la tinta base (superficie tenue) + borde
            // derecho de separación con el área de contenido.
            .bg(tema::TINTA2)
            .border_r_1()
            .border_color(tema::LINEA)
            .child(cabecera_sidebar)
            .children(items)
            // Empuja el pie de marca al fondo del sidebar (spacer flexible).
            .child(div().flex_1())
            // Pie de marca Lexusfx: crédito del autor con el tema Ethos (eyebrow en mono/humo,
            // marca en dorado brasa, email en mono tenue). Identidad del producto, discreta.
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(tema::LINEA)
                    .child(
                        div()
                            .font_family("IBM Plex Mono")
                            .text_size(gpui::px(10.0))
                            .text_color(tema::HUMO)
                            .child(SharedString::from("LEXUSFX")),
                    )
                    .child(
                        div()
                            .text_size(gpui::px(12.0))
                            .text_color(tema::BRASA)
                            .child(SharedString::from("Max")),
                    )
                    .child(
                        div()
                            .font_family("IBM Plex Mono")
                            .text_size(gpui::px(10.0))
                            .text_color(tema::HUMO)
                            .child(SharedString::from("Max@lexusfx.com")),
                    ),
            );

        // Área de contenido: la pantalla activa. Ocupa el resto del ancho y hace scroll si su
        // contenido excede el alto de la ventana (las pantallas largas —jornada, broker— no se
        // cortan). `min_w_0` deja que el flex encoja el hijo en vez de desbordar el sidebar.
        // SCROLL TRANSVERSAL (fix): el contenedor de la pantalla activa debe acotarse al alto
        // disponible de la ventana y scrollear su contenido interno. Claves:
        //  - `h_full` en un hijo de `h_flex` con `size_full` toma el alto de la ventana (referencia
        //    concreta, no "el contenido"), y `min_h_0` permite que el scroll interno recorte.
        //  - `overflow_y_scroll` sobre ESTE contenedor. El hijo `self.contenido()` NO debe imponer
        //    su propio alto (las vistas usan v_flex+gap sin h_full, así crecen y este contenedor
        //    scrollea). Sin esto, las pantallas altas (broker/config/acceso/lanzador) quedan
        //    cortadas abajo y no se puede bajar.
        let contenido = div()
            .id("contenido-scroll")
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .child(self.contenido());

        // Acciones de las 9 pantallas. Cada vista pura DESPACHA su Action (`window.dispatch_action`);
        // GPUI la burbujea hasta este contenedor raíz, donde `cx.listener` permite mutar el estado y
        // (si aplica) disparar el fetch + recarga. Es el puente único vista-pura → estado de la app.
        use crate::vista::acceso::{AplicarConexion, ProbarConexion, RecargarAcceso};
        use crate::vista::alertas::{
            AbrirDetalle, AlternarFiltroTipo, CancelarConfirmacionDescarte, CerrarDetalleAlerta,
            Descartar, IrAlSujeto, LimpiarFiltroTipos, PedirConfirmacionDescarte,
        };
        use crate::vista::broker::RecargarBroker;
        use crate::vista::config::RecargarConfig;
        use crate::vista::jornada::{
            AbrirSesionJornada, AbrirTareaJornada, AlternarDropdownJornada,
            AlternarSeccionJornada, CambiarEstadoTareaJornada, CerrarSesionJornada,
            CiclarPeerJornada, ElegirPeerJornada, SeleccionarSesion, SeleccionarTareaJornada,
        };
        use crate::vista::peers::{
            AbrirDetallePeer, CerrarDetallePeer, CerrarFormPeers, ConfirmarFormPeers,
            DeseleccionarPeer, EnviarMensajePeer, PedirKickPeer, SeleccionarPeer, VerJornadaPeer,
        };
        use crate::vista::redis::{
            AbrirColaRedis, CancelarPurga, CerrarColaRedis, ConfirmarPurga, PedirPurga,
            PurgarPeer, RefrescarRedis, SeleccionarColaRedis,
        };
        use crate::vista::tareas::{
            AbrirDetalleTarea, AbrirFormAmpliar, AbrirFormAsignar, AbrirFormBloquear,
            AbrirFormEditar, AbrirFormReasignar, AsignarTarea, CambiarEstadoTarea,
            CerrarDetalleTarea, CerrarFormTareas, ConfirmarFormTareas, ElegirPeerForm, ForzarTarea,
            PedirConfirmEstado, ReasignarTarea, SeleccionarTarea,
        };
        use crate::vista::trazabilidad::{
            AbrirMensaje, CancelarReenvio, CerrarTimeline, ConfirmarReenvio, EnfocarPeer,
            FiltrarEstadoTraza, PedirReenvio, ReenviarMensaje, SeleccionarMensaje,
        };

        // Layout raíz de dos columnas sobre el fondo Ethos. `track_focus` + el enfoque inicial de
        // `nueva` habilitan la captura de teclado (`on_key_down`); sin él, GPUI no entrega teclas.
        tema::fondo_app()
            .h_flex()
            .track_focus(&self.foco)
            .on_key_down(cx.listener(|esta, ev: &gpui::KeyDownEvent, window, cx| {
                esta.manejar_tecla(ev, window, cx);
            }))
            // --- Alertas (ya existían) ---
            .on_action(cx.listener(|esta, accion: &AbrirDetalle, _window, cx| {
                esta.abrir_detalle_alerta(accion.indice, cx);
            }))
            .on_action(cx.listener(|esta, _accion: &CerrarDetalleAlerta, _window, cx| {
                esta.cerrar_detalle_alerta(cx);
            }))
            .on_action(cx.listener(|esta, accion: &Descartar, _window, cx| {
                esta.descartar_alerta(accion.tipo.clone(), accion.sujeto.clone(), cx);
            }))
            // --- Alertas (nuevas: RFC alertas-02/03/09) ---
            // alertas-02: "Ir al sujeto" (portar tecla `g` de la TUI).
            .on_action(cx.listener(|esta, a: &IrAlSujeto, _window, cx| {
                esta.ir_al_sujeto_alerta(a.tipo.clone(), a.sujeto.clone(), cx);
            }))
            // alertas-03: filtro por tipo (chips multi-select).
            .on_action(cx.listener(|esta, a: &AlternarFiltroTipo, _window, cx| {
                esta.alternar_filtro_tipo(a.tipo.clone(), cx);
            }))
            .on_action(cx.listener(|esta, _a: &LimpiarFiltroTipos, _window, cx| {
                esta.limpiar_filtro_tipos(cx);
            }))
            // alertas-09: confirmación de descarte en 2 pasos dentro del modal.
            .on_action(cx.listener(|esta, _a: &PedirConfirmacionDescarte, _window, cx| {
                esta.pedir_confirmacion_descarte(cx);
            }))
            .on_action(cx.listener(|esta, _a: &CancelarConfirmacionDescarte, _window, cx| {
                esta.cancelar_confirmacion_descarte(cx);
            }))
            // --- Peers ---
            .on_action(cx.listener(|esta, a: &SeleccionarPeer, _window, cx| {
                esta.seleccionar_peer(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, _a: &DeseleccionarPeer, _window, cx| {
                esta.deseleccionar_peer(cx);
            }))
            .on_action(cx.listener(|esta, a: &VerJornadaPeer, _window, cx| {
                esta.ver_jornada_peer(a.id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, a: &EnviarMensajePeer, window, cx| {
                esta.enviar_mensaje_peer(a.id.clone(), window, cx);
            }))
            // --- Peers: detalle + composer + kick (peers-01/02/03) ---
            .on_action(cx.listener(|esta, a: &AbrirDetallePeer, _window, cx| {
                esta.abrir_detalle_peer(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarDetallePeer, _window, cx| {
                esta.cerrar_detalle_peer(cx);
            }))
            .on_action(cx.listener(|esta, a: &PedirKickPeer, window, cx| {
                esta.abrir_form_peers(
                    crate::vista::peers::FormPeers::Kick { id: a.id.clone() },
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|esta, _a: &ConfirmarFormPeers, _window, cx| {
                esta.confirmar_form_peers(cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarFormPeers, _window, cx| {
                esta.cerrar_form_peers(cx);
            }))
            // --- Trazabilidad ---
            .on_action(cx.listener(|esta, a: &EnfocarPeer, _window, cx| {
                esta.enfocar_traza(a.id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, a: &SeleccionarMensaje, _window, cx| {
                esta.seleccionar_mensaje(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, a: &ReenviarMensaje, _window, cx| {
                esta.reenviar_mensaje(a.msg_id, cx);
            }))
            // --- Trazabilidad: modal de timeline + reenvío con confirmación + filtro (fase 4) ---
            .on_action(cx.listener(|esta, a: &AbrirMensaje, _window, cx| {
                esta.abrir_mensaje(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarTimeline, _window, cx| {
                esta.cerrar_timeline(cx);
            }))
            .on_action(cx.listener(|esta, a: &PedirReenvio, _window, cx| {
                esta.pedir_reenvio(a.msg_id, cx);
            }))
            .on_action(cx.listener(|esta, _a: &ConfirmarReenvio, _window, cx| {
                esta.confirmar_reenvio(cx);
            }))
            .on_action(cx.listener(|esta, _a: &CancelarReenvio, _window, cx| {
                esta.cancelar_reenvio(cx);
            }))
            .on_action(cx.listener(|esta, a: &FiltrarEstadoTraza, _window, cx| {
                esta.filtrar_estado_traza(a.estado, cx);
            }))
            // --- Tareas ---
            .on_action(cx.listener(|esta, a: &SeleccionarTarea, _window, cx| {
                esta.seleccionar_tarea(a.indice, cx);
            }))
            // tareas-01: pop-up de detalle (doble-click en la fila, botón "Abrir" o Enter).
            .on_action(cx.listener(|esta, a: &AbrirDetalleTarea, _window, cx| {
                esta.abrir_detalle_tarea(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarDetalleTarea, _window, cx| {
                esta.cerrar_detalle_tarea(cx);
            }))
            .on_action(cx.listener(|esta, a: &CambiarEstadoTarea, _window, cx| {
                esta.cambiar_estado_tarea(a.tarea_id.clone(), a.estado, cx);
            }))
            .on_action(cx.listener(|esta, a: &ReasignarTarea, _window, cx| {
                esta.reasignar_tarea(a.tarea_id.clone(), a.nuevo_instancia_id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, a: &ForzarTarea, _window, cx| {
                esta.forzar_tarea(a.tarea_id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, a: &AsignarTarea, _window, cx| {
                esta.asignar_tarea(a.instancia_id.clone(), a.descripcion.clone(), a.estimado_seg, cx);
            }))
            // --- Tareas: formularios del CRUD (tareas-03..08) ---
            .on_action(cx.listener(|esta, _a: &AbrirFormAsignar, window, cx| {
                esta.abrir_form_tareas(
                    crate::vista::tareas::FormTareas::Asignar,
                    "",
                    "",
                    window,
                    cx,
                );
                // jornada-05: el MISMO formulario se abre también desde la pantalla Jornada
                // ("quiero crear tareas desde la jornada"). Si se abre ahí, el peer cuya jornada
                // se está mirando es la elección natural → se preselecciona en el selector
                // (sigue siendo cambiable; en Tareas el comportamiento no varía: sin preselección).
                if esta.activa == Pantalla::Jornada {
                    if let Some(peer) = esta.datos.jornada_peer.clone() {
                        esta.elegir_peer_form(peer, cx);
                    }
                }
            }))
            .on_action(cx.listener(|esta, a: &AbrirFormReasignar, window, cx| {
                esta.abrir_form_reasignar(a.tarea_id.clone(), window, cx);
            }))
            .on_action(cx.listener(|esta, a: &AbrirFormEditar, window, cx| {
                esta.abrir_form_editar(a.tarea_id.clone(), window, cx);
            }))
            .on_action(cx.listener(|esta, a: &AbrirFormAmpliar, window, cx| {
                esta.abrir_form_ampliar(a.tarea_id.clone(), window, cx);
            }))
            .on_action(cx.listener(|esta, a: &AbrirFormBloquear, window, cx| {
                esta.abrir_form_tareas(
                    crate::vista::tareas::FormTareas::Bloquear {
                        tarea_id: a.tarea_id.clone(),
                    },
                    "",
                    "",
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|esta, a: &crate::vista::tareas::AbrirFormEvidencia, window, cx| {
                esta.abrir_form_tareas(
                    crate::vista::tareas::FormTareas::PedirEvidencia {
                        tarea_id: a.tarea_id.clone(),
                    },
                    "",
                    "",
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|esta, a: &PedirConfirmEstado, window, cx| {
                esta.pedir_confirm_estado(a.tarea_id.clone(), a.estado, window, cx);
            }))
            .on_action(cx.listener(|esta, a: &ElegirPeerForm, _window, cx| {
                esta.elegir_peer_form(a.id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, _a: &ConfirmarFormTareas, _window, cx| {
                esta.confirmar_form_tareas(cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarFormTareas, _window, cx| {
                esta.cerrar_form_tareas(cx);
            }))
            // --- Redis ---
            .on_action(cx.listener(|esta, a: &SeleccionarColaRedis, _window, cx| {
                esta.seleccionar_cola_redis(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, a: &PurgarPeer, _window, cx| {
                esta.purgar_peer(a.id.clone(), cx);
            }))
            // --- Redis: inspección de cola + purga confirmada + refresco (fase 6) ---
            .on_action(cx.listener(|esta, a: &AbrirColaRedis, _window, cx| {
                esta.abrir_cola_redis(a.id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarColaRedis, _window, cx| {
                esta.cerrar_cola_redis(cx);
            }))
            .on_action(cx.listener(|esta, a: &PedirPurga, _window, cx| {
                esta.pedir_purga(a.id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, _a: &ConfirmarPurga, _window, cx| {
                esta.confirmar_purga(cx);
            }))
            .on_action(cx.listener(|esta, _a: &CancelarPurga, _window, cx| {
                esta.cancelar_purga(cx);
            }))
            .on_action(cx.listener(|esta, _a: &RefrescarRedis, _window, cx| {
                esta.cargar_redis(cx);
            }))
            // --- Jornada (sólo UI) ---
            .on_action(cx.listener(|esta, a: &SeleccionarSesion, _window, cx| {
                esta.seleccionar_sesion_jornada(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, a: &SeleccionarTareaJornada, _window, cx| {
                esta.seleccionar_tarea_jornada(a.indice, cx);
            }))
            // --- Jornada: detalle de tarea/sesión + selector de peer A+B (fase 5) ---
            .on_action(cx.listener(|esta, a: &AbrirTareaJornada, _window, cx| {
                esta.abrir_tarea_jornada(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, a: &AbrirSesionJornada, _window, cx| {
                esta.abrir_sesion_jornada(a.indice, cx);
            }))
            .on_action(cx.listener(|esta, _a: &CerrarSesionJornada, _window, cx| {
                esta.cerrar_sesion_jornada(cx);
            }))
            .on_action(cx.listener(|esta, _a: &AlternarDropdownJornada, _window, cx| {
                esta.alternar_dropdown_jornada(cx);
            }))
            .on_action(cx.listener(|esta, a: &AlternarSeccionJornada, _window, cx| {
                esta.alternar_seccion_jornada(a.seccion, cx);
            }))
            .on_action(cx.listener(|esta, a: &ElegirPeerJornada, _window, cx| {
                esta.elegir_peer_jornada(a.id.clone(), cx);
            }))
            .on_action(cx.listener(|esta, a: &CiclarPeerJornada, _window, cx| {
                esta.ciclar_peer_jornada(a.delta, cx);
            }))
            .on_action(cx.listener(|esta, a: &CambiarEstadoTareaJornada, window, cx| {
                esta.cambiar_estado_tarea_jornada(a.tarea_id.clone(), a.estado, window, cx);
            }))
            // --- Broker / Acceso (recarga manual) ---
            .on_action(cx.listener(|esta, _a: &RecargarBroker, _window, cx| {
                esta.cargar_broker(cx);
            }))
            .on_action(cx.listener(|esta, _a: &RecargarAcceso, _window, cx| {
                esta.cargar_broker(cx);
            }))
            // acceso-01/02: aplicar nueva conexión (persistir + reconfigurar cliente + resembrar + recargar).
            .on_action(cx.listener(|esta, a: &AplicarConexion, window, cx| {
                esta.aplicar_conexion(a.broker_url.clone(), a.token.clone(), window, cx);
            }))
            // acceso-05: check dedicado de conexión (salud + token) sin recargar todo.
            .on_action(cx.listener(|esta, _a: &ProbarConexion, _window, cx| {
                esta.probar_conexion(false, cx);
            }))
            // config-01: el MISMO check, pedido desde la pestaña Config (resultado a su panel).
            .on_action(cx.listener(|esta, _a: &crate::vista::config::ProbarConexionConfig, _window, cx| {
                esta.probar_conexion(true, cx);
            }))
            // --- Config (recargar desde disco; delega en el panel stateful) ---
            .on_action(cx.listener(|esta, _a: &RecargarConfig, window, cx| {
                if let Some(panel) = &esta.datos.panel_config {
                    panel.update(cx, |p, cx| p.recargar(window, cx));
                }
            }))
            .child(sidebar)
            .child(contenido)
            // Overlay del modal de detalle de alerta (alertas-01), por encima del layout. `children`
            // acepta el `Option` (0 o 1 elemento): sin modal abierto no pinta nada.
            .children(self.overlay_alerta())
            // Overlay del pop-up de detalle de tarea (tareas-01), mismo patrón.
            .children(self.overlay_tarea())
            // Overlay del pop-up de detalle de peer (peers-01).
            .children(self.overlay_peer())
            // Overlay del modal de timeline de mensaje (trazabilidad-01).
            .children(self.overlay_mensaje())
            // Overlays de la jornada: detalle de tarea (reutiliza el modal de fase 1) y de sesión.
            .children(self.overlay_tarea_jornada())
            .children(self.overlay_sesion_jornada())
            // Overlays de Redis: inspección de cola (redis-01) y confirmación de purga (redis-04).
            .children(self.overlay_cola_redis())
            .children(self.overlay_purga_redis())
            // Overlays de formularios (CRUD de tareas y composer/kick de peers), por encima.
            .children(self.overlay_form_tareas())
            .children(self.overlay_form_peers())
            // Confirmación de reenvío (trazabilidad-05): el último, por encima del timeline.
            .children(self.overlay_confirmar_reenvio())
    }
}
