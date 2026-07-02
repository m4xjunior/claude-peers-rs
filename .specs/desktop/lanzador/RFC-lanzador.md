# RFC — Lanzador de sesión + Terminal embebido + Chat privado (peers-desktop)

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

> Fecha: 2026-07-02. Estado: **PROPUESTO — NO implementar hasta aprobar.**
> Pantalla nueva: `crates/peers-desktop/src/vista/lanzador.rs` (10ª pestaña del sidebar).
> Design System: **Ethos** (TINTA `#100D0A`, TINTA2 `#1A1611`, PAPEL `#ECE5D7`, BRASA `#C9A96E`,
> HUMO `#938B7B`, LINEA `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999).
> Verificado contra: GPUI rev `1d217ee` (el fijado en `Cargo.toml`), `gpui-component` (longbridge, git),
> `crates/peers-desktop/src/{main.rs,app.rs,cliente.rs}`, `crates/peers-client/src/mcp.rs`.

---

## 1. El problema (en palabras de Max)

> "Quiero elegir el directorio, mi app tiene que tener acceso a mi file system, poder seleccionar
> el directorio para lanzar una sesión, y tener toda la UI visual (gpui-component) para inyectar eso,
> dejar el system prompt, escribir tareas, etc. en ese mismo apartado. Con opción de elegir entre
> sesión tmux; y poder instalar el MCP accediendo por terminal DENTRO de mi GPUI —el terminal—,
> dentro del SSH. Quiero un terminal también."

Hoy `peers-desktop` es un **panel de observación** del broker (9 pestañas de solo-lectura/gestión).
No puede **originar trabajo**: no lanza sesiones de `claude`, no da acceso al filesystem, no tiene
terminal, y el "chat privado" con el agente (ver §7) no existe. Max tiene que salir de la app,
abrir una terminal a mano, `cd` al proyecto, exportar el flag de canal, y arrancar `claude` —cada vez,
en cada máquina—. Eso contradice la VISÃO ("inicio `claude` y sale de perto"): **la app debería SER
el punto de arranque del equipo**, no solo su tablero.

## 2. La solución (visión del apartado)

Una pestaña **Lanzador** que es un flujo de 3 zonas:

1. **Configurar la sesión** — file picker nativo para el directorio, editor de system prompt,
   lista de tareas iniciales, y destino de ejecución (local / SSH a otra máquina / dentro de tmux).
2. **Terminal embebido (PTY)** — un emulador de terminal real dentro de la app GPUI donde corre
   la sesión (`claude`, o `ssh`, o `tmux attach`), y desde donde Max puede **instalar el MCP** y
   operar la máquina remota. Es el "quiero un terminal también".
3. **Chat privado** — un panel lateral por sesión donde Max habla con ESE Claude sin que la
   conversación se renderice en el pane del TUI (§7). Resuelve la contradicción arquitectónica que
   se detectó en el análisis previo: el `<channel>` actual SE renderiza; este canal NO.

**Perfiles de lanzamiento** persistidos: cada combinación (dir + system prompt + tareas + destino)
se guarda como un "perfil" reutilizable, para relanzar con un click en cualquier máquina.

---

## 3. Requisitos (trazables)

### Zona A — Configuración de la sesión

- **R1 — Selección de directorio (file picker nativo).** Botón "Elegir directorio" abre el picker
  del sistema vía `cx.prompt_for_paths(PathPromptOptions { files:false, directories:true, multiple:false })`
  (API GPUI verificada, `app.rs:1382`; devuelve `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>`).
  El resultado se lee en un `cx.spawn` (patrón `descartar_alerta`/`cliente.rs`). NO se añade ninguna
  dependencia (rfd, etc.): el picker es nativo de GPUI. **Sin acceso ilimitado al FS**: solo se guarda
  la ruta que el usuario elige explícitamente en el diálogo del sistema (sin recorrer el FS por cuenta propia).
- **R1.1** — La ruta elegida se muestra en `IBM Plex Mono` (BRASA) con un botón "Revelar en Finder"
  (`cx.reveal_path`) y "Cambiar". Validar que existe y es directorio antes de habilitar "Lanzar".
- **R1.2** — Directorios recientes: se recuerdan los últimos N (default 8) en `config.toml`,
  ofrecidos en un `Select` para reelegir sin volver a abrir el picker.
- **R2 — Editor de system prompt.** `Input` multilínea (Textarea del kit) que se pasará a la sesión
  como `--append-system-prompt "<texto>"` (flag real de Claude Code). Placeholder HUMO. Contador de
  caracteres mono. El system prompt es **opcional** (vacío = no se pasa el flag).
- **R2.1** — Plantillas de system prompt: guardar/cargar prompts nombrados (p.ej. "peer backend Rust",
  "peer frontend") en `config.toml`; `Select` para elegir plantilla y editarla antes de lanzar.
- **R3 — Tareas iniciales.** Lista editable (añadir/quitar/reordenar) de tareas que se materializan al
  lanzar. Cada tarea: descripción + estimado opcional. Dos modos de materialización (R3.1/R3.2), elegibles:
  - **R3.1 (broker)** — se crean vía `POST /tarea/asignar { instancia_id, descripcion, estimado_seg? }`
    (endpoint EXISTENTE) **una vez la sesión se registra** en el broker con un id conocido. Requiere
    resolver el binding id↔sesión (ver §6, riesgo abierto).
  - **R3.2 (prompt)** — se inyectan como texto en el system prompt / primer mensaje ("Tus tareas de hoy:
    1)… 2)…"), sin depender del broker. Fallback si R3.1 no está resuelto.
- **R4 — Destino de ejecución.** `Select` con 3 destinos:
  - **R4.1 Local** — `claude` corre en la máquina donde está la app.
  - **R4.2 SSH** — `ssh <host>` primero (host de una lista configurable, reusa el patrón multi-broker
    de RFC-acceso), y dentro `cd <dir> && claude …`. El dir es remoto (el picker local no aplica; se
    escribe la ruta remota a mano o se elige de recientes remotos por host).
  - **R4.3 tmux** — la sesión se lanza dentro de tmux (`tmux new-session -d -s <nombre> -c <dir> "claude …"`
    y luego `tmux attach -t <nombre>` en el terminal embebido), local o sobre SSH. Nombre de sesión
    editable; si ya existe, ofrecer "attach" en vez de "new". (Espeja el runbook `~/.claude/docs/`.)
- **R5 — Flag de canal siempre presente.** El comando de lanzamiento SIEMPRE incluye
  `--dangerously-load-development-channels server:claude-peers` (constante del proyecto; sin él el
  `<channel>` no se renderiza — `README.md:54`, `troubleshooting.md:264`). La app lo añade sola; Max
  no lo teclea. Opción avanzada para `--dangerously-skip-permissions` (default OFF, con aviso).
- **R6 — Previsualización del comando.** Antes de lanzar, mostrar el comando exacto que se ejecutará
  (mono, scrolleable) + botón "Copiar comando". Nada se ejecuta a ciegas.

### Zona B — Terminal embebido (PTY)

- **R7 — Emulador de terminal real dentro de la app.** Un pane GPUI que abre un **PTY** y ejecuta el
  comando de lanzamiento (o un shell libre). Debe soportar: entrada de teclado, salida con colores ANSI,
  scrollback, resize. Es donde vive la sesión `claude`, el `ssh`, y donde Max **instala el MCP** a mano
  (`claude mcp add …`) en local o remoto. **Decisión de implementación pendiente (§6):** GPUI no trae
  terminal; opciones a evaluar — (a) portar/consumir `crates/terminal` + `terminal_view` de Zed (mismo
  repo/rev ya fijado → reutilizable), (b) crate `alacritty_terminal` + parser propio, (c) `portable-pty`
  + render de rejilla propio. Recomendación inicial: (a) por afinidad de rev y madurez, con el coste de
  arrastrar sus deps.
- **R7.1** — Múltiples terminales/sesiones a la vez: `Tabs` del kit, una pestaña por sesión lanzada
  (nombre = perfil o id de sesión). Cerrar pestaña = matar el PTY (con confirmación si hay proceso vivo).
- **R7.2** — El terminal degrada: si el backend de PTY no está disponible en una plataforma, el apartado
  cae al **modo "solo preparar"** (R6 + copiar comando + lanzar en Terminal externo del sistema vía
  `cx.open_with_system` sobre un script), SIN crashear. Nunca bloquear el resto de la app.
- **R7.3** — Instalar el MCP desde el terminal: comando de conveniencia "Instalar MCP aquí" que teclea
  por Max el `claude mcp add claude-peers -- <ruta al binario peers-client>` correcto para el destino
  (local o el host SSH activo). Deriva la ruta del binario del propio contexto de instalación.

### Zona C — Chat privado (integrado, ver §7 para el diseño de fondo)

- **R8 — Canal de chat privado por sesión, NO renderizado en el TUI.** Panel lateral (drawer/sheet)
  donde Max escribe a ESE Claude y ve sus respuestas, **sin que el intercambio aparezca en el pane del
  terminal embebido** (el TUI de `claude`). Mecanismo: **tools MCP dedicadas** (no el push `<channel>`,
  que sí se renderiza). Ver §7 — es la única vía que no depende del harness de Anthropic.
- **R8.1** — `peers-client` expone dos tools nuevas: `chat_privado_recibir()` (el agente sondea si hay
  mensaje privado pendiente para él) y `chat_privado_responder(texto)` (el agente contesta a Max).
  Las instrucciones del MCP (`mcp.rs::instrucciones`) ganan una línea que enseña al agente a **consultar
  el chat privado periódicamente / cuando Max lo pida**, y a responder por ahí, entendiendo que ese
  canal es privado con Max y no debe volcarse al output visible.
- **R8.2** — El broker gana una cola de chat privado por sesión (`cprs:chat_priv:{sesion_id}`, ZSET
  como la bandeja) + endpoints bajo token: `POST /chat-privado/enviar {sesion_id, de, texto}` (Max→Claude,
  lo consume `chat_privado_recibir`), `POST /chat-privado/responder {sesion_id, texto}` (Claude→Max),
  `GET /chat-privado/historial?sesion_id=` (la app pinta el hilo). Reusa el patrón de bandeja durable.
- **R8.3** — La app pinta el hilo (burbujas Ethos: Max a la derecha BRASA-tenue, Claude a la izquierda
  TINTA2) con refresco periódico (espejo del refresco de datos, `design.md` D2). Estado por mensaje
  (enviado/leído por el agente) reusando el timbrado del broker.
- **R8.4 (constraint dura)** — El chat privado **no** debe empujarse como `<channel>` (eso lo
  renderizaría en el TUI). Se entrega SOLO cuando el agente llama `chat_privado_recibir`. Esto lo hace
  "pull" (el agente pregunta), no "push" (aparece solo). Trade-off explícito: latencia (el agente ve el
  mensaje cuando consulta) a cambio de invisibilidad garantizada. Ver §7 para la alternativa "push".

### Transversal

- **R9 — Perfiles de lanzamiento persistidos.** Un perfil = { nombre, dir, destino(local/ssh host/tmux),
  system_prompt (o plantilla), tareas, flags }. Guardar/editar/borrar/duplicar. Se guardan en
  `config.toml` (mismo archivo que ya usa la app, `config.rs`). Relanzar un perfil = un click.
- **R10 — Todo degrada, nada paniquea.** Broker offline, SSH caído, PTY no disponible, picker cancelado
  (el `Option` es `None`): banner de error Ethos (`banner_error` ya existe), la app sigue viva. Sin
  `.unwrap()`/`.expect()` en prod (constraint del proyecto).
- **R11 — Refresco y estado vivo.** El estado de cada sesión lanzada (viva/muerta, registrada en el
  broker o no, id asignado) se refleja en la UI, reusando `/listar` para cruzar la sesión con su peer.

---

## 4. Diseño de UI (gpui-component real, tema Ethos)

Componentes del kit verificados como disponibles (`gpui-component`): `Input`, `Select`, `Button`,
`Switch`, `Tabs`, `Sheet`/`Dialog`/`Modal`, `Tooltip`, `Notification`, `Badge`, `Resizable`, `Scrollable`,
`Breadcrumb`. Helpers Ethos ya en `tema.rs`: `superficie_card`, `eyebrow`, `titulo`, `chip_estado`,
`boton_primario`, `boton_secundario`, `texto_terciario`, `fondo_app`, `fila_seleccionable`.

Layout propuesto (una variante; a decidir en Design):

```
┌ Lanzador ───────────────────────────────────────────────────────────┐
│ [ Configurar ]                          │  Perfiles  ▾ [Guardar] [▶] │
│  Directorio  /Users/max/proyecto  [Elegir…] [Revelar] [recientes ▾]  │
│  Destino     ( Local | SSH: otus ▾ | tmux: front-p2v )               │
│  System prompt  ┌──────────────────────────────────┐  plantilla ▾    │
│                 │ (textarea multilínea)            │                 │
│                 └──────────────────────────────────┘  0 chars        │
│  Tareas         + [descripción............] [30m ▾]  (lista editable) │
│  Flags avanz.   [switch] --dangerously-skip-permissions              │
│  Comando →      claude --append-system-prompt … --dangerously-load…  │
│                 [ Copiar ]              [  ● Lanzar sesión  ]         │
├─────────────────────────────────────────┬───────────────────────────┤
│  TERMINAL (PTY)  [tab: sesión-1][+]      │  CHAT PRIVADO con Claude   │
│  ╭ claude 2.1 · /Users/max/proyecto      │  ┌───────────────────────┐│
│  │ > trabajando en la tarea 1…           │  │ tú: revisa el módulo X ││
│  │ [render normal del harness/TUI]       │  │ claude: hecho, sin bug ││
│  │                                       │  │  (NO aparece a la izq) ││
│  ╰                                       │  └───────────────────────┘│
│                                          │  [ escribir…      ] [↵]   │
└──────────────────────────────────────────┴───────────────────────────┘
```

`Resizable` separa Terminal | Chat privado. `Tabs` para múltiples sesiones. El Chat privado es un
`Sheet` lateral togglable (para no robar ancho cuando no se usa).

---

## 5. Criterios de aceptación (definición de hecho)

- **AC1 (R1)** — "Elegir directorio" abre el diálogo nativo del SO; al elegir, la ruta aparece en la UI;
  cancelar (None) no rompe nada. Verificable con test mac-use.
- **AC2 (R2/R6)** — Escribir un system prompt y ver el comando previsualizado incluir
  `--append-system-prompt "<texto>"` exacto (escapado correctamente) + el flag de canal (R5).
- **AC3 (R4.3 tmux)** — Elegir destino tmux con nombre "s1" genera y ejecuta
  `tmux new-session -d -s s1 -c <dir> "claude …"` y el terminal embebido queda **attached** a s1;
  relanzar con el mismo nombre ofrece "attach" en vez de "new".
- **AC4 (R4.2 SSH)** — Elegir destino SSH host=otus lanza `ssh otus` en el PTY y dentro corre
  `cd <dir remoto> && claude …`; un host inalcanzable → banner de error, sin crash (R10).
- **AC5 (R7)** — El terminal embebido renderiza salida ANSI con color, acepta teclado, hace scroll,
  y sobrevive a resize de ventana. Cerrar la pestaña mata el PTY con confirmación (R7.1).
- **AC6 (R7.3)** — "Instalar MCP aquí" teclea el `claude mcp add claude-peers -- <ruta>` correcto
  para el destino activo (local vs host SSH).
- **AC7 (R8)** — Max escribe en el chat privado; el Claude de esa sesión, al llamar
  `chat_privado_recibir`, recibe el texto y responde con `chat_privado_responder`; la respuesta aparece
  en el panel de la app **y NO aparece en el pane del terminal** (verificable mirando ambos). (R8.4)
- **AC8 (R9)** — Guardar un perfil, cerrar la app, reabrir: el perfil persiste y relanza con un click.
- **AC9 (R10 degradación)** — Con el broker apagado, el PTY no disponible, o SSH caído, cada zona
  muestra su banner y el resto de la app sigue operable; sin `.unwrap` en prod (grep limpio).
- **AC10 (compat)** — Config viejo sin la sección `[lanzador]`/perfiles deserializa sin error
  (`#[serde(default)]`).

---

## 6. Decisiones y riesgos ABIERTOS (a resolver en Design, no aquí)

1. **Backend del terminal (R7) — el mayor riesgo de esfuerzo.** GPUI no trae terminal. Evaluar reusar
   `crates/terminal` de Zed (mismo rev ya fijado → sin conflicto de resolución que documenta el
   `Cargo.toml`) vs `alacritty_terminal`/`portable-pty`. Es la pieza más pesada; puede justificar
   partir la feature en 2 fases (Fase 1 sin terminal embebido = "solo preparar + Terminal externo";
   Fase 2 = PTY embebido).
2. **Binding id↔sesión (R3.1, R11).** El broker asigna el id del peer al registrarse (derivado de la
   carpeta), no lo fija la app. Para asignar tareas por broker (R3.1) o pintar el estado (R11), la app
   debe **correlacionar** la sesión que lanzó con el peer que aparece en `/listar`. Correlación por
   `cwd` + ventana temporal es frágil. Opción robusta: pasar un id explícito a la sesión (env var /
   flag) que el `peers-client` respete al registrarse — **toca el backend** (client + broker). Decidir.
   Interactúa con el **bug de colisión de ID** ya registrado en `STATE.md` (arreglarlo primero conviene).
3. **Chat privado push vs pull (R8.4).** Pull (el agente consulta) garantiza invisibilidad pero añade
   latencia. Un "push invisible" (que el mensaje llegue solo sin renderizarse) **depende del harness de
   Anthropic**, que hoy renderiza todo `<channel>` que pasa las 6 puertas (`troubleshooting.md:270`); no
   hay puerta "entregar sin renderizar". Mientras eso no exista, **pull es la única vía fiable**. La app
   puede mitigar la latencia recordando al agente por el terminal ("revisa tu chat privado") — pero eso
   sería visible. Trade-off a aprobar por Max.
4. **Seguridad del FS y de la ejecución.** La app pasa a **ejecutar procesos arbitrarios** (claude, ssh,
   tmux, instalar MCP) y a leer rutas del FS. Alcance: solo lo que Max elige explícitamente; sin
   auto-ejecución; comando siempre previsualizado (R6); `--dangerously-skip-permissions` OFF por defecto
   con aviso. Registrar en un log local de acciones del operador (espeja peers-17 del RFC de Peers).
5. **`de` del chat privado y del operador.** Coherente con la reserva de `"broker"`/id del operador que
   pide el fix de colisión de ID. Definir un `de` estable para "Max desde la desktop".

---

## 7. Chat privado — el fondo arquitectónico (integrado desde el análisis previo)

**Por qué no es una variante del `<channel>`:** todo el sistema actual empuja el `<channel>` a stdout de
la sesión `claude`, y el harness lo **renderiza** en el TUI cuando pasa las 6 puertas
(`peers-client/main.rs:709` `empujar_canal`; comentario `main.rs:723` "el `<channel>` se renderizó").
Lo que Max quiere es lo OPUESTO: hablar con el Claude **sin** que se pinte en el TUI. Por tanto el chat
privado es un **canal paralelo**, no una config del canal existente.

**Mecanismo elegido (R8): tools MCP dedicadas (pull).** El agente recibe el mensaje privado solo cuando
llama `chat_privado_recibir` y responde con `chat_privado_responder`. No se empuja como `<channel>` →
no se renderiza → invisible en el TUI. Es la **única vía que no depende del harness de Anthropic**.

**Cómo se hace natural (sin que Max microgestione):** las instrucciones del MCP (`mcp.rs::instrucciones`,
que ya se inyectan en cada sesión) enseñan al agente a consultar su chat privado con Max de forma
periódica y cuando el contexto lo sugiera, y a tratar ese canal como privado (no volcar su contenido al
output visible). Esto es "inyección de prompt al agente" en el sentido legítimo del proyecto: se guía el
comportamiento del peer vía el system prompt del MCP.

**Lo que queda para su propio brainstorming/Design (no se cierra en esta spec):** la cadencia exacta del
pull, si conviene un modo "híbrido" (recordatorio visible mínimo + contenido privado por tool), y la UX
de leído/entregado del hilo privado.

---

## 8. Constraints (del CLAUDE.md y del proyecto)

- Sin `.unwrap()`/`.expect()` en producción; `Result`/`anyhow`. Todo degrada.
- Español salvo claves de protocolo. El tiempo real lo timbra el broker (jamás la IA).
- **No añadir dependencias nuevas para el file picker** (es nativo de GPUI). El terminal embebido SÍ
  añadirá deps (decidir cuáles en Design — §6.1); mantener el criterio de portabilidad del binario.
- Reusar DTOs de `peers-core`; reusar endpoints existentes (`/tarea/asignar`, `/listar`, `/salir`) y
  el patrón `cx.spawn` + runtime tokio ya montado (`cliente.rs:75`). No reinventar.
- Versionar el plugin (bump) si se tocan binarios (client/broker por las tools/endpoints del chat privado).
- NUNCA `Co-Authored-By`. Jornada en el cuerpo del commit.

## 9. Fuera de alcance (esta spec)

- Editor de código / árbol de archivos completo dentro de la app (esto NO es un IDE; el file picker es
  solo para elegir el dir de arranque).
- Gestión de credenciales SSH (claves, agentes): se asume el `ssh` del sistema ya configurado por Max.
- El "push invisible" del chat privado (depende del harness de Anthropic — §6.3): se documenta como
  bloqueo externo, no se implementa.
- Multi-operador / colaboración concurrente en el mismo chat privado (YAGNI).

## 11. Arquitectura de implementación (verificada contra código y docs)

> Fuentes: checkout de Zed FIJADO por el proyecto (`~/.cargo/git/checkouts/zed-…/3648fe6`, = rev
> `1d217ee` del `Cargo.toml`), docs oficiales de Zed vía context7 (`/zed-industries/zed`), y la API
> GPUI leída del propio crate. Todo lo que sigue está comprobado en ese rev, no de memoria.

### 11.1 — Terminal embebido (R7): reusar el crate `terminal` de Zed

**Hallazgo clave:** el rev de Zed que este proyecto YA compila trae dos crates reutilizables:
`crates/terminal` (backend PTY + event loop) y `crates/terminal_view` (integración GPUI). Confirmado
que existen en el checkout fijado. Arquitectura (docs `terminal_view/README.md`):

- **`terminal.rs`** expone tipos de dominio **backend-neutral** (contenido, celdas, modos, comandos de
  scroll). El backend real es **`alacritty_terminal`** (dep del crate `terminal`), pero está aislado
  tras ese límite — la UI depende de los tipos neutrales, no de Alacritty. Esto significa que **no
  parseamos ANSI a mano**: Alacritty ya mantiene la rejilla de celdas.
- **`terminal_view.rs`** integra el `Terminal` en GPUI: input por `try_keystroke()` / `input()`, IME por
  `replace_text_in_range()`, y bindings `terminal::SendText` / `terminal::SendKeystroke` para inyectar
  texto/teclas (esto es lo que usaremos para "teclear por Max" el `claude mcp add …` de R7.3 y los
  comandos tmux/ssh).
- **`TerminalBuilder`** (proceso de 2 pasos por fallos de tty). Firma REAL verificada
  (`terminal.rs:1015`):
  ```rust
  TerminalBuilder::new(
      working_directory: Option<PathBuf>,   // ← el dir del file picker (R1)
      task: Option<TaskState>,
      shell: Shell,                         // ← aquí va `claude …` | `ssh host` | `tmux …`
      env: HashMap<String, String>,         // ← inyectamos el flag de canal por env si aplica
      cursor_shape, alternate_scroll, max_scroll_history_lines,
      path_hyperlink_regexes, path_hyperlink_timeout_ms,
      is_remote_terminal: bool,             // ← true para el destino SSH (R4.2)
      window_id: u64,
      completion_tx: Option<Sender<Option<ExitStatus>>>,  // ← notifica muerte de la sesión (R11)
      cx: &App,
      activation_script: Vec<String>,
      path_style: PathStyle,
  ) -> Task<Result<TerminalBuilder>>        // ← async: usa cx.background_executor() dentro
  ```
  → El spawn del PTY corre en `cx.background_executor()` (NO bloquea el hilo de UI, R7). El
  `completion_tx` da el hook para marcar la sesión como muerta en la UI.
- **Fallback R7.2:** existe `TerminalBuilder::new_display_only(...)` — un terminal de solo-render sin
  PTY. Sirve para plataformas sin tty (el propio código contempla `HeadlessTerminal` / `ENOTTY`): si el
  PTY falla, se degrada a "solo preparar + Terminal externo" sin crashear.

**Coste / riesgo (para Design):** el crate `terminal` arrastra deps (`alacritty_terminal`, y una
dependencia oculta detectada: `release_channel::AppVersion::global(cx)` en `TerminalBuilder::new` →
habría que proveer ese global o recortar esa ruta). Dos caminos:
  - **(A) Consumir `terminal` + `terminal_view` de Zed tal cual** (mismo rev → sin conflicto de
    resolución, que es justo lo que el `Cargo.toml` del desktop ya cuida). Máxima madurez, más deps.
  - **(B) Solo `alacritty_terminal` + `portable-pty`** y render de rejilla propio en GPUI. Menos deps
    arrastradas, pero reimplementamos lo que `terminal_view` ya da (input, IME, hyperlinks, scroll).
  Recomendación: **(A)**, y si el peso de deps rompe el criterio de binario portable, reevaluar (B).
  Decidir en Design con un spike de compilación (medir qué arrastra `terminal` sobre el binario).

### 11.2 — File picker nativo (R1): `cx.prompt_for_paths`

API GPUI verificada (`app.rs:1382`), **cero dependencias nuevas**:
```rust
let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false });
cx.spawn(async move |esta, cx| {
    match rx.await {                       // oneshot::Receiver<Result<Option<Vec<PathBuf>>>>
        Ok(Ok(Some(paths))) => { /* paths[0] = dir elegido → mutar estado con esta.update */ }
        Ok(Ok(None))        => { /* cancelado: no-op, sin error */ }
        _                   => { /* error del picker (Linux): banner, sin panic */ }
    }
}).detach();
```
Complementos nativos: `cx.reveal_path(&path)` (Finder) y `cx.open_with_system(&path)` (R1.1, R7.2).

### 11.3 — Lanzar procesos (R4) sin bloquear la UI

La app **ya** tiene runtime tokio propio para el cliente HTTP (`cliente.rs:75` documenta que los
`cx.spawn`/`cx.background_spawn` de GPUI NO corren sobre tokio, por eso montan un runtime). Para los
procesos hijos hay dos vías, según se use el crate `terminal` o no:
- **Con `terminal` (recomendado):** NO lanzamos `std::process` a mano — el PTY lo gestiona
  `TerminalBuilder` vía `Shell` en el `background_executor` de GPUI. Es la vía canónica y la que da
  render + input gratis.
- **Sin `terminal` (modo "solo preparar" / Terminal externo):** `smol::process::Command` (GPUI usa smol
  en su executor) o el runtime tokio ya montado, en `cx.background_spawn`, capturando stdout/exit.
  Nunca en el hilo de UI.

### 11.4 — SSH y tmux (R4.2 / R4.3) desde el PTY

Tres composiciones de `Shell` sobre el mismo terminal embebido:
- **Local:** `Shell::WithArguments { program: "claude", args: [flags…] }` con `working_directory = dir`.
- **SSH:** `program: "ssh"`, `args: ["-t", host]`, y el comando remoto `cd <dir> && claude <flags>` como
  parte del args (o tecleado por `SendText` tras el prompt). `is_remote_terminal: true` en el builder.
  El `-t` fuerza pseudo-tty remoto (necesario para el TUI de `claude`).
- **tmux:** primero `tmux new-session -d -s <nombre> -c <dir> "claude <flags>"` (detached), luego
  `tmux attach -t <nombre>` en el PTY embebido. Si la sesión existe, `attach` directo. Sobre SSH:
  `ssh -t host tmux attach -t <nombre>`. El "detached + attach" replica el runbook de orquestación de
  Max (`~/.claude/docs/multi-claude-tmux.md`) y sobrevive al cierre de la app (la sesión tmux sigue viva
  en el host — propiedad deseada para "salir de perto").
- La regla del Enter de tmux (`send-keys … Enter` como arg separado) aplica si en algún flujo se envían
  prompts por `SendText`/`SendKeystroke` en vez de por el arg del `new-session`.

### 11.5 — Canal invisible del chat privado (R8): por qué "pull" y no "push"

Verificado en el código del proyecto: el push `<channel>` SIEMPRE se renderiza cuando pasa las 6 puertas
del harness (`troubleshooting.md:270`; `peers-client/main.rs:709,723`). No hay puerta "entregar sin
renderizar" — es decisión del harness de Anthropic, fuera de nuestro control. Por tanto:
- **El chat privado NO puede usar el mecanismo `<channel>`.** Debe ser un canal donde el agente **tira**
  (pull) del mensaje vía una tool MCP (`chat_privado_recibir`), no donde el mensaje **aparece** (push).
- Las tools MCP se declaran en `peers-client/mcp.rs` (mismo sitio que las 9 tools actuales) y el broker
  guarda la cola durable (patrón bandeja ZSET ya existente). El agente responde con
  `chat_privado_responder`, que el broker timbra y la app pinta.
- El comportamiento "consulta tu chat privado" se induce por el **system prompt del MCP**
  (`mcp.rs::instrucciones`, que ya se inyecta en cada sesión) — la "inyección de prompt al agente" que
  Max describía, en su forma legítima y soportada.
- **Límite honesto:** con "pull", el agente ve el mensaje cuando consulta → hay latencia. Un "push
  invisible" exigiría cambios en el harness de Anthropic (no disponibles). Trade-off aprobado como
  parte de esta spec; la cadencia del pull se afina en Design.

> **Nota:** se lanzó un deep-research (fan-out web + verificación adversarial) para contrastar con
> fuentes externas, pero **falló por un error interno del harness** (retry cap de StructuredOutput),
> no por la consulta. La arquitectura de arriba NO depende de él: está toda verificada contra fuente
> de primera mano — el checkout de Zed FIJADO por el proyecto (`3648fe6` = rev `1d217ee`) + docs
> oficiales de Zed vía context7. Si se quiere el barrido web adicional, re-lanzar el workflow.

## 10. Dependencias con otras specs / backlog

- **Backend (client + broker):** tools `chat_privado_*` + endpoints `/chat-privado/*` + cola durable
  (R8.1–R8.3) → toca `peers-client/mcp.rs`, `peers-broker` (store/main), `peers-core` (DTOs). Puede
  salir como sub-spec de backend si se prefiere separar del frontend GPUI.
- **Bug de colisión de ID** (`STATE.md`): conviene resolverlo antes del binding id↔sesión (§6.2).
- **RFC Acceso** (`.specs/desktop/acceso/`): reusar su multi-broker/hosts para el destino SSH (R4.2).
- **desktop-carga-datos** (`.specs/features/desktop-carga-datos/`): reusar su patrón de refresco (D2)
  para el chat privado (R8.3) y el estado de sesiones (R11).

---
#rfc #peers-desktop #lanzador #terminal #chat-privado #ssh #tmux
