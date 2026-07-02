# RFC — Delegación y cadena de mando (quién delega en quién, escalado, reporte)

> ⬆ [[../_MOC|Mapa]] · Modelo: [[../01-modelo-corporativo]] · Dominio: [[../02-modelo-dominio]] ·
> GPUI: [[../00-fundamentos-gpui]]
>
> Fecha: 2026-07-02. Estado: **PROPUESTO — NO implementar hasta aprobar.**
> Ámbito **mixto**: CONTROL/vista en `peers-desktop`; el motor reusa endpoints del broker existentes
> (`/tarea/asignar`, `/tarea/reasignar`, `/tarea/reportes`, `/enviar`, chat privado). v2 opcional: el broker
> valida la cadena de mando.
> Design System: **Ethos**. Relacionada con: [[RFC-organigrama-roles]] (define `reporta_a`/`puede_delegar_a`),
> [[../../desktop/politica-comunicacion/RFC-politica-comunicacion]] (quién habla con quién),
> [[../../features/supervisor/spec]] (escalado por alertas), [[../../desktop/registro-acciones/RFC-registro-acciones-jornada]]
> (traza de la delegación).

---

## 1. El problema (en palabras de Max)

> "Tiene que haber cómo yo… delegar agentes que yo creo… [con] toda su regla de negocio y cómo va a
> trabajar con los otros peers. Tiene que ser una corporación con lógica real de empresa."

La "lógica real de empresa" es, sobre todo, la **cadena de mando**: quién reparte trabajo a quién, quién
reporta a quién, cómo se escala un bloqueo hacia arriba. Hoy el sistema permite que **cualquiera asigne o
reasigne a cualquiera** (`/tarea/asignar`, `/tarea/reasignar` no miran jerarquía), y no hay noción de
"reportar hacia arriba" ni de "escalar a Max". La coordinación existe como prosa (`COORDENACAO.md`:
"Claudio coordina, Jefin implementa") pero no como estructura que la app respete y muestre.

## 2. La solución

Formalizar **tres flujos** sobre la cadena de mando definida en los cargos ([[RFC-organigrama-roles]] R5:
`reporta_a`, `puede_delegar_a`):

1. **Delegar (hacia abajo)** — Max o un coordinador asigna/reasigna tareas a los cargos que puede delegar.
   La app **ofrece solo destinos válidos** según `puede_delegar_a` (guía en UI); v2: el broker lo valida.
2. **Reportar (hacia arriba)** — un agente reporta progreso; su superior (`reporta_a`) lo ve consolidado.
   Reusa `/tarea/reportar` + `/tarea/reportes` + la bitácora; la novedad es **enrutarlo a quien corresponde**
   en la jerarquía.
3. **Escalar (hacia arriba, excepcional)** — un bloqueo/atasco (alerta del supervisor) sube por la cadena
   hasta encontrar quien puede actuar (coordinador → Max). La app resuelve el destino de escalado.

**Cero infra nueva:** todo se apoya en endpoints existentes + los datos de cadena de mando en los cargos +
el supervisor + la bitácora. La app es el **enrutador** que respeta la jerarquía; v2 opcional endurece en
el broker.

---

## 3. Requisitos (trazables)

### Delegar (hacia abajo)

- **R1 — Destinos de delegación restringidos por cargo.** Al asignar/reasignar una tarea, la app ofrece
  como destinos SOLO los cargos en `puede_delegar_a` del cargo del que delega (más Max, que delega a
  cualquiera). Se materializa con `POST /tarea/asignar` / `/tarea/reasignar` (existentes) hacia el id
  `rol@proyecto` del destino.
- **R2 — Delegación del Operador.** Max (`ID_OPERADOR`) puede delegar a cualquier agente de cualquier
  proyecto (dueño). Sus acciones van con `de = operador` → exentas de política (R3 de la RFC política, ya
  Fase 1) y se registran en la bitácora como acciones del operador ([[../../desktop/registro-acciones/RFC-registro-acciones-jornada]] R5).
- **R3 — Sugerencia de destino por carga.** Al delegar, mostrar la carga de cada destino candidato (nº
  tareas abiertas, ocioso/atascado del supervisor, factor de estimación) para decidir a quién asignar.
  Reusa `/listar-tareas`, `/admin/alertas`, `/factor-estimacion-peer`. No decide por Max; le da contexto.

### Reportar (hacia arriba)

- **R4 — Reporte enrutado al superior.** Un reporte de tarea (`/tarea/reportar`) queda visible para el
  superior (`reporta_a`) del cargo del agente: en la ficha del superior o en el tablero del proyecto,
  sección "reportes de mi equipo". Reusa `/tarea/reportes`; la novedad es la **agregación por subordinados**.
- **R5 — Vista "mi equipo" del coordinador.** Para un cargo con subordinados, una vista que consolida: sus
  tareas, sus reportes, sus alertas, su jornada. Es el "panel del jefe intermedio". Filtra por los ids de
  agente cuyo cargo tiene `reporta_a = <este cargo>`.

### Escalar (excepcional)

- **R6 — Escalado de alertas por la cadena.** Cuando el supervisor emite una alerta (ocioso/atascado/
  ghosteo, [[../../features/supervisor/spec]]) sobre un agente, la app resuelve el **destino de escalado**:
  el superior del agente (`reporta_a`); si no hay o no está vivo, sube hasta Max. La alerta se muestra al
  destino resuelto (y siempre a Max, que ve todo).
- **R7 — Acción de escalado.** Desde una alerta, el destino puede actuar: forzar la tarea (`/tarea/forzar`),
  reasignar (`/tarea/reasignar`), tocar el hombro (`/enviar`/chat privado), o kick (`/salir`). Las acciones
  del operador/coordinador van con `de` exento (no las bloquea la política).

### Traza y transversal

- **R8 — Toda delegación/reporte/escalado se registra.** En la bitácora (`AccionRegistrada`:
  `AsignarTarea`/`ReasignarTarea`/`ReportarTarea`/`ForzarTarea`), atribuida a quien la hizo
  ([[../../desktop/registro-acciones/RFC-registro-acciones-jornada]] R4/R5). El organigrama y la jornada la muestran.
- **R9 — El system prompt del cargo enseña la cadena de mando (v1).** Al lanzar, se vuelca al system prompt:
  "reportas a <superior>; puedes delegar a <lista>; escala bloqueos a <superior>". Así los agentes **cumplen
  por comportamiento** ([[../02-modelo-dominio]] §6). La app además lo **guía** en UI (R1).
- **R10 — Todo degrada, red en background.** Broker offline → banner, la app pinta la estructura de mando
  (config) sin estado vivo. Sin `.unwrap()`/`.expect()` en prod. Red vía `background_executor` + `bloquear_en`.

### v2 opcional (endurecimiento en el broker) — decisión de Max

- **R11 (v2) — Validación server-side de la cadena de mando.** El broker rechaza `/tarea/asignar` si el
  `de` no puede delegar en el `para` según los cargos. Requiere que el broker conozca los cargos (hoy son
  config del operador) → o se le suben, o la validación se queda en la app (v1). **Recomendación: v1
  (guía en UI) + firewall de política para lo prohibido; v2 solo si Max quiere garantía dura.**

---

## 4. Diseño de UI (gpui-component, tema Ethos)

Reusa el organigrama ([[RFC-organigrama-roles]]) como mapa de la cadena de mando (aristas `reporta-a`). Los
flujos se operan desde:

- **Delegar:** `Dialog` "Asignar tarea" con `Select` de destino **filtrado** por `puede_delegar_a` +
  columna de carga (R3). Botón primario BRASA.
- **Panel "mi equipo" (R5):** una pestaña/drawer para un cargo-coordinador: `Tabs` (Tareas · Reportes ·
  Alertas · Jornada) del equipo subordinado.
- **Escalado (R6/R7):** las alertas (pestaña Alertas ya existente) ganan un badge "→ escalada a
  <coordinador>/Max" y botones de acción; el destino las ve resaltadas.

```
┌ Asignar tarea (desde: coordinador@proyecto-x) ────────────────────────────┐
│ Descripción  ┌──────────────────────────────────┐   estimado ▾ [1h]       │
│              │ Revisar el módulo de auth         │                         │
│              └──────────────────────────────────┘                         │
│ Delegar a ▾  ( backend@proyecto-x · 3 tareas ●vivo )  ← solo puede_delegar_a│
│              ( qa@proyecto-x · 1 tarea ○ocioso )                           │
│                                        [ Cancelar ]   [ ● Asignar ]        │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Criterios de aceptación

- **AC1 (R1)** — Desde el cargo "coordinador" (con `puede_delegar_a = [backend, qa]`), el selector de
  destino ofrece solo backend y qa del proyecto (no otros). Max, en cambio, ve todos.
- **AC2 (R2/R8)** — Una asignación de Max se registra en SU bitácora como `AsignarTarea` con `de = operador`
  y no la bloquea la política (exento). El agente destino recibe la tarea.
- **AC3 (R4/R5)** — Un reporte de `backend@proyecto-x` aparece en la vista "mi equipo" de
  `coordinador@proyecto-x` (su superior), consolidado con el resto de su equipo.
- **AC4 (R6)** — Una alerta de atasco sobre `backend@proyecto-x` se marca escalada a
  `coordinador@proyecto-x`; si el coordinador no está vivo, escala a Max. Max siempre la ve.
- **AC5 (R7)** — Desde la alerta escalada, forzar/reasignar/tocar-el-hombro funciona (endpoints existentes)
  y queda en la bitácora.
- **AC6 (R9)** — El system prompt inyectado a `backend@proyecto-x` incluye "reportas a coordinador; escala
  bloqueos a coordinador" (texto, v1).
- **AC7 (compat/R10)** — Sin cadena de mando definida (cargos sin `reporta_a`), la delegación cae al
  comportamiento actual (cualquiera a cualquiera, guiado solo por política); broker caído → banner, sin crash.

---

## 6. Riesgos y decisiones abiertas (Design)

1. **v1 (guía en UI) vs v2 (validación en broker).** v1 no impide que un peer llame `/tarea/asignar`
   directamente saltándose la jerarquía (solo la app lo guía; la política bloquea lo prohibido de
   comunicación, no de asignación). v2 lo garantiza pero exige que el broker conozca los cargos.
   **Recomendación: v1**; v2 si Max detecta que los agentes se saltan la cadena.
2. **Escalado cuando el superior no está vivo.** Regla clara: subir por `reporta_a` hasta encontrar un
   agente vivo o llegar a Max. Max es el tope (siempre disponible como operador). Documentado en R6.
3. **Choque con "tócale el hombro" (VISÃO).** La cadena de mando restringe la delegación, no la
   comunicación espontánea (eso es la política). Un agente puede seguir tocando el hombro a un par aunque no
   pueda *delegarle* una tarea. Mantener separados los dos ejes (habla-con ≠ delega-en). Ya está en el modelo.
4. **Sobre-estructura.** Una cadena de mando rígida puede frenar la autonomía que la VISÃO valora.
   **Recomendación:** la cadena es **opcional por cargo** (`reporta_a: Option`); un equipo plano (todos
   reportan a Max) es válido y es el default. Max añade jerarquía cuando la quiere.
5. **Id del operador y `de` de la delegación.** Mismo tema unificado de identidad ([[../01-modelo-corporativo]]
   §7): las acciones de Max van con `ID_OPERADOR`. Resolver con el binding y la política (ya alineados).

---

## 7. Constraints

- Sin `.unwrap()`/`.expect()` en prod; `Result`/`anyhow`. Español salvo protocolo. Reusa endpoints
  existentes (`/tarea/asignar`, `/tarea/reasignar`, `/tarea/reportar`, `/tarea/reportes`, `/tarea/forzar`,
  `/enviar`, `/admin/alertas`, `/salir`) y los datos de cargo de [[RFC-organigrama-roles]]. Reusa la bitácora
  para la traza. Red en `background_executor` + `bloquear_en`. `#[serde(default)]` para compat. NUNCA
  `Co-Authored-By`. Jornada en el commit. Versionar plugin solo si v2 toca el broker.

## 8. Fuera de alcance (v1)

- Validación server-side de la cadena de mando (v2, R11).
- Flujos de aprobación multi-nivel ("el coordinador propone, Max aprueba") — YAGNI; v1 = Max o coordinador
  actúan directo.
- Balanceo automático de carga entre subordinados (v1 = sugerencia de carga, Max decide; auto = v2+).
- Métricas de desempeño por agente más allá del factor de estimación (v2).

## 9. Dependencias

- **[[RFC-organigrama-roles]]** — fuente de `reporta_a`/`puede_delegar_a` y de los ids `rol@proyecto`.
- **[[RFC-proyectos]]** — el ámbito (el equipo de un proyecto) sobre el que opera la cadena.
- **[[../../features/supervisor/spec]]** — emite las alertas que se escalan (R6).
- **[[../../desktop/registro-acciones/RFC-registro-acciones-jornada]]** — traza de delegación/reporte/escalado (R8).
- **[[../../desktop/politica-comunicacion/RFC-politica-comunicacion]]** — eje ortogonal (habla-con) que no se debe confundir con delega-en.
- **Identidad del operador** ([[../01-modelo-corporativo]] §7) — `de = ID_OPERADOR` para las acciones de Max.

---
#rfc #empresa #delegacion #cadena-de-mando #escalado #peers-desktop
