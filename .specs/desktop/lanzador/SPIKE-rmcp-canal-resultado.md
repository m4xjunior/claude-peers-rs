# SPIKE E-21 (riesgo #1) — ¿rmcp puede modelar el push custom del `<channel>`? — RESULTADO

> Fecha: 2026-07-06. Autor: claude-peers-rs-s002 (implementación). Contexto: E-21 (migrar el MCP a
> mano → `rmcp`) marca como **riesgo #1** que el push del `<channel>` use una notificación con
> método NO estándar (`notifications/claude/channel`) + capability experimental `claude/channel`,
> y la duda era si `rmcp` lo soporta o rompería el canal. **Este spike responde esa pregunta ANTES
> de reescribir el client.**

## Veredicto: ✅ POSITIVO — riesgo #1 DISIPADO

`rmcp` **sí** soporta ambos requisitos. La migración a rmcp NO rompe el `<channel>`.

## Qué se probó (con compilación real, no solo doc)

Se compiló y ejecutó un binario de spike aislado (`spike-rmcp-canal`, ya eliminado del árbol) contra
`rmcp 1.8.0` (versión que resuelve `rmcp = "1"`), verificando dos incógnitas:

### 1. Notificación con método arbitrario + params custom
- `rmcp::model::CustomNotification { method: String, params: Option<Value>, extensions }` existe y es
  precisamente "a catch-all notification either side can use to send custom messages to its peer,
  preserving the raw `method` name and `params` payload" (doc oficial).
- `impl From<CustomNotification> for ServerNotification` → se envía por
  `RunningService::send_notification(notif)` / `Peer::send_notification`.
- **Wire serializado IDÉNTICO** al `SalidaMcp::empujar_canal` actual (`mcp.rs:91-103`):
  ```json
  {"method":"notifications/claude/channel",
   "params":{"content":"…","meta":{"from_id":"…","from_summary":"…","from_cwd":"…","sent_at":"…"}}}
  ```
  Verificado con asserts sobre método + las 4 claves de `meta` + `content`.

### 2. Capability experimental
- `ServerCapabilities::builder().enable_experimental().…​.build()` emite el campo `experimental`
  (para declarar `claude/channel`). Confirmado en el wire (`"experimental":{}`).

## Implicaciones para la migración (siguiente tarea)

- El push del canal se reescribe como: construir `CustomNotification` con el mismo método y params,
  convertir a `ServerNotification`, enviar por el `Peer<RoleServer>` guardado del `RunningService`.
  El bucle de recepción actual (`main.rs:666-744`, cada 1s → `empujar_canal`) se conserva; solo
  cambia la última línea (cómo se emite la notif).
- Las capabilities experimentales van en `get_info()` → `ServerInfo` (no en un JSON manual).
- **Invariante a preservar** (guardarraíl = tests de push del client): `serverInfo.name = "claude-peers"`,
  método `notifications/claude/channel`, las 4 claves `meta`. El spike confirma que rmcp los respeta.
- Deps que arrastra rmcp 1.8.0 (medido en el spike): hyper, tower, tower-http, schemars, chrono,
  tokio-util, rmcp-macros, pastey, ref-cast. Peso a evaluar contra el criterio de binario portable,
  pero E-22 permite deps externas (robustez > minimalismo) — no es bloqueante.

## Recomendación

Proceder con la migración completa a rmcp. El riesgo #1 no se materializa: no hace falta el fallback
del "escritor de stdout mínimo para la notif custom" que la spec contemplaba por si rmcp no cubría el
push — rmcp lo cubre nativamente vía `CustomNotification`.

#spike #rmcp #e21 #channel #mcp
