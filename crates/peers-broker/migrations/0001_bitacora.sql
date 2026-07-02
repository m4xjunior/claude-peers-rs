-- Bitácora de acciones (RFC registro-acciones / ADR-001-bitacora-sqlx-identidad-durable).
-- Fichero PROPIO del broker (independiente del backend Redis/rusqlite). FK reales, CERO
-- cascade destructivo: el histórico sobrevive a kicks, vencimientos de liveness y podas.

-- Identidad DURABLE de los peers: se upserta al registrar cada acción y NUNCA se borra.
-- (A diferencia de `instancias` del store, que es PRESENCIA efímera y se borra al salir/vencer.)
-- Es además la semilla de identidad estable que pide el fix de colisión de STATE.md.
CREATE TABLE IF NOT EXISTS peers_conocidos (
    id             TEXT PRIMARY KEY,
    primer_visto   TEXT NOT NULL,
    ultimo_visto   TEXT NOT NULL,
    ultimo_resumen TEXT
);

-- ANCLA de FK para tareas, NO fuente de verdad (ADR-001): el estado real de una tarea vive en
-- el store (tarea_guardar). Aquí solo id+dueño+descripción+creación, para que la FK resuelva
-- y los JOIN "quién hizo qué sobre qué tarea" sean consultables en ambos backends.
CREATE TABLE IF NOT EXISTS tareas_conocidas (
    id           TEXT PRIMARY KEY,
    instancia_id TEXT NOT NULL REFERENCES peers_conocidos(id),
    descripcion  TEXT,
    creada_en    TEXT
);

CREATE TABLE IF NOT EXISTS acciones (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    instancia_id TEXT NOT NULL REFERENCES peers_conocidos(id) ON DELETE RESTRICT,
    accion       TEXT NOT NULL,           -- TipoAccion en snake_case (wire de peers-core)
    sujeto       TEXT,                     -- id de tarea/peer/cola afectada (nullable)
    tarea_id     TEXT REFERENCES tareas_conocidas(id) ON DELETE SET NULL,
    detalle      TEXT,
    cuando       TEXT NOT NULL             -- ISO 8601, timbrado por el broker
);

CREATE INDEX IF NOT EXISTS idx_acciones_instancia ON acciones(instancia_id, cuando DESC);
