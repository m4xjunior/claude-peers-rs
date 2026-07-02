# Modelo corporativo — LexusFX como empresa de agentes de IA

> ⬆ [[_MOC|Mapa de la arquitectura corporativa]] · Base técnica: [[00-fundamentos-gpui]] ·
> Materialización de datos: [[02-modelo-dominio]]
>
> Fecha: 2026-07-02. Estado: **PROPUESTA DE ARQUITECTURA DE NEGOCIO — para decidir con Max.**
> Este documento estructura el "modelo de empresa" que Max describió pero no había logrado ordenar.
> NO es código; es la lógica corporativa que las RFCs (`rfc/`) convierten en features engineerables.

---

## 0. El encargo (en palabras de Max)

> "Quiero tornar esos peers y el app desktop mi software donde mis agentes de IA son mis empleados, y
> tengo apartado de proyectos, donde yo con la máquina y el software consigo lanzar mis agentes,
> escribir en ellos por mi app, controlar el servidor SSH o carpeta local donde estarán. Tiene que
> haber cómo yo desarrollar y crear mis proyectos y delegar agentes que yo creo. Esos agentes son un
> system prompt (puede ser otra cosa) que se inserta en el peer al inicio de la conversación, que es
> toda su regla de negocio y cómo va a trabajar con los otros peers. Tiene que ser una corporación con
> lógica real de empresa, que yo todavía no estructuré."

La pieza que falta NO es infraestructura (el broker, las colas, la jornada, las tareas, el push ya
existen). La pieza que falta es el **modelo mental de empresa** que unifica todo eso: quién es quién, qué
manda a qué, cómo se crea un empleado, cómo se aísla un proyecto, quién puede delegar a quién. Este
documento la fija.

---

## 1. Principio rector: la metáfora es LITERAL, no decorativa

El error a evitar es tratar "empresa/empleado/proyecto" como adornos de UI sobre las mismas colas. Aquí
la metáfora corporativa **se mapea 1:1 con primitivas que YA existen en el sistema**. Cada concepto de
empresa tiene un sustrato real y verificable:

| Concepto de empresa | Sustrato real (ya existe en claude-peers-rs) |
|---------------------|----------------------------------------------|
| **Empleado** | una **sesión de Claude Code** (`peer`) registrada en el broker (`Instancia`) |
| **Descripción de puesto / contrato** | el **system prompt** inyectado al lanzar (`--append-system-prompt`) + las instrucciones del MCP (`mcp.rs::instrucciones`) |
| **Fichaje / horas trabajadas** | la **jornada** que timbra el broker con SU reloj (`Sesion`, regla sagrada: la IA nunca inventa el tiempo) |
| **Órdenes de trabajo** | las **tareas** (`Tarea`, con estimado IA vs real medido, factor de corrección aprendido) |
| **Parte de trabajo / bitácora del empleado** | el **registro de acciones** (`AccionRegistrada`, RFC registro-acciones) |
| **Oficina de correos interna** | el **broker** ruteando mensajes (`/enviar`, bandeja durable ZSET, ACK) |
| **"Tocar el hombro" de un compañero** | el **push `<channel>`** que hace aparecer el mensaje en la sesión del peer |
| **Reglas de quién habla con quién** | la **política de comunicación** (firewall peer↔peer, ya Fase 1) |
| **Supervisión / mandos intermedios** | el **supervisor** (detector de ociosos/atascados/ghosteo → alertas) |
| **Expediente público de la empresa** | las **GitHub Issues** (tarea→issue, report→comentario, cerrar→cierra) |
| **RRHH + nómina + verdad del estado** | el **broker** (fuente de verdad; la app es su espejo) |
| **El despacho del dueño** | la app **`peers-desktop`** (donde Max lanza, escribe, delega, supervisa) |

**Consecuencia de diseño:** la "corporación" NO es una capa nueva de infraestructura. Es un **modelo de
organización** (organigrama, roles, proyectos, delegación) que se **superpone** a estas primitivas y les
da estructura. Casi todo el motor ya está; lo que falta es la **identidad organizativa** y los
**contenedores** (proyecto, rol, cargo) que hoy no existen como entidad.

---

## 2. La jerarquía corporativa (los cinco niveles)

Se propone una jerarquía de cinco niveles, de lo más estable a lo más efímero:

```
  EMPRESA (LexusFX)                     ← una sola; la constante del sistema
    └─ PROYECTO (workspace)             ← "apartado de proyectos": aislado, con su equipo, su board, su carpeta/host
         └─ EQUIPO / DEPARTAMENTO       ← agrupación de roles dentro del proyecto (backend, front, QA…)
              └─ CARGO / ROL (plantilla)← la "descripción de puesto": system prompt + reglas + capacidades
                   └─ AGENTE (empleado) ← una sesión viva de Claude ocupando un cargo en un proyecto
```

- **Empresa:** el marco. Define al **Operador** (Max) como identidad reservada y no-suplantable
  (`ID_OPERADOR`, ya en `peers-core`), y los valores globales (VISÃO, principios, política por defecto).
  Es única.
- **Proyecto:** el "apartado de proyectos" que Max pide. Un contenedor **aislado**: su carpeta local o su
  host SSH, su equipo de agentes, su tablero de tareas, su política de comunicación, su registro de
  acciones. Dos proyectos no comparten equipo ni board por defecto. Es la unidad de trabajo.
- **Equipo/Departamento:** agrupación **opcional** de cargos dentro de un proyecto (v1 puede ser un simple
  campo/etiqueta; los grupos de la política de comunicación —`Patron::Grupo`, ya previsto— son su forma
  natural de crecer).
- **Cargo/Rol:** la **plantilla** reutilizable. NO es un empleado: es la definición de puesto ("peer
  backend Rust", "revisor QA", "coordinador"). Contiene el system prompt, las reglas de negocio, las
  capacidades/permisos y las relaciones esperadas con otros cargos. Se guarda una vez y se reutiliza.
- **Agente/Empleado:** una **instancia viva** de un cargo, asignada a un proyecto: una sesión de Claude
  Code corriendo en la carpeta/host del proyecto, con el system prompt del cargo inyectado. Es efímero
  (puede reiniciarse), pero su **identidad de puesto** (rol + proyecto) es durable.

> **Distinción clave (resuelve una ambigüedad del sistema hoy):** el sistema actual solo tiene
> **presencia efímera** (`Instancia` = un peer vivo, que desaparece al cerrar). El modelo corporativo
> añade **identidad durable**: el *cargo* y el *proyecto* sobreviven al peer. Esto conecta directo con la
> decisión de `bitacora.db` con `peers_conocidos`/`tareas_conocidas` (ADR-001): la identidad durable ya
> se empezó a introducir por la puerta de la trazabilidad; el modelo corporativo la formaliza.

---

## 3. Qué es "un agente" exactamente (el corazón del encargo)

Max lo definió: *"un system prompt (puede ser otra cosa) que se inserta en el peer al inicio de la
conversación, que es toda su regla de negocio y cómo va a trabajar con los otros peers."* Lo precisamos:

Un **Cargo/Rol** (la plantilla del agente) se compone de:

1. **Identidad** — nombre del puesto ("Backend Rust", "QA visual", "Coordinador"), un id de rol estable, y
   un id de agente derivado (rol + proyecto) que resuelve la colisión de ids (ver §7).
2. **System prompt (la regla de negocio)** — el texto que se inyecta con `--append-system-prompt` al
   lanzar. Es "toda su regla de negocio". Editable y versionable como plantilla (RFC Lanzador R2.1 ya
   contempla plantillas de system prompt guardadas).
3. **Protocolo de colaboración (cómo trabaja con otros)** — cómo se relaciona con los demás cargos: a
   quién reporta, a quién puede delegar, con quién NO debe hablar. Esto se materializa en **dos** sitios:
   - la **política de comunicación** (quién PUEDE escribir a quién — firewall del broker, ya Fase 1), y
   - la **cadena de mando** (quién DEBE delegar/reportar a quién — RFC Delegación, nueva).
   Las instrucciones del MCP (que ya se inyectan en cada sesión) enseñan al peer el protocolo general
   ("tócale el hombro a un compañero", "ficha tus tareas"); el system prompt del cargo lo especializa.
4. **Capacidades / permisos** — qué puede hacer ese agente: ¿puede crear tareas? ¿asignar a otros?
   ¿lanzar sub-agentes? ¿tocar producción? En v1 esto es sobre todo **texto en el system prompt** +
   reglas de política; a futuro puede endurecerse con permisos reales en el broker (ver §8, riesgos).
5. **Ubicación de trabajo por defecto** — carpeta local o host SSH donde suele desplegarse (heredado del
   proyecto, ver §4). Es el "controlar el servidor SSH o carpeta local" del encargo.

> **"puede ser otra cosa" (la puerta que Max dejó abierta):** hoy el mecanismo es el system prompt +
> instrucciones MCP. Es lo único que el harness de Anthropic garantiza. Un "cargo" más rico (herramientas
> MCP específicas por rol, permisos duros) es posible pero **depende de qué expone el harness**; se deja
> como evolución (§8), no como bloqueo de v1. El system prompt es suficiente para empezar a operar como
> empresa HOY.

---

## 4. El proyecto como contenedor aislado ("apartado de proyectos")

Un **Proyecto** es la unidad que Max pidió aislar. Contiene:

- **Ubicación** — carpeta local (elegida con el file picker nativo, [[00-fundamentos-gpui]] §6) **o** host
  SSH (reusa el multi-broker/hosts de RFC Acceso). Es donde corren físicamente sus agentes.
- **Equipo** — el conjunto de cargos/agentes asignados a ese proyecto. Un mismo cargo-plantilla ("QA")
  puede instanciarse en varios proyectos, pero cada instancia es un agente distinto ligado a SU proyecto.
- **Tablero** — sus tareas (filtradas del broker por el proyecto/equipo). El estimado-vs-real y el factor
  de estimación aprendido pueden verse por proyecto.
- **Política de comunicación local** — reglas de quién habla con quién dentro del proyecto (v1: la
  política es global en el broker; el proyecto la usa como filtro/preset — ver §8 sobre alcance).
- **Registro de acciones** — la bitácora de lo que hizo el equipo del proyecto (RFC registro-acciones,
  agregable por proyecto).
- **Perfil de lanzamiento** — la combinación reutilizable {carpeta/host + cargos + tareas iniciales +
  flags} que Max relanza con un click (RFC Lanzador R9, ya contempla "perfiles persistidos").

**Aislamiento:** por defecto, dos proyectos no comparten board ni equipo, y sus agentes no se hablan entre
proyectos (regla de política `proyecto-A/* → proyecto-B/*: bloquear` como preset opcional). Esto da el
"apartado" que Max quiere sin infra nueva: es política + filtrado + convención de ids.

---

## 5. El ciclo de vida de operar la empresa (el flujo de Max)

Este es el recorrido concreto que la app debe soportar, extremo a extremo:

```
1. CREAR PROYECTO      → elegir carpeta local o host SSH; nombrarlo; (opcional) clonar preset de equipo.
2. DEFINIR CARGOS      → escribir/elegir plantillas de rol (system prompt + reglas + relaciones).
3. CONTRATAR AGENTES   → instanciar N cargos en el proyecto (asignar rol → agente).
4. LANZAR              → la app arranca cada agente: `claude --append-system-prompt "<rol>"
                         --dangerously-load-development-channels server:claude-peers` en su
                         carpeta/host, dentro de un PTY embebido o tmux (RFC Lanzador).
5. DELEGAR / ESCRIBIR  → Max asigna tareas (board o chat privado) y escribe a un agente sin que se
                         renderice en su TUI (chat privado pull, RFC Lanzador §7). Los agentes delegan
                         entre sí según la cadena de mando (RFC Delegación).
6. TRABAJAR (autónomo) → los agentes fichan tareas, trabajan, se "tocan el hombro", reportan. El broker
                         timbra el tiempo real y aprende el factor. "Max sale de perto" (VISÃO).
7. SUPERVISAR          → el supervisor alerta de ociosos/atascados/ghosteo; la app los muestra por
                         proyecto/agente. Max actúa (forzar, reasignar, kick, desbloquear comunicación).
8. RENDIR CUENTAS      → jornada (horas), tablero (estimado vs real), bitácora (qué hizo cada uno),
                         GitHub Issues (expediente público). El "parte de trabajo" de la empresa.
9. CERRAR / PAUSAR     → cerrar tareas, pausar agentes (kick), archivar o relanzar el proyecto (perfil).
```

Cada paso ya tiene sustrato: **1-4** = RFC Proyectos + RFC Organigrama/Roles + RFC Lanzador; **5** = RFC
Delegación + chat privado + política; **6** = tareas-autogestionadas + jornada + push; **7** = supervisor +
alertas; **8** = jornada + registro-acciones + GitHub Issues; **9** = kick + perfiles. La arquitectura
corporativa es, en gran parte, **coser lo existente con un modelo de identidad organizativa**.

---

## 6. El organigrama y las relaciones (la "lógica real de empresa")

La "lógica de empresa" que Max no había estructurado se reduce a **tres relaciones** entre cargos, y cada
una tiene un motor real:

1. **Puede-hablar-con** (topología de comunicación) → **política de comunicación** (firewall del broker,
   Fase 1). Ej.: "front no habla con backend directamente, pasa por el coordinador".
2. **Delega-en / reporta-a** (cadena de mando) → **RFC Delegación** (nueva). Ej.: "el coordinador reparte
   tareas a los devs; los devs reportan al coordinador; el coordinador reporta a Max". Se materializa con
   `/tarea/asignar`, `/tarea/reasignar`, reportes y chat privado, restringidos por rol.
3. **Supervisa-a** (mando intermedio / dueño) → **supervisor** + acciones del operador. El supervisor
   vigila; Max (o un cargo "coordinador" con permiso) actúa sobre las alertas.

El **organigrama** es la vista que dibuja estas tres relaciones sobre el equipo de un proyecto (o de toda
la empresa). En GPUI es una vista `Render` que observa las entidades Proyecto/Cargo/Agente y sus
relaciones ([[00-fundamentos-gpui]] §8). No es infra: es visualización + los motores que ya existen.

### Roles arquetípicos sugeridos (punto de partida, editable)

No se imponen; son plantillas iniciales que Max puede ajustar:

- **Operador (Max)** — dueño. Identidad reservada (`ID_OPERADOR`), nunca bloqueable, delega a cualquiera,
  ve todo. No es un agente IA; es el humano en la app.
- **Coordinador / PM** — reparte y reasigna tareas, revisa reportes, escala a Max. Nodo de la cadena de
  mando. (Espeja el rol "Claudio/Julio: coordina/QA" de `COORDENACAO.md`.)
- **Especialista** (Backend, Frontend, etc.) — ejecuta tareas de su dominio, ficha su jornada, se toca el
  hombro con pares. (Espeja "Jefin/Aluísio: implementan".)
- **Revisor / QA** — revisa el trabajo de otros; puede tener comunicación de solo-lectura o canal
  dedicado. (Espeja "Front-QA".)
- **Investigador** — produce docs/decisiones, no toca producción.

> Estos roles NO son inventados: son exactamente los que `COORDENACAO.md` ya describe informalmente
> (coordinador, implementadores, QA, investigadores). El modelo corporativo **formaliza como plantillas
> de cargo** lo que hoy vive como prosa de coordinación.

---

## 7. Identidad: el problema que hay que resolver primero

Toda la lógica corporativa depende de **identidad estable**, y el sistema tiene aquí su deuda técnica más
citada (aparece en `STATE.md`, RFC política-comunicación §5.3, RFC Lanzador §6.2, RFC registro-acciones
§5.3). Se unifica aquí porque el modelo corporativo lo necesita como cimiento:

- **Operador (`ID_OPERADOR`)** — ya reservado en `peers-core`, exento de bloqueo. Es "Max desde la
  desktop/TUI". El modelo corporativo lo consagra como el dueño.
- **Colisión de ids (RESUELTO 2026-07-02, commit 1f4187f)** — el registro atómico + sufijado (-2/-3) ya
  permite varias instancias por carpeta sin colapsar. Base suficiente para múltiples agentes.
- **Lo que el modelo corporativo AÑADE:** el id de un agente debe derivar de **rol + proyecto**, no solo de
  la carpeta. Ej.: `backend@proyecto-x`, `qa@proyecto-x`. Esto:
  - hace la identidad **legible** (Max ve "backend@proyecto-x", no "instancia-2"),
  - hace la identidad **durable** (el rol/proyecto sobrevive al reinicio del peer),
  - resuelve el binding id↔sesión que el Lanzador dejó abierto (RFC Lanzador §6.2): la app **fija** el id
    al lanzar (env/flag que el `peers-client` respeta al registrarse), en vez de correlacionar por cwd.

> **Decisión pendiente para Max (recomendada):** que el `peers-client` acepte un id explícito
> (`CLAUDE_PEERS_ID` ya existe como flag) y que la app lo componga como `<rol>@<proyecto>`. Es un cambio
> pequeño de backend que **desbloquea** proyectos, delegación, chat privado y bitácora a la vez. Se
> detalla en [[02-modelo-dominio]] y en RFC Organigrama/Roles.

---

## 8. Decisiones abiertas que Max debe tomar (con recomendación)

La arquitectura de negocio tiene bifurcaciones que dependen de Max. Se listan con una recomendación para
no dejarlas al aire:

1. **¿Alcance de la política por proyecto o global?** Hoy la política de comunicación es **global** en el
   broker (`RwLock<Politica>` único). Para aislar proyectos hace falta o (a) reglas con prefijo de proyecto
   sobre la política global (`proyecto-a/* → proyecto-b/*: bloquear`), o (b) una política por proyecto.
   **Recomendación:** (a) en v1 — reusa el motor existente, sin infra nueva; los grupos (`Patron::Grupo`,
   ya previsto) modelan proyecto/equipo. (b) es evolución si Max quiere aislamiento fuerte.
2. **¿Cómo se define un "cargo" — solo system prompt, o también capacidades duras?** **Recomendación:**
   v1 = system prompt + instrucciones MCP + reglas de política (todo lo que el harness garantiza HOY).
   Capacidades duras (permisos reales por rol en el broker, herramientas MCP por rol) = v2, cuando se
   valide qué expone el harness. No bloquear la empresa por esto.
3. **¿La identidad de agente la fija la app (rol@proyecto) o se sigue derivando de la carpeta?**
   **Recomendación:** la app la fija (§7). Es el cimiento; sin esto, proyectos y delegación son frágiles.
4. **¿La delegación entre agentes la fuerza el sistema o la guía el system prompt?** **Recomendación:**
   v1 = la **guía** el system prompt del cargo (los agentes delegan porque su rol lo dice) + la política
   restringe lo prohibido. Forzarla en el broker (rechazar una asignación que viola la cadena de mando) =
   v2. Empieza por convención + firewall, endurece después.
5. **¿Un proyecto puede vivir en varias máquinas (carpeta local + hosts SSH mezclados)?**
   **Recomendación:** sí, el modelo lo permite (cada agente tiene su ubicación); el broker central ya
   distingue peers por hostname. Es una fortaleza del diseño cross-host existente.
6. **¿"Contratar" crea el agente parado o lo lanza?** **Recomendación:** separar **contratar** (definir el
   agente: rol+proyecto+ubicación, sin proceso) de **lanzar** (arrancar la sesión). Así Max diseña la
   plantilla del equipo y luego la despliega de un click (perfil de lanzamiento).

---

## 9. Qué NO es esto (límites honestos)

- **No es un orquestador que "piensa por" los agentes.** Los agentes son Claudes autónomos; la empresa les
  da estructura (rol, proyecto, cadena de mando, canal), no un planificador central. La autonomía es de la
  VISÃO ("sair de perto").
- **No inventa infraestructura.** Cero message brokers nuevos (RabbitMQ ya descartado), cero servicios
  externos. Todo sobre el broker + Redis/SQLite + GPUI existentes. La única dep nueva ya aceptada es `sqlx`
  para la bitácora (identidad durable), que el modelo corporativo aprovecha.
- **No promete "push invisible" ni permisos que el harness no da.** El chat privado es *pull* (RFC
  Lanzador §7); las capacidades de rol son texto v1. Lo que dependa del harness de Anthropic se marca como
  bloqueo externo, no como feature entregada.
- **No es un IDE.** El file picker elige la carpeta del proyecto; no hay editor de código dentro de la app
  (RFC Lanzador §9). El "puesto de trabajo" es el PTY donde corre `claude`.

---

## 10. Mapa a las RFCs (qué documento diseña qué)

| Pieza del modelo corporativo | RFC / spec que la ingenieriza |
|------------------------------|------------------------------|
| Modelo de datos (Empresa/Proyecto/Cargo/Agente + wire) | [[02-modelo-dominio]] |
| "Apartado de proyectos" (crear/aislar proyecto, ubicación local/SSH, board por proyecto) | [[rfc/RFC-proyectos]] |
| Cargos/roles, definición de agente, contratar, plantillas de system prompt, organigrama, id rol@proyecto | [[rfc/RFC-organigrama-roles]] |
| Delegación, cadena de mando, reportar-a/delega-en, escalado | [[rfc/RFC-delegacion-cadena-mando]] |
| Lanzar el agente, escribir en él (chat privado), terminal, SSH/tmux | [[../desktop/lanzador/RFC-lanzador]] (existente) |
| Quién habla con quién (firewall) | [[../desktop/politica-comunicacion/RFC-politica-comunicacion]] (existente, Fase 1) |
| Fichaje/horas, órdenes de trabajo, factor de estimación | [[../features/tareas-autogestionadas-aprendizaje/spec]], jornada (existentes) |
| Supervisión (mandos intermedios) | [[../features/supervisor/spec]] (existente) |
| Parte de trabajo / bitácora | [[../desktop/registro-acciones/RFC-registro-acciones-jornada]] (existente) |
| Re-lectura corporativa de TODO el backlog desktop (164 features) | [[rfc/RFC-relectura-features-existentes]] |

---
#empresa #modelo-corporativo #arquitectura #negocio #agentes #proyectos
