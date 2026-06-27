# claude-peers-rs

Red de descubrimiento y mensajería entre instancias de Claude Code, en Rust.
Clon refactorizado de [claude-peers](https://github.com/m4xjunior/claude-peers-mcp) (TS/bun),
para uso personal. Dos binarios distribuibles sin dependencias externas: un `scp` y corre
en cualquier servidor.

## Por qué Rust

- **Binario sin deps**: SQLite va embebido. No requiere bun/node ni instalar nada.
- **Sin los 3 errores de diseño del original**:
  1. **ID estable por papel** (`--id`) — al reiniciar una instancia con el mismo id,
     hereda su fila de mensajes pendientes (en el TS el id era aleatorio y se perdía).
  2. **Liveness por latido** (no por PID) — funciona cross-host: una instancia está viva
     si fue vista en los últimos 45s, sin depender de un PID local.
  3. **URL del broker configurable** (`--broker-url`) — el cliente apunta a donde quiera
     (broker remoto vía túnel/forward), no a `127.0.0.1` fijo.

## Arquitectura

```
┌──────────────┐   HTTP    ┌──────────────┐   stdio MCP   ┌─────────────┐
│ peers-client │ ────────► │ peers-broker │               │ Claude Code │
│  (MCP stdio) │ ◄──────── │ (axum+SQLite)│ ◄───────────► │  (sesión)   │
└──────────────┘           └──────────────┘   notif canal  └─────────────┘
```

- **`peers-broker`** — daemon HTTP (axum + tokio + SQLite). Registra instancias, rutea
  mensajes, mantiene liveness. Un solo broker por red.
- **`peers-client`** — servidor MCP stdio, uno por instancia de Claude Code. Expone las
  tools, registra la instancia y empuja los mensajes entrantes a la sesión como canal.
- **`peers-core`** — tipos compartidos del protocolo (única fuente de verdad del wire format).

Todo el sistema está en español (columnas, structs, protocolo, tools). La única excepción
son 4 claves del push (`from_id`, `from_summary`, `from_cwd`, `sent_at`): las fija el harness
de Claude Code para reconocer el canal, y se respetan tal cual.

## Uso

### 1. Levantar el broker

```bash
peers-broker                          # localhost:7899, db en ~/.claude-peers.db
peers-broker --puerto 7899 --host 0.0.0.0   # expuesto (detrás de túnel/forward)
peers-broker --db /ruta/red.db
```

### 2. Registrar el cliente en Claude Code (`.mcp.json`)

```json
{
  "claude-peers-rs": {
    "command": "/ruta/a/peers-client",
    "args": ["--id", "miNodo", "--broker-url", "http://127.0.0.1:7899"]
  }
}
```

`--id` (o `CLAUDE_PEERS_ID`) es el papel estable de la instancia. Sin él, se asigna uno
aleatorio (y se pierde la herencia de fila al reiniciar).

## Tools MCP

| Tool | Qué hace |
|------|----------|
| `listar_instancias` | Lista otras instancias (alcance: `maquina` / `directorio` / `repo`) |
| `enviar_mensaje` | Envía a otra instancia por su id (push inmediato) |
| `definir_resumen` | Fija tu resumen visible a las demás |
| `revisar_mensajes` | Respaldo manual (normalmente llegan solos por canal) |

## Configuración

| Variable / flag | Broker | Cliente | Defecto |
|-----------------|:------:|:-------:|---------|
| `--puerto` / `CLAUDE_PEERS_PORT` | ✓ | ✓ | 7899 |
| `--host` / `CLAUDE_PEERS_HOST` | ✓ | — | 127.0.0.1 |
| `--db` / `CLAUDE_PEERS_DB` | ✓ | — | `~/.claude-peers.db` |
| `--broker-url` / `CLAUDE_PEERS_BROKER_URL` | — | ✓ | `http://127.0.0.1:<puerto>` |
| `--id` / `CLAUDE_PEERS_ID` | — | ✓ | (aleatorio) |

## Compilar

```bash
cargo build --release        # target/release/{peers-broker,peers-client}
cargo test                   # suite de la lógica del broker
```

## Distribución

```bash
scp target/release/peers-broker servidor:/usr/local/bin/
# en el servidor: systemd para el broker, .mcp.json apuntando al client
```
