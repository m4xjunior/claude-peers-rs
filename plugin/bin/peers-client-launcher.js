#!/usr/bin/env node
// Launcher multiplataforma del peers-client (MCP stdio del plugin claude-peers-rs).
//
// DECISIÓN EXPLÍCITA (2026-07-06, Max): se acepta Node como capa MÍNIMA de detección de
// plataforma para el launcher del MCP. Es un cambio de postura del proyecto (hasta hoy: binario
// nativo puro, sin runtime externo). Razón: Claude Code ejecuta el `command` de un mcpServers
// stdio en "exec form" (directo, sin shell) y NO soporta configuración condicional por plataforma
// ni resuelve extensiones — así que un solo `command` bash sirve a Mac/Linux pero NUNCA a Windows
// nativo (sin bash garantizado). Un shim de Node es el patrón IDIOMÁTICO documentado
// (mcp-quickstart usa `npx`), la única vía cero-fricción para Windows+Mac+Linux con un solo
// `command`. Node ya está presente en la mayoría de entornos de dev; el costo es aceptable.
//
// ESTE SHIM NO TIENE LÓGICA DE NEGOCIO: su ÚNICO trabajo es (1) mapear plataforma/arch al binario
// nativo correcto y (2) ejecutarlo pasándole stdin/stdout/stderr y los args tal cual (el protocolo
// MCP stdio viaja transparente). Toda la lógica del peer vive en el binario Rust, no aquí.

const { spawn } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs");

// Raíz del plugin: Claude Code inyecta CLAUDE_PLUGIN_ROOT; si falta (invocación directa), se
// deriva de la ubicación de este script (bin/ → raíz del plugin es el padre de bin/).
const raiz = process.env.CLAUDE_PLUGIN_ROOT || path.resolve(__dirname, "..");
const binDir = path.join(raiz, "bin");

// Mapa plataforma/arch → nombre del binario nativo empaquetado en bin/. Los nombres siguen la
// convención `<os>-<arch>` que ya usan los binarios Mac/Linux del plugin. Windows lleva `.exe`.
function nombreBinario() {
  const so = process.platform; // 'darwin' | 'linux' | 'win32'
  const arch = process.arch; // 'arm64' | 'x64' | ...
  if (so === "darwin" && arch === "arm64") return "peers-client-darwin-arm64";
  if (so === "darwin" && arch === "x64") return "peers-client-darwin-x64";
  if (so === "linux" && arch === "x64") return "peers-client-linux-x64";
  if (so === "linux" && arch === "arm64") return "peers-client-linux-arm64";
  if (so === "win32" && arch === "x64") return "peers-client-windows-x64.exe";
  return null; // plataforma no soportada → error claro abajo
}

const nombre = nombreBinario();
if (!nombre) {
  process.stderr.write(
    `claude-peers: plataforma no soportada: ${process.platform}/${process.arch}\n`
  );
  process.exit(1);
}

const exe = path.join(binDir, nombre);
if (!fs.existsSync(exe)) {
  process.stderr.write(
    `claude-peers: binario no encontrado para ${process.platform}/${process.arch} (${exe})\n`
  );
  process.exit(1);
}

// Ejecuta el binario nativo heredando stdio (el MCP stdio pasa transparente) y reenviando los
// args (p.ej. --broker-url). Propaga el código de salida real del binario para que Claude Code
// vea el estado correcto. `windowsHide` evita una consola parpadeante en Windows.
const hijo = spawn(exe, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: true,
});

hijo.on("error", (err) => {
  process.stderr.write(`claude-peers: no se pudo ejecutar ${exe}: ${err.message}\n`);
  process.exit(1);
});
hijo.on("exit", (code, signal) => {
  if (signal) {
    // Terminado por señal (POSIX): replica el código convencional 128+señal.
    process.exit(128 + (typeof signal === "number" ? signal : 1));
  }
  process.exit(code === null ? 1 : code);
});
