# claude-peers-rs

Red de descubrimiento y mensajería entre instancias de Claude Code, en Rust.
Clon refactorizado de [claude-peers](https://github.com/m4xjunior/claude-peers-mcp) (TS/bun),
para uso personal. Dos binarios distribuibles sin dependencias externas: un `scp` y corre
en cualquier servidor.

> **Licencia:** software propietario privado (uso personal). No es código abierto.
> Ver [LICENSE](LICENSE). **Troubleshooting:** si los mensajes no llegan, consulta
> [docs/troubleshooting.md](docs/troubleshooting.md) (los 6 casos comunes con su fix).
> **Windows:** para conectar una máquina Windows (cross-compilar el `.exe` + registrar el
> MCP en PowerShell), consulta [docs/windows.md](docs/windows.md).

## Por qué Rust

- **Binario sin bun/node**: un solo ejecutable; `scp` y corre. Backend Redis por defecto
  (durabilidad + cross-host); con `--features sqlite` el broker es 100% autocontenido (sin red).
- **Sin los 3 errores de diseño del original**:
  1. **ID estable** — sin `--id`, lo deriva del nombre de la carpeta automáticamente
     (o de `CLAUDE_PEERS_ID`); al reiniciar una instancia con el mismo id hereda su fila
     de mensajes pendientes (en el TS el id era aleatorio y se perdía).
  2. **Liveness por latido** (no por PID) — funciona cross-host: una instancia está viva
     si fue vista en los últimos 45s, sin depender de un PID local.
  3. **URL del broker configurable** (`--broker-url`) — el cliente apunta a donde quiera
     (broker remoto vía túnel/forward), no a `127.0.0.1` fijo.

## Arquitectura

```
┌──────────────┐   HTTP    ┌──────────────┐   stdio MCP   ┌─────────────┐
│ peers-client │ ────────► │ peers-broker │               │ Claude Code │
│  (MCP stdio) │ ◄──────── │ (axum+Redis) │ ◄───────────► │  (sesión)   │
└──────────────┘           └──────────────┘   notif canal  └─────────────┘
```

- **`peers-broker`** — daemon HTTP (axum + tokio). Persistencia tras el trait `Almacen`:
  Redis por defecto, SQLite con `--features sqlite`. Registra instancias, rutea mensajes,
  mantiene liveness, mide la jornada. Un solo broker por red.
- **`peers-client`** — servidor MCP stdio, uno por instancia de Claude Code. Expone las
  tools, registra la instancia y empuja los mensajes entrantes a la sesión como canal.
- **`peers-core`** — tipos compartidos del protocolo (única fuente de verdad del wire format).

Todo el sistema está en español (columnas, structs, protocolo, tools). La única excepción
son 4 claves del push (`from_id`, `from_summary`, `from_cwd`, `sent_at`): las fija el harness
de Claude Code para reconocer el canal, y se respetan tal cual.

## Uso

Conectar una instancia a la red exige **tres** piezas; si falta cualquiera, el mensaje no
aparece como `<channel>` en la sesión (ver [troubleshooting](docs/troubleshooting.md)):

1. El **broker** corriendo (uno por red).
2. El **MCP `claude-peers`** registrado y conectado en Claude Code.
3. El **flag de canal** al arrancar `claude` — sin él el push no se renderiza.

### 1. Levantar el broker

```bash
peers-broker                                 # localhost:7899, db en ~/.claude-peers.db
peers-broker --puerto 7899 --host 0.0.0.0    # expuesto en red (LAN/túnel/forward)
peers-broker --host 0.0.0.0 --token <secreto>  # con auth (obligatorio si expones en red)
```

Con `--token` (o `CLAUDE_PEERS_TOKEN`), el broker exige el header `X-Peers-Token` en todas
las rutas salvo `/salud`. Sin token solo es seguro en loopback.

### 2. Registrar el MCP `claude-peers` (scope user / global)

La forma robusta y plug-and-play (`-s user` = visible en cualquier directorio). **El nombre
del MCP debe ser exactamente `claude-peers`** (lo exige el flag del paso 3):

```bash
claude mcp add claude-peers -s user \
  -e CLAUDE_PEERS_BROKER_URL=http://127.0.0.1:7899 \
  -e CLAUDE_PEERS_TOKEN=<secreto> \
  -- /ruta/a/peers-client
# verificar:
claude mcp list | grep claude-peers   # → ✔ Connected
```

> En Claude Code ≥ 2.1.x los flags son `-s` (scope) y `-e` (env). Versiones viejas usaban
> `--scope`/`--env`. El instalador [`install.sh`](install.sh) detecta cuál aplica.

`--id` (o `CLAUDE_PEERS_ID`) es el papel estable de la instancia. Sin él, se deriva del
nombre de la carpeta. Reiniciar con el mismo id hereda la fila de mensajes pendientes.

### 3. Arrancar Claude con el flag de canal (OBLIGATORIO para el push)

```bash
claude --dangerously-load-development-channels server:claude-peers
```

Sin este flag los mensajes **llegan al `peers-client` pero NO se renderizan** como `<channel>`
en la sesión (síntoma: "no recibo nada"). Práctico envolverlo en una función del shell:

```bash
claude() { command claude --dangerously-load-development-channels server:claude-peers "$@"; }
```

### 4. Red cross-host (broker central + peers remotos)

Topología recomendada: **un broker central** (p. ej. tu Mac) y los servidores apuntan a él.

```bash
# En cada servidor, ANTES de arrancar claude (el MCP lee el entorno una sola vez al lanzarse):
export CLAUDE_PEERS_BROKER_URL=http://<ip-broker>:7899
export CLAUDE_PEERS_TOKEN=<secreto>
claude --dangerously-load-development-channels server:claude-peers
```

El broker distingue peers de hosts distintos por su `hostname`, de modo que dos sesiones
remotas del mismo directorio no colisionan de id. Guía completa de despliegue LAN en
[docs/distribucion.md](docs/distribucion.md).

### TUI de control

```bash
claudepeers --tui   # panel: peers, trazabilidad, jornada, tareas, alertas, redis
```

## Tools MCP

| Tool | Qué hace |
|------|----------|
| `listar_instancias` | Lista otras instancias (alcance: `maquina` / `directorio` / `repo`) |
| `enviar_mensaje` | Envía a otra instancia por su id (push inmediato) |
| `definir_resumen` | Fija tu resumen visible a las demás |
| `revisar_mensajes` | Respaldo manual (normalmente llegan solos por canal) |
| `crear_tarea` | Crea una tarea con tu estimado; el broker mide el tiempo real |
| `reportar_tarea` | Añade una nota de progreso a una tarea abierta |
| `cerrar_tarea` | Cierra una tarea (el broker timbra el fin y aprende del estimado) |
| `listar_tareas` | Lista tus tareas con estimado / real / duración |
| `revisar_tareas` | Resumen de tus tareas abiertas |

## Configuración

| Variable / flag | Broker | Cliente | Defecto |
|-----------------|:------:|:-------:|---------|
| `--puerto` / `CLAUDE_PEERS_PORT` | ✓ | ✓ | 7899 |
| `--host` / `CLAUDE_PEERS_HOST` | ✓ | — | 127.0.0.1 |
| `--token` / `CLAUDE_PEERS_TOKEN` | ✓ | ✓ | (sin auth) |
| `--db` / `CLAUDE_PEERS_DB` | ✓ | — | `~/.claude-peers.db` |
| `--broker-url` / `CLAUDE_PEERS_BROKER_URL` | — | ✓ | `http://127.0.0.1:<puerto>` |
| `--id` / `CLAUDE_PEERS_ID` | — | ✓ | (derivado de la carpeta) |

El token del broker y el del cliente **deben coincidir**; si el broker exige token y el
cliente no lo manda, las llamadas devuelven `401`.

## Compilar

```bash
cargo build --release                    # target/release/{peers-broker,peers-client,peers-tui}
cargo build --release --features sqlite  # broker autocontenido (sin Redis)
cargo test --workspace                   # suite completa (broker, client, tui)
```

## Distribución

```bash
scp target/release/peers-broker servidor:/usr/local/bin/
# en el servidor: systemd para el broker, y registrar el MCP (ver "Uso" → paso 2).
```

El script [`install.sh`](install.sh) automatiza el registro del MCP (detectando la sintaxis
de `claude mcp add`), la verificación de conexión y la función `claude()` del shell.
