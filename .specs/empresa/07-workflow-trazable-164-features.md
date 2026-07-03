# Workflow trazable — cadena y trazabilidad sobre las 164 features

> ⬆ [[_MOC|Mapa]] · Índice desktop: [[../desktop/INDICE-RFCS]] · Pipeline: [[03-pipeline-provision]] ·
> Delegación: [[rfc/RFC-delegacion-cadena-mando]] · Registro: [[../desktop/registro-acciones/RFC-registro-acciones-jornada]]
>
> Fecha: 2026-07-03. Estado: **ARQUITECTURA / PATRÓN** — no reescribe las 164 features; les añade la capa
> de **ejecución trazable**. Max: *"ataque los archivos existentes de las 164 features aplicando un
> workflow con cadena y trazabilidad."* Aquí está el patrón único que TODA feature sigue, anclado en las
> máquinas de estado REALES de `peers-core` (`lib.rs`) verificadas.

---

## 0. El problema

Las 164 features del [[../desktop/INDICE-RFCS|índice]] están descritas como *qué hacen* (abrir tarea, kick,
reenviar, purgar…), pero sin un **modelo de ejecución uniforme**: cada una parece un caso aparte. Max
quiere que operen como una **cadena trazable** — que cada acción deje rastro, respete la jerarquía, y
transite estados verificables. Esto no exige reescribir las 164 RFCs: exige un **patrón** que todas
instancian, apoyado en lo que ya existe (máquinas de estado timbradas + bitácora + política).

---

## 1. El patrón único (la "cadena" de toda feature que muta)

Toda feature de acción (las que mutan algo) sigue **siete eslabones**. Los de solo-lectura (abrir un
pop-up, filtrar) usan solo 1, 6 y 7.

```
1. GESTO UI          Max (o un agente) dispara la acción (click / tecla / tool MCP)
        │
2. PRECONDICIÓN      ¿es válida? (destino existe · transición de estado válida · permiso/cadena de mando)
        │
3. ACCIÓN (endpoint) POST/GET al broker en background_executor + bloquear_en (anti-SIGABRT)
        │
4. TRANSICIÓN ESTADO el broker mueve la máquina de estados (EstadoMensaje / EstadoTarea / agente) y TIMBRA
        │              el tiempo con SU reloj (regla sagrada); rechaza si la transición no es válida
        │
5. EVENTO BITÁCORA    AccionRegistrada { quien, accion, sujeto, cuando } — QUIÉN hizo QUÉ, timbrado
        │
6. REFRESCO UI        update + notify → re-render; sello "actualizado hace Ns"
        │
7. VERIFICACIÓN       AC observable: el estado nuevo aparece; el evento está en la bitácora; degrada sin crash
```

- **Eslabón 2 (precondición)** es donde entra la **regla dura** ([[06-decisiones]] E-03/E-12): el broker
  valida la cadena de mando y las capacidades. Una feature de delegación (asignar/reasignar) se rechaza si
  el `de` no puede delegar en el `para`.
- **Eslabón 4 (transición)** usa las máquinas de estado REALES (§2). El broker es quien timbra y quien
  rechaza transiciones inválidas (`transicion_valida`, `lib.rs:323`).
- **Eslabón 5 (bitácora)** es la **trazabilidad**: cada mutación deja un evento atribuido a `quien`
  (operador vs agente, [[../desktop/registro-acciones/RFC-registro-acciones-jornada]] R5), en `bitacora.db`
  durable.

Este patrón es la "cadena con trazabilidad" que Max pide: **cada feature = un tránsito de estado + un
evento de bitácora**, no una acción suelta.

---

## 2. Las máquinas de estado reales (el sustrato de la trazabilidad)

Verificadas en `peers-core/src/lib.rs`. Son el vocabulario que el eslabón 4 mueve y el 7 verifica.

### 2.1 — Mensaje (`EstadoMensaje`, `lib.rs:68`; `rango()` `lib.rs:90`)

```
Enviado(0) ──► Entregado(1) ──► Leido(2) ──► Procesado(3)        + terminales: Fallido(4), DeadLetter(5)
```

- Avance **monótono** por `rango()`; el broker timbra cada transición con **HSETNX** (idempotente:
  `entregado_en`/`leido_en`/`procesado_en` se sellan solo la primera vez). `Procesado` hace `ZREM` de la
  bandeja activa (`store.rs:367-372`). Peek no-destructivo en recepción.
- **Trazabilidad de comunicación:** el ciclo de vida completo de un mensaje es auditable (features de
  Trazabilidad: abrir mensaje, timeline, reenviar). El **ghosteo** (Leído no Procesado > umbral) es una
  alerta del supervisor.

### 2.2 — Tarea (`EstadoTarea`, `lib.rs:276`; `transicion_valida` `lib.rs:323`; `es_terminal` `lib.rs:308`)

```
Abierta ⇄ EnCurso ──► Hecha        Bloqueada / Hecha / Cancelada = terminal (es_terminal)
   │  │        │  └──► Cancelada    Reabrir: (Bloqueada|Hecha|Cancelada) ──► Abierta
   └──┴────────┴──► Bloqueada       (_, Bloqueada|Hecha|Cancelada) siempre válido
```

- `transicion_valida` es **pura** (`lib.rs:323`): el broker la usa para **rechazar** cambios inválidos
  (eslabón 2/4). Solo `Hecha` con estimado+real alimenta el factor de estimación (las demás no lo
  contaminan).
- **Trazabilidad de trabajo:** cada tarea es una orden de trabajo con historia (creada→asignada→
  reasignada→forzada→hecha). Features de Tareas/Jornada.

### 2.3 — Agente (ciclo de vida, [[03-pipeline-provision]] §2)

```
Definido ► Contratado ► Provisionado ► Lanzado ► Registrado ► Vivo ► Ocioso ► Pausado ► Dado de baja
```

- La máquina que el **pipeline** mueve. Cada transición deja evento de bitácora (Contratar/Lanzar/
  Registrar/Kick…). Es la trazabilidad del **empleado**.

### 2.4 — Alerta (`TipoAlerta`, `lib.rs:677`) y Acción (`TipoAccion`, `lib.rs:1007`)

- `TipoAlerta`: `Ocioso · Atascado · Ghosteo · CierreSospechoso · CancelacionExcesiva`. Idempotencia por
  `(tipo, sujeto)` — no re-alerta cada 30s. Feed del supervisor.
- `TipoAccion` (`#[non_exhaustive]`): `CrearTarea · ReportarTarea · CerrarTarea · EditarTarea ·
  CambiarEstadoTarea · ReasignarTarea · ForzarTarea · DefinirResumen · EnviarMensaje · Kick · Purgar ·
  ResolverAlerta`. **El vocabulario de la bitácora.** La empresa añade variantes (CrearProyecto,
  DefinirCargo, ContratarAgente, LanzarAgente…) — compat por `#[non_exhaustive]`.

---

## 3. La atribución `quien` (operador vs agente) — la traza de responsabilidad

Cada evento de bitácora lleva `quien` (`AccionRegistrada.quien`, `lib.rs:1045`):
- Acción de **Max** desde la desktop/TUI → `quien = ID_OPERADOR` ([[06-decisiones]] E-10 anti-spoofing).
- Acción de un **agente** (fichar tarea, enviar mensaje) → `quien = rol@proyecto`.
- Acción del **broker** (forzar por supervisor) → `quien = ID_BROKER`.

Así la **jornada de cada uno** muestra lo que hizo (RFC registro-acciones): la de Max lo que Max hizo, la
de cada agente lo suyo. Es el "parte de trabajo" trazable por sujeto.

---

## 4. Cómo se instrumenta cada ola del backlog (sin reescribir las RFCs)

Las 5 olas del [[../desktop/INDICE-RFCS|índice]] se mapean al patrón §1. **No cambia lo que la feature
hace; añade los eslabones 2/4/5** (precondición dura + transición + bitácora):

| Ola | Features (ejemplos) | Eslabones que gana |
|-----|--------------------|--------------------|
| **1 — Ver/abrir** | tareas-01, jornada-01, peers-01, trazabilidad-01 (pop-ups) | 1,6,7 (solo lectura; muestran el estado + su historia) |
| **2 — Actuar (CRUD)** | tareas-03/04 (asignar/reasignar), peers-02/03 (enviar/kick), redis-04 (purgar), alertas-02 (resolver) | 1-7 completos: precondición **dura** (cadena de mando en asignar/reasignar) + transición + **bitácora** |
| **3 — Conexión/confianza** | acceso-05 (probar), broker-02 (métricas), config-06 | 1,3,6,7 (lectura + estado de infra) |
| **4 — Accesibilidad** | peers-18 (teclado/foco/salud) | transversal al eslabón 1 (gesto por teclado) y 6 |
| **5 — Trazabilidad** | timelines, historiales, auditoría admin | eslabón 5 (la bitácora ES esta ola) + 6/7 |

**Lectura clave:** la Ola 5 (trazabilidad) **deja de ser una fase separada** — con el patrón §1, TODA
feature de la Ola 2 ya emite su evento de bitácora en el eslabón 5. La trazabilidad no se "añade después":
es un eslabón del workflow de cada acción. Eso es lo que convierte las 164 features sueltas en una empresa
auditable.

---

## 5. Tabla feature → (endpoint · transición · evento bitácora · quién) — muestra representativa

No se enumeran las 164 (el patrón es uniforme); una muestra que cubre cada tipo. Endpoints verificados en
`peers-broker/main.rs`; `TipoAccion` en `lib.rs:1007`.

| Feature | Endpoint | Transición de estado | Evento bitácora (`accion`) | `quien` | Precondición dura (eslabón 2) |
|---------|----------|----------------------|----------------------------|---------|-------------------------------|
| peers-02 enviar mensaje | `POST /enviar` | Mensaje → `Enviado` | `EnviarMensaje` | operador/agente | política habla-con |
| peers-03 kick | `POST /salir` | Agente → `Pausado` | `Kick` | operador | solo operador/superior |
| tareas-03 asignar | `POST /tarea/asignar` | Tarea → `Abierta` (nueva) | `CrearTarea`/`AsignarTarea` | operador/coordinador | **cadena: puede_delegar_a** |
| tareas-04 reasignar | `POST /tarea/reasignar` | Tarea (dueño Δ) | `ReasignarTarea` | operador/coordinador | **cadena de mando** |
| jornada-04 cambiar estado | `POST /tarea/estado` | Tarea → `EnCurso/Bloqueada/Hecha…` | `CambiarEstadoTarea` | agente/operador | `transicion_valida` (`lib.rs:323`) |
| tareas forzar | `POST /tarea/forzar` | Tarea (forzada) | `ForzarTarea` | broker/operador | exento (acción de mando) |
| trazabilidad-05 reenviar | `POST /admin/reenviar` | Mensaje → re-`Enviado` | (traza de reenvío) | operador | política |
| redis-04 purgar | `POST /admin/purgar` | vacía bandeja/outbox | `Purgar` | operador | solo operador |
| alertas-02 resolver | `POST /admin/alerta-resolver` | Alerta → resuelta | `ResolverAlerta` | operador | — |
| peers-04 editar resumen | `POST /definir-resumen` | `Instancia.resumen` Δ | `DefinirResumen` | agente/operador | — |
| (empresa) contratar | `POST /admin/agente` | Agente → `Contratado` | `ContratarAgente` (nueva) | operador | cargo/proyecto existen |
| (empresa) lanzar | (pipeline) | Agente → `Lanzado→Registrado` | `LanzarAgente` (nueva) | operador | provisión OK (doc 03 §4) |

---

## 6. El escalado como cadena trazable (une supervisor + jerarquía + bitácora)

Un caso completo del patrón, extremo a extremo (la "cadena" en su forma más rica):

```
1. Supervisor detecta Atascado (tarea sin avance > umbral) → emite Alerta{Atascado, sujeto: backend@px}
2. La app resuelve el destino de escalado: reporta_a de backend@px = coordinador@px (RFC Delegación R6)
3. La alerta se muestra al coordinador (y siempre a Max) con badge "→ escalada"
4. El coordinador actúa: POST /tarea/forzar o /tarea/reasignar (eslabón 3)
   → precondición: es su subordinado (cadena de mando, eslabón 2, regla dura)
5. Transición: Tarea forzada/reasignada (eslabón 4, timbrada)
6. Bitácora: AccionRegistrada{ quien: coordinador@px, accion: ForzarTarea, sujeto: tarea_id } (eslabón 5)
7. Verificación: la tarea cambió de estado; el evento está en la jornada del coordinador; la alerta se
   resuelve (ResolverAlerta). Todo auditable.
```

Esto es "workflow con cadena y trazabilidad" literal: una alerta sube por la jerarquía, se actúa
respetando la cadena de mando, y cada paso deja rastro timbrado y atribuido.

---

## 7. Criterios de aceptación (del patrón, no de cada feature)

- **AC1 (cadena)** — toda feature de acción pasa por los 7 eslabones; las de lectura por 1/6/7.
- **AC2 (transición)** — las mutaciones de tarea respetan `transicion_valida` (`lib.rs:323`): una
  transición inválida es rechazada por el broker (no se aplica), verificable.
- **AC3 (regla dura)** — asignar/reasignar fuera de la cadena de mando devuelve `ok:false`; forzar como
  operador/broker se permite (exento) — [[06-decisiones]] E-12.
- **AC4 (trazabilidad)** — cada acción que muta deja un `AccionRegistrada` con `quien`/`accion`/`sujeto`/
  `cuando` timbrado por el broker; visible en la jornada del sujeto correcto (operador vs agente).
- **AC5 (tiempo)** — ningún `cuando`/estado lo timbra la UI ni la IA; siempre el broker (regla sagrada).
- **AC6 (degradación)** — broker offline → banner, la UI no crashea; la acción no se pierde silenciosamente
  (se reintenta o se reporta).

---

## 8. Constraints

- No reescribe las 164 RFCs: instancia el patrón sobre ellas. Reusa las máquinas de estado de `peers-core`
  (`EstadoMensaje`/`EstadoTarea`/`TipoAlerta`/`TipoAccion`) — cero duplicación. Toda mutación emite bitácora
  (`#[non_exhaustive]` permite variantes nuevas de empresa). El tiempo lo timbra el broker. Red en
  `background_executor` + `bloquear_en`. Sin `.unwrap()`/`.expect()` en prod. Español salvo protocolo.
  NUNCA `Co-Authored-By`. Jornada en el commit.

## 9. Fuera de alcance

- El detalle de cada una de las 164 features (ya vive en `.specs/desktop/**`). La UI que las expone →
  [[08-capa-ui]]. El motor de validación de cadena (broker) → [[06-decisiones]] E-12 + RFC Delegación.

---
#empresa #workflow #trazabilidad #cadena #maquinas-estado #bitacora #164-features
