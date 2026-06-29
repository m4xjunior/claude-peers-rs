# Tasks — Entrega durable + trazabilidad (Fase 1 + Fase 2)

> Masticado para que el workflow de desarrollo SOLO EJECUTE. Cada tarea: archivos exactos,
> qué hacer, y verificación. Orden = dependencias. Ref: spec.md (R*), RFC-001 (decisión).
> Tras editar el broker: recargar LaunchAgent. Tras tocar binarios del plugin: bump de versión.

## Dependencias (orden de ejecución)

```
T1 (tipos/estados en core) ──┬─► T2 (store: bandeja ZSET + peek)
                             ├─► T3 (store: transiciones + idempotencia)
                             └─► T4 (store: historial + quitar KEYS)
T2,T3,T4 ─► T5 (broker: /confirmar + handlers) ─► T6 (client: ACK real + reportar estados)
T5 ─► T7 (broker: /admin/historial + /admin/reenviar)
T7 ─► T8 (TUI: pantalla Trazabilidad + reenvío)
cada Tn ─► verificación de su(s) AC
```

---

## T1 — Tipos y máquina de estados en peers-core  [R1.2]

**Archivos:** `crates/peers-core/src/lib.rs`

**Hacer:**
- Añadir enum `EstadoMensaje { Enviado, Entregado, Leido, Procesado, Fallido, DeadLetter }` con `#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]`, `#[serde(rename_all="lowercase")]`.
- Extender `struct Mensaje`: reemplazar `entregado: bool` por `estado: EstadoMensaje` + `entregado_en: Option<String>` + `leido_en: Option<String>` + `procesado_en: Option<String>` + `intentos: u32` + `reenviado_de: Option<i64>` + `reenvios: u32`. Todos los `Option`/contadores con `#[serde(default)]` para compat de deserialización de mensajes viejos.
- Añadir `struct PeticionConfirmar { ids: Vec<i64>, estado: EstadoMensaje }` y `PeticionReenviar { msg_id: i64 }`, `PeticionHistorial { id: String, desde: Option<i64>, estado: Option<EstadoMensaje> }`.

**Verificar:** `cargo build -p peers-core` compila. Test unitario: serializar/deserializar un `Mensaje` con estado `Procesado` y campos timbrados roundtrip OK; deserializar un JSON viejo (con `entregado:true`, sin los nuevos campos) NO debe romper (serde default). `cargo test -p peers-core`.

---

## T2 — Bandeja ZSET no-destructiva (peek) en el store  [R1.1, R1.5]

**Archivos:** `crates/peers-broker/src/store.rs` (`encolar_mensaje`, `recibir_mensajes`), `crates/peers-core/src/almacen.rs` (firmas si cambian)

**Hacer:**
- `encolar_mensaje`: en vez de `RPUSH cprs:mensajes:{id}`, hacer `ZADD cprs:bandeja:{para_id} {msgseq} {json}` (score = msgseq) Y guardar el estado/HASH del mensaje en `cprs:msg:{msgseq}` (HSET con estado=`enviado`, enviado_en=ahora). El mensaje nace `Enviado`.
- `recibir_mensajes(id)`: **peek** — `ZRANGEBYSCORE cprs:bandeja:{id} -inf +inf`, deserializa, filtra los que están en estado `Enviado` o `Entregado`/`Leido` pero NO `Procesado`. Devuelve esos mensajes **sin borrar el ZSET**. (El marcado a `Entregado` lo hace T3 vía transición idempotente, llamado por el handler.)
- NO borrar nada aquí. El borrado de la bandeja activa será al confirmar `Procesado` (T3).

**Verificar:** test de integración Redis (patrón `redis_integracion.rs`): encolar 2 mensajes, `recibir` dos veces seguidas → ambas devuelven los 2 (no se borran). `cargo test -p peers-broker --test redis_integracion`.

---

## T3 — Transiciones de estado + idempotencia + borrado al Procesado  [R1.2, R1.3, R1.5]

**Archivos:** `crates/peers-core/src/almacen.rs` (trait), `crates/peers-broker/src/store.rs` (impl Redis), `crates/peers-broker/src/db.rs` (impl SQLite, `#[cfg(feature="sqlite")]`)

**Hacer:**
- Trait `Almacen`: añadir `async fn transicionar_mensaje(&self, msg_id: i64, nuevo: EstadoMensaje, ahora: &str) -> anyhow::Result<bool>` (devuelve `true` si transicionó, `false` si era idempotente/no-op).
- Redis impl: `HGET cprs:msg:{id} estado`; aplicar la transición solo si avanza (Enviado→Entregado→Leido→Procesado; o →Fallido/DeadLetter). Timbrar con **HSETNX** el campo de tiempo correspondiente (`entregado_en`/`leido_en`/`procesado_en`) → idempotente: timbra solo la primera vez (R1.3). Actualizar `estado` en el HASH y reescribir el JSON en el ZSET. Al llegar a `Procesado`: `ZREM cprs:bandeja:{id_destino} {miembro}` (sale de la bandeja activa, R1.5) pero el HASH `cprs:msg:{id}` y el historial (T4) persisten.
- SQLite impl: equivalente con `UPDATE mensajes SET estado=?, *_en=COALESCE(*_en, ?) WHERE id=?` (COALESCE = el HSETNX de SQLite).
- `incrementar_intentos(msg_id)` helper si hace falta para T-futuras (replay/DLQ).

**Verificar:** test: transicionar a `Entregado` dos veces → segunda devuelve `false` y `entregado_en` no cambia (idempotencia). Transicionar a `Procesado` → el mensaje sale de la bandeja (`recibir` ya no lo devuelve) pero `cprs:msg:{id}` sigue existiendo. Compila con y sin `--features sqlite`. `cargo test -p peers-broker` (ambas features).

---

## T4 — Historial durable + eliminar KEYS O(n)  [R1.6, R2.1]

**Archivos:** `crates/peers-broker/src/store.rs`, `crates/peers-core/src/almacen.rs`

**Hacer:**
- Al encolar (T2) o al transicionar, copiar/actualizar el mensaje en `cprs:historial:{para_id}` = ZSET (score=msgseq). `transicionar_mensaje` actualiza el estado también aquí.
- Trait: `async fn historial(&self, id: &str, desde: Option<i64>, estado: Option<EstadoMensaje>) -> anyhow::Result<Vec<Mensaje>>` (ZRANGEBYSCORE + filtro opcional por estado).
- Retención: en la limpieza periódica de 30s (`main.rs`), `ZREMRANGEBYRANK cprs:historial:{id} 0 -(N+1)` para retener los últimos N (ej. 500). Constante `RETENCION_HISTORIAL: usize = 500` en peers-core.
- **Eliminar el `KEYS cprs:outbox:*`** (store.rs:299): el outbox se subsume en la bandeja; donde se iteraban claves por `KEYS`, usar el SET `cprs:instancias` ya existente (igual que `listar_ids`) o indexar por `para_id` conocido. Grep `KEYS` debe quedar vacío en el store de prod.

**Verificar:** `historial(id)` devuelve mensajes incluso tras `Procesado`. `grep -rn '"KEYS"\|\.keys(' crates/peers-broker/src/store.rs` → sin resultados. Test de retención: encolar >500, el historial recorta a 500. `cargo test -p peers-broker`.

---

## T5 — Endpoint /confirmar + handlers del broker  [R1.4 lado broker]

**Archivos:** `crates/peers-broker/src/main.rs`

**Hacer:**
- Handler `async fn confirmar(State, Json<PeticionConfirmar>) -> Result<Json<RespuestaOk>, ErrorApp>`: por cada `id`, `transicionar_mensaje(id, p.estado, ahora_iso())`. Registrar en `rutas_protegidas` (bajo token): `POST /confirmar`.
- En el handler `recibir`, tras devolver los mensajes, NO transicionar (el cliente confirma explícitamente vía /confirmar para distinguir entregado real de leído). El estado a `Entregado` lo marca el cliente al confirmar (R1.4).

**Verificar:** `curl POST /confirmar` (con token) transiciona; sin token → 401. `cargo build -p peers-broker`. Recargar LaunchAgent y probar el endpoint vivo en :7899.

---

## T6 — Cliente: ACK real del flush + reportar estados  [R1.3, R1.4 lado client]

**Archivos:** `crates/peers-client/src/mcp.rs` (`enviar_json`, `empujar_canal`), `crates/peers-client/src/main.rs` (`lanzar_recepcion`), `crates/peers-client/src/broker.rs` (método `confirmar`)

**Hacer:**
- `enviar_json`: devolver `bool` (éxito real del `flush().await`, hoy ignorado con `let _`). `empujar_canal`: devolver `bool` propagando ese éxito.
- `ClienteBroker`: método `async fn confirmar(&self, ids: &[i64], estado: EstadoMensaje) -> Result<RespuestaOk>` → POST /confirmar (con token).
- `lanzar_recepcion`: mantener un `HashSet<i64>` en memoria (ventana ~500) de msg_ids ya empujados (idempotencia cliente, R1.3). Por cada mensaje recibido cuyo id NO esté en el set: empujar al canal; si `empujar_canal` devolvió `true`, añadir al set y `confirmar([id], Leido)`. Si devolvió `false` (flush falló), NO confirmar → se reintenta en el próximo ciclo.

**Verificar:** lanzar 2 peers reales (patrón de pruebas E2E de esta sesión), enviar A→B: B recibe el `<channel>` UNA vez (no en bucle aunque el peek no borre). El estado en el broker pasa a `Leido`. Verificable con `GET /admin/historial?id=B`.

---

## T7 — Broker: /admin/historial + /admin/reenviar  [R2.2, R2.3]

**Archivos:** `crates/peers-broker/src/main.rs`, `crates/peers-core/src/lib.rs` (respuestas)

**Hacer:**
- `GET /admin/historial` (query `id`, `desde?`, `estado?`) → `Json<Vec<Mensaje>>` vía `almacen.historial(...)`. En `rutas_protegidas`.
- `POST /admin/reenviar {msg_id}`: leer el mensaje del historial, crear uno nuevo con `msgseq` fresco, `estado=Enviado`, `reenviado_de=Some(msg_id)`, `reenvios = original.reenvios+1`, `encolar` en la bandeja del `para_id` original. Devolver el nuevo `msg_id`.

**Verificar:** `GET /admin/historial?id=X` lista con estados; `POST /admin/reenviar {msg_id}` → el destinatario recibe un nuevo mensaje con `reenviado_de` seteado. curl vivo en :7899. `cargo build`.

---

## T8 — TUI: pantalla Trazabilidad + reenvío  [R2.4]

**Archivos:** `crates/peers-tui/src/app.rs` (enum `Pantalla`), `crates/peers-tui/src/ui/trazabilidad.rs` (nuevo), `crates/peers-tui/src/ui/mod.rs`, `crates/peers-tui/src/cliente.rs` (métodos historial/reenviar), `crates/peers-tui/src/main.rs` (teclas)

**Hacer:**
- `Pantalla`: añadir `Trazabilidad` (tecla `6`). `ClienteAdmin`: `historial(id)` (GET /admin/historial) y `reenviar(msg_id)` (POST /admin/reenviar).
- `ui/trazabilidad.rs`: por peer seleccionado, tabla cronológica del historial: `de_id`, texto recortado, **estado coloreado** (○ enviado/gris, ◑ entregado-leído/amarillo, ● procesado/verde, ✕ fallido-dlq/rojo), timestamps. `Enter` abre timeline completo de un mensaje; `r` reenvía el seleccionado (llama `reenviar`).
- Manejar broker offline/401 igual que el resto (banner, no crash).

**Verificar:** `cargo build -p peers-tui`. Abrir `claudepeers --tui`, pantalla 6: ver el historial con estados coloreados; pulsar `r` sobre un mensaje → reenviado (aparece nuevo en el historial). Tests puros de formato de fila/color por estado.

---

## Cierre

- `cargo test --workspace` verde (con y sin `--features sqlite`).
- Recargar LaunchAgent (broker nuevo) + bump versión del plugin + recompilar binarios Mac/Linux del plugin/dist.
- Commit por fase (Fase 1 = T1-T6, Fase 2 = T7-T8), jornada en el cuerpo, sin Co-Authored-By.
- Actualizar el `.notebook/` y la memoria si surge algo no obvio.
