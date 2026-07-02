# RFC — Pestaña Alertas (peers-desktop)

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

- **Estado:** propuesta (Request For Comments — pre-implementación)
- **Ámbito:** `crates/peers-desktop/src/vista/alertas.rs` + `AppDesktop` (acciones/estado)
- **Autor:** orquestación desktop-CRUD
- **Fecha:** 2026-07-02
- **DS:** Ethos (tinta `#100D0A`, tinta2 `#1A1611`, papel `#ECE5D7`, brasa `#C9A96E`, humo `#938B7B`, línea `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999)

---

## 1. Contexto

### Qué hace HOY la pestaña Alertas (verificado en código)

`vista/alertas.rs` (puerto 1:1 de la TUI `ui/alertas.rs`) pinta, dentro de una `superficie_card` Ethos:

- **Cabecera:** eyebrow `Supervisor · R6` + título `Alertas` + conteo `{N} vigentes`.
- **Banner de error** del broker (offline / 401 / otro) si la última operación falló.
- **Tabla** `tipo | sujeto | detalle | creada` — el `tipo` va como chip coloreado por severidad de dominio (ocioso→amarillo `#EAB308`, atascado→naranja `#FF8C00`, ghosteo→rojo `#EF4444`, cierre sospechoso / cancelación excesiva→magenta `#D946EF`). Fila clicable → despacha `AbrirDetalle { indice }`.
- **Panel de detalle inline** (no modal): tipo, sujeto, creada, detalle íntegro + 2 botones (`Cerrar` / `Descartar`).
- **Única escritura:** botón `Descartar` → `POST /admin/alerta-resolver` `{ tipo, sujeto }` (idempotente).

### Qué le FALTA (frente a la TUI y al broker)

- **No porta la tecla `g` de la TUI** — "ir al sujeto": la TUI salta a la pantalla natural del sujeto (Ocioso/CancelaciónExcesiva→Peers, Atascado/CierreSospechoso→Tareas, Ghosteo→Trazabilidad). La desktop NO tiene ese salto: Max ve la alerta pero no puede actuar sobre el sujeto.
- **Sin filtros ni búsqueda:** no puede filtrar por tipo, severidad, sujeto ni rango temporal. Con muchas alertas la tabla es un muro.
- **Sin ordenación:** orden fijo del broker; no puede ordenar por gravedad ni por antigüedad.
- **Sin trazabilidad:** `Alerta` solo trae `creada_en`. No hay historial de emisión/resolución, no se ve la antigüedad ("hace 12 min"), ni el contexto del sujeto (tarea atascada, jornada del peer ocioso, timeline del mensaje ghosteado).
- **Sin acciones masivas:** no puede descartar varias, ni "descartar todas las de tipo X".
- **Sin auto-refresh visible ni control de pausa:** la TUI refresca por `refresh_ms`; en desktop no hay indicador ni toggle.
- **Detalle inline, no modal:** el detalle empuja la tabla hacia abajo; no hay overlay `Dialog` con foco y `Esc`.
- **Sin accesibilidad:** no hay navegación por teclado (↑↓/Enter/d/g de la TUI), ni roles, ni foco.
- **Sin datos derivados:** no explota `/admin/tareas`, `/jornada`, `/tarea/reportes`, `/admin/historial`, `/factor-estimacion-peer` para dar contexto a la alerta.

### Endpoints del broker disponibles (verificados en `peers-broker/src/main.rs`)

| Endpoint | Método | Uso para Alertas |
|---|---|---|
| `/admin/alertas` | GET | lista de `Alerta { tipo, sujeto, detalle, creada_en }` (fuente actual) |
| `/admin/alerta-resolver` | POST `{ tipo, sujeto }` | descartar / resolver (idempotente) — única escritura hoy |
| `/admin/tareas` | GET | todas las tareas de todas las instancias → contexto de Atascado / CierreSospechoso |
| `/admin/historial` | GET `?id&desde&estado` | timeline de mensajes → contexto de Ghosteo |
| `/admin/reenviar` | POST `{ msg_id }` | re-encolar mensaje ghosteado |
| `/jornada` | POST | sesiones+tareas de un peer → contexto de Ocioso / CancelaciónExcesiva |
| `/tarea/reportes` | GET | reportes de progreso de una tarea → contexto de Atascado |
| `/factor-estimacion-peer` | GET | factor del peer → contexto de CancelaciónExcesiva |
| `/admin/info`, `/admin/redis` | GET | estado del broker / cola de alertas |
| `/tarea/forzar`, `/tarea/reasignar`, `/enviar` | POST | acciones correctivas lanzables desde la alerta |

> **NOTA de trazabilidad (limitación real del modelo):** `Alerta` NO persiste historial de emisión/resolución; `/admin/alertas` solo devuelve las VIGENTES. Varias features de trazabilidad de esta RFC requieren un endpoint nuevo del broker (marcado como **[requiere broker]**) o una bitácora local en la desktop (marcado como **[bitácora local]**). Se documenta explícitamente para no prometer trazabilidad que el backend hoy no da.

---

## 2. Features propuestas (≥15)

Cada feature: **id · título · problema · propuesta · 2-3 variantes DS Ethos · endpoint · trazabilidad · prioridad.**

---

### alertas-01 — Pop-up modal de detalle (Dialog con foco y Esc)
- **Problema:** hoy el detalle es inline y empuja la tabla; sin foco ni cierre por teclado. Max pidió literalmente "pop-ups de visualización".
- **Propuesta:** abrir el detalle en un `Dialog`/`Modal` centrado (overlay), con backdrop tinta translúcido, cierre por `Esc`, botón `✕` y clic fuera. Contenido: chip de severidad, sujeto, creada + antigüedad relativa, detalle íntegro con scroll, y barra de acciones (Descartar / Ir al sujeto / Cerrar).
- **Variantes DS:**
  1. **Modal centrado** — card tinta2 `#1A1611`, radio 14, borde línea `#2B271F`, sombra; borde izquierdo grueso (4px) del color de severidad; título Fraunces 18px.
  2. **Drawer lateral derecho** — panel deslizante de 420px, fondo tinta2, útil para hojear alertas sin perder la tabla a la izquierda.
  3. **Popover anclado a la fila** — burbuja compacta junto a la fila clicada (para vistazo rápido, no full).
- **Endpoint:** ninguno extra (usa datos ya cargados).
- **Trazabilidad:** muestra `creada_en` + "hace Xm".
- **Prioridad:** alta.

### alertas-02 — Acción "Ir al sujeto" (portar tecla `g` de la TUI)
- **Problema:** la TUI salta a la pantalla del sujeto para actuar; la desktop no. Max ve la alerta pero queda sin salida hacia el sujeto.
- **Propuesta:** botón `Ir al sujeto` en el detalle y en el menú de fila. Mapea igual que la TUI: Ocioso/CancelaciónExcesiva→Peers, Atascado/CierreSospechoso→Tareas, Ghosteo→Trazabilidad; cambia de pestaña y preselecciona/filtra por `sujeto`.
- **Variantes DS:**
  1. **Botón dorado brasa** `Ir al sujeto →` en la barra del modal.
  2. **Chip clicable del sujeto** — el propio `sujeto` en el detalle es un enlace (subrayado brasa al hover) que navega.
  3. **Item en menú contextual** (`…` de la fila) `Ir al sujeto` + icono de flecha.
- **Endpoint:** navegación interna; al llegar puede usar `/admin/tareas`, `/jornada` o `/admin/historial` para preseleccionar.
- **Trazabilidad:** —
- **Prioridad:** alta.

### alertas-03 — Filtro por tipo de alerta (multi-select)
- **Problema:** no puede aislar "solo ghosteos" o "solo atascados".
- **Propuesta:** barra de filtros con chips-toggle por cada `TipoAlerta` (5). Activo/inactivo; el conteo de la cabecera se recalcula al filtrar.
- **Variantes DS:**
  1. **Chips pill (radio 999)** en fila bajo la cabecera; activo = fondo brasa tenue + borde brasa, inactivo = borde línea/humo.
  2. **Select múltiple** (componente `Select`) desplegable "Tipos: 3/5".
  3. **Segmented control** horizontal (Todos | Ocioso | Atascado | Ghosteo | Cierre | Cancelación).
- **Endpoint:** filtrado en cliente sobre `/admin/alertas`.
- **Trazabilidad:** —
- **Prioridad:** alta.

### alertas-04 — Filtro por severidad (agrupación cromática)
- **Problema:** los 5 tipos tienen gravedad distinta (magenta = accountability grave, amarillo = leve); no puede ver "solo lo grave".
- **Propuesta:** filtro de 3 niveles derivado del color de dominio — Grave (magenta: cierre sospechoso + cancelación), Media (naranja/rojo: atascado + ghosteo), Leve (amarillo: ocioso).
- **Variantes DS:**
  1. **3 chips-semáforo** con el color de nivel como punto + label.
  2. **Slider de umbral** "mostrar desde: [Leve … Grave]".
  3. **Switch "Solo graves"** — toggle único brasa para el caso frecuente.
- **Endpoint:** filtrado en cliente.
- **Trazabilidad:** —
- **Prioridad:** media.

### alertas-05 — Búsqueda por sujeto / texto de detalle
- **Problema:** no puede buscar la alerta de un peer o tarea concretos.
- **Propuesta:** `Input` de búsqueda que filtra por `sujeto` y `detalle` (substring, case-insensitive), con resaltado del match.
- **Variantes DS:**
  1. **Input con icono lupa** en la cabecera, fondo tinta2, borde línea, foco = borde brasa; placeholder humo "Buscar sujeto o detalle…".
  2. **Command palette** (`Cmd+K`) que abre búsqueda flotante sobre todas las alertas.
  3. **Barra de filtro inline** que aparece al pulsar `/` (estilo TUI).
- **Endpoint:** filtrado en cliente.
- **Trazabilidad:** —
- **Prioridad:** media.

### alertas-06 — Ordenación de la tabla (gravedad / antigüedad / tipo / sujeto)
- **Problema:** orden fijo del broker; no puede poner "lo más grave primero" ni "lo más antiguo primero".
- **Propuesta:** encabezados de columna clicables (asc/desc) + un selector "Ordenar por: [Gravedad ▾]". Default sugerido: gravedad desc, luego antigüedad desc.
- **Variantes DS:**
  1. **Encabezados clicables** con flecha ▲▼ brasa en la columna activa; eyebrow del header en brasa cuando ordena.
  2. **Select "Ordenar por"** en la barra de filtros (Gravedad / Más antigua / Más reciente / Tipo / Sujeto).
  3. **Botón toggle "Graves primero"** — orden fijo de un clic para el caso común.
- **Endpoint:** orden en cliente sobre `/admin/alertas`.
- **Trazabilidad:** —
- **Prioridad:** media.

### alertas-07 — Antigüedad relativa por fila ("hace 12 min")
- **Problema:** la columna `creada` muestra `HH:MM:SS` crudo; Max no ve de un vistazo cuánto lleva viva la alerta (clave para priorizar).
- **Propuesta:** columna/badge de antigüedad calculada desde `creada_en` vs ahora ("hace 3m", "hace 2h"), con escala de color (fresco humo → viejo brasa/rojo). Tooltip con el ISO exacto.
- **Variantes DS:**
  1. **Badge pill** al final de la fila; color por rango (verde-oliva reciente, brasa >30m, rojo >2h).
  2. **Texto mono humo** junto a `creada` ("14:03:12 · hace 12m").
  3. **Barra de calor** de 3px bajo la fila cuya longitud crece con la antigüedad.
- **Endpoint:** cálculo local con `creada_en`.
- **Trazabilidad:** sí — visualiza cuándo se emitió y cuánto lleva vigente.
- **Prioridad:** alta.

### alertas-08 — Contadores por tipo en la cabecera (resumen ejecutivo)
- **Problema:** solo hay "{N} vigentes" total; no se ve la distribución por gravedad.
- **Propuesta:** fila de "stat chips" bajo el título: `Ocioso 2 · Atascado 1 · Ghosteo 0 · Cierre 3 · Cancelación 1`, cada uno con su color de dominio; clic en un chip = filtra por ese tipo (une con alertas-03).
- **Variantes DS:**
  1. **Chips-contador** (número grande mono + label eyebrow + punto de color).
  2. **Mini-donut** Ethos con leyenda a la derecha (proporción por tipo).
  3. **Barra apilada** horizontal segmentada por color de tipo con conteos en tooltip.
- **Endpoint:** derivado de `/admin/alertas`.
- **Trazabilidad:** —
- **Prioridad:** media.

### alertas-09 — Descarte con confirmación y feedback (Notification)
- **Problema:** `Descartar` actúa sin confirmar ni avisar del resultado; si el broker devuelve error solo hay banner mudo.
- **Propuesta:** confirmación ligera + `Notification` toast de éxito/fallo ("Alerta '{sujeto}' descartada" / "No se pudo descartar: {error}"), y refresco optimista de la lista.
- **Variantes DS:**
  1. **Botón con confirmación en 2 pasos** — `Descartar` → `¿Seguro? [Sí, descartar]` (rojo tenue) sin salir del modal.
  2. **Toast Notification** esquina inferior derecha, tinta2 + borde brasa (éxito) / rojo (fallo), auto-dismiss 4s.
  3. **Snackbar con Deshacer** — "Descartada · Deshacer" (aunque el resolver es idempotente, "Deshacer" re-lanzaría el detector; documentar límite).
- **Endpoint:** `POST /admin/alerta-resolver { tipo, sujeto }`.
- **Trazabilidad:** sí — registra en bitácora local (alertas-15) quién y cuándo descartó.
- **Prioridad:** alta.

### alertas-10 — Descartar en la propia fila (acción rápida sin abrir detalle)
- **Problema:** para descartar hay que abrir el detalle; en TUI basta `d` sobre la fila.
- **Propuesta:** acción de descarte accesible desde la fila (hover) sin abrir el modal, replicando la `d` de la TUI.
- **Variantes DS:**
  1. **Botón fantasma que aparece al hover** al final de la fila (icono ✓/✕ brasa), tooltip "Descartar".
  2. **Menú contextual `…`** por fila con `Descartar` + `Ir al sujeto` + `Ver detalle`.
  3. **Swipe/tecla `d`** con la fila seleccionada (accesibilidad por teclado, une con alertas-14).
- **Endpoint:** `POST /admin/alerta-resolver`.
- **Trazabilidad:** bitácora local (alertas-15).
- **Prioridad:** alta.

### alertas-11 — Descarte masivo (selección múltiple / por tipo)
- **Problema:** tras resolver una incidencia real quedan varias alertas del mismo peer/tipo; hay que descartarlas una a una.
- **Propuesta:** checkboxes de selección por fila + barra de acción "Descartar {n} seleccionadas"; y atajo "Descartar todas las de tipo X" desde el chip-contador (alertas-08).
- **Variantes DS:**
  1. **Checkbox por fila** (componente `Checkbox` Ethos) + barra flotante inferior "N seleccionadas · Descartar".
  2. **Botón "Descartar todo el filtro"** que actúa sobre el resultado filtrado actual (con confirmación del total).
  3. **Modo selección** activable con un `Switch` "Selección múltiple" que muestra los checkboxes.
- **Endpoint:** N× `POST /admin/alerta-resolver` (secuencial, con progreso).
- **Trazabilidad:** bitácora local del lote.
- **Prioridad:** media.

### alertas-12 — Contexto embebido del sujeto en el detalle (según tipo)
- **Problema:** el detalle solo tiene el texto del supervisor; para decidir, Max debe cambiar de pestaña y reconstruir el contexto a mano.
- **Propuesta:** el modal de detalle carga contexto según el `tipo` y lo muestra en una sub-sección:
  - **Atascado / CierreSospechoso** → tarea + sus reportes de progreso (`/tarea/reportes`) y estado desde `/admin/tareas`.
  - **Ghosteo** → timeline del mensaje (`/admin/historial?id=…`) con estados enviado→entregado→leído→procesado.
  - **Ocioso** → jornada del peer (`/jornada`): última tarea, tiempo ocioso.
  - **CancelaciónExcesiva** → ratio canceladas/total y factor (`/factor-estimacion-peer`, `/jornada`).
- **Variantes DS:**
  1. **Sección "Contexto" plegable** dentro del modal, con mini-tabla mono.
  2. **Pestañas internas** (Detalle | Contexto | Historial) dentro del modal (`Tabs` Ethos).
  3. **Tarjeta lateral** en el drawer (alertas-01 v2) con el contexto siempre visible.
- **Endpoint:** `/tarea/reportes`, `/admin/tareas`, `/admin/historial`, `/jornada`, `/factor-estimacion-peer` según tipo.
- **Trazabilidad:** sí — reportes y timeline del sujeto.
- **Prioridad:** media.

### alertas-13 — Acciones correctivas directas desde la alerta
- **Problema:** identificada la causa, Max no puede actuar sin salir a otra pestaña y buscar el sujeto (aunque exista alertas-02, faltan acciones one-click).
- **Propuesta:** botones contextuales en el modal según tipo:
  - **Atascado** → `Forzar cierre` (`/tarea/forzar`) o `Reasignar` (`/tarea/reasignar`).
  - **Ghosteo** → `Reenviar mensaje` (`/admin/reenviar { msg_id }`).
  - **Ocioso** → `Enviar mensaje al peer` (`/enviar`) o `Asignar tarea`.
- **Variantes DS:**
  1. **Barra de acciones adaptativa** en el pie del modal, botones brasa (primaria) + secundarios línea.
  2. **Menú "Acciones ▾"** que despliega las opciones válidas para ese tipo.
  3. **Botones inline en la sección Contexto** (junto al dato que corrigen, p.ej. "Reenviar" junto al mensaje ghosteado).
- **Endpoint:** `/tarea/forzar`, `/tarea/reasignar`, `/admin/reenviar`, `/enviar`.
- **Trazabilidad:** bitácora local de la acción correctiva.
- **Prioridad:** media.

### alertas-14 — Navegación completa por teclado + accesibilidad
- **Problema:** cero accesibilidad; la TUI navega con ↑↓/Enter/d/g y la desktop no porta nada.
- **Propuesta:** teclado equivalente a la TUI: `↑↓` mueven selección de fila, `Enter` abre detalle, `d` descarta, `g` va al sujeto, `Esc` cierra modal, `Tab` navega controles; foco visible (anillo brasa) y roles ARIA-equivalentes en GPUI.
- **Variantes DS:**
  1. **Anillo de foco brasa** 2px + fila seleccionada con `fila_seleccionable` (ya existe) resaltada.
  2. **Overlay de atajos** (`?`) mostrando el mapa de teclas Ethos (mono, tinta2).
  3. **Barra de estado inferior** replicando el título de la TUI (`Enter detalle · d descartar · g ir al sujeto`).
- **Endpoint:** —
- **Trazabilidad:** —
- **Prioridad:** alta.

### alertas-15 — Bitácora local de acciones sobre alertas (auditoría desktop)
- **Problema:** no hay rastro de qué alertas descartó Max, cuándo ni por qué; el broker no persiste resolución.
- **Propuesta:** **[bitácora local]** registrar en la desktop cada descarte/acción (timestamp, tipo, sujeto, acción, resultado) y exponer un panel "Historial de acciones" filtrable, más un contador "descartadas hoy".
- **Variantes DS:**
  1. **Panel drawer "Actividad"** con lista mono cronológica (chip de acción + sujeto + hora).
  2. **Timeline vertical** con hitos (icono por acción, color por resultado éxito/fallo).
  3. **Tabla exportable** (CSV/portapapeles) para llevar el registro fuera.
- **Endpoint:** local; opcionalmente **[requiere broker]** un `/admin/alertas-historial` que persista emisión/resolución.
- **Trazabilidad:** sí — es el núcleo de trazabilidad de la pestaña.
- **Prioridad:** media.

### alertas-16 — Trazabilidad de emisión/resolución de la alerta
- **Problema:** `Alerta` solo trae `creada_en`; no se sabe si una alerta se resolvió sola vs a mano, ni cuántas veces se ha reemitido el mismo `(tipo+sujeto)`.
- **Propuesta:** **[requiere broker]** enriquecer el modelo con `resuelta_en`, `resuelta_por` (auto/manual) y `reemisiones`; mostrar en el detalle un mini-timeline "emitida → (reemitida x2) → resuelta".
- **Variantes DS:**
  1. **Timeline horizontal** de hitos (puntos conectados por línea, color por evento).
  2. **Lista de eventos** mono en la sección Contexto del modal.
  3. **Badge "reincidente xN"** en la fila cuando el `(tipo+sujeto)` ya se resolvió antes y volvió.
- **Endpoint:** **[requiere broker]** extensión de `/admin/alertas` o nuevo `/admin/alertas-historial`.
- **Trazabilidad:** sí — historial completo de vida de la alerta.
- **Prioridad:** baja (depende de trabajo en el broker).

### alertas-17 — Auto-refresh con control de pausa e indicador
- **Problema:** la TUI refresca por `refresh_ms`; en desktop no hay indicador de "vivo" ni forma de pausar mientras Max lee un detalle.
- **Propuesta:** refresco periódico de `/admin/alertas` con indicador "actualizado hace Xs", botón `Refrescar ahora` y `Switch` de pausa (para no perder el foco del modal abierto).
- **Variantes DS:**
  1. **Pill de estado** en la cabecera (punto brasa latiendo = live; humo = pausado) + "hace 8s".
  2. **Botón circular de refresco** con spinner brasa durante la carga.
  3. **Switch "Auto"** + selector de intervalo (5s/15s/30s) en la barra de filtros.
- **Endpoint:** `GET /admin/alertas` (+ `/admin/redis` para tamaño de cola).
- **Trazabilidad:** —
- **Prioridad:** baja.

### alertas-18 — Estado vacío accionable y salud del supervisor
- **Problema:** el estado vacío actual solo dice "Sin alertas vigentes"; no confirma que el supervisor esté vivo ni cuándo evaluó por última vez.
- **Propuesta:** estado vacío con confirmación de salud del supervisor (`/admin/info` / `/admin/redis`), última evaluación y umbrales activos (ocioso/atasco/ghosteo).
- **Variantes DS:**
  1. **Card centrada** tinta2 con check brasa "Todo en orden" + línea humo "Supervisor activo · última evaluación hace 12s".
  2. **Panel de umbrales** mono ("ocioso >10m · atasco >30m · ghosteo >…").
  3. **Ilustración mínima** (glifo) + CTA `Ver estado del broker`.
- **Endpoint:** `GET /admin/info`, `GET /admin/redis`.
- **Trazabilidad:** muestra cuándo evaluó el supervisor por última vez.
- **Prioridad:** baja.

---

## 3. Impacto

- **UX:** convierte una pestaña read-only en un centro de mando: ver (modal), triar (filtros/orden/severidad/contadores), actuar (ir al sujeto, descartar rápido/masivo, correctivas) y auditar (bitácora/trazabilidad).
- **Backend:** la mayoría reutiliza endpoints existentes (`/admin/alertas`, `/admin/alerta-resolver`, `/admin/tareas`, `/admin/historial`, `/jornada`, `/tarea/reportes`, `/tarea/forzar`, `/tarea/reasignar`, `/admin/reenviar`). Solo alertas-15 (opcional) y alertas-16 requieren trabajo en el broker (persistir historial de alertas).
- **DS:** todo se construye con tokens Ethos y componentes ya disponibles (`Dialog`, `Button`, `Input`, `Select`, `Checkbox`, `Switch`, `Tooltip`, `Badge`, `Notification`, `Tabs`, `Popover`) + helpers de `tema.rs` (`superficie_card`, `eyebrow`, `chip_estado`, `fila_seleccionable`, botones). Los colores de severidad de dominio se conservan.
- **Riesgo:** bajo para las de cliente; medio para las correctivas (escriben estado real vía broker) → exigen confirmación y feedback (alertas-09).

## 4. Priorización sugerida

- **Alta (portar paridad TUI + lo que Max no puede hacer hoy):** alertas-01, 02, 03, 07, 09, 10, 14.
- **Media:** alertas-04, 05, 06, 08, 11, 12, 13, 15.
- **Baja (dependen de broker o son refinamiento):** alertas-16, 17, 18.
