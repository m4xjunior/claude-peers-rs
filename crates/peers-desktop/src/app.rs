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
    div, prelude::FluentBuilder, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window,
};
// `v_flex`/`h_flex` viven en el trait `StyledExt` del kit (no en el `Styled` de gpui).
use gpui_component::StyledExt;

use crate::cliente::ClienteBroker;
use crate::vista;
use crate::vista::config::PanelConfig;

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
}

impl Pantalla {
    /// Orden de aparición en el sidebar. Fuente única para pintar la navegación y para
    /// no repetir la lista en varios sitios (añadir/quitar pantalla = tocar sólo aquí).
    pub const TODAS: [Pantalla; 9] = [
        Pantalla::Peers,
        Pantalla::Alertas,
        Pantalla::Broker,
        Pantalla::Config,
        Pantalla::Jornada,
        Pantalla::Redis,
        Pantalla::Tareas,
        Pantalla::Trazabilidad,
        Pantalla::Acceso,
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

    // --- Pantalla Trazabilidad (Fase 2) ---
    /// Peer en foco cuyo historial se muestra (espejo de `traza_peer_actual` de la TUI).
    /// `None` = aún no hay peer seleccionado en la pantalla Peers.
    pub traza_peer: Option<String>,
    /// Historial durable de la cola del peer en foco (`GET /admin/historial`), en orden
    /// cronológico ascendente tal como lo devuelve el broker. Vacío si no hay foco o datos.
    pub historial: Vec<peers_core::Mensaje>,
    /// Índice de la fila seleccionada en la tabla de trazabilidad. Sirve para resaltar la fila
    /// y decidir qué mensaje expande su timeline. Se mantiene dentro de `historial`.
    pub traza_seleccion: usize,
    /// Si `true`, se muestra el timeline completo (transiciones + timestamps) del mensaje
    /// seleccionado. Espejo del modal `Enter` de la TUI; aquí se despliega inline.
    pub traza_timeline: bool,

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
    /// Si `Some(i)`, se muestra el panel de DETALLE de la alerta `alertas[i]` con su texto
    /// íntegro (la celda de la tabla lo recorta). Espejo del modal `Enter` de la TUI; aquí se
    /// despliega inline para no depender del `Dialog` stateful del kit. `None` = tabla sin panel.
    pub alerta_detalle: Option<usize>,
    /// Último error al hablar con el broker desde la pantalla Alertas (cargar lista o descartar).
    /// Se pinta como banner en vez de dejar la tabla muda. Separado de `error_peers` a propósito.
    pub error_alertas: Option<crate::cliente::ErrorBroker>,

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
    /// Último error al comprobar el acceso al broker (cargar `/salud` o `/admin/info`). La pantalla
    /// lo pinta como banner (offline vs 401 vs otro) para explicar por qué faltan datos.
    pub error_acceso: Option<crate::cliente::ErrorBroker>,

    // --- Pantalla Redis (Fase 2) ---
    /// Colas de mensajes + outbox pendientes por peer (`GET /admin/redis`). `None` mientras no
    /// haya llegado la primera respuesta (o si el broker está offline); la pantalla pinta el
    /// aviso "sin datos todavía" en ese caso, no dos tablas vacías sin explicación.
    pub redis: Option<peers_core::RespuestaAdminRedis>,
    /// Último error al hablar con el broker desde la pantalla Redis (cargar colas o purgar). Se
    /// pinta como banner (offline vs 401 vs otro) en vez de dejar las tablas mudas. Separado del
    /// resto de `error_*` a propósito: cada pantalla explica su propio fallo.
    pub error_redis: Option<crate::cliente::ErrorBroker>,

    // --- Pantalla Tareas (Fase 2) ---
    /// TODAS las tareas de TODOS los peers (`GET /admin/tareas`), cada `Tarea` con su `instancia_id`,
    /// ordenadas por el broker (inicio desc). Alimentan la tabla global de la pantalla Tareas con el
    /// estado coloreado, estimado vs real y la columna dueño. Vacío = sin tareas o aún sin cargar.
    pub tareas: Vec<peers_core::Tarea>,
    /// Último error al hablar con el broker desde la pantalla Tareas (cargar lista o ejecutar una
    /// acción). Se pinta como banner (offline vs 401 vs otro) en vez de dejar la tabla muda.
    pub error_tareas: Option<crate::cliente::ErrorBroker>,

    // --- Pantalla Jornada (Fase 2) ---
    /// Jornada del peer enfocado (sesiones + tareas timbradas por el broker). `None` hasta que
    /// Peers fije un foco y se cargue vía `ClienteBroker::jornada`. La pantalla Jornada lo pinta;
    /// si es `None` muestra el texto guía "selecciona un peer".
    pub jornada: Option<peers_core::RespuestaJornada>,
    /// Id del peer cuya jornada está cacheada (para el título "Jornada · <id>"). `None` = sin foco.
    pub jornada_peer: Option<String>,

    // --- Pantalla Config (Fase 2) ---
    /// Panel de configuración con estado propio (Inputs editables + feedback de guardado). Es una
    /// `Entity<PanelConfig>` porque los `Input` del kit exigen estado que no cabe en una función
    /// pura; la app la construye al arrancar. `None` sólo si la construcción no llegó a hacerse:
    /// la vista pinta "Config no inicializada" en ese caso (nunca `.unwrap()`).
    pub panel_config: Option<Entity<PanelConfig>>,
}

/// Estado raíz de la app: cliente del broker, pantalla activa y datos cacheados.
pub struct AppDesktop {
    /// Cliente HTTP hacia el broker. Se guarda para que la Fase 2 dispare recargas con `cx.spawn`.
    #[allow(dead_code)]
    cliente: ClienteBroker,
    /// Pantalla actualmente visible en el área de contenido.
    activa: Pantalla,
    /// Datos que consumen las pantallas (vacío en Fundación).
    datos: EstadoPantalla,
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
        // La URL base y el token enmascarado son config fija del cliente: se copian una vez al
        // estado para que la pantalla Acceso (que sólo recibe `&EstadoPantalla`) pueda pintarlos
        // sin acceso al cliente. Nunca se guarda el token en claro, sólo su versión enmascarada.
        let datos = EstadoPantalla {
            acceso_url: cliente.base().to_string(),
            acceso_token: cliente.token_enmascarado(),
            panel_config,
            ..EstadoPantalla::default()
        };
        // Arranca la carga inicial + el refresco periódico. El `cx` aquí es `Context<Self>` (viene
        // de `cx.new(|cx| AppDesktop::nueva(...))`), así que `cx.spawn` captura la entidad ya creada.
        Self::arrancar_refresco(cx);
        Self {
            cliente,
            activa: Pantalla::Peers,
            datos,
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
            Pantalla::Config => {}
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
                        let n = esta.datos.alertas.len();
                        if n == 0 {
                            esta.datos.alertas_seleccion = 0;
                        } else if esta.datos.alertas_seleccion >= n {
                            esta.datos.alertas_seleccion = n - 1;
                        }
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
                esta.datos.error_acceso = err;
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
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.historial(&peer)) });
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

    /// Jornada: por ahora sólo asegura el roster (`POST /listar`) para poder elegir el peer en foco.
    /// La jornada detallada (`POST /jornada`) requiere un método de cliente que aún no existe; se
    /// deja la caché como está. Documentado como pendiente en el spec (R3.6).
    fn cargar_jornada(&mut self, cx: &mut Context<Self>) {
        let cliente = self.cliente.clone();
        let fondo = cx
            .background_executor()
            .spawn(async move { cliente.bloquear_en(cliente.listar_instancias()) });
        cx.spawn(async move |esta, cx| {
            let peers = fondo.await;
            let _ = esta.update(cx, |esta, cx| {
                if let Ok(lista) = peers {
                    esta.datos.instancias = lista;
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

    /// Abre el panel de detalle de la fila pulsada y la marca como seleccionada. Acota el índice a
    /// la lista actual por seguridad (la lista pudo encoger entre el pintado y el click).
    fn abrir_detalle_alerta(&mut self, indice: usize, cx: &mut Context<Self>) {
        if indice < self.datos.alertas.len() {
            self.datos.alertas_seleccion = indice;
            self.datos.alerta_detalle = Some(indice);
            cx.notify();
        }
    }

    /// Cierra el panel de detalle (deja la selección donde estaba).
    fn cerrar_detalle_alerta(&mut self, cx: &mut Context<Self>) {
        if self.datos.alerta_detalle.is_some() {
            self.datos.alerta_detalle = None;
            cx.notify();
        }
    }

    /// Descarta una alerta: `POST /admin/alerta-resolver` y, si va bien, recarga la lista. La
    /// escritura corre en un `cx.spawn` (no bloquea el hilo de UI); al volver se re-lee
    /// `admin_alertas` para que la tabla refleje el descarte sin depender del refresco periódico.
    /// Cualquier fallo se guarda en `error_alertas` para pintar el banner; nunca hace panic.
    fn descartar_alerta(&mut self, tipo: String, sujeto: String, cx: &mut Context<Self>) {
        // Cierra el panel de inmediato: la acción ya está en marcha, la UI no debe quedar abierta
        // sobre una alerta que va a desaparecer. La recarga posterior repinta la tabla.
        self.datos.alerta_detalle = None;
        cx.notify();

        let cliente = self.cliente.clone();
        cx.spawn(async move |esta, cx| {
            let resultado = cliente.alerta_resolver(&tipo, &sujeto).await;
            // Tras descartar, re-lee la lista para reflejar el estado real del broker.
            let recarga = match &resultado {
                Ok(_) => Some(cliente.admin_alertas().await),
                Err(_) => None,
            };
            // Vuelve al hilo de la entidad para mutar el estado con el resultado.
            let _ = esta.update(cx, |esta, cx| {
                match resultado {
                    Ok(_) => {
                        esta.datos.error_alertas = None;
                        if let Some(Ok(lista)) = recarga {
                            esta.datos.alertas = lista;
                            // Acota la selección a la nueva longitud para no apuntar fuera de rango.
                            let n = esta.datos.alertas.len();
                            if n == 0 {
                                esta.datos.alertas_seleccion = 0;
                            } else if esta.datos.alertas_seleccion >= n {
                                esta.datos.alertas_seleccion = n - 1;
                            }
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
        })
        .detach();
    }

    /// Construye un ítem del sidebar: un botón que resalta si es la pantalla activa y, al
    /// hacer click, navega. Se usa `on_click` con `cx.listener` para poder mutar `self`.
    fn item_nav(&self, pantalla: Pantalla, cx: &mut Context<Self>) -> impl IntoElement {
        let activa = self.activa == pantalla;
        div()
            .id(pantalla.id())
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            // Resalte simple para la activa; el tema del kit lo refinará en Fase 2.
            .when(activa, |d| d.bg(gpui::rgba(0x3b82f680)))
            .child(SharedString::from(pantalla.titulo()))
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
        }
    }
}

impl Render for AppDesktop {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Sidebar navegable: cabecera + un ítem por pantalla, en el orden canónico.
        let items = Pantalla::TODAS
            .iter()
            .map(|p| self.item_nav(*p, cx))
            .collect::<Vec<_>>();

        let sidebar = div()
            .v_flex()
            .h_full()
            .w(gpui::px(220.0))
            .gap_1()
            .p_2()
            .bg(gpui::rgba(0x00000020))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_lg()
                    .child(SharedString::from("claude-peers")),
            )
            .children(items);

        // Área de contenido: título de la pantalla activa (Fundación) vía su stub.
        let contenido = div().v_flex().size_full().child(self.contenido());

        // Layout raíz de dos columnas que ocupa toda la ventana. Aquí se registran los
        // manejadores de acciones de la pantalla Alertas: sus filas/botones DESPACHAN acciones
        // (`window.dispatch_action`) que burbujean hasta este contenedor, donde `cx.listener`
        // permite mutar el estado. Es el puente entre la vista pura y el estado de la app.
        use crate::vista::alertas::{AbrirDetalle, CerrarDetalleAlerta, Descartar};
        div()
            .h_flex()
            .size_full()
            .on_action(cx.listener(|esta, accion: &AbrirDetalle, _window, cx| {
                esta.abrir_detalle_alerta(accion.indice, cx);
            }))
            .on_action(cx.listener(|esta, _accion: &CerrarDetalleAlerta, _window, cx| {
                esta.cerrar_detalle_alerta(cx);
            }))
            .on_action(cx.listener(|esta, accion: &Descartar, _window, cx| {
                esta.descartar_alerta(accion.tipo.clone(), accion.sujeto.clone(), cx);
            }))
            .child(sidebar)
            .child(contenido)
    }
}
