# Índice maestro — RFCs de la app desktop (GPUI · tema Ethos)

> **Contexto.** La app `crates/peers-desktop` tiene el tema Ethos aplicado y carga datos, pero
> es casi **solo lectura**: no se pueden abrir tareas, no hay pop-ups de detalle, casi no hay CRUD
> ni controles, y falta accesibilidad. La TUI (`crates/peers-tui`) ya tiene el CRUD y la trazabilidad
> que la desktop no portó, y el broker (`crates/peers-broker`) expone endpoints que la desktop no usa.
>
> Este índice consolida las **9 RFCs** (una por pestaña) que proponen recuperar ese control.
> Cada RFC vive en `.specs/desktop/<pestaña>/RFC-<pestaña>.md` con features numeradas
> (`<pestaña>-NN`), cada una con problema, propuesta, 2-3 variantes de diseño Ethos, endpoint del
> broker, tipo de trazabilidad y prioridad.

---

## 1. Tabla de RFCs

| Pestaña | Nº features | Ruta del RFC | Reparto de prioridad |
|---------|:-----------:|--------------|----------------------|
| Peers | 18 | `.specs/desktop/peers/RFC-peers.md` | 6 alta · 7 media · 5 baja |
| Tareas | 18 | `.specs/desktop/tareas/RFC-tareas.md` | 4 alta · 8 media · 6 baja |
| Alertas | 18 | `.specs/desktop/alertas/RFC-alertas.md` | 7 alta · 8 media · 3 baja |
| Trazabilidad | 18 | `.specs/desktop/trazabilidad/RFC-trazabilidad.md` | 5 alta · 8 media · 5 baja |
| Jornada | 17 | `.specs/desktop/jornada/RFC-jornada.md` | 5 alta · 10 media · 2 baja |
| Redis | 20 | `.specs/desktop/redis/RFC-redis.md` | 5 alta · 9 media · 6 baja |
| Broker | 20 | `.specs/desktop/broker/RFC-broker.md` | 4 alta · 11 media · 5 baja |
| Acceso | 17 | `.specs/desktop/acceso/RFC-acceso.md` | 5 alta · 9 media · 3 baja |
| Config | 18 | `.specs/desktop/config/RFC-config.md` | 3 alta · 10 media · 5 baja |
| **Total** | **164** | 9 RFCs | **44 alta · 74 media · 40 baja** |

### RFCs nuevas (features grandes, fuera del recuento de 164 — 2026-07-02)

| Área | Ruta del RFC | Naturaleza |
|------|--------------|------------|
| Lanzador | `.specs/desktop/lanzador/RFC-lanzador.md` | pestaña nueva: elegir directorio (file picker nativo GPUI) + system prompt + tareas + destino local/SSH/tmux + **terminal PTY embebido** (reusa crates `terminal`/`terminal_view` de Zed) + **chat privado** (tools MCP pull, no `<channel>`) |
| Política de comunicación | `.specs/desktop/politica-comunicacion/RFC-politica-comunicacion.md` | firewall peer↔peer evaluado en el broker (`enviar()` main.rs:352); **ámbito mixto** (control en desktop, motor en broker); RabbitMQ descartado |

> Estas dos NO están desglosadas en features `<pestaña>-NN` como las 9 originales: son propuestas
> arquitectónicas grandes con requisitos R1..Rn y criterios de aceptación propios. Al aprobarlas,
> pueden partirse en features numeradas o en fases (p.ej. Lanzador Fase 1 sin PTY / Fase 2 con PTY).

---

## 2. MVP de control — features de prioridad ALTA (todas las pestañas)

Este es el conjunto que responde directo a la queja de Max *("no consigo abrir ninguna tarea, no
hay pop-ups de visualización, no hay casi funciones CRUD, no tengo ningún control")*. Son 44 features.

### Abrir / visualizar (pop-ups de detalle)
- **tareas-01** — Pop-up de detalle de tarea (`Dialog` modal: descripción íntegra, estado, estimado/real, dueño, motivo de bloqueo, reportes). *El "abrir tarea" que hoy no existe.*
- **tareas-02** — Reportes de progreso en el detalle (`GET /tarea/reportes`).
- **jornada-01** — Abrir tarea desde una fila de jornada (modal de detalle).
- **jornada-02** — Detalle de sesión (modal con tareas correlacionadas por `sesion_id`).
- **jornada-10** — Timeline de reportes de la tarea (`GET /tarea/reportes`).
- **peers-01** — Pop-up de detalle del peer.
- **trazabilidad-01** — Modal de detalle del mensaje (ciclo de vida).
- **trazabilidad-04** — Ver el texto completo del mensaje (hoy recortado a 80 chars).
- **alertas-01** — Modal de detalle de alerta con `Esc`.
- **alertas-10** — Ir al sujeto de la alerta (salto a la pestaña correspondiente).

### CRUD básico de tareas (lo que la TUI hace y la desktop no)
- **tareas-03** — Asignar tarea con `Dialog` (hoy despacha payload vacío).
- **tareas-04** — Reasignar con selector de peer (hoy `nuevo_instancia_id` vacío).
- **jornada-03** — Selector de peer inline para asignar/reasignar.
- **jornada-04** — Transiciones de estado hecha / cancelar / reabrir.

### Mensajería y colas (acciones del broker sin cablear)
- **peers-02** — Enviar mensaje a un peer (`POST /enviar`).
- **peers-03** — Kick / expulsar peer (`POST /salir`) — paridad TUI tecla `k`.
- **trazabilidad-02** — Filtrar mensajes por estado.
- **trazabilidad-03** — Buscar mensajes.
- **trazabilidad-05** — Reenviar mensaje en la fila (`POST /admin/reenviar`).
- **redis-01** — Ver bandeja pendiente de un peer.
- **redis-02** — Ver outbox de un peer (`outbox_pendientes` interno, exponer).
- **redis-03** — Reenviar mensaje de la cola (`POST /admin/reenviar`).
- **redis-04** — Purgar cola de un peer (`POST /admin/purgar`) — paridad TUI tecla `p`.
- **broker-06** — Purgar la cola/outbox de un peer atascado (`POST /admin/purgar`).

### Alertas (paridad TUI + operación)
- **alertas-02** — Descartar / resolver alerta (`POST /admin/alerta-resolver`).
- **alertas-03** — Navegación por teclado ↑↓ / Enter / d / g.
- **alertas-07** — Actuar sobre la tarea/peer de la alerta (forzar, reasignar…).
- **alertas-09** — Refrescar alertas.
- **alertas-14** — Filtro por gravedad/dominio.

### Estado del broker y conexión (recuperar visibilidad y confianza)
- **peers-05** — Ver jornada del peer (`/jornada`).
- **peers-10** — Refrescar peers.
- **broker-02** — Ver estado real de Redis / backend (`GET /admin/metricas` *NUEVO*).
- **broker-03** — Estado de conexión + reintento.
- **broker-04** — Ver umbrales de liveness (`GET /admin/umbrales` *NUEVO*).
- **acceso-01** — Editar URL del broker en la propia pestaña Acceso.
- **acceso-02** — Editar token en la propia pestaña Acceso.
- **acceso-05** — Probar conexión (`/salud`).
- **acceso-06** — Diagnóstico del error de conexión.
- **acceso-09** — Aviso de broker expuesto sin token (el broker ya emite el `warn!`).
- **config-01** — Probar conexión desde Config (`/salud`).
- **config-02** — Restablecer valores por defecto.
- **config-06** — Panel "Estado del broker" (`GET /admin/info`).

### Accesibilidad (queja transversal "no tengo control")
- **peers-18** — Navegación por teclado + anillo de foco + salud.

### Top-10 del MVP (las que más control desbloquean por esfuerzo)
1. **tareas-01** — Abrir tarea (pop-up de detalle). *El bloqueo #1 declarado por Max.*
2. **tareas-03** — Asignar tarea (deja de despachar payload vacío).
3. **tareas-04** — Reasignar con selector de peer.
4. **peers-03** — Kick peer (paridad TUI, endpoint listo).
5. **peers-02** — Enviar mensaje a un peer.
6. **trazabilidad-05** — Reenviar mensaje (endpoint listo).
7. **redis-04** — Purgar cola de peer atascado (paridad TUI, endpoint listo).
8. **alertas-02** — Descartar/resolver alerta.
9. **acceso-05** — Probar conexión (`/salud`).
10. **jornada-01** — Abrir tarea desde jornada (modal de detalle).

---

## 3. Orden sugerido de implementación

El criterio es: **que Max recupere control cuanto antes**, priorizando lo que ya tiene endpoint
en el broker (riesgo backend ≈ 0) y lo que desbloquea la queja literal.

### Ola 1 — "Poder ver y abrir" (solo lectura enriquecida, todo con endpoints existentes)
- **Primer componente reutilizable: un `Dialog` de detalle Ethos.** Lo consumen tareas-01/02,
  jornada-01/02/10, peers-01, trazabilidad-01/04, alertas-01. Construirlo una vez desbloquea
  10 features.
- RFCs a atacar: **Tareas** (01, 02) → **Jornada** (01, 02, 10) → **Trazabilidad** (01, 04) →
  **Peers** (01) → **Alertas** (01, 10).

### Ola 2 — "Poder actuar" (CRUD sobre lo ya montado en `rutas_protegidas`)
- **Tareas** 03, 04 (asignar/reasignar con formulario real; hoy van con payload vacío).
- **Jornada** 03, 04 (selector de peer + transiciones de estado).
- **Peers** 02, 03 (enviar mensaje, kick — paridad TUI).
- **Trazabilidad** 02, 03, 05 (filtrar, buscar, reenviar).
- **Redis** 03, 04 (reenviar de cola, purgar — paridad TUI).
- **Alertas** 02, 07, 09, 14 (resolver, actuar sobre sujeto, refrescar, filtrar).
- **Broker** 06 (purgar peer atascado).

### Ola 3 — "Conexión y confianza"
- **Acceso** 01, 02, 05, 06, 09 (editar URL/token en la propia pestaña, probar, diagnosticar, aviso de exposición).
- **Config** 01, 02, 06 (probar, restablecer, ver info del broker).
- **Peers** 05, 10 (jornada del peer, refrescar).

### Ola 4 — Accesibilidad transversal
- **peers-18** y equivalentes: navegación por teclado, anillo de foco brasa (`fila_seleccionable`
  ya existe), `Esc` cierra modales, orden de foco. Aplicar el patrón en todas las pestañas.

### Ola 5 — Trazabilidad y observabilidad (media/baja, algunas requieren broker)
- Timelines, historiales, métricas, factores por peer, auditoría de acciones admin.
  Ver §4 para las que dependen de endpoints nuevos.

---

## 4. Dependencias — endpoints existentes vs. nuevos

### Regla general
**El grueso del MVP usa endpoints que YA existen** en `crates/peers-broker/src/main.rs`
(montados en `rutas_protegidas`, auth por header `X-Peers-Token`; `/salud` es la única exenta).
La desktop simplemente no los cablea. Riesgo backend ≈ 0 para casi todo.

### Endpoints existentes reutilizables (no tocar el broker)
`GET /salud` · `GET /admin/info` · `GET /listar` · `POST /salir` (kick) · `POST /definir-resumen` ·
`POST /enviar` · `GET /jornada` · `POST /listar-tareas` · `POST /tarea/asignar` ·
`POST /tarea/reasignar` · `POST /tarea/editar` · `POST /tarea/estado` · `POST /tarea/forzar` ·
`GET /tarea/reportes` · `POST /tarea/reportar` · `GET /factor-estimacion` ·
`GET /factor-estimacion-peer` · `GET /admin/redis` · `POST /admin/purgar` ·
`GET /admin/historial` (soporta `?id=&desde=&estado=`) · `POST /admin/reenviar` ·
`GET /admin/alertas` · `POST /admin/alerta-resolver` · `POST /confirmar`.

> Notas de infrautilización detectadas en las RFCs:
> - El cliente desktop llama `historial(&id)` **sin** `desde`/`estado`, aunque el broker los soporta (trazabilidad-02/15, redis-11/12).
> - El cliente desktop **no expone** `enviar` ni `confirmar` pese a existir en el broker (trazabilidad-06/07/08).
> - `outbox_pendientes(id)` existe **internamente** en el broker pero no se expone por HTTP (redis-02).

### Features que SÍ requieren endpoints nuevos del broker (decisión aparte)

| Endpoint nuevo | Tipo | Features que lo piden | RFC |
|----------------|------|-----------------------|-----|
| `GET /admin/metricas` (uptime, hora, backend, redis-ok, latencia) | lectura · bajo riesgo | broker-01, 02, 19 | broker |
| `GET /admin/umbrales` (ocioso/atasco/ghosteo/vencimiento) | lectura | broker-04 | broker |
| `POST /admin/umbrales` (editar liveness en caliente) | **mutable** · seguridad | broker-05 | broker |
| `POST /admin/reiniciar-supervisor` | **mutable** · seguridad | broker-13 | broker |
| `GET /admin/auditoria` (persistir los `info!("admin: …")` en LIST `cprs:auditoria`) | lectura | broker-14 | broker |
| `GET /tarea/eventos?tarea_id=` (timeline de estados/reasignaciones/forzados) | lectura | tareas-13 | tareas |
| `GET /admin/bandeja?id=` (contenido crudo de bandeja, no borrado) | lectura | redis-01 (exacto) | redis |
| `GET /admin/outbox?id=` (exponer `outbox_pendientes`) | lectura | redis-02 | redis |
| `POST /admin/purgar { id, alcance }` (purga selectiva bandeja/outbox) | mutable (extiende existente) | redis-13 | redis |
| `POST /admin/purgar-lote { ids }` | mutable | redis-14 | redis |
| `/admin/alertas-historial` (persistir emisión/resolución de alertas) | lectura | alertas-15 (opcional), alertas-16 | alertas |
| allowlist de IPs / rotar token en caliente / test de conexión dedicado | mutable · seguridad | acceso-08, acceso-11 (14/17 opc.) | acceso |

### Recomendaciones de secuenciación backend
1. **Implementar primero `GET /admin/metricas`** (solo lectura, desbloquea 3 features de broker de un tiro).
2. Los mutables nuevos (`POST /admin/umbrales`, `/admin/reiniciar-supervisor`, purga selectiva/lote,
   rotar token, allowlist) **amplían la superficie de escritura** del broker → decidir con criterio de
   seguridad. Hoy el único mutable admin es `POST /admin/purgar`. Todos irían en `rutas_protegidas`.
3. La **trazabilidad persistente de alertas** es el único bloqueo real de la pestaña Alertas: el modelo
   `Alerta` solo guarda `creada_en` y `/admin/alertas` devuelve solo las vigentes. Hasta que exista
   `alertas-historial`, la trazabilidad de esa pestaña se resuelve con **bitácora local** en la desktop
   (marcado así en la RFC para no prometer lo que el backend no da).

---

## 5. Verificación

- **9/9 RFCs presentes** en `.specs/desktop/<pestaña>/RFC-<pestaña>.md`.
- **164 features** documentadas en total (18+18+18+18+17+20+20+17+18).
- Reparto global: **44 alta · 74 media · 40 baja** (dominancia media = trabajo incremental sobre
  endpoints existentes; el MVP alta es lo que devuelve el control a Max de inmediato).
