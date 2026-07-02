# RFC — Trazabilidad (desktop GPUI): mensajería operable, no solo lectura

> Estado: **propuesta** (RFC pre-decisión). Fecha: 2026-07-02.
> Ámbito: `crates/peers-desktop` — pestaña **Trazabilidad** (`src/vista/trazabilidad.rs`).
> Referencia de paridad: TUI `crates/peers-tui/src/ui/trazabilidad.rs` + teclas en `peers-tui/src/main.rs`.
> Endpoints: `crates/peers-broker/src/main.rs` (rutas admin). Cliente: `crates/peers-desktop/src/cliente.rs`.
> Design System: **Ethos** — tinta `#100D0A`, tinta2 `#1A1611`, papel `#ECE5D7`, brasa `#C9A96E`,
> humo `#938B7B`, línea `#2B271F`. Fraunces (títulos) / Inter (UI) / IBM Plex Mono (datos).
> Radios: card 14 / control 10 / pill 999. Helpers en `src/tema.rs`.

---

## 1. Contexto

### Qué hace HOY la pestaña Trazabilidad del desktop

La vista es **pura** (`render_trazabilidad(&EstadoPantalla)`), sin `cx` ni estado propio;
despacha 3 acciones que `AppDesktop` maneja. Concretamente ofrece:

- **Selector de peer en foco** (pills con los peers vivos) → `EnfocarPeer { id }` recarga el
  historial con `ClienteBroker::historial(&id)`.
- **Tabla cronológica** de mensajes: columnas `id · de · texto · estado · enviado`. La columna
  `estado` va coloreada por la máquina de estados. El texto se **recorta a 80 chars** en la fila.
- **Fila seleccionable** (click/Enter) → `SeleccionarMensaje { indice }`, que abre un **timeline
  inline** debajo de la tabla.
- **Panel timeline** del mensaje seleccionado: cabecera `#id`, ruta `de → para`, cuerpo completo,
  hitos (`○ enviado / ◑ entregado / ◑ leído / ● procesado`) con timestamp o `—`, estado actual,
  contadores `intentos / reenvíos`, traza `reenvío del mensaje #N`, y un botón **Reenviar** →
  `ReenviarMensaje { msg_id }` (`POST /admin/reenviar`).

### Qué le FALTA (lo que Max no puede hacer hoy)

1. **No puede filtrar** por estado ni por par (de/para): la tabla muestra TODO el historial del peer
   en foco, sin criba. El broker **ya soporta** `GET /admin/historial?id=&desde=&estado=` con filtro
   por `estado` y cursor `desde`, pero el cliente desktop llama `historial(id)` **sin** esos params
   (`cliente.rs:269`) → se desperdicia una capacidad ya implementada del backend.
2. **No puede buscar texto** en los mensajes (ni en el remitente/destino).
3. **No puede ver el mensaje completo desde la fila**: se recorta a 80 chars; solo se ve entero si
   abre el timeline (que además ocupa espacio vertical de la propia tabla).
4. **No puede enviar** un mensaje nuevo a un peer desde aquí (el broker tiene `POST /enviar`, pero el
   cliente desktop **no expone** `enviar` — solo `reenviar`).
5. **No puede confirmar / forzar estados** manualmente (el broker tiene `POST /confirmar {ids,estado}`
   para timbrar `Entregado/Leído/Procesado/Fallido`, sin uso en desktop).
6. **No puede purgar** la cola del peer desde aquí (existe `POST /admin/purgar {id}` — usado en Redis,
   no en Trazabilidad).
7. **No tiene paginación / carga incremental**: para colas largas no hay cursor `desde` visible
   (aunque el broker lo soporta), ni scroll dedicado a la tabla.
8. **No hay accesibilidad**: sin tooltips, sin `aria`/rol declarado, sin atajos de teclado visibles,
   el foco de fila no tiene ring propio (el código lo comenta explícitamente).
9. **Trazabilidad pobre en el timeline**: muestra los 4 hitos "felices" pero **no** los terminales
   `✕ fallido` / `✕ dead-letter` como hito con su timestamp, ni el nº de `intentos` desglosado por
   intento, ni la **cadena de reenvíos** navegable (`reenviado_de` es solo texto, no un enlace).

Objetivo del RFC: proponer (sin implementar aún) el conjunto de features que convierten esta
pestaña en una **consola de mensajería operable con trazabilidad rica**, respetando el DS Ethos y
reutilizando los endpoints que YA existen en el broker.

### Endpoints del broker verificados (los que esta pestaña puede/debe usar)

| Método | Ruta | Uso en Trazabilidad |
|---|---|---|
| GET  | `/admin/historial?id=&desde=&estado=` | Cargar historial con **filtro por estado** + **cursor** (hoy sin params) |
| POST | `/admin/reenviar {msg_id}` | Reenviar mensaje (ya usado) |
| POST | `/confirmar {ids:[i64], estado}` | Forzar avance de estado (Entregado/Leído/Procesado/Fallido) |
| POST | `/enviar {de_id, para_id, texto}` | Componer mensaje nuevo a un peer |
| POST | `/admin/purgar {id}` | Purgar cola+outbox del peer |
| GET  | `/admin/redis` | Contadores de cola/outbox por peer (badge de pendientes) |
| GET  | `/listar` (instancias) | Poblar selectores de peer (de/para) |

Campos de `Mensaje` disponibles para UI: `id, de_id, para_id, texto, enviado_en, estado,
entregado_en, leido_en, procesado_en, intentos, reenviado_de, reenvios`.

---

## 2. Features propuestas (≥15)

> Convención de variantes: **A** = control ligero en la propia fila/toolbar · **B** = acción en menú
> contextual / Popover · **C** = pop-up modal (Dialog del kit gpui-component). Cada feature indica
> endpoint y tipo de trazabilidad si aplica.

---

### trazabilidad-01 — Pop-up modal "Abrir mensaje" con timeline completo

- **Problema:** hoy el timeline es un panel **inline** que empuja la tabla hacia abajo y comparte
  scroll con ella; Max pidió "pop-ups de visualización". No hay un modal real que aísle el detalle.
- **Propuesta:** al hacer doble-click en una fila (o botón "Abrir"), abrir un `Dialog` modal con el
  timeline completo del mensaje: cabecera `#id`, ruta `de → para`, **cuerpo íntegro con wrap+scroll**,
  hitos con timestamps, estado actual, contadores, cadena de reenvío, y barra de acciones.
- **Variantes DS Ethos:**
  - **A.** Mantener panel inline actual pero como *drawer* lateral derecho (superficie tinta2, borde
    izquierdo brasa 1px, ancho 380px) que no desplaza la tabla.
  - **B.** `Popover` anclado a la fila (más ligero, para vistazo rápido; sin acciones destructivas).
  - **C.** `Dialog` centrado (recomendada): fondo `overlay` tinta al 70%, card tinta2 radio 14,
    título Fraunces papel, eyebrow "TIMELINE" en mono/humo/brasa, botón cerrar `×` en la esquina.
- **Endpoint:** ninguno extra (usa el `Mensaje` ya cargado en `historial`).
- **Trazabilidad:** timeline `enviado→entregado→leído→procesado` con timestamps timbrados por el broker.
- **Prioridad:** **alta**.

### trazabilidad-02 — Filtro por estado (chips segmentados)

- **Problema:** no puede acotar el historial a "solo fallidos" o "solo procesados"; ve todo mezclado.
  El broker YA acepta `estado=` en `/admin/historial` pero el cliente no lo pasa.
- **Propuesta:** barra de chips-filtro sobre la tabla: `Todos · ○ Enviado · ◑ Entregado · ◑ Leído ·
  ● Procesado · ✕ Fallido · ✕ Dead-letter`. Al clicar, recarga vía `historial(id, estado=Some(..))`.
- **Variantes DS Ethos:**
  - **A.** Chips pill (radio 999): activo con fondo brasa + texto salmón; inactivo borde línea +
    texto papel, hover tinta2. El color semántico del estado se muestra como punto a la izquierda.
  - **B.** `Select` desplegable "Estado: Todos ▾" (más compacto, una sola fila).
  - **C.** Toggle-group segmentado (superficie tinta2, separadores línea) estilo "tabs" internas.
- **Endpoint:** `GET /admin/historial?id=&estado=` (extender `cliente.historial` con params).
- **Trazabilidad:** filtra por el estado terminal/actual de cada mensaje.
- **Prioridad:** **alta**.

### trazabilidad-03 — Búsqueda de texto en mensajes

- **Problema:** no hay forma de encontrar un mensaje por su contenido; con colas largas es inviable.
- **Propuesta:** `Input` de búsqueda que filtra en cliente sobre `texto` (y opcionalmente `de_id`),
  con resaltado del término en la columna texto y contador "N de M".
- **Variantes DS Ethos:**
  - **A.** `Input` en la toolbar con icono lupa (brasa), placeholder humo, borde línea; radio control 10.
  - **B.** Búsqueda incremental tipo "command palette" (`Dialog` con input + lista de resultados).
  - **C.** Barra de filtro contraíble que aparece con atajo `/` (paridad con búsquedas de terminal).
- **Endpoint:** filtrado en cliente (broker no ofrece full-text); complementa `historial`.
- **Trazabilidad:** n/a (es descubrimiento).
- **Prioridad:** **alta**.

### trazabilidad-04 — Ver texto completo del mensaje en la fila (sin recorte destructivo)

- **Problema:** la columna texto se recorta a 80 chars con `…`; Max no ve el mensaje entero sin abrir
  el timeline. Queja textual: "ver mensaje completo (hoy se recorta)".
- **Propuesta:** `Tooltip` con el texto íntegro al pasar sobre la celda recortada, + opción "expandir
  fila" que hace wrap del texto en varias líneas.
- **Variantes DS Ethos:**
  - **A.** `Tooltip` (superficie tinta2, borde línea, texto papel, mono para datos) al hover.
  - **B.** Fila expansible: click en un chevron `›` (humo→brasa) despliega el texto con wrap dentro
    de la misma fila (fondo tinta2 sutil).
  - **C.** Modo "densidad": toggle en toolbar entre "compacta" (recorte actual) y "cómoda" (2-3 líneas).
- **Endpoint:** ninguno (dato ya presente).
- **Trazabilidad:** n/a.
- **Prioridad:** **alta**.

### trazabilidad-05 — Reenviar desde la fila (acción rápida) + confirmación

- **Problema:** reenviar solo está dentro del timeline; requiere seleccionar y abrir. En la TUI basta
  la tecla `r` sobre la fila.
- **Propuesta:** botón/acción "Reenviar" directo en la fila (o menú contextual), con un toast de
  confirmación ("reenviado como #N") vía `Notification` del kit.
- **Variantes DS Ethos:**
  - **A.** Botón fantasma brasa que aparece al hover de la fila (alineado a la derecha).
  - **B.** Menú contextual (click derecho / `⋯`) con "Reenviar", "Abrir", "Copiar texto".
  - **C.** Botón siempre visible en una columna de acciones fija a la derecha (icono ↻ brasa).
- **Endpoint:** `POST /admin/reenviar {msg_id}` (ya integrado).
- **Trazabilidad:** incrementa `reenvios` y setea `reenviado_de` en el mensaje nuevo.
- **Prioridad:** **alta**.

### trazabilidad-06 — Componer y enviar mensaje nuevo a un peer

- **Problema:** el broker tiene `POST /enviar`, pero el cliente desktop **no lo expone** y no hay UI.
  Max no puede "tocarle el hombro" a un peer por mensaje desde el desktop.
- **Propuesta:** botón "Nuevo mensaje" → `Dialog` con `Select` destino (peers vivos), `Input`
  multilínea para el texto, y "Enviar". Tras enviar, recarga el historial del par.
- **Variantes DS Ethos:**
  - **A.** `Dialog` centrado con formulario vertical (label eyebrow, inputs borde línea, botón
    primario brasa "Enviar", secundario fantasma "Cancelar").
  - **B.** Panel lateral persistente tipo "compositor" (siempre disponible al pie de la pestaña).
  - **C.** Barra inline al pie de la tabla (input + select + botón) sin modal.
- **Endpoint:** `POST /enviar {de_id, para_id, texto}` (añadir `cliente.enviar`).
- **Trazabilidad:** el mensaje nace `Enviado`, entra al historial del destino.
- **Prioridad:** **media**.

### trazabilidad-07 — Forzar avance de estado (confirmar manual)

- **Problema:** si un mensaje se queda "ghosteado" (Leído sin Procesado), Max no puede empujarlo a
  Procesado ni marcarlo Fallido a mano. El broker tiene `POST /confirmar {ids, estado}` sin uso en desktop.
- **Propuesta:** en el timeline/modal, botones "Marcar entregado / leído / procesado / fallido"
  (según el estado actual, solo se ofrecen avances válidos) que llaman `confirmar`.
- **Variantes DS Ethos:**
  - **A.** Fila de botones-estado en el modal, cada uno con su color semántico (verde=procesado,
    ámbar=leído, rojo=fallido), deshabilitados los inválidos.
  - **B.** `Select` "Cambiar estado a ▾" + botón confirmar (más sobrio).
  - **C.** Menú contextual en la fila "Forzar estado ▸" con submenú.
- **Endpoint:** `POST /confirmar {ids:[msg_id], estado}`.
- **Trazabilidad:** timbra el timestamp del nuevo estado (el broker pone su reloj).
- **Prioridad:** **media**.

### trazabilidad-08 — Confirmar en LOTE (selección múltiple)

- **Problema:** no se pueden seleccionar varias filas para operar en bloque; `confirmar` acepta un
  vector de ids pero la UI no lo permite.
- **Propuesta:** checkboxes por fila + toolbar de selección ("3 seleccionados · Marcar procesado ·
  Reenviar todos · Deseleccionar").
- **Variantes DS Ethos:**
  - **A.** `Checkbox` (borde línea, check brasa) en una columna inicial; barra de acción flotante
    tinta2 al haber selección.
  - **B.** Modo selección activable con botón "Seleccionar" (evita ruido visual permanente).
  - **C.** Selección por rango con Shift-click (paridad con tablas de escritorio).
- **Endpoint:** `POST /confirmar {ids:[..], estado}` (una llamada) / `reenviar` en bucle.
- **Trazabilidad:** operación auditada en lote.
- **Prioridad:** **baja**.

### trazabilidad-09 — Cadena de reenvíos navegable

- **Problema:** `reenviado_de` solo se muestra como texto "reenvío del mensaje #N"; no se puede saltar
  al original ni ver toda la cadena de reintentos manuales.
- **Propuesta:** en el modal, `reenviado_de` es un **enlace** que carga/abre el mensaje original;
  además una mini-lista "cadena de reenvíos" con cada eslabón y su estado.
- **Variantes DS Ethos:**
  - **A.** Enlace brasa subrayado "#N" que al clicar selecciona esa fila en la tabla.
  - **B.** Breadcrumb de la cadena `#12 → #34 → #56` en el header del modal (mono, separador humo).
  - **C.** Árbol/timeline vertical de la cadena con el estado de cada eslabón coloreado.
- **Endpoint:** `historial` ya trae la cadena; navegación en cliente por `reenviado_de`.
- **Trazabilidad:** linaje completo de un mensaje a través de sus reenvíos.
- **Prioridad:** **media**.

### trazabilidad-10 — Copiar mensaje / id / timeline al portapapeles

- **Problema:** no se puede exportar/copiar un mensaje ni su id para pegarlo en otro sitio (bug report,
  chat con un peer).
- **Propuesta:** acciones "Copiar texto", "Copiar id", "Copiar timeline (JSON)" en el menú de la fila
  y en el modal, con toast de confirmación.
- **Variantes DS Ethos:**
  - **A.** Iconos de copia (humo→brasa al hover) junto a cada dato en el modal.
  - **B.** Menú contextual "Copiar ▸".
  - **C.** Botón "Exportar" que copia el timeline entero como JSON formateado.
- **Endpoint:** ninguno (clipboard local del sistema).
- **Trazabilidad:** exporta el registro del mensaje para auditoría externa.
- **Prioridad:** **baja**.

### trazabilidad-11 — Purgar cola del peer en foco (con confirmación destructiva)

- **Problema:** no hay forma de vaciar la cola/outbox de un peer desde Trazabilidad; solo existe en la
  pestaña Redis. El broker tiene `POST /admin/purgar {id}`.
- **Propuesta:** botón "Purgar cola" (secundario, tono peligro) que abre un `Dialog` de confirmación
  destructiva ("Esto borra N mensajes pendientes de <peer>. Irreversible.").
- **Variantes DS Ethos:**
  - **A.** Botón rojo tenue en la toolbar; `Dialog` con doble confirmación (checkbox "entiendo").
  - **B.** Acción escondida en menú "⋯ Mantenimiento" para evitar clics accidentales.
  - **C.** Modal con input de confirmación por nombre (escribir el id del peer para habilitar).
- **Endpoint:** `POST /admin/purgar {id}` (ya en `cliente.purgar`).
- **Trazabilidad:** el historial durable se conserva; solo se limpia la bandeja activa/outbox.
- **Prioridad:** **baja**.

### trazabilidad-12 — Badge de pendientes por peer en el selector

- **Problema:** el selector de peer no dice cuántos mensajes pendientes tiene cada uno; hay que entrar
  a cada uno para verlo.
- **Propuesta:** cada pill del selector muestra un `Badge` con el nº de pendientes (`/admin/redis`),
  destacando en rojo si hay `Fallido/DeadLetter`.
- **Variantes DS Ethos:**
  - **A.** `Badge` circular brasa con el número a la derecha del nombre del peer.
  - **B.** Punto de color (verde/ámbar/rojo) según la salud de su cola.
  - **C.** Contador "3⚠" en mono humo, rojo si hay fallidos.
- **Endpoint:** `GET /admin/redis` (colas + outbox por id).
- **Trazabilidad:** salud de la cola de cada peer de un vistazo.
- **Prioridad:** **media**.

### trazabilidad-13 — Timeline con hitos terminales (Fallido / Dead-letter) e intentos

- **Problema:** el timeline solo pinta los 4 hitos felices; si un mensaje es `Fallido`/`DeadLetter`
  o tuvo varios `intentos`, esa información no aparece como hito con su timestamp.
- **Propuesta:** añadir al timeline los hitos `✕ fallido` / `✕ dead-letter` (rojo) cuando el estado
  los alcanzó, y un desglose de `intentos` (nº y, si el modelo lo soporta en fases futuras, cada
  reintento). Marcar el hito "actual" con el acento brasa.
- **Variantes DS Ethos:**
  - **A.** Extender la lista de hitos con los terminales en rojo; línea de conexión vertical humo
    entre hitos alcanzados, punteada para los no alcanzados.
  - **B.** Timeline horizontal (stepper) con el paso actual resaltado en brasa.
  - **C.** Dos columnas: "ciclo feliz" a la izquierda, "fallos/reintentos" a la derecha.
- **Endpoint:** `Mensaje.estado/intentos` (ya presente); requiere que el store timbre `Fallido`.
- **Trazabilidad:** ciclo de vida completo incluyendo caminos de error.
- **Prioridad:** **media**.

### trazabilidad-14 — Ordenar y ajustar columnas de la tabla

- **Problema:** la tabla es de orden fijo (cronológico ascendente) y anchos fijos; no se puede ordenar
  por estado/hora ni ver las columnas más cómodas.
- **Propuesta:** encabezados clicables para ordenar (id, de, estado, enviado) asc/desc, con indicador
  de orden; opcionalmente usar el componente `Table` del kit.
- **Variantes DS Ethos:**
  - **A.** Encabezados eyebrow clicables con flecha ▲▼ brasa; orden en cliente.
  - **B.** Migrar a `Table` de gpui-component (sorting nativo, temado a Ethos).
  - **C.** `Select` "Ordenar por ▾" en la toolbar (sin tocar encabezados).
- **Endpoint:** orden en cliente sobre `historial` (o `desde` para cronológico incremental).
- **Trazabilidad:** n/a (organización de la vista).
- **Prioridad:** **baja**.

### trazabilidad-15 — Paginación / carga incremental con cursor `desde`

- **Problema:** para colas largas se carga todo de golpe; el broker soporta cursor `desde` (msg_id >
  desde) pero la UI no lo usa.
- **Propuesta:** cargar los últimos N y un botón "Cargar más antiguos" que pasa `desde` = id más
  bajo mostrado, o scroll infinito.
- **Variantes DS Ethos:**
  - **A.** Botón "Cargar más ↑" al inicio de la tabla (fantasma brasa), con spinner al cargar.
  - **B.** Scroll infinito con sentinel al llegar al tope.
  - **C.** Paginación clásica "Página 1/N" en el pie (mono humo, flechas brasa).
- **Endpoint:** `GET /admin/historial?id=&desde=` (cursor ya soportado, sin usar en cliente).
- **Trazabilidad:** navegación temporal por el historial durable.
- **Prioridad:** **media**.

### trazabilidad-16 — Filtro por par (remitente/destino)

- **Problema:** el historial es "de un peer", pero mezcla mensajes de varios remitentes; no se puede
  ver solo la conversación con otro peer concreto.
- **Propuesta:** `Select` "De/Para: <peer> ▾" que filtra las filas por `de_id`/`para_id`, para ver la
  conversación 1:1.
- **Variantes DS Ethos:**
  - **A.** Dos `Select` (De, Para) en la toolbar con opción "Cualquiera".
  - **B.** Vista "conversación" al clicar un remitente en la fila (filtra a ese par).
  - **C.** Chips de los remitentes presentes en el historial (como el selector de peer, pero de `de_id`).
- **Endpoint:** filtrado en cliente sobre `historial` (broker no filtra por par).
- **Trazabilidad:** hilo de conversación entre dos peers.
- **Prioridad:** **media**.

### trazabilidad-17 — Accesibilidad de teclado + atajos visibles

- **Problema:** el código admite en la práctica solo click/Enter; no hay ring de foco, ni atajos
  documentados, ni navegación por teclado equivalente a la TUI (`↑↓`, `r`, `Enter`, `PageUp/Down`).
- **Propuesta:** navegación con flechas por filas (ring brasa en la fila enfocada), `Enter` abre modal,
  `r` reenvía, `/` busca, `Esc` cierra; una barra de atajos al pie (como la TUI).
- **Variantes DS Ethos:**
  - **A.** Ring de foco brasa 1px + fondo tinta2 en la fila enfocada; barra de atajos mono/humo al pie.
  - **B.** Overlay de ayuda `?` con todos los atajos (Dialog).
  - **C.** Tooltips con el atajo en cada botón ("Reenviar · r").
- **Endpoint:** ninguno.
- **Trazabilidad:** n/a (accesibilidad).
- **Prioridad:** **media**.

### trazabilidad-18 — Auto-refresco y "seguir cola" en vivo

- **Problema:** el historial se carga al enfocar el peer; no se actualiza solo, Max no ve llegar los
  mensajes en tiempo real ni cómo avanzan de estado.
- **Propuesta:** toggle "En vivo" que refresca el historial cada N s (usando `desde` para traer solo lo
  nuevo) y resalta brevemente las filas que cambiaron de estado.
- **Variantes DS Ethos:**
  - **A.** `Switch` "En vivo" en la toolbar (track brasa cuando activo) + indicador "actualizado hh:mm:ss".
  - **B.** Botón "Refrescar ↻" manual (más conservador) con badge "N nuevos".
  - **C.** Auto-scroll al pie con las novedades resaltadas en brasa tenue por 2s.
- **Endpoint:** `GET /admin/historial?id=&desde=` en polling incremental.
- **Trazabilidad:** observación en vivo del avance de estados timbrados por el broker.
- **Prioridad:** **baja**.

---

## 3. Impacto y dependencias

- **Cliente (`peers-desktop/src/cliente.rs`):** extender `historial(id)` → `historial(id, desde,
  estado)`; añadir `enviar(de_id, para_id, texto)` y `confirmar(ids, estado)`. `purgar` y `admin_redis`
  ya existen.
- **Estado (`AppDesktop` / `EstadoPantalla`):** añadir campos de filtro (`traza_filtro_estado`,
  `traza_busqueda`, `traza_par`), selección múltiple (`traza_marcados`), y flag de modal (para el
  Dialog en lugar del panel inline).
- **Vista:** la firma pura actual se conserva (`render_trazabilidad(&EstadoPantalla)`); las nuevas
  acciones siguen el patrón `#[derive(Action)]` + `.on_action(cx.listener(..))` ya establecido.
- **Sin cambios de broker** para el grueso: 01–12, 14–18 usan endpoints existentes. El único punto que
  depende del backend es 13 (que el store timbre `Fallido/DeadLetter`), que puede quedar en fase posterior.

## 4. Priorización sugerida

- **Alta:** 01 (modal), 02 (filtro estado), 03 (búsqueda), 04 (texto completo), 05 (reenviar en fila).
- **Media:** 06 (enviar), 07 (forzar estado), 09 (cadena reenvíos), 12 (badges), 13 (hitos terminales),
  15 (paginación), 16 (filtro por par), 17 (accesibilidad).
- **Baja:** 08 (lote), 10 (copiar), 11 (purgar), 14 (ordenar), 18 (en vivo).
