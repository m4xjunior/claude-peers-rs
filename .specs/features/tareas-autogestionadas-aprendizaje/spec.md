# Spec — Tareas autogestionadas por los peers + aprendizaje de estimación

> Fecha: 2026-06-29. Diseño aprobado por Max (brainstorming).
> Depende de: que termine el workflow de trazabilidad (Fase 1+2) — toca los mismos crates.
> Encaja con Fase 4/5 del consejo-roadmap (estados de tarea + métricas).

## El problema (en palabras de Max)

Cuando Max escribe una feature nueva, la IA estima "1 semana, semana 2, semana 3" — pero Max
la hace en **minutos o un día**. La IA estima con datos de equipos humanos genéricos, no con la
velocidad real de Max desarrollando con IA. Hay que **matar esa alucinación de estimaciones**:
que el sistema aprenda de los tiempos reales y corrija las estimaciones futuras.

## La solución

Los peers (IAs) **fichan sus propias tareas** vía tools MCP nativas (no las inputa Max): al
empezar trabajo sustancial crean una tarea con su estimado; al terminar la cierran. El broker
mide el tiempo **real** con SU reloj (nunca la IA estima el real) y **aprende un factor de
corrección global** (`la IA infla por N`) que aplica a estimaciones futuras.

## Requisitos (trazables)

### Modelo y aprendizaje

- **R1** `Tarea` gana `estimado_seg: Option<i64>` (lo que la IA estimó al abrir), junto al
  `duracion_seg` (real, ya existe). `#[serde(default)]` para no romper tareas viejas.
- **R2** El broker mantiene un `FactorEstimacion { muestras: u32, factor: f64, actualizado_en }`
  global (clave `cprs:factor_estimacion`). `factor` = cuánto infla la IA: `real ≈ estimado / factor`.
- **R3** Al cerrar una tarea CON estimado y real válidos, el broker actualiza el factor con una
  **media móvil exponencial** (α≈0.3, pondera lo reciente) del ratio `estimado_seg / real_seg`.
  Clamp del factor a [0.5, 50.0] para que un outlier no lo enloquezca. Tarea sin estimado NO
  contamina el factor.
- **R4** El broker expone el factor: dado un `estimado_seg` ingenuo, devuelve el `corregido_seg =
  estimado_seg / factor` + nº de muestras + el factor.

### Tools MCP del peer (lo nativo — los Claudes las usan solos)

- **R5** `peers-client` expone tools: `crear_tarea(descripcion, estimado_seg)`,
  `reportar_tarea(texto)`, `cerrar_tarea()`, `listar_tareas()`, `revisar_tareas()`.
  Nombres y descripciones en español (como las 4 tools actuales).
- **R6** `crear_tarea` devuelve el estimado corregido por el factor ("dijiste 5d; según el
  historial de Max ~6h, factor 6.2x de 23 muestras") para que el Claude ajuste su plan en vivo.
- **R7** Las INSTRUCCIONES del MCP (que ya se inyectan en cada sesión, mcp.rs::instrucciones)
  ganan una línea: "Antes de trabajo sustancial, crea una tarea con tu estimado (crear_tarea);
  al terminar, ciérrala (cerrar_tarea). El broker mide tu tiempo real y aprende a corregir
  estimaciones." Así los peers lo hacen nativamente, sin que Max intervenga.

### Endpoints del broker (bajo token)

- **R8** `POST /crear-tarea {instancia_id, descripcion, estimado_seg?}` → abre tarea (reusa
  jornada::abrir_tarea que ya existe) guardando el estimado; responde el corregido (R4).
- **R9** `POST /cerrar-tarea {tarea_id}` → cierra (ya existe), mide real, actualiza factor (R3).
- **R10** `POST /listar-tareas {instancia_id}` y `GET /factor-estimacion` (para la TUI).

### TUI (mínimo en esta spec)

- **R11** La pantalla Broker (o Equipo cuando exista) muestra el factor aprendido y nº de muestras.
  La pantalla de tareas/Equipo completa queda para la Fase 4 del roadmap.

## Criterios de aceptación

- **AC1** (R1,R3): cerrar una tarea con estimado=5d y real=1h actualiza el factor hacia ~120x
  (con clamp a 50). Cerrar otra con estimado=2h/real=1h lo mueve hacia ~2x (media móvil).
- **AC2** (R4,R6): `crear_tarea(desc, estimado=1 semana)` con factor aprendido devuelve un
  corregido coherente (estimado/factor) + muestras + factor.
- **AC3** (R5): las 5 tools aparecen en `tools/list` del peers-client, en español.
- **AC4** (R3 robustez): tarea cerrada SIN estimado no cambia el factor; un ratio extremo se
  clampa (el factor nunca sale de [0.5, 50]).
- **AC5** (compat): tareas/JSON viejos sin `estimado_seg` deserializan sin error.

## Constraints

- Sin `.unwrap()`/`.expect()` en producción. Español salvo claves de protocolo. Redis + SQLite
  (impl en ambos). Las tools degradan: si el broker no responde, el Claude sigue trabajando.
- El real SIEMPRE lo timbra el broker (jamás la IA) — es la regla sagrada de la VISIÓN.
- NUNCA Co-Authored-By. Jornada en el commit.

## Fuera de alcance (esta spec)

Pantalla Equipo completa, estados de tarea (asignada/bloqueada/hecha), reasignación, métricas
por agente más allá del factor — todo eso es Fase 4/5/6 del consejo-roadmap.
