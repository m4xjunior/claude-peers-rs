# Decisiones de arquitectura — empresa de agentes (todas, resueltas)

> ⬆ [[_MOC|Mapa]] · Modelo: [[01-modelo-corporativo]] · Pipeline: [[03-pipeline-provision]] ·
> Conocimiento: [[04-conocimiento-agente]] · Dominio: [[02-modelo-dominio]]
>
> Fecha: 2026-07-03. Estado: **DECIDIDO** (salvo lo marcado "PENDIENTE" — requiere validación de Max).
> El "detallar TODAS las decisiones" que pidió Max. Cada una en formato ADR ligero (MADR-es):
> **contexto · opciones · decisión · consecuencias · reversibilidad**. Consagra las 4 decisiones de
> producto de Max y resuelve los 9 huecos (H1-H9) + las decisiones abiertas de las RFCs.

**Índice**

| # | Decisión | Estado |
|---|----------|--------|
| E-01 | Conocimiento en el boot: RICO | DECIDIDO (Max) |
| E-02 | Fuente de la verdad: híbrida (broker + repo) | DECIDIDO (Max) |
| E-03 | Cumplimiento: regla dura (broker valida) | DECIDIDO (Max) |
| E-04 | Store organizativo en el broker | DECIDIDO (deriva de E-03) |
| E-05 | Identidad de agente `rol@proyecto` + provisión del id (H1/H3/H7) | DECIDIDO |
| E-06 | El separador `@` y el saneado de ids | DECIDIDO |
| E-07 | `--append-system-prompt` en el runtime (H6) | DECIDIDO |
| E-08 | Flag de canal siempre presente (H5) | DECIDIDO |
| E-09 | Ventana de carrera del id anunciado (H4) | DECIDIDO |
| E-10 | Anti-spoofing del `de_id` (H8) — prerequisito de regla dura | DECIDIDO |
| E-11 | Alcance por proyecto: `proyecto_id` server-side vs convención de id | DECIDIDO |
| E-12 | Validación de la cadena de mando: qué endpoints validan qué | DECIDIDO |
| E-13 | Organigrama: árbol vertical (v1) vs grafo libre (v2) | DECIDIDO |
| E-14 | Persistencia del store organizativo (Redis/SQLite/bitácora.db) | DECIDIDO |
| E-15 | Archivos `.lexusfx/` del repo: qué es verdad y qué es espejo | DECIDIDO |
| E-16 | Tools MCP nuevas de conocimiento (skill) | DECIDIDO |
| E-17 | Renombrado de un peer vivo (H2) | DECIDIDO |
| E-18 | Arranque del broker best-effort (H9) | DECIDIDO |
| E-19 | Multi-broker = multi-empresa | DECIDIDO |
| E-20 | Choque cadena-de-mando ↔ "tócale el hombro" | DECIDIDO |

---

## E-01 — Conocimiento en el boot: RICO
- **Contexto:** ¿cuánto sabe un agente al arrancar? Hoy: nada de negocio ([[04-conocimiento-agente]] §0).
- **Opciones:** (a) rico (empresa+todos los proyectos+organigrama+cargo); (b) acotado (proyecto+cargo+
  colegas directos); (c) mínimo (solo cargo, resto por pull).
- **Decisión (Max):** **(a) rico.**
- **Consecuencias:** `instructions` del MCP se enriquecen con 3-4 GET al broker en el boot; exige los
  endpoints de conocimiento (E-04). Riesgo de contexto largo → mitigado con resumen + punteros
  ([[04-conocimiento-agente]] §6.1).
- **Reversibilidad:** alta — reducir el conocimiento inyectado es cambiar el ensamblaje de `instructions`,
  sin tocar datos.

## E-02 — Fuente de la verdad: híbrida (broker + repo)
- **Contexto:** ¿dónde vive la verdad de empresa/proyectos/cargos?
- **Opciones:** (a) híbrida (empresa/organigrama en broker; proyecto en archivos del repo); (b) todo en el
  broker; (c) todo en archivos.
- **Decisión (Max):** **(a) híbrida.**
- **Consecuencias:** empresa/organigrama/cargos = broker (verdad central, siempre fresco); conocimiento del
  proyecto = `.lexusfx/proyecto.md` en el repo (portátil, versionado, del equipo). Los espejos
  `empresa/cargo/organigrama.md` se regeneran al provisionar ([[04-conocimiento-agente]] §2.3).
- **Reversibilidad:** media — mover una fuente de un lado a otro es migración de datos, acotada.

## E-03 — Cumplimiento: regla dura (el broker valida)
- **Contexto:** ¿la cadena de mando y las capacidades son garantía (sistema impide) o guía (prompt orienta)?
- **Opciones:** (a) regla dura (broker valida y rechaza); (b) v1 guía + v2 dura.
- **Decisión (Max):** **(a) regla dura desde ya.**
- **Consecuencias:** el broker debe **conocer** cargos/cadena/capacidades → store organizativo (E-04) +
  validación server-side (E-12) + anti-spoofing del `de_id` (E-10, prerequisito). Convierte esto de
  "config+UI" a una capa de backend real (a diseñar en specs, no codificar aquí).
- **Reversibilidad:** media — desactivar la validación (volver a guía) es un flag; activarla exige el store.

## E-04 — Store organizativo en el broker
- **Contexto:** deriva de E-02/E-03: el broker necesita persistir la estructura organizativa.
- **Decisión:** el broker gana un **store organizativo** (`Cargo`, `Proyecto`, `Agente`, organigrama) tras
  el trait `Almacen` (Redis default + SQLite feature), con endpoints `/admin/{proyecto,cargo,agente,
  organigrama}` + `/contexto-empresa`, `/mi-cargo`, `/mi-organigrama` ([[03-pipeline-provision]] §5).
- **Consecuencias:** nueva superficie de escritura admin en el broker; todo bajo token; DTOs en
  `peers-core` ([[02-modelo-dominio]]). Versionar el plugin al tocar el broker.
- **Reversibilidad:** baja una vez poblado (es la fuente de verdad); por eso se diseña con cuidado ahora.

## E-05 — Identidad `rol@proyecto` + provisión del id (cierra H1/H3/H7)
- **Contexto:** hoy el id se deriva de carpeta+tty; `rol@proyecto` es imposible por derivación, y la vía
  plugin no inyecta `CLAUDE_PEERS_ID` (`plugin/.mcp.json` `env:{}`). El client SÍ respeta un id explícito
  (`main.rs:85-88`).
- **Decisión:** el id de agente es **`<cargo>@<proyecto>`**; la app lo **provee** al lanzar exportando
  `CLAUDE_PEERS_ID=rol@proyecto` en el entorno del proceso `claude` (Paso 4.3). No depende del TTY (cierra
  H7).
- **Consecuencias:** binding id↔sesión resuelto (cierra el riesgo abierto de RFC Lanzador §6.2); ids
  legibles y durables. Único cambio imprescindible en el runtime de arranque.
- **Reversibilidad:** alta — si no se provee, cae a la derivación actual (compat).

## E-06 — El separador `@` y el saneado de ids
- **Contexto:** `contexto::sanear` convierte `@`→`-` (`contexto.rs:139-146`), pero eso aplica solo a ids
  **derivados**; un id explícito viaja tal cual.
- **Decisión:** conservar `@` como separador legible en la **capa organizativa** y en `CLAUDE_PEERS_ID`
  (el client no lo saneala cuando es explícito). Verificar que las claves Redis con `@` (`cprs:msg:{id}`,
  `cprs:bandeja:{id}`) no colisionan (Redis admite `@` en keys).
- **Consecuencias:** ids como `backend@proyecto-x` en toda la UI y el protocolo. Si algún consumidor
  asumiera `[a-z0-9-]`, ajustar (verificar en Design).
- **Reversibilidad:** media — cambiar el separador (p.ej. a `.`) es un renombrado global.
- **PENDIENTE:** confirmar en Design que ningún path del broker re-saneala un id explícito.

## E-07 — `--append-system-prompt` en el runtime (cierra H6)
- **Contexto:** el flag NO se pasa hoy en ningún sitio del runtime; solo existe en specs.
- **Decisión:** la **app de lanzamiento** (RFC Lanzador) lo introduce, con el system prompt compuesto por
  capas ([[04-conocimiento-agente]] §2.1). Escapado correcto (RFC Lanzador AC2). Es Canal A.
- **Consecuencias:** el lanzamiento deja de ser `claude` pelado; el cargo se materializa como prompt real.
- **Reversibilidad:** alta — sin el flag, el agente arranca sin cargo (compat, modo actual).

## E-08 — Flag de canal siempre presente (cierra H5)
- **Contexto:** `--dangerously-load-development-channels server:claude-peers` es manual salvo `install.sh`;
  la vía plugin no lo pone.
- **Decisión:** la app de lanzamiento lo añade SIEMPRE (RFC Lanzador R5); Max no lo teclea.
- **Consecuencias:** el push `<channel>` funciona sin pasos manuales. `--dangerously-skip-permissions`
  queda OFF por defecto con aviso.
- **Reversibilidad:** alta.

## E-09 — Ventana de carrera del id anunciado (cierra H4)
- **Contexto:** en `initialize` se anuncia `estado.id` real o el preferido (`main.rs:201-206`); si el
  broker sufijó y el registro no completó, el agente recibe un id que no es el suyo.
- **Decisión:** con `rol@proyecto` (ids únicos por diseño), la colisión casi nunca ocurre → riesgo
  minimizado. Además, re-anunciar el id real tras estabilizar (el latido actualiza `estado.id`,
  `main.rs:652`) vía una línea de `instructions`/tool.
- **Consecuencias:** el round-trip de identidad deja de romperse (el bug histórico del `.notebook`).
- **Reversibilidad:** alta.

## E-10 — Anti-spoofing del `de_id` (cierra H8) — PREREQUISITO de la regla dura
- **Contexto:** la exención de política confía en el `de_id` declarado por el cliente
  (`main.rs:459-467`); un peer podría declararse `broker`/`operador`. Con regla dura (E-03) esto es
  inaceptable: la validación de cadena de mando se basaría en un `de` spoofable.
- **Decisión:** **prerequisito de E-03.** El broker debe atar el `de_id` a la identidad **registrada** del
  conector (el peer que abrió la conexión/registró ese id), no al valor declarado en el payload. Diseño:
  el `de` de una acción se toma del id registrado del emisor, no del campo libre.
- **Consecuencias:** la regla dura es fiable; el operador (`ID_OPERADOR`) se reserva de verdad. Toca el
  broker (validación de identidad del emisor).
- **Reversibilidad:** baja — es un cimiento de seguridad; una vez puesto, no se quita.
- **PENDIENTE:** diseño fino de cómo el broker ata `de_id`↔conexión (interactúa con el modelo stateless
  HTTP actual; puede requerir un token/handle por peer). **A resolver en Design antes de la regla dura.**

## E-11 — Alcance por proyecto: `proyecto_id` server-side (por E-03)
- **Contexto:** ¿aislar proyectos por convención de id (`@proyecto`, cliente) o por campo server-side?
- **Opciones:** (a) convención de id (filtro cliente, cero backend); (b) `proyecto_id` en Tarea/Mensaje +
  filtros server-side.
- **Decisión:** **(b) server-side**, porque E-03 (regla dura) exige que el broker haga cumplir el
  aislamiento (no basta filtrar en la UI). Campo `proyecto_id` opcional (`#[serde(default)]`) en las
  entidades de trabajo, derivable del `@proyecto` del id para compat.
- **Consecuencias:** el aislamiento es garantía, no cosmética. Migración compat (default = sin proyecto =
  comportamiento actual).
- **Reversibilidad:** media.
- **Nota:** revisa la recomendación previa de RFC Proyectos (que sugería v1 por convención) — **E-03 la
  actualiza a server-side**.

## E-12 — Validación de la cadena de mando: qué valida el broker
- **Contexto:** con regla dura, ¿qué endpoints validan qué?
- **Decisión:** el broker valida, en el punto donde ya vive la política (`main.rs:465-505`):
  - `enviar` → política de comunicación (ya) + (nuevo) nada extra (comunicar ≠ delegar).
  - `tarea/asignar` / `tarea/reasignar` → el `de` debe tener `puede_delegar_a` que incluya el cargo del
    `para`, o ser el operador. Si no → `ok:false` "fuera de tu cadena de mando".
  - capacidades (`crear_tarea`, `forzar`, etc.) → el `de` debe tener la `Capacidad`. Si no → `ok:false`.
- **Consecuencias:** los agentes no pueden saltarse la jerarquía ni sus permisos. Requiere el store (E-04)
  + anti-spoofing (E-10).
- **Reversibilidad:** media (flag para volver a "guía").

## E-13 — Organigrama: árbol vertical (v1) vs grafo libre (v2)
- **Contexto:** el grafo con 3 tipos de arista es complejo en GPUI.
- **Decisión:** **v1 = árbol vertical** de `reporta-a` + aristas `habla-con` superpuestas
  ([[05-organigrama-visual-ethos]] §2); grafo libre con layout automático = v2.
- **Consecuencias:** implementable en flexbox sin `Element` custom; legible hasta ~15 nodos.
- **Reversibilidad:** alta (v2 es aditivo).

## E-14 — Persistencia del store organizativo
- **Contexto:** ¿dónde se guardan Cargo/Proyecto/Agente/organigrama?
- **Decisión:** tras el trait `Almacen` (Redis `cprs:org:*` + SQLite espejo), como `Politica`; la
  **identidad durable** de agentes conocidos se ancla en `bitacora.db`/`peers_conocidos` (ADR-001 desktop,
  ya decidido). Las plantillas de cargo pueden además cachearse en `config.toml` para arranque offline de
  la app, pero la **verdad** es el broker (E-02).
- **Consecuencias:** coherente con la arquitectura de stores existente; nada nuevo de infra.
- **Reversibilidad:** media.

## E-15 — Archivos `.lexusfx/` del repo: verdad vs espejo
- **Contexto:** el modelo híbrido pone conocimiento en el repo; ¿qué es fuente y qué es cache?
- **Decisión:** `proyecto.md` = **verdad del equipo** (editable, versionada, no se sobrescribe);
  `empresa.md`/`cargo.md`/`organigrama.md` = **espejos read-only** regenerados desde el broker en cada
  provisión (Paso 4.6). Marcados "generado — no editar".
- **Consecuencias:** el conocimiento del proyecto viaja con el código; los espejos nunca divergen (se
  regeneran).
- **Reversibilidad:** alta.

## E-16 — Tools MCP nuevas de conocimiento (la skill)
- **Contexto:** el agente necesita operar la empresa (consultar proyectos, cargo, organigrama).
- **Decisión:** añadir tools de **lectura** organizativa (`contexto_empresa`, `listar_proyectos`,
  `mi_cargo`, `mi_organigrama`, `mi_cadena_mando`, `quien_es`) al array JSON de `mcp.rs:110` + brazo en
  `ejecutar_tool` (el MCP es a mano, no rmcp). Las de acción ya existen y las valida el broker (E-12).
- **Consecuencias:** el agente "sabe operar" la empresa; la garantía de permisos la da el broker, no la
  tool. Versionar plugin.
- **Reversibilidad:** alta (quitar una tool no rompe).

## E-17 — Renombrado de un peer vivo (H2)
- **Contexto:** el id se lee una sola vez al lanzar (`Args::parse()`); no hay renombrado en caliente.
- **Decisión:** **no soportar renombrado en caliente en v1.** Cambiar el rol de un agente = reiniciar su
  sesión con otro `CLAUDE_PEERS_ID`. Aceptable (relanzar es barato con perfiles).
- **Consecuencias:** simplicidad; sin endpoint de renombrado.
- **Reversibilidad:** alta (v2 podría añadirlo).

## E-18 — Arranque del broker best-effort (H9)
- **Contexto:** el hook `asegurar-broker.sh` levanta el broker si el puerto no responde; en topología
  mixta `install.sh` solo avisa.
- **Decisión:** fuera del ciclo de vida del agente (el broker es infra de empresa, no un empleado). Se
  mantiene el hook; se documenta que un broker central por empresa (E-19) es el patrón. Endurecer el hook
  (evitar duplicados sin `curl`) es mejora menor, no bloqueante.
- **Reversibilidad:** alta.

## E-19 — Multi-broker = multi-empresa
- **Contexto:** ¿varios brokers?
- **Decisión:** **un broker por empresa** (RRHH central único). Multi-broker = multi-empresa (fuera de
  alcance; RFC Acceso piensa multi-broker para **conmutar**, no federar). El organigrama "empresa" es el
  de un broker.
- **Reversibilidad:** alta (federación es v-futuro).

## E-20 — Choque cadena-de-mando ↔ "tócale el hombro"
- **Contexto:** la VISÃO valora que los peers se interrumpan (autonomía); la cadena de mando restringe.
- **Decisión:** los dos ejes son **ortogonales**: **habla-con** (política, comunicación espontánea) ≠
  **delega-en** (cadena, asignación de trabajo). Un agente puede tocar el hombro a un par aunque no pueda
  delegarle una tarea. La cadena es **opcional por cargo** (`reporta_a: Option`); un equipo plano (todos a
  Max) es válido y es el default.
- **Consecuencias:** no se sacrifica la autonomía; la jerarquía se añade donde Max la quiere.
- **Reversibilidad:** alta.

---

## Decisiones que quedan PENDIENTES de validar con Max (marcadas arriba)

- **E-06** — confirmar que ningún path del broker re-saneala un id explícito con `@`.
- **E-10** — el diseño fino del anti-spoofing `de_id`↔conexión (interactúa con el HTTP stateless actual;
  puede requerir un handle/token por peer). **Es el prerequisito técnico más delicado de la regla dura
  (E-03); conviene un spike antes de comprometer la validación server-side.**

Todo lo demás está decidido y es coherente con las 4 elecciones de producto de Max.

---
#empresa #decisiones #adr #regla-dura #identidad #conocimiento
