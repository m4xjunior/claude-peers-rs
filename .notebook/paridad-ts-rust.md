# Paridad TS ↔ Rust (qué cambia en el cutover)

#cutover #protocolo

## Los dos sistemas son mundos separados (NO interoperan)

- **Client Rust** habla con **broker Rust** por rutas en español (`/registrar`, `/listar`,
  `/enviar`, `/recibir`…). Ver `crates/peers-client/src/broker.rs`.
- **server TS** habla con **broker TS** por rutas en inglés (`/register`, `/list-peers`,
  `/send-message`, `/poll-messages`…). Ver `claude-peers-mcp/broker.ts`.
- → No se puede mezclar client Rust con broker TS ni viceversa. El cutover es "apagar uno,
  encender el otro", no migración gradual de un mismo broker.

## Divergencias que importan para el cutover

| Aspecto | TS | Rust | Impacto |
|---|---|---|---|
| Tools | `list_peers`,`send_message`,`set_summary`,`check_messages` (EN) | `listar_instancias`,`enviar_mensaje`,`definir_resumen`,`revisar_mensajes` (ES) | Max eligió MANTENER ES (2026-06-29) |
| serverInfo.name | `claude-peers` | `claude-peers-rs` (`peers-client/src/mcp.rs:167`) | Es el `source=` del `<channel>` |
| Almacén | SQLite `~/.claude-peers.db` | **Redis** `cprs:` namespace (decisión Max 27/06, ver COORDENACAO.md L60) | Mensajes/peers vivos NO migran solos |
| ID peer | aleatorio (se pierde en restart) | estable por `--id` (hereda cola) | Es EL fix del "no recibo mensajes" |

## Contrato del push `<channel>` — IDÉNTICO en ambos (verificado E2E 2026-06-29)

`peers-client/src/mcp.rs:empujar_canal()` emite `notifications/claude/channel` con
`params.content` + `params.meta{from_id,from_summary,from_cwd,sent_at}` — **4 claves en INGLÉS**
(las fija el harness de Claude, no son negociables). Prueba E2E real confirmó que el push sale
con ese formato exacto. Esta es la única parte en EN de todo el proyecto.

## ⚠️ Inconsistencia de docs detectada (no bloqueante)

`README.md` (L24,L28) y `docs/stack-decisions.md` dicen "SQLite embebido, NO Redis". Pero el
código real ya pivotó a **Redis por defecto** (Cargo.toml `deadpool-redis`; `peers-broker/src/main.rs:413`
`construir_almacen` → AlmacenRedis; COORDENACAO.md L60 decisión de Max). El README quedó
desactualizado tras el pivote. SQLite sigue disponible tras `--features sqlite`.
→ Pendiente: actualizar README para reflejar Redis-default.
