-- Puente WhatsApp (Evolution API) — historial durable de la conversación con la Letícia.
--
-- Fichero de la bitácora (mismo que peers_conocidos/acciones): es observabilidad/histórico
-- transversal, no negocio del store Redis/rusqlite. Sin FK a peers_conocidos porque `entrada`
-- no tiene autor-peer (es ella escribiendo) y `autor_peer_id` en `salida` es best-effort
-- (quien invocó `/whatsapp/enviar`), no una relación que deba bloquear el insert si el peer
-- nunca llegó a registrarse en la bitácora.
CREATE TABLE IF NOT EXISTS whatsapp_mensajes (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    direccion      TEXT NOT NULL,   -- 'entrada' | 'salida' (DireccionWhatsapp, snake_case)
    texto          TEXT NOT NULL,
    nombre         TEXT,            -- pushName de Evolution; solo tiene sentido en 'entrada'
    autor_peer_id  TEXT,            -- quien respondió; solo tiene sentido en 'salida'
    cuando         TEXT NOT NULL,   -- ISO 8601, timbrado por el broker
    -- `key.id` de Evolution: dedupe de reintentos del webhook (a-at-least-once es la norma en
    -- webhooks). NULL en 'salida' (no viene de Evolution, el broker no la necesita ahí).
    evo_message_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_whatsapp_cuando ON whatsapp_mensajes(cuando DESC, id DESC);

-- Único SOLO cuando no es NULL (índice parcial): permite N filas 'salida' con NULL sin chocar
-- entre sí, y rechaza automáticamente un reintento del webhook con el mismo `key.id`.
CREATE UNIQUE INDEX IF NOT EXISTS idx_whatsapp_evo_msg_id
    ON whatsapp_mensajes(evo_message_id) WHERE evo_message_id IS NOT NULL;
