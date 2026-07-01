# Decisiones de Stack — claude-peers-rs

> **Autor:** Front-QA (peer `djnewbwj`, CLI en el Mac de Max) · **Fecha:** 27/06/2026
> **Fatia:** investigación de stack (`docs/stack-decisions.md`). No toca código de nadie.
> **Método:** documentación real vía context7 (`rmcp`, `octocrab`) + lectura del código ya
> existente (`Cargo.toml`, `README.md`, los 3 crates) + los principios de `VISAO.md`.
> **Criterio rector (de la propia VISIÓN):** ZERO dependencias externas · binario auto-contido ·
> mantenible. Aplico mi ojo de calidad: cada dependencia se evalúa por su **acoplamiento**
> (distancia + peso + volatilidad), no por popularidad.

---

## TL;DR (las 3 recomendaciones)

| Decisión | Recomendación | En una línea |
|----------|---------------|--------------|
| **1. Broker: Redis vs SQLite** | ❌ **NO Redis → SQLite embebido (ya implementado)** | Redis viola el principio nº1 (es servicio externo). SQLite bundled da la misma durabilidad sin deps. |
| **2. Client GitHub** | ✅ **`octocrab`** (con `rustls`, sin openssl) | Ciclo de issues tipado y completo; degradar a `reqwest` directo sería reinventar peor. |
| **3. MCP en Rust** | ✅ **`rmcp`** (SDK **oficial**, features mínimas) | Hay SDK oficial con server stdio; implementar JSON-RPC a mano sería deuda técnica injustificada. |

---

## ⚠️ HALLAZGO DE QA (leer antes de la decisión 1)

La tarea que me pasaron pedía *"investigar qué crate de Redis (deadpool-redis / fred / redis)"*.
Antes de elegir un crate, mi trabajo como QA es señalar una **incoherencia entre la VISIÓN y el
código** que ya existe:

- `VISAO.md` menciona conceptualmente *"Broker en Redis (no perder solicitud)"* — pero como
  **justificación del POR QUÉ** (durabilidad, outbox+ACK, sobrevivir reinicios), no como mandato
  de implementación.
- El **código ya pivotó a SQLite embebido**, y de forma deliberada:
  - `README.md`: *"Binario sin deps: **SQLite va embebido**. No requiere bun/node ni instalar nada."*
  - Arquitectura dibujada en el README: `peers-broker (axum + **SQLite**)`.
  - `crates/peers-broker/Cargo.toml` ya declara `rusqlite = { version = "0.32", features = ["bundled"] }`
    → SQLite compilado **dentro** del binario.
- El **principio nº1 de la VISIÓN** es literal: *"ZERO dependencias externas… el Max baja 1
  binario en cualquier PC y funciona. Auto-contido."*

**Redis es un servicio externo**: hay que instalarlo, levantarlo y mantenerlo aparte (o vía túnel).
Eso **contradice frontalmente** el principio nº1 y lo que el código ya construyó. Recomendar "el
mejor crate de Redis" sin señalar esto sería cumplir la letra de la tarea traicionando su intención.

Por eso la decisión 1 no es "qué crate de Redis", sino **"¿Redis debe existir aquí?"** — y la
respuesta fundamentada es **no**.

---

## Decisión 1 — Durabilidad del broker: **SQLite embebido, no Redis**

### El requisito real (lo que se quería de Redis)
De la VISIÓN, la durabilidad debe garantizar:
1. **Ninguna solicitud se pierde** entre peers (el bug que Max sintió: "no vi las mensajes llegar").
2. **Outbox + ACK**: si un peer reinicia a mitad de una tarea, ésta **sobrevive** y él la retoma.
3. **Jornada medida por el broker** (ponto real, no inventado) — persistir inicio/fin/duración.

Todo esto es **persistencia transaccional local**, no un requisito de Redis específicamente.

### Comparativa por el criterio de la VISIÓN (zero-deps / binario / mantenible)

| Dimensión | **SQLite embebido** (`rusqlite` bundled) | **Redis** (deadpool-redis / fred / redis) |
|-----------|------------------------------------------|--------------------------------------------|
| Dep externa | ❌ **Ninguna** — compila dentro del binario | ⚠️ **Servicio aparte** — instalar+correr Redis (o túnel) |
| "1 binario y corre" | ✅ Cumple el sueño de Max literal | ❌ Rompe: el PC nuevo necesitaría Redis primero |
| Durabilidad | ✅ WAL + transacciones ACID; outbox+ACK como filas | ✅ AOF/RDB + outbox por streams |
| Acoplamiento (coupling-analysis) | 🟢 Bajo: el broker posee su DB (misma "casa") | 🔴 Alto: dep de máxima distancia (otro proceso/red) + volátil |
| Concurrencia | 1 broker (el diseño dice "un solo broker por red") → SQLite sobra | Redis brilla con N escritores; **aquí no hay N brokers** |
| Operación | 🟢 Cero — archivo `.db` | 🔴 Otro daemon que monitorear, versionar, asegurar |

### Por qué SQLite gana **en este proyecto concreto**
- El README dice **"un solo broker por red"**. La razón clásica para Redis (varios procesos
  compartiendo estado) **no aplica**: hay un único broker que es dueño de su estado. Cuando hay
  un solo escritor, SQLite con WAL cubre durabilidad y concurrencia de lectores de sobra.
- El sueño es *"scp el binario y corre en cualquier servidor"* (textual en el README). Con Redis,
  el "cualquier servidor" deja de ser cierto: primero hay que aprovisionar Redis.
- Outbox+ACK no necesita Redis: es una tabla `mensajes(id, destino, payload, entregado_at)` +
  un `UPDATE … SET entregado_at` al recibir ACK. Reinicio del peer → relee las no-entregadas.

### Recomendación
**Mantener `rusqlite` con `features=["bundled"]` (ya está en el Cargo.toml) y NO introducir Redis.**
Si en el futuro aparece la necesidad real de N brokers/escala horizontal, la decisión se reabre —
y ahí `fred` sería el candidato (ver nota abajo). Pero hoy, Redis es complejidad sin requisito.

### Nota — SI algún día se necesita Redis (decisión congelada, no para ahora)
Por si Max ya lo cravó por un motivo fuera del doc, dejo la comparativa de crates hecha:
- **`fred`** → recomendado si hubiera Redis. Async-first (tokio), pool de conexiones integrado,
  reconnect/backoff y soporte cluster nativos. Es el más completo y mantenido como "todo en uno".
- **`redis-rs`** (crate `redis`) → el oficial de facto; sólido, pero el pooling se delega a
  `deadpool-redis`/`bb8` (dos deps en vez de una).
- **`deadpool-redis`** → **no es un cliente**, es un pool sobre `redis-rs`. Compararlo con `fred`
  es categoría equivocada: `deadpool-redis` = `redis` + pool; `fred` ya trae el suyo.
  → Si Redis: **`fred`** (una dep, todo incluido) > `redis` + `deadpool-redis` (dos deps).

---

## Decisión 2 — Client de GitHub Issues: **`octocrab`** (con `rustls`)

### Contexto
La VISIÓN quiere *"empresa de verdad"*: cada tarea → una issue, cada report → un comentario,
cerrar la tarea → cierra la issue. Es CRUD de issues + comentarios contra la API de GitHub.

### `octocrab` vs `reqwest` directo

| Criterio | **`octocrab`** | **`reqwest` directo (a mano)** |
|----------|----------------|--------------------------------|
| Issues lifecycle | ✅ Tipado y completo: `.create()`, `.create_comment()`, `.update(...).state(Closed)` | ⚠️ Reimplementar cada endpoint, paths, paginación, parseo |
| Auth | ✅ `.personal_token(...)` listo | ⚠️ Header `Authorization: Bearer` + `User-Agent` obligatorio a mano |
| Deps nativas | ✅ Soporta **`rustls`** (sin openssl) | ✅ Igual (ya usamos `reqwest` rustls en el client) |
| Mantenibilidad | 🟢 Alta: el wire-format de GitHub lo mantiene el crate | 🔴 Baja: cada cambio de la API GH es deuda nuestra |
| Peso | ⚠️ Trae su árbol (incluye un `reqwest` propio) | 🟢 Menor, pero a cambio de reescribir lógica |

### Evidencia (context7, docs reales de octocrab)
El ciclo exacto que pide la VISIÓN, ya cubierto:
```rust
// tarea → issue
let issue = octocrab.issues(owner, repo).create(titulo).body(cuerpo).send().await?;
// report → comentario
octocrab.issues(owner, repo).create_comment(issue.number, texto).await?;
// cerrar tarea → cierra issue
octocrab.issues(owner, repo).update(issue.number).state(models::IssueState::Closed).send().await?;
```
Auth con token + **`rustls`** (clave para zero-deps nativas):
```toml
octocrab = { version = "0.39", default-features = false, features = ["rustls"] }
```

### El punto de acoplamiento importante (coupling-analysis)
GitHub Issues debe ser una **dependencia que DEGRADA, no que bloquea**. Si GH está caído, el
broker NO puede dejar de rutear mensajes ni de medir jornada. → El módulo `github.rs` (fatia de
Aluísio Back) debe tratar el fallo de GH como *contract coupling* débil: error capturado, log, y
seguir. La fuente de verdad del trabajo es el broker (SQLite); GitHub es un **espejo** para que
Max observe, no el sistema de récord. (Esto encaja con la partición: `github.rs` aislado.)

### Recomendación
**`octocrab` con `default-features=false, features=["rustls"]`.** Reinventar con `reqwest` sería
más código, más frágil y sin ganar nada en el criterio zero-deps (ambos usan rustls).

---

## Decisión 3 — MCP en Rust: **`rmcp`** (SDK oficial), no implementar a mano

### Contexto
`peers-client` es un **servidor MCP stdio**, uno por instancia de Claude Code: expone las 4 tools
(`listar_instancias`, `enviar_mensaje`, `definir_resumen`, `revisar_mensajes`), hace el handshake
`initialize`/`capabilities`, y empuja los mensajes entrantes como `<channel>`. La pregunta: ¿hay
crate bueno para MCP stdio JSON-RPC, o se implementa a mano?

### Hay SDK **oficial**: `rmcp`
`rmcp` es el SDK oficial de Rust para el Model Context Protocol (alta reputación en context7,
async-first sobre tokio). Cubre justo lo que necesita `peers-client`:
- **Transport stdio** nativo: `rmcp::transport::io::stdio()` (feature `transport-io` + `server`).
- **Tools por macro** — el handshake y el ruteo JSON-RPC los genera el SDK:
```rust
#[tool_router(server_handler)]
impl PeersClient {
    #[tool(name = "enviar_mensaje", description = "Envía a otra instancia por id")]
    async fn enviar_mensaje(&self, Parameters(args): Parameters<EnviarArgs>) -> Json<Resultado> { … }
}
```

### `rmcp` vs implementar JSON-RPC a mano

| Criterio | **`rmcp` (oficial)** | **A mano (stdio + serde_json)** |
|----------|----------------------|----------------------------------|
| Handshake `initialize`/capabilities | ✅ Lo implementa el SDK al protocolo correcto | 🔴 Reimplementar el spec MCP entero y seguir sus cambios |
| Framing JSON-RPC stdio | ✅ Resuelto | 🔴 Leer/escribir frames, IDs, errores a mano |
| Correctitud vs Claude Code | ✅ Sigue el protocolo oficial → menos sorpresas | 🔴 Cada divergencia = un bug raro de "no aparece el canal" |
| Deps | ⚠️ Trae `schemars` + derivados (no servicio externo; todo compila dentro) | 🟢 Menos deps, pero a cambio de mantener el protocolo |
| Zero-deps externas | ✅ Cumple (es una **librería**, no un servicio) | ✅ Cumple |
| Mantenibilidad | 🟢 El protocolo lo mantiene Anthropic | 🔴 Deuda técnica nuestra a perpetuidad |

> Matiz importante para el criterio de la VISIÓN: "ZERO deps externas" se refiere a **servicios
> externos** (Redis, node) que rompen el "1 binario y corre". `rmcp` es una **dependencia de
> compilación** que se enlaza dentro del binario — NO viola el principio. Distinguir "servicio
> externo" de "crate enlazado" es clave para no rechazar `rmcp` por el motivo equivocado.

### Riesgo a vigilar (honestidad de QA)
`rmcp` aún evoluciona (pre-1.0). Mitigación: **fijar versión exacta** en `Cargo.toml` (no rango
abierto) y aislar el uso del SDK detrás de un módulo fino (`mcp.rs`) — si el SDK rompiera API, el
golpe queda contenido en un archivo, no esparcido. Aun así, implementar el protocolo a mano sería
**más** riesgo (todo el spec a cuestas), no menos.

### Recomendación
**`rmcp` con features mínimas (`server`, `transport-io`, `macros`), versión fijada.** Implementar
JSON-RPC a mano solo se justificaría si `rmcp` no soportara stdio — y lo soporta nativamente.

```toml
# crates/peers-client/Cargo.toml (sugerido — lo integra Jefin, dueño del Cargo.toml)
rmcp = { version = "=0.x.y", default-features = false, features = ["server", "transport-io", "macros"] }
```
> La versión exacta `=0.x.y` la fija Jefin al integrar (mirar la última estable en crates.io al momento).

---

## Resumen para el integrador (Jefin)

1. **Broker:** quedarse con `rusqlite` bundled (ya está). **No añadir Redis** — rompería el zero-deps
   y no hay requisito de N brokers. (Si Max ya cravó Redis: usar `fred`, no `deadpool-redis` suelto.)
2. **GitHub:** `octocrab` con `default-features=false, features=["rustls"]`. Módulo `github.rs` que
   **degrada** si GH está caído (GitHub es espejo observacional, no fuente de verdad).
3. **MCP:** `rmcp` (SDK oficial) con features mínimas y **versión fijada**, aislado en `mcp.rs`.

Todo alineado con el criterio de la VISIÓN: el único "servicio externo" que el sistema necesita es
la **red** (resuelta por túnel cloudflared, fatia de Aluísio Front). Redis no debería sumarse a esa
lista. `octocrab` y `rmcp` son crates enlazados (no servicios) → no violan "1 binario y corre".

---

*Revisión adversarial bienvenida (Claudio es el arquitecto/revisor). Si algún punto choca con una
decisión ya tomada por Max que no esté en VISAO.md/README.md/Cargo.toml, señálenlo y lo reviso —
mi recomendación se basa solo en la evidencia que esos archivos dan hoy.*
