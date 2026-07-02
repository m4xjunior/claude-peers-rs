# RFC — Política de comunicación entre peers (bloquear quién habla con quién)

> ⬆ [[_MOC|Mapa de la vault]] · [[INDICE-RFCS|Índice]]

> Fecha: 2026-07-02. Estado: **FASE 1 IMPLEMENTADA (motor + endpoints, 2026-07-02 por Jefim s004)** —
> AC1-AC6 verificados E2E; UI (R10-R12) pendiente. Id del operador unificado en `peers-core`
> (`ID_OPERADOR`/`REMITENTES_EXENTOS`). Despliegue a producción autorizado por Max.
> **Ámbito mixto:** el CONTROL (panel de reglas) vive en la desktop, pero el MOTOR (evaluación de la
> política) es del **broker** — por eso está en el vault desktop pero toca `peers-broker`. Ver §3.
> Toca: `crates/peers-broker` (main.rs `enviar`, store/trait), `crates/peers-core` (DTOs), y un panel
> en `peers-desktop` + `peers-tui`. Verificado contra `main.rs:352 enviar()` y `almacen.rs:76 encolar_mensaje`.

---

## 0. Decisión de arquitectura previa: ¿RabbitMQ? → NO.

Max preguntó si conviene RabbitMQ o si "Rust lo integra nativamente". **Respuesta: ni uno ni otro —
no hace falta un message broker nuevo.**

- **RabbitMQ viola el principio #1 de la VISÃO** ("ZERO dependencias externas; por eso es Rust/binario").
  Es un servicio Erlang aparte que habría que instalar y mantener en CADA máquina — justo lo que el
  proyecto evita (fue lo que "travó no ai-studio" con bun/node).
- **Rust NO trae un message broker nativo.** Pero el proyecto **ya tiene uno**: `peers-broker` sobre
  **Redis**, con colas ZSET durables, outbox con ACK y estados timbrados (`store.rs:10`). RabbitMQ sería
  un segundo broker redundante encima del que ya funciona y ya resuelve durabilidad/no-pérdida.
- **Esta feature NO es un broker: es una regla de autorización** en el punto donde ya pasan todos los
  mensajes. Cero infra nueva. Cero dep nueva.

---

## 1. El problema (en palabras de Max)

> "Prohibición de que otros peers se comuniquen con otros peers. Puedo elegir bloquear comunicaciones,
> etc."

Hoy **cualquier peer puede escribir a cualquier peer**: `POST /enviar` solo comprueba que el destino
exista (`main.rs:357`), no si el emisor tiene permiso. Max quiere **controlar la topología de
comunicación** de su equipo: aislar peers, crear silos, o poner el equipo en modo "solo el operador
habla" — como un firewall de mensajería interno.

Casos de uso reales del proyecto:
- Aislar un peer que está en una tarea sensible para que otros no lo interrumpan (choca con el
  "tócale el hombro" — por eso debe ser configurable, no fijo).
- Cortar un bucle de mensajes entre dos peers que se están saturando mutuamente.
- Modo "solo operador": solo Max (desde la desktop) puede iniciar mensajes; los peers no se hablan
  entre sí (útil en demos o en trabajo dirigido).

## 2. La solución

Una **política de comunicación** evaluada en el broker en el ÚNICO punto por donde pasan todos los
mensajes: `enviar()` (`main.rs:352`), entre el chequeo de existencia y `encolar_mensaje`. La política es
una lista de reglas `(de, para) → permitir | bloquear`, persistida en el store (Redis/SQLite) y editable
desde la UI. **Default: todo permitido** (compat total con hoy). Es opt-in: Max añade bloqueos cuando
quiere.

---

## 3. Requisitos (trazables)

### Modelo

- **R1** — `ReglaComunicacion { de: Patron, para: Patron, accion: Permitir|Bloquear, motivo: Option<String> }`.
  `Patron` = `Cualquiera` (`*`) | `Id(String)` | (futuro) `Grupo(String)`. Serialize en español salvo…
  (protocolo interno). `#[serde(default)]` donde toque para compat.
- **R2** — La política es una **lista ordenada** de reglas + una `accion_por_defecto` (default
  `Permitir`). Se evalúa de arriba abajo; **primera regla que casa gana** (como un firewall). Si ninguna
  casa → `accion_por_defecto`. Clave de store `cprs:politica_comunicacion`.
- **R3** — Regla de sistema NO editable: **el operador (`de = "<id reservado del operador>"`) y el
  `"broker"` NUNCA se bloquean** (para que Max y el propio broker siempre puedan escribir: forzar tareas,
  alertas, chat privado). Coherente con la reserva de id del fix de colisión (`STATE.md`).

### Broker — punto de intercepción

- **R4** — En `enviar()` (`main.rs:352`), tras `instancia_existe` y ANTES de `encolar_mensaje`, evaluar
  `politica.permite(de_id, para_id)`. Si bloquea:
  - responder `RespuestaEnviar { ok: false, error: Some("comunicación bloqueada por política: <motivo>") }`
    (NO 500; es una respuesta de negocio, el emisor la ve).
  - **NO encolar** el mensaje. Opcional (R7): registrar el intento bloqueado para trazabilidad.
- **R5** — Cubrir TODOS los caminos que encolan, no solo `/enviar`: `tarea/forzar` y `admin/reenviar`
  reusan `encolar_mensaje` (`main.rs:928,1098`). Decisión: la política se evalúa en `encolar_mensaje`
  del trait (un solo sitio) O explícitamente en cada handler. **Recomendado:** un helper
  `evaluar_politica(de, para)` llamado en `enviar`; `tarea/forzar` y `admin/reenviar` son acciones del
  OPERADOR (de = broker/operador) → exentas por R3, así que no se bloquean solas. Documentar esta
  decisión para que no haya sorpresa (forzar una tarea nunca se bloquea; es Max actuando).
- **R6** — Endpoints (bajo token):
  - `GET /admin/politica` → la política actual (lista de reglas + default).
  - `POST /admin/politica` → reemplaza la política completa (idempotente; validar patrones).
  - `POST /admin/politica/regla` → añade/quita/reordena una regla (o se hace todo con el PUT completo de
    arriba — decidir en Design cuál API, prefiero el reemplazo completo por simplicidad).
- **R7 (trazabilidad, opcional pero recomendado)** — Los intentos bloqueados se registran en
  `cprs:comunicacion_bloqueada` (LIST acotada, últimas N): `{de, para, motivo_regla, cuando}`. La UI lo
  pinta como "N intentos bloqueados" — Max ve que la política está actuando, no en silencio.

### Persistencia

- **R8** — Impl en AMBOS backends del trait `Almacen`: `politica_leer()`, `politica_guardar(politica)`,
  y (si R7) `registrar_bloqueo(...)` / `bloqueos_recientes()`. Redis (default) + SQLite (feature flag).
- **R9** — La política se carga en memoria del broker al arrancar y se refresca al escribir (no leer
  Redis en cada `/enviar` — es ruta caliente). Un `RwLock<Politica>` en el estado del broker; se
  actualiza en el `POST /admin/politica`. (Ownership: liberar el lock antes de cualquier `.await`.)

### UI (desktop + TUI)

- **R10 — Desktop (gpui-component):** panel "Política de comunicación" (en pestaña Broker o Acceso, o
  nueva sub-vista). Tabla de reglas (de → para → acción, motivo) con CRUD: añadir regla (`Select` de peer
  o `*` para de/para, `Switch` permitir/bloquear, `Input` motivo), reordenar (drag o ▲▼), borrar. Toggle
  rápido "Modo solo-operador" (preset: una regla `* → *: bloquear` + el default). Reusa componentes ya
  disponibles (`Table`, `Select`, `Switch`, `Button`, `Dialog`).
- **R11 — Desktop: matriz visual (variante Ethos, opcional).** Una rejilla peer×peer donde cada celda es
  permitir (BRASA tenue) / bloquear (rojo `#7F1D1D`), clicable para alternar. Da una vista de topología
  de un golpe. Alternativa a la tabla; decidir en Design.
- **R12 — TUI (paridad):** pantalla o modal para ver/editar la política por teclado (la TUI corre por
  SSH; debe poder gestionar la política sin la desktop). Mínimo: ver reglas + añadir/quitar bloqueo.
- **R13** — Todo degrada: broker offline/401 → banner, sin crash. Sin `.unwrap()` en prod.

---

## 4. Criterios de aceptación

- **AC1 (R2/R4)** — Con una regla `A → B: bloquear`, `A` enviando a `B` recibe `ok:false` con el motivo y
  el mensaje NO llega a la bandeja de `B`. `A → C` (sin regla) sí llega (default Permitir).
- **AC2 (R2 orden)** — Con `[A→B: permitir, *→B: bloquear]`, `A→B` pasa (primera regla gana) y `X→B`
  (cualquier otro) se bloquea.
- **AC3 (R3)** — El operador (id reservado) y `broker` SIEMPRE pueden escribir a cualquier peer, aunque
  haya `*→*: bloquear`. `tarea/forzar` y el chat privado nunca se bloquean.
- **AC4 (R6/R9)** — `POST /admin/politica` actualiza la política en caliente (sin reiniciar el broker) y
  el siguiente `/enviar` la respeta. `GET /admin/politica` la devuelve. Sin token → 401.
- **AC5 (R7)** — Un intento bloqueado queda registrado y la UI muestra el contador/lista.
- **AC6 (compat)** — Sin política configurada (clave ausente), todo se permite (comportamiento actual);
  JSON viejo sin la sección deserializa sin error.
- **AC7 (R10/R12)** — Se puede crear un bloqueo desde la desktop Y desde la TUI, y el efecto es el mismo
  (misma política en el broker).

---

## 5. Riesgos y decisiones abiertas (Design)

1. **Choque con el "tócale el hombro" (VISÃO).** El proyecto presume peers que se interrumpen entre sí
   ("compañero que te toca el hombro", `mcp.rs::instrucciones`). Bloquear comunicación va contra esa
   filosofía por diseño — por eso es **opt-in y por reglas**, no global. Documentar que activar bloqueos
   cambia el comportamiento colaborativo esperado.
2. **Alcance del bloqueo: ¿solo mensajes, o también descubrimiento?** ¿Un peer bloqueado debe seguir
   apareciendo en `listar_instancias` del otro? Propuesta: la política v1 solo filtra **envío**
   (`/enviar`); el descubrimiento (`/listar`) NO se filtra (Max sí quiere verlos todos desde la UI).
   Filtrar descubrimiento por-peer es más complejo (cada `/listar` sabría por identidad del que pregunta)
   → fuera de v1.
3. **Id del operador.** Depende de reservar un id estable para "Max desde la desktop/TUI" — mismo tema
   que el fix de colisión de ID (`STATE.md`) y el `de` del chat privado. Unificar los tres.
4. **Grupos (futuro).** `Patron::Grupo` permitiría reglas por equipo ("front-* no habla con backend-*").
   YAGNI en v1; el modelo lo deja abierto (el enum `Patron` ya contempla la variante).
5. **Rendimiento.** La evaluación es sobre una lista en memoria (`RwLock`), O(n reglas) por envío — n es
   pequeño (decenas). Sin impacto. No tocar Redis en la ruta caliente (R9).

---

## 6. Constraints

- Sin `.unwrap()`/`.expect()` en prod; `Result`/`anyhow`. **NO liberar el `RwLock` de la política a
  través de un `.await`** (regla Rust del proyecto). Español salvo protocolo. Redis + SQLite (ambos).
- **CERO dependencias nuevas** (ni RabbitMQ ni ningún broker/cola externa — §0). Reusa el broker y el
  store existentes.
- No romper el wire de `/enviar` (los peers viejos siguen enviando igual; solo pueden recibir un
  `ok:false` nuevo, que ya es un caso contemplado en `RespuestaEnviar`). Versionar plugin si se tocan
  binarios. NUNCA `Co-Authored-By`. Jornada en el commit.

## 7. Fuera de alcance (v1)

- Filtrado del descubrimiento (`/listar`) por identidad (riesgo #5.2).
- Grupos/equipos (`Patron::Grupo`) — modelo abierto, impl posterior.
- Rate-limiting / anti-flood (distinto de bloqueo binario; otra feature si Max lo pide).
- Cifrado o firma de mensajes entre peers (no es el problema aquí).

## 8. Dependencias

- **Id reservado del operador** — compartido con el fix de colisión de ID (`STATE.md`) y con la spec
  `lanzador-sesion-terminal` (chat privado). Resolver los tres juntos.
- **UI** — reusa el patrón de carga/refresco de `desktop-carga-datos` y los helpers Ethos de `tema.rs`.

---
#rfc #peers-desktop #politica #comunicacion #broker #seguridad
