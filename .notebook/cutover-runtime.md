# Runtime real del Mac de Max (estado del cutover)

#cutover #runtime

## Qué corre hoy (verificado 2026-06-29 ~10:00 España)

- **Broker TS vivo**: `bun` PID 9823 en `localhost:7899` con ~5 conexiones (terminales + 1 `ssh`
  = peer remoto por túnel). Usa SQLite `~/.claude-peers.db`.
- **Redis vivo**: PID 3656 en `localhost:6379` → el broker Rust (default Redis) arranca sin más.
- **bun instalado**: `~/.bun/bin/bun` v1.3.11.
- **Rust**: cargo 1.94.1. Build release verde en ~40s (`target/release/{peers-broker,peers-client}`,
  ~2.7MB y ~2.2MB, LTO fat + strip).

## Dónde está la config MCP del peers (CLAVE para el cutover)

`~/.claude.json` → `mcpServers.claude-peers`:
```json
{ "type":"stdio", "command":"bun", "args":["/Users/maxmeireles/claude-peers-mcp/server.ts"], "env":{} }
```
→ Es scope USER (vale para todas las sesiones). El cutover cambia `command`/`args` a
`peers-client` + `--id`. CUIDADO: tocar este archivo es config sagrada → cambio explícito,
anunciado, con backup.

## Cómo conviven TS y Rust SIN chocar durante pruebas

- Broker Rust de prueba en **otro puerto** (usé 7988) → no toca el 7899 del TS.
- Almacenes distintos (SQLite del TS vs Redis `cprs:` del Rust) → cero interferencia de datos.
- Limpieza de prueba: `redis-cli --scan --pattern 'cprs:*' | xargs redis-cli del` (NO afecta al TS).

## Síntoma que confirma la misión (reportado por Max)

Max tiene 4 terminales Claude Code abiertos pero "ningún peer respondió" y `list_peers` solo
mostró 2. → Es el bug del id aleatorio del TS (al reiniciar pierde la cola). El Rust con `--id`
estable + Redis durable lo resuelve. Es la pendencia que el cutover viene a cerrar.
