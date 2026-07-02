# ADR-001: Bitácora de acciones en SQLx sobre fichero propio con identidad durable

- **Fecha**: 2026-07-02
- **Estado**: Aceptado
- **Decisores**: Max (driver, pidió "FK reales, no datos sueltos"), Julio s003 (coord/QA, fijó la dirección), Jefim s004 (dev senior, diseño final)
- **Tags**: persistencia, sqlx, trazabilidad, identidad
- **Relacionado**: [[RFC-registro-acciones-jornada]] (§R8/§5.6), [[../politica-comunicacion/RFC-politica-comunicacion]] (§5.3 id del operador), `STATE.md` (fix de colisión de ID)

## Contexto y problema

La bitácora de acciones (RFC registro-acciones) debe dar **rastreabilidad relacional durable**. La spec
inicial proponía FK `acciones.instancia_id → instancias(id) ON DELETE CASCADE` conviviendo en el mismo
`.db` del backend rusqlite. Dos fuerzas lo invalidan:

1. **`instancias` es PRESENCIA efímera, no identidad**: `salir` (kick) y `limpiar_vencidas` (cada 30s)
   BORRAN filas de instancias. Con CASCADE, la bitácora de un peer se destruiría cada vez que se
   desconecta — lo contrario exacto de lo que pide Max.
2. **En producción el backend es REDIS**: no existe ningún `.db` principal con `instancias`/`tareas` al
   que apuntar FKs. "Mismo fichero + WAL" solo funcionaría en el backend sqlite (que nadie corre en prod).

## Opciones consideradas

- **A. Fichero propio `bitacora.db` (SQLx) con tablas de identidad durable dentro** ✅ elegida
- **B. Mismo `.db` de rusqlite + WAL, FK a `instancias`/`tareas`** (recomendación inicial de la spec §5.6a)
- **C. Fichero propio sin FK físicas (FK "lógicas")**

## Decisión

**Opción A.** El broker abre SIEMPRE (con ambos backends) un `SqlitePool` de SQLx sobre un fichero
propio (`--bitacora-db`, default `~/.config/claude-peers/bitacora.db`) con `journal_mode=WAL`,
`busy_timeout` y `PRAGMA foreign_keys=ON` por conexión. Esquema (migración `sqlx::migrate!`):

- `peers_conocidos(id PK, primer_visto, ultimo_visto, ultimo_resumen)` — **identidad DURABLE**: se
  upserta al registrar cada acción y **NUNCA se borra**. Es además la semilla de identidad estable que
  pide el fix de colisión de STATE.md.
- `tareas_conocidas(id PK, instancia_id → peers_conocidos, descripcion, creada_en)` — **ANCLA de FK,
  no fuente de verdad**: el estado real de las tareas sigue viviendo en el store (Redis/rusqlite via
  `tarea_guardar`). Si diverge, es solo etiqueta para el JOIN; jamás se lee como estado autoritativo.
- `acciones(id, instancia_id NOT NULL → peers_conocidos ON DELETE RESTRICT, accion, sujeto,
  tarea_id → tareas_conocidas ON DELETE SET NULL, detalle, cuando)` + índice `(instancia_id, cuando DESC)`.
  **Cero CASCADE destructivo**: el histórico sobrevive a kicks, vencimientos y podas.

Complementos de la decisión:

- **SQLx 0.8 con consultas DINÁMICAS** (`query`/`bind`, jamás `query!`): se renuncia a los checks
  compile-time a cambio de que NADIE necesite `DATABASE_URL` ni `.sqlx/` para compilar (crítico
  para el equipo multiagente). Features: `runtime-tokio`, `sqlite`, `migrate` y `macros` — esta
  última SOLO porque `sqlx::migrate!` (embebido versionado de `migrations/`) es un proc-macro que
  vive tras ella; NO se usa para validar SQL contra una BD. Para añadir checks compile-time
  después: `cargo sqlx prepare` + commitear `.sqlx/` y migrar las consultas a `query!`.
- **Componente `bitacora.rs` FUERA del trait `Almacen`** — desviación CONSCIENTE de R8b: la bitácora es
  transversal a ambos backends; dentro del trait habría dos impls idénticas compartiendo el mismo pool
  (duplicación sin valor). `EstadoApp.bitacora: Option<Bitacora>` degrada con `warn!` si el fichero no
  abre (R9: la bitácora es observabilidad, nunca tumba el negocio).
- Sin conflicto de `links`: rusqlite 0.32 y sqlx 0.8 comparten `libsqlite3-sys 0.30` (verificado en
  `Cargo.lock`).

## Consecuencias

**Positivas**: FK reales en AMBOS backends (también producción-Redis); histórico inmune al ciclo de
liveness; cero contención sqlx↔rusqlite (ficheros distintos); identidad durable reutilizable por el fix
de colisión; JOINs consultables (quién+tarea+descripción).

**Negativas / trade-offs honestos**: un fichero más que respaldar; `tareas_conocidas` duplica id+descripción
(mitigado: es ancla declarada, no verdad); sin validación compile-time de SQL (mitigado: tests + camino
documentado para activarla); la FK protege integridad referencial pero no autentica al actor (`quien`
confía en el emisor declarado — anti-spoofing es el fix aparte de STATE.md).

## Pros y contras de las opciones

### A. Fichero propio + identidad durable ✅
- ✅ Funciona idéntico con Redis y sqlite; ✅ FK reales sin CASCADE destructivo; ✅ sin locks cruzados con rusqlite.
- ❌ Segundo fichero; ❌ tablas-ancla que mantener upsertadas.

### B. Mismo .db + WAL (spec §5.6a)
- ✅ Un solo fichero; FK directas a tablas "reales".
- ❌ INVIABLE en producción (backend Redis, no hay .db); ❌ CASCADE + liveness borra el histórico; ❌ dos escritores (rusqlite síncrono + sqlx async) sobre el mismo fichero.

### C. FK lógicas
- ✅ Simplicidad máxima.
- ❌ Max pidió FK REALES ("no datos sueltos") — descartada por requisito, no por técnica.
