# RFC — Registro de acciones de los peers (bitácora) visible en la Jornada

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]] · [[jornada/RFC-jornada|RFC Jornada]]

| Campo | Valor |
|-------|-------|
| **Título** | Registrar el rastro de acciones que ejecutan los peers y mostrarlo en la pestaña Jornada |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Ámbito** | **mixto**: MOTOR en `peers-broker` (registrar) + `peers-core` (DTO/trait/store); VISTA en `peers-desktop/src/vista/jornada.rs` (+ espejo `peers-tui`) |
| **Planificada por** | Julio (s003, coord/QA) — implementa Jefim (s004, dev senior) |
| **Design System** | Ethos (tinta #100D0A · tinta2 #1A1611 · papel #ECE5D7 · brasa #C9A96E · humo #938B7B · línea #2B271F) |
| **Fecha** | 2026-07-02 |
| **Estado** | EN CURSO — DTOs R1-R3 en peers-core (2026-07-02); diseño SQLx/FK en revisión Julio↔Jefim (ver STATE.md: propuesta de .db propio con identidad durable vs FK a `instancias` efímera). |

---

## 1. El problema (en palabras de Max)

> "Ver el registro de acciones de vosotros [los peers Julio/Jefim] también en la Jornada."

Hoy la Jornada de un peer muestra **sesiones** (fichaje) y **tareas** (con estimado/real/estado), pero
**NO muestra el rastro de lo que el peer HIZO**: cada `crear_tarea`, `reportar_tarea`, `cerrar_tarea`,
`definir_resumen`, `enviar_mensaje`, kick (`salir`), purga… es una acción con un instante, y hoy se
pierde o queda dispersa. Max quiere ver, en la Jornada, **la bitácora cronológica de acciones del peer**
— igual que un parte de trabajo de un empleado: qué tocó, a qué hora, sobre qué sujeto.

### Qué hay HOY (verificado en el código, para NO inventar)

| Dato existente | Dónde | Sirve como acción? |
|----------------|-------|--------------------|
| `Tarea { inicio, fin, estado, ... }` | `peers-core/lib.rs:342` | Parcial: deriva "creada/cerrada", pero no reasignaciones ni reportes con hora exacta como evento. |
| Reportes de progreso `"<ahora> — <texto>"` | `cprs:reportes:{id}` (`main.rs:519`) | Sí, pero atados a UNA tarea, no a un feed del peer. |
| `Instancia { resumen, visto_en }` | `peers-core/lib.rs:46` | Solo el ÚLTIMO resumen y el último latido; no el historial de cambios. |
| `info!("admin: purgada…")` / `"alerta descartada"` | `main.rs:1076,1149` | Solo log stdout, **NO persistido**, no consultable. |
| Historial durable de mensajes | `cprs:historial` | Es de mensajes de cola, no de acciones del peer. |

**Conclusión:** NO existe hoy un feed unificado de "acciones del peer". Hay que **crearlo** (motor en el
broker) y **mostrarlo** (vista en Jornada). Es el núcleo de esta RFC.

---

## 2. La solución

Un **registro de acciones** (bitácora / audit log) por peer: cada vez que el broker procesa una acción
atribuible a un peer, **timbra un evento** `{ quien, accion, sujeto?, detalle?, cuando }` con SU reloj
(regla sagrada: el tiempo lo pone el broker) y lo persiste en una LIST acotada por peer. La Jornada añade
una **tercera sección "Acciones"** (timeline cronológico) que lee esos eventos vía un endpoint nuevo.

- **Reutiliza el patrón que ya existe** para reportes/alertas/historial: LIST en Redis + espejo SQLite,
  retención por poda (últimas N), timbrado por el broker. Cero infra nueva, cero deps nuevas.
- **Default no intrusivo:** registrar es barato (un RPUSH); leer es opt-in desde la Jornada.
- **Encaja en la RFC Jornada** como su sección de trazabilidad temporal (jornada-04/10 hablan de timeline).

---

## 3. Requisitos (trazables)

### Modelo — `peers-core`

- **R1** — `AccionRegistrada { quien: String, accion: TipoAccion, sujeto: Option<String>, detalle: Option<String>, cuando: String }`.
  `cuando` = ISO 8601 timbrado por el broker. `sujeto` = id de tarea/peer/cola afectado. `detalle` = texto
  corto libre (p.ej. el nuevo resumen, el motivo del kick).
- **R2** — `TipoAccion` (enum, `#[non_exhaustive]`, serialize en español): `CrearTarea`, `ReportarTarea`,
  `CerrarTarea`, `EditarTarea`, `CambiarEstadoTarea`, `ReasignarTarea`, `ForzarTarea`, `DefinirResumen`,
  `EnviarMensaje`, `Kick`, `Purgar`, `ResolverAlerta`. Extensible sin romper (nuevas variantes = compat).
- **R3** — `#[serde(default)]` donde toque; una bitácora ausente deserializa como vacía (compat total).

### Motor — `peers-broker`

- **R4** — Helper único `registrar_accion(quien, accion, sujeto, detalle)` que timbra `cuando` con el reloj
  del broker y hace el RPUSH/INSERT. Llamado en los handlers que ya mutan: `tarea/asignar`, `tarea/reportar`,
  `tarea/estado`, `tarea/editar`, `tarea/reasignar`, `tarea/forzar`, `definir-resumen`, `enviar`, `salir`,
  `admin/purgar`, `admin/alerta-resolver`. UN sitio por handler, tras el éxito de la mutación (nunca registrar
  una acción que falló).
- **R5** — `quien` se resuelve del emisor real de la acción: el `instancia_id`/`de` del payload. Las acciones
  del operador (Max desde desktop/TUI) se atribuyen a su id reservado (mismo id que la RFC política-comunicación
  y el fix de colisión — **unificar**). Así la Jornada de Max muestra lo que Max hizo, y la de cada peer lo suyo.
- **R6** — Endpoint bajo token: `GET /acciones?instancia_id=&desde=&limite=` → `Vec<AccionRegistrada>` en orden
  cronológico inverso (lo más reciente primero), acotado a `limite` (default 100). `desde` = cursor opcional.
- **R7** — Retención: poda por peer a las últimas N (reusar el patrón `podar_historial`, `main.rs:1442`).
  Constante en `peers-core` (p.ej. `RETENCION_ACCIONES = 500`). No crece sin límite.

### Persistencia — `peers-core` (SQLx con FK + Redis)

> **Decisión de Max (2026-07-02):** la bitácora se persiste con **SQLx** en una tabla relacional con
> **FOREIGN KEYS** — para tener rastreabilidad real y NO "datos sueltos". Alcance acotado: SOLO la tabla
> `acciones` estrena SQLx; el resto del store sigue como está (rusqlite / Redis). No se migra todo.

- **R8 — Tabla `acciones` con SQLx + FK:**
  ```sql
  CREATE TABLE IF NOT EXISTS acciones (
      id          INTEGER PRIMARY KEY AUTOINCREMENT,
      instancia_id TEXT NOT NULL,
      accion      TEXT NOT NULL,          -- TipoAccion serializado
      sujeto      TEXT,                    -- id de tarea/peer/cola afectado (nullable)
      tarea_id    TEXT,                    -- FK opcional a tareas cuando la acción es sobre una tarea
      detalle     TEXT,
      cuando      TEXT NOT NULL,           -- ISO 8601 timbrado por el broker
      FOREIGN KEY (instancia_id) REFERENCES instancias(id) ON DELETE CASCADE,
      FOREIGN KEY (tarea_id)     REFERENCES tareas(id)      ON DELETE SET NULL
  );
  CREATE INDEX IF NOT EXISTS idx_acciones_instancia ON acciones(instancia_id, cuando DESC);
  ```
  - **PRAGMA foreign_keys=ON** debe activarse por conexión (SQLite NO lo aplica por defecto). Con SQLx:
    en el `after_connect` del pool o vía `SqliteConnectOptions::foreign_keys(true)`.
  - Migración versionada en `migrations/` con `sqlx::migrate!()` (aplicada al arrancar el broker).
  - Consultas **compile-time checked** con `sqlx::query!`/`query_as!` (requiere `DATABASE_URL` o
    `.sqlx/` offline cache commiteado — decidir en Design: preferir modo offline para no exigir DB en build).
- **R8b — Trait `Almacen`:** `registrar_accion(accion)` (INSERT) y `acciones(instancia_id, desde, limite)`
  (SELECT ... ORDER BY cuando DESC LIMIT). Pool `SqlitePool` (async nativo — a diferencia de rusqlite que
  es síncrono; ver §5.6). Redis sigue como cache/cola pero la VERDAD de la bitácora es la tabla SQL.
- **R9** — Degradar: si el INSERT de bitácora falla, **NO** tumbar la acción de negocio — la mutación
  principal ya ocurrió; el fallo se loguea (`warn!`) sin propagar error al cliente. La bitácora es
  observabilidad, no una transacción crítica. (La FK con `ON DELETE SET NULL` evita que borrar una tarea
  rompa el histórico: la acción queda, pierde solo el enlace.)

### Vista — `peers-desktop` (+ espejo `peers-tui`)

- **R10 — Desktop:** tercera sección en la Jornada, **"Acciones"** (tras Sesiones y Tareas): timeline
  cronológico. Cada fila: hora (IBM Plex Mono, humo) · icono/etiqueta del `TipoAccion` (chip brasa tenue) ·
  sujeto (si lo hay, clicable → salta al detalle de esa tarea/peer) · detalle recortado. Reusa
  `fila_seleccionable` y los helpers Ethos de `tema.rs`. Vacío → estado "sin acciones registradas".
- **R11 — Desktop: filtro por tipo** (opcional v1): un `Select`/pills para filtrar por `TipoAccion`
  (p.ej. solo "mensajes" o solo "tareas"). Si añade complejidad, se difiere a v2.
- **R12 — TUI (paridad mínima):** la Jornada de la TUI muestra las últimas N acciones en texto
  (hora · acción · sujeto). La TUI corre por SSH; Max debe poder ver la bitácora sin la desktop.
- **R13** — Todo degrada: broker offline/401 → banner "no se pudo cargar el registro", sin crash,
  sin `.unwrap()`/`.expect()` en prod. Red SIEMPRE en `background_executor` + `bloquear_en`
  (regla anti-SIGABRT del proyecto — la misma trampa que causó el crash del release viejo).

---

## 4. Criterios de aceptación

- **AC1 (R4/R6)** — Tras un `crear_tarea` de un peer, `GET /acciones?instancia_id=<peer>` devuelve un
  evento `CrearTarea` con `cuando` timbrado por el broker y `sujeto = tarea_id`.
- **AC2 (R5)** — Una acción hecha por el operador (Max) aparece en SU jornada, no en la del peer destino;
  una acción de un peer aparece en la del peer.
- **AC3 (R10)** — La pestaña Jornada de la desktop muestra la sección "Acciones" con el timeline; clicar
  un sujeto salta a su detalle. Sin acciones → estado vacío legible (no error).
- **AC4 (R7)** — Con > N acciones, solo se retienen las últimas N (poda); el endpoint respeta `limite`.
- **AC5 (R9 compat)** — Un peer sin bitácora (clave ausente) devuelve lista vacía sin error; JSON viejo
  deserializa sin romper.
- **AC6 (R12)** — La TUI muestra las últimas acciones del peer seleccionado por SSH.
- **AC7 (R13)** — Broker caído: la sección Acciones muestra banner de error, la app NO crashea.

---

## 5. Riesgos y decisiones (Design)

1. **Ámbito mixto (broker + desktop).** Como la política de comunicación: el motor va en el broker (único
   sitio por donde pasan las acciones), el control/vista en la desktop. Implementar motor primero (Fase 1),
   vista después (Fase 2). El motor se puede validar por API (`curl /acciones`) antes de tocar UI.
2. **Volumen.** Cada acción = 1 evento. Con retención de 500/peer y RPUSH+LTRIM, coste O(1) por acción y
   memoria acotada. Sin impacto en la ruta caliente (registrar es tras la mutación, no la bloquea).
3. **`quien` del operador.** Depende del id reservado del operador — **mismo tema** que la RFC
   política-comunicación (§5.3) y el fix de colisión de ID (`STATE.md`). Resolver los tres juntos: un
   solo id estable "Max desde desktop/TUI".
4. **¿Registrar lecturas?** NO. Solo acciones que MUTAN (crear, cambiar, enviar, kick, purgar). Ver una
   pantalla no es una acción de bitácora (ruido). v1 = solo mutaciones (los handlers de R4).
5. **Relación con reportes de tarea.** Los reportes (`cprs:reportes:{id}`) siguen viviendo atados a la
   tarea (detalle de tarea). La bitácora los duplica como evento `ReportarTarea` en el feed del peer, con
   `sujeto = tarea_id` → clic salta al detalle. No se borra lo existente.
6. **SQLx (async) coexiste con rusqlite (síncrono) — DECISIÓN DE DISEÑO clave.** El backend SQLite actual
   usa `rusqlite` (síncrono, feature `sqlite`). SQLx es async y trae su propio pool. Opciones para Jefim:
   (a) añadir `sqlx` con runtime tokio + `sqlite` feature y un `SqlitePool` SEPARADO solo para la tabla
   `acciones` (más simple, dos accesos al mismo fichero .db — cuidado con locks: usar WAL mode
   `PRAGMA journal_mode=WAL` para lectores/escritores concurrentes); (b) apuntar SQLx a un fichero .db
   propio de bitácora (aísla del store principal, sin locks compartidos, pero las FK a `instancias`/`tareas`
   solo funcionan si viven en la MISMA db → si se aísla, las FK se vuelven lógicas, no reales). **Recomendado
   (a):** mismo fichero + WAL, así las FK a instancias/tareas son REALES (que es justo lo que Max pide:
   rastreabilidad con FK, no datos sueltos). Jefim valida el enfoque con los fundamentos SQLx y lo documenta.
7. **Dependencia nueva JUSTIFICADA.** Esta RFC añade `sqlx` (con features sqlite+runtime-tokio). Es la
   ÚNICA dep nueva y Max la pidió explícitamente por la rastreabilidad FK. No viola el "cero deps externas
   de runtime" (SQLx es una lib Rust compilada al binario, no un servicio aparte como sería RabbitMQ).

---

## 6. Constraints

- Sin `.unwrap()`/`.expect()` en prod; `Result`/`anyhow`. Red en desktop SIEMPRE vía `background_executor`
  + `bloquear_en` (regla anti-SIGABRT). RwLock/borrow nunca a través de `.await`. Español salvo protocolo.
- **El tiempo lo timbra el broker** (regla sagrada). La UI NUNCA calcula `cuando`.
- **CERO dependencias nuevas.** Reusa Redis/SQLite y los patrones de historial/reportes ya existentes.
- No romper contratos: `/jornada` sigue igual; "Acciones" es una sección NUEVA con endpoint NUEVO
  (`/acciones`), no altera `RespuestaJornada`. Versionar plugin si se tocan binarios. NUNCA `Co-Authored-By`.
  Jornada en el commit.

## 7. Fuera de alcance (v1)

- Registrar lecturas/navegación (solo mutaciones).
- Exportar la bitácora a fichero (posible v2).
- Filtro avanzado por rango de fechas en la UI (v1 = últimas N + filtro por tipo opcional).
- Correlación cross-peer ("qué hizo el equipo entero") — v1 es por-peer; un feed global agregado es v2.

## 8. Dependencias

- **Id reservado del operador** — compartido con [[politica-comunicacion/RFC-politica-comunicacion]] (§5.3)
  y el fix de colisión (`STATE.md`). Resolver juntos.
- **Vista** — se integra en [[jornada/RFC-jornada]] como su sección de trazabilidad temporal; reusa el
  patrón de carga de `desktop-carga-datos` y los helpers de `tema.rs`.
- **Anti-SIGABRT** — la vista debe seguir el patrón `mutar_*`/`background_executor` (el crash del release
  viejo fue exactamente por saltarse esto).

---

## 9. Plan de implementación (para Jefim)

**Fase 1 — Motor (broker + core), validable por API sin UI:**
1. `peers-core`: `AccionRegistrada` + `TipoAccion` + constante `RETENCION_ACCIONES`.
2. Trait `Almacen`: `registrar_accion` + `acciones` en Redis y SQLite.
3. Broker: helper `registrar_accion(...)` + llamada en los ~11 handlers mutadores (R4). Poda en el barrido.
4. Endpoint `GET /acciones` bajo token.
5. Verificación: `curl` acciones tras crear/reportar/cerrar → eventos timbrados (AC1-AC5).

**Fase 2 — Vista (desktop + TUI):**
6. Desktop: sección "Acciones" en `vista/jornada.rs` (timeline Ethos, red vía `background_executor`).
7. TUI: últimas N acciones en la jornada.
8. Verificación: QA visual (Julio) — la sección aparece, clic en sujeto salta, degrada sin crash (AC3/AC7).

> Build verde por fase, SIN commit; Max hace la prueba final. Julio hace QA (API en Fase 1, visual en Fase 2).

---
#rfc #peers-desktop #jornada #registro-acciones #bitacora #broker #trazabilidad
