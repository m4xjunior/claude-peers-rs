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

## Receta B — Servidor Linux (otus u otro): UN comando con `install.sh`

El instalador `install.sh` hace TODO solo (idempotente): coloca binarios, levanta el broker
como servicio (systemd), registra el MCP en `~/.claude.json` y añade la función `claude()`.

**1. Compilar el binario portable (en tu Mac, una vez) y empaquetarlo:**
```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# Empaqueta binarios + instalador para enviar:
mkdir -p dist/bin
cp target/x86_64-unknown-linux-musl/release/peers-broker dist/bin/
cp target/x86_64-unknown-linux-musl/release/peers-client dist/bin/
cp install.sh dist/
```

**2. Copiar al servidor y ejecutar el instalador (1 comando allí):**
```bash
scp -r dist/* otus:/tmp/cprs/
ssh otus 'cd /tmp/cprs && ./install.sh'
```

Eso es todo. El servidor queda con el broker corriendo (arranque + autoreinicio), el MCP
registrado y la función `claude()` lista. El usuario abre `claude --dangerously-skip-permissions`
y la equipe aparece.

**Variantes por entorno (variables ante `./install.sh`):**
- `PREFIX=~/.local ./install.sh` — sin sudo (binarios en `~/.local/bin`).
- `CLAUDE_PEERS_REDIS_URL=redis://10.0.0.5:6379 ./install.sh` — Redis remoto.
- `CLAUDE_PEERS_BROKER_URL=https://peers.midominio.com ./install.sh` — NO levanta broker local;
  el client de este server apunta al broker central (ver Receta C).

> El instalador detecta el SO: en Mac usa launchd (igual que la Receta A), en Linux systemd.
> Re-ejecutarlo es seguro (no duplica la función shell ni rompe el servicio).

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

## Setup REAL de Max (LAN, 2026-06-29) — broker en el Mac, servers se conectan

**Broker central: el Mac de Max**, expuesto en la LAN con token.
- Mac: LaunchAgent con `--host 0.0.0.0 --puerto 7899` + `CLAUDE_PEERS_TOKEN=lexusfx-peers-2026`.
- IP del Mac en la LAN: **`10.0.1.60`** (verificada; cambia si cambia la red — re-verificar con
  `ifconfig | grep "inet "`).
- Client del Mac (`~/.claude.json`): `env.CLAUDE_PEERS_TOKEN=lexusfx-peers-2026` (apunta a
  `127.0.0.1:7899` por defecto, local).

**En cada servidor de la MISMA LAN** (vía el plugin o install.sh), el client apunta al Mac.
Lo más simple: exportar las variables en el entorno donde corre `claude` (p.ej. en el
`~/.bashrc`/`~/.zshrc` del server, junto a la función `claude()`):
```bash
export CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899
export CLAUDE_PEERS_TOKEN=lexusfx-peers-2026
```
El `peers-client` lee ambas (clap env). Reinicia el `claude` del server y `listar_instancias`
mostrará los peers del Mac; los mensajes cruzan en ambos sentidos.

> Requiere que el server alcance `10.0.1.60:7899` (misma LAN, sin firewall bloqueando). Probar
> desde el server: `curl -s -o /dev/null -w "%{http_code}\n" http://10.0.1.60:7899/salud` → 200.

## Por qué esto supera al TS

- El TS necesitaba `bun` instalado + `git clone` + `bun install` (falló en el ai-studio).
- Aquí es **1 binario** (`scp` y corre). Redis es la única dep, y se puede eliminar con
  `--features sqlite`. Verdadero plug&play.

## Flag de canal (obligatorio, recordatorio)

El push `<channel>` es un recurso experimental de Claude Code: solo se entrega si `claude`
arranca con `--dangerously-load-development-channels server:claude-peers`. Por eso la función
`claude()` del shell lo inyecta — sin ella, el broker está perfecto pero la equipe no "aparece".
