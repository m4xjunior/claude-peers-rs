//! Pantalla Acceso — CRUD de conexión, seguridad y diagnóstico de auth con el broker.
//!
//! INTENCIÓN (qué cambia respecto a la Fase 1): la pantalla dejó de ser un espejo SOLO-LECTURA de
//! la TUI. Ahora es OPERABLE — el jefe cambia broker/token desde aquí sin rebotar a Config. Se
//! implementan las features ALTA de la RFC de Acceso:
//!   · acceso-01  editar `broker_url` in-situ (Input + "Guardar y reconectar").
//!   · acceso-02  editar/pegar el token in-situ (Input con máscara + "Aplicar").
//!   · acceso-05  "Probar conexión" — check dedicado (`/salud` + ruta protegida) SIN recargar todo.
//!   · acceso-06  diagnóstico diferenciado del error (offline / host desconocido / 401 / otro) con
//!                título + remedio + CTA al control relevante.
//!   · acceso-09  aviso de seguridad "broker expuesto sin token" (host no-loopback + config sin token).
//!
//! DECISIÓN DE ARQUITECTURA (por qué un componente stateful `PanelAcceso` y no sólo una vista pura):
//! los `Input` de gpui-component exigen `Entity<InputState>`, que sólo se crea con `Window`/`Context`
//! y debe vivir en una vista con estado (no en un render efímero). Es el MISMO patrón que la pantalla
//! Config (`PanelConfig`): `AppDesktop` crea la `Entity<PanelAcceso>` al arrancar y la guarda en
//! `EstadoPantalla`; el stub `render_acceso(&EstadoPantalla)` (firma de la Fundación intacta) delega
//! en ella. Así los Inputs editables (acceso-01/02) encajan sin romper el contrato de vista pura ni
//! el enrutado del sidebar.
//!
//! REPARTO DE RESPONSABILIDADES:
//!   · `PanelAcceso` (stateful) posee los Inputs y DESPACHA Actions al `AppDesktop` (que tiene el
//!     `ClienteBroker`): `AplicarConexion { broker_url, token }` para acceso-01/02 y `ProbarConexion`
//!     para acceso-05. El panel NO habla con el broker directamente (no tiene el cliente); refleja el
//!     resultado que `AppDesktop` le inyecta con `entity.update(...)` (métodos `set_resultado_prueba`
//!     / `set_diagnostico`). Ese es el puente stateful → app, análogo al vista-pura → app del resto.
//!   · El estado de conexión de SOLO-LECTURA (endpoint/versión/salud) lo sigue leyendo la vista pura
//!     de `EstadoPantalla` (info/salud), que se pinta ENCIMA de la card de edición.
//!
//! POR QUÉ ACTIONS Y NO LISTENER LOCAL (a diferencia de Config, que sí usa listener local): Config
//! sólo muta su propio estado (escribe el TOML), pero Acceso necesita RECONSTRUIR el `ClienteBroker`
//! —que vive en `AppDesktop`— y disparar una recarga. Eso obliga a rebotar por el árbol de Actions
//! hasta el `AppDesktop`, igual que las demás pantallas operables (alertas/tareas/redis).

use gpui::{
    div, prelude::FluentBuilder, px, rgba, App, AppContext, Context, Entity, IntoElement,
    ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    StyledExt,
};

use crate::app::EstadoPantalla;
use crate::tema;
use crate::validacion::{validar_url, ValidacionUrl};

// -------------------------------------------------------------------------------------------------
// ACCIONES — el panel es stateful pero necesita el `ClienteBroker` (que vive en `AppDesktop`) para
// reconfigurar la conexión y probarla, así que DESPACHA estas Actions y `AppDesktop` las maneja en su
// contenedor raíz con `.on_action(cx.listener(...))` (mismo patrón que alertas/tareas). Namespace
// `acceso` para no colisionar. `no_json`: sólo se despachan por código (`window.dispatch_action`),
// nunca desde un keymap → no arrastramos `serde::Deserialize`/`schemars` como en el resto.
// -------------------------------------------------------------------------------------------------

use gpui::Action;

// Acción sin datos: re-consultar el estado completo del broker (`/admin/info` + `/salud` +
// `/factor-estimacion`) para refrescar endpoint/versión/salud. Ya cableada en Fase 3 a
// `AppDesktop::cargar_broker`. Se conserva su nombre y semántica (botón "Comprobar acceso").
gpui::actions!(acceso, [RecargarAcceso]);

// Acción sin datos: "Probar conexión" (acceso-05). `AppDesktop` hace un check DEDICADO y ligero
// —`GET /salud` (sin token) para separar "broker vivo" de "token válido", y una ruta PROTEGIDA
// (`POST /listar`) para validar el token— y devuelve el resultado al panel SIN recargar el resto de
// la pantalla. El payload es vacío: el destino/credenciales los toma `AppDesktop` de su cliente ya
// configurado (el panel guardó antes con `AplicarConexion` si el jefe cambió algo).
gpui::actions!(acceso, [ProbarConexion]);

/// Aplicar una nueva configuración de conexión (acceso-01 + acceso-02). `AppDesktop`:
///   1) persiste `broker_url`/`token` en `config.toml` (`Config::guardar`, permisos 0600);
///   2) reconstruye/reconfigura el `ClienteBroker` en caliente (`ClienteBroker::reconfigurar`);
///   3) dispara `cargar_broker` contra el nuevo destino para validar y refrescar la card.
/// `token` vacío = broker sin token (modo anónimo): `AppDesktop`/`Config` lo normalizan a `None`.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = acceso, no_json)]
pub struct AplicarConexion {
    /// URL base nueva del broker (ya trim; `AppDesktop`/cliente normalizan la barra final).
    pub broker_url: String,
    /// Token nuevo en CLARO (o cadena vacía = sin token). Sólo viaja del panel al `AppDesktop`
    /// dentro del proceso; nunca se persiste en claro fuera del `config.toml` (0600).
    pub token: String,
}

// -------------------------------------------------------------------------------------------------
// COLORES DE SEVERIDAD (los decide esta pantalla, no el tema — son semántica de dominio "salud del
// broker"/"resultado de prueba", no decoración). Calibrados a la paleta cálida Ethos.
// -------------------------------------------------------------------------------------------------

/// Verde salvia apagado — salud "ok" / prueba correcta. Coherente con la paleta (no verde puro).
const OK: Rgba = Rgba { r: 0x4A as f32 / 255.0, g: 0x9A as f32 / 255.0, b: 0x63 as f32 / 255.0, a: 1.0 };
/// Rojo terroso — salud caída / prueba fallida / error de auth. No es rojo puro (paleta cálida).
const MAL: Rgba = Rgba { r: 0xC0 as f32 / 255.0, g: 0x4A as f32 / 255.0, b: 0x3E as f32 / 255.0, a: 1.0 };
/// Ámbar/brasa de aviso — postura de seguridad "expuesto sin token" (acceso-09). Reusa el dorado
/// de marca porque es una advertencia que el jefe debe VER, no un error del broker.
const AVISO: Rgba = tema::BRASA;

// -------------------------------------------------------------------------------------------------
// DIAGNÓSTICO DIFERENCIADO (acceso-06) — clasifica el `ErrorBroker` en una categoría con título +
// remedio + a qué control saltar. El `ErrorBroker` ya distingue Offline/NoAutorizado/Otro; dentro de
// Offline se sub-clasifica "host desconocido" (DNS) inspeccionando el mensaje de reqwest, porque un
// nombre mal escrito y un broker caído se arreglan distinto (editar URL vs arrancar el broker).
// -------------------------------------------------------------------------------------------------

/// Categoría de fallo de acceso para el banner tipado. Cada variante lleva su color, su icono, su
/// título y el remedio sugerido; `foco` indica qué control resaltar (URL o token) para la CTA.
#[derive(Clone, Copy, PartialEq)]
enum FocoRemedio {
    /// El remedio pasa por editar la URL (host desconocido, broker caído).
    Url,
    /// El remedio pasa por editar el token (401).
    Token,
    /// Sin control concreto (error genérico).
    Ninguno,
}

/// Traduce un `ErrorBroker` a (icono, título, remedio, color, foco) para el banner de diagnóstico.
/// Es SOLO presentación: no toca red. `Offline` se abre en dos: "host desconocido" (DNS, el mensaje
/// de reqwest contiene "dns"/"resolve"/"lookup") vs "sin conexión" (rechazo/timeout, broker caído).
fn diagnosticar(err: &crate::cliente::ErrorBroker) -> (&'static str, String, String, Rgba, FocoRemedio) {
    use crate::cliente::ErrorBroker;
    match err {
        ErrorBroker::Offline(detalle) => {
            let d = detalle.to_lowercase();
            if d.contains("dns") || d.contains("resolve") || d.contains("lookup") || d.contains("name or service") {
                (
                    "⚑",
                    "Host desconocido".to_string(),
                    "No se pudo resolver el nombre del broker. Revisa la URL (¿host bien escrito? ¿falta el esquema http://?).".to_string(),
                    MAL,
                    FocoRemedio::Url,
                )
            } else {
                (
                    "⭘",
                    "Sin conexión".to_string(),
                    "El broker no respondió (rechazo o timeout). Comprueba que el proceso está arrancado y la URL/puerto son correctos.".to_string(),
                    MAL,
                    FocoRemedio::Url,
                )
            }
        }
        ErrorBroker::NoAutorizado => (
            "✕",
            "Token rechazado (401)".to_string(),
            "El broker exige un token y el actual no coincide. Pega/edita el token correcto y vuelve a aplicar.".to_string(),
            MAL,
            FocoRemedio::Token,
        ),
        ErrorBroker::Otro(msg) => (
            "!",
            "Error del broker".to_string(),
            msg.clone(),
            MAL,
            FocoRemedio::Ninguno,
        ),
    }
}

// -------------------------------------------------------------------------------------------------
// RESULTADO DE "PROBAR CONEXIÓN" (acceso-05) — lo produce `AppDesktop` y lo inyecta en el panel. Es
// un check en dos pasos separados para distinguir "broker vivo" de "token válido".
// -------------------------------------------------------------------------------------------------

/// Resultado del último "Probar conexión". `AppDesktop` lo rellena vía `set_resultado_prueba`. Los
/// tres pasos son independientes: el broker puede estar vivo (salud ok) pero rechazar el token (401).
#[derive(Clone, Default)]
pub struct ResultadoPrueba {
    /// `GET /salud` respondió (broker alcanzable y vivo). `None` = aún no probado en esta sesión.
    pub salud_ok: Option<bool>,
    /// La ruta protegida (`POST /listar`) autenticó correctamente (token válido). `None` si no se
    /// llegó a probar (p.ej. el broker estaba caído) o aún no se ha probado.
    pub token_ok: Option<bool>,
    /// Texto de resumen del intento (p.ej. "broker vivo · token válido" o el motivo del fallo).
    pub resumen: String,
    /// `true` mientras el check está EN CURSO (spinner/estado "probando…").
    pub en_curso: bool,
    /// RTT del `GET /salud` en milisegundos (acceso-12). `None` si no se llegó a medir (broker
    /// caído antes de responder, o aún no probado). El broker NO expone latencia server-side
    /// (`RespuestaSalud` no trae timestamp) — se cronometra en el cliente, alrededor de la
    /// llamada bloqueante real (`ClienteBroker::bloquear_en`), no del scheduling de GPUI.
    pub latencia_ms: Option<u64>,
}

// -------------------------------------------------------------------------------------------------
// PANEL STATEFUL — Inputs de url/token + resultado de prueba + diagnóstico + feedback de guardado.
// -------------------------------------------------------------------------------------------------

/// Feedback del último "Aplicar" (guardar+reconectar). Texto de estado, no `Notification`, para
/// mantener la pantalla autocontenida (mismo criterio que `PanelConfig`).
enum EstadoAplicar {
    /// Aún no se ha aplicado nada en esta sesión.
    Inicial,
    /// Aplicado y persistido correctamente.
    Ok(String),
    /// Falló la validación de frontera (URL vacía/mal formada) antes de tocar nada.
    Error(String),
}

/// Panel de conexión con estado propio: los dos Inputs editables (url/token), el resultado de la
/// última prueba, el diagnóstico del último error y el feedback del último "Aplicar". Vive como
/// `Entity<PanelAcceso>` dentro de `EstadoPantalla`; lo crea `AppDesktop` al arrancar.
pub struct PanelAcceso {
    /// Input de la URL base del broker (acceso-01).
    entrada_url: Entity<InputState>,
    /// Input del token, con toggle de máscara —es un secreto— (acceso-02).
    entrada_token: Entity<InputState>,
    /// Resultado del último "Probar conexión" (acceso-05). Lo inyecta `AppDesktop`.
    prueba: ResultadoPrueba,
    /// Diagnóstico del último error de acceso (acceso-06). `None` si la última operación fue OK.
    /// Lo inyecta `AppDesktop` desde `cargar_broker`/`ProbarConexion`.
    diagnostico: Option<crate::cliente::ErrorBroker>,
    /// Feedback del último "Aplicar".
    ultimo_aplicar: EstadoAplicar,
    /// Validación EN VIVO de `entrada_url` (acceso-16): se recalcula en cada edición vía
    /// `cx.subscribe` (mismo patrón que `PanelConfig::validar_url`, config-04). Pinta el borde
    /// del campo y, si aplica, ofrece la sugerencia de autocorrección (variante C del RFC).
    validacion_url: ValidacionUrl,
    /// Feedback efímero de "Copiar token" (acceso-03): `Some(generación)` mientras se muestra
    /// "✓ Copiado"; se limpia solo tras 2s SI la generación no cambió (evita que un timer viejo
    /// borre el feedback de una copia más reciente si el jefe pulsa varias veces seguidas).
    feedback_copiado: Option<u64>,
    /// Contador que se incrementa en cada copia; es la "generación" que arbitra qué timer manda.
    generacion_copiado: u64,
    /// Historial local de conexiones (acceso-13), más reciente primero. `AppDesktop` lo inyecta
    /// tras cada "Aplicar"/"Probar" vía `set_historial` (leído de `config.toml`, nunca del broker).
    historial: Vec<crate::config::EventoConexion>,
}

impl PanelAcceso {
    /// Crea el panel sembrando los Inputs con la config actual del disco (URL + token en claro para
    /// poder editarlos). Ausente/corrupta → defaults (nunca crashea), igual que `PanelConfig`.
    pub fn nuevo(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg = crate::config::Config::cargar().unwrap_or_default();

        let entrada_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("http://127.0.0.1:7899")
                .default_value(cfg.broker_url.clone())
        });
        // El token se muestra en CLARO en el Input (es un campo editable) con toggle de máscara para
        // ocultarlo cuando el jefe quiera. Vacío si el broker corre sin token.
        let entrada_token = cx.new(|cx| {
            let s = InputState::new(window, cx).placeholder("(sin token)");
            match cfg.token.as_deref() {
                Some(t) if !t.is_empty() => s.default_value(t.to_string()),
                _ => s,
            }
        });

        // acceso-16: validación EN VIVO de la URL — cada edición del Input notifica al panel para
        // repintar el borde/nota sin esperar a "Aplicar" (mismo patrón que config-04).
        cx.subscribe(&entrada_url, |esta, input, _ev: &InputEvent, cx| {
            esta.validacion_url = validar_url(&input.read(cx).value());
            cx.notify();
        })
        .detach();

        let validacion_url = validar_url(&cfg.broker_url);

        Self {
            entrada_url,
            entrada_token,
            prueba: ResultadoPrueba::default(),
            diagnostico: None,
            ultimo_aplicar: EstadoAplicar::Inicial,
            validacion_url,
            feedback_copiado: None,
            generacion_copiado: 0,
            // acceso-13: el historial YA persistido se muestra desde el primer render (no hace
            // falta esperar a la primera acción de esta sesión para verlo).
            historial: cfg.acceso.historial,
        }
    }

    /// `AppDesktop` inyecta aquí el historial de conexiones tras registrar un evento (acceso-13).
    /// Espejo del contrato de `set_resultado_prueba`/`set_diagnostico`: el panel no habla con el
    /// disco directamente, refleja lo que la app le pasa.
    pub fn set_historial(&mut self, historial: Vec<crate::config::EventoConexion>, cx: &mut Context<Self>) {
        self.historial = historial;
        cx.notify();
    }

    /// `AppDesktop` inyecta aquí el resultado de "Probar conexión" (acceso-05). Se llama vía
    /// `entity.update(cx, |p, cx| p.set_resultado_prueba(...))` desde el manejador de `ProbarConexion`.
    pub fn set_resultado_prueba(&mut self, resultado: ResultadoPrueba, cx: &mut Context<Self>) {
        self.prueba = resultado;
        cx.notify();
    }

    /// Marca la prueba como EN CURSO (estado "probando…") mientras `AppDesktop` hace el check en
    /// segundo plano. Separado de `set_resultado_prueba` para dar feedback inmediato al pulsar.
    pub fn marcar_prueba_en_curso(&mut self, cx: &mut Context<Self>) {
        self.prueba = ResultadoPrueba {
            en_curso: true,
            resumen: "Probando…".to_string(),
            ..ResultadoPrueba::default()
        };
        cx.notify();
    }

    /// `AppDesktop` inyecta el diagnóstico del último error de acceso (acceso-06), o `None` si la
    /// última operación fue correcta (limpia el banner). Se llama tras `cargar_broker`.
    pub fn set_diagnostico(&mut self, err: Option<crate::cliente::ErrorBroker>, cx: &mut Context<Self>) {
        self.diagnostico = err;
        cx.notify();
    }

    /// Re-siembra los Inputs desde el disco (p.ej. tras aplicar, para reflejar la URL normalizada).
    /// Descarta ediciones no guardadas (intencional: "recargar" = volver a lo persistido).
    pub fn resembrar_desde_disco(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = crate::config::Config::cargar().unwrap_or_default();
        self.entrada_url
            .update(cx, |s, cx| s.set_value(cfg.broker_url.clone(), window, cx));
        self.entrada_token.update(cx, |s, cx| {
            s.set_value(cfg.token.clone().unwrap_or_default(), window, cx)
        });
        // El `set_value` de arriba dispara la suscripción de acceso-16 (recalcula
        // `validacion_url`), pero se fija explícito por si el valor no cambió (la suscripción de
        // `InputEvent` no siempre notifica en un no-op) — nunca dejar el borde con un estado viejo.
        self.validacion_url = validar_url(&cfg.broker_url);
        cx.notify();
    }

    /// Copia el token REAL (en claro, tal como está en `entrada_token`) al portapapeles (acceso-03).
    /// Mismo patrón que `PanelLanzador::copiar_comando`: `cx.write_to_clipboard` sin deps nuevas.
    /// El feedback "✓ Copiado" se auto-oculta a los 2s (variante A del RFC); se lee de
    /// `entrada_token` (no de `EstadoPantalla::acceso_token`, que sólo cachea el valor ENMASCARADO
    /// — el token en claro únicamente vive en este Input editable).
    fn copiar_token(&mut self, cx: &mut Context<Self>) {
        let token = self.entrada_token.read(cx).value().to_string();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(token));

        self.generacion_copiado += 1;
        let mi_generacion = self.generacion_copiado;
        self.feedback_copiado = Some(mi_generacion);
        cx.notify();

        cx.spawn(async move |esta, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = esta.update(cx, |esta, cx| {
                // Sólo se limpia si NADIE volvió a copiar mientras esperábamos (si el jefe pulsó
                // otra vez, esa copia tiene su propio timer y su propia generación más alta).
                if esta.feedback_copiado == Some(mi_generacion) {
                    esta.feedback_copiado = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Reemplaza el contenido del Input de URL por la autocorrección sugerida (acceso-16, variante
    /// C) y revalida. No aplica ni persiste por sí solo: el jefe sigue pulsando «Guardar y
    /// reconectar» para confirmar, igual que si hubiera tecleado la URL corregida a mano.
    fn usar_sugerencia(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(sugerencia) = self.validacion_url.sugerencia.clone() {
            self.entrada_url
                .update(cx, |s, cx| s.set_value(sugerencia.clone(), window, cx));
            self.validacion_url = validar_url(&sugerencia);
            cx.notify();
        }
    }

    /// Lee los Inputs, valida la frontera (acceso-16: esquema/host/barra final/puerto) y DESPACHA
    /// `AplicarConexion` al `AppDesktop` (acceso-01 + acceso-02). No toca el broker ni el disco
    /// directamente: eso lo hace `AppDesktop`, que tiene el cliente. Si la URL es inválida, muestra
    /// el error local (con la sugerencia, si aplica) sin despachar.
    fn aplicar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let broker_url = self.entrada_url.read(cx).value().trim().to_string();
        let token = self.entrada_token.read(cx).value().trim().to_string();

        // acceso-16: no se despacha con una URL mal formada. La validación EN VIVO ya pinta el
        // borde/nota mientras el jefe teclea; aquí se repite en frontera (defensa en
        // profundidad: el campo pudo mutar por `set_value` sin pasar por el listener de tecleo).
        let validacion = validar_url(&broker_url);
        if let Some(motivo) = validacion.motivo.clone() {
            self.validacion_url = validacion;
            self.ultimo_aplicar = EstadoAplicar::Error(format!("broker_url inválida: {motivo}"));
            cx.notify();
            return;
        }

        // Feedback optimista: `AppDesktop` confirmará con el diagnóstico tras reconectar.
        self.ultimo_aplicar = EstadoAplicar::Ok(format!("Aplicado: {broker_url}"));
        cx.notify();

        window.dispatch_action(Box::new(AplicarConexion { broker_url, token }), cx);
    }

    /// Envuelve un `Input` del kit en el marco de control Ethos (radio, superficie TINTA).
    /// Idéntico criterio que `PanelConfig::input_tematizado`: el Input hereda fuente/color del
    /// subárbol; el `div` tematizado le da el marco sin pelear con el tema del kit. `es_token`
    /// añade el toggle de máscara (ojo) para el secreto. `borde` (acceso-16) permite pisar el
    /// color por defecto (LINEA) con el de validación EN VIVO — `None` = LINEA neutro (token, que
    /// no se valida aquí).
    fn input_tematizado(estado: &Entity<InputState>, es_token: bool, borde: Option<Rgba>) -> impl IntoElement {
        let input = if es_token {
            Input::new(estado).cleanable(true).mask_toggle()
        } else {
            Input::new(estado).cleanable(true)
        };
        div()
            .w_full()
            .px_3()
            .py_1()
            .rounded(tema::radio(tema::RADIO_CONTROL))
            .bg(tema::TINTA)
            .border_1()
            .border_color(borde.unwrap_or(tema::LINEA))
            .child(input)
    }
}

impl Render for PanelAcceso {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Card de EDICIÓN: los dos Inputs (url/token) + botones "Aplicar" y "Probar conexión" +
        // feedback + resultado de prueba + diagnóstico. Es la parte OPERABLE de la pantalla.
        let mut card = tema::superficie_card().v_flex().w_full().gap_4().p_5();

        // Campo URL (acceso-01) + validación EN VIVO (acceso-16): borde verde salvia/terracota
        // según `validacion_url`, nota con el motivo si es inválida, y botón de autocorrección si
        // hay una sugerencia mecánica (variante C del RFC — p.ej. faltaba el esquema http://).
        let borde_url = if self.validacion_url.es_valida() { Some(OK) } else { Some(MAL) };
        let mut campo_url = campo_form(
            "broker_url",
            "URL base del broker (sin barra final). Enter no aplica: usa «Aplicar».",
            Self::input_tematizado(&self.entrada_url, false, borde_url),
        );
        if let Some(motivo) = &self.validacion_url.motivo {
            campo_url = campo_url.child(
                div().text_sm().text_color(MAL).child(SharedString::from(format!("⚠ {motivo}"))),
            );
        }
        if let Some(sugerencia) = self.validacion_url.sugerencia.clone() {
            campo_url = campo_url.child(
                div().h_flex().gap_2().items_center()
                    .child(tema::texto_terciario(format!("¿Quisiste decir «{sugerencia}»?")))
                    .child(
                        tema::boton_secundario("acceso-usar-sugerencia", "Usar sugerencia")
                            .on_click(cx.listener(|esta, _e, window, cx| esta.usar_sugerencia(window, cx))),
                    ),
            );
        }
        card = card.child(campo_url);

        // Campo token (acceso-02) + botón "Copiar" (acceso-03): fila horizontal Input+botón para
        // que el Input siga ocupando el ancho disponible. El botón alterna su label a "✓ Copiado"
        // 2s tras copiar (variante A del RFC — confirmación efímera, sin exponer el valor extra).
        let label_copiar: SharedString = if self.feedback_copiado.is_some() {
            "✓ Copiado".into()
        } else {
            "Copiar".into()
        };
        card = card.child(campo_form(
            "token",
            "X-Peers-Token. Vacío = broker sin token. Usa el ojo para revelar/ocultar.",
            div()
                .h_flex()
                .gap_2()
                .w_full()
                .child(div().flex_1().child(Self::input_tematizado(&self.entrada_token, true, None)))
                .child(
                    tema::boton_secundario("acceso-copiar-token", label_copiar)
                        .on_click(cx.listener(|esta, _e, _window, cx| esta.copiar_token(cx))),
                ),
        ));

        // Botonera: aplicar (primario, reconfigura+reconecta) + probar (secundario, check ligero).
        card = card.child(
            div()
                .h_flex()
                .gap_3()
                .pt_1()
                .child(
                    tema::boton_primario("acceso-aplicar", "Guardar y reconectar")
                        .on_click(cx.listener(|esta, _e, window, cx| esta.aplicar(window, cx))),
                )
                .child(
                    tema::boton_secundario("acceso-probar", "Probar conexión")
                        .on_click(cx.listener(|esta, _e, window, cx| {
                            // Feedback inmediato + despacho: `AppDesktop` hace el check y lo inyecta.
                            esta.marcar_prueba_en_curso(cx);
                            window.dispatch_action(Box::new(ProbarConexion), cx);
                        })),
                ),
        );

        // Feedback del último "Aplicar" (validación de frontera / confirmación optimista).
        match &self.ultimo_aplicar {
            EstadoAplicar::Inicial => {}
            EstadoAplicar::Ok(msg) => {
                card = card.child(div().text_sm().text_color(OK).child(SharedString::from(format!("✓ {msg}"))));
            }
            EstadoAplicar::Error(msg) => {
                card = card.child(div().text_sm().text_color(MAL).child(SharedString::from(format!("⚠ {msg}"))));
            }
        }

        // Resultado de "Probar conexión" (acceso-05): checklist de 3 pasos, sin recargar el resto.
        if self.prueba.en_curso || self.prueba.salud_ok.is_some() || !self.prueba.resumen.is_empty() {
            card = card.child(bloque_prueba(&self.prueba));
        }

        // Diagnóstico diferenciado del último error (acceso-06): banner tipado con remedio + CTA.
        if let Some(err) = &self.diagnostico {
            card = card.child(banner_diagnostico(err));
        }

        // Historial local de conexiones (acceso-13): mini-feed con las últimas entradas (variante C
        // del RFC — más simple que un Popover/Dialog dedicado; el historial completo cabe en pocas
        // líneas porque está acotado a `MAX_HISTORIAL_ACCESO` en disco).
        if !self.historial.is_empty() {
            card = card.child(bloque_historial(&self.historial));
        }

        card
    }
}

/// Constructor de la `Entity<PanelAcceso>` para `AppDesktop` (no filtra el tipo interno a `app.rs`).
pub fn nuevo_panel(window: &mut Window, cx: &mut App) -> Entity<PanelAcceso> {
    cx.new(|cx| PanelAcceso::nuevo(window, cx))
}

// -------------------------------------------------------------------------------------------------
// VISTA PURA (firma de la Fundación intacta) — cabecera + estado de SOLO-LECTURA (endpoint/versión/
// salud + aviso de exposición) + la card de EDICIÓN delegada en la `Entity<PanelAcceso>`.
// -------------------------------------------------------------------------------------------------

/// Render de la pantalla Acceso. Compone: cabecera, aviso de seguridad (acceso-09), panel de estado
/// (endpoint/versión/salud en mono), y —debajo— el panel stateful de edición/prueba/diagnóstico. Si
/// el panel no está inicializado, pinta sólo lo de solo-lectura (nunca `.unwrap()` sobre el Option).
pub fn render_acceso(datos: &EstadoPantalla) -> impl IntoElement {
    // Endpoint anunciado por el broker (host:puerto) si llegó `/admin/info`; si no, la URL cacheada.
    let endpoint = match &datos.info {
        Some(info) => format!("{}:{}", info.host, info.puerto),
        None => datos.acceso_url.clone(),
    };
    let version = datos
        .info
        .as_ref()
        .map(|i| i.version.clone())
        .unwrap_or_else(|| "(desconocida)".to_string());

    // Estado de salud → chip coloreado (verde ok / rojo caído).
    let (salud_texto, salud_ok) = match &datos.salud {
        Some(s) => (format!("{} · {} instancias", s.estado, s.instancias), s.estado == "ok"),
        None => ("sin datos todavía".to_string(), false),
    };
    let salud_color = if salud_ok { OK } else { MAL };

    // acceso-09 — ¿broker EXPUESTO sin token? Se deduce del host de `/admin/info` (no loopback) + la
    // ausencia de token en el cliente (`acceso_sin_token`, copia tipada de `ClienteBroker::sin_token`,
    // no una comparación del string enmascarado). Sin `info` no podemos afirmar el host, así que sólo
    // avisamos cuando SABEMOS que el host es no-loopback.
    let expuesto = datos
        .info
        .as_ref()
        .map(|i| !es_loopback(&i.host))
        .unwrap_or(false)
        && datos.acceso_sin_token;

    // `raiz_scrollable()` (NO `fondo_app()`, sin `size_full`): el contenedor `contenido-scroll` de
    // app.rs necesita que esta vista NO fije su propio alto para poder medirla y scrollearla.
    let mut raiz = tema::raiz_scrollable().v_flex().gap_5().p_6();

    // Cabecera.
    raiz = raiz.child(
        div()
            .v_flex()
            .gap_1()
            .child(tema::eyebrow("Conexión"))
            .child(tema::titulo("Red / Acceso")),
    );

    // acceso-09 — aviso de seguridad persistente (franja ámbar, escudo, CTA implícita "pon un token
    // abajo"). Se pinta ARRIBA del todo para que no pase desapercibido: es un agujero de seguridad.
    if expuesto {
        raiz = raiz.child(banner_expuesto());
    }

    // Panel de estado de SOLO-LECTURA (endpoint/versión/salud). La URL/token editables viven en el
    // panel stateful de abajo; aquí sólo se muestra lo que el broker anuncia.
    raiz = raiz.child(
        tema::superficie_card()
            .v_flex()
            .gap_3()
            .p_5()
            .child(fila_kv("endpoint", &endpoint))
            .child(fila_kv("versión", &version))
            .child(div().w_full().h(px(1.0)).bg(tema::LINEA))
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .items_center()
                    .child(etiqueta("salud"))
                    .child(tema::chip_estado(salud_texto, salud_color)),
            ),
    );

    // Botón "Comprobar acceso" (recarga completa: /admin/info + /salud + /factor). Se conserva de la
    // Fase 1 — es la recarga PESADA; "Probar conexión" (en el panel) es el check LIGERO de acceso-05.
    raiz = raiz.child(
        div().h_flex().gap_2().child(
            tema::boton_secundario("acceso-recargar", "Comprobar acceso").on_click(|_e, window, cx| {
                window.dispatch_action(Box::new(RecargarAcceso), cx);
            }),
        ),
    );

    // Card de EDICIÓN/prueba/diagnóstico: delegada en la `Entity<PanelAcceso>` (acceso-01/02/05/06).
    match &datos.panel_acceso {
        Some(panel) => {
            raiz = raiz.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(tema::eyebrow("Gestionar conexión"))
                    .child(panel.clone()),
            );
        }
        None => {
            // Sin panel inicializado: al menos mostramos el error crudo (Fase 1) para no quedar mudos.
            if let Some(err) = datos.error_acceso.as_ref() {
                raiz = raiz.child(banner_diagnostico(err));
            }
        }
    }

    raiz
}

// -------------------------------------------------------------------------------------------------
// HELPERS DE PRESENTACIÓN
// -------------------------------------------------------------------------------------------------

/// Campo de formulario del panel: eyebrow (label) + Input + ayuda terciaria. Espejo de
/// `PanelConfig::campo`, replicado aquí para no acoplar las dos pantallas por un helper trivial.
/// Devuelve `Div` (no `impl IntoElement`) para que el caller pueda seguir encadenando `.child(...)`
/// — lo necesita acceso-16 para añadir la nota de error/sugerencia SÓLO al campo de URL.
fn campo_form(etiqueta: &'static str, ayuda: &'static str, input: impl IntoElement) -> gpui::Div {
    div()
        .v_flex()
        .gap_2()
        .child(tema::eyebrow(etiqueta))
        .child(input)
        .child(tema::texto_terciario(ayuda))
}

/// Bloque de resultado de "Probar conexión" (acceso-05): 3 filas con dot verde/rojo/neutro. Distingue
/// "broker vivo" (salud) de "token válido" (ruta protegida) para que el jefe sepa QUÉ falla.
fn bloque_prueba(p: &ResultadoPrueba) -> impl IntoElement {
    let paso = |etiqueta: &str, estado: Option<bool>, pendiente: &str| {
        let (dot, texto, color): (&str, String, Rgba) = match estado {
            Some(true) => ("●", format!("{etiqueta} ✓"), OK),
            Some(false) => ("●", format!("{etiqueta} ✕"), MAL),
            None => ("○", format!("{etiqueta} — {pendiente}"), tema::HUMO),
        };
        div()
            .h_flex()
            .gap_2()
            .items_center()
            .child(div().text_color(color).child(SharedString::from(dot)))
            .child(
                div()
                    .font_family(tema::FUENTE_MONO)
                    .text_size(px(12.0))
                    .text_color(tema::PAPEL)
                    .child(SharedString::from(texto)),
            )
    };

    let mut bloque = div()
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .bg(tema::TINTA)
        .border_1()
        .border_color(tema::LINEA)
        .child(tema::eyebrow("prueba de conexión"))
        .child(paso("broker alcanzable · /salud", p.salud_ok, if p.en_curso { "probando…" } else { "sin probar" }))
        .child(paso("token válido · ruta protegida", p.token_ok, if p.en_curso { "probando…" } else { "sin probar" }));

    // acceso-12: RTT de /salud, chip de color por umbral (verde <50ms / ámbar <200ms / rojo ≥200ms).
    // Sólo se pinta si hubo medición (broker vivo cuando se probó); offline no tiene RTT que mostrar.
    if let Some(ms) = p.latencia_ms {
        let color = if ms < 50 {
            OK
        } else if ms < 200 {
            AVISO
        } else {
            MAL
        };
        bloque = bloque.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(tema::texto_terciario("latencia"))
                .child(tema::chip_estado(format!("{ms} ms"), color)),
        );
    }

    bloque.when(!p.resumen.is_empty(), |d| {
        d.child(tema::texto_terciario(p.resumen.clone()))
    })
}

/// Mini-feed del historial local de conexiones (acceso-13, variante C del RFC): las últimas
/// entradas con dot verde/rojo + hora + tipo + detalle, mono para hora/detalle (son datos
/// técnicos). Muestra como mucho las 5 más recientes en la card — el historial COMPLETO vive
/// acotado en disco (`MAX_HISTORIAL_ACCESO`), pero la card no necesita listarlo todo para ser útil.
fn bloque_historial(historial: &[crate::config::EventoConexion]) -> impl IntoElement {
    const MAX_VISIBLES: usize = 5;

    let mut bloque = div()
        .v_flex()
        .gap_2()
        .p_3()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .bg(tema::TINTA)
        .border_1()
        .border_color(tema::LINEA)
        .child(tema::eyebrow("historial de conexión"));

    for ev in historial.iter().take(MAX_VISIBLES) {
        let color = if ev.ok { OK } else { MAL };
        let tipo_txt = match ev.tipo {
            crate::config::TipoEventoConexion::Aplicar => "Aplicar",
            crate::config::TipoEventoConexion::Probar => "Probar",
        };
        bloque = bloque.child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(div().text_color(color).child(SharedString::from("●")))
                .child(
                    div()
                        .font_family(tema::FUENTE_MONO)
                        .text_size(px(11.0))
                        .text_color(tema::HUMO)
                        .child(SharedString::from(hora_de_iso(&ev.cuando))),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(tema::PAPEL)
                        .child(SharedString::from(tipo_txt)),
                )
                .child(
                    div()
                        .flex_1()
                        .font_family(tema::FUENTE_MONO)
                        .text_size(px(11.0))
                        .text_color(tema::HUMO)
                        .child(SharedString::from(ev.detalle.clone())),
                ),
        );
    }

    if historial.len() > MAX_VISIBLES {
        bloque = bloque.child(tema::texto_terciario(format!(
            "… y {} más (persistidas en config.toml)",
            historial.len() - MAX_VISIBLES
        )));
    }

    bloque
}

/// Extrae `HH:MM:SS` de una marca RFC 3339 (p.ej. `2026-07-03T09:05:00Z` → `09:05:00`). Réplica
/// deliberada de `vista::jornada::hora_iso` (privada allí): duplicar una función trivial de 6
/// líneas es más barato que acoplar dos módulos de pantalla por un helper de formato.
fn hora_de_iso(iso: &str) -> String {
    if let Some(pos) = iso.find('T') {
        let resto = &iso[pos + 1..];
        let fin = resto.find(['.', 'Z', '+']).unwrap_or(resto.len());
        return resto[..fin].to_string();
    }
    iso.to_string()
}

/// Banner de diagnóstico diferenciado (acceso-06): icono + título por categoría + remedio + pista de
/// a qué control ir. Rojo terroso tenue; el título en el color de severidad. Reemplaza el banner
/// plano de la Fase 1 por uno que EXPLICA el fallo (offline/DNS/401/otro) y sugiere la acción.
fn banner_diagnostico(err: &crate::cliente::ErrorBroker) -> impl IntoElement {
    let (icono, titulo, remedio, color, foco) = diagnosticar(err);
    let cta = match foco {
        FocoRemedio::Url => "→ edita «broker_url» abajo y pulsa «Guardar y reconectar».",
        FocoRemedio::Token => "→ pega el token en «token» abajo y pulsa «Guardar y reconectar».",
        FocoRemedio::Ninguno => "",
    };

    div()
        .w_full()
        .v_flex()
        .gap_1()
        .px_4()
        .py_3()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .border_1()
        .border_color(color)
        .bg(rgba(0xC04A3E22)) // rojo terroso muy tenue sobre la tinta
        .child(
            div()
                .h_flex()
                .gap_2()
                .items_center()
                .child(div().text_color(color).child(SharedString::from(icono)))
                .child(
                    div()
                        .text_color(tema::PAPEL)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(SharedString::from(titulo)),
                ),
        )
        .child(tema::texto_terciario(remedio))
        .when(!cta.is_empty(), |d| d.child(tema::texto_terciario(cta)))
}

/// Banner de seguridad "broker expuesto sin token" (acceso-09): franja ámbar con escudo. Persistente
/// mientras la condición sea cierta. El color de marca (brasa) señala advertencia sin ser un error.
fn banner_expuesto() -> impl IntoElement {
    div()
        .w_full()
        .h_flex()
        .gap_3()
        .items_center()
        .px_4()
        .py_3()
        .rounded(tema::radio(tema::RADIO_CONTROL))
        .border_1()
        .border_color(AVISO)
        .bg(tema::acento_tenue(0.13)) // acento de marca muy tenue (helper: se reajusta si cambia BRASA)
        .child(div().text_color(AVISO).text_size(px(18.0)).child(SharedString::from("🛡")))
        .child(
            div()
                .v_flex()
                .gap_1()
                .child(
                    div()
                        .text_color(tema::PAPEL)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(SharedString::from("Broker expuesto en la red SIN token")),
                )
                .child(tema::texto_terciario(
                    "Escucha en un host no-loopback y no hay token: cualquiera en la red puede conectarse. Pon un token abajo y aplica.",
                )),
        )
}

/// `true` si el host es loopback (127.0.0.0/8, ::1 o "localhost"). Usado por acceso-09 para no avisar
/// cuando el broker sólo escucha localmente (sin token en loopback no es un agujero de red).
fn es_loopback(host: &str) -> bool {
    let h = host.trim();
    h == "localhost"
        || h == "::1"
        || h == "0:0:0:0:0:0:0:1"
        || h.starts_with("127.")
}

/// Fila etiqueta→valor del panel de estado (mono para datos técnicos). Espejo del helper de Fase 1.
fn fila_kv(etiqueta_txt: &str, valor: &str) -> impl IntoElement {
    div()
        .h_flex()
        .gap_3()
        .items_center()
        .child(etiqueta(etiqueta_txt))
        .child(
            div()
                .font_family(tema::FUENTE_MONO)
                .text_color(tema::PAPEL)
                .text_size(px(13.0))
                .child(SharedString::from(valor.to_string())),
        )
}

/// Etiqueta de campo de ancho fijo (eyebrow) para alinear la columna de valores.
fn etiqueta(texto: &str) -> impl IntoElement {
    tema::eyebrow(texto).w(px(120.0))
}
