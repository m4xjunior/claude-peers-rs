# RFC — Pestaña Acceso (desktop GPUI): controles de conexión, seguridad y trazabilidad

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

| Campo | Valor |
|-------|-------|
| **Título** | CRUD, controles de conexión/seguridad y trazabilidad de auth para la pestaña Acceso de `peers-desktop` |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Impacto** | **ALTO** — es la puerta de entrada: sin poder cambiar broker/token/allowlist desde aquí, la app es una ventana de solo lectura sobre una conexión ya montada por otro |
| **Fecha** | 2026-07-02 |
| **Estado** | PROPUESTO (pendiente de decisión — NO implementar hasta aprobar features y variantes) |
| **Pantalla** | `crates/peers-desktop/src/vista/acceso.rs` |
| **Referencia TUI** | `crates/peers-tui/src/ui/acceso.rs` (solo lectura) + `crates/peers-tui/src/ui/config.rs` + teclas en `crates/peers-tui/src/main.rs` |

---

## 1. Contexto

### Qué hace HOY la pestaña Acceso (desktop)

`render_acceso(&EstadoPantalla)` es una vista **pura** (sin `cx`). Muestra, en una `superficie_card` Ethos con filas key→value en fuente mono:

- `broker_url` (cacheado en `datos.acceso_url`)
- `endpoint` (host:puerto de `GET /admin/info`, o la URL como respaldo)
- `versión` (del broker, vía `/admin/info`)
- `token` (enmascarado, `datos.acceso_token`)
- chip de **salud** (`GET /salud`: estado + nº instancias) coloreado verde/rojo
- un único botón interactivo: **"Comprobar acceso"** → despacha `RecargarAcceso`, que reejecuta `cargar_broker` (`/admin/info` + `/salud`)
- banner de error si la última comprobación falló (offline / 401 / otro)
- nota "para cambiarlo, ve a la pantalla Config"

Es decir: **es casi idéntica a la TUI (solo lectura)**. Toda la interacción real (editar URL, editar token, guardar) vive en la **otra** pestaña (Config, `vista/config.rs`, `PanelConfig`), obligando a Max a saltar de pantalla para cualquier cambio de conexión. Acceso no puede: cambiar de broker, editar/rotar/copiar el token, gestionar múltiples brokers, ver por qué falla un 401, ni auditar cuándo se conectó/desconectó.

### Qué le FALTA (resumen)

1. **CRUD de conexión in-situ** — cambiar `broker_url` y `token` desde Acceso, no rebotar a Config.
2. **Gestión del secreto** — copiar, revelar, rotar, pegar, validar el token; hoy solo se ve enmascarado.
3. **Multi-broker** — guardar y alternar entre varios brokers (local, OTUS, túnel) sin reescribir la URL a mano.
4. **Diagnóstico de auth** — al fallar, saber si es DNS/offline/TLS/401; hoy solo un texto plano.
5. **Seguridad de red** — allowlist de IPs y visibilidad de si el broker está expuesto sin token (el broker ya loguea ese warning, pero la app no lo muestra).
6. **Trazabilidad** — historial de conexiones, cambios de config, intentos de auth fallidos, latencia.

### Endpoints reales verificados en `crates/peers-broker/src/main.rs`

| Endpoint | Método | Qué da | Auth |
|----------|--------|--------|------|
| `/salud` | GET | `{estado, instancias}` | **exento** (única ruta sin token) |
| `/admin/info` | GET | `{host, puerto, instancias, version}` | token |
| `/admin/redis` | GET | colas + outbox por peer | token |
| `/listar` | POST | instancias registradas | token |
| resto | — | tareas, mensajes, alertas, jornada… | token (`verificar_token` con header `X-Peers-Token`) |

**Modelo de auth actual (verificado):** `token_autorizado(configurado, recibido)` — sin token en el broker → todo autorizado (compat local); con token → header `X-Peers-Token` debe coincidir exacto; si no, `401 "token inválido o ausente"`. El token del broker se fija por `--token`/`env CLAUDE_PEERS_TOKEN` **al arrancar el proceso** (no hay endpoint para rotarlo en caliente). El broker emite un `warn!` si escucha en un host no-loopback **sin** token.

**Consecuencia de diseño (importante para las features):** el broker **NO** expone hoy endpoints de: rotar token, gestionar allowlist de IPs, probar conexión dedicado, ni auditar auth. Las features marcadas *(requiere endpoint nuevo)* quiero que se añada; las demás se construyen con lo que YA existe (`/salud`, `/admin/info`) + estado local en la app (`config.toml`, ampliado).

### Design System Ethos (tokens usados en las variantes)

Fondo `TINTA #100D0A` · superficie `TINTA2 #1A1611` · texto `PAPEL #ECE5D7` · acento `BRASA #C9A96E` · terciario `HUMO #938B7B` · borde `LINEA #2B271F`. Radios: card 14 / control 10 / pill 999. Fuentes Fraunces (títulos) / Inter (UI) / IBM Plex Mono (datos: url/token/host/latencia). Helpers en `tema.rs`: `superficie_card`, `eyebrow`, `titulo`, `chip_estado`, `boton_primario`, `texto_terciario`, `fondo_app`. Componentes gpui-component: Dialog/Modal, Button, Input, Select, Switch, Tooltip, Badge, Table, Popover, Notification.

---

## 2. Features propuestas (≥15)

> Convención: cada feature indica **problema** (qué NO puede hacer Max hoy), **propuesta**, **2-3 variantes de diseño Ethos**, **endpoint/método**, **trazabilidad** (si aplica) y **prioridad**. Las variantes son alternativas a decidir, NO todas a implementar.

---

### acceso-01 — Editar `broker_url` desde Acceso
- **Problema:** para cambiar de broker Max tiene que abandonar Acceso e irse a Config; la fila `broker_url` es texto muerto.
- **Propuesta:** hacer la fila `broker_url` editable in-situ; al confirmar, reconstruye el `ClienteBroker` con la nueva base y reejecuta `cargar_broker`.
- **Variantes DS:**
  - **A (inline edit):** clic en el valor mono → se transforma en un `Input` Ethos (borde `LINEA`, foco `BRASA`), Enter confirma, Esc cancela. Icono lápiz `HUMO` a la derecha que aparece en hover.
  - **B (botón + modal):** botón fantasma "Editar" junto a la fila → `Dialog` modal "Cambiar broker" con un `Input` grande, botón primario dorado "Conectar" y secundario "Cancelar".
  - **C (barra de edición):** al enfocar la card aparece una barra inferior fija con el `Input` de URL y "Guardar y reconectar", estilo command-bar.
- **Endpoint/método:** ninguno nuevo — persiste en `config.toml` (`Config::guardar`) + reconstruye cliente + `GET /admin/info` para validar.
- **Trazabilidad:** registra el cambio en el historial de conexión (ver acceso-13).
- **Prioridad:** **alta**

---

### acceso-02 — Editar / pegar el token desde Acceso
- **Problema:** el token solo se ve enmascarado; para cambiarlo hay que ir a Config. No se puede pegar un token nuevo aquí.
- **Propuesta:** control de edición del token en la propia card, con validación inmediata contra el broker.
- **Variantes DS:**
  - **A (campo con máscara):** `Input` con `mask_toggle()` (ojo `HUMO`), placeholder "(sin token)", borde `LINEA`; botón "Aplicar y probar" dorado que valida antes de persistir.
  - **B (modal seguro):** `Dialog` "Cambiar token" con el `Input` enmascarado + checkbox "Recordar en config.toml (0600)" + aviso terciario "el token se guarda cifrado por permisos de archivo".
  - **C (pegar directo):** botón "Pegar desde portapapeles" (icono) que rellena y auto-valida; útil para el flujo real de Max (copia el token del broker y lo pega).
- **Endpoint/método:** validación con cualquier ruta protegida (p.ej. `POST /listar`) para detectar 401; persiste en `config.toml`.
- **Trazabilidad:** evento "token cambiado" (sin loguear el valor) en historial de auth.
- **Prioridad:** **alta**

---

### acceso-03 — Copiar el token real al portapapeles
- **Problema:** Max no puede extraer el token desde la app para pegarlo en otra máquina/sesión; solo ve `lexus…2026`.
- **Propuesta:** acción "Copiar token" que copia el valor **en claro** al portapapeles, con confirmación efímera.
- **Variantes DS:**
  - **A (icono en la fila):** icono copiar `HUMO`→`BRASA` en hover al final de la fila `token`; al pulsar, `Tooltip`/toast "Copiado" 2s.
  - **B (botón bajo la card):** botón secundario "Copiar token" con `Notification` del kit ("Token copiado — se borra del portapapeles en 30s").
  - **C (long-press / confirm):** requiere confirmación ("¿Copiar el secreto en claro?") en un `Popover` antes de copiar, por higiene.
- **Endpoint/método:** ninguno — lee el token de la config en memoria + API de portapapeles de GPUI.
- **Trazabilidad:** evento "token copiado" (marca temporal, sin valor).
- **Prioridad:** media

---

### acceso-04 — Revelar / ocultar el token enmascarado
- **Problema:** solo se ve enmascarado; a veces Max necesita verificar visualmente el valor completo.
- **Propuesta:** toggle de revelado temporal del token en la fila.
- **Variantes DS:**
  - **A (ojo):** icono ojo `HUMO` que alterna entre `lexus…2026` y el valor completo en mono `PAPEL`; se reoculta solo tras 10s.
  - **B (hold-to-reveal):** mantener pulsado revela; soltar reoculta (sin dejar el secreto en pantalla).
  - **C (chip "revelado"):** al revelar, aparece un `Badge` ámbar "visible" para recordar que hay un secreto expuesto.
- **Endpoint/método:** ninguno — puramente cliente.
- **Prioridad:** baja

---

### acceso-05 — Probar conexión (test dedicado, sin recargar todo)
- **Problema:** "Comprobar acceso" reejecuta todo `cargar_broker`; no hay un test rápido y explícito "¿me conecto y autentico?" con resultado claro.
- **Propuesta:** botón "Probar conexión" que hace un check secuencial (resuelve host → `GET /salud` → ruta protegida para validar token) y reporta cada paso.
- **Variantes DS:**
  - **A (checklist en Popover):** `Popover` bajo el botón con 3 filas: "① alcanzable ✓ / ② salud ok ✓ / ③ token válido ✗" con dots `BRASA`/rojo terroso.
  - **B (línea de estado animada):** debajo del botón, texto que va cambiando ("Resolviendo… · Pidiendo /salud… · Validando token…") y termina en chip verde/rojo.
  - **C (modal diagnóstico):** `Dialog` con los 3 pasos + latencia de cada uno + botón "Reintentar".
- **Endpoint/método:** `GET /salud` (sin token) + una ruta protegida (`POST /listar`) para separar "broker vivo" de "token válido".
- **Trazabilidad:** cada test se registra en el historial de conexión con su resultado.
- **Prioridad:** **alta**

---

### acceso-06 — Diagnóstico diferenciado del error de auth
- **Problema:** al fallar solo hay un banner de texto; Max no distingue offline vs DNS vs TLS vs 401 vs 500.
- **Propuesta:** clasificar `ErrorBroker` en categorías y mostrar causa + remedio sugerido.
- **Variantes DS:**
  - **A (banner tipado):** el `banner_error` actual gana un icono y un título por categoría — "Sin conexión" (offline), "Token rechazado (401)" (con CTA "Editar token"), "Host desconocido" (con CTA "Editar URL").
  - **B (tarjeta de diagnóstico):** `superficie_card` con severidad de color, "qué pasó / qué revisar / acción" y botón que salta al control relevante (acceso-01/02).
  - **C (inline en la fila):** el error se ancla a la fila culpable (rojo terroso bajo `broker_url` si es DNS, bajo `token` si es 401).
- **Endpoint/método:** ninguno nuevo — se apoya en el `Display`/variantes de `ErrorBroker` (ya distingue 401/offline/otro).
- **Trazabilidad:** intentos fallidos con su categoría → historial de auth (acceso-14).
- **Prioridad:** **alta**

---

### acceso-07 — Multi-broker: guardar y alternar perfiles de conexión
- **Problema:** Max trabaja contra varios brokers (local, OTUS `10.0.0.67`, túnel `p2v.lexusfx.com`) y debe reescribir la URL+token a mano cada vez.
- **Propuesta:** lista de "perfiles de broker" persistidos, cada uno con nombre + url + token; selector para activar uno.
- **Variantes DS:**
  - **A (Select en cabecera):** `Select` Ethos "Broker: [OTUS ▾]" arriba de la card; al elegir, reconecta. Botón "+ Nuevo perfil".
  - **B (lista de tarjetas):** columna de `superficie_card` pequeñas (una por broker) con chip de salud propio; la activa lleva borde `BRASA`. Clic = activar; menú ⋯ = editar/borrar.
  - **C (pill switcher):** fila de `pill`s (radio 999) con el nombre de cada broker; la activa rellena `BRASA` sobre `TINTA`.
- **Endpoint/método:** ninguno nuevo en el broker — nueva sección `[[brokers]]` en `config.toml` (extensión del modelo `Config`). El activo dispara `GET /admin/info` + `/salud`.
- **Trazabilidad:** evento "broker activo cambiado a X" en historial.
- **Prioridad:** media

---

### acceso-08 — Allowlist de IPs (control de quién puede conectarse)
- **Problema:** el broker no restringe por IP; cualquiera en la red con el token entra. Max no tiene control de red desde la app.
- **Propuesta:** gestión de una allowlist de IPs/CIDR que el broker aplica como middleware antes de `verificar_token`.
- **Variantes DS:**
  - **A (tabla CRUD):** `Table` Ethos con columnas IP/CIDR · nota · añadida-el; botón dorado "+ Añadir IP", acción borrar por fila (icono `HUMO`→rojo en hover).
  - **B (modal gestor):** botón "Allowlist (3)" con `Badge` del nº → `Dialog` con la lista editable + `Input` "añadir IP/CIDR" + `Switch` "activar allowlist".
  - **C (chips):** las IPs como `pill`s eliminables; un `Input` al final para añadir; toggle global "solo permitir estas IPs".
- **Endpoint/método:** **(requiere endpoints nuevos)** `GET /admin/allowlist`, `POST /admin/allowlist/add`, `POST /admin/allowlist/remove`, `POST /admin/allowlist/toggle` — más un middleware nuevo en el broker (antes de `verificar_token`).
- **Trazabilidad:** historial de cambios de allowlist (quién/cuándo añadió/quitó una IP).
- **Prioridad:** media

---

### acceso-09 — Aviso de "broker expuesto sin token" (seguridad)
- **Problema:** el broker ya emite `warn!("broker EXPUESTO … SIN token")` cuando escucha en host no-loopback sin token, pero **la app no lo muestra**; Max no se entera del agujero.
- **Propuesta:** detectar la condición (host de `/admin/info` no es loopback **y** config sin token) y mostrar una alerta de seguridad persistente en Acceso.
- **Variantes DS:**
  - **A (banner de seguridad):** franja ámbar/terracota arriba de la card, icono escudo, "⚠ Broker expuesto en la red SIN token — cualquiera puede conectarse" + CTA "Poner token" (salta a acceso-02).
  - **B (badge en el endpoint):** `Badge` rojo "EXPUESTO" junto a la fila `endpoint`; hover = `Tooltip` con la explicación.
  - **C (semáforo de postura):** un indicador de "postura de seguridad" (verde: loopback o con token / rojo: expuesto sin token) en la cabecera.
- **Endpoint/método:** ninguno nuevo — se deduce de `host` (`/admin/info`) + presencia de token en config. Opcional: exponer un flag `expuesto_sin_token` en `/admin/info`.
- **Trazabilidad:** —
- **Prioridad:** **alta**

---

### acceso-10 — Estado de auth explícito (autenticado / anónimo / rechazado)
- **Problema:** Acceso no dice si la sesión está autenticada con token, corriendo en modo anónimo (broker sin token) o rechazada; solo hay chip de salud.
- **Propuesta:** chip de estado de autenticación separado del de salud.
- **Variantes DS:**
  - **A (chip auth):** junto a "salud", un chip "auth: OK" (`BRASA`) / "anónimo" (`HUMO`) / "rechazado 401" (rojo terroso) usando `chip_estado`.
  - **B (fila dedicada):** nueva fila key→value "autenticación" con el estado en mono + dot de color.
  - **C (badge en el token):** `Badge` pegado a la fila `token`: "verificado ✓" / "sin verificar" / "inválido".
- **Endpoint/método:** ninguno nuevo — se infiere del resultado de una ruta protegida (`POST /listar`: 200 = auth ok, 401 = rechazado, sin token configurado = anónimo).
- **Trazabilidad:** transiciones de estado de auth → historial (acceso-14).
- **Prioridad:** media

---

### acceso-11 — Regenerar / rotar token del broker en caliente
- **Problema:** rotar el token exige reiniciar el proceso del broker con otro `--token`; no hay rotación en caliente ni desde la app.
- **Propuesta:** acción "Regenerar token" que genera un token nuevo, lo aplica en el broker y re-siembra la config local en una operación atómica.
- **Variantes DS:**
  - **A (modal de rotación):** `Dialog` "Rotar token" → genera un candidato (mono, con botón copiar), aviso "todas las sesiones con el token viejo se desconectarán", botones "Aplicar" (dorado) / "Cancelar".
  - **B (botón + confirm):** botón peligro (borde terracota) "Regenerar token" con `Popover` de confirmación de doble paso.
  - **C (asistente):** flujo de 2 pasos (generar → confirmar aplicación) con barra de progreso Ethos.
- **Endpoint/método:** **(requiere endpoint nuevo)** `POST /admin/rotar-token` en el broker (regenera y actualiza el token en caliente); la app persiste el nuevo en `config.toml`.
- **Trazabilidad:** evento crítico "token rotado" con marca temporal (sin valores) en historial de auth.
- **Prioridad:** baja

---

### acceso-12 — Latencia / RTT al broker
- **Problema:** Max no ve la salud de la conexión (rápida/lenta); solo "ok/mal".
- **Propuesta:** medir el RTT de `/salud` en cada comprobación y mostrarlo, con umbral de color.
- **Variantes DS:**
  - **A (chip latencia):** chip "RTT 12 ms" en la card; verde <50ms / ámbar <200ms / rojo terroso ≥200ms.
  - **B (sparkline):** mini-gráfico Ethos (línea `BRASA` sobre `TINTA2`) de las últimas N mediciones bajo la fila endpoint.
  - **C (fila mono):** fila "latencia" con el valor en mono + tendencia (▲/▼) respecto a la anterior.
- **Endpoint/método:** ninguno nuevo — se cronometra el `GET /salud` existente en el cliente.
- **Trazabilidad:** serie temporal de RTT (para el sparkline / historial).
- **Prioridad:** baja

---

### acceso-13 — Historial de conexiones (trazabilidad de sesión)
- **Problema:** no hay rastro de cuándo la app se conectó/desconectó/reconectó ni a qué broker; imposible auditar la actividad de conexión.
- **Propuesta:** log local de eventos de conexión (conectado, perdido, reconectado, broker cambiado) visible desde Acceso.
- **Variantes DS:**
  - **A (timeline en Popover):** botón "Historial" → `Popover`/`Dialog` con un timeline vertical Ethos: dot `BRASA` + hora mono + evento ("Conectado a OTUS", "Conexión perdida", "Reconectado").
  - **B (tabla desplegable):** `Table` colapsable bajo la card con columnas hora · evento · broker · resultado.
  - **C (mini-feed):** las 3 últimas líneas siempre visibles al pie de la card en `texto_terciario`, con "ver todo" que abre el modal.
- **Endpoint/método:** ninguno nuevo — eventos generados por la app (transiciones de `cargar_broker`) persistidos localmente (ring buffer / archivo).
- **Trazabilidad:** ESTE ES el artefacto de trazabilidad de conexión.
- **Prioridad:** media

---

### acceso-14 — Historial de intentos de autenticación (auditoría de seguridad)
- **Problema:** no se registra cuándo hubo un 401, con qué broker, ni cuántos fallos seguidos; no hay auditoría de auth.
- **Propuesta:** log de intentos de auth (éxito/401) con marca temporal y broker, separado del historial general.
- **Variantes DS:**
  - **A (tabla de auditoría):** `Table` "Intentos de auth" con hora · broker · resultado (✓/401) · latencia; filas de fallo con fondo rojo terroso muy tenue.
  - **B (contador + detalle):** `Badge` "3 fallos hoy" en la cabecera → `Dialog` con el detalle.
  - **C (timeline de seguridad):** timeline específico donde los 401 son dots rojos y los éxitos dots `BRASA`.
- **Endpoint/método:** local (app) por defecto; opcional **(endpoint nuevo)** `GET /admin/auth-log` si el broker registra los 401 server-side (más fiable para detectar ataques desde otras IPs).
- **Trazabilidad:** ES el artefacto de auditoría de seguridad.
- **Prioridad:** media

---

### acceso-15 — Reconexión automática con backoff + control manual
- **Problema:** al caer el broker, la desktop reintenta según `refresh_ms` sin política visible; Max no puede pausar/forzar reconexión ni ve el estado de reintento.
- **Propuesta:** política de reconexión con backoff exponencial, indicador de estado y controles pausar/reconectar-ya.
- **Variantes DS:**
  - **A (barra de estado):** cuando está offline, una barra "Reconectando… próximo intento en 4s" con spinner Ethos + botón "Reintentar ahora".
  - **B (switch + chip):** `Switch` "Auto-reconectar" + chip "reintentando (3/∞)"; botón "Forzar ahora".
  - **C (control en la card):** fila "reconexión" con estado (activa/pausada) + botones pequeños play/pause y "ahora".
- **Endpoint/método:** ninguno nuevo — lógica de cliente sobre `GET /salud`; el `refresh_ms` de config alimenta el intervalo base.
- **Trazabilidad:** cada intento de reconexión se refleja en el historial de conexión (acceso-13).
- **Prioridad:** media

---

### acceso-16 — Validar y normalizar la `broker_url` antes de conectar
- **Problema:** hoy se puede guardar una URL mal formada (sin esquema, con barra final) y solo falla al conectar, sin pista clara.
- **Propuesta:** validación de frontera de la URL (esquema http/https, host, puerto, sin barra final) con feedback inmediato.
- **Variantes DS:**
  - **A (validación inline):** al escribir en el `Input` de URL, borde verde salvia si válida / terracota si no, con nota terciaria "falta http:// " o "quita la barra final".
  - **B (chip de forma):** chip "URL válida ✓" / "revisar formato" junto al campo.
  - **C (autocorrección sugerida):** si detecta `10.0.0.67:7899` sin esquema, sugiere `http://10.0.0.67:7899` con un botón "usar sugerencia" dorado.
- **Endpoint/método:** ninguno — validación pura en el cliente (parse, don't validate) antes de reconstruir el `ClienteBroker`.
- **Trazabilidad:** —
- **Prioridad:** media

---

### acceso-17 — Ver instancias conectadas y kickear una desde Acceso
- **Problema:** Acceso muestra "N instancias" como número muerto; Max no ve quiénes son ni puede expulsar a un peer indebido — control de seguridad/red que hoy no existe en esta pestaña.
- **Propuesta:** expandir "N instancias" a una lista de peers conectados con acción de expulsión.
- **Variantes DS:**
  - **A (popover lista):** clic en "N instancias" → `Popover` con la lista (id · host · última actividad) y un icono expulsar por fila.
  - **B (mini-tabla):** `Table` compacta bajo el chip de salud con columna acción "Kick" (botón peligro pequeño).
  - **C (badges expandibles):** cada instancia como `Badge`; clic abre `Dialog` con detalle + botón "Expulsar de la red".
- **Endpoint/método:** `POST /listar` (ya existe) para la lista; expulsión reusa `POST /salir` (ya existe, hoy lo llama el propio peer) — **verificar** si `/salir` admite expulsión por un tercero o requiere un `POST /admin/kick` nuevo.
- **Trazabilidad:** evento "peer X expulsado" en historial de seguridad (acceso-14).
- **Prioridad:** media

---

## 3. Impacto y orden sugerido

- **Núcleo (alta) — habilita el CRUD que Max reclama:** acceso-01 (editar URL), acceso-02 (editar token), acceso-05 (probar conexión), acceso-06 (diagnóstico de error), acceso-09 (aviso de exposición). Todo con endpoints **existentes** (`/salud`, `/admin/info`, `/listar`).
- **Segunda ola (media):** acceso-03, acceso-07 (multi-broker), acceso-10, acceso-13/14 (trazabilidad), acceso-15, acceso-16, acceso-17.
- **Requieren cambios en el broker (evaluar aparte):** acceso-08 (allowlist), acceso-11 (rotar token), y opcionalmente el server-side de acceso-14/17.
- **Cosmético/observabilidad (baja):** acceso-04, acceso-12.

**Riesgo transversal:** manejo del secreto (token) en portapapeles y en pantalla (acceso-03/04/11) — decidir la política de higiene (auto-borrado, confirmaciones) antes de implementar.
