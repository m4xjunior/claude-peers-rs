# Spec — Supervisor (detector de ociosos / atascados / ghosteo)

> Fecha: 2026-06-29 (diseñado en loop autónomo; defaults sensatos, Max valida a las 15:59).
> Fase 5 del consejo-roadmap. Encaja sobre la jornada, las tareas y la trazabilidad ya existentes.

## El problema

Max no puede supervisar 24/7 a sus "empleados IA". Necesita que el sistema AVISE solo cuando:
un agente está ocioso, una tarea lleva mucho sin avanzar, o un mensaje fue leído pero nunca
procesado (ghosteo — el bug original de aistudio). Trazabilidad PASIVA (puedo mirar) → ACTIVA
(el sistema me avisa).

## La solución

Una tarea periódica en el broker (junto a la limpieza de 30s ya existente) que detecta 3
condiciones y emite ALERTAS a una cola `cprs:alertas`, que la TUI pinta como banner.

## Requisitos

- **R1** Tarea periódica (cada 30s, reusa el spawn de limpieza) que evalúa los 3 detectores.
- **R2 Ocioso:** peer VIVO (visto < VENCIMIENTO) sin tarea en curso desde hace > `UMBRAL_OCIOSO_SEG` (default 600s = 10min).
- **R3 Atascado:** tarea abierta (sin `fin`) desde hace > `UMBRAL_ATASCO_SEG` (default 1800s = 30min) sin reporte.
- **R4 Ghosteo:** mensaje en estado `Leido` (no `Procesado`) desde hace > `UMBRAL_GHOSTEO_SEG` (default 300s = 5min).
- **R5** Las alertas van a `cprs:alertas` (LIST acotada, últimas 50). Cada alerta: `{tipo, sujeto, detalle, creada_en}`.
- **R6** Endpoint `GET /admin/alertas` → `Vec<Alerta>` (bajo token).
- **R7** Idempotencia: no re-alertar lo mismo cada 30s. Una alerta por (tipo+sujeto) hasta que la condición se resuelva (set de alertas activas `cprs:alertas_activas`).
- **R8** Umbrales configurables vía env/flags del broker (con los defaults de arriba).

## Criterios de aceptación

- **AC1** (R2): un peer vivo sin tarea > umbral genera 1 alerta `ocioso`; no se duplica en el siguiente ciclo.
- **AC2** (R4 ghosteo): un mensaje en `Leido` > umbral genera alerta `ghosteo`; al pasar a `Procesado`, deja de alertar.
- **AC3** (R6): `GET /admin/alertas` con token → 200 + lista; sin token → 401.
- **AC4** (R7): re-ejecutar el detector no duplica alertas activas.
- **AC5** (compat): el detector degrada — si falla leyendo una cola, loguea y sigue con las demás; no tumba el broker.

## Constraints

Sin `.unwrap()`/`.expect()` en prod. Español salvo protocolo. Redis + SQLite. El tiempo lo
mide el broker. NUNCA Co-Authored-By.

## Fuera de alcance

La pantalla TUI "Supervisor" completa (va en la TUI completa, fase final). Aquí solo el backend
+ endpoint; la TUI lo consumirá después. Notificaciones externas (email/telegram) = YAGNI.
