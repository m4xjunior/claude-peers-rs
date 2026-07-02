# RFC — Pestaña Redis (peers-desktop): control, CRUD y trazabilidad del almacén

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

| Campo | Valor |
|-------|-------|
| **Ámbito** | `crates/peers-desktop` → `src/vista/redis.rs` |
| **Autor** | Max (LexusFX) |
| **Fecha** | 2026-07-02 |
| **Estado** | PROPUESTO (pendiente de decisión, ANTES de implementar) |
| **Referencia** | Paridad con `peers-tui/src/ui/redis.rs` + `main.rs`; endpoints en `peers-broker/src/main.rs` |
| **Design System** | Ethos — tinta `#100D0A`, tinta2 `#1A1611`, papel `#ECE5D7`, brasa `#C9A96E`, humo `#938B7B`, línea `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999 |

---

## 1. Contexto

### Qué hace HOY la pestaña Redis en desktop

`vista/redis.rs` es un puerto casi 1:1 de la TUI y es **prácticamente solo lectura**:

- Pinta **DOS tablas al 50/50**: izquierda **Colas de mensajes** (`peer → nº pendientes`), derecha **Outbox pendiente** (`peer → nº pendientes`). Datos vienen de `GET /admin/redis` (`RespuestaAdminRedis { total_instancias, colas: Vec<ColaResumen>, outbox: Vec<ColaResumen> }`).
- La tabla de colas es **seleccionable** (click → `SeleccionarColaRedis { indice }`, resalta la fila).
- **Única escritura:** botón "Purgar cola + outbox" del peer seleccionado → `POST /admin/purgar { id }`. Espejo de la tecla `p` de la TUI.
- Banner de error rojo si el broker falla; estado vacío mientras no hay datos.

### Qué le FALTA (lo que Max no puede hacer hoy)

`ColaResumen` solo trae `{ id, pendientes }` — un **contador ciego**. Max ve "peer X tiene 7 mensajes" pero **no puede**:

1. **Ver el contenido** de esa cola (qué mensajes, de quién, con qué texto/estado). No hay pop-up de inspección.
2. **Ver el outbox** desglosado (`ItemOutbox { id, de_id, para_id, texto, confirmado, ... }`) — solo el número.
3. **Reenviar** un ítem pendiente del outbox/historial (el broker YA expone `POST /admin/reenviar { msg_id }`, la desktop no lo usa — solo lo usa la pantalla Trazabilidad de la TUI).
4. **Inspeccionar el historial durable** de una cola vía `GET /admin/historial?id=&desde=&estado=` (existe, no cableado aquí).
5. **Refrescar manualmente** (hoy depende del polling implícito; no hay botón ni indicador de "última actualización").
6. **Purgar con confirmación** (hoy `Purgar` es un click directo sin diálogo → borrado destructivo sin red de seguridad).
7. **Purga selectiva** (solo cola, solo outbox, o solo mensajes ya procesados) — `/admin/purgar` hoy borra ambos.
8. **Ver la máquina de estados** de un mensaje (`EstadoMensaje`: Enviado→Entregado→Leído→Procesado / Fallido / DeadLetter) — trazabilidad que ya vive en `Mensaje`.
9. **Ordenar/filtrar/buscar** peers en las tablas (con decenas de peers, encontrar uno es scroll a ciegas).
10. **Ver agregados** (total de mensajes pendientes en toda la red, peer más cargado, colas atascadas).
11. **Accesibilidad** — nada es navegable por teclado (la TUI SÍ: `j/k`, `p`); no hay foco visible ni tooltips.

### Endpoints del broker verificados (disponibles y hoy sin usar en desktop)

| Endpoint | Método | Payload / Query | Devuelve | Usado en desktop |
|----------|--------|-----------------|----------|------------------|
| `/admin/redis` | GET | — | `RespuestaAdminRedis` | ✅ sí |
| `/admin/purgar` | POST | `{ id }` | ok | ✅ sí |
| `/admin/historial` | GET | `id`, `desde?`, `estado?` | `Vec<Mensaje>` | ❌ no |
| `/admin/reenviar` | POST | `{ msg_id }` | `{ ok, msg_id }` | ❌ no |
| `/admin/info` | GET | — | `{ host, puerto, instancias, version }` | ❌ no |
| `/admin/tareas` | GET | — | tareas globales | ❌ no (otra pestaña) |

> **Nota de scope:** algunas features de abajo requieren un **endpoint nuevo** que hoy NO existe (p.ej. contenido crudo de la cola de bandeja por peer, o purga selectiva). Cada feature lo marca explícitamente como `[EXISTE]` o `[NUEVO]`. Las `[NUEVO]` son propuestas de extensión del broker; no se dan por hechas.

---

## 2. Features propuestas (≥15)

Cada una: **problema** (qué no puede hacer Max) → **propuesta** → **2-3 variantes de diseño Ethos** → **endpoint** → **trazabilidad** → **prioridad**.

---

### redis-01 — Inspeccionar el contenido de una cola (pop-up)
- **Problema:** Max ve "peer X: 7 mensajes" pero no puede abrir la cola para ver QUÉ mensajes hay, de quién, con qué texto/estado. Es un contador ciego.
- **Propuesta:** doble-click (o botón "Ver cola") sobre la fila → **modal Dialog** con la lista de mensajes de la bandeja de ese peer, cada uno con `id`, `de_id`, texto (truncado + expandible), `estado` y timestamp.
- **Variantes Ethos:**
  - **A (Dialog modal):** `superficie_card` centrada sobre overlay tinta a 70% de opacidad, título Fraunces "Cola de {peer}", tabla mono de mensajes con `chip_estado` por fila, botón secundario "Cerrar".
  - **B (panel lateral drawer):** panel que entra desde la derecha (40% ancho), borde izquierdo brasa, se cierra con Esc; deja las dos tablas visibles detrás.
  - **C (expansión inline):** la fila se expande acordeón mostrando los mensajes debajo, con sangría y borde izquierdo brasa tenue.
- **Endpoint:** `GET /admin/historial?id={peer}&estado=enviado` `[EXISTE]` para aproximar los pendientes; contenido exacto de bandeja = `[NUEVO]` `GET /admin/bandeja?id=` si se quiere lo realmente encolado (no borrado).
- **Trazabilidad:** por mensaje, su `estado` actual y `enviado_en`.
- **Prioridad:** **alta**

### redis-02 — Inspeccionar el outbox de un peer (pop-up)
- **Problema:** la tabla derecha solo dice "peer X: 3 ítems outbox". No se ve QUÉ hay pendiente de entregar ni si está `confirmado`.
- **Propuesta:** click en fila de outbox → modal con los `ItemOutbox { id, de_id, para_id, texto, confirmado, ... }` de ese peer, badge verde/ámbar según `confirmado`.
- **Variantes Ethos:**
  - **A (Dialog):** modal gemelo del redis-01 pero con columna "ACK" (`chip_estado` verde=confirmado, humo=pendiente).
  - **B (tabla derecha operable):** hacer la tabla de outbox seleccionable (hoy es solo lectura) y mostrar detalle en un panel inferior fijo dentro de la misma card.
  - **C (badge + tooltip):** el número de outbox se vuelve un `Badge` pill; hover muestra Tooltip con los primeros 3 ítems.
- **Endpoint:** `[NUEVO]` `GET /admin/outbox?id=` (el broker tiene `outbox_pendientes(id)` internamente; falta exponerlo por HTTP; hoy `/admin/redis` solo cuenta `.len()`).
- **Trazabilidad:** estado de ACK (`confirmado`) por ítem.
- **Prioridad:** **alta**

### redis-03 — Reenviar un ítem del outbox/historial
- **Problema:** si un peer ignoró un mensaje (quedó en outbox sin ACK), Max no tiene forma de re-empujarlo desde desktop. La TUI SÍ lo hace (tecla `r` en Trazabilidad).
- **Propuesta:** en el pop-up de cola/outbox, botón "Reenviar" por fila → `POST /admin/reenviar { msg_id }`; toast de confirmación con el nuevo `msg_id`.
- **Variantes Ethos:**
  - **A (botón dorado por fila):** `boton_primario` compacto "↻ Reenviar" a la derecha de cada mensaje del modal.
  - **B (acción en menú contextual):** click derecho sobre la fila → Popover con "Reenviar / Copiar id / Ver timeline".
  - **C (barra de acción del modal):** seleccionas un mensaje dentro del modal y un botón único abajo "Reenviar seleccionado".
- **Endpoint:** `POST /admin/reenviar { msg_id }` `[EXISTE]` (devuelve `{ ok, msg_id }`).
- **Trazabilidad:** el broker timbra `reenviado_de` + incrementa `reenvios`; mostrar "reenviado #N" en el mensaje.
- **Prioridad:** **alta**

### redis-04 — Confirmación antes de purgar (Dialog destructivo)
- **Problema:** hoy "Purgar cola + outbox" es un click directo → borrado irreversible sin red de seguridad. Un mal click y se pierde la bandeja de un peer.
- **Propuesta:** el botón abre un **Dialog de confirmación** que nombra el peer y el nº exacto de mensajes+outbox que se borrarán, con botón de peligro.
- **Variantes Ethos:**
  - **A (Dialog rojo):** modal con título Fraunces "¿Purgar {peer}?", cuerpo "Se borrarán 7 mensajes y 3 outbox. Irreversible.", botón peligro (fondo rojo semántico `#7F1D1D`) + secundario "Cancelar".
  - **B (confirm-por-escritura):** input donde Max teclea el id del peer para habilitar el botón (patrón GitHub delete).
  - **C (hold-to-confirm):** botón que hay que mantener pulsado 1.5s, con anillo brasa que se rellena.
- **Endpoint:** `POST /admin/purgar { id }` `[EXISTE]`.
- **Trazabilidad:** registrar en un log local de acciones (ver redis-15) la purga con timestamp.
- **Prioridad:** **alta**

### redis-05 — Refrescar manual + indicador de "última actualización"
- **Problema:** no hay botón de refresco ni forma de saber cuán fresca es la foto de las colas.
- **Propuesta:** botón "↻ Refrescar" en la cabecera + texto "actualizado hace Xs" que se recalcula.
- **Variantes Ethos:**
  - **A (botón + eyebrow):** `boton_secundario` "↻" a la derecha del título; debajo `eyebrow` "ACTUALIZADO 12:04:31".
  - **B (icono giratorio):** icono brasa que gira mientras el fetch está en curso, se detiene al llegar la respuesta.
  - **C (pill de estado):** `Badge` pill verde "en vivo" / ámbar "12s" según antigüedad.
- **Endpoint:** `GET /admin/redis` `[EXISTE]` (re-fetch).
- **Trazabilidad:** timestamp de la última respuesta OK.
- **Prioridad:** **alta**

### redis-06 — Auto-refresco configurable (intervalo)
- **Problema:** las colas cambian solas (peers consumen mensajes); una foto estática miente a los pocos segundos.
- **Propuesta:** toggle "Auto" + selector de intervalo (2s / 5s / 10s / off) que dispara el re-fetch periódico.
- **Variantes Ethos:**
  - **A (Switch + Select):** `Switch` "Auto" y a su lado un `Select` con los intervalos, en la cabecera.
  - **B (segmented pills):** grupo de pills 2s/5s/10s/off, la activa con fondo brasa tenue y borde brasa.
  - **C (Switch simple):** solo on/off con intervalo fijo 5s, minimalista.
- **Endpoint:** `GET /admin/redis` `[EXISTE]` (en loop temporizado).
- **Trazabilidad:** —
- **Prioridad:** media

### redis-07 — Buscar / filtrar peers en las tablas
- **Problema:** con decenas de peers, encontrar uno es scroll a ciegas.
- **Propuesta:** `Input` de búsqueda que filtra ambas tablas por substring del `id` en vivo.
- **Variantes Ethos:**
  - **A (Input con icono lupa):** input tinta2, borde línea, placeholder humo "buscar peer…", icono lupa brasa a la izquierda.
  - **B (filtro pill):** chips rápidos "con pendientes" / "vacías" / "todas" además del texto libre.
  - **C (comando /):** atajo `/` enfoca el buscador (paridad con el modelo mental TUI).
- **Endpoint:** filtrado client-side sobre `/admin/redis` `[EXISTE]`.
- **Trazabilidad:** —
- **Prioridad:** media

### redis-08 — Ordenar las tablas por columna
- **Problema:** las filas salen en orden de registro; Max no puede ver "quién tiene MÁS pendientes" de un vistazo.
- **Propuesta:** cabeceras de columna clicables que ordenan por `id` (A-Z) o por `pendientes` (desc/asc).
- **Variantes Ethos:**
  - **A (cabecera clicable + flecha):** el `eyebrow` de la columna se vuelve clicable, muestra ▲/▼ brasa según orden.
  - **B (Select de orden):** un `Select` "Ordenar por: pendientes ↓ / peer A-Z".
  - **C (auto-orden por carga):** siempre ordena por pendientes desc, sin control (peer más cargado arriba).
- **Endpoint:** ordenación client-side sobre `/admin/redis` `[EXISTE]`.
- **Trazabilidad:** —
- **Prioridad:** media

### redis-09 — Resaltar colas atascadas / de alto volumen
- **Problema:** un contador alto (peer con 200 mensajes sin consumir) se ve igual que uno con 1. No hay señal visual de "esto está mal".
- **Propuesta:** umbral configurable; filas por encima se pintan con acento de alerta.
- **Variantes Ethos:**
  - **A (borde izquierdo rojo):** filas > umbral con borde izquierdo rojo semántico + número en rojo.
  - **B (badge "atascada"):** `Badge` pill ámbar/rojo junto al peer cuando supera el umbral.
  - **C (barra de proporción):** mini-barra brasa bajo el número, proporcional al máximo de la red (heatmap sutil).
- **Endpoint:** `GET /admin/redis` `[EXISTE]` (umbral es lógica de vista).
- **Trazabilidad:** —
- **Prioridad:** media

### redis-10 — Panel de agregados de la red
- **Problema:** no hay una cifra global: cuántos mensajes pendientes hay en TODA la red, cuántos peers con outbox, peer más cargado.
- **Propuesta:** fila de "stat cards" sobre las tablas: total mensajes pendientes, total outbox, nº peers activos, peer más cargado.
- **Variantes Ethos:**
  - **A (stat cards):** 4 `superficie_card` pequeñas con número mono grande (brasa) + `eyebrow` de etiqueta.
  - **B (barra resumen):** una sola tira horizontal con los 4 datos separados por punto medio humo.
  - **C (donut/sparkline):** mini-visualización de distribución de carga por peer.
- **Endpoint:** derivado de `/admin/redis` `[EXISTE]`; `total_instancias` ya viene.
- **Trazabilidad:** —
- **Prioridad:** media

### redis-11 — Timeline de estados de un mensaje
- **Problema:** `EstadoMensaje` (Enviado→Entregado→Leído→Procesado / Fallido / DeadLetter) ya existe en `Mensaje`, pero desktop no muestra el ciclo de vida de ningún mensaje. La TUI SÍ tiene timeline (Trazabilidad).
- **Propuesta:** desde el pop-up de cola (redis-01), click en un mensaje → sub-vista con su línea temporal timbrada (`enviado_en`, `entregado_en`, `leido_en`, `procesado_en`).
- **Variantes Ethos:**
  - **A (stepper vertical):** pasos con punto brasa relleno (alcanzado) / humo (pendiente), timestamp mono a la derecha de cada paso.
  - **B (chips de estado en fila):** secuencia horizontal de `chip_estado`, el actual resaltado en brasa.
  - **C (tabla de transiciones):** filas "estado | timestamp" en mono.
- **Endpoint:** `GET /admin/historial?id=&desde=` `[EXISTE]` (el `Mensaje` trae los timestamps por estado).
- **Trazabilidad:** ES la trazabilidad — ciclo de vida completo timbrado por el broker.
- **Prioridad:** media

### redis-12 — Historial durable por cola (auditoría)
- **Problema:** las colas actuales solo muestran lo PENDIENTE. Max no puede ver el histórico de lo ya procesado/borrado por peer.
- **Propuesta:** pestaña/tab dentro del pop-up de cola: "Pendientes | Historial", el segundo lista el durable con filtro por estado.
- **Variantes Ethos:**
  - **A (Tabs Ethos):** dos tabs mono, la activa con subrayado brasa; cuerpo comparte layout.
  - **B (Select de estado):** un `Select` "estado: todos/enviado/entregado/leído/procesado/fallido" filtra la lista.
  - **C (línea de tiempo scrollable):** cursor `desde` para paginar hacia atrás con "cargar más".
- **Endpoint:** `GET /admin/historial?id=&desde=&estado=` `[EXISTE]`.
- **Trazabilidad:** historial durable completo por peer.
- **Prioridad:** media

### redis-13 — Purga selectiva (cola vs outbox vs procesados)
- **Problema:** `/admin/purgar` borra cola + outbox en bloque. Max no puede limpiar solo el outbox de un peer, o solo los mensajes ya procesados, conservando lo pendiente.
- **Propuesta:** en el Dialog de purga (redis-04), radios de alcance: "todo | solo cola | solo outbox | solo procesados".
- **Variantes Ethos:**
  - **A (radios + Dialog):** grupo de `Checkbox`/radios dentro del Dialog de confirmación, cada uno con su recuento estimado.
  - **B (menú split-button):** botón "Purgar ▾" que despliega Popover con los 4 alcances.
  - **C (dos botones):** "Purgar cola" y "Purgar outbox" separados en la barra de acción.
- **Endpoint:** `[NUEVO]` `POST /admin/purgar { id, alcance }` (extender el payload; hoy borra ambos incondicionalmente).
- **Trazabilidad:** log de acción con el alcance elegido (redis-15).
- **Prioridad:** baja

### redis-14 — Purga masiva / bulk (multi-selección)
- **Problema:** para vaciar 10 peers hay que seleccionar y purgar uno a uno.
- **Propuesta:** checkboxes por fila + "Purgar seleccionados (N)"; o "Purgar todas las colas vacías/atascadas".
- **Variantes Ethos:**
  - **A (checkbox por fila + barra bulk):** `Checkbox` a la izquierda de cada fila; barra inferior con contador y botón peligro.
  - **B (acciones rápidas):** botones "Purgar todas las vacías" / "Purgar todas > umbral".
  - **C (modo selección):** toggle "Seleccionar" que revela los checkboxes solo cuando se activa.
- **Endpoint:** `POST /admin/purgar { id }` `[EXISTE]` en bucle (N llamadas), o `[NUEVO]` `POST /admin/purgar-lote { ids }`.
- **Trazabilidad:** log de cada purga del lote (redis-15).
- **Prioridad:** baja

### redis-15 — Log local de acciones administrativas (auditoría de la sesión)
- **Problema:** cuando Max purga o reenvía no queda rastro; no puede revisar "qué hice y cuándo" en esta sesión.
- **Propuesta:** panel/registro colapsable "Actividad" que lista cada acción admin (purgar, reenviar) con timestamp, peer y resultado.
- **Variantes Ethos:**
  - **A (drawer inferior):** panel colapsable al pie, líneas mono "12:04:31 · purgó cola de {peer} (7 msgs)".
  - **B (Notification stack):** cada acción emite una `Notification` toast efímera que también se acumula en un historial.
  - **C (tab lateral):** un tab "Log" junto a las tablas.
- **Endpoint:** local (no requiere broker); opcionalmente `[NUEVO]` `GET /admin/auditoria` si se quiere persistente server-side.
- **Trazabilidad:** ES trazabilidad de las acciones del propio admin.
- **Prioridad:** baja

### redis-16 — Copiar id de peer / msg_id al portapapeles
- **Problema:** para reenviar por CLI o depurar, Max necesita el `id`/`msg_id` exacto y no puede copiarlo.
- **Propuesta:** botón/acción "copiar" en filas y en el detalle de mensaje.
- **Variantes Ethos:**
  - **A (icono al hover):** icono de copia humo que aparece al pasar sobre la fila, brasa al hover.
  - **B (click en el id):** el propio texto del id es clicable y copia, con Tooltip "copiado ✓".
  - **C (menú contextual):** entrada "Copiar id" en el Popover de click derecho.
- **Endpoint:** local (portapapeles GPUI); sin broker.
- **Trazabilidad:** —
- **Prioridad:** baja

### redis-17 — Navegación por teclado + foco visible (accesibilidad)
- **Problema:** la desktop no es navegable por teclado; la TUI SÍ (`j/k` mover, `p` purgar). Sin foco visible ni atajos, Max pierde velocidad y no hay a11y.
- **Propuesta:** `↑/↓` mueven la selección, `Enter` abre el pop-up de cola, `p` abre el diálogo de purga, `r` reenvía, `/` enfoca buscador, `Esc` cierra modales.
- **Variantes Ethos:**
  - **A (anillo de foco brasa):** la fila enfocada muestra borde brasa 2px + fondo brasa tenue; leyenda de atajos al pie.
  - **B (barra de ayuda):** franja inferior con los atajos activos en `eyebrow` mono (espejo del footer de la TUI).
  - **C (paleta de comandos):** `Cmd+K` abre un Popover con todas las acciones buscables.
- **Endpoint:** despacha las mismas acciones GPUI que ya existen + las nuevas.
- **Trazabilidad:** —
- **Prioridad:** media

### redis-18 — Ir al peer / cross-navegación
- **Problema:** viendo una cola atascada, Max no puede saltar al peer en la pestaña Peers para ver su jornada/actividad. Todo está aislado.
- **Propuesta:** acción "Ver peer" en el detalle → cambia a la pestaña Peers con ese peer seleccionado.
- **Variantes Ethos:**
  - **A (botón dorado):** `boton_secundario` "→ Ver peer" en el pop-up de cola.
  - **B (id clicable):** el `id` del peer es un enlace brasa que navega.
  - **C (menú contextual):** entrada "Ir a Peers" en el Popover de fila.
- **Endpoint:** navegación interna; datos del peer vía la pestaña Peers (`/listar`) `[EXISTE]`.
- **Trazabilidad:** enlaza cola ↔ actividad/jornada del peer.
- **Prioridad:** baja

### redis-19 — Estado de conexión del broker + reintento
- **Problema:** hoy solo hay un banner rojo genérico cuando falla; no distingue offline / 401 / timeout, ni ofrece reintentar sin cambiar de pestaña.
- **Propuesta:** indicador de estado del broker en cabecera (verde/rojo) + botón "Reintentar" en el banner de error.
- **Variantes Ethos:**
  - **A (pill de estado):** `Badge` pill "broker: en línea" verde / "offline" rojo, con host de `/admin/info`.
  - **B (banner accionable):** el banner rojo actual gana un `boton_secundario` "Reintentar".
  - **C (punto + tooltip):** punto de color junto al título, Tooltip con detalle (código, host, versión).
- **Endpoint:** `GET /admin/info` `[EXISTE]` (host/puerto/versión) + reintento de `/admin/redis`.
- **Trazabilidad:** último error con su código y hora.
- **Prioridad:** media

### redis-20 — Exportar el estado de colas (snapshot)
- **Problema:** Max no puede llevarse una foto del estado (para un reporte o comparar antes/después de una purga).
- **Propuesta:** botón "Exportar" que copia/guarda el `RespuestaAdminRedis` como JSON o CSV.
- **Variantes Ethos:**
  - **A (botón + toast):** `boton_secundario` "Exportar JSON"; al pulsar, `Notification` "copiado al portapapeles ✓".
  - **B (Select de formato):** "Exportar ▾" → JSON / CSV / Markdown.
  - **C (guardar archivo):** diálogo del SO para guardar el snapshot con nombre `redis-YYYYMMDD-HHMM.json`.
- **Endpoint:** `GET /admin/redis` `[EXISTE]` (serializa lo ya cargado; sin llamada extra).
- **Trazabilidad:** el snapshot es evidencia puntual del estado.
- **Prioridad:** baja

---

## 3. Resumen de prioridades

| Prioridad | Features |
|-----------|----------|
| **Alta** | redis-01, redis-02, redis-03, redis-04, redis-05 |
| **Media** | redis-06, redis-07, redis-08, redis-09, redis-10, redis-11, redis-12, redis-17, redis-19 |
| **Baja** | redis-13, redis-14, redis-15, redis-16, redis-18, redis-20 |

**Total: 20 features.**

## 4. Endpoints nuevos que estas features pedirían al broker

| Endpoint propuesto | Para | Features |
|--------------------|------|----------|
| `GET /admin/bandeja?id=` | contenido crudo real de la bandeja (no borrado) de un peer | redis-01 |
| `GET /admin/outbox?id=` | exponer `outbox_pendientes(id)` (hoy solo se cuenta `.len()`) | redis-02 |
| `POST /admin/purgar { id, alcance }` | purga selectiva (cola/outbox/procesados) | redis-13 |
| `POST /admin/purgar-lote { ids }` | purga en lote | redis-14 |
| `GET /admin/auditoria` | log admin persistente (opcional) | redis-15 |

> El resto de features (12 de 20) se implementan con endpoints **ya existentes** (`/admin/redis`, `/admin/purgar`, `/admin/historial`, `/admin/reenviar`, `/admin/info`) o son lógica pura de la vista.
