# Distribución plug&play — claude-peers-rs en servidores

> Cómo dejar la red de peers funcionando en un servidor (Linux) o en el Mac, de modo que
> al iniciar `claude` la equipe aparezca sola, sin instalar bun/node ni escribir flags.
> Es la VISIÓN: *"iniciar el claude en cualquier computador y tener la equipe"*.

## El modelo (2 piezas, 1 binario cada una)

```
┌──────────────┐   HTTP    ┌──────────────┐   stdio MCP   ┌─────────────┐
│ peers-client │ ────────► │ peers-broker │               │ Claude Code │
│  (MCP stdio) │ ◄──────── │ (axum+Redis) │ ◄───────────► │  (sesión)   │
└──────────────┘           └──────────────┘   push canal   └─────────────┘
 1 por sesión               1 por máquina/red
 (lo spawnea claude)        (servicio siempre-vivo)
```

- **`peers-broker`** — servicio que vive siempre (LaunchAgent en Mac / systemd en Linux).
- **`peers-client`** — lo arranca Claude Code solo, al leer `~/.claude.json`. No lo gestionas tú.

## Requisito único: Redis (o SQLite, sin red)

El broker por defecto usa **Redis** (`redis://127.0.0.1:6379`, namespace `cprs:`). En un server
que ya tiene Redis, cero trabajo. Si quieres **cero dependencias de red** (binario 100%
autocontenido), compila con SQLite:

```bash
cargo build --release --features sqlite
peers-broker --db /var/lib/claude-peers/red.db   # SQLite embebido, sin Redis
```

## Receta A — Mac (lo que está montado HOY, 2026-06-29)

1. **Binarios:** `target/release/{peers-broker,peers-client}` (este repo).
2. **Broker como servicio (LaunchAgent):** `~/Library/LaunchAgents/com.lexusfx.claude-peers.plist`
   → broker en `:7899`, `RunAtLoad` + `KeepAlive` (arranca al login, se reinicia si muere).
   - Cargar: `launchctl load ~/Library/LaunchAgents/com.lexusfx.claude-peers.plist`
   - Logs: `/tmp/claude-peers-broker.{out,err}.log`
3. **MCP en `~/.claude.json`** (scope user → vale en cualquier carpeta):
   ```json
   "claude-peers": { "type":"stdio",
     "command":"/Users/maxmeireles/claude-peers-rs/target/release/peers-client", "args":[], "env":{} }
   ```
4. **Función `claude()` en `~/.zshrc`** — inyecta el flag de canal automáticamente, así
   `claude --dangerously-skip-permissions` ya sale conectado. El id estable lo deriva el
   binario del nombre de la carpeta (o de `CLAUDE_PEERS_ID` si lo exportas).

## Receta B — Servidor Linux (otus u otro)

1. **Compilar portable** (estático musl, corre en cualquier Linux sin libc-version-hell):
   ```bash
   rustup target add x86_64-unknown-linux-musl
   cargo build --release --target x86_64-unknown-linux-musl
   # binarios: target/x86_64-unknown-linux-musl/release/{peers-broker,peers-client}
   ```
2. **Copiar al server:**
   ```bash
   scp target/x86_64-unknown-linux-musl/release/peers-* otus:/usr/local/bin/
   ```
3. **Broker como servicio (systemd):** crear `/etc/systemd/system/claude-peers.service`:
   ```ini
   [Unit]
   Description=claude-peers-rs broker
   After=network.target redis.service

   [Service]
   ExecStart=/usr/local/bin/peers-broker --puerto 7899
   Environment=CLAUDE_PEERS_REDIS_URL=redis://127.0.0.1:6379
   Restart=always
   RestartSec=2
   User=otus

   [Install]
   WantedBy=multi-user.target
   ```
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable --now claude-peers
   ```
4. **MCP del usuario** (`~/.claude.json` del server, o `claude mcp add --scope user`):
   ```bash
   claude mcp add --scope user --transport stdio claude-peers -- /usr/local/bin/peers-client
   ```
5. **Función `claude()`** equivalente en el `~/.bashrc`/`~/.zshrc` del server (igual que el Mac).

## Receta C — Red entre servidores (cross-host, vía túnel)

El broker es uno solo; los clients de otros servers le hablan por HTTP. La única dependencia
externa aceptada por la VISIÓN es la **red**, resuelta por túnel:

1. Broker en el server "central" escuchando en `0.0.0.0`:
   `peers-broker --host 0.0.0.0 --puerto 7899` (detrás de cloudflared/firewall).
2. Exponer por túnel: `cloudflared tunnel ... → http://127.0.0.1:7899` → `peers.tudominio.com`.
3. En cada server remoto, el client apunta al broker central:
   ```bash
   claude mcp add --scope user --transport stdio claude-peers -- \
     /usr/local/bin/peers-client --broker-url https://peers.tudominio.com
   ```
   (o `CLAUDE_PEERS_BROKER_URL=https://peers.tudominio.com`)
4. La liveness es por latido (no por PID) → funciona cross-host sin problemas.

## Por qué esto supera al TS

- El TS necesitaba `bun` instalado + `git clone` + `bun install` (falló en el ai-studio).
- Aquí es **1 binario** (`scp` y corre). Redis es la única dep, y se puede eliminar con
  `--features sqlite`. Verdadero plug&play.

## Flag de canal (obligatorio, recordatorio)

El push `<channel>` es un recurso experimental de Claude Code: solo se entrega si `claude`
arranca con `--dangerously-load-development-channels server:claude-peers`. Por eso la función
`claude()` del shell lo inyecta — sin ella, el broker está perfecto pero la equipe no "aparece".
