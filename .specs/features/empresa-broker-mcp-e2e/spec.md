# Spec — Cableado broker↔MCP end-to-end: la empresa vive en el broker, los agentes la operan por MCP

> Fecha: 2026-07-03. Decisiones de fondo: `.specs/empresa/06-decisiones.md` (E-01..E-23). Modelo:
> `.specs/empresa/{01,02,03,04}`. **Público: los agentes implementadores de Max** (spec "masticada para que
> el workflow SOLO EJECUTE"). Verificado contra el código real (`archivo:línea`). Template exacto = la
> feature "política de comunicación" (misma forma extremo a extremo).

## Contexto

Hoy la "empresa" (proyectos, cargos, agentes, organigrama, cadena de mando) **no existe en el broker**: es
solo config/specs. Y el MCP (`crates/peers-client/src/mcp.rs`) está **escrito a mano** y solo expone 9
tools de mensajería/tareas — ningún agente puede **consultar ni operar la empresa**. Max quiere que la
empresa **viva en el broker** y que los agentes, **por el MCP**, la conozcan y la operen. Además (E-21) el
MCP debe migrar a **`rmcp`** (SDK oficial, ya en `Cargo.toml:37`, hoy sin usar), y (E-22) se **permiten
dependencias externas** por robustez.

Estado verificado que se reusa como TEMPLATE:
- Trait `Almacen` (`crates/peers-core/src/almacen.rs:20`); política en `almacen.rs:285-305`.
- Redis `AlmacenRedis` (`crates/peers-broker/src/store.rs`); política en `store.rs:862-907`; helper de
  clave `k_*` (~`store.rs:92`); pool `conn()` (`store.rs:48`).
- SQLite `AlmacenSqlite` (`crates/peers-broker/src/db.rs`); política en `db.rs:994-1060`; DDL en `abrir`
  (`db.rs:199-209`).
- Estado del broker + cache `RwLock<Politica>` (`main.rs:134`), carga al arrancar (`main.rs:1726`),
  handlers admin (`main.rs:1431-1457`), router (`main.rs:1874`), intercepción en `enviar`
  (`main.rs:465-505`), token middleware (`main.rs:211-225`).
- Client: `ClienteBroker` + helper `post` (`crates/peers-client/src/broker.rs:35`); MCP a mano
  (`mcp.rs:110` tools, `mcp.rs:262` instructions, `main.rs:237` `ejecutar_tool`, `tool_*`), `EstadoCliente`
  (`main.rs:48`), "mi id" = `estado.id.read().await.clone().ok_or(...)?`.

## Objetivo

Un agente, vía MCP (migrado a `rmcp`), puede **consultar** la empresa (contexto, proyectos, su cargo, su
organigrama, su cadena de mando) — y esos datos **viven en el broker**, persistidos en Redis+SQLite. En
Fase 2, el broker **valida** las acciones contra la cadena de mando (regla dura). End-to-end verificable.

## Requisitos (trazables)

### Fase 0 — Migración del MCP a `rmcp` (E-21) — prerequisito de todo lo demás

- **R0.1** — `peers-client` depende de `rmcp` (`server`, `transport-io`, `macros`) y reescribe el servidor
  MCP con `#[tool]`/`#[tool_router]`/`ServerHandler`/`serve(stdio())`. Las 9 tools actuales se re-declaran
  como funciones Rust `#[tool]` con el MISMO nombre/descripción/schema (paridad exacta, español).
- **R0.2 (RIESGO #1 — spike primero)** — El push del canal usa una **notificación custom**
  `notifications/claude/channel` + capability `experimental["claude/channel"]` (`mcp.rs:83-105,240-241`).
  **Verificar que `rmcp` puede (a) declarar la capability experimental en `get_info()`/`ServerInfo`, y
  (b) emitir una notificación con método arbitrario** (`Peer<RoleServer>`/sink). Si NO puede: fallback =
  conservar un sink de notificación custom junto a `rmcp` para ESA notificación. **El wire con el harness
  se preserva EXACTO:** `serverInfo.name = "claude-peers"`, método `notifications/claude/channel`, y las 4
  claves `meta` en inglés (`from_id`/`from_summary`/`from_cwd`/`sent_at`).
- **R0.3** — Las `instructions` (`mcp.rs:262`) se sirven vía `get_info()` de `rmcp`, conservando el anuncio
  del `id_efectivo` y el protocolo actual; se ENRIQUECEN en R3 (conocimiento).
- **R0.4** — Paridad de comportamiento: `initialize`/`tools/list`/`tools/call`/`ping` y el push siguen
  funcionando idénticos para el harness (mismo `<channel>`); los tests existentes del client pasan.

### Fase 1 — Store organizativo en el broker + tools de LECTURA (la empresa vive en el broker)

**Modelo (peers-core)** — reusa el patrón de DTOs de política (`lib.rs:855-992`):
- **R1** — DTOs `Cargo`, `Proyecto`, `Agente`, `Organigrama` (+ `Ubicacion`, `Capacidad`) según
  `.specs/empresa/02-modelo-dominio.md` §2, con `#[serde(default)]` para compat. `Organigrama` = vista
  agregada (proyectos + cargos + agentes + relaciones) que sirve `/mi-organigrama`/`/contexto-empresa`.
- **R2** — DTOs de petición/respuesta para los endpoints (§Endpoints), en `peers-core` (el client solo
  consume `peers_core::*`).

**Persistencia (trait Almacen + ambos backends)** — espeja política:
- **R3** — Métodos nuevos en `Almacen` (`almacen.rs`): `proyecto_guardar/leer/listar`, `cargo_guardar/leer/
  listar`, `agente_guardar/leer/listar`, `organigrama_leer` (derivado). Todos `async … -> anyhow::Result`.
- **R4** — Impl Redis (`store.rs`): claves `cprs:org:proyecto:{id}`, `cprs:org:cargo:{id}`,
  `cprs:org:agente:{id}` + sets índice (`cprs:org:proyectos`, etc.). `get/set` + serde con fallback; listar
  = SMEMBERS + lectura (NUNCA `KEYS`, como el resto).
- **R5** — Impl SQLite (`db.rs`): tablas `proyectos`/`cargos`/`agentes` (blob JSON como política, o columnas
  — decidir en design), DDL en `abrir`; espejo exacto del contrato Redis.

**Endpoints (broker) — bajo token, `rutas_protegidas`:**
- **R6** — Siembra (operador): `POST /admin/proyecto`, `POST /admin/cargo`, `POST /admin/agente` (upsert
  idempotente) + `GET` equivalentes para listar.
- **R7** — Consulta (para las tools del agente): `GET /contexto-empresa` (empresa + catálogo de proyectos),
  `GET /mi-cargo?id=`, `GET /mi-organigrama?id=`, `GET /mi-cadena?id=` (reporta_a / puede_delegar_a).
- **R8** — Validación de integridad al guardar cargo: `reporta_a`/`puede_delegar_a` existen y no forman
  ciclo (DFS); si falla → `ok:false` de negocio (no 500), patrón `enviar` (`main.rs:474-479`).

**Client — métodos + tools MCP de lectura (rmcp):**
- **R9** — Métodos `ClienteBroker` (`broker.rs`) para R7 (patrón `self.post`/get con header token).
- **R10** — Tools MCP nuevas (`#[tool]`, español): `contexto_empresa`, `listar_proyectos`, `mi_cargo`,
  `mi_organigrama`, `mi_cadena_mando`. Devuelven texto formateado para el agente (molde `tool_listar`).
  "Mi id" = `estado.id` (el client ya lo conoce; no lo pasa el agente).
- **R11** — `instructions` enriquecidas (E-01, [[.specs/empresa/04-conocimiento-agente]] §2.2): bloques
  Empresa/Proyectos/Tu-equipo servidos desde el broker en el boot + Bloque skill (cuándo usar las tools).
  Degrada a la versión mínima actual si el broker no responde.

### Fase 2 — Escritura con regla dura (E-03/E-12) — secuenciada tras Fase 1

- **R12 (prerequisito, E-10)** — **Anti-spoofing del `de_id`:** el broker ata el `de` de una acción a la
  identidad **registrada** del emisor (no al campo libre del payload). Spike de diseño primero (handle/token
  por peer sobre el HTTP actual). Sin esto, la validación dura es falsificable.
- **R13** — Validación server-side en `tarea/asignar`/`tarea/reasignar` (`main.rs`, junto a donde vive
  `evaluar_politica`, `main.rs:465-505`): el `de` debe poder delegar en el cargo del `para` (según el store
  organizativo) o ser operador; si no → `ok:false` "fuera de tu cadena de mando". Capacidades (`crear_tarea`
  /`forzar`) validadas igual.
- **R14** — Cada mutación organizativa deja bitácora (`AccionRegistrada`, variantes nuevas de `TipoAccion`
  `#[non_exhaustive]`: `CrearProyecto`/`DefinirCargo`/`ContratarAgente`; compat).

## Criterios de aceptación

- **AC1 (R0.1-R0.4)** — Tras migrar a `rmcp`: `tools/list` muestra las 9 tools con nombres/schemas
  idénticos; un mensaje entrante sigue apareciendo como `<channel>` en la sesión receptora (push intacto);
  los tests del client pasan. (Si el spike R0.2 falla, el fallback mantiene el push.)
- **AC2 (R6/R4/R5)** — `POST /admin/cargo` persiste un cargo; reiniciar el broker (Redis) y con
  `--features sqlite` lo devuelve `GET`. Cargo con `reporta_a` inexistente o cíclico → `ok:false` (R8).
- **AC3 (R7/R10)** — Un agente lanzado como `backend@proyecto-x` llama la tool `mi_cargo` y recibe su
  cargo+cadena desde el broker; `mi_organigrama` devuelve su equipo; `listar_proyectos` el catálogo.
  Verificable en una sesión real: preguntar "¿cuál es tu cargo y a quién reportas?" y que responda con
  datos del broker.
- **AC4 (R11)** — Un agente recién arrancado nombra la misión de LexusFX y lista los proyectos en su primer
  turno (instructions enriquecidas); con el broker sin esos endpoints, arranca con las instructions mínimas
  (degrada).
- **AC5 (R13, Fase 2)** — Con cargos `coordinador (puede_delegar_a=[backend])` y `qa`, `coordinador`
  asignando a `backend` pasa; asignando a `qa` (fuera de su cadena) → `ok:false`. Max (operador) puede a
  cualquiera.
- **AC6 (compat)** — Redis/JSON viejos sin claves `cprs:org:*` deserializan; la app opera como hoy (empresa
  opt-in). `db.rs` sin las tablas nuevas las crea en `abrir`.
- **AC7 (paridad backends)** — Todos los tests corren con y sin `--features sqlite` (roundtrip default +
  upsert + listar), como `db.rs:1105-1153`.

## Constraints

- Español salvo claves de protocolo. Sin `.unwrap()`/`.expect()` en prod; `anyhow::Result`. El tiempo lo
  timbra el broker. `#[serde(default)]` para compat; `TipoAccion` `#[non_exhaustive]`. El `RwLock` de
  cache **nunca cruza `.await`**. Reusa DTOs de `peers-core` (cero duplicación). Redis default + SQLite
  feature (impl en ambos). **E-22: dependencias externas permitidas** (rmcp, y libs de robustez si hacen
  falta), manteniendo el **binario portable** (libs compiladas, no servicios externos). Preservar EXACTO el
  wire con el harness (nombre `claude-peers`, notificación del canal, 4 claves `meta`). **Versionar el
  plugin** (bump) al tocar los binarios. Recargar el broker/LaunchAgent tras recompilar. Jornada en el
  cuerpo del commit. NUNCA `Co-Authored-By`.

## Fuera de alcance (esta spec)

- La UI desktop que expone todo esto → `.specs/empresa/08-capa-ui.md`. El pipeline de provisión/lanzamiento
  (id `rol@proyecto`, `--append-system-prompt`) → `.specs/empresa/03-pipeline-provision.md` (feature
  aparte). El modelo de loop/supervisión → `.specs/empresa/10-loop-engineering-supervision.md`. La
  contratación/lanzamiento desde la app (toca desktop). El anti-spoofing (R12) se **diseña** aquí como
  prerequisito de Fase 2, pero su spike puede salir como sub-spec.
