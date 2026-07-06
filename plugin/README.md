# claude-peers-rs — plugin de Claude Code

Instala la red de peers (descubrimiento + mensajería entre instancias de Claude Code) como
**plugin nativo de Claude Code**. El binario va incluido: instalar el plugin registra el MCP
y arranca el broker solo — **sin compilar, sin `bun`, sin `mcp add` manual**.

## Instalación (un comando, desde cualquier máquina con Claude Code)

```
/plugin marketplace add m4xjunior/claude-peers-rs
/plugin install claude-peers-rs
```

Eso es todo. Al iniciar Claude:
- El MCP `claude-peers` queda registrado (apunta al binario incluido, elegido por SO/arch).
- El hook `SessionStart` asegura que el broker esté vivo (lo arranca si no responde).
- La equipe aparece: usa `listar_instancias`, `enviar_mensaje`, etc.

> Para el push `<channel>` (mensajes que aparecen solos en la sesión) Claude debe arrancar con
> `--dangerously-load-development-channels server:claude-peers`. Sin el flag, las tools funcionan
> pero los mensajes no se inyectan solos. (Recurso experimental de Claude Code.)

## Plataformas incluidas

| SO / arch | Binario |
|-----------|---------|
| macOS Apple Silicon | `peers-{client,broker}-darwin-arm64` |
| Linux x86_64 | `peers-{client,broker}-linux-x64` (estático musl) |
| Windows x86_64 | `peers-client-windows-x64.exe` (cliente; ver nota sobre el broker) |

**Windows:** el `peers-client.exe` va incluido (cross-compilado con `x86_64-pc-windows-gnu`). El
`peers-broker.exe` **no** se incluye todavía: el broker usa `libc::kill` (POSIX) para la
anti-colisión de ids, que no existe en Windows — portarlo requiere `#[cfg(windows)]` + la Windows
API. Un usuario Windows opera como **cliente** de un broker de la red (setear `CLAUDE_PEERS_BROKER_URL`
o estar en la misma red); no levanta broker local.

### El launcher: por qué Node (decisión 2026-07-06)

El MCP se registra con `command: "node"` + `bin/peers-client-launcher.js`, un shim MÍNIMO que
detecta `process.platform`/`process.arch` y ejecuta el binario nativo correcto. **Decisión explícita
de Max:** se acepta Node como capa de arranque (cambio de postura: hasta hoy el plugin era binario
puro). Razón: Claude Code ejecuta el `command` de un MCP stdio en *exec form* (directo, sin shell) y
**no** soporta configuración condicional por plataforma ni resuelve extensiones — así que un `command`
bash sirve a Mac/Linux pero **nunca** a Windows nativo (sin bash garantizado). Un shim de Node es el
patrón idiomático de la doc de Claude Code (`mcp-quickstart` usa `npx`) y la única vía cero-fricción
para las tres plataformas con un solo `command`. **El shim no tiene lógica de negocio** — solo el
switch de plataforma + `spawn` del binario; toda la lógica del peer vive en el binario Rust.

El hook `SessionStart` (`hooks/asegurar-broker.js`) también es Node por el mismo motivo: en Windows
skipea el arranque del broker local (no hay `.exe` del broker); con `CLAUDE_PEERS_BROKER_URL` seteado
tampoco levanta nada. Siempre termina en exit 0 (best-effort, no bloquea la sesión).

Para añadir arch (Linux ARM, Intel Mac): compilar el target y dejar el binario en `bin/` con el
nombre `peers-client-<os>-<arch>` que el shim espera (ver el mapa en `peers-client-launcher.js`).

## Configuración (variables de entorno)

| Variable | Efecto |
|----------|--------|
| `CLAUDE_PEERS_ID` | id de rol estable (si no, se deriva de la carpeta) |
| `CLAUDE_PEERS_PORT` | puerto del broker (defecto 7899) |
| `CLAUDE_PEERS_BROKER_URL` | apunta a un broker remoto (no levanta uno local) |
| `CLAUDE_PEERS_TOKEN` | token de acceso al broker (si el broker lo exige) |

### Config por archivo (recomendado en Windows, o si las env vars no se propagan)

En Windows las variables de entorno del SO a veces NO llegan al proceso del cliente. Vía robusta que
**no** depende del entorno: crea el archivo `~/.claude/claude-peers.json` (en Windows:
`C:\Users\<usuario>\.claude\claude-peers.json`) con:

```json
{
  "broker_url": "http://10.0.1.60:7899",
  "token": "el-token-del-broker"
}
```

El launcher lo lee y pasa `--broker-url`/`--token` explícitos al binario. **Sobrevive a
`claude update plugin`** (vive fuera del plugin). Precedencia: args del `.mcp.json` > este archivo >
variables de entorno. Diagnóstico de arranque en `<tmp>/claude-peers-launcher.log`
(en Windows: `%TEMP%\claude-peers-launcher.log`).
| `CLAUDE_PEERS_REDIS_URL` | Redis del broker (defecto redis://127.0.0.1:6379) |

## Servidor 24/7

El hook arranca el broker bajo demanda — suficiente para uso normal. Para un servidor
siempre-vivo (arranque al boot + autoreinicio), usa el `install.sh` de la raíz del repo
(systemd en Linux / launchd en Mac). Ver `docs/distribucion.md`.

## Requisito de persistencia

El broker usa Redis por defecto. Si no hay Redis, recompila con `--features sqlite` (binario
100% autocontenido) y reemplaza los binarios del plugin.
