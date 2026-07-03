# Design — Cableado broker↔MCP end-to-end (org store + tools rmcp)

> Acompaña a `spec.md`. Decisiones de arquitectura (Dn) mapeadas a los Rn del spec, con el patrón EXACTO a
> copiar (política) y el detalle de la migración a `rmcp`. Todo verificado contra `archivo:línea`.

## Decisiones de arquitectura

### D1 — El store organizativo ESPEJA la política de comunicación (verificado)
El patrón "entidad persistida en `Almacen` + ambos backends" ya está resuelto por la política. Se copia 1:1
para `Proyecto`/`Cargo`/`Agente`:

- **Trait** (`crates/peers-core/src/almacen.rs`, junto a `politica_leer` en `:285-305`):
  ```rust
  async fn proyecto_guardar(&self, p: &Proyecto) -> anyhow::Result<()>;
  async fn proyecto_leer(&self, id: &str) -> anyhow::Result<Option<Proyecto>>;
  async fn proyectos_listar(&self) -> anyhow::Result<Vec<Proyecto>>;
  // … cargo_* y agente_* idénticos …
  async fn organigrama_leer(&self) -> anyhow::Result<Organigrama>; // derivado: junta proyectos+cargos+agentes
  ```
- **Redis** (`crates/peers-broker/src/store.rs`, copiando `:862-907`): helper de clave
  `fn k_org_cargo(id:&str)->String { format!("{NS}org:cargo:{id}") }` (~`:92`); `conn.set`/`conn.get` con
  serde y fallback; **índice por SET** (`cprs:org:cargos`) para `listar` sin `KEYS`:
  ```rust
  // guardar: set del objeto + SADD al índice
  let _: () = conn.set(k_org_cargo(&c.id), serde_json::to_string(c)?).await?;
  let _: () = conn.sadd(k_org_cargos_idx(), &c.id).await?;
  // listar: SMEMBERS índice + get por id, filtrando corruptos con filter_map(...ok())
  ```
- **SQLite** (`crates/peers-broker/src/db.rs`, copiando `:994-1060`): DDL en `abrir` (`:199-209`):
  ```sql
  CREATE TABLE IF NOT EXISTS cargos (id TEXT PRIMARY KEY, json TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS proyectos (id TEXT PRIMARY KEY, json TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS agentes (id TEXT PRIMARY KEY, json TEXT NOT NULL);
  ```
  blob JSON (como política, `db.rs:195-198`: se reemplaza entero y el modelo crece → evita migración por
  campo). UPSERT `ON CONFLICT(id) DO UPDATE`; listar `SELECT json FROM … ` + `filter_map(Result::ok)`.
- **Por qué blob JSON y no columnas:** igual que política — la entidad se guarda/lee entera y su forma
  evolucionará (capacidades, etc.); el blob evita migraciones. Si se necesita query por campo (v2), se
  añaden columnas indexadas.

### D2 — Migración del MCP a `rmcp` (E-21): el mayor cambio, de-riesgado con un spike
`rmcp` (SDK oficial) reemplaza el JSON-RPC a mano. Estructura destino (verificada en la fuente oficial):
```rust
#[derive(Clone)]
struct ServidorPeers { estado: Arc<EstadoCliente>, tool_router: ToolRouter<Self> }

#[tool_router]
impl ServidorPeers {
    #[tool(description = "Lista otras instancias …")]  // MISMO texto español que hoy
    async fn listar_instancias(&self, Parameters(args): Parameters<ListarArgs>) -> String { … }
    // … las 9 tools actuales + las 5 nuevas de empresa …
}

#[tool_handler]
impl ServerHandler for ServidorPeers {
    fn get_info(&self) -> ServerInfo {
        // name = "claude-peers" (CRÍTICO), instructions = instrucciones(id), capabilities…
    }
}
// main: ServidorPeers{…}.serve(stdio()).await?.waiting().await?;
```
- **Las tools pasan de un array JSON (`mcp.rs:110`) a funciones `#[tool]`.** El schema lo genera
  `schemars` — se define un struct de args por tool (o `()` para las sin args). Nombres/descripciones =
  copia literal de los actuales (paridad).
- **`instructions`** salen por `get_info()` (no un campo manual) — misma función `instrucciones(id)`,
  enriquecida en D4.
- **RIESGO #1 — el push del canal (R0.2).** Es el punto que `rmcp` puede NO cubrir de fábrica. Plan:
  - **Spike (T1):** ¿`ServerInfo`/`ServerCapabilities` permite declarar `experimental["claude/channel"]`?
    ¿`Peer<RoleServer>` (o el sink de la conexión) permite `send_notification` con un **método arbitrario**
    y params custom? La doc oficial expone `context.peer.notify_*()` solo para notifs estándar.
  - **Si SÍ:** el push se emite con esa API; se guarda el `Peer` en `EstadoCliente` para el bucle de
    recepción (que hoy corre cada 1s y llama `empujar_canal`, `main.rs:707`).
  - **Si NO:** **fallback** = envolver `rmcp` para lo estándar (tools/handshake) y **conservar un escritor
    de stdout mínimo** SOLO para la notificación `notifications/claude/channel` (la lógica actual de
    `SalidaMcp::empujar_canal`, `mcp.rs:83-105`, que ya serializa una línea JSON con flush). Riesgo:
    coordinar dos escritores sobre el mismo stdout → serializar con el `Mutex` que ya existe
    (`SalidaMcp.writer`, `mcp.rs:31`). Documentar como deuda hasta que `rmcp` exponga notifs custom.
  - **Invariante:** el harness debe seguir viendo EXACTAMENTE el mismo wire (`serverInfo.name`, método,
    4 claves `meta`). Los tests de push del client son el guardarraíl.

### D3 — Endpoints: copian el andamiaje de `admin_politica_*` (verificado)
- Handlers `admin_proyecto_guardar`/`admin_cargo_guardar`/… copian `admin_politica_guardar`
  (`main.rs:1441`): extractor `State<...>` + `Json<Peticion>`, llaman `almacen.<x>_guardar`, responden
  `RespuestaOk`/`ok:false`. Los `GET /contexto-empresa`/`mi-cargo`/… copian `admin_politica_leer`
  (`main.rs:1431`).
- **Registro en el router** (`main.rs:1874`, dentro de `rutas_protegidas`): `.route("/admin/cargo",
  post(admin_cargo_guardar).get(admin_cargos_listar))`, etc. Todo bajo token (`verificar_token`,
  `main.rs:211-225`); NO tocar `/salud`.
- **Cache opcional:** el organigrama se consulta en el boot de cada agente (no ruta caliente como
  `enviar`), así que **no** necesita `RwLock` en `Estado` v1 — se lee del `Almacen` por request. (Si se
  vuelve caliente, cachear como `RwLock<Politica>`, `main.rs:134`, liberando el lock antes de `.await`.)

### D4 — `instructions` enriquecidas + tools de lectura (client)
- `instrucciones(id_efectivo)` (`mcp.rs:262`) gana bloques Empresa/Proyectos/Tu-equipo. Para rellenarlos, el
  client consulta el broker en el boot (`get_info` es sync; hacer la consulta ANTES, al construir el
  servidor, y pasar el texto ya compuesto). Degrada: si el broker no responde, usa el texto mínimo actual.
- Tools nuevas = funciones `#[tool]` que llaman `estado.broker.<metodo>` y formatean texto (molde
  `tool_listar`, `main.rs:266`). "Mi id" = `estado.id.read().await.clone().ok_or("Aún no registrado")?`
  (idiom de `tool_crear_tarea`, `main.rs:410`).
- Métodos `ClienteBroker` nuevos (`broker.rs`) usan el helper `post`/get (`broker.rs:35`, ya pone el header
  `X-Peers-Token`).

### D5 — Validación de cadena de mando (Fase 2) en el punto de la política (verificado)
`evaluar_politica` se llama en `enviar` entre `instancia_existe` y `encolar_mensaje` (`main.rs:465-505`).
La validación de delegación es análoga pero en `tarea/asignar`/`tarea/reasignar`: un helper
`puede_delegar(de, para, &organigrama) -> Result<(), Motivo>` que lee el store, comprueba `puede_delegar_a`
del cargo del `de` (o `de == operador`), y devuelve `ok:false` si no. **Prerequisito R12 (anti-spoofing):**
el `de` debe venir de la identidad registrada, no del payload — spike aparte (E-10).

## Componentes tocados

| Archivo | Qué se toca | Cuánto |
|---------|-------------|--------|
| `crates/peers-core/src/lib.rs` | DTOs `Cargo`/`Proyecto`/`Agente`/`Organigrama`/`Ubicacion`/`Capacidad` + `Peticion*`/`Respuesta*` + variantes `TipoAccion` | grueso del modelo |
| `crates/peers-core/src/almacen.rs` | firmas nuevas en el trait (bloque nuevo tras política) | medio |
| `crates/peers-broker/src/store.rs` | impl Redis (claves `k_org_*`, guardar/leer/listar) | medio |
| `crates/peers-broker/src/db.rs` | impl SQLite (DDL + métodos espejo) | medio |
| `crates/peers-broker/src/main.rs` | handlers admin + consulta, registro en router, (Fase 2) validación en asignar/reasignar | medio |
| `crates/peers-client/Cargo.toml` | añadir `rmcp` (server, transport-io, macros) | 1 línea |
| `crates/peers-client/src/mcp.rs` | **reescritura a `rmcp`** (tools `#[tool]`, `ServerHandler`, get_info); push del canal (spike) | **grueso** |
| `crates/peers-client/src/main.rs` | adaptar arranque a `serve(stdio())`; brazos/funciones de tools nuevas | grueso |
| `crates/peers-client/src/broker.rs` | métodos nuevos para los endpoints de empresa | bajo |
| `plugin/` (versión) | bump por tocar binarios | 1 línea |

## Riesgos

- **rmcp y la notificación custom del canal (D2, R0.2):** el mayor riesgo. **Mitigación:** spike PRIMERO
  (T1); fallback documentado (escritor de stdout mínimo para esa notif, serializado por el `Mutex`
  existente). Si el spike revela que `rmcp` no expone notifs custom NI capabilities experimentales, escalar
  a Max la decisión (fallback vs contribuir a rmcp vs mantener el push a mano). **No romper el `<channel>`
  es innegociable.**
- **Reescritura del client (D2):** es un cambio grande de un binario que Max USA a diario. **Mitigación:**
  hacerlo tras el spike, con los tests de push/tools como guardarraíl, y **recompilar el release que Max
  ejecuta** (lección de `STATE.md`: el binario en disco no se actualiza solo).
- **`de_id` spoofable (Fase 2, R12):** la regla dura es falsificable sin anti-spoofing. **Mitigación:**
  Fase 2 NO arranca hasta el spike de E-10; Fase 1 (lectura) no lo necesita.
- **Paridad de backends:** un método nuevo en Redis sin su espejo SQLite rompe `--features sqlite`.
  **Mitigación:** cada task implementa AMBOS y corre los tests con y sin la feature (AC7).
- **Tamaño de `instructions`:** conocimiento rico = texto largo. **Mitigación:** bloques concisos +
  punteros a las tools (`04-conocimiento-agente` §6.1).
