#!/usr/bin/env bash
# Hook SessionStart del plugin claude-peers-rs.
# Asegura que el broker esté vivo ANTES de que el peers-client (MCP) intente conectarse.
# Si ya responde en el puerto, no hace nada. Si no, lo lanza desacoplado (nohup) para que
# sobreviva a la sesión. Esto reemplaza el "auto-launch" que el server.ts del TS hacía:
# así el plugin es autosuficiente sin necesitar systemd/launchd (aunque en un servidor
# 24/7 sigue siendo mejor el servicio del install.sh; el hook es el fallback universal).
set -euo pipefail

raiz="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
puerto="${CLAUDE_PEERS_PORT:-7899}"
url="${CLAUDE_PEERS_BROKER_URL:-http://127.0.0.1:$puerto}"

# Si apuntamos a un broker REMOTO, no levantamos nada local.
if [[ -n "${CLAUDE_PEERS_BROKER_URL:-}" ]]; then
  exit 0
fi

# ¿El broker ya responde? (curl breve; si no hay curl, asumimos que hay que arrancarlo)
if command -v curl >/dev/null && curl -s --max-time 1 "$url/salud" >/dev/null 2>&1; then
  exit 0
fi

# Arrancar el broker desacoplado de esta sesión.
nohup "$raiz/bin/peers-broker-launcher" --puerto "$puerto" \
  >/tmp/claude-peers-broker.out.log 2>/tmp/claude-peers-broker.err.log &
disown 2>/dev/null || true
exit 0
