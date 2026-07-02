# RFC — Pestaña Jornada (peers-desktop / GPUI): CRUD, controles y trazabilidad

| Campo | Valor |
|-------|-------|
| **Título** | Dotar de acciones, pop-ups y trazabilidad temporal a la pestaña Jornada de la app desktop |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Ámbito** | `crates/peers-desktop/src/vista/jornada.rs` |
| **Espejo** | `crates/peers-tui/src/ui/jornada.rs` + acciones en `crates/peers-tui/src/main.rs` |
| **Design System** | Ethos (tinta #100D0A · tinta2 #1A1611 · papel #ECE5D7 · brasa #C9A96E · humo #938B7B · línea #2B271F · Fraunces/Inter/IBM Plex Mono) |
| **Fecha** | 2026-07-02 |
| **Estado** | PROPUESTO (pendiente de decisión — RFC previa a implementar) |

---

## Contexto

### Qué hace HOY la pestaña Jornada (verificado en `vista/jornada.rs`)

Es el "fichaje" de un peer: renderiza `RespuestaJornada { sesiones, tareas }` que llega de `POST /jornada`. Pinta:

- **Card resumen:** nº de sesiones · total trabajado (suma de `duracion_seg`) · chip "● en curso" / "○ sin sesión abierta".
- **Tabla de sesiones:** inicio · fin ("(abierta)" si `fin` vacío) · duración.
- **Tabla de tareas:** descripción · estimado (IA) · real (broker) · chip de estado.

### Qué le FALTA (la queja de Max)

La vista está **explícitamente declarada como SOLO LECTURA** en su propio doc-comment (líneas 13–15): las únicas `Action` que despacha son `SeleccionarSesion` y `SeleccionarTareaJornada`, y ambas **solo mueven la selección de fila** — no llaman al cliente, no abren nada, no mutan nada en el broker. Consecuencias concretas:

1. **No se puede abrir una tarea.** Click en una fila de tarea solo la resalta; no hay modal de detalle. La TUI SÍ tiene modal detalle (`app.tarea_detalle`, `cerrar_detalle_tarea`) con acciones `e/+/f/b/h/c/R`.
2. **No hay ninguna operación CRUD.** El broker expone `/tarea/editar`, `/tarea/estado`, `/tarea/forzar`, `/tarea/reasignar`, `/tarea/reportes`, `/admin/purgar`… y la desktop **no usa ninguno** en esta pestaña.
3. **No hay selector de peer.** El texto guía dice "selecciona uno en la pantalla Peers" — obliga a cambiar de pestaña. La TUI cicla el peer con `[` / `]` sin salir (`main.rs` línea 408).
4. **No hay trazabilidad temporal navegable:** no se ven los reportes de progreso de una tarea (`GET /tarea/reportes`), ni el timeline estimado→real, ni qué sesión contiene qué tareas, ni el factor de estimación por-peer (`GET /factor-estimacion-peer`).
5. **No hay exportación ni resumen de jornada.** El total trabajado es un número muerto: no se puede copiar, exportar ni desglosar.
6. **Cero accesibilidad:** filas sin rol/foco de teclado, sin tooltips, sin atajos.

### Endpoints del broker disponibles y HOY sin usar en esta pestaña (verificados en `peers-broker/src/main.rs`)

| Endpoint | Método | Payload | Devuelve |
|----------|--------|---------|----------|
| `/jornada` | POST | `{ instancia_id }` | `RespuestaJornada { sesiones, tareas }` |
| `/tarea/editar` | POST | `{ tarea_id, descripcion?, estimado_seg? }` | `Tarea` |
| `/tarea/estado` | POST | `{ tarea_id, estado, motivo?, evidencia? }` | `Tarea` |
| `/tarea/asignar` | POST | `{ instancia_id, descripcion, estimado_seg? }` | `{ ok, tarea_id }` |
| `/tarea/reasignar` | POST | `{ tarea_id, nuevo_instancia_id }` | `Tarea` |
| `/tarea/forzar` | POST | `{ tarea_id }` | `{ ok }` |
| `/tarea/reportes` | GET | `?tarea_id=` | `Vec<String>` |
| `/tarea/reportar` | POST | `{ tarea_id, texto }` | `RespuestaOk` |
| `/listar-tareas` | POST | `{ instancia_id }` | `Vec<Tarea>` |
| `/factor-estimacion-peer` | GET | `?instancia_id=` | `FactorEstimacion { factor, muestras }` |
| `/factor-estimacion` | GET | — | `FactorEstimacion` |
| `/listar` | POST | — | lista de instancias (para el selector de peer) |
| `/admin/purgar` | POST | `{ id }` | `RespuestaOk` |

> **Nota de contrato:** el `duracion_seg`/`fin` de sesiones y tareas SIEMPRE lo timbra el broker con su reloj (regla sagrada del proyecto). Las features de esta RFC **nunca** envían tiempos calculados por la UI; solo disparan transiciones/ediciones y el broker devuelve la `Tarea`/`Sesion` ya timbrada.

---

## Restricción de arquitectura (importante para las variantes de diseño)

`render_jornada` es una función **pura** (`&EstadoPantalla -> impl IntoElement`, sin `cx`). Por eso hoy usa el patrón "despachar `Action` → `AppDesktop` la maneja con `.on_action(cx.listener(...))`". Toda feature nueva sigue ese patrón:

- Las **filas/botones** despachan una `Action` nueva (namespace `jornada`).
- `AppDesktop` maneja la `Action`: o muta estado local de UI (abrir modal, cambiar peer, filtrar) o hace `cx.spawn` para llamar al cliente HTTP y luego `cx.notify()`.
- Los **modales** se dibujan con `Dialog`/`Modal` de gpui-component montados en el render raíz de `AppDesktop` (no dentro de la vista pura), leyendo un `Option<...>` de estado (igual que `tarea_detalle` en la TUI).

---

## Features propuestas (≥15)

---

### jornada-01 — Abrir tarea (modal de detalle)

- **Problema:** click en una tarea solo la resalta. Max no puede ver el detalle completo (id, sesión, motivo de bloqueo, evidencia, issue GitHub, estimado corregido).
- **Propuesta:** doble-click o Enter sobre la fila de tarea abre un **modal de detalle** con todos los campos de `Tarea` + zona de acciones (que enlazan a jornada-04..09). Espejo del `tarea_detalle` de la TUI.
- **Variantes de diseño (Ethos):**
  - **A — Dialog modal centrado:** `Dialog` de gpui-component, superficie tinta2 radio 14, cabecera con `eyebrow("Tarea")` + `titulo(descripcion)`, cuerpo en grid de pares label(humo)/valor(mono papel), footer con botones de acción dorados. *(recomendada)*
  - **B — Panel lateral (drawer) derecho:** se desliza desde la derecha ocupando 40%, deja las tablas visibles detrás atenuadas.
  - **C — Fila expandible in-place:** al hacer click la fila se expande verticalmente mostrando el detalle bajo ella (acordeón), sin overlay.
- **Endpoint:** ninguno para abrir (usa la `Tarea` ya cargada); el botón "reportes" dentro dispara `GET /tarea/reportes`.
- **Trazabilidad:** es el contenedor de la trazabilidad de la tarea (ver jornada-10).
- **Prioridad:** **ALTA**

---

### jornada-02 — Detalle de sesión (modal)

- **Problema:** una sesión es solo 3 celdas. No se ve su `id`, ni **qué tareas cayeron dentro** de esa sesión, ni si sigue abierta con cronómetro.
- **Propuesta:** click en una fila de sesión abre modal con: id de sesión, inicio/fin absolutos, duración, y **lista de tareas cuyo `sesion_id` coincide** (correlación local, `Tarea::sesion_id` ya existe en el modelo).
- **Variantes:**
  - **A — Dialog con sub-tabla:** cabecera `eyebrow("Sesión")` + rango horario en mono brasa; debajo mini-tabla de tareas de la sesión (reusa `fila_tarea`).
  - **B — Popover anclado a la fila:** `Popover` compacto que sale de la fila con el desglose, sin robar foco.
  - **C — Split del panel:** al seleccionar sesión, la tabla de tareas de abajo **se filtra** a las de esa sesión (jornada-11 lo cubre como filtro).
- **Endpoint:** ninguno nuevo (correlación con datos de `/jornada`).
- **Trazabilidad:** timeline inicio→fin de la sesión + tareas contenidas.
- **Prioridad:** **ALTA**

---

### jornada-03 — Selector de peer inline (sin salir de la pestaña)

- **Problema:** si no hay peer enfocado, la pantalla solo dice "ve a Peers". Cambiar de peer obliga a abandonar la jornada.
- **Propuesta:** **selector de peer** en la cabecera de la card resumen. Al elegir, dispara `POST /jornada { instancia_id }` y repinta.
- **Variantes:**
  - **A — `Select` (dropdown) dorado:** en la cabecera, pill radio 999, borde línea, texto papel, chevron brasa; lista poblada con `POST /listar`.
  - **B — Flechas `[ ‹ id › ]`:** espejo literal de `[`/`]` de la TUI: dos botones-chevron a los lados del id en mono; cicla el índice.
  - **C — Buscador con `Input` + Popover:** input filtrable que muestra peers coincidentes (útil con muchos peers).
- **Endpoint:** `POST /listar` (poblar) + `POST /jornada` (recargar).
- **Prioridad:** **ALTA**

---

### jornada-04 — Cambiar estado de tarea: Hecha / Cancelada / Reabrir

- **Problema:** Max no puede marcar una tarea como hecha/cancelada ni reabrirla desde la desktop. La TUI lo hace con `h`/`c`/`R`.
- **Propuesta:** botones de transición en el modal de detalle (jornada-01) y/o en menú contextual de fila. El broker valida con `transicion_valida`; al pasar a `Hecha` aprende el factor.
- **Variantes:**
  - **A — Trío de botones en el footer del modal:** "Hecha" (relleno brasa), "Cancelar" (ghost con borde rojo semántico), "Reabrir" (ghost humo). Estados no válidos se muestran deshabilitados (tooltip explica por qué).
  - **B — Menú contextual (Popover) en la fila:** click-derecho / botón "⋯" abre menú con las transiciones válidas dinámicamente según `transicion_valida(estado_actual, X)`.
  - **C — `Select` de estado inline en la fila:** el chip de estado se vuelve editable; elegir otro estado dispara la transición.
- **Endpoint:** `POST /tarea/estado { tarea_id, estado }`.
- **Trazabilidad:** cada transición queda timbrada por el broker (fin/duración en `Hecha`).
- **Prioridad:** **ALTA**

---

### jornada-05 — Bloquear tarea con motivo

- **Problema:** no se puede marcar una tarea como `Bloqueada` ni registrar el motivo (`bloqueo_motivo`). La TUI usa `b` → input de motivo.
- **Propuesta:** acción "Bloquear" que abre un `Input` para el motivo y dispara `POST /tarea/estado { estado: bloqueada, motivo }`.
- **Variantes:**
  - **A — Mini-Dialog de confirmación con `Input`:** título "Bloquear tarea", campo motivo (placeholder "¿qué la frena?"), botón "Bloquear" ámbar. *(recomendada)*
  - **B — Popover con textarea:** anclado al botón "Bloquear" del detalle, más ligero.
  - **C — Toggle `Switch` + campo condicional:** switch "bloqueada" que revela el input del motivo al activarse.
- **Endpoint:** `POST /tarea/estado { tarea_id, estado: "bloqueada", motivo }`.
- **Trazabilidad:** el motivo se persiste y se muestra en el detalle (jornada-01).
- **Prioridad:** **MEDIA**

---

### jornada-06 — Editar descripción de tarea

- **Problema:** una descripción mal escrita/incompleta no se puede corregir desde la desktop. La TUI usa `e`.
- **Propuesta:** acción "Editar" que abre un `Input` precargado con la descripción actual y dispara `POST /tarea/editar`.
- **Variantes:**
  - **A — Edición in-place:** doble-click en la celda descripción la convierte en `Input`; Enter guarda, Esc cancela.
  - **B — Dialog de edición:** modal con `Input` multilínea, botón "Guardar" brasa.
  - **C — Campo editable dentro del modal detalle:** la descripción del detalle es directamente editable con icono lápiz brasa al hover.
- **Endpoint:** `POST /tarea/editar { tarea_id, descripcion }`.
- **Prioridad:** **MEDIA**

---

### jornada-07 — Ampliar / ajustar estimado

- **Problema:** no se puede corregir el estimado de una tarea. La TUI usa `+` (suma minutos al estimado vigente). El broker valida rango plausible.
- **Propuesta:** acción "Estimado" que abre `Input` numérico (minutos a sumar o valor absoluto) y dispara `POST /tarea/editar { estimado_seg }`.
- **Variantes:**
  - **A — `Input` numérico con stepper +/−:** en el detalle, muestra el estimado vigente y permite ±; feedback si cae fuera de rango `[ESTIMADO_MIN, ESTIMADO_MAX]`.
  - **B — Presets pill:** botones "+15m", "+30m", "+1h" (pills radio 999 brasa) que suman rápido.
  - **C — Dialog dedicado:** con campo "sumar minutos" (espejo exacto de la TUI) y preview del nuevo total.
- **Endpoint:** `POST /tarea/editar { tarea_id, estimado_seg }`.
- **Trazabilidad:** mostrar estimado original vs corregido por factor.
- **Prioridad:** **MEDIA**

---

### jornada-08 — Forzar tarea ("tócale el hombro")

- **Problema:** Max no puede empujar un recordatorio de una tarea al peer dueño desde la jornada. La TUI usa `f`.
- **Propuesta:** botón "Forzar" que dispara `POST /tarea/forzar`; devuelve `ok=false` si el peer no está vivo (mostrar aviso, no error).
- **Variantes:**
  - **A — Botón ghost con icono campana en el detalle:** al pulsar, toast/`Notification` "recordatorio enviado a {peer}" o "{peer} no está vivo".
  - **B — Acción en menú contextual de fila (jornada-04 B).**
  - **C — Botón brasa con confirmación:** micro-popover "¿Recordar a {peer}?" antes de enviar.
- **Endpoint:** `POST /tarea/forzar { tarea_id }`.
- **Prioridad:** **MEDIA**

---

### jornada-09 — Reasignar tarea a otro peer

- **Problema:** no se puede mover una tarea de un peer a otro desde la jornada. El broker tiene `/tarea/reasignar` (atómico, evita que aparezca en dos jornadas).
- **Propuesta:** acción "Reasignar" que abre un `Select` de peers destino y dispara `POST /tarea/reasignar`.
- **Variantes:**
  - **A — Dialog con `Select` de peers:** poblado por `POST /listar`, excluyendo al dueño actual; botón "Reasignar" brasa.
  - **B — Drag & drop entre jornadas:** (futuro) arrastrar la fila a un peer del selector.
  - **C — Popover con lista buscable de peers.**
- **Endpoint:** `POST /tarea/reasignar { tarea_id, nuevo_instancia_id }` + `POST /listar`.
- **Trazabilidad:** el broker logea dueño anterior → nuevo; mostrar en el detalle un "reasignada de X".
- **Prioridad:** **MEDIA**

---

### jornada-10 — Ver reportes de progreso de una tarea (timeline)

- **Problema:** los reportes de progreso (`GET /tarea/reportes`) NO se ven en la desktop. Es trazabilidad clave: "¿qué avanzó y cuándo?".
- **Propuesta:** dentro del modal detalle (jornada-01), una sección/pestaña "Reportes" que carga `GET /tarea/reportes?tarea_id=` y los lista como timeline.
- **Variantes:**
  - **A — Timeline vertical con puntos brasa:** cada reporte es un nodo (línea vertical `LINEA` + punto brasa + texto papel en mono para timestamps).
  - **B — Lista simple en card tinta2:** reportes apilados, cada uno en su fila con separador `LINEA`.
  - **C — `Tabs` dentro del modal:** pestaña "Detalle" / "Reportes" / "Timeline de estados".
- **Endpoint:** `GET /tarea/reportes?tarea_id=`.
- **Trazabilidad:** historial de notas de progreso de la tarea.
- **Prioridad:** **ALTA**

---

### jornada-11 — Filtrar tabla de tareas por sesión / estado

- **Problema:** con muchas tareas no hay forma de acotar la vista (por sesión que las contiene, o por estado).
- **Propuesta:** barra de filtros sobre la tabla de tareas: chips de estado (abierta/en curso/…) + "solo de esta sesión" cuando hay una sesión seleccionada.
- **Variantes:**
  - **A — Chips-filtro pill togglables:** fila de pills radio 999; activo = relleno brasa, inactivo = borde línea/humo.
  - **B — `Select` multi + `Checkbox`:** dropdown de estados + checkbox "solo sesión seleccionada".
  - **C — Segmented control (Tabs):** "Todas / Activas / Terminadas".
- **Endpoint:** ninguno (filtro local sobre `/jornada`).
- **Prioridad:** **MEDIA**

---

### jornada-12 — Exportar / copiar resumen de jornada

- **Problema:** el total trabajado y el desglose son números muertos: no se pueden copiar ni exportar para un parte de horas.
- **Propuesta:** botón "Exportar" que genera un resumen (texto/CSV/JSON) de sesiones+tareas del peer y lo copia al portapapeles o guarda a fichero.
- **Variantes:**
  - **A — Botón "Copiar resumen" + `Notification`:** copia un bloque markdown (id, total, tabla) al clipboard; toast de confirmación.
  - **B — Dialog "Exportar" con formato:** `Select` (Markdown / CSV / JSON) + botón "Guardar…" que abre file dialog.
  - **C — Menú `Popover` con las 3 salidas rápidas.**
- **Endpoint:** ninguno (usa datos de `/jornada` ya cargados).
- **Trazabilidad:** el export es en sí un artefacto de auditoría de la jornada.
- **Prioridad:** **MEDIA**

---

### jornada-13 — Cronómetro en vivo de la sesión abierta

- **Problema:** cuando hay sesión abierta (`fin` vacío) solo se ve "(abierta)". No hay tiempo transcurrido en vivo.
- **Propuesta:** para la sesión abierta, mostrar un **cronómetro** que cuenta desde `inicio` (calculado en cliente para display; el tiempo oficial lo timbra el broker al cerrar).
- **Variantes:**
  - **A — Chip pulsante brasa "● 1h23 en curso":** en la card resumen y en la fila de la sesión abierta; refresco por timer de GPUI cada 1s.
  - **B — Barra de progreso sutil bajo la card resumen** (si hubiera jornada objetivo).
  - **C — Contador mono grande** en la card resumen junto al total.
- **Endpoint:** ninguno (display local; verdad la sigue timbrando el broker).
- **Trazabilidad:** visibiliza la sesión viva en tiempo real.
- **Prioridad:** **BAJA**

---

### jornada-14 — Factor de estimación del peer (precisión)

- **Problema:** el `GET /factor-estimacion-peer` (accountability individual: cuánto infla/acierta este peer sus estimados) NO se muestra en la jornada, siendo el sitio natural.
- **Propuesta:** métrica en la card resumen: "precisión de estimación" con el factor del peer y nº de muestras.
- **Variantes:**
  - **A — Métrica extra en la card resumen:** label humo "factor peer" + valor mono; color brasa si confiable (muchas muestras), humo si pocas.
  - **B — Badge junto al id del peer:** pill con "×1.4 (12 muestras)" y `Tooltip` explicativo.
  - **C — Mini medidor (gauge) semántico:** verde ≈1.0, ámbar si infla mucho.
- **Endpoint:** `GET /factor-estimacion-peer?instancia_id=` (y `/factor-estimacion` para comparar con el global).
- **Trazabilidad:** métrica de fiabilidad histórica del peer.
- **Prioridad:** **MEDIA**

---

### jornada-15 — Desviación estimado vs real por tarea (visual)

- **Problema:** la tabla muestra estimado y real como dos números crudos; no se ve de un vistazo quién se pasó/quedó corto.
- **Propuesta:** columna/indicador de desviación por tarea (real vs estimado): +% se pasó, −% fue más rápido.
- **Variantes:**
  - **A — Barra horizontal comparativa:** dos micro-barras (estimado humo / real brasa) por fila; la más larga se ve al instante.
  - **B — Chip de delta semántico:** "+34%" rojo si se pasó, "−12%" verde si fue más rápido; neutro humo si sin datos.
  - **C — Sparkline/indicador en el modal detalle** en vez de en la tabla (menos ruido en la lista).
- **Endpoint:** ninguno (cálculo local sobre `estimado_seg` vs `duracion_seg`).
- **Trazabilidad:** desviación medida por tarea (base del aprendizaje del factor).
- **Prioridad:** **MEDIA**

---

### jornada-16 — Reportar progreso manual sobre una tarea (jefe)

- **Problema:** Max no puede añadir una nota de progreso a una tarea desde la desktop; solo los peers reportan.
- **Propuesta:** en el modal detalle, botón "Añadir nota" que dispara `POST /tarea/reportar { tarea_id, texto }`.
- **Variantes:**
  - **A — `Input` al pie del timeline de reportes (jornada-10):** escribir + Enter añade la nota y refresca el timeline. *(recomendada, integra con jornada-10)*
  - **B — Dialog dedicado "Nota de progreso".**
  - **C — Popover compacto anclado a botón "+".**
- **Endpoint:** `POST /tarea/reportar { tarea_id, texto }` + refresco con `GET /tarea/reportes`.
- **Trazabilidad:** enriquece el historial de progreso de la tarea.
- **Prioridad:** **BAJA**

---

### jornada-17 — Accesibilidad y navegación por teclado

- **Problema:** cero accesibilidad. Filas sin foco de teclado, sin roles, sin atajos; hoy todo es click. La TUI es 100% teclado (`j/k`, `[`/`]`, `e/+/f/b/h/c/R`, Enter).
- **Propuesta:** navegación por teclado equivalente a la TUI + tooltips en todos los controles.
- **Variantes:**
  - **A — Focus ring brasa + atajos espejo de la TUI:** flechas/`j`/`k` mueven selección, `Enter` abre detalle, `[`/`]` cambian peer, letras disparan acciones; barra de ayuda inferior con los atajos.
  - **B — Solo tooltips + tab-order:** mínimo viable, `Tooltip` en cada botón + orden de tabulación correcto.
  - **C — Paleta de comandos (Cmd+K):** buscador de acciones sobre la tarea/sesión seleccionada.
- **Endpoint:** ninguno.
- **Prioridad:** **MEDIA**

---

## Resumen de prioridades

| Prioridad | Features |
|-----------|----------|
| **ALTA** | jornada-01 (abrir tarea), jornada-02 (detalle sesión), jornada-03 (selector peer), jornada-04 (estados hecha/cancelar/reabrir), jornada-10 (reportes/timeline) |
| **MEDIA** | jornada-05 (bloquear), jornada-06 (editar), jornada-07 (estimado), jornada-08 (forzar), jornada-09 (reasignar), jornada-11 (filtros), jornada-12 (exportar), jornada-14 (factor peer), jornada-15 (desviación), jornada-17 (a11y) |
| **BAJA** | jornada-13 (cronómetro vivo), jornada-16 (reportar nota) |

## Impacto

- **Endpoints ya existentes** → ninguna feature requiere cambios en el broker (todas usan rutas ya montadas en `rutas_protegidas`). Riesgo backend ≈ 0.
- **Patrón de UI** → se respeta la vista pura + `Action` → `AppDesktop`; los modales/toasts se montan en el render raíz (como `tarea_detalle` en la TUI). Sin reescritura del contrato.
- **Paridad con TUI** → al implementar 01/04/05/06/07/08 se alcanza la paridad de CRUD que hoy solo tiene la TUI.
- **Regla sagrada del tiempo** → intacta: la UI nunca calcula tiempos oficiales; solo dispara transiciones y muestra lo que el broker timbra (excepto el cronómetro de display jornada-13, marcado como no-oficial).
