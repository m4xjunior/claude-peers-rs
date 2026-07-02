# RFC — Organigrama y Roles (definir cargos, contratar agentes, el organigrama vivo)

> ⬆ [[../_MOC|Mapa]] · Modelo: [[../01-modelo-corporativo]] · Dominio: [[../02-modelo-dominio]] ·
> GPUI: [[../00-fundamentos-gpui]]
>
> Fecha: 2026-07-02. Estado: **PROPUESTO — NO implementar hasta aprobar.**
> Pantalla nueva: `crates/peers-desktop/src/vista/organigrama.rs` (pestaña del sidebar).
> Design System: **Ethos** (TINTA `#100D0A`, TINTA2 `#1A1611`, PAPEL `#ECE5D7`, BRASA `#C9A96E`,
> HUMO `#938B7B`, LINEA `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999).
> Relacionada con: [[RFC-proyectos]] (el proyecto contiene el equipo), [[RFC-delegacion-cadena-mando]]
> (las relaciones entre cargos), [[../../desktop/lanzador/RFC-lanzador]] (system prompt R2/R2.1),
> [[../../desktop/politica-comunicacion/RFC-politica-comunicacion]] (puede-hablar-con).

---

## 1. El problema (en palabras de Max)

> "Tiene que haber cómo yo… delegar agentes que yo creo. Esos agentes son un system prompt (puede ser otra
> cosa) que se inserta en el peer al inicio de la conversación, que es toda su regla de negocio y cómo va a
> trabajar con los otros peers. Tiene que ser una corporación con lógica real de empresa."

Hoy un "agente" no existe como entidad **que Max defina**. Un peer es una sesión de Claude que arranca con
las instrucciones genéricas del MCP; no hay forma de **crear un cargo** ("Backend Rust", "Coordinador"),
darle su regla de negocio (system prompt), decir a quién reporta y con quién habla, guardarlo como
plantilla, y **contratarlo** en un proyecto. La "lógica de empresa" (quién manda a quién, quién es de qué
departamento) vive como prosa en `COORDENACAO.md`, no como estructura operable.

## 2. La solución

Dos entidades y una vista:

1. **Cargo (plantilla de puesto)** — la "descripción de cargo" reutilizable: system prompt (la regla de
   negocio), departamento, cadena de mando (`reporta_a`, `puede_delegar_a`), capacidades y ubicación por
   defecto ([[../02-modelo-dominio]] §2.1). Se crea, edita, versiona y **reutiliza** entre proyectos.
2. **Agente (empleado)** — una instancia de un cargo en un proyecto, con id `rol@proyecto`
   ([[../02-modelo-dominio]] §5). **Contratar** = crear el agente (parado); **lanzar** = arrancar su sesión
   (RFC Lanzador). Separados a propósito.
3. **Organigrama (vista)** — el árbol/grafo que dibuja el equipo y sus tres relaciones (habla-con,
   reporta-a, supervisa-a) sobre el proyecto activo o toda la empresa ([[../01-modelo-corporativo]] §6).

**Cero infra nueva:** cargos, agentes y plantillas son **config del operador** en `config.toml`; el system
prompt se inyecta con `--append-system-prompt` (ya verificado en RFC Lanzador R2); el organigrama es una
vista `Render` que observa esas entidades + la política + `/listar`.

---

## 3. Requisitos (trazables)

### Cargos (plantillas de puesto)

- **R1 — CRUD de Cargo.** Crear/editar/duplicar/borrar `Cargo { id, nombre, system_prompt, departamento?,
  reporta_a?, puede_delegar_a[], capacidades[], notas? }`. Persistido en `config.toml` (`[[cargo]]`),
  `#[serde(default)]` para compat.
- **R2 — Editor de system prompt (la regla de negocio).** `Input` multilínea (Textarea del kit) con
  contador de caracteres mono, placeholder HUMO. Es el mismo mecanismo que RFC Lanzador R2/R2.1
  (plantillas de system prompt guardadas): **unificar** — las "plantillas de system prompt" del Lanzador y
  el `system_prompt` del Cargo son lo mismo. Un cargo ES una plantilla nombrada + metadatos de organigrama.
- **R3 — Departamento/equipo (opcional v1).** Campo/etiqueta para agrupar cargos ("backend", "front",
  "qa"). En la política, un departamento puede mapear a `Patron::Grupo` (ya previsto) para reglas por
  equipo ("front-* no habla con backend-*"). v1: etiqueta; v2: grupo real de política.
- **R4 — Capacidades (v1 informativas).** Lista de `Capacidad` (CrearTareas, AsignarTareas, RevisarSolo…).
  En v1 se **vuelcan al system prompt** al lanzar (texto: "puedes asignar tareas a backend y qa"); en v2 el
  broker las valida ([[../02-modelo-dominio]] §6). No bloquear v1 por v2.
- **R5 — Cadena de mando en el cargo.** `reporta_a` (id de cargo superior) y `puede_delegar_a` (ids de
  cargo). Son datos del **puesto**, no del agente ([[../02-modelo-dominio]] §4). Alimentan el organigrama y
  [[RFC-delegacion-cadena-mando]].

### Agentes (contratar)

- **R6 — Contratar agente.** Instanciar un cargo en un proyecto: crea `Agente { id: <cargo>@<proyecto>,
  cargo_id, proyecto_id, ubicacion (hereda del proyecto, override posible), estado: Contratado }`. NO
  arranca proceso (eso es lanzar). Persistido en el `Proyecto` (config).
- **R7 — Id legible y estable `rol@proyecto`.** Al contratar, se compone el id ([[../02-modelo-dominio]]
  §5). Si ya hay un agente de ese cargo en el proyecto, sufijar (`qa-2@proyecto-x`) — legible, no colapsa
  (coherente con el fix de colisión commit 1f4187f).
- **R8 — Lanzar / pausar el agente.** "Lanzar" delega en RFC Lanzador (sesión `claude` con el system prompt
  del cargo + `CLAUDE_PEERS_ID=<id>` + flag de canal, en la ubicación). "Pausar" = kick (`/salir`). El
  estado del agente (Contratado→Lanzado→Vivo→Ocioso→Pausado) se deriva cruzando con `/listar` + alertas.
- **R9 — Escribir a un agente.** Desde su ficha, abrir el **chat privado** (RFC Lanzador §7, pull) o
  enviar un mensaje normal (`/enviar`, se renderiza como `<channel>`). Max elige: privado (no se ve en el
  TUI del agente) o público (se toca el hombro).

### Organigrama (vista)

- **R10 — Árbol/grafo del equipo.** Dibuja los cargos/agentes del proyecto activo (o de la empresa) como
  nodos, con aristas por relación: **reporta-a** (jerarquía vertical, cadena de mando), **habla-con** (de
  la política), **supervisa-a** (derivado + supervisor). El Operador (Max) es la raíz.
- **R11 — Estado vivo en el organigrama.** Cada nodo-agente muestra su estado (vivo BRASA / ocioso HUMO /
  bloqueado naranja / pausado gris) cruzando `/listar` + `/admin/alertas`. De un vistazo, Max ve quién
  trabaja.
- **R12 — Acciones desde el nodo.** Click en un agente → ficha (chat, tareas, jornada, kick, reasignar
  cargo). Click en un cargo → editar plantilla. Es el hub de operación del equipo.

### Transversal

- **R13 — Todo degrada, red en background.** Broker offline → el organigrama pinta la estructura (config)
  aunque no tenga estado vivo (banner "estado no disponible"), sin crash. Red vía `background_executor` +
  `bloquear_en`. Sin `.unwrap()`/`.expect()` en prod. `#[serde(default)]` para compat.

---

## 4. Diseño de UI (gpui-component, tema Ethos)

Componentes: `Input` (textarea system prompt), `Select` (departamento, reporta_a, capacidades), `Button`,
`Dialog`, `Tabs`, `Badge`, `Tooltip`, `Table` (lista de cargos). El organigrama (R10) es una vista
custom con `div()` + flexbox (nodos = `superficie_card` radio 14; aristas = líneas LINEA/BRASA); si el
grafo se complica, empezar por un **árbol vertical** (más simple en flexbox) antes que un grafo libre.

```
┌ Organigrama — proyecto-x ─────────────────────────────────────────────────┐
│                         ┌─────────────┐                                    │
│                         │  OPERADOR   │  (Max, raíz — nunca bloqueable)     │
│                         └──────┬──────┘                                     │
│                         ┌──────┴───────┐                                    │
│                         │ coordinador@ │ ●vivo                              │
│                         └──┬────────┬──┘                                    │
│                  ┌─────────┴──┐  ┌──┴─────────┐   ─ ─ habla-con (política)  │
│                  │ backend@   │  │  qa@       │   ─── reporta-a (mando)      │
│                  │  ●vivo     │  │  ○ocioso   │                             │
│                  └────────────┘  └────────────┘                            │
├──────────────── Cargos (plantillas) ──────────────────────────────────────┤
│  [ + Nuevo cargo ]  coordinador · backend · qa · investigador  [Editar…]   │
│  Editando "backend": system prompt ┌───────────────────────┐  reporta_a ▾  │
│                                     │ Eres el peer backend… │  delega_a ▾   │
│                                     └───────────────────────┘  caps ▾       │
└────────────────────────────────────────────────────────────────────────────┘
```

Variantes a decidir en Design: árbol vertical vs grafo con aristas de tipo; ficha de agente como drawer vs
modal; editor de cargo inline vs pantalla propia.

---

## 5. Criterios de aceptación

- **AC1 (R1/R2)** — Crear un cargo "backend" con su system prompt persiste en `config.toml`; reabrir la app
  lo muestra; editar el prompt se guarda.
- **AC2 (R6/R7)** — Contratar "backend" en "proyecto-x" crea un agente id `backend@proyecto-x`; contratar
  otro backend da `backend-2@proyecto-x`. No arranca proceso (estado Contratado).
- **AC3 (R8)** — "Lanzar" el agente arranca `claude --append-system-prompt "<system prompt del cargo>"
  --dangerously-load-development-channels server:claude-peers` con `CLAUDE_PEERS_ID=backend@proyecto-x` en
  su ubicación; aparece en `/listar` con ese id (verifica el binding).
- **AC4 (R9)** — Desde la ficha del agente, el chat privado le llega por pull (no se renderiza en su TUI —
  AC de RFC Lanzador) y un `/enviar` normal sí se renderiza. Max elige el canal.
- **AC5 (R10/R11)** — El organigrama dibuja reporta-a (vertical) y habla-con (de la política); cada
  agente muestra su estado vivo. Cambiar la política redibuja las aristas habla-con.
- **AC6 (R4 v1)** — Las capacidades y la cadena de mando del cargo aparecen en el system prompt inyectado
  (texto), no como permiso duro (eso es v2).
- **AC7 (compat/R13)** — Config sin cargos/agentes deserializa; broker caído → organigrama pinta estructura
  con banner de estado, sin crash.

---

## 6. Riesgos y decisiones abiertas (Design)

1. **Unificar "plantilla de system prompt" (Lanzador R2.1) con "Cargo".** Son el mismo objeto visto de dos
   formas. **Recomendación:** el Cargo es la fuente; el Lanzador consume cargos como plantillas. Evita dos
   almacenes de prompts divergentes. Decidir la migración del `config.toml`.
2. **Capacidades v1 (texto) vs v2 (duras).** v1 basta para operar como empresa hoy. v2 (broker valida
   `AsignarTareas` etc.) depende de endurecer el broker y de qué expone el harness. **Recomendación:** v1
   ahora, v2 cuando Max lo pida. Documentar el límite (una capacidad en v1 es una *promesa* al agente, no
   una *garantía* del sistema).
3. **Organigrama: árbol vs grafo.** Un grafo con tres tipos de arista es potente pero complejo en GPUI.
   **Recomendación:** v1 = árbol vertical de reporta-a con aristas punteadas de política superpuestas; grafo
   libre = v2.
4. **Id `rol@proyecto` y binding.** Depende de que `peers-client` respete `CLAUDE_PEERS_ID` explícito
   ([[../02-modelo-dominio]] §5). Bloqueante para AC3; resolver primero.
5. **¿Cómo se relaciona el cargo con las instrucciones del MCP?** El MCP ya inyecta instrucciones genéricas
   ("tócale el hombro", "ficha tareas"). El system prompt del cargo las **especializa**, no las reemplaza.
   Decidir el orden/precedencia (recomendado: MCP genérico + append del cargo, que es lo que
   `--append-system-prompt` hace por diseño).

---

## 7. Constraints

- Sin `.unwrap()`/`.expect()` en prod; `Result`/`anyhow`. Español salvo protocolo. Reusa DTOs de
  `peers-core` (`Cargo`/`Agente` nuevos, ver [[../02-modelo-dominio]]); NO redefinir en la desktop. Reusa
  el file picker nativo, el system prompt del Lanzador, la política y `/listar`. Config en `config.toml`.
  Red en `background_executor` + `bloquear_en`. Versionar plugin solo si se toca un binario (el binding
  `CLAUDE_PEERS_ID` puede requerir tocar `peers-client` → bump). NUNCA `Co-Authored-By`. Jornada en el commit.

## 8. Fuera de alcance (v1)

- Capacidades duras / permisos por rol en el broker (v2, §6.2).
- Grafo de organigrama libre con layout automático (v1 = árbol vertical).
- Herramientas MCP distintas por cargo (depende del harness; v2+).
- Contratación "masiva" con IA que sugiere el equipo (YAGNI).
- Versionado/histórico de cambios de un cargo (posible v2; v1 = editar in-place).

## 9. Dependencias

- **[[RFC-proyectos]]** — el proyecto es el contenedor donde se contrata y lanza el equipo.
- **[[RFC-delegacion-cadena-mando]]** — consume `reporta_a`/`puede_delegar_a` de los cargos.
- **[[../../desktop/lanzador/RFC-lanzador]]** — lanzar el agente (system prompt R2, id, ubicación, chat privado).
- **[[../../desktop/politica-comunicacion/RFC-politica-comunicacion]]** — aristas "habla-con" del organigrama.
- **Binding id↔sesión** ([[../02-modelo-dominio]] §5) — `peers-client` respeta `CLAUDE_PEERS_ID`.

---
#rfc #empresa #organigrama #roles #cargos #agentes #peers-desktop
