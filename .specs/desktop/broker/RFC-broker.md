# RFC-broker — CRUD, control y trazabilidad para la pestaña **Broker** (peers-desktop)

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

## Header & Metadata

| Campo | Valor |
|-------|-------|
| **Título** | Panel Broker operable en la app desktop GPUI (salud, liveness, umbrales, admin, trazabilidad) |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Impacto** | **MEDIO-ALTO** — la pestaña Broker es el "cuadro de mando" del sistema; hoy es un cartel de solo lectura |
| **Fecha** | 2026-07-02 |
| **Estado** | PROPUESTO (pendiente de decisión — RFC, aún no implementar) |
| **Pantalla objetivo** | `crates/peers-desktop/src/vista/broker.rs` |
| **Referencia CRUD** | `crates/peers-tui/src/ui/broker.rs` + teclas en `crates/peers-tui/src/main.rs` |
| **Backend** | `crates/peers-broker/src/main.rs` (rutas), `crates/peers-core/src/lib.rs` (constantes/structs) |

---

## Contexto

### Qué hace HOY la pestaña Broker desktop (`vista/broker.rs`)

Es **100% solo lectura**. Renderiza tres paneles apilados (`superficie_card`) alimentados por tres GET:

1. **Arranque** — `GET /admin/info` → `host`, `puerto`, `version`, `instancias`.
2. **Salud** — `GET /salud` → `estado` (chip brasa si `ok`, humo si no) + `instancias vivas`.
3. **Estimación** — `GET /factor-estimacion` → `factor` (ej. "6.2x") + `muestras`.

El **único** control es un botón secundario "Recargar" que despacha la acción `RecargarBroker` (refresca los tres endpoints). No hay pop-ups, ni edición, ni acciones admin, ni historial, ni trazabilidad. La TUI equivalente es aún más pobre (ni siquiera tiene botón: se refresca sola en el tick).

### Qué le FALTA a Max hoy

Max no puede, desde el panel Broker:

- Purgar la cola/outbox de un peer atascado (`POST /admin/purgar` existe y NO se usa aquí).
- Ver ni editar los **umbrales de liveness** (ocioso/atasco/ghosteo) que gobiernan las alertas — hoy solo se fijan por env/flag al arrancar el broker (`--umbral-ocioso`, etc.) y **no hay endpoint** para leerlos ni cambiarlos en caliente.
- Ver el **detalle** del factor de estimación (cuándo se actualizó, `actualizado_en` existe en el struct y no se muestra), ni el factor **por peer** (`GET /factor-estimacion-peer` existe y NO se usa aquí).
- Ver el resumen global de colas/outbox (`GET /admin/redis` existe, lo usa la pestaña Redis pero el panel Broker no ofrece un salto ni un mini-widget de "presión de colas").
- Ver **uptime**, hora del broker, backend activo (Redis vs SQLite), estado de la conexión a Redis, ni si hay token configurado — nada de esto se expone hoy.
- Copiar la `broker_url` / versión, reintentar la conexión tras un 401/offline, ni ver el **último error** de conexión.

### Endpoints REALES verificados (`main.rs`, líneas 1459-1498)

Existen y están montados en `rutas_protegidas` (token `X-Peers-Token`), salvo `/salud` que es pública:

- `GET /salud` — `{ estado, instancias }` (pública, exenta de auth).
- `GET /admin/info` — `{ host, puerto, instancias, version }`.
- `GET /admin/redis` — `{ total_instancias, colas[], outbox[] }`.
- `POST /admin/purgar` `{ id }` — borra cola + outbox de un peer. Idempotente.
- `GET /admin/historial?id=&desde=&estado=` — historial durable de una cola.
- `POST /admin/reenviar` `{ msg_id }` — re-encola un mensaje del historial.
- `GET /admin/alertas` — alertas vigentes; `POST /admin/alerta-resolver` `{ tipo, sujeto }`.
- `GET /admin/tareas` — todas las tareas de todas las instancias.
- `GET /factor-estimacion` — `{ muestras, factor, actualizado_en }`.
- `GET /factor-estimacion-peer?instancia_id=<id>` — factor de UN peer.

**Endpoints que NO existen hoy** y que varias features requieren (se marcan como *NUEVO endpoint* y son parte de la propuesta, decisión pendiente): `GET /admin/metricas` (uptime/hora/backend/redis-ok), `GET /admin/umbrales`, `POST /admin/umbrales`, `POST /admin/reiniciar-supervisor`. Los defaults de umbrales viven en `peers-core`: `UMBRAL_OCIOSO_SEG=600`, `UMBRAL_ATASCO_SEG=1800`, `UMBRAL_GHOSTEO_SEG=300`; `VENCIMIENTO_MS=45000` (ventana de liveness por latido).

---

## Design System Ethos (referencia rápida para las variantes)

Fondo tinta `#100D0A`, superficie `#1A1611`, texto papel `#ECE5D7`, acento **brasa** `#C9A96E`, terciario humo `#938B7B`, borde línea `#2B271F`. Radios: card 14 / control 10 / pill 999. Fuentes: Fraunces (títulos), Inter (UI), IBM Plex Mono (datos). Helpers existentes en `tema.rs`: `superficie_card`, `eyebrow`, `chip_estado`, `titulo`, `texto_terciario`, `boton_secundario`, `fila_seleccionable`. Componentes gpui-component: `Dialog/Modal`, `Button`, `Input`, `Select`, `Checkbox`, `Switch`, `Tooltip`, `Badge`, `Table`, `Popover`, `Notification`.

---

## Features propuestas (≥15)

> Convención de acciones GPUI: cada botón despacha una acción del namespace `broker` (patrón ya usado por `RecargarBroker`), que `AppDesktop` maneja con `.on_action(cx.listener(...))` en Fase 3.

### broker-01 — Uptime y hora del broker

- **Problema:** Max no sabe hace cuánto arrancó el broker ni con qué reloj timbra; si sospecha un reinicio silencioso, no tiene evidencia.
- **Propuesta:** en el panel "Arranque", añadir filas `arrancado_en`, `uptime` (formateado "3d 4h 12m") y `hora broker` (ISO, para detectar desfase de reloj con la máquina de Max).
- **Variantes DS:**
  - (A) Dos filas `campo()` mono más bajo host/puerto/version.
  - (B) `Badge` brasa "up 3d 4h" a la derecha del título "Arranque" (eyebrow) — glanceable.
  - (C) Pop-over al pasar sobre el chip de salud con arrancado_en + uptime + hora.
- **Endpoint:** *NUEVO* `GET /admin/metricas` → `{ arrancado_en, uptime_seg, hora_broker }` (el broker ya conoce `ahora_iso()`; guardar el instante de arranque en `EstadoApp`).
- **Trazabilidad:** N/A (estado puntual).
- **Prioridad:** media.

### broker-02 — Backend activo y estado de Redis

- **Problema:** el broker puede correr con Redis o SQLite (feature flag) y Max no ve cuál está activo ni si Redis responde; si Redis cae, hoy solo lo nota por errores en otras pestañas.
- **Propuesta:** fila "backend" (`redis` / `sqlite`) + chip "redis: ok/caído" con latencia de un `PING` en ms.
- **Variantes DS:**
  - (A) `chip_estado("redis ok", BRASA)` / `chip_estado("redis caído", HUMO)` en panel Salud.
  - (B) Fila `campo_chip("backend", ...)` + latencia mono "· 1.2 ms".
  - (C) Semáforo mini (punto 8px brasa/humo) alineado a la izquierda de la etiqueta.
- **Endpoint:** *NUEVO* `GET /admin/metricas` (mismo que broker-01) → `{ backend, redis_ok, redis_latencia_ms }`.
- **Trazabilidad:** opcional — log de caídas de Redis (ver broker-14).
- **Prioridad:** alta.

### broker-03 — Estado de conexión y reintento manual

- **Problema:** si el broker está offline o devuelve 401, la vista se queda con datos viejos sin avisar; Max no puede forzar un reintento ni ver el último error (la TUI sí levanta banner "broker offline"/"401", la desktop no).
- **Propuesta:** banner de estado sobre la cabecera: `conectado` / `offline` / `token inválido (401)`, con botón "Reintentar" y el texto del último error.
- **Variantes DS:**
  - (A) Banner full-width con borde línea y punto de estado; brasa si ok, humo si degradado.
  - (B) Chip discreto junto al título "Broker" + `Tooltip` con el último error completo.
  - (C) `Notification` (toast) al cambiar de estado (conectó / se cayó), sin banner permanente.
- **Endpoint:** reusa `GET /salud` (ping barato, exento de token) para el latido de conexión.
- **Trazabilidad:** N/A.
- **Prioridad:** alta.

### broker-04 — Ver parámetros de liveness (umbrales, solo lectura)

- **Problema:** las alertas (ocioso/atasco/ghosteo) se disparan por umbrales que Max no ve; no sabe si "ocioso" son 10 min o 30 min sin leer los flags de arranque.
- **Propuesta:** panel nuevo "Liveness" con `ocioso`, `atasco`, `ghosteo` (en formato "10m", "30m", "5m") + `ventana de latido` (`VENCIMIENTO_MS` → "45 s").
- **Variantes DS:**
  - (A) Panel `superficie_card` con 4 filas `campo()` mono.
  - (B) Tres `Badge` pill horizontales (ocioso/atasco/ghosteo) con el valor grande en brasa.
  - (C) Tabla compacta 2 columnas (parámetro → valor) con la fila de ventana de latido separada por borde.
- **Endpoint:** *NUEVO* `GET /admin/umbrales` → `{ ocioso_seg, atasco_seg, ghosteo_seg, vencimiento_ms }` (leer del struct `Umbrales` + constante core).
- **Trazabilidad:** N/A (lectura).
- **Prioridad:** alta.

### broker-05 — Editar umbrales de liveness en caliente

- **Problema:** cambiar un umbral hoy exige reiniciar el broker con otro flag/env; Max no puede afinar la sensibilidad de las alertas desde la UI.
- **Propuesta:** pop-up de edición (Dialog) con tres `Input` numéricos (o `Select` de presets: "estricto/normal/relajado"), validación en frontera (>0, atasco>ocioso), y botón "Guardar" que persiste y aplica en caliente al spawn del supervisor.
- **Variantes DS:**
  - (A) `Dialog` modal centrado, título Fraunces "Editar umbrales", 3 `Input` mono con sufijo "s", botón brasa "Guardar" + secundario "Cancelar".
  - (B) Edición inline: cada `campo` del panel Liveness se vuelve editable al pulsar un lápiz brasa (`Input` reemplaza el valor en sitio), guardado con Enter.
  - (C) `Select` de preset (estricto/normal/relajado) + "Personalizado…" que abre el Dialog de (A).
- **Endpoint:** *NUEVO* `POST /admin/umbrales` `{ ocioso_seg, atasco_seg, ghosteo_seg }` → recalcula el supervisor sin reiniciar el proceso (mover `Umbrales` a un `Arc<RwLock>` en `EstadoApp`).
- **Trazabilidad:** registrar cambio (quién/cuándo/valores antes→después) en un log de auditoría admin (ver broker-14).
- **Prioridad:** media (requiere endpoint mutable nuevo — decisión de seguridad).

### broker-06 — Purgar cola + outbox de un peer

- **Problema:** un peer con cola atascada o mensajes zombies no se puede limpiar desde la desktop; el endpoint `POST /admin/purgar` ya existe y no está cableado aquí.
- **Propuesta:** panel "Colas" (mini, alimentado por `/admin/redis`) con una fila por peer con pendientes; botón "Purgar" por fila que abre confirmación destructiva.
- **Variantes DS:**
  - (A) `fila_seleccionable` por peer con `Badge` de nº pendientes + botón "Purgar" en humo; al pulsar, `Dialog` de confirmación con texto en brasa "Esto borra N mensajes y M en outbox. Irreversible.".
  - (B) Menú contextual (`Popover`) sobre la fila con acciones Purgar / Ver historial.
  - (C) Botón global "Purgar cola…" que abre Dialog con `Select` de peer + resumen de lo que se borra antes de confirmar.
- **Endpoint:** `POST /admin/purgar` `{ id }` (existe) + `GET /admin/redis` (existe, para la lista).
- **Trazabilidad:** registrar la purga en el log admin (ver broker-14).
- **Prioridad:** alta.

### broker-07 — Presión de colas (mini-widget de salud de mensajería)

- **Problema:** Max no tiene una señal rápida en el panel Broker de si hay backlog de mensajes/outbox acumulándose (indicador temprano de peers ghosteando).
- **Propuesta:** en el panel Salud, fila "presión de colas" con total de mensajes pendientes + total outbox, y color por umbral (brasa si 0, humo/ámbar si crece).
- **Variantes DS:**
  - (A) Dos cifras grandes mono ("msgs 3 · outbox 1") con etiqueta eyebrow.
  - (B) Barra de progreso fina (sparkbar) por peer top-3 con más backlog.
  - (C) `Badge` con recuento + `Tooltip` que lista los peers con backlog.
- **Endpoint:** `GET /admin/redis` (existe — sumar `colas[].pendientes` y `outbox[].pendientes`).
- **Trazabilidad:** opcional — serie temporal simple de backlog (broker-15).
- **Prioridad:** media.

### broker-08 — Detalle del factor de estimación (drill-down)

- **Problema:** el factor "6.2x" se muestra sin contexto; `actualizado_en` existe en el struct `FactorEstimacion` y NO se pinta; Max no sabe cuán fresco es ni cómo interpretarlo.
- **Propuesta:** hacer clic en el factor abre un `Popover`/Dialog con: `factor`, `muestras`, `actualizado_en` (relativo "hace 4m"), y explicación "real ≈ estimado / factor" + los límites de clamp (FACTOR_MIN/MAX).
- **Variantes DS:**
  - (A) `Popover` anclado al número brasa "6.2x" con las 3 filas mono + nota humo.
  - (B) `Dialog` modal "Factor de estimación" con una barra visual que sitúa el factor entre MIN y MAX.
  - (C) `Tooltip` simple con `actualizado_en` + "N muestras" (mínima, sin drill-down).
- **Endpoint:** `GET /factor-estimacion` (existe — ya trae `actualizado_en`).
- **Trazabilidad:** ver broker-09 (histórico).
- **Prioridad:** media.

### broker-09 — Factor de estimación por peer (accountability individual)

- **Problema:** el factor global oculta al peer mentiroso; `GET /factor-estimacion-peer?instancia_id=` existe y no se usa en desktop. Max no puede ver quién sesga sus estimados.
- **Propuesta:** tabla "Factor por peer" (id → factor → muestras), ordenable, con resaltado de outliers (factor lejos del global).
- **Variantes DS:**
  - (A) `Table` gpui-component con columnas Peer/Factor/Muestras; celda de factor coloreada (brasa si cercano al global, humo si desviado).
  - (B) Lista `fila_seleccionable` con barra horizontal comparativa vs. el factor global (línea de referencia).
  - (C) Se pliega dentro del Dialog de broker-08 como pestaña "Por peer".
- **Endpoint:** `GET /factor-estimacion-peer?instancia_id=<id>` (existe) — iterar sobre los ids de `/admin/redis` o `/admin/info`. *Mejora opcional:* endpoint batch `GET /admin/factores` que devuelva todos de una.
- **Trazabilidad:** N/A (agregado actual).
- **Prioridad:** media.

### broker-10 — Health checks explícitos (auto-diagnóstico)

- **Problema:** no hay una vista "todo verde/algo mal" que Max pueda mirar en 2 segundos; tiene que inferir la salud de tres paneles.
- **Propuesta:** panel "Diagnóstico" con una lista de checks (broker responde, redis ok, token válido, hay instancias vivas, sin alertas críticas), cada uno con chip ok/fallo y detalle.
- **Variantes DS:**
  - (A) Lista de filas: punto de estado (8px) + nombre del check + resultado mono; falla en humo con motivo.
  - (B) `Badge` resumen "5/5 OK" arriba + expandible a la lista completa.
  - (C) Tarjetas mini en grid 2 columnas, una por check, con borde brasa/humo.
- **Endpoint:** compone `GET /salud` + `GET /admin/info` + `GET /admin/alertas` (todos existen); redis vía broker-02.
- **Trazabilidad:** ver broker-14 (log de checks fallidos).
- **Prioridad:** media.

### broker-11 — Copiar broker_url / versión / info al portapapeles

- **Problema:** Max no puede copiar rápido la URL del broker o la versión para pegarla en un issue/mensaje a un peer.
- **Propuesta:** icono "copiar" en las filas host/puerto/version y un botón "Copiar info" que copia un bloque `host:puerto @ vX.Y.Z`.
- **Variantes DS:**
  - (A) Icono copiar (ghost, humo→brasa en hover) al final de cada fila `campo()`, con `Notification` "copiado" al pulsar.
  - (B) Un solo botón secundario "Copiar info" en la cabecera junto a "Recargar".
  - (C) `Popover` "Copiar…" con opciones (URL / versión / bloque completo).
- **Endpoint:** ninguno (datos ya en `EstadoPantalla.info`).
- **Trazabilidad:** N/A.
- **Prioridad:** baja.

### broker-12 — Autorefresco configurable del panel

- **Problema:** el panel solo se refresca al pulsar "Recargar" (o el ciclo global); Max no puede fijar un autorefresco ni ver la última vez que se actualizó.
- **Propuesta:** `Switch` "auto" + `Select` de intervalo (2s/5s/15s/off) y sello "actualizado hace Ns" en la cabecera.
- **Variantes DS:**
  - (A) `Switch` brasa + texto "cada 5s" a la derecha de "Recargar".
  - (B) `Select` compacto (pill) con el intervalo; el botón "Recargar" muestra spinner mientras carga.
  - (C) Texto humo "actualizado 10:44:12" bajo el título, clic = recargar (sin switch).
- **Endpoint:** reusa los tres GET actuales en un timer del lado desktop (no toca broker).
- **Trazabilidad:** N/A.
- **Prioridad:** baja.

### broker-13 — Reiniciar el supervisor de alertas

- **Problema:** el loop que detecta ociosos/atascados/ghosteo corre en background; si Max cambió umbrales (broker-05) o sospecha alertas obsoletas, no puede forzar una re-evaluación inmediata.
- **Propuesta:** botón "Re-evaluar alertas ahora" que dispara una pasada del detector fuera del tick periódico.
- **Variantes DS:**
  - (A) Botón secundario en el panel Liveness "Re-evaluar ahora" con `Notification` del resultado ("3 alertas activas").
  - (B) Acción en `Popover` de "acciones admin" (menú) junto a purgar/umbrales.
  - (C) `Dialog` que muestra el resultado de la pasada (alertas creadas/resueltas) tras ejecutar.
- **Endpoint:** *NUEVO* `POST /admin/reiniciar-supervisor` → invoca `detectar_alertas(...)` una vez de forma manual.
- **Trazabilidad:** registra la ejecución manual (ver broker-14).
- **Prioridad:** baja.

### broker-14 — Log de auditoría de acciones admin

- **Problema:** las acciones destructivas/mutables (purgar, editar umbrales, reenviar, resolver alerta) no dejan rastro visible; Max no puede ver "quién hizo qué y cuándo" desde el panel Broker.
- **Propuesta:** panel/pestaña "Auditoría" con timeline de acciones admin (acción, sujeto, timestamp del broker, resultado ok/error).
- **Variantes DS:**
  - (A) Lista vertical tipo timeline: punto brasa + acción en Inter + sujeto mono + hora relativa; scrollable.
  - (B) `Table` con columnas Hora/Acción/Sujeto/Resultado, filtrable por tipo de acción.
  - (C) `Popover` "actividad reciente" (últimas 10) desde un icono de reloj en la cabecera; ver todo abre el panel completo.
- **Endpoint:** *NUEVO* `GET /admin/auditoria?desde=` (el broker ya loguea con `info!("admin: ...")`; persistir esos eventos en una LIST `cprs:auditoria` y exponerlos).
- **Trazabilidad:** **ES** la feature de trazabilidad del panel — historial durable de operaciones administrativas.
- **Prioridad:** media.

### broker-15 — Serie temporal de salud (mini-gráfica)

- **Problema:** Max ve valores instantáneos (instancias vivas, backlog, factor) pero no su tendencia; no distingue "3 vivos y estable" de "3 vivos y cayendo".
- **Propuesta:** sparklines de instancias vivas, backlog de colas y factor de estimación en las últimas N muestras que la desktop va acumulando en memoria.
- **Variantes DS:**
  - (A) Tres sparklines finos (línea brasa sobre superficie card) bajo cada cifra correspondiente.
  - (B) Un panel "Tendencia" con las tres series apiladas y leyenda humo.
  - (C) Solo la variación (delta ▲▼ brasa/humo) junto a cada cifra, sin gráfica.
- **Endpoint:** ninguno nuevo — la desktop muestrea `/salud` + `/admin/redis` + `/factor-estimacion` en cada refresco y guarda un buffer circular.
- **Trazabilidad:** tendencia ligera (no persistente).
- **Prioridad:** baja.

### broker-16 — Ver detalle del outbox por peer (drill-down desde Colas)

- **Problema:** el resumen de outbox da un número, pero Max no puede ver QUÉ mensajes están sin entregar en el outbox de un peer para decidir si purgar o reenviar.
- **Propuesta:** clic en la fila de un peer (panel Colas) abre un Dialog con el historial de esa cola y su estado por mensaje.
- **Variantes DS:**
  - (A) `Dialog` con `Table` (msg_id / de→para / estado / hora) + acción por fila "Reenviar".
  - (B) Timeline por mensaje (enviado→entregado→leído→procesado) tipo pop-up de trazabilidad.
  - (C) Panel lateral (drawer) que se despliega desde la derecha con la lista.
- **Endpoint:** `GET /admin/historial?id=<peer>&estado=` (existe) + `POST /admin/reenviar` `{ msg_id }` (existe).
- **Trazabilidad:** **timeline de estados** por mensaje — reusa la trazabilidad ya soportada por el broker.
- **Prioridad:** media.

### broker-17 — Reenviar un mensaje atascado desde el panel Broker

- **Problema:** cuando un peer ghostea un mensaje, reenviarlo hoy solo se hace desde la pestaña Trazabilidad de la TUI; el panel Broker no ofrece la acción aunque el endpoint existe.
- **Propuesta:** acción "Reenviar" en el drill-down de outbox (broker-16) y atajo directo desde el chip de "presión de colas" para el mensaje más antiguo sin entregar.
- **Variantes DS:**
  - (A) Botón brasa "Reenviar" por fila en el Dialog de historial + `Notification` con el nuevo `msg_id`.
  - (B) `Popover` de confirmación anclado al botón ("re-encola como mensaje NUEVO").
  - (C) Acción masiva "Reenviar todo el outbox de este peer" con confirmación.
- **Endpoint:** `POST /admin/reenviar` `{ msg_id }` (existe — re-encola como mensaje nuevo).
- **Trazabilidad:** el nuevo mensaje aparece en el historial con su propio timeline.
- **Prioridad:** media.

### broker-18 — Resolver alertas críticas desde el diagnóstico del Broker

- **Problema:** el health check (broker-10) puede señalar "hay alertas críticas" pero Max tiene que ir a la pestaña Alertas para descartarlas; falta cerrar el bucle en sitio.
- **Propuesta:** en el check de alertas, expandir la lista de alertas vigentes con acción "Resolver" inline.
- **Variantes DS:**
  - (A) Sub-lista bajo el check con `chip_estado` por tipo (ocioso/atasco/ghosteo) + botón "Resolver" humo.
  - (B) `Badge` con recuento que abre `Popover` con las alertas y su acción.
  - (C) Solo enlace "ver en Alertas" (salto de pestaña) sin resolver aquí (mínimo acoplamiento).
- **Endpoint:** `GET /admin/alertas` (existe) + `POST /admin/alerta-resolver` `{ tipo, sujeto }` (existe).
- **Trazabilidad:** la resolución manual queda en el log de auditoría (broker-14) y en el ciclo de vida de la alerta.
- **Prioridad:** baja.

### broker-19 — Indicador de seguridad (token configurado / exposición en red)

- **Problema:** el broker puede correr sin token en un host no-loopback (riesgo). `host_es_loopback` y `token_autorizado` existen en el backend pero Max no ve el estado de seguridad en la UI.
- **Propuesta:** chip de seguridad en el panel Arranque: "protegido (token)" / "sin token · localhost" / **"EXPUESTO sin token"** (aviso en humo/ámbar cuando host no es loopback y no hay token).
- **Variantes DS:**
  - (A) `chip_estado` brasa "protegido" / humo "expuesto" + `Tooltip` explicando el riesgo.
  - (B) Banner de advertencia full-width solo cuando hay exposición sin token.
  - (C) Icono candado (cerrado brasa / abierto humo) junto a host:puerto.
- **Endpoint:** *NUEVO* campo en `GET /admin/metricas` → `{ token_presente: bool, host_loopback: bool }` (el broker ya calcula ambos en arranque; el token NUNCA se expone en claro, solo el booleano).
- **Trazabilidad:** N/A.
- **Prioridad:** media.

### broker-20 — Accesibilidad y navegación por teclado del panel

- **Problema:** el panel no tiene foco visible, ni atajos, ni etiquetas para lectores de pantalla; Max (y cualquier operador) navega solo con ratón. Queja explícita: "no tengo accesibilidad".
- **Propuesta:** foco visible (borde brasa) en botones/filas, atajos (`r` recargar, `p` purgar, `u` umbrales), `Tooltip`/aria-label en iconos, y orden de tabulación coherente.
- **Variantes DS:**
  - (A) Anillo de foco brasa (2px, radio control 10) en todo elemento interactivo + hint de atajo en `Tooltip`.
  - (B) Barra de atajos discreta al pie del panel (humo, mono) listando las teclas activas.
  - (C) Modo "teclado" que resalta las teclas de acción sobre cada control (overlay temporal al pulsar `?`).
- **Endpoint:** ninguno (capa de presentación GPUI).
- **Trazabilidad:** N/A.
- **Prioridad:** media.

---

## Resumen de cobertura de endpoints

| Endpoint | Estado | Features que lo usan |
|----------|--------|----------------------|
| `GET /salud` | existe | 03, 07, 10, 15 |
| `GET /admin/info` | existe | 10, 11, 15 |
| `GET /admin/redis` | existe | 06, 07, 15, 16 |
| `POST /admin/purgar` | existe | 06 |
| `GET /admin/historial` | existe | 16 |
| `POST /admin/reenviar` | existe | 16, 17 |
| `GET /admin/alertas` | existe | 10, 18 |
| `POST /admin/alerta-resolver` | existe | 18 |
| `GET /factor-estimacion` | existe | 08, 15 |
| `GET /factor-estimacion-peer` | existe | 09 |
| `GET /admin/metricas` | **NUEVO** | 01, 02, 19 |
| `GET /admin/umbrales` | **NUEVO** | 04 |
| `POST /admin/umbrales` | **NUEVO** | 05 |
| `POST /admin/reiniciar-supervisor` | **NUEVO** | 13 |
| `GET /admin/auditoria` | **NUEVO** | 14 |

## Prioridad agregada

- **Alta:** broker-02 (redis/backend), broker-03 (conexión/reintento), broker-04 (ver umbrales), broker-06 (purgar).
- **Media:** broker-01, 05, 07, 08, 09, 10, 14, 16, 17, 19, 20.
- **Baja:** broker-11, 12, 13, 15, 18.

## Notas de decisión pendiente

1. Los endpoints mutables nuevos (`POST /admin/umbrales`, `/admin/reiniciar-supervisor`) amplían la superficie de escritura del broker → decidir con criterio de seguridad (hoy solo `/admin/purgar` es mutable). Todos irían en `rutas_protegidas` (token).
2. `GET /admin/metricas` es de bajo riesgo (solo lectura) y desbloquea 3 features de un tiro → candidato a implementar primero.
3. `GET /admin/auditoria` requiere persistir eventos que hoy solo se loguean con `info!(...)` — decidir backend (LIST `cprs:auditoria` en Redis, coherente con el patrón JSON-en-Redis del resto).
