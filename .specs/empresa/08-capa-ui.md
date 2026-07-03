# Capa de UI — las funcionalidades que exponen las features de empresa

> ⬆ [[_MOC|Mapa]] · GPUI: [[00-fundamentos-gpui]] · Organigrama: [[05-organigrama-visual-ethos]] ·
> Pipeline: [[03-pipeline-provision]] · Workflow: [[07-workflow-trazable-164-features]] · UI base: [[../desktop/INDICE-RFCS]]
>
> Fecha: 2026-07-03. Estado: **ARQUITECTURA DE UI — para revisar/decidir en Design.**
> Max: *"no pensamos ni en las funcionalidades de UI que van a exponer estas features. Esa es tu tarea."*
> Aquí está la capa de UI de la empresa: pantallas, widgets, flujos y el **mapa feature→UI**. Tema Ethos,
> gpui-component real, verificado contra [[00-fundamentos-gpui]] (`Entity`/`Render`/async background) y
> los helpers de `tema.rs`.

---

## 0. Principio: la UI es un espejo reactivo, no la fuente

Toda pantalla de empresa es una **vista `Render`** que observa entidades (`Proyecto`/`Cargo`/`Agente` +
estado vivo del broker) y se redibuja con `update`+`notify` ([[00-fundamentos-gpui]] §4). La verdad vive en
el broker (empresa/organigrama) y el repo (proyecto); la UI **dispara acciones** (workflow §1 de
[[07-workflow-trazable-164-features]]) y **re-lee**. Nunca inventa estado. Red SIEMPRE en
`background_executor` + `bloquear_en` (anti-SIGABRT).

---

## 1. Mapa de navegación (sidebar Ethos)

La app pasa de 9 pestañas de observación a un **despacho corporativo**. Nuevas entradas en el sidebar
(sobre las 9 existentes), agrupadas:

```
◈ LexusFX                          ← cabecera: empresa + selector de PROYECTO ACTIVO
├─ EMPRESA
│   ├─ Organigrama      ← doc 05: el grafo del equipo (proyecto/empresa)
│   ├─ Proyectos        ← RFC Proyectos: crear/aislar/relanzar
│   └─ Cargos           ← RFC Organigrama-roles: plantillas de puesto (editor de system prompt)
├─ OPERAR
│   ├─ Lanzador         ← RFC Lanzador: wizard contratar+provisionar+lanzar (+ terminal + chat privado)
│   ├─ Mi equipo        ← RFC Delegación: panel del coordinador (tareas/reportes/alertas del equipo)
│   └─ Alertas          ← supervisor + escalado (existente, enriquecida)
└─ (las 9 pestañas de observación existentes: Peers, Tareas, Jornada, Trazabilidad, Redis, Broker,
    Acceso, Config — ahora filtrables por proyecto activo)
```

- **Selector de proyecto activo** (cabecera, `Breadcrumb`/`Select`): fija el ámbito; las 9 pestañas ganan
  un toggle "este proyecto / todos" (workflow §4). Es el "apartado de proyectos" hecho navegación.
- La empresa es **opt-in**: sin proyectos/cargos, el sidebar EMPRESA/OPERAR está vacío y la app opera como
  hoy (compat).

---

## 2. Pantallas nuevas (inventario con anatomía)

### 2.1 — Organigrama
Ya especificada en detalle visual: [[05-organigrama-visual-ethos]]. Es el hub. Resumen: árbol vertical
`reporta-a` + aristas `habla-con`, nodos con estado vivo, click→ficha de empleado / editor de cargo,
botones Contratar/Editar cargos/Aislar.

### 2.2 — Proyectos (RFC Proyectos)
Grid de cards de proyecto (nombre Fraunces + ubicación mono + "N agentes ●●●○") + ficha con pestañas
(Equipo · Tablero · Actividad · Alertas). Crear (picker nativo / host SSH), archivar, duplicar, **Lanzar
equipo** (dispara el pipeline por cada agente). Detalle: [[rfc/RFC-proyectos]].

### 2.3 — Cargos (editor de puesto) — NUEVA, núcleo del "delegar agentes que yo creo"
La pantalla donde Max **crea el agente** (define su regla de negocio). Anatomía:

```
┌ Cargos ────────────────────────────────────────────────  [ + Nuevo cargo ] ─┐
│  Plantillas:  coordinador · backend · qa · investigador        [ Duplicar ]  │
├─ Editando «backend» ─────────────────────────────────────────────────────────┤
│  Nombre        [ Backend Rust                    ]   Departamento [ backend ▾]│
│  System prompt  (la regla de negocio)                         1420 chars      │
│  ┌──────────────────────────────────────────────────────────────────────────┐│
│  │ Eres el peer backend. Trabajas en Rust idiomático, sin unwrap en prod…    ││
│  │ Colaboras tocando el hombro; fichas tus tareas…                           ││
│  └──────────────────────────────────────────────────────────────────────────┘│
│  Reporta a ▾   [ coordinador ]     Puede delegar en ▾ [ (ninguno)          ]  │
│  Capacidades   ☑ crear tareas  ☐ asignar a otros  ☐ tocar producción  ☑ …    │
│  Vista previa del system prompt compuesto (las 5 capas) →  [ Ver ]            │
│                                             [ Cancelar ]     [ ● Guardar ]     │
└───────────────────────────────────────────────────────────────────────────────┘
```

- **System prompt:** `Input` multilínea (Textarea del kit), contador mono, placeholder HUMO. Es el Canal A
  ([[04-conocimiento-agente]] §2.1). Unifica con las "plantillas de system prompt" del Lanzador (E-16 /
  RFC Organigrama-roles §6.1).
- **Reporta a / Puede delegar en:** `Select` de cargos (cadena de mando; el broker valida integridad,
  [[06-decisiones]] E-12). Ciclo → error inline.
- **Capacidades:** checkboxes; se vuelcan al system prompt Y el broker las hace cumplir (regla dura).
- **Vista previa del prompt compuesto:** muestra las 5 capas (identidad+negocio+cadena+capacidades+puntero)
  que se inyectarán — "nada se ejecuta a ciegas" (RFC Lanzador R6).
- Guardar → `POST /admin/cargo` (workflow: precondición integridad → bitácora `DefinirCargo`).

### 2.4 — Lanzador / Wizard de contratación+lanzamiento — NUEVA, el pipeline hecho UI
La UI del pipeline ([[03-pipeline-provision]]). Un flujo de pasos que **visualiza** la máquina de estados
del agente. Reusa la RFC Lanzador (file picker, terminal PTY, chat privado, SSH/tmux) y la envuelve en el
flujo corporativo:

```
┌ Lanzar equipo — proyecto-x ──────────────────────────────────────────────────┐
│  Paso ●──●──●──○──○   Definir · Contratar · Provisionar · Lanzar · Verificar   │
│                                                                                │
│  ▸ Equipo a lanzar:                                                             │
│    ☑ coordinador@proyecto-x   ubicación: local /Users/max/px   [prompt ✓]      │
│    ☑ backend@proyecto-x       ubicación: local /Users/max/px   [prompt ✓]      │
│    ☑ qa@proyecto-x            ubicación: ssh otus:/srv/px      [prompt ✓]      │
│                                                                                │
│  ▸ Provisión (checklist por agente):                                           │
│    backend@px:  claude ✓ · MCP ✓ · CLAUDE_PEERS_ID ✓ · canal ✓ · .lexusfx ✓    │
│    qa@px:       claude ✓ · MCP ✗  [ Instalar MCP aquí ]                        │
│                                                                                │
│  ▸ Comando (previsualizado):                                                   │
│    claude --append-system-prompt "…" --dangerously-load-…  [ Copiar ]          │
│                                          [ Atrás ]   [ ● Lanzar los 3 ]        │
├─ Terminal (PTY) [backend@px][qa@px][+]     │   Chat privado con backend@px      │
│  ╭ claude 2.1 · trabajando en tarea 1…     │   ┌───────────────────────────┐   │
│  ╰                                          │   │ tú: revisa el módulo auth │   │
│                                             │   └───────────────────────────┘   │
└─────────────────────────────────────────────┴────────────────────────────────────┘
```

- **Barra de pasos** = la máquina de estados (Definido→Contratado→Provisionado→Lanzado→Verificar). Cada
  agente avanza por ella; los estados fallidos se marcan (el checklist de provisión, doc 03 §4).
- **Checklist de provisión visible** — hace tangibles "los varios pasos que el código tiene que tener":
  Max VE qué falta (MCP no instalado → botón "Instalar MCP aquí", RFC Lanzador R7.3).
- **Comando previsualizado** con `--append-system-prompt` y el flag de canal (E-07/E-08). Copiar.
- **Terminal PTY + chat privado** (RFC Lanzador §11.1/§7): reusados tal cual. El chat privado es por agente
  (pull, no se renderiza en su TUI).
- **Degrada:** PTY no disponible → modo "solo preparar + Terminal externo" (RFC Lanzador R7.2), sin crash.

### 2.5 — Ficha de empleado (drawer) — NUEVA, el "detalle del peer" corporativo
Abierta desde el organigrama/Peers al click en un agente. Drawer derecho con secciones (reusa peers-01/05
enriquecidos):

- **Identidad:** id `rol@proyecto` (mono BRASA), cargo, proyecto, ubicación, estado vivo (glifo+color).
- **Cadena de mando:** reporta a / delega en (del cargo) — con enlaces a esos nodos.
- **Jornada:** sesiones + tareas (estimado vs real) + factor de estimación (peers-05/15).
- **Tareas:** lista con estado (chips de dominio) + acciones (asignar/reasignar restringidas por cadena).
- **Actividad (bitácora):** timeline de `AccionRegistrada` del agente (RFC registro-acciones).
- **Alertas:** las suyas, con resolver/escalar.
- **Chat privado:** botón que abre el hilo (pull). Enviar mensaje normal (público) como alternativa.
- **Acciones al pie:** relanzar · pausar (kick, con confirmación) · editar cargo.

### 2.6 — Mi equipo (panel del coordinador) — NUEVA (RFC Delegación R5)
Para un cargo con subordinados: `Tabs` (Tareas · Reportes · Alertas · Jornada) del equipo que le reporta.
Consolida `/listar-tareas` + `/tarea/reportes` + `/admin/alertas` filtrados por `reporta_a`. El selector de
destino al delegar se **restringe** a `puede_delegar_a` (workflow eslabón 2).

### 2.7 — Alertas + escalado (enriquece la existente)
La pestaña Alertas gana el **badge de escalado** ("→ coordinador@px / Max") y las acciones de escalado
(forzar/reasignar/tocar el hombro) con `de` exento. Filtrable por proyecto. Es el centro del mando
intermedio automático (supervisor) + humano (RFC Delegación R6/R7).

---

## 3. Widgets reutilizables nuevos (además de los helpers de `tema.rs`)

Componentes que varias pantallas comparten (construir una vez):

| Widget | Uso | Base |
|--------|-----|------|
| `chip_estado_agente` | glifo+color del estado vivo (● ◦ ◐ ✕) | paleta de estados TUI + `chip_estado` |
| `selector_proyecto` | fijar proyecto activo (cabecera) | `Select`/`Breadcrumb` |
| `nodo_organigrama` | la card de nodo (empleado/cargo/operador) | `superficie_card` + `fila_seleccionable` |
| `editor_system_prompt` | textarea + contador + vista previa | `Input` multilínea |
| `selector_cadena_mando` | `Select` de cargos con validación de ciclo | `Select` + validación cliente |
| `barra_pasos_pipeline` | la máquina de estados del agente como stepper | `div`+flexbox |
| `checklist_provision` | los 6 ítems de provisión con ✓/✗ + reparar | `div`+flexbox |
| `badge_escalado` | "→ superior/Max" en alertas | pill 999 |
| `ficha_empleado` | drawer con secciones | `Sheet`/`Resizable` + tabs |

Todos Ethos, cero deps nuevas ([[00-fundamentos-gpui]] §6). Reusan `superficie_card`, `eyebrow`, `titulo`,
`chip_estado`, `boton_primario/secundario`, `fila_seleccionable`, `banner_error` de `tema.rs`, más un
`boton_peligro` (rojo apagado) para acciones destructivas (kick).

---

## 4. Estados de UI transversales (obligatorios en cada pantalla)

Cada vista corporativa cubre los 4 estados (constraint del proyecto):

| Estado | Presentación Ethos |
|--------|--------------------|
| **Cargando** | spinner BRASA + sello "actualizado hace Ns" (patrón `desktop-carga-datos` D2) |
| **Vacío** | mensaje legible ("sin proyectos aún — crea el primero"), no error |
| **Error** | `banner_error` (broker offline/401), la app sigue viva, botón reintentar |
| **Degradado** | estructura sin estado vivo (organigrama en gris; PTY→Terminal externo), banner explicativo |

Sin `.unwrap()`/`.expect()` en prod. Todo cruce con el broker en background.

---

## 5. Mapa feature → UI (qué pantalla expone qué)

| Feature / capacidad | Pantalla que la expone |
|---------------------|------------------------|
| Crear/aislar/relanzar proyecto | Proyectos (2.2) |
| Definir cargo (system prompt, cadena, capacidades) | Cargos (2.3) |
| Contratar + provisionar + lanzar agente | Lanzador/Wizard (2.4) |
| Ver el equipo y quién manda a quién | Organigrama (2.1) |
| Escribir a un agente (privado/público) | Ficha de empleado (2.5) / chat privado (2.4) |
| Delegar / reasignar (restringido por cadena) | Ficha (2.5) / Mi equipo (2.6) / Tareas (existente) |
| Supervisar y escalar | Alertas (2.7) / Mi equipo (2.6) |
| Fichaje, tareas, factor | Ficha (2.5) / Jornada (existente) |
| Bitácora / parte de trabajo | Ficha → Actividad (2.5) / Jornada (RFC registro-acciones) |
| Las 164 features de observación/CRUD | 9 pestañas existentes, filtradas por proyecto activo |

---

## 6. Criterios de aceptación

- **AC1** — El sidebar muestra EMPRESA/OPERAR cuando hay proyectos/cargos; vacío → la app opera como hoy
  (compat, empresa opt-in).
- **AC2** — El selector de proyecto activo filtra las 9 pestañas existentes (toggle este/todos).
- **AC3** — Cargos (2.3): crear un cargo con system prompt + cadena + capacidades persiste (`/admin/cargo`)
  y la vista previa muestra las 5 capas del prompt compuesto.
- **AC4** — Wizard (2.4): la barra de pasos refleja la máquina de estados; el checklist de provisión muestra
  ✓/✗ por ítem; "Instalar MCP aquí" repara; el comando previsualizado incluye `--append-system-prompt` + el
  flag de canal; lanzar arranca las sesiones con id `rol@proyecto`.
- **AC5** — Ficha de empleado (2.5): abre desde organigrama/Peers, muestra identidad+cadena+jornada+tareas+
  bitácora+alertas+chat; las acciones de delegación se restringen por `puede_delegar_a`.
- **AC6** — Cada pantalla cubre los 4 estados (cargando/vacío/error/degradado) sin crash ni `.unwrap()`.
- **AC7** — Todo cruce con el broker va en `background_executor`+`bloquear_en`; ninguna vista hace `.await`
  de red en el hilo de UI (regla anti-SIGABRT).

---

## 7. Constraints

- Ethos exacto; gpui-component real (componentes verificados en RFC Lanzador §4 y `00-fundamentos-gpui`).
  Cero deps nuevas para la UI (el PTY las añade, RFC Lanzador §6.1). Reusa DTOs de `peers-core` y los
  helpers de `tema.rs`. Red en background. La UI dispara acciones (workflow doc 07) y re-lee; no es fuente
  de verdad. Sin `.unwrap()`/`.expect()` en prod. Español salvo protocolo. Versionar plugin al tocar
  binarios. NUNCA `Co-Authored-By`. Jornada en el commit.

## 8. Fuera de alcance (v1)

- Editor de código / árbol de archivos (no es un IDE — RFC Lanzador §9). Dashboard global cross-proyecto
  (v2). Arrastrar nodos del organigrama para reorganizar (v2). Temas alternativos al Ethos. Colaboración
  multi-operador concurrente (YAGNI).

---
#empresa #ui #ethos #gpui #pantallas #wizard #feature-ui-map
