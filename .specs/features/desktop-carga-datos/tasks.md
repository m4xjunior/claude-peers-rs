# Tasks — peers-desktop: carga de datos

Orden por dependencia. Cada tarea es atómica y verificable. Todo en `crates/peers-desktop/`.
El cliente ya tiene TODOS los métodos necesarios (verificado). No hay que tocar el cliente salvo
sorpresa. Toolchain 1.95.0 (ya fijado).

## T1 — Métodos `cargar_<pantalla>` en AppDesktop `[R3, R4, R7, R8]`
`app.rs`. Añadir un método por pantalla con datos, siguiendo el patrón de `descartar_alerta`
(cx.spawn → cliente.X().await → esta.update → match Ok/Err setea datos/error_* → cx.notify):
- `cargar_peers` → `listar_instancias()` → `datos.peers` / `error_peers`
- `cargar_tareas` → `admin_tareas()` → `datos.tareas` / `error_tareas`
- `cargar_alertas` → `admin_alertas()` → `datos.alertas` / `error_alertas`
- `cargar_broker` → `admin_info()` + `salud()` + `factor_estimacion()` → campos broker / error
- `cargar_redis` → `admin_redis()` → `datos.redis` / `error_redis`
- `cargar_jornada` → `jornada(peer_enfocado)` (si hay peer) → `datos.jornada` / error
- `cargar_trazabilidad` → `historial(peer_enfocado)` → `datos.historial` / error
- `cargar_acceso` → `salud()` → estado de conexión / error
- (Config no carga: edita local)
**Verificar:** `cargo build -p peers-desktop` exit 0 tras añadirlos.

## T2 — `cargar_pantalla_activa` (dispatcher) `[R2, R3.9]`
`app.rs`. `fn cargar_pantalla_activa(&mut self, cx)` con `match self.pantalla { Peers => self.cargar_peers(cx), ... Config => {} }`.
**Verificar:** compila; Config no llama a nada.

## T3 — Carga inicial + timer periódico `[R1, R5]`
`app.rs` en `AppDesktop::nueva` (o justo tras crear la entidad): (a) llamar `cargar_pantalla_activa(cx)`
una vez; (b) lanzar el `cx.spawn` con `loop { sleep(2s); esta.update(|s,cx| s.cargar_pantalla_activa(cx)) o break }.detach()`.
**RIESGO (del design D6):** `tokio::time::sleep` puede panicar si no hay runtime tokio en el executor
de GPUI. Si pasa: cambiar a `cx.background_executor().timer(Duration::from_secs(2)).await` (timer nativo GPUI).
**Verificar:** lanzar `cargo run -p peers-desktop` con broker vivo → screenshot mac-use: Peers muestra
peers reales (no "sin peers vivos"). Esperar 2s en otra pantalla → se puebla sola.

## T4 — Carga al cambiar de pantalla `[R2]`
`app.rs`. En el `on_click` del sidebar (donde se asigna `self.pantalla = nueva`), añadir
`self.cargar_pantalla_activa(cx)` después.
**Verificar:** mac-use → clic en Broker muestra uptime/puerto al instante (no "sin datos"); clic en
Tareas muestra las tareas; en Alertas las alertas.

## T5 — Verificar/cablear banners de error `[R7]`
`vista/*.rs`. Confirmar que cada vista pinta su `error_<pantalla>` cuando es `Some` (banner rojo).
Si alguna no lo hace, añadirlo.
**Verificar:** apagar el broker (`launchctl unload ...plist`), lanzar la app → banner de error en vez
de vacío mudo, sin panic; reencender → se recupera (R8).

## T6 — Verificar acciones end-to-end `[R9, R10]`
Con broker vivo y datos: probar por mac-use las acciones que la UI expone:
- Alertas: descartar (ya cableado — confirmar que recarga).
- Tareas: "Asignar" y acciones de tarea.
- Redis: purgar.
- Config: guardar.
Cablear (patrón descartar_alerta + recarga) las que estén presentes en UI pero no funcionen.
**Verificar:** ejecutar una acción → la tabla refleja el cambio.

## T7 — Verificación final + commit `[criterio de hecho]`
- `cargo build --release -p peers-desktop` exit 0.
- `cargo build --workspace` no rompe los otros crates.
- Test mac-use completo: las pantallas con datos, errores con banner, una acción funcionando.
- Commit atómico en castellano (sin Co-Authored-By).

## Notas
- Campos `datos.*` y `error_*` YA existen en `Datos` (verificado). No redefinir tipos (peers-core).
- Sin `.unwrap()` en producción.
- Si aparecen >5 sub-pasos inesperados en alguna tarea, parar y re-planear (regla de la skill).
