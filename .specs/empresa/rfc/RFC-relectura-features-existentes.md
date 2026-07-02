# RFC — Relectura corporativa de las features ya documentadas

> ⬆ [[../_MOC|Mapa]] · Modelo: [[../01-modelo-corporativo]] · Dominio: [[../02-modelo-dominio]]
>
> Fecha: 2026-07-02. Estado: **ANÁLISIS / PUENTE** (no propone features nuevas; reencuadra las existentes).
> Max pidió: *"me entregue la ingeniería de estas features levando en cuenta que debe hacer lo mismo para
> las que ya están documentadas."* Este documento hace exactamente eso: toma el backlog ya escrito (las 9
> RFCs de pestaña = 164 features, + Lanzador, + Política, + registro-acciones, + supervisor, +
> tareas-autogestionadas) y lo **reinterpreta bajo el modelo corporativo**, marcando qué cambia, qué se
> reusa tal cual, y qué dependencias nuevas introduce la capa empresa.

---

## 0. Tesis

Ninguna feature documentada se tira. La capa corporativa (proyecto/cargo/agente/cadena de mando) **no
reemplaza** el backlog: lo **organiza y lo dota de contexto**. En términos prácticos, casi todas las 164
features siguen igual, con **dos cambios transversales**:

1. **Ámbito por proyecto.** Donde una feature hoy opera sobre "todos los peers/tareas/alertas", bajo la
   empresa opera sobre "el proyecto activo" (con toggle "todos"). Es un filtro por el sufijo `@proyecto`
   del id ([[../02-modelo-dominio]] §7), no un rediseño.
2. **Identidad legible `rol@proyecto`.** Donde una feature muestra un id crudo ("instancia-2"), bajo la
   empresa muestra "backend@proyecto-x" + su cargo. Mejora directa de legibilidad, misma UI.

El resto es contextualización: la feature X "significa" tal cosa en clave de empresa. Abajo, pestaña por
pestaña.

---

## 1. Las 9 pestañas (backlog de 164 features) bajo la empresa

### Peers → "Empleados / plantilla del equipo"
La pestaña Peers ES la lista de empleados. Bajo la empresa:
- **peers-01 (detalle del peer)** se enriquece con **cargo, proyecto y cadena de mando** (a quién reporta,
  a quién delega). El "detalle del peer" pasa a ser la **ficha del empleado**.
- **peers-02/03/04 (enviar/kick/resumen)** = tocar el hombro / pausar empleado / anotar en qué anda. Sin
  cambio de mecanismo; ganan el **chat privado** como canal alternativo (RFC Lanzador §7).
- **peers-05 (jornada)** = las horas fichadas del empleado (ya es literal).
- **peers-15 (factor de estimación)** = "qué tan bien estima este empleado" — métrica de desempeño.
- **peers-16/17 (auditoría/log del operador)** = el parte de trabajo → converge con registro-acciones.
- **Cambio corporativo:** filtrar por proyecto activo; mostrar cargo y estado del organigrama.
- **Reusa:** todos los endpoints ya listados en la RFC Peers. **Sin backend nuevo** salvo el binding de id.

### Tareas → "Órdenes de trabajo / delegación"
- **tareas-01/02 (abrir tarea + reportes)** = ver una orden de trabajo y su avance. El bloqueo #1 de Max
  sigue siendo el #1: es la unidad operativa de la empresa.
- **tareas-03/04 (asignar/reasignar)** = **delegar** ([[RFC-delegacion-cadena-mando]]). Bajo la empresa, el
  selector de destino se **restringe por `puede_delegar_a`** del cargo (R1 de Delegación). Misma UI, destino
  filtrado.
- **tareas-13 (timeline de eventos, endpoint nuevo)** = historia de la orden (asignada→reasignada→forzada→
  hecha). Es la traza de delegación → converge con la bitácora.
- **Cambio corporativo:** ámbito por proyecto (el tablero del proyecto, RFC Proyectos R8); delegación
  restringida por cadena de mando.
- **Reusa:** endpoints de tarea existentes. El único endpoint nuevo (tareas-13) es de trazabilidad, no de la
  empresa.

### Alertas → "Supervisión / escalado"
- Toda la pestaña Alertas es el **panel de mandos intermedios**. Bajo la empresa:
- **alertas-07/10 (actuar sobre el sujeto / ir al sujeto)** = escalado ([[RFC-delegacion-cadena-mando]] R6/R7):
  la alerta se enruta al superior del agente afectado; las acciones (forzar/reasignar) van con `de` exento.
- **Cambio corporativo:** cada alerta gana "escalada a <coordinador>/Max"; filtrable por proyecto.
- **Reusa:** supervisor + `/admin/alertas` + `/admin/alerta-resolver` (existentes). Sin backend nuevo.

### Jornada → "Fichaje / partes de trabajo"
- Ya es literalmente el fichaje. Bajo la empresa:
- **jornada-01/02/10** (abrir tarea/sesión/timeline) = ver el parte de trabajo de un empleado.
- La **tercera sección "Acciones"** (registro-acciones) es el diario del empleado. Converge directo con la
  empresa: es "qué hizo hoy este funcionario".
- **Cambio corporativo:** agregable por **proyecto** (jornada del equipo del proyecto) y por **cargo**.
- **Reusa:** `/jornada`, `/listar-tareas`, `/acciones` (registro-acciones). Sin backend nuevo.

### Trazabilidad → "Correo interno / auditoría de comunicación"
- El ciclo de vida del mensaje (enviado→entregado→leído→procesado) es la **auditoría de la oficina de
  correos**. El ghosteo (leído-no-procesado) es un empleado que ignoró un mensaje → alerta del supervisor.
- **Cambio corporativo:** filtrable por proyecto; el "reenviar" (trazabilidad-05) es re-tocar el hombro.
- **Reusa:** `/admin/historial`, `/admin/reenviar` (existentes). Sin backend nuevo.

### Redis → "Salud de la infraestructura de correos"
- Colas/outbox/purga = mantenimiento de la oficina de correos (mensajes atascados de un empleado).
- **Cambio corporativo:** ver colas **por proyecto** (agrupar peers por `@proyecto`).
- **Reusa:** `/admin/redis`, `/admin/purgar` (existentes). Endpoints nuevos (bandeja/outbox exactos,
  redis-01/02) son de observabilidad, no de la empresa.

### Broker → "RRHH central / servidor de la empresa"
- El broker ES RRHH + nómina (reloj) + correos + verdad del estado. La pestaña Broker es su **panel de
  administración**.
- **broker-02/04 (métricas/umbrales, endpoints nuevos)** = salud del "servidor central de la empresa".
- **Cambio corporativo:** el broker es único (una empresa, un RRHH). Multi-broker = multi-empresa (fuera de
  alcance; RFC Acceso ya piensa multi-broker para conmutar, no para federar).
- **Reusa:** `/admin/info`, `/salud` (existentes) + los nuevos de métricas (ya planificados en la RFC Broker).

### Acceso → "Dónde vive la empresa (conexión + hosts)"
- URL/token/hosts SSH = la dirección de RRHH y de las oficinas (hosts) donde trabajan los empleados.
- **Cambio corporativo:** la **lista de hosts SSH** de Acceso es la fuente de `Ubicacion::Ssh` de los
  proyectos ([[RFC-proyectos]] R3). Se **unifican**: un host configurado en Acceso es una ubicación
  disponible para proyectos/agentes.
- **Reusa:** `/salud` (probar conexión). Sin backend nuevo.

### Config → "Políticas y ajustes de la empresa"
- Parámetros, tema, defaults. Bajo la empresa gana: **plantillas de cargo, proyectos, perfiles de
  lanzamiento** (todo config del operador en el mismo `config.toml`).
- **Cambio corporativo:** Config es el hogar de la config corporativa (cargos/proyectos/perfiles), además
  de lo actual.
- **Reusa:** el `config.toml` y su patrón (compartido con la TUI). Sin backend nuevo.

---

## 2. Las RFCs grandes bajo la empresa

### Lanzador (existente) → "Contratar puesto de trabajo + escribir al empleado"
Es la pieza **más central** para la empresa: es donde un agente **nace** (se lanza con su system prompt) y
donde Max **le escribe** (chat privado). Bajo el modelo corporativo:
- Sus **plantillas de system prompt (R2.1)** SON los **cargos** ([[RFC-organigrama-roles]] R2) — unificar.
- Su **binding id↔sesión (§6.2, riesgo abierto)** se resuelve con el id `rol@proyecto`
  ([[../02-modelo-dominio]] §5) — la empresa **le da la solución** que el Lanzador dejó abierta.
- Sus **perfiles de lanzamiento (R9)** son el "desplegar el equipo de un proyecto" ([[RFC-proyectos]] R9).
- El **chat privado** es "escribir al empleado sin que se vea en su TUI" — canal directo dueño↔empleado.
- **Veredicto:** el Lanzador es el motor de ejecución de la empresa; la empresa le aporta la identidad
  (rol@proyecto), el contenedor (proyecto) y el significado (cargo). Se refuerzan mutuamente.

### Política de comunicación (Fase 1 implementada) → "Firewall organizativo / silos"
- Es el eje **habla-con** del organigrama ([[../01-modelo-corporativo]] §6). Bajo la empresa:
- El **modo solo-operador** = "la empresa en modo dirigido" (solo Max inicia).
- Los **grupos (`Patron::Grupo`, previsto)** = **departamentos/proyectos** ("front-* no habla con backend-*",
  "proyecto-a/* no habla con proyecto-b/*"). Es el mecanismo de **aislamiento de proyecto** ([[RFC-proyectos]] R12).
- **Veredicto:** ya está lista (Fase 1); la empresa la **usa** como firewall entre proyectos y equipos. La
  UI pendiente (R10-R12) se dibuja en el organigrama (aristas habla-con).

### Registro de acciones (en curso) → "Parte de trabajo / diario del empleado"
- Es el **diario laboral** de cada empleado y del operador. Bajo la empresa:
- Su **identidad durable** (`peers_conocidos` en `bitacora.db`, ADR-001) es el germen de la **identidad
  organizativa durable** que la empresa formaliza ([[../01-modelo-corporativo]] §2). Alineados.
- La **atribución por `quien`** (operador vs peer, R5) es exactamente la distinción empresa: acción del
  dueño vs del empleado.
- **Veredicto:** la empresa **hereda y extiende** su modelo de identidad durable; no lo duplica.

### Supervisor (existente) → "Mandos intermedios automáticos"
- Detecta ociosos/atascados/ghosteo → alertas. Bajo la empresa: es el **capataz automático** que vigila lo
  que Max no puede 24/7 (literal, la spec ya lo dice: *"supervisar 24/7 a sus empleados IA"*). Alimenta el
  **escalado** por la cadena de mando ([[RFC-delegacion-cadena-mando]] R6).
- **Veredicto:** encaja sin cambios; la empresa le añade **enrutamiento por jerarquía** (a quién avisar).

### Tareas autogestionadas + factor de estimación (en curso) → "Fichaje autónomo + productividad"
- Los agentes fichan sus tareas solos (tools MCP); el broker mide el real y aprende el factor. Bajo la
  empresa: es la **autonomía del empleado** (VISÃO: "sair de perto") + una **métrica de productividad** por
  agente/cargo/proyecto.
- **Veredicto:** ya es el corazón del "empleado que trabaja solo". La empresa lo agrupa por proyecto/cargo.

---

## 3. Cambios transversales que la empresa introduce (resumen para planificar)

| Cambio | Alcance | Coste | Bloqueante |
|--------|---------|-------|-----------|
| Filtrar por proyecto activo (sufijo `@proyecto`) | todas las pestañas | bajo (filtro cliente) | no |
| Mostrar `cargo` + id legible `rol@proyecto` | Peers, Tareas, Alertas, organigrama | bajo (config + label) | no |
| Restringir delegación por cadena de mando | Tareas (asignar/reasignar), Jornada | bajo (filtrar `Select`) | no (v1 en UI) |
| Enrutar alertas por jerarquía (escalado) | Alertas | bajo (resolver superior) | no |
| Unificar plantillas de prompt = cargos | Lanzador, Config, Organigrama | medio (migrar config) | no |
| Unificar hosts SSH (Acceso) = ubicaciones de proyecto | Acceso, Proyectos | bajo | no |
| **Binding id explícito `CLAUDE_PEERS_ID` en `peers-client`** | backend (client) | bajo | **SÍ** (cimiento de todo) |
| (v2) Aislamiento server-side por `proyecto_id` | core + broker | medio | no (opcional) |
| (v2) Validación de capacidades/cadena en el broker | broker | medio-alto | no (opcional) |

**Único bloqueante real:** el binding de id (`peers-client` respeta `CLAUDE_PEERS_ID` explícito sin
re-derivarlo de la carpeta). Todo lo demás de la empresa es config del operador + vistas GPUI sobre
endpoints existentes. Esto confirma la tesis: **la empresa es sobre todo una capa de organización y UI, no
de infraestructura.**

---

## 4. Orden sugerido (empresa sobre el backlog existente)

Encaja con las 5 olas del [[../../desktop/INDICE-RFCS|índice]] existente, añadiendo la capa empresa como
contexto, no como bloqueo:

1. **Cimiento (antes de todo lo corporativo):** binding de id `rol@proyecto` (backend pequeño) + resolver
   la identidad del operador (ya casi hecho). Desbloquea proyectos, organigrama, delegación, chat privado.
2. **Ola 1-2 del índice, con etiqueta de proyecto:** abrir tarea, CRUD, kick, enviar, purgar… se
   implementan como hoy, pero mostrando cargo + filtrables por proyecto. La empresa "viene gratis" como
   contexto.
3. **Capa empresa propiamente (nuevas RFCs):** [[RFC-proyectos]] → [[RFC-organigrama-roles]] →
   [[RFC-delegacion-cadena-mando]]. Cada una reusa lo de las olas anteriores.
4. **Lanzador** (contratar+lanzar+chat privado) — pieza pesada (PTY); puede ir en fases (sin PTY / con PTY).
5. **v2 (endurecimiento opcional):** aislamiento server-side, capacidades duras, validación de cadena en el
   broker — solo si Max los pide tras operar con v1.

---

## 5. Constraints (heredadas, aplican a toda la relectura)

- Ninguna feature documentada se descarta ni se reescribe: se **contextualiza**. La empresa es **opt-in**
  (sin proyectos/cargos definidos, la app opera como hoy).
- Cero infra nueva salvo la ya aceptada (`sqlx`/`bitacora.db`). El binding de id es el único backend
  imprescindible. Español salvo protocolo. Red en `background_executor` + `bloquear_en`. Sin
  `.unwrap()`/`.expect()` en prod. `#[serde(default)]` para compat. NUNCA `Co-Authored-By`.

---
#rfc #empresa #relectura #backlog #integracion #peers-desktop
