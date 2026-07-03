# Pipeline de provisión — "de cero a agente operativo" (máquina de estados)

> ⬆ [[_MOC|Mapa]] · Modelo: [[01-modelo-corporativo]] · Dominio: [[02-modelo-dominio]] ·
> Conocimiento: [[04-conocimiento-agente]] · GPUI: [[00-fundamentos-gpui]]
>
> Fecha: 2026-07-03. Estado: **ARQUITECTURA — para revisar antes de codificar.**
> Este es el documento que faltaba: **los pasos concretos que el código debe ejecutar** desde "Max hace
> clic" hasta "el agente está trabajando con su conocimiento del negocio". No es prosa conceptual: cada
> paso tiene precondición, acción (endpoint/proceso real con `archivo:línea`), evento de trazabilidad,
> modo de fallo/rollback e idempotencia. Verificado contra el pipeline REAL del código (Fases A-G abajo).

---

## 0. Por qué este documento

Max: *"hasta llegar al agente son VARIOS PASOS que el código tiene que tener para que podamos crear estas
features; eso no fue una arquitectura."* Correcto. El modelo corporativo ([[01-modelo-corporativo]])
describía el *qué*; faltaba el *cómo mecánico*. Aquí está el pipeline como una **máquina de estados del
ciclo de vida del agente**, análoga a `EstadoTarea` que ya existe en `peers-core`
(`lib.rs:276`, Abierta→EnCurso→…). Un agente no "aparece": **atraviesa estados discretos**, cada uno con
código que lo hace avanzar o lo degrada sin romper.

Este pipeline se apoya en lo que ya funciona (registro atómico, latido, push, jornada) y **cierra los 9
huecos** (H1-H9) que hoy impiden operar como empresa. Cada hueco se marca con el paso que lo resuelve.

---

## 1. El pipeline REAL de hoy (verificado — punto de partida)

Antes de diseñar, el mecanismo actual (de la exploración del código, con `archivo:línea`):

**Fase A — Instalación (una vez/máquina):** `install.sh:141-206` hace `claude mcp add … claude-peers --
<peers-client>` inyectando `-e CLAUDE_PEERS_BROKER_URL` y `-e CLAUDE_PEERS_TOKEN` (`install.sh:158-160`).
La función shell `claude(){ command claude --dangerously-load-development-channels server:claude-peers
"$@"; }` (`install.sh:232`) añade el flag de canal. Vía plugin: `plugin/.mcp.json` con `env:{}` **vacío**
(`command=${CLAUDE_PLUGIN_ROOT}/bin/peers-client-launcher`).

**Fase B — Broker (daemon):** `peers-broker/main.rs:1694` arranca, monta `Almacen` (Redis/SQLite), carga
política y bitácora, lanza el **barrido cada 30s** (`main.rs:1785-1822`: `limpiar_vencidas`,
`podar_*`, `detectar_alertas`), escucha con token salvo `/salud`.

**Fase C — Client (por sesión):** `peers-client/main.rs:66` lee env **una vez** (`Args::parse()`,
`main.rs:75`), deriva `id_efectivo` = `--id`/`CLAUDE_PEERS_ID` **si vino**, si no
`contexto::id_desde_directorio` (`main.rs:85-88`), resuelve `repo_git`/`repo_github`/`tty`/`hostname`,
chequea salud (no bloqueante), **registra** (`main.rs:110-131`, `id_preferido=Some(id_efectivo)`), lanza
3 tareas de fondo: **latido 15s** (`main.rs:145`), **recepción 1s** (`main.rs:148`), **señales**
(`main.rs:151`), y entra al **bucle stdin MCP** (`main.rs:159`).

**Fase D — Handshake MCP:** `initialize` → `resultado_initialize(version, id_anunciado)`
(`mcp.rs:231`) devuelve capabilities `claude/channel`, `serverInfo.name="claude-peers"` (CRÍTICO,
`mcp.rs:249`) e **`instructions`** (`mcp.rs:262`, estático, sin negocio). `tools/list` → 9 tools
(`mcp.rs:110-225`). `tools/call` → `ejecutar_tool` (`main.rs:237-264`).

**Fase E — Registro atómico (broker):** `registrar` (`peers-broker/main.rs:321-348`): toma `registro_lock`
(anti-TOCTOU), `resolver_id_sin_colision` (sufija `-2..-99` si otro peer vivo lo ocupa,
`main.rs:257-264`), `almacen.registrar` (`SADD cprs:instancias` idempotente, `store.rs:166-171`),
`jornada::abrir_sesion` (`main.rs:345`). Liveness caduca a `VENCIMIENTO_MS=45_000` (`lib.rs:21`).

**Fase F — Mensajería:** `enviar` (`main.rs:469-518`) valida existencia → **evalúa política en memoria**
(`main.rs:465-467`, punto donde entrará la validación de cadena de mando) → `encolar_mensaje`
(ZSET bandeja/historial, `store.rs:278-307`). Recepción: peek no-destructivo cada 1s + `empujar_canal`
(`mcp.rs:83-105`, 4 claves `meta` en inglés) + confirma `Entregado`/`Leido` solo si el flush triunfó.

**Fase G — Jornada:** inicio en `/registrar`, fin en `/salir`; el reloj lo pone SIEMPRE el broker
(`jornada.rs:3-6`).

**Los 9 huecos** (lo que impide operar como empresa): H1 la vía plugin nunca inyecta `CLAUDE_PEERS_ID`
(`env:{}`); H2 el id se lee una sola vez (no hay renombrado en caliente); H3 la derivación saneala `@`→`-`
(`contexto.rs:139-146`) → `rol@proyecto` imposible por derivación; H4 el id anunciado en `initialize` puede
no ser el real (ventana de carrera con el sufijo); H5 el flag de canal es manual salvo `install.sh`;
**H6 `--append-system-prompt` NO existe en el runtime** (solo en specs); H7 el id colapsa sin TTY
(headless); H8 la exención de política confía en el `de_id` declarado (spoofable); H9 el arranque del
broker es best-effort por hook.

---

## 2. La máquina de estados del AGENTE (el marco)

Un agente atraviesa estos estados. Cada transición es código con precondición y traza. Análoga a
`EstadoTarea` (misma filosofía: estados serde, transiciones válidas, terminal).

```
        (Max define)         (Max contrata)      (app provisiona)     (app lanza)
DEFINIDO ──────────► CONTRATADO ──────────► PROVISIONADO ──────────► LANZADO
   cargo                agente                 requisitos OK           proceso claude
 (plantilla)         rol@proyecto            (id,canal,prompt,cwd)     corriendo
                                                                          │ (registro atómico)
                                                                          ▼
   PAUSADO ◄──────── OCIOSO ◄──────── VIVO ◄──────────────────────── REGISTRADO
   (kick /salir)   (supervisor)   (latido<45s)   (en /listar, jornada abierta)
      │                                                                    
      ▼                                                                    
  DADO_DE_BAJA (jornada cerrada, SREM instancias)                         
```

Estados y su sustrato real:

| Estado | Qué significa | Sustrato / verificación |
|--------|---------------|-------------------------|
| **Definido** | existe el **cargo** (plantilla), aún no hay empleado | `Cargo` en el store organizativo (broker) |
| **Contratado** | existe el **agente** `rol@proyecto`, sin proceso | `Agente` persistido en el organigrama |
| **Provisionado** | requisitos de lanzamiento OK (id, canal, prompt, cwd, knowledge) | checklist §3.4 (cierra H1/H5/H6) |
| **Lanzado** | el proceso `claude …` corre (PTY/ssh/tmux) | RFC Lanzador; `completion_tx` avisa muerte |
| **Registrado** | el peer se registró en el broker con su id | `registrar` OK (`main.rs:321`), jornada abierta |
| **Vivo** | latió < 45s, aparece en `/listar` | `latido` (`main.rs:350`), `VENCIMIENTO_MS` |
| **Ocioso** | vivo sin tarea > umbral | supervisor `TipoAlerta::Ocioso` (`lib.rs:677`) |
| **Pausado** | expulsado por el operador | `/salir` (kick), reversible (relanzar) |
| **Dado de baja** | jornada cerrada, fuera de `cprs:instancias` | `store.rs:187-192` |

> **Nota durabilidad:** Definido/Contratado son **identidad durable** (sobreviven al proceso; viven en el
> store organizativo + `bitacora.db`/`peers_conocidos`). Registrado→Vivo→Ocioso son **presencia efímera**
> (el peer). El pipeline es el puente entre ambas: **contratar** crea identidad durable; **lanzar** crea
> presencia; el binding `rol@proyecto` (§3.3) las une.

---

## 3. El pipeline paso a paso (cada paso: precondición · acción · traza · fallo · idempotencia)

Formato fijo por paso. "Traza" = evento de bitácora emitido (`AccionRegistrada { quien, accion, sujeto,
cuando }`, `lib.rs:1045`); donde no exista `TipoAccion`, se propone una variante nueva (el enum es
`#[non_exhaustive]`, `lib.rs:1007` → añadir no rompe).

### Fase 1 — Crear proyecto (Definir el contenedor)

**Paso 1.1 — Crear proyecto**
- **Precondición:** Max autenticado como operador (`ID_OPERADOR`); nombre de proyecto único.
- **Acción:** `POST /admin/proyecto { id (slug), nombre, ubicacion }` (NUEVO). Persiste `Proyecto`
  ([[02-modelo-dominio]] §2.2) en el store organizativo del broker (por decisión de **regla dura + fuente
  broker**, no solo `config.toml`). El broker timbra `creado_en`.
- **Traza:** `AccionRegistrada { quien: operador, accion: CrearProyecto (nueva), sujeto: proyecto_id }`.
- **Fallo:** id duplicado → `ok:false` de negocio (no 500), como `enviar` con destino inexistente
  (`main.rs:474-479`). Broker offline → banner en la UI, sin persistir (reintentable).
- **Idempotencia:** crear con id existente = no-op idempotente (upsert de metadatos), patrón `SADD`
  (`store.rs:166-171`).

**Paso 1.2 — Resolver y validar la ubicación**
- **Precondición:** proyecto creado; ubicación declarada (Local/Ssh/Tmux, [[02-modelo-dominio]] §2.2).
- **Acción:** si `Local` → validar que la carpeta existe y es dir (el picker nativo ya devolvió una ruta
  real, `cx.prompt_for_paths`, [[00-fundamentos-gpui]] §6). Si `Ssh` → `ssh <host> test -d <carpeta>`
  (host de la lista de RFC Acceso). Si `Tmux` → resolver host + nombre de sesión.
- **Traza:** ninguna (validación de lectura; no muta).
- **Fallo:** carpeta inexistente / host inalcanzable → banner Ethos, "Lanzar" deshabilitado (R13 de las
  RFCs). Nunca panic.
- **Idempotencia:** revalidar es seguro (solo lectura).

### Fase 2 — Definir cargo y componer el system prompt (el "contrato")

**Paso 2.1 — Definir/editar el cargo (plantilla)**
- **Precondición:** operador; el cargo tiene `system_prompt` no vacío.
- **Acción:** `POST /admin/cargo { id, nombre, system_prompt, departamento?, reporta_a?,
  puede_delegar_a[], capacidades[] }` (NUEVO). Persiste `Cargo` ([[02-modelo-dominio]] §2.1) en el store
  organizativo. **Por regla dura, el broker guarda la cadena de mando** para validar después.
- **Traza:** `AccionRegistrada { quien: operador, accion: DefinirCargo (nueva), sujeto: cargo_id }`.
- **Fallo:** `reporta_a`/`puede_delegar_a` apuntan a cargos inexistentes → `ok:false` con el detalle
  (validación de integridad del organigrama). Ciclo en la cadena de mando → rechazar (§Paso 2.1a).
- **Idempotencia:** re-guardar el mismo cargo = upsert.

**Paso 2.1a — Validar el organigrama (integridad de la cadena)**
- **Precondición:** cambio en `reporta_a`/`puede_delegar_a`.
- **Acción:** el broker verifica que la relación `reporta_a` no forme ciclo (DFS sobre el grafo de cargos)
  y que los destinos existan. Es el prerequisito de la **regla dura** de delegación.
- **Traza:** ninguna si pasa; si falla, el rechazo se registra como intento inválido (opcional).
- **Fallo:** ciclo detectado → `ok:false` "la cadena de mando formaría un ciclo".
- **Idempotencia:** validación pura, repetible.

**Paso 2.2 — Componer el system prompt (el ENSAMBLAJE — núcleo)**
- **Precondición:** cargo definido; proyecto conocido; organigrama disponible.
- **Acción:** la app compone el texto que irá en `--append-system-prompt` (Canal A). Es una
  **concatenación por capas** (detalle en [[04-conocimiento-agente]] §2):
  1. **Identidad:** "Eres `<cargo.nombre>` en el proyecto `<proyecto.nombre>`. Tu id de red es
     `<rol@proyecto>`."
  2. **Regla de negocio:** el `cargo.system_prompt` íntegro.
  3. **Cadena de mando:** "Reportas a `<reporta_a>`. Puedes delegar en `<puede_delegar_a>`. Escala
     bloqueos a `<reporta_a>`." (de los datos del cargo).
  4. **Capacidades:** "Puedes: `<capacidades>`." (y, por regla dura, el broker las hará cumplir).
- **Traza:** ninguna (composición local; se registra al lanzar).
- **Fallo:** cargo sin `system_prompt` → usar solo capas 1/3/4 (degradación, el prompt nunca queda vacío).
- **Idempotencia:** función pura del (cargo, proyecto, organigrama) → mismo prompt.

> **Cierra parte de H6:** aquí se **construye** el system prompt. El paso 5.1 lo **inyecta** con el flag
> `--append-system-prompt` que hoy no existe en el runtime.

### Fase 3 — Contratar el agente (identidad durable)

**Paso 3.1 — Contratar (instanciar el cargo en el proyecto)**
- **Precondición:** cargo y proyecto existen.
- **Acción:** `POST /admin/agente { cargo_id, proyecto_id, ubicacion? }` (NUEVO). El broker **compone el id
  `rol@proyecto`** (§3.3), lo registra en el organigrama con estado `Contratado`, y lo upserta en
  `peers_conocidos` de `bitacora.db` (identidad durable, ADR-001 desktop). No arranca proceso.
- **Traza:** `AccionRegistrada { quien: operador, accion: ContratarAgente (nueva), sujeto: rol@proyecto }`.
- **Fallo:** cargo/proyecto inexistente → `ok:false`. Colisión de id → sufijar (`qa-2@proyecto-x`).
- **Idempotencia:** contratar dos veces el mismo (cargo, proyecto) con la misma intención = el mismo id
  (idempotente); si Max quiere DOS agentes del mismo cargo, es explícito (botón "otro") → sufijo.

**Paso 3.2 — (implícito) el agente queda en el organigrama**
- El agente `Contratado` ya es visible en el organigrama ([[05-organigrama-visual-ethos]]) con su nodo
  "apagado" (no vivo). Max puede definir el equipo entero antes de lanzar nada.

**Paso 3.3 — Componer el id `rol@proyecto` (cierra H3)**
- **Regla:** `id = sanear(cargo_id) + "@" + sanear(proyecto_id)`, p.ej. `backend@proyecto-x`. **Decisión
  de dominio:** el `@` se conserva como separador legible **en la capa organizativa**; para el transporte,
  el broker acepta el `@` (no lo pasa por `contexto::sanear`, que es solo para ids **derivados**). El id
  explícito viaja por `CLAUDE_PEERS_ID` y el client lo respeta tal cual (`main.rs:85-88`) — **no** pasa por
  la derivación que saneala `@`→`-`. Ver [[06-decisiones]] ADR sobre el separador.
- **Colisión:** el registro atómico del broker sufija (`-2`) si dos agentes chocan (`main.rs:257-264`),
  ahora sobre ids legibles.

### Fase 4 — Provisionar (los "varios pasos" que faltaban — cierra H1/H5/H6/H7)

Este es el corazón de la crítica de Max. **Antes de lanzar, el código debe garantizar 6 requisitos.** Es
un **checklist idempotente** (cada ítem: comprobar → reparar si falta → verificar).

**Paso 4.1 — `claude` instalado y accesible**
- Comprobar `command -v claude` (local) o `ssh <host> command -v claude` (remoto). Si falta → banner con
  instrucción, no lanzar. (No lo instala la app; lo reporta.)

**Paso 4.2 — MCP `claude-peers` registrado en el destino**
- Comprobar que `claude mcp list` incluye `claude-peers` (nombre EXACTO, `mcp.rs:249`). Si falta → ofrecer
  "Instalar MCP aquí" (RFC Lanzador R7.3: teclea `claude mcp add claude-peers -- <ruta>` en el PTY).
- **Traza:** `AccionRegistrada { accion: ProvisionarMcp (nueva), sujeto: host }`.

**Paso 4.3 — Inyectar `CLAUDE_PEERS_ID = rol@proyecto` (cierra H1/H3)**
- **El hueco central.** Hoy la vía plugin nunca lo pasa (`env:{}`). La app, al lanzar, exporta
  `CLAUDE_PEERS_ID=<rol@proyecto>` en el entorno del proceso `claude` (o lo pasa como `-e` en el
  `claude mcp add` del destino, o vía `TerminalBuilder.env` del PTY, [[00-fundamentos-gpui]] §6). El client
  ya lo respeta (`main.rs:85-88`). **Sin este paso, no hay identidad `rol@proyecto` y todo el modelo
  corporativo se cae** — por eso es el bloqueante #1.

**Paso 4.4 — Flag de canal presente (cierra H5)**
- Garantizar `--dangerously-load-development-channels server:claude-peers` en el comando de lanzamiento (la
  app lo añade sola, RFC Lanzador R5). Sin él, el push `<channel>` se descarta (`mcp.rs:246-248`).

**Paso 4.5 — System prompt compuesto disponible (cierra H6)**
- El texto del Paso 2.2 se pasa como `--append-system-prompt "<compuesto>"`. **Este flag NO existe hoy en
  el runtime** — la app de lanzamiento (RFC Lanzador) es quien lo introduce. Escapado correcto del texto
  (RFC Lanzador AC2).

**Paso 4.6 — cwd y archivos de conocimiento del proyecto listos (parte híbrida)**
- Garantizar que el cwd es la carpeta del proyecto y que existen los **archivos de conocimiento del repo**
  (`.lexusfx/empresa.md`, `proyecto.md`, `cargo.md`, [[04-conocimiento-agente]] §2). Si faltan, la app los
  **materializa** (los escribe desde el organigrama del broker + la plantilla del cargo) antes de lanzar.
  Es la mitad "repo" del modelo híbrido de conocimiento.
- **Traza:** `AccionRegistrada { accion: MaterializarConocimiento (nueva), sujeto: proyecto_id }`.

> **Resultado de la Fase 4:** el agente pasa a **Provisionado**. Los 6 ítems son el "código que tiene que
> tener" entre contratar y lanzar. Cada uno es comprobar→reparar→verificar (idempotente): relanzar no
> duplica nada.

### Fase 5 — Lanzar (crear la presencia)

**Paso 5.1 — Arrancar el proceso**
- **Precondición:** agente Provisionado (checklist 4.1-4.6 verde).
- **Acción:** RFC Lanzador arranca `claude --append-system-prompt "<compuesto>"
  --dangerously-load-development-channels server:claude-peers` con `env CLAUDE_PEERS_ID=<rol@proyecto>` en
  la ubicación (PTY local / `ssh -t host` / `tmux new-session`). El PTY corre en `background_executor`
  ([[00-fundamentos-gpui]] §5); `completion_tx` notifica la muerte del proceso (RFC Lanzador §11.1).
- **Traza:** `AccionRegistrada { quien: operador, accion: LanzarAgente (nueva), sujeto: rol@proyecto,
  detalle: ubicacion }`.
- **Fallo:** proceso no arranca / SSH cae → banner, estado vuelve a `Provisionado` (rollback natural: sin
  proceso). Sin crash (R10 RFC Lanzador).
- **Idempotencia:** relanzar un agente ya vivo → detectar por `/listar` y ofrecer "attach"/"ya vivo" en vez
  de duplicar (RFC Lanzador AC3 tmux).

### Fase 6 — Registrar y estabilizar la identidad (cierra H4)

**Paso 6.1 — Registro atómico en el broker**
- **Precondición:** el client arrancó (`main.rs:66`).
- **Acción:** el client hace `POST /registrar` con `id_preferido = CLAUDE_PEERS_ID` (`main.rs:110-131`);
  el broker resuelve sin colisión (`main.rs:329`) y abre jornada (`main.rs:345`). El agente pasa a
  **Registrado**.
- **Traza:** el broker ya loguea "instancia registrada" (`main.rs:346`); añadir evento de bitácora
  `accion: RegistrarAgente (nueva)` para el feed.
- **Fallo:** broker offline → el client no aborta, reintenta por latido (`main.rs:128`, `630-655`). El
  agente queda `Lanzado` pero no `Registrado` hasta que el broker vuelve.
- **Idempotencia:** re-registro del mismo peer (mismo host+pid) NO colisiona (`main.rs:289-295`); hereda
  su cola.

**Paso 6.2 — Cerrar la ventana de carrera del id anunciado (cierra H4)**
- **Problema real:** en `initialize` se anuncia `estado.id` real si existe, si no el preferido
  (`main.rs:201-206`); si el broker sufijó (`-2`) y el `initialize` llega antes de completar el registro,
  el agente recibe un id que no es el suyo.
- **Acción propuesta:** dado que con `rol@proyecto` explícito la colisión casi nunca ocurre (ids únicos por
  diseño), el riesgo se minimiza; además, el client puede **re-anunciar** el id real por una línea de
  `instructions`/tool tras estabilizar (`estado.id` se actualiza en el latido, `main.rs:652`). Ver
  [[06-decisiones]] ADR H4.
- **Idempotencia:** re-anunciar el mismo id es inocuo.

### Fase 7 — Boot del agente: cargar conocimiento y materializar tareas

**Paso 7.1 — Cargar el conocimiento (Canal B enriquecido)**
- **Precondición:** MCP handshake en curso (`initialize`).
- **Acción:** `resultado_initialize` (`mcp.rs:231`) devuelve `instructions` **enriquecidas** (hoy
  estáticas): el client, al construirlas, consulta al broker `GET /contexto-empresa` + `GET /mi-cargo?id=`
  + `GET /mi-organigrama?id=` (NUEVOS) y **inyecta** empresa + proyectos + organigrama + cargo. Además, el
  agente puede leer los archivos `.lexusfx/*` del cwd (parte repo). Detalle completo en
  [[04-conocimiento-agente]].
- **Traza:** ninguna (lectura de arranque).
- **Fallo:** broker no responde el conocimiento → `instructions` caen a la versión mínima actual
  (degradación); el agente sigue operativo con menos contexto.
- **Idempotencia:** cada `initialize` recompone el conocimiento fresco.

**Paso 7.2 — Materializar tareas iniciales**
- **Precondición:** agente Registrado + conocimiento cargado.
- **Acción:** las tareas del perfil de lanzamiento se crean vía `POST /tarea/asignar { instancia_id:
  rol@proyecto, descripcion, estimado_seg? }` (endpoint EXISTENTE). Ahora el binding `rol@proyecto` hace
  que **la app sepa exactamente a qué peer asignar** (resuelve el binding id↔sesión que RFC Lanzador §6.2
  dejó abierto).
- **Traza:** `AccionRegistrada { quien: operador, accion: AsignarTarea (existe), sujeto: tarea_id }`.
- **Fallo:** peer aún no registrado → reintentar tras liveness (Paso 6.1) o materializar por prompt (RFC
  Lanzador R3.2, fallback).
- **Idempotencia:** las tareas llevan id único del broker; reintentar no duplica si se guarda el `tarea_id`.

### Fase 8 — Operativo (Vivo)

El agente late (15s), aparece en `/listar`, tiene jornada abierta, conoce el negocio, ficha sus tareas
(tools `crear_tarea`/`cerrar_tarea`, el broker mide el real), colabora por `<channel>` (respetando la
política + la cadena de mando validada server-side), y es supervisado (alertas). Todo lo posterior es el
**workflow trazable** de operación ([[07-workflow-trazable-164-features]]).

---

## 4. Mapa "hueco → paso que lo cierra"

| Hueco (hoy) | Paso del pipeline que lo cierra |
|-------------|--------------------------------|
| **H1** vía plugin no inyecta `CLAUDE_PEERS_ID` | 4.3 (la app exporta el id al lanzar) |
| **H2** id se lee una vez, sin renombrado | fuera de alcance del lanzamiento; relanzar cambia rol (aceptado) |
| **H3** derivación saneala `@`→`-` | 3.3 (id explícito no pasa por la derivación) |
| **H4** id anunciado ≠ id real (carrera) | 6.2 (ids únicos por `rol@proyecto` + re-anuncio) |
| **H5** flag de canal manual | 4.4 (la app lo añade siempre) |
| **H6** `--append-system-prompt` inexistente | 2.2 (compone) + 4.5 (inyecta) — vía RFC Lanzador |
| **H7** id colapsa sin TTY (headless) | 3.3 (id explícito, no depende de TTY) |
| **H8** exención de política spoofable | prerequisito de regla dura → [[06-decisiones]] (anti-spoofing) |
| **H9** arranque broker best-effort | Paso 1/Fase B; el broker es infra, no del ciclo del agente |

---

## 5. Endpoints nuevos que este pipeline requiere (a diseñar; regla dura)

Consecuencia de "**fuente broker + regla dura**": el broker gana un **store organizativo**. Endpoints
(bajo token, en `rutas_protegidas`):

| Endpoint | Verbo | Para | Paso |
|----------|-------|------|------|
| `/admin/proyecto` | GET/POST | CRUD de proyectos | 1.1 |
| `/admin/cargo` | GET/POST | CRUD de cargos (con validación de cadena) | 2.1/2.1a |
| `/admin/agente` | GET/POST | contratar/listar agentes | 3.1 |
| `/admin/organigrama` | GET | el grafo completo (cargos+agentes+relaciones) | 3.2, 05 |
| `/contexto-empresa` | GET | empresa + catálogo de proyectos (conocimiento boot) | 7.1 |
| `/mi-cargo?id=` | GET | cargo+cadena+capacidades de un agente | 7.1 |
| `/mi-organigrama?id=` | GET | vista del organigrama desde un agente | 7.1 |

Persistencia: trait `Almacen` (Redis + SQLite, como `Politica`), + identidad durable en `bitacora.db`
donde aplique. Detalle de tipos: [[02-modelo-dominio]]. Detalle de qué sirve cada endpoint al agente:
[[04-conocimiento-agente]].

---

## 6. Constraints

- Sin `.unwrap()`/`.expect()` en prod; `Result`/`anyhow`. Español salvo protocolo. Redis + SQLite (ambos).
  El tiempo lo timbra el broker (jornada, `cuando` de bitácora). Cada paso que muta **registra** en la
  bitácora (`AccionRegistrada`); las variantes nuevas de `TipoAccion` son compat (`#[non_exhaustive]`,
  `lib.rs:1007`). Reusa DTOs de `peers-core` (nada duplicado). El id explícito viaja por `CLAUDE_PEERS_ID`
  (ya soportado). Versionar el plugin al tocar binarios (client/broker por endpoints/tools nuevos). NUNCA
  `Co-Authored-By`. Jornada en el commit.

## 7. Fuera de alcance (de este documento)

- El diseño de la UI del pipeline (wizard de contratación+lanzamiento) → [[08-capa-ui]].
- El detalle del conocimiento inyectado y las tools nuevas → [[04-conocimiento-agente]].
- El detalle visual del organigrama → [[05-organigrama-visual-ethos]].
- El mecanismo de terminal/PTY/SSH/tmux → [[../desktop/lanzador/RFC-lanzador]] (ya especificado).
- Renombrado de un peer vivo (H2) — reiniciar la sesión con otro rol es aceptable en v1.

---
#empresa #pipeline #provision #maquina-estados #ciclo-vida-agente #trazabilidad
