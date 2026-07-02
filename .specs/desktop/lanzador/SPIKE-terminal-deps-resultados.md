# Spike de deps — Terminal embebido (Fase 2, R7)

> Ejecutado por Jefferson (dev). SOLO MEDICIÓN, no integración (autorizado por Julio).
> Proyecto scratch aislado, fuera del worktree de Fase 1. Toolchain: cargo +1.95.0.

## Resultado (a) — peso en binario

| Métrica | Valor |
|---------|-------|
| Binario spike (alacritty_terminal linkado), release | **0.41 MB** |
| Mismo, con `strip` | **0.32 MB** |

El proyecto ya usa `strip = true` en release (perfil del proyecto), así que el coste real de código
que `alacritty_terminal` añade al binario ronda **~0.3 MB**. Marginal para una app GPUI.

## Resultado (b) — tiempo de build

| Métrica | Valor |
|---------|-------|
| Build limpio release del backend PTY puro | **10.2 s** (real), 28.4 s user (paralelo) |

Es el coste de compilar `alacritty_terminal` + sus 4 deps nuevas UNA vez (luego se cachea). No
mueve la aguja del build del proyecto de forma significativa.

## Resultado (c) — ¿release_channel arrastra medio Zed?

**NO.** `release_channel` depende SOLO de `gpui` (ya lo tenemos) + `semver`. Es **ligero y acotable**.
El global `AppVersion::global(cx)` que `TerminalBuilder::new` exige se puede proveer con eso, sin
arrastrar el resto de Zed.

⚠️ **Matiz crítico:** el crate `crates/terminal` de Zed (la vía **A** de la RFC) SÍ arrastra Zed
pesado: `settings`, `theme`, `theme_settings`, `task`, `util` — crates internos con sus árboles.
Por eso la medición se hizo sobre `alacritty_terminal` DIRECTO (vía **B**), que evita todo eso.

## El número que importa — coste incremental REAL

- El proyecto ya tiene **846 deps** en `Cargo.lock`.
- `alacritty_terminal` arrastra **33 deps transitivas**, pero **29 ya están** en el árbol
  (vía gpui/tokio/reqwest/serde…).
- **Deps VERDADERAMENTE NUEVAS: solo 4.**

| Dep nueva | Qué es | Peso |
|-----------|--------|------|
| `alacritty_terminal` | backend PTY + rejilla de celdas (lo que queremos) | el grueso |
| `vte` | parser de escapes ANSI | pequeño, Rust puro |
| `rustix-openpty` | `openpty` seguro (rustix ya está en el árbol) | trivial |
| `cursor-icon` | enum de formas de cursor | trivial |

**Ninguna arrastra C/cmake/bindgen** — todas Rust puro. No rompe el criterio de binario portable
por la vía de "arrastrar toolchains nativas".

## Recomendación de dev (Max decide)

- La vía **B (alacritty_terminal directo)** cuesta **4 deps nuevas, ~0.3 MB, Rust puro**. Es un
  coste bajo y honesto — NO es "arrastrar medio Zed".
- La vía **A (crate `terminal` de Zed)** da más gratis (input/IME/scroll/hyperlinks de
  `terminal_view`) pero arrastra `settings/theme/task/util` de Zed → mucho más peso. Solo vale si el
  ahorro de reimplementar input/scroll compensa ese peso.
- El principio #1 de la VISIÓN ("zero deps externas") se respeta MÁS con B que con A. Pero B implica
  reimplementar el render de rejilla + input en GPUI (semanas). Trade-off esfuerzo vs peso.

**Para Max:** ¿0.3 MB + 4 deps Rust-puro por un terminal embebido real (B, con curro de render), o
preferimos el fallback ligero (R7.2 display-only / Terminal.app externa vía `open_with_system`, cero
deps)? Los números dicen que la dep NO es cara; el coste real de B es el ESFUERZO de render, no el peso.
