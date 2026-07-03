//! Punto de entrada de peers-desktop — app nativa (GPUI) que porta la TUI de claude-peers-rs.
//!
//! INTENCIÓN: montar la ventana con el andamiaje mínimo que exige gpui-component:
//! `init(cx)` ANTES de tocar cualquier componente, y `Root` como PRIMER nivel de la ventana
//! (sin Root no funcionan diálogos/notificaciones del kit). El estado de la app vive en
//! `AppDesktop`; aquí sólo se abre la ventana y se delega.

mod app;
mod cliente;
mod config;
mod tema;
mod validacion;
mod vista;

use app::AppDesktop;
// `AppContext` aporta `cx.new(...)` (crear entidades); no está en un prelude auto-importado.
use gpui::{px, size, AppContext, WindowBounds, WindowOptions};
use gpui_component::Root;

fn main() {
    // `with_assets` inyecta iconos/fuentes del kit; sin ellos los componentes que usan iconos
    // (Sidebar, Button con icon, etc.) no renderizan su glifo.
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        // OBLIGATORIO antes de usar cualquier feature de gpui-component (tema, componentes…).
        gpui_component::init(cx);

        // El init del kit deja su Theme global en modo CLARO → los Input/Select del kit pintaban
        // fondo blanco sobre el Ethos oscuro (texto dorado sobre blanco, ilegible). Se registra la
        // paleta Ethos como tema del kit ANTES de abrir la ventana; ver tema::aplicar_tema_kit.
        tema::aplicar_tema_kit(cx);

        // Las opciones de ventana se calculan AQUÍ, con `cx: &mut App`, porque
        // `WindowBounds::centered` exige `&App` (no está disponible el `AsyncApp` del spawn).
        // Este es el patrón canónico de los ejemplos del kit (examples/input).
        let opciones = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.0), px(800.0)), cx)),
            titlebar: Some(titulo_ventana()),
            ..Default::default()
        };

        // La apertura de ventana se hace dentro de un spawn detach: es el patrón recomendado
        // por el kit para que la inicialización asíncrona del tema esté lista al abrir. Dentro del
        // `open_window`, el `cx` de la clausura ES `&mut App`, por eso `cx.new` funciona ahí.
        cx.spawn(async move |cx| {
            let resultado = cx.open_window(opciones, |window, cx| {
                let vista = cx.new(|cx| AppDesktop::nueva(window, cx));
                // El primer nivel DEBE ser Root: habilita dialogs/notifications del kit.
                // `vista` va sin `.into()`: `Entity<AppDesktop>: Into<AnyView>` es directo y poner
                // `.into()` deja el tipo destino ambiguo (E0283). `Root::new` acepta `impl Into<AnyView>`.
                cx.new(|cx| Root::new(vista, window, cx))
            });

            if let Err(e) = resultado {
                // Sin ventana no hay app; registrar y salir sin panic.
                eprintln!("no se pudo abrir la ventana: {e}");
            }
        })
        .detach();
    });
}

/// Título de la ventana como campo de `WindowOptions`. Se aísla en una función para no
/// arrastrar el `Some(...).into()` al literal del struct y mantener el `main` legible.
fn titulo_ventana() -> gpui::TitlebarOptions {
    gpui::TitlebarOptions {
        title: Some("claude-peers · desktop".into()),
        ..Default::default()
    }
}
