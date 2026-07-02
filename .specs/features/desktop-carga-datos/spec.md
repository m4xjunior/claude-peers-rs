# Spec — peers-desktop: carga de datos del broker

## Contexto

La app desktop GPUI (`crates/peers-desktop`) fue creada por un workflow: UI completa y navegable
(9 pantallas + sidebar), pero **NO carga datos del broker**. Test de usuario final (mac-use)
confirmó: todas las pantallas vacías ("sin peers vivos", "sin datos de /admin/info todavía",
"No hay tareas abiertas"). Causa raíz verificada: la carga async quedó como TODO
(`app.rs:199` — *"Se guarda para que la Fase 2 dispare recargas con cx.spawn"*). El cliente HTTP
y todos sus métodos existen; falta **disparar** las cargas.

El patrón correcto YA existe y funciona en `descartar_alerta` (`app.rs:267-299`):
`cliente.clone()` → `cx.spawn(async move |esta, cx| { let r = cliente.X().await; esta.update(cx, |esta, cx| { mutar estado }) })`.

## Objetivo

Que cada pantalla muestre los datos REALES del broker: al abrir la app y al cambiar de pantalla,
se dispara la carga correspondiente; los datos aparecen; los errores se muestran sin panic.

## Requisitos (trazables)

### Carga de datos

- **R1** — Al arrancar la app, se dispara una carga inicial de la pantalla activa (Peers por
  defecto) sin intervención del usuario.
- **R2** — Al cambiar de pantalla en el sidebar, se dispara la carga de datos de esa pantalla
  (si no están ya cargados o si toca refrescar).
- **R3** — Cada pantalla carga vía el método del cliente que le corresponde:
  - **R3.1** Peers → `listar_instancias()`
  - **R3.2** Tareas → `admin_tareas()`
  - **R3.3** Alertas → `admin_alertas()`
  - **R3.4** Broker → `admin_info()` + `salud()` + `factor_estimacion()`
  - **R3.5** Redis → `admin_redis()`
  - **R3.6** Jornada → `jornada(id)` del peer enfocado (+ `listar_instancias` para el roster)
  - **R3.7** Trazabilidad → `historial(id)` del peer enfocado
  - **R3.8** Acceso → `salud()` + datos de conexión (url/token ya en estado)
  - **R3.9** Config → sin fetch (edita config local; no aplica)
- **R4** — La carga corre en `cx.spawn` (async, no bloquea el hilo de UI) y muta el estado con
  `esta.update(cx, ...)` al volver, siguiendo el patrón de `descartar_alerta`.

### Refresco

- **R5** — Refresco periódico automático (cada ~2s, configurable) de la pantalla activa, para que
  la vista refleje el estado vivo del equipo (peers que entran/salen, tareas que cambian). Espejo
  del refresco de la TUI.
- **R6** — Alternativa mínima aceptable si R5 es complejo en GPUI: una tecla/botón "refrescar"
  manual por pantalla. (R5 preferido; R6 es el fallback.)

### Errores

- **R7** — Si un fetch falla (broker caído, 401, timeout), la pantalla muestra un banner de error
  con el motivo (reusar los campos `error_*` que ya existen en `Datos`), NUNCA panic.
- **R8** — Tras un error, un refresco/reintento posterior que tenga éxito limpia el banner.

### Acciones (verificar las que la UI ya expone)

- **R9** — Verificar que las acciones ya presentes en la UI funcionan end-to-end contra el broker:
  - **R9.1** Alertas: descartar (ya implementado en `descartar_alerta` — verificar).
  - **R9.2** Tareas: botón "Asignar" y acciones de tarea (asignar/reasignar/forzar/estado).
  - **R9.3** Redis: purgar cola.
  - **R9.4** Peers: enviar mensaje / expulsar (si la UI las expone).
  - **R9.5** Config: guardar (persistir cambios de broker_url/token).
- **R10** — Toda acción que muta estado en el broker recarga la pantalla afectada al terminar
  (como hace R1/R2), para reflejar el resultado sin esperar el refresco periódico.

## Criterio de "hecho" (verificación end-to-end)

1. Con el broker vivo y peers registrados, lanzar `cargo run -p peers-desktop`.
2. Test mac-use (usuario final): Peers muestra los peers reales (aistudio, power-v2, etc.);
   Broker muestra uptime/puerto/instancias; Tareas muestra las tareas; Alertas las alertas.
3. Provocar un fallo (broker apagado) → banner de error, sin panic; reencender → se recupera.
4. Ejecutar una acción (p.ej. descartar una alerta) → se refleja en la tabla.

## Fuera de alcance (YAGNI por ahora)

- RAG/Qdrant (es otra feature — plan-inyeccion-arranque).
- Onboarding de equipo (otra feature).
- Empaquetado .app/bundle firmado (distribución, posterior).
