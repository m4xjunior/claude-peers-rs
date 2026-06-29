# Fase 1 — Conectividad cross-host (token + broker en red) — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Que los Claudes del Mac y de los servidores se vean y hablen contra UN broker central en el Mac, con autenticación por token.

**Architecture:** El broker (Mac) expone su API HTTP en la red (`--host 0.0.0.0`) protegida por un token (`X-Peers-Token`). Los `peers-client` remotos apuntan al Mac con `--broker-url` + `CLAUDE_PEERS_TOKEN`. El token se verifica en un middleware axum; `/salud` queda exento. Si el broker se expone sin token, avisa con un warning ruidoso.

**Tech Stack:** Rust, axum 0.8 (middleware `from_fn`), reqwest (header en el client), clap (flag `--token`).

## Global Constraints

- Sin `.unwrap()`/`.expect()` en producción; errores con `anyhow`/`Result` (CLAUDE.md Rust).
- Todo en español (código, comentarios, mensajes) salvo las 4 claves del push.
- El header de auth se llama exactamente `X-Peers-Token`.
- Compat: sin token configurado en el broker → no se exige auth (uso local localhost sigue igual).
- NUNCA `Co-Authored-By` en commits. Declarar jornada en el cuerpo del commit.

---

### Task 1: Token en el broker (flag + middleware de auth)

**Files:**
- Modify: `crates/peers-broker/src/main.rs` (struct `Args`, `main`, nuevo middleware)
- Test: `crates/peers-broker/src/main.rs` (módulo `#[cfg(test)] mod pruebas`)

**Interfaces:**
- Consumes: `Args` (clap) existente; axum `Router`, `State`.
- Produces: `fn auth_token(token_esperado: Option<String>) -> impl tower::Layer` no — usamos
  `axum::middleware::from_fn_with_state`. Produce: campo `Args.token: Option<String>` y una
  función `async fn verificar_token(headers, request, next) -> Response` registrada como layer.

- [ ] **Step 1: Añadir el flag `--token` al struct `Args`**

En `crates/peers-broker/src/main.rs`, dentro de `struct Args`, tras el campo `db`:

```rust
    /// Token de acceso. Si se setea, el broker exige el header `X-Peers-Token` en todas las
    /// rutas salvo /salud. Sin token → sin auth (uso local localhost sigue funcionando igual).
    #[arg(long, env = "CLAUDE_PEERS_TOKEN")]
    token: Option<String>,
```

- [ ] **Step 2: Escribir el test del middleware (falla primero)**

En `crates/peers-broker/src/main.rs`, en el `mod pruebas` existente, añadir:

```rust
    #[test]
    fn token_correcto_pasa_y_ausente_falla() {
        // Lógica pura de decisión del middleware, sin levantar axum.
        assert!(token_autorizado(Some("abc"), Some("abc")));   // coincide → pasa
        assert!(!token_autorizado(Some("abc"), Some("xyz")));  // distinto → falla
        assert!(!token_autorizado(Some("abc"), None));         // falta → falla
        assert!(token_autorizado(None, None));                 // sin token configurado → pasa
        assert!(token_autorizado(None, Some("lo-que-sea")));   // sin config → pasa (ignora)
    }
```

- [ ] **Step 3: Run test (debe fallar: `token_autorizado` no existe)**

Run: `cargo test -p peers-broker token_correcto -- --nocapture`
Expected: FAIL — `cannot find function token_autorizado`

- [ ] **Step 4: Implementar `token_autorizado` + el middleware**

En `crates/peers-broker/src/main.rs`, antes de `async fn main`:

```rust
/// Decide si una petición está autorizada. Pura y testeable.
/// - Sin token configurado (None) → siempre autorizado (compat local).
/// - Con token configurado → autorizado solo si el recibido coincide exacto.
fn token_autorizado(configurado: Option<&str>, recibido: Option<&str>) -> bool {
    match configurado {
        None => true,
        Some(esperado) => recibido == Some(esperado),
    }
}

/// Middleware axum: aplica `token_autorizado` usando el header `X-Peers-Token`.
/// /salud queda exento (se monta fuera de esta capa).
async fn verificar_token(
    axum::extract::State(token): axum::extract::State<Option<String>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let recibido = req
        .headers()
        .get("x-peers-token")
        .and_then(|v| v.to_str().ok());
    if token_autorizado(token.as_deref(), recibido) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, "token inválido o ausente").into_response()
    }
}
```

- [ ] **Step 5: Run test (debe pasar)**

Run: `cargo test -p peers-broker token_correcto -- --nocapture`
Expected: PASS

- [ ] **Step 6: Montar el middleware en el Router + warning de exposición**

En `async fn main`, separar `/salud` (sin auth) del resto (con auth). Reemplazar la
construcción del `Router` actual por:

```rust
    use axum::middleware::from_fn_with_state;

    // Warning ruidoso si se expone en red sin token (agujero accidental).
    if args.host != "127.0.0.1" && args.host != "localhost" && args.token.is_none() {
        warn!("broker EXPUESTO en {} SIN token — cualquiera en la red puede conectarse. \
               Usa --token para protegerlo.", args.host);
    }

    let rutas_protegidas = Router::new()
        .route("/registrar", post(registrar))
        .route("/latido", post(latido))
        .route("/definir-resumen", post(definir_resumen))
        .route("/listar", post(listar))
        .route("/enviar", post(enviar))
        .route("/recibir", post(recibir))
        .route("/salir", post(salir))
        .route("/tarea/abrir", post(tarea_abrir))
        .route("/tarea/reportar", post(tarea_reportar))
        .route("/tarea/cerrar", post(tarea_cerrar))
        .route("/jornada", post(jornada_consolidada))
        .layer(from_fn_with_state(args.token.clone(), verificar_token));

    let app = Router::new()
        .route("/salud", get(salud))   // exenta de auth
        .merge(rutas_protegidas)
        .with_state(estado);
```

- [ ] **Step 7: Compilar y correr todos los tests del broker**

Run: `cargo build -p peers-broker && cargo test -p peers-broker`
Expected: compila; todos los tests PASS (incluye pid_vivo y token).

- [ ] **Step 8: Commit**

```bash
git add crates/peers-broker/src/main.rs
git commit -m "feat(broker): auth por token X-Peers-Token (/salud exenta) + warning si expuesto sin token

Jornada: <HH:MM>→<HH:MM> (España)"
```

---

### Task 2: El client envía el token

**Files:**
- Modify: `crates/peers-client/src/broker.rs` (struct `ClienteBroker`, `nuevo`, `post`, `esta_vivo`)
- Modify: `crates/peers-client/src/main.rs` (leer `--token` y pasarlo a `ClienteBroker::nuevo`)
- Test: `crates/peers-client/src/broker.rs` (no hay red en test → test de construcción mínima)

**Interfaces:**
- Consumes: `ClienteBroker::nuevo(base)` actual.
- Produces: `ClienteBroker::nuevo(base, token: Option<String>)` — NUEVA firma (2 args). Todos
  los llamadores deben pasar el token.

- [ ] **Step 1: Añadir campo `token` y cambiar la firma de `nuevo`**

En `crates/peers-client/src/broker.rs`, reemplazar el struct y `nuevo`:

```rust
#[derive(Clone)]
pub struct ClienteBroker {
    http: Client,
    base: String,
    token: Option<String>,
}

impl ClienteBroker {
    pub fn nuevo(base: impl Into<String>, token: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("cliente HTTP no pudo construirse");
        Self { http, base: base.into(), token }
    }
```

- [ ] **Step 2: Inyectar el header en `post` y `esta_vivo`**

En `post`, cambiar la construcción del request para añadir el header si hay token:

```rust
        let url = format!("{}{}", self.base, ruta);
        let mut req = self.http.post(&url).json(cuerpo);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("broker no responde en {url} (¿está levantado?)"))?;
```

En `esta_vivo`, igual (aunque /salud no exige token, no estorba enviarlo):

```rust
    pub async fn esta_vivo(&self) -> bool {
        let url = format!("{}/salud", self.base);
        let mut req = self.http.get(&url);
        if let Some(t) = &self.token {
            req = req.header("X-Peers-Token", t);
        }
        match req.send().await {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }
```

- [ ] **Step 3: Añadir flag `--token` al client y pasarlo a `ClienteBroker::nuevo`**

En `crates/peers-client/src/main.rs`, en `struct Args` tras `puerto`:

```rust
    /// Token de acceso al broker (debe coincidir con el del broker si éste lo exige).
    #[arg(long, env = "CLAUDE_PEERS_TOKEN")]
    token: Option<String>,
```

Y donde se construye el broker (`let broker = ClienteBroker::nuevo(url.clone());`), cambiar a:

```rust
    let broker = ClienteBroker::nuevo(url.clone(), args.token.clone());
```

- [ ] **Step 4: Compilar el workspace**

Run: `cargo build --workspace`
Expected: compila sin errores (la nueva firma de 2 args queda satisfecha en el único llamador).

- [ ] **Step 5: Commit**

```bash
git add crates/peers-client/src/broker.rs crates/peers-client/src/main.rs
git commit -m "feat(client): envía X-Peers-Token al broker (flag/env --token)

Jornada: <HH:MM>→<HH:MM> (España)"
```

---

### Task 3: Verificación E2E del token (broker en red + 2 clients)

**Files:**
- (sin cambios de código; es una prueba manual reproducible)

**Interfaces:**
- Consumes: binarios `target/release/{peers-broker,peers-client}` recompilados.

- [ ] **Step 1: Recompilar release**

Run: `cargo build --release`
Expected: Finished release.

- [ ] **Step 2: Levantar un broker de prueba con token en otro puerto (no toca el :7899 vivo)**

```bash
CLAUDE_PEERS_TOKEN=secreto123 ./target/release/peers-broker --puerto 7901 --host 127.0.0.1 \
  >/tmp/wf-broker-token.log 2>&1 &
sleep 1
```

- [ ] **Step 3: Probar que SIN token el broker rechaza (401), CON token acepta**

```bash
# sin token → 401
curl -s -o /dev/null -w "sin token: %{http_code}\n" -X POST http://127.0.0.1:7901/listar \
  -H 'content-type: application/json' -d '{"alcance":"maquina","directorio":"/","repo_git":null}'
# con token → 200
curl -s -o /dev/null -w "con token: %{http_code}\n" -X POST http://127.0.0.1:7901/listar \
  -H 'content-type: application/json' -H 'X-Peers-Token: secreto123' \
  -d '{"alcance":"maquina","directorio":"/","repo_git":null}'
# /salud SIEMPRE responde (exenta)
curl -s -o /dev/null -w "salud sin token: %{http_code}\n" http://127.0.0.1:7901/salud
```
Expected: `sin token: 401`, `con token: 200`, `salud sin token: 200`.

- [ ] **Step 4: Probar que el client con token se registra contra el broker protegido**

```bash
( printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"cc","version":"1"}}}'; sleep 2 ) \
  | CLAUDE_PEERS_ID=probador CLAUDE_PEERS_TOKEN=secreto123 \
    ./target/release/peers-client --broker-url http://127.0.0.1:7901 2>/tmp/wf-client-token.stderr >/dev/null
grep "registrado como" /tmp/wf-client-token.stderr
```
Expected: `registrado como instancia 'probador'`.

- [ ] **Step 5: Limpiar la prueba**

```bash
curl -s -X POST http://127.0.0.1:7901/salir -H 'content-type: application/json' \
  -H 'X-Peers-Token: secreto123' -d '{"id":"probador"}' >/dev/null
pkill -f 'peers-broker --puerto 7901'
```

- [ ] **Step 6: Activar el token en el broker REAL del Mac (:7899) y exponerlo en red**

Editar el LaunchAgent `~/Library/LaunchAgents/com.lexusfx.claude-peers.plist`: añadir
`--host 0.0.0.0` a `ProgramArguments` y `CLAUDE_PEERS_TOKEN` con un token elegido a
`EnvironmentVariables`. Recargar:

```bash
launchctl unload ~/Library/LaunchAgents/com.lexusfx.claude-peers.plist
launchctl load ~/Library/LaunchAgents/com.lexusfx.claude-peers.plist
sleep 2
curl -s -o /dev/null -w "salud: %{http_code}\n" http://127.0.0.1:7899/salud
```
Expected: `salud: 200`. (El Mac local sigue funcionando; el client del Mac necesitará el
token también → setear `CLAUDE_PEERS_TOKEN` en su entorno / `.mcp.json`.)

> NOTA: este paso cambia config viva del Mac. Anunciar a Max antes (config sagrada). El token
> elegido debe ir también al entorno del client del Mac y de los servers.

- [ ] **Step 7: En el servidor: apuntar el client al Mac + token, y verificar que se ven**

En el servidor (vía el `.mcp.json` del plugin o el entorno):
`CLAUDE_PEERS_BROKER_URL=http://<ip-mac>:7899` y `CLAUDE_PEERS_TOKEN=<token>`.
Reiniciar el Claude del server. Desde el Mac, `listar_instancias` debe mostrar al peer del
server; mensaje Mac↔server debe llegar como `<channel>`.

Expected: round-trip cross-host real. **Criterio de hecho de la Fase 1.**

---

## Self-Review

- **Cobertura del spec (Fase 1):** token en broker (Task 1) ✓; broker en red + warning (Task 1
  step 6) ✓; client manda token (Task 2) ✓; client remoto apunta al Mac (Task 3 step 7) ✓;
  verificación cross-host (Task 3) ✓.
- **Placeholders:** los `<HH:MM>` de los commits y `<ip-mac>`/`<token>` son valores que Max
  aporta en runtime (hora real con `date`, su IP/token), no placeholders de lógica. El código
  está completo.
- **Consistencia de tipos:** `ClienteBroker::nuevo` pasa a 2 args en Task 2 y se actualiza su
  único llamador en el mismo task. `token_autorizado`/`verificar_token` definidos en Task 1 y
  usados solo ahí. Header `X-Peers-Token` idéntico en broker (Task 1) y client (Task 2).
