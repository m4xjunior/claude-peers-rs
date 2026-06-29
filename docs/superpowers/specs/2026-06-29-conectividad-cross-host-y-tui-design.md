# Diseño — Conectividad cross-host + TUI de control (claude-peers-rs)

> Fecha: 2026-06-29 · Autor: Claudio (arquitecto) con Max
> Objetivo del usuario: "desde mi Mac quiero que mis Claudes vean y hablen con las
> instancias de los servidores", y un panel de control total (`claudepeers --tui`).

## Problema

Hoy cada máquina corre su propio broker en `localhost:7899` → islas separadas. El Claude del
Mac y el del servidor NO se ven porque preguntan a brokers distintos. No es un bug del Rust:
falta la topología de red cross-host. El `peers-client` YA soporta `--broker-url` y la liveness
es por latido (no PID) — el código está preparado, falta configurar y dar control de acceso.

## Decisiones tomadas (con Max)

1. **Topología:** 1 broker central en el Mac. Los servers NO corren broker propio; su
   `peers-client` apunta por IP al broker del Mac (`--broker-url http://<mac>:7899`).
2. **Red:** misma red local / privada / pública alcanzable por IP (sin túnel por ahora; el
   broker debe ser alcanzable por IP — local, privada o pública).
3. **Control de acceso:** TOKEN compartido (`X-Peers-Token`). El broker rechaza sin token válido.
4. **TUI:** tercer binario `peers-tui` (ratatui), completa. Lanzada con `claudepeers --tui`.
5. **Orden:** FASE 1 conectividad (que se vean) primero y probar; FASE 2 la TUI encima.

## Arquitectura

```
┌─────────── TU MAC ───────────┐         SERVIDORES
│ peers-broker :7899 (0.0.0.0)  │◄──HTTP+token── peers-client (server)
│   └─ Redis cprs:              │                --broker-url http://<mac>:7899
│ peers-tui ──HTTP+token──┐     │◄──HTTP+token── peers-client (server 2)
│   (panel de control)    └─────┤
│ peers-client (Mac, local)     │
└───────────────────────────────┘
```

La TUI habla con el broker por su **API HTTP** (no toca Redis directo) → reutilizable contra
cualquier broker y mantiene el broker como única fuente de verdad.

---

## FASE 1 — Conectividad cross-host (entregable que resuelve el objetivo urgente)

### 1.1 Token de autenticación en el broker
- Nuevo: middleware axum que exige header `X-Peers-Token` en todas las rutas salvo `/salud`.
- Token desde `--token` / `CLAUDE_PEERS_TOKEN` (broker). Si no se setea → sin auth (compat
  local: localhost no necesita token; el token se exige solo cuando se configura).
- SEGURIDAD (regla de borde): si el broker escucha en host ≠ 127.0.0.1 (expuesto en red) Y no
  hay token → emite un WARNING ruidoso al arrancar ("broker expuesto sin token"). No bloquea
  (puede ser una LAN/VPN cerrada), pero nunca es silencioso. Evita el agujero accidental.
- El `peers-client` manda el header si tiene `CLAUDE_PEERS_TOKEN`. `ClienteBroker` lo inyecta.
- Decisión de borde: si el broker tiene token configurado y el client no lo manda → 401 claro.

### 1.2 Broker alcanzable en red
- Ya existe `--host` (default 127.0.0.1). Documentar/usar `--host 0.0.0.0` para exponerlo.
- El LaunchAgent del Mac pasa a `--host 0.0.0.0 --token <tok>` (config explícita, anunciada).

### 1.3 Client remoto apunta al Mac
- En el servidor: `peers-client --broker-url http://<ip-mac>:7899` + `CLAUDE_PEERS_TOKEN`.
- Vía el plugin: variables de entorno en el `.mcp.json` del plugin o en el entorno del server.

### 1.4 Verificación Fase 1 (criterio de hecho)
- Desde el server, `listar_instancias` muestra los peers del Mac y viceversa.
- Mensaje server→Mac llega como `<channel>`; Mac→server también. Round-trip cross-host real.
- Sin token válido → el broker rechaza (401). Con token → entra.

---

## FASE 2 — TUI de control (`peers-tui`)

### 2.1 Binario y stack
- `crates/peers-tui` con `ratatui` + `crossterm`. Lanzado por `claudepeers --tui` (la función
  shell detecta `--tui` y ejecuta `peers-tui` con la URL+token del broker).
- Config en `~/.config/claude-peers/config.toml` (broker_url, token, refresh, etc.).

### 2.2 Pantallas (Tab / teclas 1-5)
1. **Peers** — tabla viva (id, dir, resumen, visto, vivo/muerto), refresh 1s.
   Acciones: `m` enviar mensaje, `k` kick (`/salir`), `r` editar resumen (`/definir-resumen`),
   `Enter` detalle + jornada (`/jornada`).
2. **Red/Acceso** — host:puerto anunciado; token (ver/regenerar); allowlist de IPs (opcional).
3. **Redis** — claves `cprs:`, colas pendientes por peer, outbox sin confirmar; purgar.
4. **Broker** — estado/uptime/puerto, parámetros de liveness, reiniciar.
5. **Config** — todo parametrizable, persistido en el toml.

### 2.3 Endpoints nuevos en el broker (hoy NO existen — verificado)
- `GET /admin/redis` — resumen de claves/colas/outbox (para pantalla Redis).
- `POST /admin/purgar` — purga cola/outbox de un peer.
- `GET/POST /admin/config` — leer/editar parámetros en vivo (puerto requiere reinicio; liveness no).
- `POST /admin/regenerar-token` — rota el token.
- `GET /admin/info` — uptime, puerto, host, nº instancias (para pantalla Broker).
- Todos bajo el mismo middleware de token.

### 2.4 Flujo de datos y errores
- Refresh async (tokio) cada 1s vía `/listar` + `/salud` + `/admin/info`.
- Broker offline → la TUI muestra "broker offline" y reintenta; nunca panic (err-no-unwrap-prod).
- Token en config local, nunca en git.

### 2.5 Testing
- Lógica pura (parseo config, validación IP/token, formato de filas) con tests unitarios.
- Render de ratatui con su `TestBackend` (buffer assertions).

---

## Lo que NO entra (YAGNI)
- Brokers federados / replicación entre brokers (descartado: 1 broker central basta).
- Túnel cloudflared (la red es local/privada/pública por IP directa; el túnel es receta aparte
  documentada en docs/distribucion.md si algún día se necesita).
- Editor genérico de claves Redis arbitrarias (solo las vistas de dominio: peers, colas, outbox).

## Archivos afectados (estimado)
- Fase 1: `crates/peers-broker/src/main.rs` (middleware token + --token), `crates/peers-client/
  src/broker.rs` (inyectar header), `peers-core` (si hace falta tipo de error 401), LaunchAgent.
- Fase 2: nuevo `crates/peers-tui/` completo; `crates/peers-broker/src/main.rs` (+endpoints
  /admin/*); función `claude()` del shell (rama --tui).
