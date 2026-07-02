# RFC-desktop-config — Configuración operable en la pestaña Config (peers-desktop)

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

## Header & Metadata

| Campo | Valor |
|-------|-------|
| **Título** | Convertir la pestaña Config de solo-formulario en un panel de conexión, diagnóstico y control real |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Impacto** | MEDIO-ALTO — Config es la puerta de entrada de toda la app (sin conexión válida, ninguna otra pestaña carga) |
| **Fecha** | 2026-07-02 |
| **Estado** | PROPUESTO (pendiente de decisión) |
| **Pestaña** | `crates/peers-desktop/src/vista/config.rs` |
| **Referencia TUI** | `crates/peers-tui/src/ui/config.rs` + teclas en `peers-tui/src/main.rs` |
| **Referencia broker** | `crates/peers-broker/src/main.rs` (rutas líneas 1459-1499) |

---

## Background — qué hace hoy la pestaña y qué le falta

**Qué hace hoy (verificado en `vista/config.rs`):**

- Muestra **3 campos editables inline** (`Input` de gpui-component): `broker_url`, `token` (con toggle de máscara) y `refresh_ms`.
- Un único botón dorado **"Guardar"** que valida mínimamente (`broker_url` no vacío, `refresh_ms` entero > 0) y persiste `~/.config/claude-peers/config.toml` con permisos 0600.
- Feedback de una línea (verde salvia OK / terracota error) bajo el botón.
- Existe una `Action RecargarConfig` **declarada pero no cableada** (`recargar()` es `dead_code`) y **no hay ningún botón** que la dispare.

**Qué le falta (el problema real de Max: "no tengo ningún control"):**

1. **No prueba la conexión.** Max edita `broker_url`/`token`, guarda, y no sabe si el broker responde. No hay botón "Probar conexión" que golpee `GET /salud`. Descubre que la URL está mal cuando otra pestaña sale vacía.
2. **No hay "restablecer defaults".** `Config::default()` existe en código pero no hay botón para volver a `http://127.0.0.1:7899 / sin token / 1000ms`.
3. **La validación es de guardado, no de edición.** No hay feedback en vivo mientras escribe (URL malformada, refresh absurdo). El error solo aparece al pulsar Guardar.
4. **`refresh_ms` es un número desnudo.** No hay slider/stepper, ni presets (500/1000/2000/5000), ni aviso de "esto martillará el broker".
5. **El token no se puede rotar ni vaciar con confianza.** No hay "borrar token", "copiar", ni indicador de si el broker actual exige token (401 vs 200).
6. **No muestra info del broker conectado.** `GET /admin/info` devuelve host, puerto, versión y nº de instancias — nada de eso se ve. Max no sabe a qué versión de broker está hablando.
7. **No hay diagnóstico de colas.** `GET /admin/redis` da colas/outbox pendientes por peer — no se expone; no hay forma de purgar (`POST /admin/purgar`) desde la UI.
8. **Sin trazabilidad de la propia config.** No se ve la ruta del archivo, ni cuándo se guardó por última vez, ni un historial de cambios.
9. **Sin accesibilidad.** Los `Input` no tienen orden de tab explícito, ni labels asociados accesibles, ni atajos de teclado (la TUI tiene `e`/`s`/`↑↓`; la desktop no tiene nada equivalente).
10. **Sin recarga desde disco.** Si Max edita el TOML a mano (o lo hace la TUI), la desktop no lo relee sin reiniciar.

**Endpoints del broker disponibles y hoy NO usados por esta pestaña:**

| Endpoint | Método | Qué da | Auth |
|----------|--------|--------|------|
| `/salud` | GET | `{estado, instancias}` — **exento de token** (línea 1499) | No |
| `/admin/info` | GET | `{host, puerto, instancias, version}` | Token |
| `/admin/redis` | GET | colas + outbox pendientes por peer | Token |
| `/admin/purgar` | POST `{id}` | borra cola + outbox de un id | Token |
| `/admin/historial` | GET | historial durable de una cola | Token |
| `/listar` | POST | instancias vivas | Token |

**Nota de arquitectura relevante:** `PanelConfig` es un `Entity` con `cx` propio, así que puede lanzar `cx.spawn(...)` para llamadas HTTP asíncronas al broker y mutar su estado local con el resultado — no necesita rebotar por el árbol de Actions. Esto habilita todas las features de "probar/diagnosticar" sin tocar la Fundación.

---

## Features propuestas (≥15)

> Cada feature: **problema** (qué no puede hacer Max hoy) → **propuesta** → **2-3 variantes de diseño Ethos** → **endpoint/método** → **trazabilidad** → **prioridad**.
> DS Ethos: fondo `#100D0A`, superficie `#1A1611`, texto `#ECE5D7`, acento dorado `#C9A96E`, humo `#938B7B`, línea `#2B271F`. Radios card 14 / control 10 / pill 999. Fraunces (títulos) / Inter (UI) / IBM Plex Mono (datos).

---

### config-01 — Botón "Probar conexión"
- **Problema:** Max guarda una `broker_url`/`token` y no sabe si el broker responde hasta que otra pestaña sale vacía. No hay verificación.
- **Propuesta:** Botón "Probar conexión" que lanza `GET {broker_url}/salud` con timeout ~3s y pinta el resultado: OK (verde salvia, "conectado · N instancias · Xms") o error (terracota, mensaje del fallo: timeout / conexión rechazada / 401).
- **Variantes Ethos:**
  1. **Botón secundario junto a Guardar** (borde LINEA, texto dorado) con un chip de estado (`chip_estado`) a su derecha que cambia de humo→salvia→terracota.
  2. **Botón con spinner inline**: al pulsar, el label pasa a "Probando…" con un punto dorado pulsante (pill 999); resultado reemplaza el chip.
  3. **Badge de latencia**: además del OK, un `Badge` mono con los ms de respuesta (`142 ms`) en dorado sobre TINTA2.
- **Endpoint:** `GET /salud` (exento de token → prueba la URL aunque el token esté mal).
- **Trazabilidad:** registrar timestamp del último test y su resultado (ver config-15).
- **Prioridad:** **ALTA** — es el control nº1 que falta; sin él Max edita a ciegas.

### config-02 — Restablecer valores por defecto
- **Problema:** No hay forma de volver a la config de fábrica; `Config::default()` existe pero es inalcanzable desde la UI.
- **Propuesta:** Botón "Restablecer" que re-siembra los 3 Inputs con `Config::default()` (`http://127.0.0.1:7899`, sin token, 1000ms) sin guardar todavía (el usuario confirma con Guardar).
- **Variantes Ethos:**
  1. **Link terciario** (texto humo, hover dorado) discreto bajo el formulario: "Restablecer valores por defecto".
  2. **Botón fantasma** (sin fondo, borde LINEA) en la barra de acciones junto a Guardar/Probar.
  3. **Pop-up de confirmación** (Dialog Ethos) si hay ediciones sin guardar: "¿Descartar cambios y volver a defaults?" con botón dorado "Restablecer" y fantasma "Cancelar".
- **Endpoint:** ninguno (local, `Config::default()`).
- **Trazabilidad:** entrada en el historial local de cambios de config.
- **Prioridad:** **ALTA**.

### config-03 — Recargar config desde disco
- **Problema:** Si el TOML cambia por fuera (edición manual o la TUI lo guarda), la desktop no lo ve sin reiniciar. `recargar()` ya existe pero es `dead_code`.
- **Propuesta:** Cablear un botón "Recargar desde disco" que dispare `RecargarConfig`/`panel.recargar(...)`, re-sembrando los Inputs con lo persistido y avisando si descarta ediciones.
- **Variantes Ethos:**
  1. **Icono de refresco dorado** (pill redondo) en la cabecera de la card, junto al título "Broker".
  2. **Botón fantasma** "Recargar" en la barra de acciones.
  3. **Auto-detección**: banner sutil TINTA2 con borde dorado "El archivo cambió fuera de la app — Recargar" cuando `mtime` del TOML difiere del cargado.
- **Endpoint:** ninguno (relee disco).
- **Trazabilidad:** timestamp de última recarga.
- **Prioridad:** **MEDIA**.

### config-04 — Validación en vivo de `broker_url`
- **Problema:** El error de URL solo aparece al Guardar; no hay feedback mientras escribe. Una URL sin esquema (`127.0.0.1:7899`) pasa la validación actual (solo comprueba no-vacío) y falla en runtime.
- **Propuesta:** Validar en cada edición: esquema `http(s)://` presente, host parseable, sin barra final. Marcar el campo con borde terracota + nota terciaria si es inválida; borde dorado si es válida.
- **Variantes Ethos:**
  1. **Borde semántico del wrapper del Input**: LINEA normal → terracota si inválido, salvia si válido.
  2. **Icono de estado** a la derecha del Input (✓ dorado / ⚠ terracota) tipo pill.
  3. **Nota terciaria dinámica** bajo el campo que reemplaza la ayuda estática por el motivo del error.
- **Endpoint:** ninguno (validación local en frontera).
- **Trazabilidad:** —
- **Prioridad:** **MEDIA**.

### config-05 — Control `refresh_ms` con presets + stepper
- **Problema:** `refresh_ms` es un campo de texto libre; Max puede escribir `1` (martillar el broker) o `999999` sin aviso. No hay guía.
- **Propuesta:** Sustituir/complementar el Input por pills de preset (500 · 1000 · 2000 · 5000 ms) + stepper +/−, con aviso si baja de un umbral (p.ej. < 300ms → "puede saturar el broker").
- **Variantes Ethos:**
  1. **Fila de pills** (radio 999): la activa en dorado sólido sobre TINTA, las demás borde LINEA; "Custom" abre el Input.
  2. **Slider dorado** con etiqueta de valor mono y marcas en los presets.
  3. **Select Ethos** (dropdown TINTA2 con borde LINEA) con los presets + opción "Personalizado…".
- **Endpoint:** ninguno (afecta el timer de refresco local).
- **Trazabilidad:** —
- **Prioridad:** **MEDIA**.

### config-06 — Panel "Estado del broker" (info en vivo)
- **Problema:** Max no sabe a qué broker habla: host, puerto, versión, nº de instancias vivas. Todo eso existe en `/admin/info` y no se muestra.
- **Propuesta:** Card secundaria "Broker conectado" que, al conectar, muestra `host:puerto`, versión y nº de instancias, refrescándose con el `refresh_ms`.
- **Variantes Ethos:**
  1. **Card TINTA2** con 4 filas eyebrow+valor mono (Host / Puerto / Versión / Instancias).
  2. **Barra de resumen** compacta en la cabecera: `● conectado · v0.x · 4 peers` con punto de estado dorado.
  3. **Grid 2×2 de mini-stats** (número grande Fraunces + label humo), estilo dashboard.
- **Endpoint:** `GET /admin/info`.
- **Trazabilidad:** —
- **Prioridad:** **ALTA**.

### config-07 — Copiar / borrar token
- **Problema:** El token solo se puede editar carácter a carácter. No hay "copiar al portapapeles" ni "borrar" (vaciar → `None`) con un gesto claro.
- **Propuesta:** Botones inline en el campo token: copiar (al portapapeles) y borrar (vacía el Input; al guardar se persiste como broker-sin-token).
- **Variantes Ethos:**
  1. **Dos iconos pill** a la derecha del Input (copiar / papelera), color humo → dorado en hover.
  2. **Menú contextual** (Popover) del campo token con "Copiar", "Borrar", "Pegar desde portapapeles".
  3. **Botón "Rotar token"** que abre un Dialog con Input nuevo + confirmación (para cambio deliberado de secreto).
- **Endpoint:** ninguno (portapapeles + local).
- **Trazabilidad:** registrar "token modificado/borrado" (sin loguear el valor) en el historial de config.
- **Prioridad:** **BAJA**.

### config-08 — Indicador de si el broker exige token
- **Problema:** Max no sabe si el broker actual corre con token o sin él. Si deja el token vacío contra un broker protegido, todo da 401 y no es obvio por qué.
- **Propuesta:** Tras "Probar conexión", además del `/salud` (exento), intentar `GET /admin/info` (protegido): 200 → "token válido"; 401 → "el broker exige token y el actual no vale"; ambos fallan → sin conexión.
- **Variantes Ethos:**
  1. **Chip de estado del token** bajo el campo: "sin token requerido" (humo) / "token válido" (salvia) / "token inválido/faltante" (terracota).
  2. **Semáforo de dos puntos** (salud + auth) en la barra de estado.
  3. **Fila en el panel config-06**: "Auth: requerida ✓ / no requerida".
- **Endpoint:** `GET /salud` + `GET /admin/info` (distinguir 200/401).
- **Trazabilidad:** último resultado de auth con timestamp.
- **Prioridad:** **MEDIA**.

### config-09 — Diagnóstico de colas y purga
- **Problema:** No hay forma desde la desktop de ver colas/outbox pendientes ni de purgar una cola atascada; la TUI y `/admin/redis`+`/admin/purgar` lo permiten.
- **Propuesta:** Sección "Colas del broker" (colapsable) que lista peers con pendientes (mensajes/outbox) y ofrece purgar por peer.
- **Variantes Ethos:**
  1. **Tabla Ethos** (id | pendientes msg | pendientes outbox | acción "Purgar") con botón terracota por fila.
  2. **Lista de chips**: un chip por cola con pendientes; click abre Popover con "Purgar".
  3. **Pop-up modal** (Dialog) "Purgar cola de {id}" con confirmación dorada, ya que es destructivo.
- **Endpoint:** `GET /admin/redis` (listar) + `POST /admin/purgar {id}` (purgar).
- **Trazabilidad:** registrar cada purga (id + timestamp) en historial local de acciones admin.
- **Prioridad:** **MEDIA**.

### config-10 — Mostrar ruta y estado del archivo de config
- **Problema:** El feedback dice "Guardado en {ruta}" solo tras guardar; no hay indicación permanente de dónde vive el archivo, sus permisos (0600) ni su `mtime`.
- **Propuesta:** Fila informativa fija: ruta del TOML (mono, copiable), permisos actuales y fecha de última modificación.
- **Variantes Ethos:**
  1. **Pie de card** con texto mono humo + icono copiar dorado.
  2. **Fila eyebrow+valor** "Archivo · ~/.config/claude-peers/config.toml (0600)".
  3. **Tooltip** sobre un icono de info que muestra ruta + permisos + mtime.
- **Endpoint:** ninguno (metadata del filesystem).
- **Trazabilidad:** mtime = trazabilidad natural del último guardado.
- **Prioridad:** **BAJA**.

### config-11 — Botón "Abrir carpeta de config" / "Abrir en editor"
- **Problema:** Para editar el TOML a mano Max tiene que teclear la ruta en Finder/terminal.
- **Propuesta:** Botón que abre `~/.config/claude-peers/` en el explorador del SO (o el archivo en el editor por defecto).
- **Variantes Ethos:**
  1. **Link terciario** "Abrir carpeta" junto a la ruta (config-10).
  2. **Icono de carpeta dorado** pill al lado de la ruta.
  3. **Menú "…"** (Popover) con "Abrir carpeta", "Abrir en editor", "Copiar ruta".
- **Endpoint:** ninguno (`open`/`xdg-open`/`explorer`).
- **Trazabilidad:** —
- **Prioridad:** **BAJA**.

### config-12 — Guardado con estado dirty y confirmación al salir
- **Problema:** No hay indicación de que hay cambios sin guardar; Max puede cambiar de pestaña y perder ediciones sin aviso.
- **Propuesta:** Marcar el formulario como "sucio" cuando algún Input difiere de lo persistido; habilitar Guardar solo si hay cambios; avisar al navegar fuera con cambios pendientes.
- **Variantes Ethos:**
  1. **Botón Guardar deshabilitado** (opacidad reducida, sin fondo dorado) hasta que haya cambios; punto dorado junto al título cuando está sucio.
  2. **Barra de cambios sticky** al pie: "Tienes cambios sin guardar · Guardar / Descartar".
  3. **Dialog de confirmación** al intentar salir de la pestaña con cambios pendientes.
- **Endpoint:** ninguno (estado local).
- **Trazabilidad:** —
- **Prioridad:** **MEDIA**.

### config-13 — Accesibilidad y navegación por teclado
- **Problema:** La pestaña no tiene atajos (la TUI tiene `↑↓`/`e`/`s`), ni orden de tab explícito, ni labels accesibles, ni foco visible dorado (la propia nota del código dice que el foco dorado no está cableado).
- **Propuesta:** Orden de tab lógico (broker_url → token → refresh → Probar → Guardar), atajo `Cmd/Ctrl+S` para Guardar, `Esc` para descartar, foco visible dorado en el campo activo, labels asociados.
- **Variantes Ethos:**
  1. **Marco de foco dorado** (`track_focus` sobre el `focus_handle` del InputState) en el wrapper del Input activo.
  2. **Pista de atajos** en el pie de la card (mono humo): "⌘S guardar · Esc descartar · Tab siguiente".
  3. **Modo teclado explícito** que replica la TUI (`↑↓` mueve campo activo, `Enter` edita), para usuarios que vienen de la TUI.
- **Endpoint:** ninguno.
- **Trazabilidad:** —
- **Prioridad:** **MEDIA**.

### config-14 — Perfiles de conexión (multi-broker)
- **Problema:** Max solo tiene un `broker_url`/`token`. Alternar entre broker local, LAN (`10.0.0.x`) y túnel obliga a re-teclear todo cada vez.
- **Propuesta:** Guardar varios perfiles nombrados (local / lan / túnel) y cambiar entre ellos con un click; el activo se persiste como la config vigente.
- **Variantes Ethos:**
  1. **Select Ethos** de perfiles arriba del formulario (dropdown TINTA2), + "Nuevo perfil" / "Duplicar".
  2. **Fila de pills** de perfiles (activo dorado), estilo tabs.
  3. **Pop-up "Gestionar perfiles"** (Dialog con Table: nombre | url | token enmascarado | acciones).
- **Endpoint:** ninguno para cambiar; cada perfil probaría con `GET /salud`.
- **Trazabilidad:** registrar cambios de perfil activo con timestamp.
- **Prioridad:** **BAJA** (potente, pero no es el bloqueo actual).

### config-15 — Historial de conexiones y cambios de config (trazabilidad)
- **Problema:** No hay ningún registro de qué se cambió, cuándo se guardó, ni el resultado de los tests de conexión. Cero auditoría de la propia config.
- **Propuesta:** Timeline local (persistido junto al TOML o en un JSON hermano) con eventos: guardado, restablecido, test de conexión (OK/fallo + latencia), purga de cola, cambio de perfil — cada uno timbrado.
- **Variantes Ethos:**
  1. **Timeline vertical** en card lateral: punto dorado + hora mono + texto papel por evento (últimos N).
  2. **Tabla Ethos** (hora | evento | detalle | resultado) con badges de resultado.
  3. **Pop-up "Historial de config"** (Dialog) accesible desde un icono de reloj en la cabecera.
- **Endpoint:** ninguno (persistencia local); los eventos de conexión usan `GET /salud` y `/admin/info`.
- **Trazabilidad:** ESTA feature ES la trazabilidad de la pestaña.
- **Prioridad:** **MEDIA**.

### config-16 — Toggle de tema / densidad
- **Problema:** El tema Ethos es fijo; no hay control de apariencia (claro/oscuro/contraste) ni densidad (compacto/cómodo) para pantallas pequeñas o de alta resolución.
- **Propuesta:** Sección "Apariencia" con toggle de densidad (compacto/cómodo) y, si aplica, variante de contraste alto del tema Ethos; se persiste en la config.
- **Variantes Ethos:**
  1. **Switch Ethos** (pista LINEA, thumb dorado) para densidad + segmented control para contraste.
  2. **Fila de pills** (Compacto · Cómodo) estilo config-05.
  3. **Pop-up "Apariencia"** con preview en vivo de una card de ejemplo.
- **Endpoint:** ninguno (preferencia local).
- **Trazabilidad:** cambio registrado en config-15.
- **Prioridad:** **BAJA**.

### config-17 — Auto-reconexión y política de reintentos
- **Problema:** Si el broker cae, la app no indica ni gestiona la reconexión; no hay control de cuántas veces reintenta ni cada cuánto.
- **Propuesta:** Toggle "Reconectar automáticamente" + selector de backoff (fijo/exponencial) y máximo de reintentos; indicador de estado de conexión en vivo.
- **Variantes Ethos:**
  1. **Switch + Select** en la card de broker, con chip de estado "reconectando… (intento 2/5)".
  2. **Banner sticky** terracota cuando se pierde la conexión con botón "Reintentar ahora".
  3. **Semáforo de conexión** permanente en la cabecera (dorado=OK, humo=reconectando, terracota=caído).
- **Endpoint:** `GET /salud` como sonda de reconexión.
- **Trazabilidad:** eventos de caída/reconexión en config-15.
- **Prioridad:** **MEDIA**.

### config-18 — Diagnóstico completo ("Ejecutar diagnóstico")
- **Problema:** Cuando algo no conecta, Max no tiene un único botón que verifique todo (URL alcanzable, salud, auth, versión compatible, latencia) y le diga qué está mal.
- **Propuesta:** Botón "Ejecutar diagnóstico" que corre una batería (resolución DNS/host → `/salud` → `/admin/info` con token → chequeo de versión → medición de latencia) y presenta un checklist con ✓/⚠ por paso.
- **Variantes Ethos:**
  1. **Checklist en Dialog modal**: filas con icono de estado, texto papel y detalle terciario; pie con "Copiar diagnóstico".
  2. **Acordeón inline** en la propia card, cada paso una fila que se colorea al completar.
  3. **Stepper horizontal** (pills conectados) que avanza y se pone dorado/terracota por paso.
- **Endpoint:** `GET /salud` + `GET /admin/info` (encadenados).
- **Trazabilidad:** guardar el último diagnóstico completo con timestamp (config-15).
- **Prioridad:** **MEDIA**.

---

## Resumen de prioridades

| Prioridad | Features |
|-----------|----------|
| **ALTA** | config-01 (probar conexión), config-02 (restablecer), config-06 (info broker) |
| **MEDIA** | config-03, config-04, config-05, config-08, config-09, config-12, config-13, config-15, config-17, config-18 |
| **BAJA** | config-07, config-10, config-11, config-14, config-16 |

**Total: 18 features.**

## Cobertura de endpoints del broker

| Endpoint | Verificado en `main.rs` | Features que lo usan |
|----------|--------------------------|----------------------|
| `GET /salud` (exento token) | línea 1499 / 202 | 01, 08, 17, 18 |
| `GET /admin/info` | línea 1486 / 1026 | 06, 08, 18 |
| `GET /admin/redis` | línea 1487 / 1037 | 09 |
| `POST /admin/purgar` | línea 1488 / 1059 | 09 |
| (local / filesystem) | — | 02, 03, 04, 05, 07, 10, 11, 12, 13, 14, 15, 16 |
