#!/usr/bin/env node
// Hook SessionStart del plugin claude-peers-rs (versión multiplataforma).
//
// DECISIÓN (2026-07-06, Max): igual que el launcher del MCP, este hook pasa a Node para funcionar
// en Windows+Mac+Linux con un solo comando. El `asegurar-broker.sh` original (bash) rompía en
// Windows sin bash. Node es el runtime agnóstico ya adoptado por el proyecto para esta capa.
//
// Qué hace: asegura que el broker esté vivo ANTES de que el peers-client (MCP) intente conectarse.
//   - Si se apunta a un broker REMOTO (CLAUDE_PEERS_BROKER_URL seteado) → NO levanta nada (el caso
//     de Daniela: cliente en la red del equipo, el broker ya existe). Skip inmediato.
//   - Si el broker local ya responde en el puerto → no hace nada.
//   - Si no responde → lo lanza DESACOPLADO de la sesión (sobrevive al cierre).
//   - En Windows: por ahora NO hay peers-broker.exe (el broker no cross-compila por libc::kill,
//     POSIX puro; pendiente portar pid_vivo con Windows API). Así que en Windows este hook SIEMPRE
//     skipea el arranque local — el usuario Windows es cliente de un broker remoto. Sin crash.
//
// SIEMPRE termina con exit 0: un hook SessionStart no debe bloquear la sesión por no poder
// levantar el broker (el client reintenta por latido). Best-effort.

const http = require("node:http");
const path = require("node:path");
const fs = require("node:fs");
const { spawn } = require("node:child_process");

const raiz = process.env.CLAUDE_PLUGIN_ROOT || path.resolve(__dirname, "..");
const puerto = process.env.CLAUDE_PEERS_PORT || "7899";
const urlRemota = process.env.CLAUDE_PEERS_BROKER_URL;

function salir() {
  process.exit(0);
}

// 1. Broker remoto configurado → no levantamos nada local (caso Daniela).
if (urlRemota) {
  salir();
}

// 2. Windows: sin broker local (no hay .exe del broker todavía). Skip.
if (process.platform === "win32") {
  salir();
}

// 3. ¿El broker local ya responde en /salud? (GET breve; timeout 1s). Si sí, nada que hacer.
const urlSalud = `http://127.0.0.1:${puerto}/salud`;
const req = http.get(urlSalud, { timeout: 1000 }, (res) => {
  res.resume(); // drena la respuesta
  if (res.statusCode && res.statusCode >= 200 && res.statusCode < 500) {
    salir(); // el broker ya está vivo
  } else {
    lanzarBroker();
  }
});
req.on("timeout", () => {
  req.destroy();
  lanzarBroker();
});
req.on("error", () => {
  // No responde (broker caído / puerto libre) → intentar levantarlo.
  lanzarBroker();
});

// 4. Arranca el broker nativo desacoplado de esta sesión (detached), best-effort.
function lanzarBroker() {
  const nombre =
    process.platform === "darwin"
      ? process.arch === "arm64"
        ? "peers-broker-darwin-arm64"
        : "peers-broker-darwin-x64"
      : "peers-broker-linux-x64"; // Linux x64 (el broker no se compila para Windows por ahora)
  const exe = path.join(raiz, "bin", nombre);
  if (!fs.existsSync(exe)) {
    salir(); // sin binario del broker para esta plataforma → skip silencioso
  }
  try {
    const salida = fs.openSync("/tmp/claude-peers-broker.out.log", "a");
    const errsal = fs.openSync("/tmp/claude-peers-broker.err.log", "a");
    const hijo = spawn(exe, ["--puerto", String(puerto)], {
      detached: true,
      stdio: ["ignore", salida, errsal],
    });
    hijo.unref(); // que sobreviva al fin de este proceso hook
  } catch (_e) {
    // Best-effort: si no se pudo lanzar, el client reintentará por latido. No romper la sesión.
  }
  salir();
}
