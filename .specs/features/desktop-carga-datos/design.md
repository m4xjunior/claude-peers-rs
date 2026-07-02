# Design — peers-desktop: carga de datos

## Decisiones de arquitectura

### D1 — Un método `cargar_<pantalla>` por pantalla, patrón `cx.spawn` (verificado)
Reusar el patrón ya funcional de `descartar_alerta` (app.rs:267). Cada carga:
```rust
fn cargar_peers(&mut self, cx: &mut Context<Self>) {
    let cliente = self.cliente.clone();
    cx.spawn(async move |esta, cx| {
        let r = cliente.listar_instancias().await;
        let _ = esta.update(cx, |esta, cx| {
            match r {
                Ok(lista) => { esta.datos.peers = lista; esta.datos.error_peers = None; }
                Err(e)    => { esta.datos.error_peers = Some(e); }
            }
            cx.notify();
        });
    }).detach();
}
```
Los campos `datos.peers`, `datos.error_peers`, etc. YA existen en `Datos`. Solo faltan los métodos
que los pueblan.

### D2 — Refresco periódico único, de la pantalla activa (R5, verificado vía context7)
Un solo `cx.spawn` con loop lanzado al construir la app (en `AppDesktop::new` o al abrir la
ventana), que cada ~2s llama a `cargar_<pantalla activa>`:
```rust
cx.spawn(async move |esta, cx| {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if esta.update(cx, |esta, cx| esta.cargar_pantalla_activa(cx)).is_err() { break; }
    }
}).detach();
```
`cargar_pantalla_activa` hace `match self.pantalla { Peers => self.cargar_peers(cx), ... }`.
Ventaja: un solo timer, no 9. Solo refresca lo que el usuario mira → sin carga innecesaria al broker.
El `break` al fallar el `update` corta el loop si la entidad muere (cierre de ventana).

### D3 — Carga al cambiar de pantalla (R2)
En el handler del click del sidebar (donde se hace `self.pantalla = nueva`), añadir
`self.cargar_pantalla_activa(cx)` inmediatamente → el usuario ve datos sin esperar los 2s del timer.

### D4 — Carga inicial (R1)
Llamar `cargar_pantalla_activa(cx)` una vez al arrancar (mismo punto donde se lanza el timer D2).

### D5 — Errores (R7/R8)
Reusar los campos `error_<pantalla>` de `Datos` (ya existen). Cada `cargar_*` los setea en `Err`
y los limpia (`None`) en `Ok`. Las vistas ya tienen (o tendrán) el banner que los pinta. Ningún
`.unwrap()`: el `Result` del cliente se maneja con match; el `esta.update` con `let _ =`.

### D6 — Acciones (R9/R10)
Las acciones que muten el broker (asignar tarea, purgar, guardar config...) siguen el patrón de
`descartar_alerta`: `cx.spawn` → acción → recargar la pantalla afectada. Verificar cuáles ya están
cableadas (descartar_alerta lo está) y cablear las que no.

## Componentes tocados
- `crates/peers-desktop/src/app.rs` — añadir los métodos `cargar_*` + `cargar_pantalla_activa` +
  el timer + la carga inicial + carga al cambiar de pantalla. (el grueso del trabajo)
- `crates/peers-desktop/src/cliente.rs` — solo si falta algún método (jornada, historial ya deben
  existir; verificar). No romper los existentes.
- `crates/peers-desktop/src/vista/*.rs` — solo si alguna vista no pinta el banner de error o no
  lee bien el campo de datos. La mayoría no se toca (ya leen `datos.*`).

## Riesgos
- **Cargo/toolchain:** requiere 1.95.0 (ya fijado en rust-toolchain.toml). Verificado.
- **tokio en GPUI:** la app ya usa reqwest async; confirmar que hay runtime tokio disponible en el
  `cx.spawn` (GPUI usa su propio executor; `tokio::time::sleep` necesita el runtime tokio — si no,
  usar el timer de GPUI `cx.background_executor().timer(dur)`). VERIFICAR en implementación: si
  `tokio::time::sleep` panica por "no reactor running", cambiar a `smol::Timer` o al timer de GPUI.
- **Frecuencia:** 2s por defecto; si genera mucho tráfico al broker con muchos peers, subir a 3-5s.
