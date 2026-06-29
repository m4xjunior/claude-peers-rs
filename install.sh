#!/usr/bin/env bash
# install.sh — instalador plug&play de claude-peers-rs.
#
# Un solo comando deja la red de peers lista en un servidor (Linux/systemd) o Mac
# (launchd): coloca los binarios, levanta el broker como servicio (arranque + autoreinicio),
# registra el MCP en la config del usuario y añade la función claude() que inyecta el flag
# de canal. Idempotente: re-ejecutarlo no rompe nada.
#
# Uso:
#   ./install.sh                 # instala desde los binarios de target/ (build local)
#   PREFIX=~/.local ./install.sh # binarios en ~/.local/bin en vez de /usr/local/bin
#   CLAUDE_PEERS_BROKER_URL=https://peers.midominio.com ./install.sh
#                                # NO levanta broker local; el client apunta al broker remoto
#   CLAUDE_PEERS_REDIS_URL=redis://10.0.0.5:6379 ./install.sh
#                                # broker local con Redis remoto
#
# Requisitos: los binarios peers-broker y peers-client ya compilados (cargo build --release,
# o --target x86_64-unknown-linux-musl para un Linux portable). El script los busca en:
#   1) ./bin/                          (junto al instalador, para distribución)
#   2) ./target/<triple>/release/      (build con target musl)
#   3) ./target/release/               (build nativo)

set -euo pipefail

# --- Configuración (override por entorno) ---
PREFIX="${PREFIX:-/usr/local}"
BIN_DIR="$PREFIX/bin"
PUERTO="${CLAUDE_PEERS_PORT:-7899}"
REDIS_URL="${CLAUDE_PEERS_REDIS_URL:-redis://127.0.0.1:6379}"
BROKER_URL="${CLAUDE_PEERS_BROKER_URL:-}"   # si está, NO se levanta broker local
SERVICIO="claude-peers"
LABEL="com.lexusfx.claude-peers"

log()  { printf '\033[36m[claude-peers]\033[0m %s\n' "$*"; }
warn() { printf '\033[33m[claude-peers] AVISO:\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[31m[claude-peers] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# --- 1. Localizar los binarios ---
DIR_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
encontrar_bin() {
  local nombre="$1"
  for cand in \
    "$DIR_SCRIPT/bin/$nombre" \
    "$DIR_SCRIPT"/target/*/release/"$nombre" \
    "$DIR_SCRIPT/target/release/$nombre"; do
    [[ -x "$cand" ]] && { echo "$cand"; return 0; }
  done
  return 1
}

SRC_BROKER="$(encontrar_bin peers-broker)" || err "no encontré el binario peers-broker (compílalo: cargo build --release)"
SRC_CLIENT="$(encontrar_bin peers-client)" || err "no encontré el binario peers-client (compílalo: cargo build --release)"
log "binarios: $SRC_BROKER / $SRC_CLIENT"

# --- 2. Instalar binarios en BIN_DIR ---
SUDO=""
if [[ ! -w "$BIN_DIR" ]] 2>/dev/null && [[ "$PREFIX" == /usr/* ]]; then
  command -v sudo >/dev/null && SUDO="sudo" || err "sin permiso de escritura en $BIN_DIR y sin sudo; usa PREFIX=~/.local"
fi
$SUDO mkdir -p "$BIN_DIR"
$SUDO install -m 0755 "$SRC_BROKER" "$BIN_DIR/peers-broker"
$SUDO install -m 0755 "$SRC_CLIENT" "$BIN_DIR/peers-client"
log "binarios instalados en $BIN_DIR"

DEST_BROKER="$BIN_DIR/peers-broker"
DEST_CLIENT="$BIN_DIR/peers-client"

SO="$(uname -s)"

# --- 3. Broker como servicio (solo si no apuntamos a un broker remoto) ---
if [[ -n "$BROKER_URL" ]]; then
  log "CLAUDE_PEERS_BROKER_URL=$BROKER_URL → no se levanta broker local; el client usará el remoto"
else
  case "$SO" in
    Linux)
      if command -v systemctl >/dev/null; then
        UNIT="/etc/systemd/system/$SERVICIO.service"
        log "instalando servicio systemd en $UNIT"
        $SUDO tee "$UNIT" >/dev/null <<EOF
[Unit]
Description=claude-peers-rs broker
After=network.target redis.service

[Service]
ExecStart=$DEST_BROKER --puerto $PUERTO
Environment=CLAUDE_PEERS_REDIS_URL=$REDIS_URL
Restart=always
RestartSec=2
User=$(id -un)

[Install]
WantedBy=multi-user.target
EOF
        $SUDO systemctl daemon-reload
        $SUDO systemctl enable --now "$SERVICIO"
        log "servicio $SERVICIO activo (systemctl status $SERVICIO)"
      else
        warn "no hay systemd; arranca el broker a mano: $DEST_BROKER --puerto $PUERTO &"
      fi
      ;;
    Darwin)
      PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
      log "instalando LaunchAgent en $PLIST"
      mkdir -p "$HOME/Library/LaunchAgents"
      cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$DEST_BROKER</string><string>--puerto</string><string>$PUERTO</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>CLAUDE_PEERS_REDIS_URL</key><string>$REDIS_URL</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/claude-peers-broker.out.log</string>
  <key>StandardErrorPath</key><string>/tmp/claude-peers-broker.err.log</string>
</dict>
</plist>
EOF
      launchctl unload "$PLIST" 2>/dev/null || true
      launchctl load "$PLIST"
      log "LaunchAgent $LABEL cargado (broker en :$PUERTO)"
      ;;
    *) warn "SO '$SO' no soportado para servicio automático; arranca el broker a mano" ;;
  esac
fi

# --- 4. Registrar el MCP en la config del usuario (~/.claude.json) ---
# Usamos `claude mcp add` si está disponible (idempotente con --force); si no, editamos el JSON.
ARGS_CLIENT=()
[[ -n "$BROKER_URL" ]] && ARGS_CLIENT+=(--broker-url "$BROKER_URL")

if command -v claude >/dev/null; then
  log "registrando MCP 'claude-peers' (scope user) vía claude mcp add"
  claude mcp remove --scope user claude-peers >/dev/null 2>&1 || true
  claude mcp add --scope user --transport stdio claude-peers -- "$DEST_CLIENT" "${ARGS_CLIENT[@]}"
else
  warn "el binario 'claude' no está en PATH; registra el MCP a mano en ~/.claude.json:"
  warn "  \"claude-peers\": { \"type\":\"stdio\", \"command\":\"$DEST_CLIENT\", \"args\":[$([[ -n "$BROKER_URL" ]] && echo "\"--broker-url\",\"$BROKER_URL\"")] }"
fi

# --- 5. Función claude() en el shell (inyecta el flag de canal) ---
# Detecta el rc del shell del usuario.
RC=""
case "$(basename "${SHELL:-/bin/bash}")" in
  zsh)  RC="$HOME/.zshrc" ;;
  bash) RC="$HOME/.bashrc" ;;
  *)    RC="$HOME/.profile" ;;
esac
MARCA="# >>> claude-peers-rs >>>"
if [[ -f "$RC" ]] && grep -qF "$MARCA" "$RC"; then
  log "función claude() ya presente en $RC (no la duplico)"
else
  log "añadiendo función claude() a $RC"
  cat >> "$RC" <<EOF

$MARCA
# Envuelve 'claude' para inyectar el flag del canal de peers (push <channel>).
# 'command claude' = claude SIN peers (escape). El id estable lo deriva peers-client de la carpeta.
claude() { command claude --dangerously-load-development-channels server:claude-peers "\$@"; }
# <<< claude-peers-rs <<<
EOF
fi

log "✓ instalación completa."
log "  Abre una terminal nueva (o 'source $RC') y lanza: claude --dangerously-skip-permissions"
log "  La equipe aparecerá sola. Verifica el broker: curl -s http://127.0.0.1:$PUERTO/salud"
