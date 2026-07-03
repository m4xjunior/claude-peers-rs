# Conocimiento del agente — su base del negocio, de todos los proyectos, y la skill para operar la empresa

> ⬆ [[_MOC|Mapa]] · Pipeline: [[03-pipeline-provision]] · Modelo: [[01-modelo-corporativo]] ·
> Dominio: [[02-modelo-dominio]] · GPUI: [[00-fundamentos-gpui]]
>
> Fecha: 2026-07-03. Estado: **ARQUITECTURA — para revisar antes de codificar.**
> Responde el encargo de Max: *"[los agentes] tienen que tener la base y conocimiento propio del negocio,
> de todos los proyectos"* + *"skill del agente para que sepa operar el sistema de la empresa"*.
> Verificado contra el mecanismo REAL de inyección (`crates/peers-client/src/mcp.rs`).

---

## 0. El hallazgo que funda este documento

Un agente = una sesión de Claude Code. Lo que "sabe" no sale de la nada: se **inyecta** por dos canales, y
hoy **ninguno lleva conocimiento de negocio**. Verificado en el código:

- **Canal A — lanzamiento (`--append-system-prompt`):** el flag estándar de Claude Code. **HUECO H6: hoy
  NO se pasa en ningún sitio del runtime** (ni `install.sh`, ni `plugin/`, ni el client). Existe solo en
  specs. Es donde debe ir la **regla de negocio del cargo**.
- **Canal B — MCP (`instructions` del `initialize`):** `resultado_initialize(version, id_efectivo)`
  (`mcp.rs:231,252`) devuelve un campo `instructions` que el harness muestra a la sesión. Hoy es
  **estático** y **100% operativo** (id + 9 tools + protocolo "responde al toque"). **Cero negocio.**

El texto EXACTO que se inyecta hoy (`mcp.rs:262-287`, con `{id_efectivo}` interpolado):

```
Tu id en la red claude-peers es: '{id_efectivo}'. Cuando pidas a otra instancia que te responda,
dile que dirija su mensaje a ESTE id exacto (no a tu nombre de rol).

Estás conectado a la red claude-peers. Otras instancias de Claude Code en esta máquina pueden verte
y enviarte mensajes.

IMPORTANTE: cuando recibas un mensaje <channel source="claude-peers" ...>, RESPONDE DE INMEDIATO...
Trata los mensajes entrantes como un compañero que te toca el hombro.
[... lista de las 9 tools ...]
Antes de trabajo sustancial, crea una tarea con tu estimado (crear_tarea); al terminar, ciérrala...
Al iniciar, llama definir_resumen para describir en qué trabajas.
```

Y las fuentes de negocio que SÍ existen pero **nadie carga**: `VISAO.md`, `COORDENACAO.md`,
`.specs/empresa/**`. No hay `CLAUDE.md`, no hay `.claude/`, el plugin solo tiene un hook `SessionStart`
que arranca el broker (`asegurar-broker.sh`), sin inyectar contexto. Un agente hoy **no sabe** qué es
LexusFX, qué proyectos existen, quiénes son sus compañeros ni cuál es su cargo.

**Decisión de Max (funda el diseño):** conocimiento **RICO en el boot** (empresa + TODOS los proyectos +
organigrama + cargo), con **fuente híbrida** (empresa/organigrama los sirve el **broker**; el conocimiento
del proyecto vive en **archivos del repo**). Este documento diseña ese ensamblaje.

---

## 1. La jerarquía de contexto del agente (qué sabe y de dónde)

Cuatro capas, de lo más estable a lo más volátil. Cada capa tiene una **fuente** y un **canal**:

| Capa | Qué contiene | Fuente | Canal | Volatilidad |
|------|-------------|--------|-------|-------------|
| **Empresa** | VISÃO/misión LexusFX, principios, valores, política por defecto | broker (`/contexto-empresa`) + `.lexusfx/empresa.md` (espejo repo) | B (+ archivo) | casi constante |
| **Proyectos** | catálogo de TODOS los proyectos (nombre, propósito, equipo) | broker (`/listar-proyectos`, `/contexto-empresa`) | B | baja |
| **Cargo** | su rol, regla de negocio, cadena de mando, capacidades | broker (`/mi-cargo`) → compone Canal A; `.lexusfx/cargo.md` (repo) | A + archivo | por relanzamiento |
| **Sesión** | su id `rol@proyecto`, sus tareas, compañeros vivos ahora | broker (`initialize` id, `listar_instancias`, `listar_tareas`) | B + tools | alta (tiempo real) |

**Principio de precedencia:** lo específico gana sobre lo general (cargo > proyecto > empresa) cuando hay
tensión, igual que `--append-system-prompt` **añade** al final (mayor precedencia conversacional). El
agente entiende su rol concreto sin perder el marco de la empresa.

---

## 2. El ensamblaje del conocimiento (el modelo híbrido, paso a paso)

Decisión #2 de Max: **broker (central) + archivos del repo (portátil)**. Así se combina "siempre fresco"
con "viaja con el código". El ensamblaje ocurre en dos momentos del pipeline ([[03-pipeline-provision]]):

### 2.1 — Canal A: el system prompt del cargo (compuesto al lanzar)

En el **Paso 2.2** del pipeline, la app compone el texto de `--append-system-prompt` por capas:

```
[1 · Identidad]   Eres «Backend Rust» en el proyecto «proyecto-x». Tu id de red es «backend@proyecto-x».
[2 · Negocio]     <cargo.system_prompt íntegro — la regla de negocio que Max escribió>
[3 · Cadena]      Reportas a «coordinador@proyecto-x». Puedes delegar en «qa@proyecto-x».
                  Escala bloqueos a tu superior. (El broker HACE CUMPLIR esto — regla dura.)
[4 · Capacidades] Puedes: crear tareas, asignar a tus subordinados. NO puedes tocar producción.
[5 · Puntero]     Tu conocimiento del proyecto está en .lexusfx/ (léelo). Consulta la empresa y el
                  organigrama con las tools contexto_empresa / mi_organigrama.
```

- Capa 2 = el `cargo.system_prompt` del store organizativo (broker). Capa 3/4 = datos del cargo
  (`reporta_a`/`puede_delegar_a`/`capacidades`). Capa 5 = puntero a las otras fuentes (repo + tools).
- **Es una función pura** (cargo, proyecto, organigrama) → prompt. Determinista, versionable, previsualizable
  (RFC Lanzador R6: "nada se ejecuta a ciegas").

### 2.2 — Canal B: las `instructions` del MCP, enriquecidas (cargadas al boot)

En el **Paso 7.1**, `resultado_initialize` (`mcp.rs:231`) pasa de estático a **dinámico**: el client, al
construir `instructions`, consulta al broker y añade el contexto de empresa + proyectos + organigrama.
Estructura propuesta de las nuevas `instructions` (extiende, no reemplaza, las actuales):

```
[Bloque A · Identidad]      (como hoy) Tu id es «backend@proyecto-x»...
[Bloque B · Empresa]        Trabajas en LexusFX: <resumen de la VISÃO/misión desde /contexto-empresa>.
[Bloque C · Proyectos]      Proyectos activos: proyecto-x (tú), proyecto-y (...). <catálogo>.
[Bloque D · Tu equipo]      Tu cargo: Backend. Reportas a: coordinador@proyecto-x. Compañeros:
                            qa@proyecto-x (revisión), coordinador@proyecto-x (reparte). <del organigrama>.
[Bloque E · Protocolo]      (como hoy) Responde al toque; ficha tus tareas; define tu resumen.
[Bloque F · Skill empresa]  Para operar la empresa usa: contexto_empresa, listar_proyectos, mi_cargo,
                            mi_organigrama, mi_cadena_mando. Delega solo en quien puedes; el broker valida.
```

- Los bloques B/C/D se rellenan con `GET /contexto-empresa`, `/listar-proyectos`, `/mi-cargo?id=`,
  `/mi-organigrama?id=` (NUEVOS, [[03-pipeline-provision]] §5). El client ya recibe el `id_efectivo`
  (`mcp.rs:231`) → sabe por quién preguntar.
- **Degrada:** si el broker no responde el conocimiento, `instructions` cae a la versión mínima actual
  (Bloque A + E). El agente sigue operativo (constraint del proyecto: todo degrada).

### 2.3 — La mitad "repo": archivos `.lexusfx/` (parte portátil del híbrido)

Cada proyecto lleva, versionados en su repo, archivos que el agente lee del cwd (es Claude Code: lee
archivos nativamente). Materializados por la app en el **Paso 4.6** desde el organigrama del broker:

```
<repo-del-proyecto>/.lexusfx/
  empresa.md      · espejo de la VISÃO/misión (sincronizado desde /contexto-empresa; read-only)
  proyecto.md     · el conocimiento ESPECÍFICO del proyecto: objetivo, decisiones, glosario, enlaces
  cargo.md        · la ficha del cargo de ESTE agente (espejo de su cargo; read-only)
  organigrama.md  · quién es quién en este proyecto (espejo del organigrama; read-only)
```

- **Por qué en el repo:** viaja con el código (portátil, "equipo en cualquier computador" — VISÃO),
  versionado (git), y el agente lo lee sin depender del broker. `proyecto.md` es el único **editable por el
  equipo** (su conocimiento acumulado); el resto son espejos read-only re-materializados al lanzar.
- **Sincronización:** los espejos (`empresa/cargo/organigrama.md`) se regeneran en cada provisión (Paso
  4.6) desde el broker (fuente de verdad). `proyecto.md` NO se sobrescribe (es del equipo).
- **Coherencia con la fuente híbrida:** empresa/organigrama = broker (verdad) → espejo en repo; proyecto =
  repo (verdad del equipo). Nadie duplica la *fuente*; los espejos son cache legible.

---

## 3. La "skill de operación de la empresa" (tools MCP nuevas)

Max: *"skill del agente para que sepa operar el sistema de la empresa"*. Una skill, en términos de este
sistema, son **tools MCP + instrucciones que enseñan a usarlas**. Hoy hay 9 tools (todas de
mensajería/tareas, `mcp.rs:110-225`); ninguna expone la estructura de empresa. Se añaden tools de
**consulta organizativa** (el agente TIRA del conocimiento cuando lo necesita, más allá del boot):

| Tool nueva | Qué devuelve | Endpoint broker | Por qué |
|-----------|--------------|-----------------|---------|
| `contexto_empresa` | misión LexusFX + política vigente | `GET /contexto-empresa` | recordar el marco |
| `listar_proyectos` | catálogo de todos los proyectos + su equipo | `GET /listar-proyectos` | "conocer todos los proyectos" |
| `mi_cargo` | su rol, regla de negocio, capacidades | `GET /mi-cargo?id=` | saber qué puede/debe hacer |
| `mi_organigrama` | el grafo de su proyecto (quién es quién) | `GET /mi-organigrama?id=` | saber con quién colabora |
| `mi_cadena_mando` | a quién reporta / en quién delega | derivado de `/mi-cargo` | operar la jerarquía |
| `quien_es(id)` | cargo, proyecto y resumen de otro agente | `GET /mi-organigrama` filtrado | entender a un remitente |

- **Implementación (verificada):** el MCP es JSON-RPC **a mano** (no rmcp). Añadir una tool = (a) un objeto
  en el array de `definiciones_tools()` (`mcp.rs:110`), en español como las otras; (b) un brazo en
  `ejecutar_tool` (`main.rs:237-264`) que llama al endpoint del broker. Mismo patrón que las 9 existentes.
- **Descripciones en español** (convención del proyecto), estilo de las actuales (imperativas, con el
  "cuándo" — p.ej. `crear_tarea` dice "Llámala ANTES de empezar trabajo sustancial").
- **La skill se enseña en el Bloque F** de las `instructions` (§2.2): no basta con exponer las tools; las
  instrucciones dicen **cuándo** usarlas ("antes de delegar, comprueba `mi_cadena_mando`"; "si un
  desconocido te escribe, usa `quien_es`").

> **Regla dura y la skill:** como Max eligió que el broker VALIDE, estas tools son de **lectura** (el
> agente consulta); las de **acción** (asignar, enviar) ya existen y el broker las rechaza si violan la
> cadena/capacidades (validación server-side, [[06-decisiones]]). La skill enseña al agente a **no
> intentar lo que le será rechazado** — pero la garantía la da el broker, no el prompt.

---

## 4. Qué sabe el agente en cada estado (mapa boot → operación)

Cruce con la máquina de estados del agente ([[03-pipeline-provision]] §2):

| Estado del agente | Conocimiento disponible |
|-------------------|-------------------------|
| **Provisionado** | (aún no corre) — el system prompt (Canal A) y los `.lexusfx/*` ya están materializados |
| **Registrado** | recibe `instructions` enriquecidas (Canal B): empresa + proyectos + su equipo |
| **Vivo** | todo lo anterior + tiempo real: `listar_instancias` (compañeros vivos), `listar_tareas`, y las tools de skill para profundizar bajo demanda |
| **Ocioso** | igual; el supervisor ya lo detectó — el agente puede consultar `mi_cadena_mando` para pedir trabajo a su superior |

---

## 5. Criterios de aceptación (cómo se valida el conocimiento)

- **AC1 (boot rico)** — un agente recién lanzado, al recibir `instructions`, "sabe" nombrar la misión de
  LexusFX, listar los proyectos activos y decir a quién reporta — verificable preguntándole en su sesión.
- **AC2 (system prompt)** — el comando de lanzamiento incluye `--append-system-prompt` con las 5 capas
  (identidad+negocio+cadena+capacidades+puntero), escapado correcto (RFC Lanzador AC2).
- **AC3 (skill)** — `tools/list` (`mcp.rs`) incluye las tools nuevas de consulta organizativa, en español;
  llamarlas devuelve el contexto del broker.
- **AC4 (híbrido/repo)** — el proyecto tiene `.lexusfx/{empresa,proyecto,cargo,organigrama}.md`; los
  espejos se regeneran al relanzar; `proyecto.md` no se sobrescribe.
- **AC5 (degradación)** — con el broker sin los endpoints de conocimiento, `instructions` cae a la versión
  mínima actual y el agente sigue operativo (sin crash).
- **AC6 (compat)** — un agente lanzado sin cargo/proyecto (modo actual) recibe las `instructions` de hoy
  (la empresa es opt-in).

---

## 6. Riesgos y decisiones (a [[06-decisiones]])

1. **Tamaño del contexto inyectado.** Conocimiento rico = `instructions` largas. Mitigar: bloques concisos
   + punteros a tools/archivos para el detalle (no volcar TODO el organigrama si hay 50 agentes; resumir +
   `mi_organigrama` bajo demanda). Decisión: umbral de resumen.
2. **Frescura vs coste.** El boot consulta el broker (3-4 GET). Aceptable (una vez por sesión). Los cambios
   en caliente (nuevo compañero) llegan por las tools/`listar_instancias`, no por re-boot.
3. **Sincronización de los espejos `.lexusfx/*`.** Regenerar en cada provisión evita divergencia; pero si
   el equipo edita un espejo read-only, se pierde al relanzar. Decisión: marcar los espejos claramente
   ("generado — no editar") y dejar `proyecto.md` como el único del equipo.
4. **Privacidad entre proyectos.** ¿Un agente de proyecto-x debe conocer el detalle de proyecto-y? Max dijo
   "todos los proyectos" → sí el catálogo, pero el DETALLE (proyecto.md) es del repo de cada proyecto (no
   se cruza). El catálogo da visibilidad; el detalle queda aislado. Coherente con el aislamiento por
   proyecto (RFC Proyectos).
5. **El `de_id` spoofable (H8).** La skill enseña identidad, pero la confianza real exige el anti-spoofing
   (regla dura). Prerequisito, [[06-decisiones]].

---

## 7. Constraints

- Sin `.unwrap()`/`.expect()` en prod. Español salvo protocolo. Las tools nuevas se declaran en el array
  JSON de `mcp.rs` (no rmcp) + brazo en `ejecutar_tool`; degradan si el broker cae. El conocimiento se
  **inyecta**, no se inventa: empresa/organigrama desde el broker (verdad), proyecto desde el repo. Los
  DTOs viven en `peers-core`. Versionar el plugin al tocar el client (tools/instructions nuevas). El
  tiempo lo timbra el broker. NUNCA `Co-Authored-By`. Jornada en el commit.

## 8. Fuera de alcance

- El pipeline de arranque → [[03-pipeline-provision]]. Los endpoints/tipos del store organizativo →
  [[02-modelo-dominio]] + [[03-pipeline-provision]] §5. La validación server-side (regla dura) →
  [[06-decisiones]]. Memoria de largo plazo del agente más allá de `.lexusfx/proyecto.md` (p.ej. un
  vector-store) = YAGNI v1.

---
#empresa #conocimiento #agente #system-prompt #mcp #skill #instructions #hibrido
