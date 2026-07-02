# RFC — Pestaña Tareas (desktop GPUI): CRUD, pop-ups y trazabilidad

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

## Header & Metadata

| Campo | Valor |
|-------|-------|
| **Título** | Operabilidad completa de la pestaña Tareas en `peers-desktop` (CRUD, detalle, filtros, trazabilidad) |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Impacto** | **ALTO** — es la pestaña donde el jefe dirige el trabajo de sus "empleados IA"; hoy es casi solo-lectura |
| **Fecha** | 2026-07-02 |
| **Estado** | PROPUESTO (pendiente de decidir alcance/orden) |
| **DS** | Ethos — tinta `#100D0A`, tinta2 `#1A1611`, papel `#ECE5D7`, brasa `#C9A96E`, humo `#938B7B`, línea `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999 |

---

## Contexto

### Qué hace HOY la pestaña desktop (`crates/peers-desktop/src/vista/tareas.rs`)

La vista `render_tareas(&EstadoPantalla)` es **pura** (sin `cx`, sin estado propio): despacha `Action`s que `AppDesktop` debería cablear con `.on_action(...)` en Fase 3. Muestra:

- Una **tabla global** de tareas de todos los peers (columnas: peer · tarea · estado · estimado · real), con chip de estado coloreado y marca `⚠` brasa para las overrun.
- **Selección de fila** por click (`SeleccionarTarea{indice}`).
- Una **barra de acciones** bajo la tabla sobre la fila seleccionada: transiciones (`En curso / Bloquear / Hecha / Cancelar / Reabrir`), `Reasignar` y `Forzar`, y un botón `Asignar` en la cabecera.

### Qué le FALTA (verificado contra el código real)

Las `Action` **existen pero se despachan con payloads vacíos o degradados** y muchas nunca abren un formulario:

1. **`AsignarTarea`** se despacha con `instancia_id`, `descripcion` y `estimado_seg` **vacíos** (cabecera, línea 225–234). No hay formulario. En la TUI son dos pasos de input (`TareaNuevaDescripcion` → `TareaNuevaEstimado`, main.rs:944–964).
2. **`ReasignarTarea`** se despacha con `nuevo_instancia_id` **vacío** (línea 457–469). No hay selector de peer. La TUI cicla el destino al siguiente peer vivo (`reasignar_tarea`).
3. **No existe ABRIR TAREA (pop-up de detalle).** La TUI tiene un modal completo (Enter) con descripción íntegra, estado, estimado/real, dueño, **motivo de bloqueo** y **lista de reportes de progreso** vía `GET /tarea/reportes?tarea_id=` (ui/tareas.rs:173–246). En desktop **no hay ningún `Dialog`/`Modal`**: la descripción se recorta a 80 y no hay forma de leerla entera ni ver reportes.
4. **No existe EDITAR descripción/estimado.** El broker expone `POST /tarea/editar { tarea_id, descripcion?, estimado_seg? }` (main.rs:747) y la TUI lo usa (`e`). La desktop no lo cablea en absoluto.
5. **No existe AMPLIAR estimado.** La TUI lo hace (`+`, suma minutos al vigente vía `tarea_editar`, main.rs:927). La desktop no.
6. **Bloquear no captura el motivo.** El broker acepta `POST /tarea/estado { tarea_id, estado, motivo? }` (main.rs:779) y la TUI abre un input de motivo (`TareaBloquear`). La desktop despacha `CambiarEstadoTarea{estado: Bloqueada}` con motivo `None` (línea 405–410).
7. **No hay filtros por estado ni toggle global/peer.** La TUI tiene `g` (global↔peer), `1`-`5`/`0` (filtro por estado), y `[ ]` (ciclar peer enfocado). La desktop está clavada en global sin filtro (ui/tareas.rs:28–35, main.rs:374–388).
8. **No hay marca de evidencia** ni prueba de trabajo, aunque el modelo la tiene (`Tarea.evidencia`, core lib.rs:375) y `POST /tarea/estado`/`/cerrar-tarea` la aceptan.
9. **No hay ninguna trazabilidad**: ni reportes, ni timeline de estados, ni issue de GitHub (`Tarea.issue_number`, core lib.rs:363), ni quién reasignó.
10. **No hay confirmación destructiva** para Cancelar, ni feedback de éxito (toast/Notification): solo el banner de error.

### Endpoints del broker disponibles (verificados en `crates/peers-broker/src/main.rs`, rutas 1468–1495)

| Método | Ruta | Uso |
|--------|------|-----|
| `GET`  | `/admin/tareas` | Lista global de todas las tareas (ya se usa) |
| `POST` | `/listar-tareas` `{ instancia_id }` | Tareas de un peer (para vista por-peer) |
| `POST` | `/tarea/asignar` `{ instancia_id, descripcion, estimado_seg? }` | Crear tarea + notificar al peer |
| `POST` | `/tarea/editar` `{ tarea_id, descripcion?, estimado_seg? }` | Parche parcial de metadatos |
| `POST` | `/tarea/estado` `{ tarea_id, estado, motivo?, evidencia? }` | Transición (valida con `transicion_valida`, timbra real en Hecha) |
| `POST` | `/tarea/reasignar` `{ tarea_id, nuevo_instancia_id }` | Cambia dueño (atómico) + notifica |
| `POST` | `/tarea/forzar` `{ tarea_id }` | "Tócale el hombro": encola recordatorio al dueño |
| `GET`  | `/tarea/reportes?tarea_id=` | Historial de reportes de progreso (solo lectura) |
| `POST` | `/cerrar-tarea` `{ tarea_id, evidencia? }` | Cierra + aprende factor |
| `GET`  | `/factor-estimacion` · `/factor-estimacion-peer` | Factor de corrección (global / por peer) |
| `POST` | `/listar` | Instancias vivas (para el selector de reasignar/asignar) |

Transiciones válidas (core `transicion_valida`, lib.rs:323): `Abierta↔EnCurso`; cualquiera→`Bloqueada`/`Hecha`/`Cancelada`; terminal/parado→`Abierta` (reabrir); mismo estado = no-op.

> **Nota de convención DS:** el color del **estado** de dominio se conserva semántico (gris/cian/naranja/verde/rojo) dentro del chip; el **brasa** se reserva para el cromo (acción primaria, fila activa, labels/eyebrow), igual que ya hace `alertas.rs`. Todas las variantes de abajo respetan esto.

---

## Features propuestas (≥ 15)

### tareas-01 — Pop-up ABRIR TAREA (detalle completo)
- **Problema:** Max no puede abrir ninguna tarea ni leer su descripción completa (se recorta a 80 chars); no ve dueño, tiempos, motivo de bloqueo ni nada.
- **Propuesta:** doble-click en la fila (o botón "Abrir") lanza un `Dialog` modal con: descripción íntegra (wrap), estado (chip), estimado vs real, dueño, `bloqueo_motivo`, evidencia e issue de GitHub si existe. Es el hub desde el que colgar editar/estado/reportes.
- **Variantes DS:**
  1. **Dialog centrado** (560×auto) sobre superficie tinta2 `#1A1611`, borde línea `#2B271F` radio 14, título Fraunces "Detalle de tarea", eyebrow humo por campo + valor papel; datos (tiempos/id) en IBM Plex Mono.
  2. **Panel lateral (drawer)** deslizante desde la derecha (~420px), la tabla queda visible detrás atenuada — permite navegar filas sin cerrar.
  3. **Card expandible inline** bajo la fila (acordeón), sin overlay, para no perder contexto de la lista.
- **Endpoint:** `GET /admin/tareas` (ya cargado) + `GET /tarea/reportes` (para tareas-02).
- **Trazabilidad:** es el contenedor de toda la traza (reportes, timeline, issue).
- **Prioridad:** **alta**.

### tareas-02 — Reportes de progreso dentro del detalle
- **Problema:** el jefe no ve el historial de reportes que la IA fue dejando en la tarea; la TUI sí (`tarea_reportes`).
- **Propuesta:** sección "Reportes de progreso (N)" dentro del pop-up de detalle, lista scrollable de notas; vacío → texto humo "(sin reportes)".
- **Variantes DS:**
  1. **Lista de bullets** con viñeta brasa `•`, texto papel, separador línea entre ítems.
  2. **Timeline vertical** con nodos (círculo brasa) y línea `#2B271F`, cada reporte como tarjeta tinta2 con hora en mono humo.
  3. **Badge contador** en la pestaña del detalle + tabla compacta (índice · texto) con `Table` del kit.
- **Endpoint:** `GET /tarea/reportes?tarea_id=`.
- **Trazabilidad:** historial de progreso (núcleo de la trazabilidad de tareas).
- **Prioridad:** **alta**.

### tareas-03 — Formulario ASIGNAR tarea nueva
- **Problema:** el botón "Asignar" despacha campos vacíos → no crea nada usable.
- **Propuesta:** `Dialog` con `Select` de peer (de `/listar`), `Input` de descripción y `Input` de estimado en minutos (opcional); botón primario "Asignar" deshabilitado hasta que haya peer+descripción.
- **Variantes DS:**
  1. **Modal de 3 campos** apilados verticalmente, labels eyebrow humo, botón brasa full-width abajo.
  2. **Popover anclado** al botón de la cabecera (compacto, para asignación rápida).
  3. **Formulario en dos pasos** (peer→descripción/estimado) reproduciendo el flujo TUI, con indicador de paso 1/2.
- **Endpoint:** `POST /tarea/asignar` + `POST /listar` para poblar el select.
- **Prioridad:** **alta**.

### tareas-04 — Selector de peer en REASIGNAR
- **Problema:** `ReasignarTarea` va con `nuevo_instancia_id` vacío; no hay forma de elegir destino.
- **Propuesta:** al pulsar "Reasignar" abre un `Select`/`Popover` con la lista de peers vivos (excluyendo el dueño actual); al confirmar despacha con el id elegido.
- **Variantes DS:**
  1. **Popover con lista** anclado al botón Reasignar, filas `fila_seleccionable` con chip de estado vivo/offline del peer.
  2. **Dialog con Select** + preview "de {dueño} → {nuevo}" en mono.
  3. **Menú contextual** (click derecho en la fila) → submenú "Reasignar a ▸" con los peers.
- **Endpoint:** `POST /tarea/reasignar` + `POST /listar`.
- **Trazabilidad:** el broker ya loguea "tarea X reasignada: viejo → nuevo"; exponer ese evento en el timeline (tareas-13).
- **Prioridad:** **alta**.

### tareas-05 — EDITAR descripción de la tarea
- **Problema:** no se puede corregir/aclarar la descripción de una tarea ya creada.
- **Propuesta:** botón "Editar" en el detalle abre `Input` (multilínea) precargado con la descripción actual; guarda con `descripcion` y `estimado_seg = None` (parche parcial).
- **Variantes DS:**
  1. **Edición inline** en el propio Dialog de detalle (el texto se vuelve editable, aparecen "Guardar/Cancelar").
  2. **Sub-modal** de edición pequeño con `Input` grande y contador de caracteres humo.
  3. **Doble-click sobre la celda descripción** en la tabla → edición rápida en la fila.
- **Endpoint:** `POST /tarea/editar { tarea_id, descripcion }`.
- **Prioridad:** **media**.

### tareas-06 — AMPLIAR estimado (sumar minutos)
- **Problema:** cuando una tarea se alarga, no hay forma de ampliar el estimado (la TUI lo hace con `+`).
- **Propuesta:** control "Ampliar estimado" que pide minutos a **sumar** al vigente; internamente `tarea_editar` con `estimado_seg = actual + extra*60`. Validar rango plausible (el broker rechaza fuera de `[ESTIMADO_MIN, ESTIMADO_MAX]`).
- **Variantes DS:**
  1. **Stepper pill** (`− 15min +`) con presets 15/30/60, valor en mono brasa.
  2. **Input numérico** con sufijo "min" y hint "estimado actual: {X}".
  3. **Slider** brasa con marcas, para tanteo rápido.
- **Endpoint:** `POST /tarea/editar { tarea_id, estimado_seg }`.
- **Trazabilidad:** registrar ampliaciones como evento en el timeline (cuánto y cuándo).
- **Prioridad:** **media**.

### tareas-07 — BLOQUEAR con motivo obligatorio
- **Problema:** desktop bloquea con `motivo=None`; se pierde la razón del bloqueo (la TUI la pide y la muestra en `bloqueo_motivo`).
- **Propuesta:** al pulsar "Bloquear" abre `Input` de motivo; despacha `tarea/estado{estado:Bloqueada, motivo}`. Mostrar el motivo en la fila (chip naranja con tooltip) y en el detalle.
- **Variantes DS:**
  1. **Dialog de confirmación** con `Input` de motivo y botón naranja "Bloquear".
  2. **Popover inline** anclado al botón Bloquear, motivo + presets ("espera dependencia", "falta info", "revisión Max").
  3. **Motivo como Tooltip** sobre el chip naranja de la fila, editable al click.
- **Endpoint:** `POST /tarea/estado { estado: Bloqueada, motivo }`.
- **Trazabilidad:** el motivo queda en `bloqueo_motivo`; incluir en el timeline con hora.
- **Prioridad:** **media**.

### tareas-08 — CONFIRMAR acciones destructivas (Cancelar / Reabrir terminal)
- **Problema:** Cancelar una tarea es un click sin red de seguridad; un mis-click la cancela.
- **Propuesta:** `Dialog` de confirmación para Cancelar (y para Reabrir una Hecha, que revierte el aprendizaje del factor). Texto claro con nombre de tarea y consecuencia.
- **Variantes DS:**
  1. **Alert dialog** con icono ⚠ rojo semántico, botón destructivo rojo + "Volver" secundario.
  2. **Confirmación tipo "hold-to-confirm"** (mantener pulsado el botón brasa 1s).
  3. **Undo toast**: cancela ya, pero muestra `Notification` 5s con "Deshacer" (reabrir).
- **Endpoint:** `POST /tarea/estado { estado: Cancelada | Abierta }`.
- **Prioridad:** **media**.

### tareas-09 — FILTRO por estado (chips)
- **Problema:** no se puede filtrar; con muchas tareas la tabla es inmanejable (la TUI filtra con `1`-`5`/`0`).
- **Propuesta:** barra de chips-pill sobre la tabla: `Todas · Abierta · En curso · Bloqueada · Hecha · Cancelada`, con contador por estado; filtra `datos.tareas` en cliente.
- **Variantes DS:**
  1. **Chips-pill** (radio 999) con estado activo relleno brasa y resto borde línea; contador entre paréntesis.
  2. **Segmented control** (barra única dividida) con el activo en tinta2 sobre fondo brasa tenue.
  3. **Dropdown `Select`** compacto "Estado: Todas ▾" para ahorrar espacio horizontal.
- **Endpoint:** ninguno (filtrado cliente sobre `/admin/tareas`).
- **Prioridad:** **media**.

### tareas-10 — TOGGLE vista Global ↔ por-Peer + selector de peer
- **Problema:** desktop solo muestra la vista global; no se puede aislar las tareas de un peer (la TUI usa `g` y `[ ]`).
- **Propuesta:** switch "Global / Por peer"; en modo por-peer, `Select` de peer que llama `/listar-tareas`. En global, mantener la columna "peer".
- **Variantes DS:**
  1. **`Switch` + `Select`**: switch brasa a la izquierda, select de peer aparece al activar "Por peer".
  2. **Tabs** ("Global" | "Por peer") estilo Ethos, la segunda revela el selector.
  3. **Breadcrumb** "Tareas › Global" clicable que abre un menú de peers.
- **Endpoint:** `POST /listar-tareas { instancia_id }` (por-peer) vs `GET /admin/tareas` (global).
- **Prioridad:** **media**.

### tareas-11 — BÚSQUEDA por texto en descripción/peer
- **Problema:** no hay forma de encontrar una tarea concreta entre muchas.
- **Propuesta:** `Input` de búsqueda que filtra por substring en `descripcion` e `instancia_id` (case-insensitive), combinable con el filtro de estado.
- **Variantes DS:**
  1. **Search box** en la cabecera con icono lupa humo y placeholder "Buscar tarea o peer…", borde línea, foco borde brasa.
  2. **Command-palette** (atajo `⌘K`) flotante que salta a la tarea al elegirla.
  3. **Filtro inline** en el encabezado de la columna descripción (icono filtro que despliega input).
- **Endpoint:** ninguno (filtrado cliente).
- **Prioridad:** **baja**.

### tareas-12 — ORDENAR columnas + fijar overrun arriba
- **Problema:** el orden es el del broker; el jefe no puede priorizar (la TUI sube las overrun/atascadas automáticamente, #14/R3).
- **Propuesta:** headers clicables para ordenar por peer/estado/estimado/real; por defecto, overrun (⚠) arriba. Indicador de sentido (▲▼).
- **Variantes DS:**
  1. **Headers clicables** con flecha brasa en la columna activa; overrun con fondo brasa 6% opacidad.
  2. **Botón "Ordenar por ▾"** (Select) separado de los headers.
  3. **Agrupación por estado** (secciones colapsables con título eyebrow y contador).
- **Endpoint:** ninguno (orden cliente; reusa `tarea_overrun`).
- **Trazabilidad:** resalta visualmente las atascadas (señal de que falta un reporte).
- **Prioridad:** **baja**.

### tareas-13 — TIMELINE de estados de la tarea (auditoría)
- **Problema:** no se ve la historia de la tarea: cuándo se abrió, quién la reasignó, cuándo se bloqueó/cerró. El broker ya loguea estos eventos (`info!` en reasignar/forzar/estado) pero no se persisten como historial consultable.
- **Propuesta:** sección "Historial" en el detalle con la línea temporal de transiciones (abierta→en curso→bloqueada→hecha), reasignaciones y forzados, cada uno con hora. **Requiere** que el broker persista un log de eventos por tarea (nuevo `GET /tarea/eventos?tarea_id=`, espejo de `/tarea/reportes`).
- **Variantes DS:**
  1. **Timeline vertical** con nodos coloreados por tipo de evento (estado=color semántico, reasignar=brasa, forzar=humo), hora en mono.
  2. **Feed cronológico** tipo log, monoespaciado, con iconos por tipo.
  3. **Mini-tabla** (hora · evento · detalle) ordenada desc.
- **Endpoint:** **nuevo** `GET /tarea/eventos?tarea_id=` (a añadir en broker, reusando patrón `tarea_reportes` y el `historial` durable ya existente para mensajes).
- **Trazabilidad:** ESTE es el corazón de la auditoría de tareas (quién/cuándo/qué).
- **Prioridad:** **media**.

### tareas-14 — MARCAR HECHA con evidencia (prueba de trabajo)
- **Problema:** al marcar Hecha no se adjunta evidencia; sin ella la tarea es "no-verificada" y solo alimenta el factor por-peer (core lib.rs:371–375), pero el jefe no lo sabe ni puede aportarla.
- **Propuesta:** al pulsar "Hecha" abre `Dialog` con `Input` de evidencia opcional (commit SHA / PR / URL / nota); despacha `tarea/estado{estado:Hecha, evidencia}`. Mostrar badge "verificada" vs "sin evidencia" en el detalle.
- **Variantes DS:**
  1. **Dialog con Input + hint** "SHA, PR o nota (opcional)"; badge verde "verificada" al guardar con evidencia.
  2. **Split button** "Hecha ▾" con opciones "Hecha con evidencia…" / "Hecha sin evidencia".
  3. **Checkbox "adjuntar evidencia"** que revela el input dentro del confirm.
- **Endpoint:** `POST /tarea/estado { estado: Hecha, evidencia }` (o `/cerrar-tarea { evidencia }`).
- **Trazabilidad:** la evidencia queda en `Tarea.evidencia` y decide si contamina el factor global.
- **Prioridad:** **media**.

### tareas-15 — Enlace a la ISSUE de GitHub espejo
- **Problema:** las tareas pueden tener `issue_number` (issue espejo en GitHub, core lib.rs:363) pero la desktop no lo muestra ni enlaza.
- **Propuesta:** en el detalle, si `issue_number.is_some()`, chip "Issue #N" clicable que abre el repo del peer dueño en el navegador (`cx.open_url`), resolviendo owner/repo desde la instancia.
- **Variantes DS:**
  1. **Chip brasa con icono** "⌥ Issue #123" que abre el navegador.
  2. **Badge estado** (abierta/cerrada) junto al número, color semántico.
  3. **Fila "GitHub" en el detalle** con el número en mono + botón "Abrir en GitHub".
- **Endpoint:** dato ya presente en `Tarea`; el owner/repo sale de la instancia (broker `repo_de_instancia`). Opcional: exponer `repo_github` en `/listar`.
- **Trazabilidad:** vincula la tarea interna con su artefacto externo auditable.
- **Prioridad:** **baja**.

### tareas-16 — FORZAR con feedback + confirmación de entrega
- **Problema:** "Forzar" despacha a ciegas; el broker devuelve `{ ok: false }` si el peer no está vivo (main.rs:947–953), pero la desktop no muestra el resultado.
- **Propuesta:** tras forzar, mostrar `Notification` (toast) "Recordatorio enviado a {peer}" o "{peer} no está vivo: no se pudo forzar". Deshabilitar "Forzar" si el peer aparece offline.
- **Variantes DS:**
  1. **Toast `Notification`** esquina inferior derecha, verde/naranja según `ok`.
  2. **Badge efímero** sobre la fila (pulso brasa 1.5s "tocado").
  3. **Estado del botón** "Forzar" → "Enviado ✓" durante 2s, luego revierte.
- **Endpoint:** `POST /tarea/forzar { tarea_id }` (leer el `ok` de la respuesta).
- **Trazabilidad:** registrar el forzado en el timeline (tareas-13).
- **Prioridad:** **baja**.

### tareas-17 — Feedback de éxito global (toasts) para toda mutación
- **Problema:** hoy solo hay banner de error; las mutaciones exitosas (asignar, editar, cambiar estado, reasignar) no dan feedback → el jefe no sabe si "prendió".
- **Propuesta:** patrón `Notification` uniforme tras cada `Action` resuelta con éxito, espejo del `flash` de la TUI (`poner_flash`). Éxito verde, error rojo, ambos autodescartables.
- **Variantes DS:**
  1. **Toasts apilables** abajo-derecha, superficie tinta2, borde según severidad, texto papel + hora mono.
  2. **Barra de status** fija abajo (una línea) que muestra el último resultado.
  3. **Inline por fila** (la fila mutada parpadea brasa 0.6s y muestra un ✓).
- **Endpoint:** transversal (todos los `/tarea/*`).
- **Prioridad:** **media**.

### tareas-18 — Panel de FACTOR de estimación (estimado vs real por peer)
- **Problema:** el sistema aprende un factor de corrección por peer/global, pero la desktop no lo muestra; el jefe no ve si un peer infla sus estimados.
- **Propuesta:** cabecera con el factor global vigente y, en el detalle de tarea (o en un badge por fila), el factor del peer dueño ("infla ×1.8, 12 muestras"). Colorear el estimado según desviación real vs estimado.
- **Variantes DS:**
  1. **Badge de factor** junto al peer ("×1.8") con tooltip de muestras; brasa si confianza alta, humo si baja.
  2. **Barra estimado/real** dual dentro del detalle (dos barras superpuestas, real desborda en rojo si overrun).
  3. **KPI en cabecera** "Factor global ×1.4 (48 muestras)" en Fraunces + mono.
- **Endpoint:** `GET /factor-estimacion` y `GET /factor-estimacion-peer?instancia_id=`.
- **Trazabilidad:** métrica de fiabilidad de estimación acumulada por peer.
- **Prioridad:** **baja**.

---

## Impacto

- **Alcance:** la vista pura `render_tareas` no cambia de filosofía (sigue despachando `Action`s), pero se amplían los payloads (hoy vacíos) y se añaden Dialogs/Popovers stateful gestionados en `AppDesktop` (donde vive `cx`). Los pop-ups y toasts requieren estado en `EstadoPantalla` (tarea seleccionada abierta, reportes cargados, filtro/vista activos, cola de toasts).
- **Backend:** casi todo ya existe. Sólo **tareas-13** exige un endpoint nuevo (`GET /tarea/eventos`); **tareas-15/18** pueden requerir exponer `repo_github`/factor en `/listar` (opcional).
- **Riesgo:** bajo — se reusa el patrón `Action` + `.on_action` ya documentado en el propio archivo y el flujo de fetch de `app.rs::cargar_peers`. La paridad con la TUI es la referencia de correctitud.
- **Orden sugerido:** primero las **alta** (01–04: detalle, reportes, asignar, reasignar) que desbloquean el "no puedo abrir ninguna tarea"; luego **media** (05–10, 13, 14, 17); por último **baja** (11, 12, 15, 16, 18).
