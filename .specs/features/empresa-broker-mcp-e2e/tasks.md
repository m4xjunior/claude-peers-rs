# Tasks — Cableado broker↔MCP end-to-end (org store + tools rmcp)

> Masticado para que el workflow SOLO EJECUTE. Cada tarea: **Archivos** exactos · **Hacer** · **Verificar**.
> Orden = dependencias. Ref: `spec.md` (R*), `design.md` (D*). Templates verificados: política de
> comunicación (`store.rs:862-907`, `db.rs:994-1060`, `almacen.rs:285-305`, `main.rs:1431-1457,1874`) y la
> receta de tool (`mcp.rs`, `broker.rs:35`). **E-22: se permiten deps externas (rmcp).**

## Dependencias (orden de ejecución)

```
T1 (spike rmcp push) ──► T2 (migrar MCP a rmcp) ──► T7 (tools lectura) ──► T8 (instructions)
                                                        ▲
T3 (DTOs core) ──► T4 (trait) ──┬► T5 (Redis) ──┐      │
                                └► T6 (SQLite) ──┴► (endpoints en main) ──► T7
T3 ──────────────────────────────────────────────────────────────────► T7
(Fase 2, tras spike E-10)  T9 (anti-spoofing) ──► T10 (validación cadena)
```

Fase 0 = T1-T2. Fase 1 = T3-T8. Fase 2 = T9-T10 (tras el spike de anti-spoofing).

---

## T1 — Spike: ¿`rmcp` puede emitir la notificación custom del canal?  [R0.2]
**Archivos:** (throwaway) un binario de prueba o un test en `crates/peers-client/`.
**Hacer:**
- Añadir `rmcp` (`server`, `transport-io`, `macros`) a `crates/peers-client/Cargo.toml`.
- Probar en un servidor `rmcp` mínimo: (a) declarar `capabilities.experimental["claude/channel"]` en
  `get_info()`/`ServerInfo`; (b) obtener el `Peer<RoleServer>` (o sink de la conexión) y emitir una
  notificación con **método literal** `"notifications/claude/channel"` y params `{content, meta{from_id,
  from_summary, from_cwd, sent_at}}`.
- Registrar el resultado en el propio spike y en `.specs/empresa/06-decisiones.md` E-21 (PENDIENTE).
**Verificar:** un cliente MCP de prueba (o el propio harness) recibe la notificación con ese método exacto.
Si NO es posible con la API pública de `rmcp` → documentar el **fallback** (D2: escritor stdout mínimo para
esa notif) y proceder con él en T2. **No avanzar a T2 sin cerrar esta pregunta.**

## T2 — Migrar el servidor MCP a `rmcp` (paridad exacta)  [R0.1, R0.3, R0.4]
**Archivos:** `crates/peers-client/src/mcp.rs`, `crates/peers-client/src/main.rs`,
`crates/peers-client/Cargo.toml`.
**Hacer:**
- Reescribir el servidor con `#[tool_router]`/`#[tool]`/`#[tool_handler] ServerHandler`/`serve(stdio())`.
- Re-declarar las **9 tools actuales** como funciones `#[tool]` con nombre/descripción/schema IDÉNTICOS
  (structs de args con `schemars::JsonSchema`; español). Mapear cada una a la lógica `tool_*` existente.
- `get_info()`: `serverInfo.name = "claude-peers"` (CRÍTICO), `instructions = instrucciones(id_efectivo)`,
  capabilities con `experimental["claude/channel"]` (o el fallback de T1).
- Push del canal: según T1 (API de `rmcp` o fallback). Preservar el bucle de recepción (`main.rs:666-744`)
  y el marcado de estado `Entregado`/`Leido` solo si el flush triunfó.
**Verificar:** `cargo build -p peers-client` exit 0; `cargo test -p peers-client` verde; arrancar el client
contra un broker de prueba y comprobar `tools/list` = 9 tools idénticas y que un mensaje entrante aparece
como `<channel>` en una sesión real (paridad AC1).

## T3 — DTOs organizativos en `peers-core`  [R1, R2]
**Archivos:** `crates/peers-core/src/lib.rs`.
**Hacer:**
- Definir `Cargo`, `Proyecto`, `Agente`, `Ubicacion`, `Capacidad`, `Organigrama` según
  `.specs/empresa/02-modelo-dominio.md` §2: `#[derive(Debug, Clone, Serialize, Deserialize, Default)]`
  donde aplique; cada campo `#[serde(default)]`; enums `#[serde(rename_all="lowercase")]`.
- Definir `Peticion*`/`Respuesta*` para los endpoints (T7): p.ej. `PeticionPorId{ id }`,
  `RespuestaCargo`, `RespuestaOrganigrama`, `RespuestaContextoEmpresa`.
- Añadir variantes a `TipoAccion` (`#[non_exhaustive]`, `lib.rs:1007`): `CrearProyecto`, `DefinirCargo`,
  `ContratarAgente` (snake_case).
**Verificar:** `cargo build -p peers-core` exit 0; un test de roundtrip serde con `{}` → default (compat).

## T4 — Firmas nuevas en el trait `Almacen`  [R3]
**Archivos:** `crates/peers-core/src/almacen.rs`.
**Hacer:**
- Importar los DTOs nuevos en el `use crate::{…}` (`almacen.rs:11-14`).
- Añadir, tras el bloque política (`:285-305`), con doc-comentarios: `proyecto_guardar/leer/proyectos_listar`,
  `cargo_guardar/leer/cargos_listar`, `agente_guardar/leer/agentes_listar`, `organigrama_leer`. Todas
  `async fn … -> anyhow::Result<…>`.
**Verificar:** `cargo build -p peers-core` exit 0 (el trait compila; las impls faltan → romperá el broker
hasta T5/T6, esperado).

## T5 — Impl Redis del store organizativo  [R4]
**Archivos:** `crates/peers-broker/src/store.rs`.
**Hacer:**
- Añadir helpers de clave `k_org_cargo(id)`/`k_org_proyecto(id)`/`k_org_agente(id)` + índices
  `k_org_cargos()`/… (~`store.rs:92`), documentados en el doc de módulo.
- Implementar los métodos de T4 en `impl Almacen for AlmacenRedis` copiando `store.rs:862-907`: `set`+`sadd`
  al índice para guardar; `get`+serde con fallback para leer; `smembers`+get+`filter_map(...ok())` para
  listar. `organigrama_leer` = junta `proyectos_listar`+`cargos_listar`+`agentes_listar`. NUNCA `KEYS`.
**Verificar:** `cargo build -p peers-broker` exit 0; test de integración Redis (patrón
`crates/peers-broker/tests/redis_integracion.rs`): guardar cargo → leer → listar.

## T6 — Impl SQLite del store organizativo (espejo)  [R5]
**Archivos:** `crates/peers-broker/src/db.rs`.
**Hacer:**
- DDL en `abrir` (junto a `:199-209`): tablas `cargos`/`proyectos`/`agentes` (`id TEXT PRIMARY KEY, json
  TEXT NOT NULL`).
- Implementar los métodos en `impl Almacen for AlmacenSqlite` copiando `db.rs:994-1060`: UPSERT
  `ON CONFLICT(id) DO UPDATE`; leer `query_row(...).ok()`+serde; listar `SELECT json …`+`filter_map(Result::ok)`.
  Espejo EXACTO del contrato Redis (mismo default/orden).
**Verificar:** `cargo build -p peers-broker --features sqlite` exit 0; test roundtrip default+upsert+listar
(patrón `db.rs:1105-1153`), con y sin la feature.

## T7 — Endpoints + métodos ClienteBroker + tools MCP de lectura  [R6, R7, R8, R9, R10]
**Archivos:** `crates/peers-broker/src/main.rs`, `crates/peers-client/src/broker.rs`,
`crates/peers-client/src/mcp.rs` (o el módulo de tools de T2).
**Hacer (broker):**
- Handlers `admin_cargo_guardar`/`admin_proyecto_guardar`/`admin_agente_guardar` (copiar
  `admin_politica_guardar`, `main.rs:1441`) + `GET` de consulta `contexto_empresa`/`mi_cargo`/
  `mi_organigrama`/`mi_cadena` (copiar `admin_politica_leer`, `main.rs:1431`).
- Validación de integridad al guardar cargo (R8): `reporta_a`/`puede_delegar_a` existen y sin ciclo (DFS);
  si falla → `ok:false` (patrón `main.rs:474-479`).
- Registrar rutas en `rutas_protegidas` (`main.rs:1874`), bajo token.
- Bitácora en cada guardar (R14): `AccionRegistrada` con la variante nueva.
**Hacer (client):**
- Métodos `ClienteBroker` para los `GET` (usar el helper, `broker.rs:35`).
- Tools `#[tool]`: `contexto_empresa`, `listar_proyectos`, `mi_cargo`, `mi_organigrama`, `mi_cadena_mando`
  (sin args; "mi id" = `estado.id.read().await.clone().ok_or(...)?`). Formatear texto (molde `tool_listar`).
**Verificar:** `curl -H "X-Peers-Token: …" -X POST …/admin/cargo` persiste; `GET …/mi-cargo?id=backend@px`
devuelve el cargo. En una sesión real, la tool `mi_cargo` responde con datos del broker (AC3). Cargo cíclico
→ `ok:false` (AC2). `cargo test --workspace` (con y sin `--features sqlite`) verde.

## T8 — `instructions` enriquecidas al boot  [R11]
**Archivos:** `crates/peers-client/src/mcp.rs`, `crates/peers-client/src/main.rs`.
**Hacer:**
- Antes de construir el servidor `rmcp`, consultar el broker (`contexto_empresa` + `mi_cargo` +
  `mi_organigrama`) y componer los bloques Empresa/Proyectos/Tu-equipo + Bloque skill
  (`.specs/empresa/04-conocimiento-agente.md` §2.2). Pasar el texto ya compuesto a `get_info()`.
- Degradar: si el broker no responde, usar el texto mínimo actual (`instrucciones` de hoy).
**Verificar:** un agente recién lanzado (id `rol@proyecto`) nombra la misión de LexusFX y lista proyectos en
su primer turno (AC4); con el broker sin esos endpoints, arranca con instructions mínimas (degrada).

## T9 — (Fase 2) Spike + diseño: anti-spoofing del `de_id`  [R12]
**Archivos:** (diseño) `.specs/empresa/` sub-spec; (spike) `crates/peers-broker/`.
**Hacer:**
- Diseñar cómo el broker ata el `de` de una acción a la identidad **registrada** del emisor (no al payload):
  opciones = handle/token por peer emitido en `/registrar`, o firma. Evaluar sobre el HTTP stateless actual.
- Decisión a `.specs/empresa/06-decisiones.md` E-10.
**Verificar:** un peer NO puede declararse `operador`/`broker` ni otro id y saltarse la validación (test).
**No avanzar a T10 sin cerrar esto** (la regla dura es falsificable sin ello).

## T10 — (Fase 2) Validación server-side de la cadena de mando  [R13, R14]
**Archivos:** `crates/peers-broker/src/main.rs`.
**Hacer:**
- Helper `puede_delegar(de, para, &organigrama) -> Result<(),Motivo>` (lee el store; `de==operador` exento).
- Llamarlo en `tarea/asignar`/`tarea/reasignar` (junto a `evaluar_politica`, `main.rs:465-505`); si no puede
  → `ok:false` "fuera de tu cadena de mando". Igual para capacidades (`crear_tarea`/`forzar`).
- Bitácora del intento (permitido y rechazado).
**Verificar:** AC5 — `coordinador` (puede_delegar_a=[backend]) asignando a `backend` pasa; a `qa` → `ok:false`;
operador a cualquiera. `cargo test --workspace` verde.

## Cierre
- `cargo build --release` (broker + client) y `cargo test --workspace` **con y sin `--features sqlite`** en
  verde. **Recompilar el release que Max ejecuta** (lección STATE.md: el binario en disco no se actualiza
  solo). Recargar el broker/LaunchAgent. **Bump de versión del plugin** (se tocaron binarios). Commit por
  fase (Fase 0 / Fase 1 / Fase 2), jornada en el cuerpo, **sin `Co-Authored-By`**. No romper el `<channel>`
  (guardarraíl: los tests de push). Si el spike T1 no cierra, escalar a Max antes de reescribir el client.
