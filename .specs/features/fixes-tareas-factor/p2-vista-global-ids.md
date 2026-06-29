# Spec — P2: vista global de tareas (#14) + IDs sin colisión + retención (#15)

> Fecha: 2026-06-29. Continuación de los 15 edge cases. Cierra los 2 P2-medio restantes.

## #14 — Vista global de todas las tareas (cuadro de mando del jefe)

Hoy la pantalla Tareas muestra UN peer (el enfocado con `[`/`]`). Max (jefe) quiere ver TODAS
las tareas de TODOS los peers en una sola vista, con filtro y orden.

- **R1** `GET /admin/tareas` → todas las tareas de todas las instancias (Vec<Tarea> con instancia_id),
  ordenadas por inicio desc. Reusa listar_ids + listar tareas por peer en el store (un método nuevo
  del trait `tareas_todas() -> Vec<Tarea>` impl Redis+SQLite).
- **R2** TUI pantalla Tareas: tecla `g` alterna entre "vista peer" (actual, `[`/`]`) y "vista global"
  (todas, con columna PEER). En vista global, `[`/`]` no aplica.
- **R3** Filtro por estado: tecla `1-5` filtra (Abierta/EnCurso/Bloqueada/Hecha/Cancelada), `0` = todas.
  Orden: atascadas/overrun primero (las que superaron su estimado estando abiertas).
- **R4** Las acciones de gestión (e/f/h/c/a…) siguen operando sobre la fila seleccionada en ambas vistas
  (la tarea sabe su instancia_id).

## #15 — IDs de tarea sin colisión + retención

- **R5** Generar tarea_id con INCR atómico `cprs:tareaseq` (como `cprs:msgseq`): `tar-{seq}` global
  único, en vez de `tar-{inst}-{Rfc3339}`. SQLite: AUTOINCREMENT o un contador en tabla. Dos
  crear_tarea simultáneas del mismo peer NUNCA colisionan.
- **R6** Retención: `podar_tareas(retener)` por instancia (como `podar_historial`), const
  RETENCION_TAREAS=500. Llamado en el loop de mantenimiento del broker. Impl Redis+SQLite.

## Criterios de aceptación

- **AC1** (#14): `GET /admin/tareas` devuelve tareas de >1 peer con su instancia_id; la TUI las pinta
  con columna peer en vista global (`g`), y filtra por estado (`1-5`/`0`).
- **AC2** (#14): las acciones de gestión funcionan en vista global sobre la fila seleccionada.
- **AC3** (#15): dos crear_tarea del mismo peer "a la vez" producen IDs distintos (INCR atómico).
- **AC4** (#15): podar_tareas conserva las RETENCION_TAREAS más recientes por peer; las viejas se purgan.
- **AC5**: build Redis+sqlite OK, sin .unwrap en prod, IDs viejos (tar-inst-fecha) siguen leyéndose (compat).

## Constraints

Sin `.unwrap()`/`.expect()` en prod. Español salvo protocolo. Redis + SQLite (ambos). Reusa msgseq/
podar_historial como patrón. NO romper los 81 tests ni los endpoints /tarea/* existentes. NUNCA Co-Authored-By.
