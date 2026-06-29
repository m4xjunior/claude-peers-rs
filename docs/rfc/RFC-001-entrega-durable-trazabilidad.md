# RFC-001 — Entrega durable de mensajes con trazabilidad ("empleados IA")

## Header & Metadata

| Campo | Valor |
|-------|-------|
| **Título** | Entrega durable de mensajes con estados, historial y reenvío en claude-peers-rs |
| **Driver** | Max (LexusFX) |
| **Aprobador** | Max |
| **Contribuyentes** | Consejo de 4 agentes (arquitectura, producto, mensajería, robustez) + peer `aistudio` (diagnóstico del bug) |
| **Impacto** | **ALTO** — toca el corazón del sistema (entrega de mensajes), del que depende toda la red de peers |
| **Fecha** | 2026-06-29 |
| **Estado** | PROPUESTO (pendiente de decisión) |

## Background

**Estado actual:** claude-peers-rs entrega mensajes entre peers con una LIST de Redis: `encolar_mensaje` hace `RPUSH cprs:mensajes:{id}`, y `recibir_mensajes` (store.rs:236-244) hace **`LRANGE` + `DEL`** — drena y **borra** la cola al leer. El cliente (`lanzar_recepcion`, cada 1s) emite el push `notifications/claude/channel` a stdout de forma **fire-and-forget** (`enviar_json` ignora el `Result` del flush, mcp.rs:54-58).

**El problema (confirmado en código, diagnosticado por el peer `aistudio`):** si el push se emite cuando la sesión del Claude receptor no tiene un turno activo que renderice el `<channel>`, **el mensaje ya fue borrado de Redis** → se pierde sin rastro. `revisar_mensajes` manual encuentra la cola vacía. No hay forma de auditar, recuperar ni reenviar.

**Por qué ahora:** la visión de Max es que estas IAs sean sus **funcionarios** — un equipo con trazabilidad y accountability, como una empresa real. Un sistema donde el trabajo se pierde en silencio es incompatible con esa visión. Max pidió explícitamente: historial de mensajes por cola, poder **reenviar** un mensaje que el Claude ignoró, y ver **quién recibió/leyó/procesó qué**.

**Costo de no actuar:** mensajes perdidos sin rastro → Max no puede confiar en delegar trabajo a sus "empleados IA"; el sistema sirve para chatear pero no para coordinar trabajo real.

## Assumptions

1. **La escala real es decenas de peers en LAN/túnel, no millones de eventos.** Confianza: ALTA. Invalidada si la red crece a miles de agentes concurrentes (reabriría la opción Streams).
2. **Hay un solo lector por cola de destinatario** (el `peers-client` de ese peer). Confianza: ALTA. Invalidada si la TUI y el cliente leen la misma bandeja a ritmos distintos concurrentemente (reabriría atomicidad Lua, Fase 7).
3. **El `id` estable por carpeta (ya implementado) identifica de forma fiable al peer que vuelve tras reiniciar.** Confianza: ALTA (verificado esta sesión).
4. **Redis sigue siendo el backend por defecto**; SQLite queda como alternativa tras feature flag. Confianza: ALTA.

## Decision Criteria (definidos ANTES de las opciones)

| # | Criterio | Peso | Tipo |
|---|----------|------|------|
| C1 | **Resuelve la pérdida de mensajes** (el bug raíz) | 35% | **Must-have** |
| C2 | **Habilita trazabilidad** (historial + estados + reenvío) que Max pidió | 25% | **Must-have** |
| C3 | **Reusa el código/patrón existente** (JSON-en-Redis homogéneo, no reescribe el store) | 20% | Alto |
| C4 | **Esfuerzo de implementación** acotado | 12% | Medio |
| C5 | **Robustez bajo fallos** (receptor caído, doble entrega) | 8% | Medio |

## Options Considered

### Opción A — Bandeja durable ZSET + estados en HASH (parchear, NO migrar) ⭐ RECOMENDADA

Reescribir la entrega sin migrar de motor: la cola pasa de LIST destructiva a un **ZSET `cprs:bandeja:{id}`** (score = `msgseq`, orden total que ya existe). `recibir_mensajes` hace **peek** (`ZRANGEBYSCORE` de los `Enviado`, los marca `Entregado` SIN borrar). El estado vive en un HASH `cprs:msg:{id}` con una **máquina de estados** `Enviado→Entregado→Leído→Procesado` (+ `Fallido`/`DeadLetter`), cada transición **timbrada por el reloj del broker** (mismo patrón que la jornada). Nuevo endpoint `POST /confirmar`. **Idempotencia** por `msg_id` (evita re-empujar en bucle). Historial durable `cprs:historial:{id}`. Reenvío `POST /admin/reenviar`. El `ItemOutbox{confirmado}` ya existente se subsume aquí.

**Pros:** resuelve C1 y C2 al 100%; reusa el patrón JSON-en-Redis (C3); gran parte ya está cableada a medias (el modelo tiene `ItemOutbox` + `id` estable); esfuerzo medio.
**Cons:** requiere idempotencia (sin ella, el peek no-destructivo re-empuja cada 1s); transiciones en 2 viajes Redis (lost-update teórico bajo concurrencia → Fase 7 Lua si hace falta).
**Costo:** Fase 1 (P0) media · Fase 2 (trazabilidad visible) media · Fases 3-6 incrementales.

### Opción B — Migrar a Redis Streams (consumer groups, XACK, PEL)

Reescribir la cola a Redis Streams: `XADD` para encolar, `XREADGROUP` con consumer groups, `XACK` para confirmar, PEL (pending entries list) para los no-confirmados, `XRANGE` para historial.

**Pros:** ACK, PEL y replay son **nativos** (no hay que construirlos); diseñado para entrega confiable.
**Cons:** **reescribe TODO el store** y rompe el patrón homogéneo JSON-en-LIST del resto (instancias, sesiones, tareas, jornada); el `stream_id` sería estado redundante con el `msgseq` que ya da orden total; la escala (decenas de peers LAN) NO justifica el PEL; mayor esfuerzo y riesgo de regresión en el corazón del sistema.
**Costo:** ALTO (reescritura del store + migración de datos + re-verificación de todo lo que toca la cola).

### Opción C — No hacer nada (status quo)

Dejar el `LRANGE+DEL` actual.

**Pros:** cero esfuerzo.
**Cons:** **falla C1 (must-have)** — los mensajes se siguen perdiendo. Incompatible con la visión de "empleados IA". Inaceptable.

## Evaluación contra los criterios

| Criterio (peso) | A: ZSET+estados | B: Streams | C: Status quo |
|---|---|---|---|
| C1 Resuelve pérdida (35%) | ✅ Total | ✅ Total | ❌ No |
| C2 Trazabilidad (25%) | ✅ Total (historial+estados+reenvío) | ✅ Total (nativo) | ❌ No |
| C3 Reusa código (20%) | ✅ Alto (mismo patrón) | ❌ Bajo (reescribe store) | ✅ N/A |
| C4 Esfuerzo (12%) | 🟡 Medio | 🔴 Alto | 🟢 Cero |
| C5 Robustez (8%) | 🟡 Buena (+Lua futuro) | ✅ Nativa | ❌ Mala |
| **Veredicto** | **GANA** | Sobre-ingeniería | Falla must-have |

## Recomendación

**Opción A — Bandeja durable ZSET + estados en HASH.** Gana porque cubre los dos must-have (C1, C2) reusando el patrón existente (C3) sin la reescritura masiva y el riesgo de B. El bug es de **persistencia** (se borra al leer), no de transporte ni de falta de consumer-groups: traer Streams sería resolver un problema de escala que no tenemos. La única razón de B (PEL/XACK nativos) se replica con ZSET+estado+idempotencia a una fracción del costo. Se adopta UN punto de la lente Streams sin migrar: eliminar el `KEYS cprs:outbox:*` O(n) (store.rs:299), indexando por `para_id`.

## Action Items (roadmap masticado — ver tasks.md para el detalle ejecutable)

**Fase 1 — Entrega durable (mata el bug) [P0]:** peek no-destructivo · máquina de estados timbrada · idempotencia · ACK real + `/confirmar`.
**Fase 2 — Trazabilidad visible [P0/P1]:** historial durable + `/admin/historial` · reenvío `/admin/reenviar` · pantalla TUI Trazabilidad.
**Fase 3 — Recuperación [P1]:** replay de no-procesados al re-registrar · dead-letter queue.
**Fase 4 — Gestión de equipo [P1]:** estados de tarea · pantalla TUI Equipo.
**Fase 5 — Supervisión [P2]:** detector de ociosos/ghosteo · métricas por agente.
**Fase 6 — Reasignación [P2]:** reasignar tarea de agente caído.
**Fase 7 — Concurrencia [P3, diferible]:** atomicidad Lua (solo si la concurrencia lo exige).

## Descartado (YAGNI)

Redis Streams · WebSocket/SSE · exactly-once/2PC · reintentos automáticos con backoff (esconden el problema) · chat estilo Slack · RBAC entre peers · Postgres · editor visual de flujos · notificaciones externas · prioridades/TTL por mensaje · dashboard web. (Detalle y razones en `consejo-roadmap.json`.)

## Outcome

*(Placeholder — se rellena cuando Max decida.)*
Decisión: ___ · Fecha: ___ · Notas: ___
