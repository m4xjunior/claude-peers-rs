# RFC — Pestaña **Peers** de la desktop GPUI (gestión + trazabilidad)

> Fecha: 2026-07-02. Estado: **borrador para comentarios** (propone features ANTES de decidir/implementar).
> Ámbito: `crates/peers-desktop/src/vista/peers.rs`. Referencias de paridad: `crates/peers-tui/src/ui/peers.rs`,
> teclas en `crates/peers-tui/src/main.rs`, endpoints en `crates/peers-broker/src/main.rs`.
> Design System: **Ethos** (TINTA `#100D0A`, TINTA2 `#1A1611`, PAPEL `#ECE5D7`, BRASA `#C9A96E`,
> HUMO `#938B7B`, LINEA `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999).

---

## Contexto

### Qué hace HOY la pestaña Peers (desktop)

`render_peers(&EstadoPantalla)` es una vista **pura, casi solo lectura**:

- Pinta la tabla `(id, directorio [+repo], resumen, visto, estado)` desde `POST /listar` (ya cargado en `EstadoPantalla.instancias`).
- Deriva una columna **estado** (ocioso/atascado/trabajando) cruzando `alertas` vigentes por `sujeto == id` (`estado_peer`).
- Filas **seleccionables** (`SeleccionarPeer { indice }`) que, con un peer marcado, muestran una **barra de acciones** con 3 botones:
  **Enviar mensaje** (`EnviarMensajePeer`), **Ver jornada** (`VerJornadaPeer`), **Cerrar** (`DeseleccionarPeer`).
- Banner de error si la última carga falló.

### Qué le FALTA (frente a la TUI y a los endpoints del broker)

La TUI en Peers ya tiene, por teclado, más de lo que la desktop porta:

- `m` → componer y **enviar mensaje** al peer (input modal real, no solo despachar acción sin flujo).
- `k` → **kick** del peer (`ClienteAdmin::salir` → `POST /salir`). **La desktop NO tiene kick.**
- `r` → **editar el resumen** del peer (`Input::Resumen` → `POST /definir-resumen`). **La desktop NO lo tiene** (`r` en TUI-Peers = editar resumen, no refrescar).

Y el broker expone endpoints que la pestaña Peers de la desktop **no usa**: `/salir` (kick), `/definir-resumen`,
`/jornada` (jornada consolidada: sesiones + tareas), `/listar-tareas` (tareas de la jornada del peer),
`/admin/historial` (mensajes por instancia), `/admin/reenviar`, `/admin/alertas`, `/admin/redis` (colas/outbox por peer),
`/factor-estimacion-peer` (factor aprendido del peer), `/latido`, `/salud`.

Además, las acciones que SÍ existen (`EnviarMensajePeer`, `VerJornadaPeer`) dependen de que la Fase 3 (`AppDesktop`)
implemente el flujo; hoy no hay **pop-up de detalle del peer**, ni composición de mensaje con `Input`/`Dialog`,
ni confirmación de acciones destructivas, ni filtro/búsqueda, ni refresco manual, ni accesibilidad (foco/teclado/tooltips).

### Objetivo de esta RFC

Documentar **≥15 áreas** de la pestaña Peers que deberían tener función/control/CRUD/trazabilidad, cada una con problema,
propuesta, 2-3 variantes de diseño Ethos, endpoint del broker, tipo de trazabilidad y prioridad. **No se implementa aquí**:
esto es para comentar y decidir alcance.

### Endpoints del broker disponibles para esta pestaña (verificados en `main.rs`)

| Endpoint | Método | Payload (verificado) | Uso en Peers |
|---|---|---|---|
| `/listar` | POST | — | tabla de peers (ya usado) |
| `/salir` | POST | `{ id }` | kick / expulsar peer |
| `/definir-resumen` | POST | `{ id, resumen }` | editar resumen del peer |
| `/enviar` | POST | `{ de, para, texto }` | enviar mensaje al peer |
| `/jornada` | POST | `{ instancia_id }` → `RespuestaJornada { sesiones, tareas }` | jornada consolidada |
| `/listar-tareas` | POST | `{ instancia_id }` → `Vec<Tarea>` | tareas del peer |
| `/tarea/asignar` | POST | `{ instancia_id, descripcion, estimado_seg? }` | asignar tarea a peer |
| `/tarea/reasignar` | POST | `{ tarea_id, nuevo_instancia_id }` | reasignar hacia/desde peer |
| `/factor-estimacion-peer` | GET | `?instancia_id=<id>` | factor aprendido del peer |
| `/admin/historial` | GET | (por instancia) | timeline de mensajes del peer |
| `/admin/reenviar` | POST | (mensaje) | reenviar mensaje pendiente |
| `/admin/alertas` | GET | — | alertas vigentes (cruce por sujeto) |
| `/admin/redis` | GET | — | colas/outbox pendientes por peer |
| `/admin/info` | GET | — | host/puerto/instancias/version del broker |
| `/latido` | POST | `{ id }` | ping/keepalive del peer |
| `/salud` | GET | — | salud del broker (exenta de auth) |

---

## Features propuestas (18)

### peers-01 — Pop-up de detalle del peer (Dialog modal)
- **Problema:** Max clica una fila y solo aparece una barra con 3 botones. No hay una vista consolidada del peer
  (id, directorio, repo, resumen, visto_en, estado, nº alertas, nº tareas abiertas, factor de estimación). "No consigo abrir ninguna tarea/peer, no hay pop-ups de visualización."
- **Propuesta:** Al hacer doble-click / `Enter` sobre la fila, abrir un `Dialog` modal **Detalle del peer** con secciones:
  Identidad (id BRASA + directorio + repo), Estado operativo (chip), Última señal (`visto_en` mono), Métricas
  (nº tareas abiertas, nº alertas vivas, factor), y una barra de acciones al pie (mensaje, kick, jornada, editar resumen).
- **Variantes de diseño Ethos:**
  1. **Dialog modal centrado** (`Dialog` del kit) sobre `superficie_card` TINTA2, borde LINEA, radio 14; título Fraunces PAPEL, eyebrows mono/HUMO por sección; overlay TINTA al 60%.
  2. **Panel lateral (drawer) derecho** que empuja la tabla: no tapa la lista, permite navegar peers con la tabla visible. Encabezado BRASA fijo, cuerpo scrolleable.
  3. **Expansión inline** (acordeón bajo la fila) que despliega el detalle sin modal — más ligero, útil en pantallas pequeñas.
- **Endpoint:** `/listar` (ya cargado) + `/jornada` (tareas/sesiones) + `/factor-estimacion-peer` + cruce `alertas`.
- **Trazabilidad:** consolida nº alertas y nº tareas; enlaces "ver alertas del peer" / "ver jornada".
- **Prioridad:** **alta**.

### peers-02 — Componer y enviar mensaje con Input real (no solo despachar Action)
- **Problema:** `EnviarMensajePeer` despacha una Action pero no hay flujo de composición: Max no puede escribir el texto ni ver a quién manda. En la TUI, `m` abre `Input::Mensaje { para_id }`.
- **Propuesta:** Pop-up de composición con `Input` multilínea, cabecera "Para: `<id>`" (BRASA), botones **Enviar** (dorado) / **Cancelar**, y feedback de éxito (`Notification`).
- **Variantes de diseño Ethos:**
  1. **Dialog modal** con `Textarea` (Input multilínea) TINTA2, placeholder HUMO "Escribe el mensaje…", contador de caracteres mono; botón primario BRASA.
  2. **Composer embebido** en el drawer de detalle (peers-01), sin segundo modal.
  3. **Popover anclado** al botón "Enviar mensaje" de la barra de acciones: input de una línea + Enter, para mensajes rápidos.
- **Endpoint:** `POST /enviar { de, para, texto }` (`de` = id del operador desktop / "broker").
- **Trazabilidad:** el mensaje entra al outbox del peer → visible en peers-08 (timeline) y peers-13 (colas).
- **Prioridad:** **alta**.

### peers-03 — Kick / expulsar peer (con confirmación)
- **Problema:** La desktop **no tiene kick**. La TUI sí (`k` → `POST /salir`). Max no puede desconectar un peer colgado/zombie.
- **Propuesta:** Botón **Expulsar** en la barra de acciones y en el detalle, con `Dialog` de confirmación (acción destructiva).
- **Variantes de diseño Ethos:**
  1. **Botón peligro** (variante roja apagada `#7F1D1D`, no BRASA) + `Dialog` de confirmación "¿Expulsar a `<id>`? Cerrará su sesión."
  2. **Acción en menú contextual** (Popover al click-derecho / botón "⋯" de la fila): agrupa acciones destructivas fuera de la vista principal.
  3. **Swipe/hold-to-confirm**: botón que exige mantener pulsado 1s (barra de progreso BRASA) para evitar clicks accidentales.
- **Endpoint:** `POST /salir { id }`.
- **Trazabilidad:** tras expulsar, refrescar `/listar`; `Notification` "peer '<id>' expulsado" y registro en un log de acciones del operador (ver peers-17).
- **Prioridad:** **alta**.

### peers-04 — Editar el resumen del peer (CRUD)
- **Problema:** La desktop no permite editar el `resumen`. La TUI sí (`r` → `Input::Resumen` → `POST /definir-resumen`). Max no puede corregir/anotar en qué anda un peer.
- **Propuesta:** Acción **Editar resumen** que abre un `Input` pre-relleno con el resumen actual; al confirmar, `POST /definir-resumen`.
- **Variantes de diseño Ethos:**
  1. **Edición in-place**: al hacer click en la celda `resumen`, se convierte en `Input` (borde BRASA al foco), Enter guarda, Esc cancela.
  2. **Dialog modal** "Editar resumen de `<id>`" con `Input` y botones guardar/cancelar.
  3. **Popover** anclado a un lápiz (icono) que aparece en hover sobre la celda resumen.
- **Endpoint:** `POST /definir-resumen { id, resumen }`.
- **Trazabilidad:** cambio de resumen puede loggearse en el historial de actividad del peer (peers-16).
- **Prioridad:** **media**.

### peers-05 — Ver jornada / trazabilidad del peer (pop-up rico)
- **Problema:** `VerJornadaPeer` navega a otra pantalla, pero no hay una vista de jornada consolidada (sesiones + tareas + tiempos) dentro del contexto del peer.
- **Propuesta:** Pop-up **Jornada de `<id>`**: lista de sesiones (inicio/fin, duración), tareas de la jornada (estado, estimado vs real), total trabajado.
- **Variantes de diseño Ethos:**
  1. **Dialog con dos columnas**: izquierda "Sesiones" (timeline vertical con puntos BRASA), derecha "Tareas" (filas con chips de estado).
  2. **Drawer con tabs** ("Sesiones" / "Tareas" / "Resumen") — reusa el patrón `Tabs` del kit.
  3. **Card apilada** en el detalle (peers-01): sección jornada colapsable con las últimas 3 sesiones y "ver todas".
- **Endpoint:** `POST /jornada { instancia_id }` → `RespuestaJornada { sesiones, tareas }`.
- **Trazabilidad:** núcleo de trazabilidad del peer — su jornada laboral (cuánto trabajó, en qué tareas).
- **Prioridad:** **alta**.

### peers-06 — Tareas abiertas del peer (lista + acciones rápidas)
- **Problema:** Max no puede ver, desde Peers, qué tareas tiene abiertas un peer ni actuar sobre ellas ("no hay casi funciones CRUD").
- **Propuesta:** Sección/pop-up **Tareas de `<id>`** con la lista (descripción, estado, estimado/real) y accesos directos a asignar/reasignar/forzar.
- **Variantes de diseño Ethos:**
  1. **Tabla embebida** en el detalle del peer, filas con chip de estado (Abierta/gris, EnCurso/cian, Bloqueada/naranja, Hecha/verde, Cancelada/rojo — colores de dominio ya definidos en la TUI).
  2. **Lista de cards** (una card por tarea, radio 14) con botón "⋯" por tarea que abre acciones.
  3. **Contador + badge** en la fila del peer ("3 tareas") que al click abre `Popover` con la lista.
- **Endpoint:** `POST /listar-tareas { instancia_id }` → `Vec<Tarea>`.
- **Trazabilidad:** enlaza cada tarea con sus reportes (`GET /tarea/reportes`) desde el detalle.
- **Prioridad:** **media**.

### peers-07 — Asignar tarea nueva a un peer (CRUD create)
- **Problema:** Desde Peers no se puede crear/asignar una tarea a un peer. Max debe ir a otra pantalla.
- **Propuesta:** Botón **Asignar tarea** en la barra/detalle → pop-up con descripción + estimado; crea la tarea y notifica al peer por canal.
- **Variantes de diseño Ethos:**
  1. **Dialog** con `Input` descripción (multilínea) + `Input`/`Select` de estimado (30m/1h/1d/personalizado); botón primario BRASA "Asignar".
  2. **Wizard de 2 pasos** (Popover): 1) descripción, 2) estimado + confirmación.
  3. **Quick-add** inline: campo compacto en el detalle ("+ Nueva tarea para <id>") con Enter para crear con estimado por defecto.
- **Endpoint:** `POST /tarea/asignar { instancia_id, descripcion, estimado_seg? }`.
- **Trazabilidad:** la tarea creada aparece en peers-06 y en la jornada (peers-05); el peer recibe un `<channel>`.
- **Prioridad:** **media**.

### peers-08 — Reasignar tarea entre peers (drag o selector)
- **Problema:** No hay forma de mover una tarea de un peer a otro desde la desktop.
- **Propuesta:** Acción **Reasignar** sobre una tarea del detalle → selector del peer destino; `POST /tarea/reasignar`.
- **Variantes de diseño Ethos:**
  1. **`Select` de peers** (dropdown TINTA2, opción activa BRASA) dentro de un `Dialog` "Reasignar tarea → …".
  2. **Drag & drop** de la tarea (card) desde el detalle de un peer sobre la fila de otro peer en la tabla (indicador de drop LINEA→BRASA).
  3. **Popover** con lista de peers vivos (chip de estado por peer) filtrable.
- **Endpoint:** `POST /tarea/reasignar { tarea_id, nuevo_instancia_id }`.
- **Trazabilidad:** registra dueño anterior → nuevo; notifica al nuevo dueño; visible en historial de la tarea.
- **Prioridad:** **baja**.

### peers-09 — Timeline / historial de mensajes del peer (trazabilidad)
- **Problema:** No se puede ver el historial de mensajes enviados/recibidos por un peer ni su estado (enviado→entregado→leído→procesado).
- **Propuesta:** Pop-up **Mensajes de `<id>`**: lista con `de/para`, texto, timestamp y chip de `EstadoMensaje`; opción de reenviar pendientes.
- **Variantes de diseño Ethos:**
  1. **Timeline vertical** (línea LINEA con nodos): entrantes a la izquierda, salientes a la derecha; estado como pill 999 (BRASA=procesado, HUMO=pendiente).
  2. **Tabla mono** (IBM Plex Mono para timestamps/estado) dentro del detalle.
  3. **Feed tipo chat** (burbujas TINTA2/BRASA-tenue) con marca de estado al pie de cada burbuja.
- **Endpoint:** `GET /admin/historial` (por instancia) + `POST /admin/reenviar` para reenviar pendientes.
- **Trazabilidad:** el ciclo de vida del mensaje (enviado→entregado→leído→procesado) — auditoría de comunicación.
- **Prioridad:** **media**.

### peers-10 — Refrescar manual + auto-refresh con indicador
- **Problema:** No hay control de refresco manual ni indicación de "datos de hace X". La tabla puede estar obsoleta sin avisar.
- **Propuesta:** Botón **Refrescar** (re-`POST /listar` + `/admin/alertas`) + toggle de auto-refresh con intervalo, y sello "actualizado hace Ns".
- **Variantes de diseño Ethos:**
  1. **Icono de refresco** (↻) en la cabecera junto al conteo, gira mientras carga (spinner BRASA); a la derecha, "hace 5s" en mono/HUMO.
  2. **`Switch`** "Auto" + `Select` de intervalo (2s/5s/10s) en una barra de herramientas superior.
  3. **Pull-to-refresh** (arrastrar la tabla hacia abajo) con barra de progreso BRASA.
- **Endpoint:** `POST /listar` + `GET /admin/alertas`.
- **Trazabilidad:** timestamp de última sincronización visible; no requiere endpoint nuevo.
- **Prioridad:** **alta**.

### peers-11 — Filtro / búsqueda de peers
- **Problema:** Con muchos peers no hay forma de buscar por id/directorio/repo ni filtrar por estado. Tabla plana e inmanejable.
- **Propuesta:** Barra de búsqueda (`Input`) que filtra en vivo por id/directorio/resumen + chips de filtro por estado (ocioso/atascado/trabajando).
- **Variantes de diseño Ethos:**
  1. **Search bar Ethos**: `Input` con icono lupa HUMO, placeholder "Buscar peer…", borde BRASA al foco; a la derecha chips-pill de estado toggleables.
  2. **Filtro por columnas**: mini-input bajo cada eyebrow de columna (id, directorio, resumen).
  3. **Command palette** (`Cmd+K`) que abre un `Popover` de búsqueda flotante con resultados navegables por teclado.
- **Endpoint:** ninguno (filtrado cliente sobre `instancias` ya cargadas).
- **Trazabilidad:** no aplica.
- **Prioridad:** **media**.

### peers-12 — Ordenar la tabla por columna
- **Problema:** El orden es fijo (el de `/listar`). Max no puede ordenar por "visto" (más recientes), por estado (atascados primero) ni por id.
- **Propuesta:** Encabezados clicables que ordenan asc/desc; indicador de dirección.
- **Variantes de diseño Ethos:**
  1. **Eyebrow clicable** con flecha ▲/▼ BRASA en la columna activa; el resto de eyebrows en HUMO.
  2. **`Select` "Ordenar por"** en la barra de herramientas (Visto / Estado / Id / Directorio).
  3. **Orden por severidad por defecto** (atascado→ocioso→trabajando) con toggle "orden manual".
- **Endpoint:** ninguno (orden cliente).
- **Trazabilidad:** no aplica.
- **Prioridad:** **baja**.

### peers-13 — Colas y outbox pendientes por peer (salud de mensajería)
- **Problema:** Max no ve si un peer tiene mensajes atascados en su cola/outbox. No hay señal de mensajería atascada.
- **Propuesta:** Columna/badge **cola** por peer (nº pendientes en inbox + outbox) y, en el detalle, desglose con acción "reenviar pendientes".
- **Variantes de diseño Ethos:**
  1. **Badge numérico** en la fila (pill 999): 0=oculto, >0 en ámbar; al hover, tooltip "3 en cola, 1 en outbox".
  2. **Mini-barra** de estado de mensajería en el detalle (inbox/outbox como dos barras horizontales).
  3. **Icono sobre** (✉) con contador que abre `Popover` con la lista de pendientes.
- **Endpoint:** `GET /admin/redis` (colas/outbox por instancia) + `POST /admin/reenviar`.
- **Trazabilidad:** salud de la mensajería del peer; enlaza al timeline (peers-09).
- **Prioridad:** **media**.

### peers-14 — Alertas vivas del peer (badge + panel)
- **Problema:** El estado (ocioso/atascado) se ve como chip, pero no el DETALLE de las alertas que lo causaron (cuándo se emitió, motivo).
- **Propuesta:** Badge de nº de alertas en la fila; en el detalle, panel **Alertas de `<id>`** con tipo, detalle, `creada_en` y acción resolver/descartar.
- **Variantes de diseño Ethos:**
  1. **Badge pill** junto al chip de estado (naranja para atascado, ámbar para ocioso); click abre panel de alertas.
  2. **Sección en el detalle** con lista de alertas (icono ⚠ por severidad) y botón "Descartar".
  3. **Banner contextual** en el detalle del peer si tiene alertas críticas (fondo rojo-tenue `#7F1D1D33`).
- **Endpoint:** `GET /admin/alertas` (cruce por `sujeto == id`) + `POST /admin/alerta-resolver`.
- **Trazabilidad:** cuándo se emitió/resolvió cada alerta del peer.
- **Prioridad:** **media**.

### peers-15 — Factor de estimación aprendido del peer
- **Problema:** El broker aprende un factor de estimación por peer (`/factor-estimacion-peer`), pero la desktop no lo muestra. Max no sabe qué peer sub/sobre-estima.
- **Propuesta:** Mostrar el factor y nº de muestras en el detalle del peer (p.ej. "estima ×1.4 sobre 12 tareas").
- **Variantes de diseño Ethos:**
  1. **Métrica en el detalle**: número grande Fraunces BRASA + label mono/HUMO "factor de estimación (12 muestras)".
  2. **Micro-gráfico** (sparkline) de estimado-vs-real de las últimas tareas.
  3. **Chip semántico**: verde si ~1.0, ámbar si >1.3 (sobre-estima), naranja si <0.7 (sub-estima).
- **Endpoint:** `GET /factor-estimacion-peer?instancia_id=<id>`.
- **Trazabilidad:** métrica derivada del histórico de tareas Hechas del peer.
- **Prioridad:** **baja**.

### peers-16 — Log de actividad / auditoría del peer
- **Problema:** No hay un historial consolidado de "qué le pasó a este peer" (registró, latió, se le asignó/reasignó tarea, se editó resumen, se le expulsó).
- **Propuesta:** Sección **Actividad** en el detalle: línea de tiempo de eventos del peer (registro, latidos, cambios de resumen, tareas, expulsión).
- **Variantes de diseño Ethos:**
  1. **Timeline vertical** con nodos por tipo de evento (icono + timestamp mono + texto PAPEL/HUMO).
  2. **Feed cronológico** (más reciente arriba) con eyebrows de fecha como separadores.
  3. **Tabla filtrable** por tipo de evento (registro/tarea/mensaje/resumen/kick).
- **Endpoint:** derivable de `/jornada` (sesiones=registro/latidos) + `/admin/historial` (mensajes) + log de acciones del operador (peers-17). Requiere agregación cliente; opcionalmente un endpoint nuevo de auditoría por peer.
- **Trazabilidad:** auditoría completa del ciclo de vida del peer.
- **Prioridad:** **baja**.

### peers-17 — Registro de acciones del operador (quién hizo qué desde la desktop)
- **Problema:** Cuando Max expulsa, edita resumen o reasigna, no queda constancia de la acción del operador (útil con varios operadores).
- **Propuesta:** Un log local/remoto de acciones ejecutadas desde la desktop (kick, editar resumen, asignar, reasignar, enviar) con timestamp; panel "Historial de acciones".
- **Variantes de diseño Ethos:**
  1. **Toast persistente** (`Notification` que se apila en un centro de notificaciones colapsable, esquina inferior).
  2. **Panel lateral "Registro"** togglable (drawer izquierdo) con la lista de acciones recientes.
  3. **Barra de estado inferior** que muestra la última acción ("✓ mensaje enviado a claudia · 17:31").
- **Endpoint:** ninguno obligatorio (log cliente); opcional endpoint de auditoría del broker.
- **Trazabilidad:** trazabilidad de las acciones administrativas de Max.
- **Prioridad:** **baja**.

### peers-18 — Accesibilidad: foco por teclado, tooltips y estado de salud del broker
- **Problema:** La vista no tiene navegación por teclado (flechas/Enter como la TUI), ni tooltips, ni indicación de si el broker está vivo. "No tengo ningún control" incluye la ausencia de teclado y de feedback.
- **Propuesta:** Navegación por teclado en la tabla (↑/↓ mueven selección, Enter abre detalle, `m`/`k`/`r` replican la TUI), tooltips en botones/estados, y un indicador de salud del broker en la cabecera.
- **Variantes de diseño Ethos:**
  1. **Fila con foco visible** (borde BRASA + fondo TINTA2) y atajos de teclado espejo de la TUI; `Tooltip` del kit en cada botón e icono de estado.
  2. **Barra de atajos** al pie ("↑↓ mover · ↵ detalle · m mensaje · k expulsar · r resumen") en mono/HUMO, como el footer de la TUI.
  3. **Indicador de salud** (punto verde/rojo pulsante) junto al título "Peers (N)": verde si `/salud` responde, rojo + banner si no.
- **Endpoint:** `GET /salud` (indicador) + `GET /admin/info` (host/puerto/versión en tooltip).
- **Trazabilidad:** estado de conexión con el broker en tiempo real.
- **Prioridad:** **alta** (accesibilidad y confianza operativa).

---

## Resumen de prioridades

| Prioridad | Features |
|---|---|
| **Alta** | peers-01 (detalle), peers-02 (enviar mensaje), peers-03 (kick), peers-05 (jornada), peers-10 (refrescar), peers-18 (accesibilidad/salud) |
| **Media** | peers-04 (editar resumen), peers-06 (tareas del peer), peers-07 (asignar tarea), peers-09 (timeline mensajes), peers-11 (filtro), peers-13 (colas/outbox), peers-14 (alertas del peer) |
| **Baja** | peers-08 (reasignar), peers-12 (ordenar), peers-15 (factor), peers-16 (auditoría peer), peers-17 (log operador) |

## Notas de implementación (para la fase de decisión)

- La paridad mínima con la TUI son **peers-02, peers-03, peers-04** (las teclas `m`/`k`/`r`). Empezar por ahí cierra la queja principal.
- El **pop-up de detalle (peers-01)** es el contenedor natural de peers-05/06/09/13/14/15/16 → conviene diseñarlo primero como shell con secciones colapsables.
- Todo debe **degradar** como la TUI: broker offline/401 → banner (`banner_error` ya existe), sin crash.
- Reusar helpers de `tema.rs` (`superficie_card`, `eyebrow`, `chip_estado`, `boton_primario/secundario`, `fila_seleccionable`);
  añadir `boton_peligro` (rojo-tenip) para kick y un helper `pill_badge` para contadores.
