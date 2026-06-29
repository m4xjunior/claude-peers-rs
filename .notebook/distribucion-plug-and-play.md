# Distribución plug&play (cómo queda "claude y ya está conectado")

#distribucion #servers #cutover

## El requisito de Max (2026-06-29)

No quiere escribir flags ni CLAUDE_PEERS_ID. Quiere que `claude --dangerously-skip-permissions`
salga ya conectado a la red de peers. Dos piezas lo logran:

### 1. id estable AUTOMÁTICO (sin --id)
`peers-client/src/contexto.rs:id_desde_directorio()` deriva el id del nombre de la carpeta
(saneado a [a-z0-9-], minúsculas). `main.rs:id_efectivo` = `--id`/`CLAUDE_PEERS_ID` si vino,
si no el derivado. Se usa como `id_preferido` en registro + latido + re-registro.
- `claude-peers-rs/` → id `claude-peers-rs`; `/tmp` → `tmp`; override por `CLAUDE_PEERS_ID`.
- Antes registraba TODOS como `"instancia"` (colisión) — bug arreglado. Verificado E2E.

### 2. Flag de canal vía función shell (no env var en CC 2.1.195)
`~/.zshrc` define `claude() { command claude --dangerously-load-development-channels server:claude-peers "$@"; }`.
Inyecta el flag siempre. `command claude` = escape sin peers. El `--dangerously-skip-permissions`
lo añade Max al usar; la función NO lo fuerza (deja la decisión de permisos a Max).
Backup del .zshrc: `~/.zshrc.bak-cutover-*`. Validado con `zsh -n`.

## Servidores (receta completa en docs/distribucion.md)

- Linux: compilar musl estático → `scp` a /usr/local/bin → systemd para el broker
  (Restart=always) → `claude mcp add --scope user ... peers-client`.
- Cross-host: broker `--host 0.0.0.0` tras cloudflared; clients remotos con
  `--broker-url https://...`. Liveness por latido = funciona cross-host.
- Cero-deps real: `cargo build --features sqlite` + `peers-broker --db ...` elimina Redis.

## Estado del runtime tras cutover (Mac)
- Broker Rust :7899 vía LaunchAgent `com.lexusfx.claude-peers` (RunAtLoad+KeepAlive, PPID launchd).
- `~/.claude.json` claude-peers → peers-client Rust (args vacíos; id auto por carpeta).
- TS apagado. Backup config: `~/.claude.json.bak-cutover-*`.
- Pendiente de Max: reiniciar sus terminales con la función `claude()` nueva para entrar al Rust.
