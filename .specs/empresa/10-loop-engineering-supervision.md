# Loop engineering + supervisión jerárquica — el modo de operación iterativo de la empresa

> ⬆ [[_MOC|Mapa]] · Decisión: [[06-decisiones]] E-23 · Delegación: [[rfc/RFC-delegacion-cadena-mando]] ·
> Supervisor: [[../features/supervisor/spec]] · Workflow: [[07-workflow-trazable-164-features]] ·
> Organigrama: [[05-organigrama-visual-ethos]]
>
> Fecha: 2026-07-03. Estado: **ARQUITECTURA — driver del loop DECIDIDO por el arquitecto, a validar con Max.**
> Max: *"la app debe hacer loop engineering en las tareas con cada peer, supervisado por el supervisor de
> cada dpto, y yo superviso el todo, donde veo todo y todos y lo que se hace iterativamente."* Aquí se
> define el **modo de operación iterativo** de la empresa: cómo trabaja un agente en loop, cómo lo
> supervisa su dpto, y cómo Max lo ve todo en vivo. Reusa las primitivas existentes; añade la **noción de
> iteración** y la **consola de supervisión**.

---

## 0. Qué es "loop engineering" aquí

No es un bucle de CPU: es el **ciclo de trabajo iterativo** de un empleado sobre una tarea, con revisión
en cada vuelta. La empresa fabrica software (Max: *"a empresa desenvolve softwares"*), y el software se
hace **iterando**: hacer → reportar → revisar → corregir → repetir → cerrar. La app orquesta ese ciclo por
cada peer y lo hace **visible y supervisado** en tres niveles: el agente, su supervisor de dpto, y Max.

**Principio (decisión E-23, driver = agente auto-itera):** el agente **conduce su propia iteración**
(autonomía de la VISÃO, "sair de perto"), guiado por su system prompt + la skill MCP. La app y los
supervisores **no re-prompean cada paso** (eso gastaría tokens de más — justo lo que Max quiere evitar):
**observan, miden, revisan y corrigen cuando hace falta.** El agente hace solo lo designado; el sistema
mide y encauza.

---

## 1. La iteración (la unidad del loop)

Una **iteración** es una vuelta de trabajo sobre una tarea, delimitada por un **reporte**. Se apoya en
tools que YA existen (`crear_tarea`/`reportar_tarea`/`cerrar_tarea`, `mcp.rs`) — la novedad es
**nombrarlas como iteraciones y medirlas**:

```
crear_tarea(desc, estimado)         → abre la tarea (iteración 0); el broker timbra inicio
   │
   ├─ trabaja …
   ├─ reportar_tarea(texto)          → CIERRA la iteración N, ABRE la N+1; el broker timbra cada reporte
   ├─ trabaja …
   ├─ reportar_tarea(texto)          → iteración N+1 → N+2
   │
cerrar_tarea()                       → cierra la tarea; el broker mide el real y aprende el factor
```

- **Cada reporte = un latido de progreso de la iteración.** El broker ya persiste los reportes
  (`cprs:reportes:{id}`, `tarea_reportar`); aquí se **interpretan como iteraciones** con su timestamp
  (el broker timbra `cuando`, nunca la IA — regla sagrada).
- **Métrica de iteración (nueva, derivada):** nº de iteraciones por tarea, cadencia (tiempo entre
  reportes), y "estancamiento" (sin reporte > umbral = el `Atascado` que el supervisor ya detecta,
  `UMBRAL_ATASCO_SEG`, `lib.rs:715`). No hace falta un tipo nuevo: se deriva de los reportes + la jornada.
- **El agente decide cuántas iteraciones**; el sistema las cuenta y las expone. Esto es "lo que se hace
  iterativamente" que Max quiere ver.

> **Por qué esto es barato en tokens:** el agente ya reporta como parte de su flujo (las instrucciones MCP
> se lo piden). No añadimos re-prompts: **reusamos sus reportes como señal de iteración.** El coste extra
> es cero para el agente; la inteligencia está en cómo el broker/la app **agregan** esa señal.

---

## 2. Los tres niveles de supervisión (la jerarquía)

Max: *"supervisado por el supervisor de cada dpto, y yo superviso el todo."* Tres niveles, cada uno con su
alcance y su mecanismo (todo sobre primitivas existentes):

### Nivel 1 — El agente (auto-supervisión)
Se conduce a sí mismo: ficha su tarea, itera, reporta, cierra. Su skill MCP ([[04-conocimiento-agente]]
§3) le enseña a consultar su cadena de mando y a escalar si se bloquea. Es el driver del loop (E-23).

### Nivel 2 — El supervisor de departamento (revisa el loop de su equipo)
Un **cargo/agente** con rol de supervisor (`departamento` + subordinados que le `reporta_a`,
[[02-modelo-dominio]] §2.1). Su trabajo es **revisar las iteraciones de su equipo**:
- Ve los reportes de sus subordinados (panel "mi equipo", [[08-capa-ui]] §2.6) — vía `/tarea/reportes` +
  `/listar-tareas` filtrados por `reporta_a`.
- Recibe las **alertas** del supervisor automático (ocioso/atascado/ghosteo) de su dpto, **escaladas por
  la cadena** ([[rfc/RFC-delegacion-cadena-mando]] R6): un `Atascado` de un subordinado le llega a él.
- **Corrige el loop:** toca el hombro (`enviar`/chat privado), reasigna (`/tarea/reasignar`), fuerza
  (`/tarea/forzar`), o desbloquea — todo respetando la regla dura (E-12), y todo dejando bitácora
  (`AccionRegistrada`).
- Es **un agente más** que corre en loop: su "tarea" es supervisar; sus iteraciones son sus revisiones.

### Nivel 3 — Max (ve todo y todos, en vivo)
El **operador** (`ID_OPERADOR`) en la cúspide: ve **todos los departamentos, todos los peers, y lo que se
hace iterativamente**, en una **consola de supervisión en vivo** (§3). Supervisa a los supervisores;
interviene donde quiera (nunca bloqueable). Es el "veo todo y todos" literal.

```
        Max (ve TODO, en vivo)                    ← consola global de supervisión (§3)
          │ supervisa a
   ┌──────┴──────┬───────────────┐
 sup. backend  sup. frontend   sup. QA           ← Nivel 2: cada uno revisa el loop de SU dpto
   │             │               │
 [b1 b2]       [f1 f2]         [q1]               ← Nivel 1: agentes auto-iteran en sus tareas
  loop          loop            loop
```

---

## 3. La consola de supervisión en vivo (lo que Max ve)

"Veo todo y todos, y lo que se hace iterativamente" = una vista en vivo que agrega los loops. Es la unión
del **organigrama** ([[05-organigrama-visual-ethos]]) con un **feed de iteraciones**:

```
┌ Supervisión — LexusFX (todos los proyectos) ───────────────  ● 7 vivos · 2 atascados · hace 3s ↻ ┐
│  Por departamento:                                                                                │
│   ▸ backend   ██████░░ 3 tareas · 12 iter/h · 1 atascada    sup: coordinador-be@px  ●             │
│   ▸ frontend  ████████ 2 tareas · 8 iter/h                  sup: coordinador-fe@px  ●             │
│   ▸ qa        ███░░░░░ 1 tarea  · 2 iter/h · ⚠ ghosteo       sup: lead-qa@px         ◐             │
│                                                                                                   │
│  Feed de iteraciones (en vivo):                                                                   │
│   17:04  backend@px    reportó   «módulo auth compila, faltan tests»      tarea t-12 · iter 4     │
│   17:03  qa@px         ⚠ atascada 32m sin reporte  → escalada a lead-qa@px                         │
│   17:01  frontend@px   cerró     «pantalla lista» t-09 · 3 iter · real 48m (est ×1.2)             │
│   16:58  coordinador   reasignó  t-07 backend@px → backend-2@px  (regla dura ✓)                    │
└────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Barra por dpto:** progreso (tareas abiertas/hechas), **cadencia de iteración** (iter/h derivada de los
  reportes), atascos, y **quién es su supervisor** con su estado vivo. De un vistazo, Max ve la salud de
  cada equipo.
- **Feed de iteraciones en vivo:** el stream cronológico de reportes/cierres/escalados/acciones —
  exactamente "lo que se hace iterativamente". Es la **bitácora** (`AccionRegistrada` +
  `reportar_tarea`) renderizada como feed, refrescada en vivo (patrón `desktop-carga-datos` D2).
- **Drill-down:** click en un dpto → su panel "mi equipo"; click en un peer → su ficha; click en una
  iteración → el detalle de la tarea. La supervisión es navegable de arriba abajo.
- **Alcance:** global (toda la empresa) o por proyecto (selector activo). Max ve el todo; cada supervisor
  ve su dpto (misma vista, filtrada por su `reporta_a`).

---

## 4. Cómo la app "hace loop engineering" (el rol de la aplicación)

La app NO ejecuta el trabajo ni re-promptea al agente (eso es del agente, E-23). La app **orquesta y hace
visible** el loop:
1. **Lanza** el agente con su tarea designada (pipeline, [[03-pipeline-provision]]) — el agente arranca su
   loop solo.
2. **Mide** cada iteración (agrega reportes/jornada del broker) y calcula cadencia/estancamiento.
3. **Enruta** las alertas por la cadena de mando (escalado al supervisor del dpto, luego a Max).
4. **Ofrece las palancas** de corrección (reasignar/forzar/tocar el hombro) a quien tiene permiso (regla
   dura), desde la consola.
5. **Cierra el bucle de aprendizaje:** cada tarea cerrada alimenta el factor de estimación (ya existe) —
   el sistema aprende cuántas iteraciones/tiempo real toma cada tipo de trabajo, y **corrige las
   estimaciones futuras**. Eso es "engineering" del loop: no solo iterar, sino **medir y mejorar**.

> **Loop engineering = iterar + medir + supervisar + aprender.** Las cuatro piezas ya tienen sustrato:
> iterar (tools de tarea), medir (jornada/reportes timbrados), supervisar (alertas + jerarquía), aprender
> (factor de estimación). Este documento las **une en un modo de operación** con una consola que lo hace
> visible en los tres niveles.

---

## 5. Requisitos (trazables)

### Iteración
- **R1** — Interpretar cada `reportar_tarea` como el cierre de una iteración; derivar **nº de iteraciones**
  y **cadencia** (tiempo entre reportes) desde `cprs:reportes:{id}` + jornada. Sin tipo nuevo; derivado.
- **R2** — Exponer las métricas de iteración por tarea/peer/dpto vía endpoint (o derivarlas en la app
  desde `/tarea/reportes` + `/jornada` + `/listar-tareas`). Decidir en Design: derivar en cliente
  (cero backend) vs endpoint agregado nuevo.
- **R3** — El **estancamiento** reusa el `Atascado` del supervisor (`UMBRAL_ATASCO_SEG`) — no re-inventar.

### Supervisión jerárquica
- **R4** — Cada dpto tiene un **cargo supervisor** (un agente con subordinados que le `reporta_a`). Las
  alertas de un subordinado se **escalan** a su supervisor (cadena de mando, RFC Delegación R6); si no está
  vivo, suben a Max.
- **R5** — El supervisor revisa el loop de su equipo desde "mi equipo" ([[08-capa-ui]] §2.6) y corrige con
  las palancas existentes (reasignar/forzar/tocar el hombro), bajo regla dura (E-12) y con bitácora.
- **R6** — Max ve **todos** los dptos y peers en la **consola de supervisión** (§3), en vivo, con drill-down.
  Cada supervisor ve la misma consola filtrada a su dpto.

### Consola en vivo
- **R7** — Barra por dpto (progreso · iter/h · atascos · supervisor + estado) + **feed de iteraciones en
  vivo** (reportes/cierres/escalados/acciones de la bitácora), refresco en vivo (patrón D2).
- **R8** — Drill-down: dpto → mi equipo; peer → ficha; iteración → detalle de tarea.
- **R9** — Alcance global (empresa) o por proyecto (selector activo).

### Aprendizaje
- **R10** — Cada tarea cerrada alimenta el factor de estimación (ya existe); exponer "iteraciones/tiempo
  real por tipo de trabajo" como métrica del dpto (mejora del loop).

### Transversal
- **R11** — Todo degrada (broker offline → banner, sin crash), red en `background_executor` + `bloquear_en`,
  tiempo timbrado por el broker, sin `.unwrap()`/`.expect()` en prod. La app **no re-promptea** al agente
  (driver = agente, E-23): observa/mide/enruta/corrige.

---

## 6. Criterios de aceptación

- **AC1 (R1)** — Una tarea con 3 reportes muestra "3 iteraciones" y la cadencia (min entre reportes),
  derivadas de datos timbrados por el broker.
- **AC2 (R4)** — Un `Atascado` de `backend@px` se escala a su supervisor (`coordinador-be@px`); si no vive,
  a Max. Ambos lo ven en su consola.
- **AC3 (R5)** — El supervisor reasigna/fuerza una tarea de su subordinado desde "mi equipo"; el broker lo
  permite (regla dura: es su cadena) y lo registra en bitácora; una acción fuera de su cadena se rechaza.
- **AC4 (R6/R7)** — La consola de Max muestra todos los dptos con su cadencia de iteración y el feed en
  vivo; refresca solo; drill-down navega a mi equipo/ficha/tarea.
- **AC5 (R10)** — Cerrar una tarea con estimado+real actualiza el factor; el dpto muestra "iteraciones/tiempo
  real" agregado.
- **AC6 (E-23/R11)** — La app NO re-promptea al agente entre iteraciones (verificable: el agente reporta por
  su cuenta; la app solo lee). Todo degrada sin crash.

---

## 7. Riesgos y decisiones (a [[06-decisiones]])

1. **Driver del loop (E-23).** Base = agente auto-itera; alternativas (app-driven / supervisor-driven) más
   caras en tokens/latencia. **Validar con Max** (quedó sin responder por un fallo de la herramienta).
2. **¿Métricas de iteración en cliente o endpoint nuevo?** Derivar en cliente (cero backend) es suficiente
   v1; un endpoint agregado (`/admin/iteraciones`) es optimización si el cálculo se vuelve pesado. Recom:
   cliente v1.
3. **El supervisor es un agente → también consume tokens.** Mitigar: su loop es de **revisión** (lee
   reportes, actúa solo en excepciones/alertas), no de trabajo continuo. La mayoría del tiempo observa; el
   supervisor automático (broker, gratis) hace la detección; el supervisor-agente solo decide.
4. **Sobre-supervisión.** Demasiada jerarquía frena la autonomía. La cadena es **opcional por cargo**
   (E-20); un equipo plano (todos reportan a Max) es válido. Max añade supervisores donde el volumen lo pida.

---

## 8. Constraints

- Reusa tools de tarea, reportes, supervisor (alertas), delegación (escalado), jornada (medición) y factor
  (aprendizaje) EXISTENTES — la novedad es agregarlos en un modo de operación + consola. El agente conduce
  (E-23); la app observa/mide/enruta/corrige, **no re-promptea**. El tiempo lo timbra el broker. Red en
  background. Degrada sin crash. Con E-22, se pueden añadir libs de robustez si hacen falta (p.ej. para el
  stream en vivo), manteniendo el binario portable. Español salvo protocolo. NUNCA `Co-Authored-By`.

## 9. Fuera de alcance (v1)

- Re-prompting automático del agente (driver b/c — solo si Max lo pide). Métricas de desempeño más allá de
  iteraciones + factor. Balanceo automático de carga entre subordinados. Un motor de reglas de escalado
  configurable (v1 = escalado por `reporta_a`).

---
#empresa #loop-engineering #supervision #iteracion #jerarquia #consola-vivo
