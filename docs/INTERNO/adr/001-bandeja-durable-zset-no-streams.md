# ADR-001: Bandeja de mensajes durable con ZSET + estados (no Redis Streams)

- **Date**: 2026-06-29
- **Status**: Accepted
- **Deciders**: Max (LexusFX), consejo de 4 agentes, peer `aistudio` (diagnóstico)
- **Tags**: arquitectura, mensajería, redis, durabilidad

## Context and Problem Statement

La entrega de mensajes entre peers usaba una LIST de Redis con drenado **destructivo**:
`recibir_mensajes` hacía `LRANGE` + `DEL` (store.rs:236-244), borrando el mensaje al leerlo.
El push al canal es fire-and-forget; si se emitía sin un turno activo del receptor que
renderizara el `<channel>`, el mensaje ya estaba borrado → se perdía sin rastro. Esto
imposibilita la trazabilidad ("quién recibió/leyó/procesó qué") que la visión de "empleados
IA" exige. Había que rediseñar la entrega para que nada se pierda y todo sea auditable.

## Decision Drivers

- Resolver la pérdida de mensajes (must-have).
- Habilitar trazabilidad: historial, estados, reenvío.
- Reusar el patrón existente (JSON-en-Redis homogéneo) sin reescribir el store.
- Escala real: decenas de peers en LAN, no millones de eventos.

## Considered Options

- **A. Bandeja durable ZSET + estados en HASH** (parchear, no migrar).
- **B. Migrar a Redis Streams** (XADD/XREADGROUP/XACK, consumer groups, PEL).
- **C. No hacer nada** (status quo destructivo).

## Decision Outcome

Chosen option: **"A. Bandeja durable ZSET + estados en HASH"**, porque el bug es de
**persistencia** (se borra al leer), no de transporte ni de falta de consumer-groups. El
`msgseq` (`cprs:msgseq`) ya da orden total; un ZSET con score=msgseq + estado en HASH cubre el
100% de los requisitos (peek no-destructivo, máquina de estados timbrada por el broker,
historial, reenvío) reusando el patrón JSON-en-Redis del resto del sistema. Migrar a Streams
(opción B) ofrecía PEL/XACK nativos pero a costa de reescribir TODO el store — sobre-ingeniería
para la escala real. Se adoptó UN punto de la lente Streams sin migrar: eliminar el `KEYS
cprs:outbox:*` O(n), indexando por `para_id`.

### Positive Consequences

- Ningún mensaje se pierde: sobrevive a un push sin turno activo (peek no-destructivo).
- Trazabilidad completa: estados `Enviado→Entregado→Leído→Procesado` timbrados por el reloj
  del broker, historial durable, reenvío.
- Reusa el patrón existente; gran parte ya estaba cableada (`ItemOutbox` + id estable).

### Negative Consequences

- Exige idempotencia de entrega (sin ella, el peek re-empuja cada 1s = ruido). Resuelto con
  un set de ids vistos en el cliente + `HSETNX` en el broker.
- Las transiciones son 2 viajes a Redis (lost-update teórico bajo concurrencia) → si algún día
  hay multi-lector concurrente, se añade atomicidad con script Lua (diferido, YAGNI hoy).

## Pros and Cons of the Options

### A. Bandeja ZSET + estados ✅ Chosen
- ✅ Resuelve pérdida + trazabilidad al 100%.
- ✅ Reusa patrón JSON-en-Redis; bajo riesgo de regresión.
- ❌ Requiere construir idempotencia y máquina de estados a mano.

### B. Redis Streams
- ✅ ACK/PEL/replay nativos.
- ❌ Reescribe todo el store; rompe el patrón homogéneo del resto.
- ❌ `stream_id` redundante con `msgseq`; PEL no compra nada a escala LAN.

### C. No hacer nada
- ✅ Cero esfuerzo.
- ❌ Falla el must-have: los mensajes se siguen perdiendo. Inaceptable.

## Links

- Driven by: [RFC-001](../rfc/RFC-001-entrega-durable-trazabilidad.md)
- Spec: `.specs/features/entrega-durable-trazabilidad/`
- Diagnóstico original del bug: peer `aistudio` (sesión 2026-06-29).
