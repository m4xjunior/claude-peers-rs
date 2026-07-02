# Fundamentos de GPUI — referencia para la app `peers-desktop`

> ⬆ [[_MOC|Mapa de la arquitectura corporativa]]
>
> Fecha: 2026-07-02. Estado: **REFERENCIA** (no es una RFC; es el marco técnico que sostiene
> todas las RFCs de la app desktop y de la arquitectura corporativa).
> Verificado contra: fuentes primarias de GPUI (README + docs de contextos + blog de ownership de
> Zed) y el rev **FIJADO** por el proyecto en `crates/peers-desktop/Cargo.toml`
> (`gpui @ rev 1d217ee39d381ac101b7cf49d3d22451ac1093fe`, `gpui-component` de longbridge).
> Complementa la §11 de [[rfc/../desktop/lanzador/RFC-lanzador|RFC Lanzador]] (terminal/PTY/file picker,
> ya verificados contra el checkout fijado).

---

## 0. Por qué este documento

Max pidió los **fundamentos de GPUI, sus bibliotecas y su API** antes de estructurar la arquitectura
de negocio. La razón es concreta: **la arquitectura corporativa (proyectos, roles, agentes, delegación)
se materializa como UI en `peers-desktop`, y GPUI impone un modelo de datos y de concurrencia muy
particular**. Si el modelo de negocio no encaja en cómo GPUI posee el estado y ejecuta tareas asíncronas,
las RFCs prometen cosas que el framework no permite (y el proyecto ya sufrió un crash SIGABRT justo por
saltarse la regla de concurrencia — ver `STATE.md`). Este documento fija ese marco de una vez para que
todas las RFCs lo referencien en vez de re-derivarlo.

**Regla de oro del proyecto (heredada, no negociable):** la red y el trabajo pesado van SIEMPRE en
`cx.background_executor()`/`background_spawn`, y el `RwLock`/`borrow` de un estado NUNCA cruza un `.await`.
Todo lo demás de este documento se apoya en eso.

---

## 1. Qué es GPUI

GPUI es el framework de UI **acelerado por GPU** que Zed construyó para sí mismo, en Rust. No es
web (no hay DOM ni CSS reales): es retained-mode con render propio, layout con **Taffy** (motor flexbox)
y un modelo de propiedad de estado pensado para convivir con el borrow-checker de Rust. Se distribuye
como crate de git (no hay release en crates.io); por eso el proyecto lo fija por `rev` exacto.

El proyecto NO usa GPUI "pelado": usa **`gpui-component`** (kit de componentes de longbridge: `Input`,
`Select`, `Button`, `Switch`, `Tabs`, `Dialog`/`Sheet`/`Modal`, `Table`, `Tooltip`, `Notification`,
`Badge`, `Resizable`, `Scrollable`, `Breadcrumb`…) sobre GPUI. La app pinta con el kit + helpers propios
del tema **Ethos** (`crates/peers-desktop/src/tema.rs`).

### Las tres "capas" (registros) de GPUI

GPUI ofrece tres formas de trabajar, de más alto a más bajo nivel:

1. **Entidades (estado):** `Entity<T>` — el backbone del estado y de la comunicación entre partes.
2. **Vistas declarativas (`Render` + `div` + estilo tipo Tailwind):** una **vista es una entidad que
   implementa `Render`**; construye un árbol de elementos con `div()` y modificadores de estilo
   (flexbox). Es el 95% de lo que escribimos en `peers-desktop`.
3. **Elementos imperativos (`Element` trait):** control total de layout/paint/eventos, para listas
   virtualizadas o editores. Rara vez lo tocaremos (el kit ya lo hace por nosotros).

---

## 2. El modelo de propiedad (lo que hay que interiorizar)

Rust odia los grafos de objetos mutables compartidos. GPUI lo resuelve con una idea única:

> **Un solo objeto raíz, `App`, es dueño de TODO el estado de la aplicación.** Cuando creas un modelo o
> una vista (juntos: **entidades**), le cedes la propiedad a la `App`. Tú te quedas con un **handle**.

- **`Entity<T>`** es un handle tipo `Rc`: un tag de tipo en tiempo de compilación + un contador de
  referencias al objeto real, que vive dentro de la `App`. **El handle NO da acceso directo al estado**:
  para leer o mutar necesitas una referencia a `App`/`Context` viva.
- **`Context` (`cx`)** es la puerta a todos los servicios de GPUI: crear entidades, actualizarlas,
  abrir ventanas, lanzar tareas asíncronas, suscribirse a eventos, acceder a globals, al FS, al portapapeles.
  Casi toda función de GPUI recibe un `cx`.

### Los tres patrones de acceso a estado

```rust
// 1) Crear: registra la entidad en la App y devuelve el handle.
let peer_panel = cx.new(|cx| PanelPeers::new(cx));

// 2) Mutar: "arrienda" el estado a la App durante el closure (acceso &mut exclusivo).
peer_panel.update(cx, |panel, cx| {
    panel.instancias = nuevas;
    cx.notify();               // avisa a los observadores → re-render
});

// 3) Leer: acceso inmutable.
let n = peer_panel.read(cx).instancias.len();
```

**La restricción crucial:** el acceso al estado exige un `cx` vivo. **No puedes guardar una referencia
al estado y cruzarla por un `.await`, ni usarla fuera del closure.** Dentro de una tarea asíncrona vuelves
a "entrar" al estado con `entity.update(cx, …)` cuando ya tienes el resultado. Esto es exactamente lo que
el proyecto codifica en `cliente.rs` y lo que, al violarse, produjo el SIGABRT.

### Efectos diferidos (por qué es seguro)

GPUI **encola** los efectos secundarios (notify, emit) durante un `update` y los procesa al terminar
("run-to-completion sin reentrada"). Por eso puedes llamar `cx.notify()` en medio de una mutación sin
disparar un re-render reentrante en mitad de tu propio código.

---

## 3. Vistas y render

Una **vista** = `Entity<T>` donde `T: Render`. Al inicio de cada frame GPUI llama `render` sobre la vista
raíz de la ventana y reconstruye el árbol de elementos.

```rust
impl Render for PanelPeers {
    fn render(&mut self, _win: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex().flex_col().gap_2()
            .bg(TINTA2)                       // helpers de color del tema Ethos
            .p_4()
            .child(self.cabecera(cx))
            .children(self.instancias.iter().map(|i| self.fila(i, cx)))
    }
}
```

- **`div()`** es el elemento de composición primario; los modificadores (`.flex()`, `.gap_2()`, `.bg()`,
  `.p_4()`, `.rounded_lg()`, `.border_1()`…) son el equivalente a utilidades Tailwind, resueltos por Taffy.
- El render es **declarativo y se re-ejecuta**: no mutas nodos, describes el árbol para el estado actual.
  Cambiar el estado + `cx.notify()` es lo que provoca el próximo render.
- **`Window`** representa la ventana (tiene su propio `TaffyLayoutEngine` y su `Scene`). Se crea con
  `cx.open_window(...)`.

### Eventos e interacción

- **Interacción por elemento:** `.on_click(cx.listener(|this, ev, win, cx| { … }))`, `.on_mouse_down`,
  hover, etc. El `cx.listener` te devuelve al `&mut self` de la vista con un `cx` vivo.
- **Acciones + teclado:** structs de acción del usuario (`#[derive(Action)]`) enlazadas a teclas
  (keybindings). Es el mecanismo para la paridad de teclado con la TUI que piden varias RFCs
  (p.ej. peers-18: ↑/↓/Enter/`m`/`k`/`r`).
- **Focus:** anillo de foco y orden de tabulación (base de la accesibilidad transversal del backlog).

---

## 4. Comunicación entre entidades (la base de un panel "vivo")

Dos sistemas, ambos vía `cx`:

| Mecanismo | API | Para qué |
|-----------|-----|----------|
| **Observación** | `cx.observe(&otra, |this, otra, cx| …)` + `cx.notify()` | "algo cambió en esa entidad, revísala" (sin payload) |
| **Eventos tipados** | `cx.subscribe(&otra, |this, otra, ev, cx| …)` + `cx.emit(Ev)` (con `impl EventEmitter<Ev>`) | mensajes con datos entre entidades desacopladas |

Esto es lo que permite que, p.ej., el **Lanzador** (crear sesión) notifique al **panel de Peers** que
hay una sesión nueva, o que el hilo del **chat privado** avise a la vista cuando llega respuesta —
sin acoplar ambas vistas.

---

## 5. Concurrencia y asincronía (la trampa del proyecto)

GPUI tiene su propio executor (basado en **smol**), **no** tokio. Hay dos formas de lanzar trabajo:

```rust
// Tarea ligada a la UI: vuelve al hilo de UI para tocar estado con `cx`.
cx.spawn(async move |esta, cx| {
    let datos = /* … await … */;
    esta.update(cx, |this, cx| { this.datos = datos; cx.notify(); }).ok();
}).detach();

// Trabajo pesado / bloqueante fuera del hilo de UI.
cx.background_spawn(async move { calcular_pesado() }).await
```

**El problema real del proyecto (documentado en `STATE.md`, crash SIGABRT):** el cliente HTTP es
`reqwest` (async sobre **tokio**), pero los `spawn` de GPUI corren sobre **smol** → hacer `.await` de
reqwest dentro de `cx.spawn` intenta usar `tokio::Handle::current()` donde no hay reactor tokio → *"no
reactor running"* → `abort()`. La solución que el proyecto ya adoptó (`cliente.rs:75`):

- La app monta **su propio runtime tokio** para la red.
- Toda llamada HTTP se envuelve en un helper (`bloquear_en`) que ejecuta el future de reqwest **en ese
  runtime tokio**, dentro de `cx.background_executor()`/`background_spawn` — **nunca** con un `.await`
  directo de reqwest bajo un `cx.spawn`.
- Al volver con el resultado, se re-entra al estado con `entity.update(cx, …)` + `cx.notify()`.

> **Regla para TODAS las RFCs (repetida en cada constraint):** ninguna vista hace `.await` de red
> directamente en el hilo de UI; el patrón es *background → resultado → `update` + `notify`*. Cualquier
> feature nueva (chat privado, tablero de proyecto, organigrama en vivo) hereda esta regla.

---

## 6. Servicios de plataforma que ya usamos o usaremos (API verificada)

Cero dependencias nuevas: son nativos de GPUI. Verificados contra el rev fijado (ver también §11 del
RFC Lanzador).

| Servicio | API | Uso corporativo |
|----------|-----|-----------------|
| **File picker nativo** | `cx.prompt_for_paths(PathPromptOptions { files, directories, multiple })` → `oneshot::Receiver<Result<Option<Vec<PathBuf>>>>` | elegir la **carpeta de un proyecto** (RFC Proyectos, RFC Lanzador R1) |
| **Revelar / abrir** | `cx.reveal_path(&p)` (Finder), `cx.open_with_system(&p)` | abrir el workspace del proyecto |
| **Diálogo modal** | `cx.prompt(...)` / `Dialog` del kit | confirmaciones destructivas (kick, cerrar proyecto) |
| **Terminal / PTY** | crates `terminal` + `terminal_view` de Zed (mismo rev) sobre `alacritty_terminal`; `TerminalBuilder::new(...)` async en `background_executor` | consola embebida por proyecto/sesión (RFC Lanzador §11.1) |
| **Portapapeles** | `cx.write_to_clipboard` / `read_from_clipboard` | "copiar comando", copiar id de tarea/peer |
| **Globals** | `cx.global::<T>()` / `set_global` | tema Ethos como Theme global del kit (`tema::aplicar_tema_kit`, ver `STATE.md`) |
| **Persistencia** | fuera de GPUI: `toml` + `dirs` → `~/.config/claude-peers/config.toml` | perfiles de lanzamiento, proyectos, plantillas de rol |

---

## 7. Cómo se organiza HOY `peers-desktop` (para no reinventar)

```
crates/peers-desktop/src/
  main.rs        · arranque, ventana, aplica tema Ethos al kit (Theme global)
  app.rs         · AppDesktop: vista raíz, estado de pantallas, navegación, acciones
  cliente.rs     · cliente HTTP del broker (runtime tokio propio + bloquear_en) — la capa anti-SIGABRT
  config.rs      · lee/escribe config.toml (compartido con la TUI)
  tema.rs        · paleta y helpers Ethos: superficie_card, eyebrow, titulo, chip_estado,
                   boton_primario/secundario, texto_terciario, fondo_app, fila_seleccionable, banner_error
  vista/         · una vista por pestaña: peers, tareas, alertas, trazabilidad, redis, broker,
                   config, jornada, acceso (+ las nuevas: lanzador, proyectos, organigrama…)
```

Los DTOs del protocolo viven **solo** en `peers-core` (regla dura: la desktop NUNCA redefine DTOs;
los pinta). Todo lo corporativo nuevo (entidades de empresa, proyecto, rol) debe seguir la misma regla:
**el tipo se define una vez en `peers-core` y la desktop lo consume.**

---

## 8. Implicaciones para la arquitectura corporativa (el puente)

Estas son las consecuencias directas de GPUI sobre el modelo de negocio que se diseña en los documentos
`01`/`02` y las RFCs:

1. **"Proyecto", "Rol", "Agente" son entidades (`Entity<T>`) en la app + DTOs en `peers-core`.** El árbol
   de organigrama, el tablero de un proyecto y la lista de agentes son vistas (`Render`) que observan esas
   entidades. Cambiar un agente → `update` + `notify` → re-render. No hace falta framework de estado extra
   (nada de Redux/signals): el modelo de entidades de GPUI YA es el store reactivo.
2. **Lanzar un agente = lanzar un proceso (`claude …`) en un PTY** (RFC Lanzador). El "empleado" es una
   sesión de Claude Code; su "contrato" (rol/system prompt) se inyecta con `--append-system-prompt`. Todo
   esto ya está verificado como factible en el rev fijado.
3. **El estado "vivo" del equipo (quién trabaja, en qué, su jornada) viene del broker por HTTP**, se trae
   en background y se refleja con `update`+`notify`. La app es un **espejo reactivo** del broker, no la
   fuente de verdad (la verdad del tiempo/jornada la timbra el broker — regla sagrada).
4. **Toda acción corporativa que muta (contratar, asignar, delegar, bloquear comunicación, cerrar proyecto)
   es una llamada HTTP al broker** en background + registro en la bitácora (`registro-acciones`). La UI no
   inventa estado: dispara la acción y re-lee.
5. **Degradación obligatoria:** broker offline, PTY no disponible, SSH caído → banner Ethos, la app sigue
   viva. Sin `.unwrap()`/`.expect()` en producción. Aplica a cada pantalla corporativa nueva.

---

## 9. Glosario rápido GPUI ↔ concepto corporativo

| GPUI | En la arquitectura corporativa |
|------|-------------------------------|
| `App` (dueña de todo el estado) | la propia app-empresa: el "edificio" donde vive todo |
| `Entity<T>` (handle Rc) | referencia a una entidad de negocio (proyecto, agente, rol) |
| Vista (`Render`) | una pantalla: organigrama, tablero de proyecto, ficha de agente |
| `observe`/`notify` | "el equipo cambió, repinta el organigrama" |
| `subscribe`/`emit` | "se lanzó una sesión / llegó respuesta del chat privado" |
| `background_executor` + tokio propio | el canal por donde la app habla con el broker (RRHH central) |
| PTY (`TerminalBuilder`) | el "puesto de trabajo" real donde corre el empleado (`claude`) |
| broker (HTTP, Redis) | RRHH + reloj de fichaje + oficina de correos + verdad del estado |

---

## 10. Fuentes

- GPUI README y docs de contextos (`zed-industries/zed/crates/gpui`), blog *"Ownership and data flow in
  GPUI"* (zed.dev) — modelo `App`/`Entity`/`Context`, `cx.new`/`update`/`read`, `observe`/`notify`,
  `subscribe`/`emit`, efectos diferidos, la regla de no cruzar `.await` con referencias de estado.
- Rev **fijado** del proyecto: `gpui @ 1d217ee…` (`crates/peers-desktop/Cargo.toml`); crates `terminal`/
  `terminal_view`, `prompt_for_paths`, `TerminalBuilder` verificados en §11 del [[../desktop/lanzador/RFC-lanzador|RFC Lanzador]].
- Código del proyecto: `crates/peers-desktop/src/{main.rs,app.rs,cliente.rs,tema.rs}` (patrón anti-SIGABRT,
  helpers Ethos, tema global del kit) y `crates/peers-core/src/lib.rs` (DTOs).

---
#referencia #gpui #peers-desktop #fundamentos #empresa
