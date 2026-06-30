# Troubleshooting — Red cross-host Mac ↔ servidor (claude-peers-rs)

Runbook operativo de los 6 casos reales donde nos trabamos el **2026-06-30** montando y operando la red de peers entre el **Mac (broker central)** y un **servidor remoto**. Cada caso trae síntoma exacto, causa raíz verificada, comandos de diagnóstico y el fix con comandos copy-paste.

## Topología de referencia (constantes de hoy)

| Pieza | Valor |
|---|---|
| Broker central | **Mac** |
| IP:puerto del broker | `10.0.1.60:7899` |
| Token de la red | `lexusfx-peers-2026` |
| URL del broker | `http://10.0.1.60:7899` |
| Endpoint de salud | `http://10.0.1.60:7899/salud` |
| Default si NO hay env | `127.0.0.1:7899` (broker LOCAL — isla, NO sirve para cross-host) |
| Versión Claude Code | `2.1.196` (cambió sintaxis de `mcp add` y exige `command claude` por la función shell) |

> **Regla mental:** el Mac es el único broker. Todo lo demás (servidor incluido) es **cliente** y debe apuntar a `10.0.1.60:7899` con el token. Si una máquina cae al default `127.0.0.1:7899`, se aísla en su propio broker local y no la ve nadie.

---

## Tabla resumen — síntoma → caso

| # | Síntoma (lo que ves) | Causa raíz | Caso |
|---|---|---|---|
| 1 | El server no aparece en la TUI del Mac / no se ven entre máquinas | env del broker/token NO exportadas antes de arrancar `claude` → cae al default local | [Caso 1](#caso-1--el-server-no-aparece-en-la-tui-del-mac--no-se-ven-entre-máquinas) |
| 2 | Ni llega el mensaje al peer (ayer sí, hoy no) | MCP `claude-peers` NO registrado en el server → flag apunta a server inexistente → push descartado | [Caso 2](#caso-2--ni-llega-el-mensaje-al-peer-ayer-llegaba-hoy-no) |
| 3 | Mensajes "robados": el broker marca `leido` pero el humano no ve nada | sesiones `claude` duplicadas con el mismo `cwd` → varios clients dreanan la misma cola | [Caso 3](#caso-3--mensajes-robados-el-broker-dice-leido-pero-el-humano-no-ve-nada) |
| 4 | `claude mcp add --scope` falla con `unknown option` | Claude Code 2.1.196 cambió sintaxis (`-s`/`-e`) + función `claude()` del zshrc intercepta | [Caso 4](#caso-4--claude-mcp-add---scope-falla-con-unknown-option) |
| 5 | El estado `leido` del broker engaña | `leido` = bytes escritos al socket stdio del MCP, NO recepción real ni render en humano | [Caso 5](#caso-5--el-estado-leido-del-broker-engaña) |
| 6 | El push `<channel>` no salta solo aunque el flag esté puesto | flag necesario pero no suficiente: el harness evalúa 6 puertas; la #1 (MCP no configurado) fue la real | [Caso 6](#caso-6--el-push-channel-no-salta-solo-aunque-el-flag-esté-puesto) |

---

## Caso 1 — "El server no aparece en la TUI del Mac / no se ven entre máquinas"

### Síntoma
En la TUI del Mac no aparece el server como instancia. `listar_instancias` desde el Mac no lo lista. Parecen dos redes separadas que no se ven.

### Causa raíz (verificada)
Las variables `CLAUDE_PEERS_BROKER_URL` y `CLAUDE_PEERS_TOKEN` **no estaban exportadas en el entorno del proceso `claude`** del server **antes** de arrancarlo.

Cadena exacta del fallo:

1. El `.mcp.json` del plugin trae `env: {}` vacío → el `peers-client` **no** recibe las vars por config; las **hereda del entorno del proceso `claude`** que lo lanza.
2. El `peers-client` lee esas vars **una sola vez al arrancar**.
3. Si no están en el entorno → cae al **default `127.0.0.1:7899`**.
4. Se registra en un **broker LOCAL del propio server** (una isla), no en el broker central del Mac.

> **Trampa clave:** escribir las vars en `~/.bashrc` **NO basta si `claude` ya está corriendo**. Los rc-files solo aplican a shells **nuevos**; el proceso `claude` vivo conserva el entorno con el que nació.

### Diagnóstico
Desde el **server**, confirmar qué ve el proceso `claude` vivo y si alcanza el broker:

```bash
# 1) ¿Qué env tiene el proceso claude que está corriendo? (debe mostrar las dos vars)
pid=$(pgrep -n -f '(^|/)claude( |$)')
tr '\0' '\n' < /proc/$pid/environ | grep -E 'CLAUDE_PEERS_(BROKER_URL|TOKEN)'
# Vacío = está cayendo al default local → ESTE es el problema

# 2) ¿El server alcanza el broker del Mac? (debe dar 200)
curl -s -o /dev/null -w "%{http_code}\n" http://10.0.1.60:7899/salud
```

Interpretación:
- El `grep` no devuelve nada → el proceso nació sin las vars → está en la isla `127.0.0.1`.
- El `curl` da algo distinto de `200` → problema de red/firewall (resuélvelo antes de seguir).

### Fix
Exportar las dos vars **y DESPUÉS** arrancar `claude` (orden estricto):

```bash
# En el server, en la MISMA shell, en este orden:
export CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899
export CLAUDE_PEERS_TOKEN=lexusfx-peers-2026

# Verifica alcance ANTES de arrancar (debe imprimir 200)
curl -s -o /dev/null -w "%{http_code}\n" http://10.0.1.60:7899/salud

# Ahora sí, arranca claude (hereda el entorno correcto)
claude
```

Para que persista en shells nuevos (no afecta a procesos ya vivos), añádelo al rc:

```bash
cat >> ~/.bashrc <<'EOF'
export CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899
export CLAUDE_PEERS_TOKEN=lexusfx-peers-2026
EOF
```

> Si `claude` **ya estaba corriendo** cuando exportaste, **no vale**: hay que cerrarlo y volver a arrancarlo desde la shell que ya tiene las vars exportadas.

---

## Caso 2 — "Ni llega el mensaje al peer (ayer llegaba, hoy no)"

### Síntoma
Envías un mensaje al peer del server y nunca llega. En los logs aparece algo como:

```
server:claude-peers · no MCP server configured with that name
```

Ayer funcionaba, hoy no — sin tocar nada aparente.

### Causa raíz (verificada)
El MCP **`claude-peers` NO estaba registrado** en el server (`enabledPlugins`: ninguno; user scope vacío).

El flag `--dangerously-load-development-channels server:claude-peers` apunta por nombre a un MCP server que **no existe** en ese harness → el push se **descarta** con `no MCP server configured with that name`.

El `peers-client` corría **suelto** (como proceso), pero el **harness de `claude` no lo conocía** como el server al que se refiere el flag. El binario y el flag estaban desconectados.

### Diagnóstico
Desde el **server**:

```bash
# ¿Está el MCP claude-peers registrado y conectado?
claude mcp list | grep -i peers
# Si no aparece, o aparece pero NO dice "Connected" → ESTE es el problema
```

### Fix
Registrar el MCP en **user scope** (global) con la sintaxis nueva de 2.1.196. Sustituye `~/.claude/plugins/cache/lexusfx/claude-peers-rs/0.1.10/bin/peers-client-launcher` por la ruta real del launcher del `peers-client`:

```bash
command claude mcp add claude-peers -s user \
  -e CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899 \
  -e CLAUDE_PEERS_TOKEN=lexusfx-peers-2026 \
  -- ~/.claude/plugins/cache/lexusfx/claude-peers-rs/0.1.10/bin/peers-client-launcher
```

Verificar:

```bash
claude mcp list | grep -i peers   # debe decir: claude-peers ... Connected
```

> **REINICIAR la sesión `claude`** después de registrar el MCP. El harness carga los MCP servers **al arrancar**; una sesión viva no los recoge en caliente.

> `-s user` = scope user (global a todos los proyectos del usuario). Es lo que queremos para que el peers-client esté siempre disponible, no atado a un repo concreto.

---

## Caso 3 — "Mensajes robados: el broker dice 'leido' pero el humano no ve nada"

### Síntoma
El broker marca el mensaje como `leido` (a veces "leído en 2s"), pero el humano de la sesión real **nunca lo ve** en su TUI. El mensaje desaparece sin renderizarse.

### Causa raíz (verificada)
**Sesiones `claude` duplicadas compitiendo por el mismo id.**

El id de instancia se deriva del `cwd`. Si hay **N sesiones en el mismo directorio** (por reinicios, sesiones remotas `ccd-cli`, o `kill`s que dejaron procesos colgados), las N levantan **N `peers-client`** que **dreanan la MISMA cola**.

El primer client que hace `recibir` (peek + idempotencia) **confirma `leido` y se queda el mensaje** → la sesión real (la que mira el humano) nunca lo ve. El mensaje fue consumido por una sesión fantasma.

### Diagnóstico
En el **server (Linux)**, listar cada `peers-client`, su `cwd` y su `tty`:

```bash
for c in $(pgrep peers-client); do
  ppid=$(ps -o ppid= -p $c | tr -d " ")
  echo "client $c → cwd=$(readlink /proc/$ppid/cwd) tty=$(ps -o tty= -p $ppid)"
done
```

Interpretación:
- **>1 client con el MISMO `cwd`** = colisión (la causa).
- Las sesiones **reales** tienen `tty` real (`pts/N`).
- Las **zombi** son viejas o sin tty (`?` = remota / sin terminal).

### Fix
Matar las duplicadas dejando **1 client por `cwd`**. Si resisten, `kill -9`:

```bash
# Ejemplo: matar un PID concreto de los listados arriba
kill <PID_DUPLICADO>
# Si no muere:
kill -9 <PID_DUPLICADO>
```

> **OJO con `ccd-cli`:** las sesiones remotas (`.claude/remote/ccd-cli/`) las **respawnea un daemon** → vuelven a aparecer aunque las mates. Para eliminarlas de verdad hay que **parar el daemon de origen** que las relanza, no solo matar el client.

```bash
# Localizar el daemon que respawnea sesiones remotas ccd-cli
pgrep -af ccd-cli
# Parar el daemon de origen (no solo el client hijo)
```

---

## Caso 4 — "`claude mcp add --scope` falla con `unknown option`"

### Síntoma
```
$ claude mcp add claude-peers --scope user --env KEY=val -- <cmd>
error: unknown option '--scope'
```

### Causa raíz (verificada)
Dos cosas a la vez en **Claude Code 2.1.196**:

1. **Cambió la sintaxis** de `claude mcp add`:
   - `--scope` → **`-s`**
   - `--env` → **`-e`**
2. La **función `claude()` del `.zshrc`** intercepta el comando `claude` → hay que usar **`command claude`** para hacer bypass de la función y llamar al binario real.

### Diagnóstico
```bash
# ¿Hay una función claude interceptando?
type claude        # si dice "claude is a shell function" → necesitas 'command claude'

# ¿Qué versión tienes?
command claude --version   # confirma 2.1.196 (sintaxis nueva)
```

### Fix
Sintaxis correcta para 2.1.196, con `command` para bypassear la función shell:

```bash
command claude mcp add <nombre> -s user -e KEY=val -- <cmd>
```

Aplicado al peers-client (idéntico al Caso 2):

```bash
command claude mcp add claude-peers -s user \
  -e CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899 \
  -e CLAUDE_PEERS_TOKEN=lexusfx-peers-2026 \
  -- ~/.claude/plugins/cache/lexusfx/claude-peers-rs/0.1.10/bin/peers-client-launcher
```

---

## Caso 5 — "El estado `leido` del broker engaña"

### Aclaración (no es un bug, es semántica)
`leido` **NO** significa "el humano lo vio". El broker marca `leido` cuando el `peers-client` **escribe los bytes al socket stdio del MCP** (flush OK).

Lo que `leido` **sí** garantiza:
- Los bytes salieron del client hacia el MCP (flush correcto).

Lo que `leido` **NO** garantiza:
- Que el **harness** renderice el `<channel>`.
- Que el **humano** lo vea en su TUI.
- Que no haya sido un **client zombi** quien hizo el flush (ver [Caso 3](#caso-3--mensajes-robados-el-broker-dice-leido-pero-el-humano-no-ve-nada)).

> Por eso **"leído en 2s" NO prueba recepción real**. Un client fantasma drenando la cola también marca `leido` y el mensaje nunca llega a la sesión real.

### Cómo verificar recepción REAL
No te fíes del estado del broker. Pide al peer una **prueba de vida activa**: que responda **solo si le llegó el mensaje sin tener que usar `revisar_mensajes`**:

```
Pide al peer:
  "Si te llega ESTE mensaje por el canal automático (sin que tengas
   que llamar revisar_mensajes), respóndeme exactamente: PONG auto OK"
```

- Recibes `PONG auto OK` → recepción **real** confirmada (el push automático funciona).
- No recibes nada, o el peer tuvo que hacer `revisar_mensajes` para encontrarlo → el push automático **NO** está entregando (revisa Casos 2 y 6).

---

## Caso 6 — "El push `<channel>` no salta solo aunque el flag esté puesto"

### Síntoma
El flag `--dangerously-load-development-channels server:claude-peers` está puesto, pero el push `<channel>` **no se renderiza solo** en la sesión destino. El peer solo ve el mensaje si hace `revisar_mensajes` manualmente.

### Causa raíz (verificada)
El flag es **NECESARIO pero NO suficiente**. El harness (binario `claude` 2.1.196, función interna que evalúa **6 puertas**) **descarta el push** si falla cualquiera de estas:

| # | Puerta | Condición de fallo |
|---|---|---|
| 1 | **Capability `claude/channel`** | El MCP **no declaró** la capability — típicamente porque el MCP no está configurado/conectado (ver [Caso 2](#caso-2--ni-llega-el-mensaje-al-peer-ayer-llegaba-hoy-no)). **← La real de hoy** |
| 2 | **Provider** | El provider no es **firstParty** (Anthropic). |
| 3 | **Feature** | La feature no está disponible. |
| 4 | **`channelsEnabled`** | `false` por **org policy** (team/enterprise). |
| 5 | **Lista `--channels`** | El server **no está** en la lista del flag. |
| 6 | **Marketplace** | Solo plugins de marketplace. |

La que nos frenó hoy fue la **#1**: el MCP no estaba configurado → no declaró la capability → push descartado, por mucho que el flag estuviera puesto.

### Diagnóstico
```bash
# Puerta #1 — ¿MCP configurado y conectado? (la causa real de hoy)
claude mcp list | grep -i peers          # debe decir "Connected"

# Puerta #5 — ¿el flag incluye el server por nombre?
ps aux | grep -- '--dangerously-load-development-channels' | grep -v grep
# debe verse: --dangerously-load-development-channels server:claude-peers

# Puerta #2 — ¿sesión Anthropic firstParty? (no Bedrock/Vertex/proxy de terceros)
```

### Fix
Asegurar las tres condiciones que dependen de nosotros, en orden:

1. **MCP configurado + conectado** (puerta #1) → [Caso 2](#caso-2--ni-llega-el-mensaje-al-peer-ayer-llegaba-hoy-no):
   ```bash
   command claude mcp add claude-peers -s user \
     -e CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899 \
     -e CLAUDE_PEERS_TOKEN=lexusfx-peers-2026 \
     -- ~/.claude/plugins/cache/lexusfx/claude-peers-rs/0.1.10/bin/peers-client-launcher
   claude mcp list | grep -i peers   # Connected
   ```

2. **Flag presente** (puertas #5/#6), respetando la función shell del [Caso 4](#caso-4--claude-mcp-add---scope-falla-con-unknown-option):
   ```bash
   command claude --dangerously-load-development-channels server:claude-peers
   ```

3. **Sesión Anthropic firstParty** (puerta #2) — no Bedrock/Vertex/proxy de terceros.

> Puertas #3 (feature) y #4 (org policy) no se arreglan desde el cliente: dependen de disponibilidad de feature y política de la org (team/enterprise). Si `channelsEnabled=false` por policy, el push no saltará aunque todo lo demás esté perfecto.

---

## Checklist de instalación plug-and-play (server nuevo)

Pasos en orden para enchufar un server nuevo a la red del Mac. Sustituye `~/.claude/plugins/cache/lexusfx/claude-peers-rs/0.1.10/bin/peers-client-launcher` por la ruta real.

```bash
# 1) Exportar env del broker ANTES de arrancar claude (Caso 1)
export CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899
export CLAUDE_PEERS_TOKEN=lexusfx-peers-2026

# 2) Verificar alcance al broker del Mac (debe dar 200) (Caso 1)
curl -s -o /dev/null -w "%{http_code}\n" http://10.0.1.60:7899/salud

# 3) Persistir env en el rc (solo afecta a shells NUEVOS) (Caso 1)
cat >> ~/.bashrc <<'EOF'
export CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899
export CLAUDE_PEERS_TOKEN=lexusfx-peers-2026
EOF

# 4) Registrar el MCP en user scope, sintaxis 2.1.196 + bypass función shell (Casos 2 y 4)
command claude mcp add claude-peers -s user \
  -e CLAUDE_PEERS_BROKER_URL=http://10.0.1.60:7899 \
  -e CLAUDE_PEERS_TOKEN=lexusfx-peers-2026 \
  -- ~/.claude/plugins/cache/lexusfx/claude-peers-rs/0.1.10/bin/peers-client-launcher

# 5) Verificar que el MCP está Connected (Caso 2)
claude mcp list | grep -i peers     # claude-peers ... Connected

# 6) Comprobar que NO hay clients duplicados por cwd (Caso 3)
for c in $(pgrep peers-client); do
  ppid=$(ps -o ppid= -p $c | tr -d " ")
  echo "client $c → cwd=$(readlink /proc/$ppid/cwd) tty=$(ps -o tty= -p $ppid)"
done
# Esperado: 1 solo client por cwd. Si hay más, mátalos (kill -9) y revisa daemon ccd-cli.

# 7) Arrancar claude CON el flag de channels (hereda env del paso 1) (Caso 6)
command claude --dangerously-load-development-channels server:claude-peers

# 8) (Tras arrancar) Si registraste el MCP con claude ya vivo, REINICIA la sesión
#    para que el harness lo cargue (Caso 2).
```

### Verificación final (recepción REAL, no `leido`) (Caso 5)

Desde el Mac, envía al peer del server:

```
"Si te llega ESTE mensaje por el canal automático (sin llamar
 revisar_mensajes), respóndeme exactamente: PONG auto OK"
```

- Llega `PONG auto OK` → red cross-host operativa de punta a punta.
- No llega → recorre Casos 1 → 2 → 6 → 3 en ese orden.

### Resumen de orden de causas (de más frecuente a menos)

1. **env no exportada antes de arrancar** (Caso 1) → isla local.
2. **MCP no registrado/conectado** (Caso 2 / puerta #1 del Caso 6) → push descartado.
3. **Flag ausente o mal** (Caso 6, puertas #5/#6).
4. **Clients duplicados** (Caso 3) → mensajes robados pese a `leido`.
5. **Confiar en `leido`** (Caso 5) → falsa sensación de entrega.
