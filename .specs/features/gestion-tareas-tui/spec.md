# Spec — Gestión interactiva de tareas en la TUI (jefe ↔ empleados IA)

> Fecha: 2026-06-29. Diseño aprobado por Max (brainstorming).
> Convierte la pantalla Tareas (hoy solo lectura) en un panel de gestión completo: Max dirige
> a sus empleados IA — ve, edita, amplía, fuerza, asigna, y gestiona el ciclo de vida.

## El problema

Hoy la pantalla Tareas es solo lectura (estimado vs real). Max quiere GESTIONAR las tareas como
un jefe: ver el detalle, editarlas, ampliar el tiempo, forzar que la tarea llegue a la sesión
del Claude, crear/asignar tareas a un peer, gestionar su ciclo de vida y ver el progreso.

## La solución

Estados de tarea (kanban) + acciones interactivas en la pantalla Tareas de la TUI, sobre
endpoints nuevos del broker. Reusa lo existente (jornada, /enviar para forzar, tarea_guardar).

## Requisitos

### Modelo (peers-core)

- **R1** enum `EstadoTarea { Abierta, EnCurso, Bloqueada, Hecha, Cancelada }` (Serialize lowercase).
- **R2** `Tarea` gana `estado: EstadoTarea` (#[serde(default)] = Abierta para compat) y
  `bloqueo_motivo: Option<String>`. Mantiene `estimado_seg`/`duracion_seg`/`fin` existentes.
- **R3** Transiciones válidas: Abierta↔EnCurso, →Bloqueada(motivo), →Hecha, →Cancelada. Solo
  `Hecha` con estimado+real válidos alimenta el factor (Cancelada/otros NO lo contaminan).

### Endpoints del broker (bajo token)

- **R4** `POST /tarea/editar {tarea_id, descripcion?, estimado_seg?}` → edita campos (reusa tarea_guardar).
- **R5** `POST /tarea/estado {tarea_id, estado, motivo?}` → transiciona; si pasa a Hecha mide
  real y aprende factor; si Bloqueada guarda motivo. Reabrir = volver a Abierta/EnCurso.
- **R6** `POST /tarea/asignar {instancia_id, descripcion, estimado_seg?}` → crea tarea asignada a
  un peer (reusa /crear-tarea) Y le notifica por canal ("Tienes una tarea nueva: ...").
- **R7** `POST /tarea/reasignar {tarea_id, nuevo_instancia_id}` → cambia el dueño + notifica al nuevo.
- **R8** `POST /tarea/forzar {tarea_id}` → empuja la tarea como `<channel>` a la sesión del peer
  dueño (reusa /enviar con texto formateado: "⏰ Recordatorio de tu tarea: <desc>"). Es el
  "tócale el hombro" de Max.
- **R9** `GET /tarea/reportes {tarea_id}` → historial de reportes (reportar_tarea ya existe).
- **R10** Métodos del trait: `tarea_estado`, `tarea_editar`, `tarea_reportes` (impl Redis + SQLite).

### TUI — pantalla Tareas interactiva

- **R11** La tabla muestra por tarea: descripción, estado coloreado (Abierta/gris, EnCurso/cian,
  Bloqueada/naranja, Hecha/verde, Cancelada/rojo), estimado vs real, peer dueño.
- **R12** Acciones (teclado + mouse, navegando peers con [ ]):
  - `Enter` / click → modal DETALLE: descripción completa, tiempos, estado, motivo, reportes.
  - `e` → editar descripción/estimado (input modal).
  - `+` → ampliar estimado (submenú +30m/+1h/+1d, o input).
  - `f` → forzar envío a la sesión del peer (R8).
  - `n` → nueva tarea (input descripción+estimado, asignada al peer enfocado) (R6).
  - `a` → reasignar a otro peer (selector) (R7).
  - `b` → marcar Bloqueada (input motivo); `h` → Hecha; `c` → Cancelar; `R` → Reabrir (R5).
- **R13** Todo degrada: broker offline/401 → banner, sin crash. Mouse y teclado coexisten.

## Criterios de aceptación

- **AC1** (R4): editar descripción/estimado de una tarea persiste y se refleja en la tabla.
- **AC2** (R5): pasar a Hecha mide real y aprende factor; Cancelar NO toca el factor; Bloqueada guarda motivo.
- **AC3** (R8): forzar envío → el peer dueño recibe un `<channel>` con el recordatorio de la tarea.
- **AC4** (R6/R7): crear tarea asignada / reasignar → el peer destino queda como dueño y recibe notificación.
- **AC5** (R9): el detalle muestra los reportes de progreso de la tarea.
- **AC6** (R2 compat): tareas viejas sin `estado` deserializan como Abierta.
- **AC7** (R12): las acciones funcionan por teclado Y por mouse; offline no crashea.

## Constraints

Sin `.unwrap()`/`.expect()` en prod. Español salvo protocolo. Redis + SQLite (impl ambos). El
tiempo real lo mide el broker. NUNCA Co-Authored-By. Reusa /enviar, /crear-tarea, jornada,
tarea_guardar — no reinventar.

## Fuera de alcance

Dependencias entre tareas, subtareas, fechas límite, prioridades — YAGNI por ahora.
