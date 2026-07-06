#!/usr/bin/env node
// Launcher multiplataforma del peers-client (MCP stdio del plugin claude-peers-rs).
//
// DECISIÓN EXPLÍCITA (2026-07-06, Max): se acepta Node como capa MÍNIMA de detección de
// plataforma para el launcher del MCP. Es un cambio de postura del proyecto (hasta hoy: binario
// nativo puro, sin runtime externo). Razón: Claude Code ejecuta el `command` de un mcpServers
// stdio en "exec form" (directo, sin shell) y NO soporta configuración condicional por plataforma
// ni resuelve extensiones — así que un solo `command` bash sirve a Mac/Linux pero NUNCA a Windows
// nativo (sin bash garantizado). Un shim de Node es el patrón idiomático (mcp-quickstart usa npx).
//
// ESTE SHIM NO TIENE LÓGICA DE NEGOCIO: su ÚNICO trabajo es mapear plataforma/arch al binario
// nativo y ejecutarlo pasándole stdio + args tal cual (el protocolo MCP stdio viaja transparente).
//
// DIAGNÓSTICO (2026-07-06, bug de Daniela en Windows, error -32000): el shim escribe un log a
// `<os-tmp>/claude-peers-launcher.log` con cada arranque (plataforma, binario elegido, si existe,
// y cualquier error de spawn). Así, si el MCP no conecta, hay un rastro concreto en vez de una
// muerte silenciosa (release del client lleva panic=abort → sin mensaje). El log NUNCA va a stdout
// (eso rompería el protocolo MCP): solo a archivo y a stderr (que Claude Code captura aparte).

const { spawn } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs");
const os = require("node:os");

// --- Diagnóstico a archivo (best-effort, jamás rompe el arranque) ---
const LOG = path.join(os.tmpdir(), "claude-peers-launcher.log");
function diag(msg) {
  const linea = `[${new Date().toISOString()}] ${msg}\n`;
  try {
    fs.appendFileSync(LOG, linea);
  } catch (_e) {
    /* si no se puede escribir el log, seguimos igual */
  }
  // stderr NO es el canal del protocolo MCP (ese es stdout) → seguro para diagnóstico.
  try {
    process.stderr.write(`claude-peers: ${msg}\n`);
  } catch (_e) {}
}

// Raíz del plugin: Claude Code inyecta CLAUDE_PLUGIN_ROOT; si falta (invocación directa), se
// deriva de la ubicación de este script (bin/ → la raíz del plugin es el padre de bin/).
const raiz = process.env.CLAUDE_PLUGIN_ROOT || path.resolve(__dirname, "..");
const binDir = path.join(raiz, "bin");
diag(`arranque: platform=${process.platform} arch=${process.arch} node=${process.version} raiz=${raiz}`);

// Mapa plataforma/arch → binario nativo. En Windows, `.exe`.
function nombreBinario() {
  const so = process.platform; // 'darwin' | 'linux' | 'win32'
  const arch = process.arch; // 'arm64' | 'x64' | ...
  if (so === "darwin" && arch === "arm64") return "peers-client-darwin-arm64";
  if (so === "darwin" && arch === "x64") return "peers-client-darwin-x64";
  if (so === "linux" && arch === "x64") return "peers-client-linux-x64";
  if (so === "linux" && arch === "arm64") return "peers-client-linux-arm64";
  if (so === "win32" && arch === "x64") return "peers-client-windows-x64.exe";
  // Fallback Windows: cualquier arch → intentar el x64 (WoW64 corre x64 en arm64, y evita quedar
  // sin binario por un `process.arch` inesperado). Mejor intentar que abortar de una.
  if (so === "win32") return "peers-client-windows-x64.exe";
  return null;
}

const nombre = nombreBinario();
if (!nombre) {
  diag(`ERROR: plataforma no soportada (${process.platform}/${process.arch})`);
  process.exit(1);
}

const exe = path.join(binDir, nombre);
if (!fs.existsSync(exe)) {
  diag(`ERROR: binario no encontrado: ${exe}. Contenido de bin/: ${listarBin(binDir)}`);
  process.exit(1);
}
diag(`binario elegido: ${exe} (existe)`);

// --- Config explícita broker-url/token (Windows, bug de Daniela: las env vars del SO no se
// propagan de forma fiable al proceso hijo). Solución robusta: el launcher LEE la config de varias
// fuentes y la pasa como ARGS EXPLÍCITOS (--broker-url/--token) al .exe, en vez de confiar en que
// la env var herede. Precedencia (la primera que exista gana, por clave):
//   1. args que ya vengan en el `command` del .mcp.json (respetados tal cual, no se duplican).
//   2. archivo de config del usuario: <HOME>/.claude/claude-peers.json  → { broker_url, token }.
//      SOBREVIVE a `claude update plugin` (vive fuera del plugin) y NO depende de env vars del SO.
//   3. env vars CLAUDE_PEERS_BROKER_URL / CLAUDE_PEERS_TOKEN (si el SO sí las propaga).
// Así Daniela solo edita UN archivo de texto una vez, sin tocar setx/registro de Windows. ---
function leerConfigUsuario() {
  const home = process.env.HOME || process.env.USERPROFILE || os.homedir();
  const ruta = path.join(home, ".claude", "claude-peers.json");
  try {
    if (fs.existsSync(ruta)) {
      const cfg = JSON.parse(fs.readFileSync(ruta, "utf8"));
      diag(`config de usuario leída: ${ruta}`);
      return cfg && typeof cfg === "object" ? cfg : {};
    }
  } catch (e) {
    diag(`aviso: no se pudo leer ${ruta}: ${e && e.message}`);
  }
  return {};
}

const argsBase = process.argv.slice(2);
const cfgUsuario = leerConfigUsuario();
// Añade un arg SOLO si no venía ya en argsBase (no pisar lo que el .mcp.json declare explícito).
function agregarArg(args, flag, valorArchivo, valorEnv) {
  if (args.includes(flag)) return args; // ya presente: respetar
  const valor = valorArchivo || valorEnv;
  if (valor) {
    args.push(flag, String(valor));
    diag(`arg añadido: ${flag} (fuente: ${valorArchivo ? "config" : "env"})`);
  }
  return args;
}
let argsFinal = argsBase.slice();
argsFinal = agregarArg(argsFinal, "--broker-url", cfgUsuario.broker_url, process.env.CLAUDE_PEERS_BROKER_URL);
argsFinal = agregarArg(argsFinal, "--token", cfgUsuario.token, process.env.CLAUDE_PEERS_TOKEN);

// Ejecuta el binario nativo. PUENTE DE PIPES EXPLÍCITO (fix del push en Windows, 2026-07-06):
// en vez de stdio:"inherit" (que HEREDA los handles de stdin/stdout del SO), usamos stdio:"pipe"
// y forwardeamos los bytes ACTIVAMENTE con Node (process.stdin→hijo.stdin, hijo.stdout→process.stdout).
//
// POR QUÉ: en Windows, un handle heredado a través de DOS niveles de spawn (Claude Code → node → .exe)
// no propaga bien las escrituras ASYNC del hijo (las notificaciones `notifications/claude/channel`
// del push, que ocurren FUERA del ciclo request/response). Síntoma exacto de Daniela: revisar_mensajes
// (respuesta síncrona a un tool/call) llega, pero el push automático no. Con el puente, Node gestiona
// el forwarding byte a byte por su event loop — no depende del encadenamiento de handles del SO, así
// que las notificaciones async del .exe llegan a Claude Code igual en las 3 plataformas. En Mac/Linux
// el comportamiento es idéntico al inherit (verificado). stderr SÍ se hereda (los logs del client van
// al stderr de Claude Code como antes). `windowsHide` evita una consola parpadeante.
let hijo;
try {
  hijo = spawn(exe, argsFinal, {
    stdio: ["pipe", "pipe", "inherit"],
    windowsHide: true,
  });
} catch (err) {
  diag(`ERROR: spawn lanzó excepción: ${err && err.message}`);
  process.exit(1);
}

// Forwarding bidireccional explícito. `.pipe()` conecta los streams y reenvía todos los datos +
// el fin (EOF): cuando Claude Code cierra stdin, se cierra el del hijo (el client termina limpio).
process.stdin.pipe(hijo.stdin);
hijo.stdout.pipe(process.stdout);
// Si un extremo del pipe se rompe (EPIPE al cerrar la sesión), no queremos un crash del shim:
// lo tragamos best-effort — el `exit` del hijo hará la salida ordenada.
process.stdin.on("error", () => {});
hijo.stdin.on("error", () => {});
hijo.stdout.on("error", () => {});

hijo.on("error", (err) => {
  // Errores async de spawn (ENOENT, EACCES, bloqueo de Defender…) caen aquí. `code` distingue.
  diag(`ERROR: el binario no se pudo ejecutar (${err && err.code}): ${err && err.message}`);
  process.exit(1);
});
hijo.on("exit", (code, signal) => {
  diag(`el binario terminó: code=${code} signal=${signal}`);
  if (signal) {
    process.exit(1);
  }
  process.exit(code === null ? 1 : code);
});

// Lista bin/ para el diagnóstico (qué binarios hay realmente empaquetados).
function listarBin(dir) {
  try {
    return fs.readdirSync(dir).join(", ");
  } catch (e) {
    return `(no se pudo leer ${dir}: ${e && e.message})`;
  }
}
