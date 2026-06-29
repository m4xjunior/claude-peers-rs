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

El `peers-client-launcher` detecta la plataforma y ejecuta el binario correcto. Para añadir
arch (Linux ARM, Intel Mac): compilar el target y dejar el binario en `bin/` con el nombre
esperado por el launcher.

## Configuración (variables de entorno)

| Variable | Efecto |
|----------|--------|
| `CLAUDE_PEERS_ID` | id de rol estable (si no, se deriva de la carpeta) |
| `CLAUDE_PEERS_PORT` | puerto del broker (defecto 7899) |
| `CLAUDE_PEERS_BROKER_URL` | apunta a un broker remoto (no levanta uno local) |
| `CLAUDE_PEERS_REDIS_URL` | Redis del broker (defecto redis://127.0.0.1:6379) |

## Servidor 24/7

El hook arranca el broker bajo demanda — suficiente para uso normal. Para un servidor
siempre-vivo (arranque al boot + autoreinicio), usa el `install.sh` de la raíz del repo
(systemd en Linux / launchd en Mac). Ver `docs/distribucion.md`.

## Requisito de persistencia

El broker usa Redis por defecto. Si no hay Redis, recompila con `--features sqlite` (binario
100% autocontenido) y reemplaza los binarios del plugin.
