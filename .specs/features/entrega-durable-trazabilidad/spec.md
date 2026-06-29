# Spec — Entrega durable + trazabilidad (claude-peers-rs)

> Decisión de fondo en `docs/rfc/RFC-001-entrega-durable-trazabilidad.md` (Opción A: ZSET+estados, no Streams).
> Insumo del consejo: `consejo-roadmap.json`. Fecha: 2026-06-29.

## Objetivo

Que ningún mensaje entre peers se pierda, con trazabilidad completa (estados timbrados + historial + reenvío), para que la red de claude-peers-rs sea una plataforma de "empleados IA" con accountability.

## Alcance de esta spec

**Fase 1 (P0 — mata el bug) + Fase 2 (P0/P1 — trazabilidad visible).** Las Fases 3-7 quedan en el ROADMAP para specs posteriores. Esta spec es lo que el workflow de desarrollo ejecuta sin decidir nada.

## Requisitos (trazables)

### Fase 1 — Entrega durable

- **R1.1** La entrega NO debe borrar el mensaje al leerlo. `recibir_mensajes` hace *peek* (lee sin destruir).
- **R1.2** Cada mensaje tiene un estado de la máquina `Enviado → Entregado → Leído → Procesado` (+ `Fallido`, `DeadLetter`). Cada transición la timbra el broker con SU reloj (nunca la IA).
- **R1.3** La entrega es idempotente: el mismo `msg_id` no se re-empuja al canal en bucle, ni se re-timbra `Entregado`.
- **R1.4** El cliente solo confirma `Entregado`/`Leído` si el flush del push a stdout tuvo éxito real (no fire-and-forget ciego).
- **R1.5** El borrado real de la bandeja activa solo ocurre al confirmar `Procesado` (o por retención del historial).
- **R1.6** Eliminar el `KEYS cprs:outbox:*` O(n) (store.rs:299): indexar por `para_id`.

### Fase 2 — Trazabilidad visible

- **R2.1** Existe un historial durable por cola (`cprs:historial:{id}`) que retiene todo mensaje con su estado final aunque ya se procesara, con retención configurable.
- **R2.2** Endpoint `GET /admin/historial?id=&desde=&estado=` (bajo token) que la TUI consume.
- **R2.3** Endpoint `POST /admin/reenviar {msg_id}` que re-encola un mensaje del historial con nuevo `msgseq`, estado `Enviado`, y traza `reenviado_de`/`reenvios`.
- **R2.4** Nueva pantalla TUI "Trazabilidad" por peer: timeline de mensajes con estado coloreado + timestamps; tecla `r` reenvía, `Enter` ve el timeline completo.

## Criterios de aceptación (definición de hecho)

- **AC1** (R1.1, R1.3): enviar un mensaje a un peer cuyo cliente está vivo pero sin turno activo → tras reiniciar/atender, el mensaje sigue disponible y aparece UNA vez (no en bucle). Verificable: `recibir` repetido devuelve el mismo `Enviado`/`Entregado` sin re-empujar; tras `/confirmar Procesado`, desaparece de la bandeja activa pero queda en historial.
- **AC2** (R1.2): cada estado tiene su timestamp ISO timbrado por el broker; `enviado_en ≤ entregado_en ≤ leido_en ≤ procesado_en`.
- **AC3** (R1.4): si el flush a stdout falla (simulable cerrando stdout), el mensaje NO pasa a `Entregado` y se reintenta.
- **AC4** (R1.6): no queda ningún `KEYS` en el código de producción del store (grep limpio).
- **AC5** (R2.1, R2.2): `GET /admin/historial?id=X` devuelve los mensajes de X con estados aunque ya estén `Procesado`.
- **AC6** (R2.3): `POST /admin/reenviar {msg_id}` crea un nuevo mensaje en la bandeja del destinatario con `reenviado_de = msg_id` y el destinatario lo recibe.
- **AC7** (R2.4): la TUI muestra la pantalla Trazabilidad con los estados coloreados y la tecla `r` reenvía el seleccionado.

## Constraints (del CLAUDE.md y del proyecto)

- Sin `.unwrap()`/`.expect()` en producción; `Result`/`anyhow`. Sin `KEYS` en rutas calientes.
- Todo en español salvo las claves del protocolo del `<channel>` (`from_id`, `from_summary`, `from_cwd`, `sent_at`).
- Backend Redis por defecto; mantener compat con `--features sqlite` (impl los métodos nuevos del trait en ambos).
- NO romper el wire actual de `/enviar` (los peers viejos siguen enviando igual); los cambios son del lado de recepción/estado.
- Versionar el plugin (bump) al cambiar binarios. Recargar el LaunchAgent del broker tras recompilar.
- NUNCA `Co-Authored-By`. Jornada en el cuerpo del commit.

## Fuera de alcance (esta spec)

Fases 3-7 del roadmap (replay, DLQ, estados de tarea, pantalla Equipo, supervisor, métricas, reasignación, Lua). Van en specs posteriores cuando la Fase 1-2 esté verificada.
