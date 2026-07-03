//! Traducción de `gpui::Keystroke` a secuencias de escape ANSI para el PTY embebido — RFC Lanzador
//! **Fase 2**, Zona B (R7). Adaptado de `crates/terminal/src/mappings/keys.rs` de Zed (rev
//! `1d217ee`, MISMO rev fijado en `Cargo.toml`): esa tabla es una función pura autocontenida (sólo
//! depende de `gpui::Keystroke` + el bitflag de modo del terminal), sin arrastrar `terminal_view`
//! ni `settings`/`theme`/`task` — por eso se porta entera en vez de reimplementarla a mano (el
//! DISENO-FASE2 advertía que reimplementar input/mapeo de teclas es "semanas de trabajo y
//! superficie de bugs que la vía (A) ya resolvió"; portar la tabla YA VERIFICADA de Zed evita
//! exactamente ese riesgo sin arrastrar sus deps pesadas).
//!
//! DIFERENCIA con el original: usa `alacritty_terminal::term::TermMode` (el bitflag NATIVO del
//! crate, que YA incluye `APP_CURSOR`) en vez del `Modes` propio que definía `terminal_view` — no
//! hace falta reinventar un bitflag paralelo cuando el que necesitamos ya existe en la dependencia
//! que estamos consumiendo directamente (vía B).

use std::borrow::Cow;

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

#[derive(Debug, PartialEq, Eq)]
enum ModificadoresTerminal {
    Ninguno,
    Alt,
    Ctrl,
    Shift,
    CtrlShift,
    Otro,
}

impl ModificadoresTerminal {
    fn de(ks: &Keystroke) -> Self {
        match (
            ks.modifiers.alt,
            ks.modifiers.control,
            ks.modifiers.shift,
            ks.modifiers.platform,
        ) {
            (false, false, false, false) => Self::Ninguno,
            (true, false, false, false) => Self::Alt,
            (false, true, false, false) => Self::Ctrl,
            (false, false, true, false) => Self::Shift,
            (false, true, true, false) => Self::CtrlShift,
            _ => Self::Otro,
        }
    }

    fn alguno(&self) -> bool {
        !matches!(self, Self::Ninguno)
    }
}

/// Traduce una pulsación de teclado a la secuencia de bytes que espera el PTY, o `None` si la
/// pulsación no tiene equivalente ANSI (el caller entonces usa `keystroke.key_char` como texto
/// imprimible plano — ver `vista::lanzador`). `modo` es el `TermMode` actual del `Term` (afecta
/// flechas/home/end en modo aplicación, R7 básico) y `option_como_meta` replica la opción de Zed de
/// tratar Option/Alt como Meta en macOS (aquí fija en `false` — v1 no expone esa preferencia).
pub fn a_secuencia_esc(
    keystroke: &Keystroke,
    modo: TermMode,
    option_como_meta: bool,
) -> Option<Cow<'static, str>> {
    let modificadores = ModificadoresTerminal::de(keystroke);

    // Bindings manuales, incluyendo combinaciones con modificadores. Idéntico al catálogo de Zed
    // (verificado contra xterm ctlseqs) — no se reinventa, se porta.
    let manual: Option<&'static str> = match (keystroke.key.as_ref(), &modificadores) {
        ("tab", ModificadoresTerminal::Ninguno) => Some("\x09"),
        ("escape", ModificadoresTerminal::Ninguno) => Some("\x1b"),
        ("enter", ModificadoresTerminal::Ninguno) => Some("\x0d"),
        ("enter", ModificadoresTerminal::Shift) => Some("\x0a"),
        ("enter", ModificadoresTerminal::Alt) => Some("\x1b\x0d"),
        ("backspace", ModificadoresTerminal::Ninguno) => Some("\x7f"),
        ("tab", ModificadoresTerminal::Shift) => Some("\x1b[Z"),
        ("backspace", ModificadoresTerminal::Ctrl) => Some("\x08"),
        ("backspace", ModificadoresTerminal::Alt) => Some("\x1b\x7f"),
        ("backspace", ModificadoresTerminal::Shift) => Some("\x7f"),
        ("space", ModificadoresTerminal::Ctrl) => Some("\x00"),
        ("home", ModificadoresTerminal::Ninguno) if modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1bOH")
        }
        ("home", ModificadoresTerminal::Ninguno) if !modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1b[H")
        }
        ("end", ModificadoresTerminal::Ninguno) if modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1bOF")
        }
        ("end", ModificadoresTerminal::Ninguno) if !modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1b[F")
        }
        ("up", ModificadoresTerminal::Ninguno) if modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1bOA")
        }
        ("up", ModificadoresTerminal::Ninguno) if !modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1b[A")
        }
        ("down", ModificadoresTerminal::Ninguno) if modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1bOB")
        }
        ("down", ModificadoresTerminal::Ninguno) if !modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1b[B")
        }
        ("right", ModificadoresTerminal::Ninguno) if modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1bOC")
        }
        ("right", ModificadoresTerminal::Ninguno) if !modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1b[C")
        }
        ("left", ModificadoresTerminal::Ninguno) if modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1bOD")
        }
        ("left", ModificadoresTerminal::Ninguno) if !modo.contains(TermMode::APP_CURSOR) => {
            Some("\x1b[D")
        }
        ("back", ModificadoresTerminal::Ninguno) => Some("\x7f"),
        ("insert", ModificadoresTerminal::Ninguno) => Some("\x1b[2~"),
        ("delete", ModificadoresTerminal::Ninguno) => Some("\x1b[3~"),
        ("pageup", ModificadoresTerminal::Ninguno) => Some("\x1b[5~"),
        ("pagedown", ModificadoresTerminal::Ninguno) => Some("\x1b[6~"),
        ("f1", ModificadoresTerminal::Ninguno) => Some("\x1bOP"),
        ("f2", ModificadoresTerminal::Ninguno) => Some("\x1bOQ"),
        ("f3", ModificadoresTerminal::Ninguno) => Some("\x1bOR"),
        ("f4", ModificadoresTerminal::Ninguno) => Some("\x1bOS"),
        ("f5", ModificadoresTerminal::Ninguno) => Some("\x1b[15~"),
        ("f6", ModificadoresTerminal::Ninguno) => Some("\x1b[17~"),
        ("f7", ModificadoresTerminal::Ninguno) => Some("\x1b[18~"),
        ("f8", ModificadoresTerminal::Ninguno) => Some("\x1b[19~"),
        ("f9", ModificadoresTerminal::Ninguno) => Some("\x1b[20~"),
        ("f10", ModificadoresTerminal::Ninguno) => Some("\x1b[21~"),
        ("f11", ModificadoresTerminal::Ninguno) => Some("\x1b[23~"),
        ("f12", ModificadoresTerminal::Ninguno) => Some("\x1b[24~"),
        // Combinaciones Ctrl+letra → notación caret (C0 0x01-0x1a). Ctrl+Shift+letra produce el
        // MISMO código (el shift sólo cambia mayúscula/minúscula visual, no el control code).
        ("a", ModificadoresTerminal::Ctrl) | ("A", ModificadoresTerminal::CtrlShift) => {
            Some("\x01")
        }
        ("b", ModificadoresTerminal::Ctrl) | ("B", ModificadoresTerminal::CtrlShift) => {
            Some("\x02")
        }
        ("c", ModificadoresTerminal::Ctrl) | ("C", ModificadoresTerminal::CtrlShift) => {
            Some("\x03")
        }
        ("d", ModificadoresTerminal::Ctrl) | ("D", ModificadoresTerminal::CtrlShift) => {
            Some("\x04")
        }
        ("e", ModificadoresTerminal::Ctrl) | ("E", ModificadoresTerminal::CtrlShift) => {
            Some("\x05")
        }
        ("f", ModificadoresTerminal::Ctrl) | ("F", ModificadoresTerminal::CtrlShift) => {
            Some("\x06")
        }
        ("g", ModificadoresTerminal::Ctrl) | ("G", ModificadoresTerminal::CtrlShift) => {
            Some("\x07")
        }
        ("h", ModificadoresTerminal::Ctrl) | ("H", ModificadoresTerminal::CtrlShift) => {
            Some("\x08")
        }
        ("i", ModificadoresTerminal::Ctrl) | ("I", ModificadoresTerminal::CtrlShift) => {
            Some("\x09")
        }
        ("j", ModificadoresTerminal::Ctrl) | ("J", ModificadoresTerminal::CtrlShift) => {
            Some("\x0a")
        }
        ("k", ModificadoresTerminal::Ctrl) | ("K", ModificadoresTerminal::CtrlShift) => {
            Some("\x0b")
        }
        ("l", ModificadoresTerminal::Ctrl) | ("L", ModificadoresTerminal::CtrlShift) => {
            Some("\x0c")
        }
        ("m", ModificadoresTerminal::Ctrl) | ("M", ModificadoresTerminal::CtrlShift) => {
            Some("\x0d")
        }
        ("n", ModificadoresTerminal::Ctrl) | ("N", ModificadoresTerminal::CtrlShift) => {
            Some("\x0e")
        }
        ("o", ModificadoresTerminal::Ctrl) | ("O", ModificadoresTerminal::CtrlShift) => {
            Some("\x0f")
        }
        ("p", ModificadoresTerminal::Ctrl) | ("P", ModificadoresTerminal::CtrlShift) => {
            Some("\x10")
        }
        ("q", ModificadoresTerminal::Ctrl) | ("Q", ModificadoresTerminal::CtrlShift) => {
            Some("\x11")
        }
        ("r", ModificadoresTerminal::Ctrl) | ("R", ModificadoresTerminal::CtrlShift) => {
            Some("\x12")
        }
        ("s", ModificadoresTerminal::Ctrl) | ("S", ModificadoresTerminal::CtrlShift) => {
            Some("\x13")
        }
        ("t", ModificadoresTerminal::Ctrl) | ("T", ModificadoresTerminal::CtrlShift) => {
            Some("\x14")
        }
        ("u", ModificadoresTerminal::Ctrl) | ("U", ModificadoresTerminal::CtrlShift) => {
            Some("\x15")
        }
        ("v", ModificadoresTerminal::Ctrl) | ("V", ModificadoresTerminal::CtrlShift) => {
            Some("\x16")
        }
        ("w", ModificadoresTerminal::Ctrl) | ("W", ModificadoresTerminal::CtrlShift) => {
            Some("\x17")
        }
        ("x", ModificadoresTerminal::Ctrl) | ("X", ModificadoresTerminal::CtrlShift) => {
            Some("\x18")
        }
        ("y", ModificadoresTerminal::Ctrl) | ("Y", ModificadoresTerminal::CtrlShift) => {
            Some("\x19")
        }
        ("z", ModificadoresTerminal::Ctrl) | ("Z", ModificadoresTerminal::CtrlShift) => {
            Some("\x1a")
        }
        ("@", ModificadoresTerminal::Ctrl) => Some("\x00"),
        ("[", ModificadoresTerminal::Ctrl) => Some("\x1b"),
        ("\\", ModificadoresTerminal::Ctrl) => Some("\x1c"),
        ("]", ModificadoresTerminal::Ctrl) => Some("\x1d"),
        ("^", ModificadoresTerminal::Ctrl) => Some("\x1e"),
        ("_", ModificadoresTerminal::Ctrl) => Some("\x1f"),
        ("?", ModificadoresTerminal::Ctrl) => Some("\x7f"),
        _ => None,
    };
    if let Some(esc) = manual {
        return Some(Cow::Borrowed(esc));
    }

    // Bindings automáticos con código de modificador xterm (flechas/F-keys con Shift/Alt/Ctrl que
    // no están en la tabla manual de arriba).
    if modificadores.alguno() {
        let codigo = codigo_modificador(keystroke);
        let con_modificador = match keystroke.key.as_ref() {
            "up" => Some(format!("\x1b[1;{codigo}A")),
            "down" => Some(format!("\x1b[1;{codigo}B")),
            "right" => Some(format!("\x1b[1;{codigo}C")),
            "left" => Some(format!("\x1b[1;{codigo}D")),
            "f1" => Some(format!("\x1b[1;{codigo}P")),
            "f2" => Some(format!("\x1b[1;{codigo}Q")),
            "f3" => Some(format!("\x1b[1;{codigo}R")),
            "f4" => Some(format!("\x1b[1;{codigo}S")),
            "f5" => Some(format!("\x1b[15;{codigo}~")),
            "f6" => Some(format!("\x1b[17;{codigo}~")),
            "f7" => Some(format!("\x1b[18;{codigo}~")),
            "f8" => Some(format!("\x1b[19;{codigo}~")),
            "f9" => Some(format!("\x1b[20;{codigo}~")),
            "f10" => Some(format!("\x1b[21;{codigo}~")),
            "f11" => Some(format!("\x1b[23;{codigo}~")),
            "f12" => Some(format!("\x1b[24;{codigo}~")),
            "insert" => Some(format!("\x1b[2;{codigo}~")),
            "pageup" => Some(format!("\x1b[5;{codigo}~")),
            "pagedown" => Some(format!("\x1b[6;{codigo}~")),
            "end" => Some(format!("\x1b[1;{codigo}F")),
            "home" => Some(format!("\x1b[1;{codigo}H")),
            _ => None,
        };
        if let Some(esc) = con_modificador {
            return Some(Cow::Owned(esc));
        }
    }

    // Alt/Option como Meta (macOS: sólo si `option_como_meta` — replica el default de Zed de NO
    // interceptar Option para dejar pasar acentos/símbolos compuestos del teclado del sistema).
    if !cfg!(target_os = "macos") || option_como_meta {
        let alt_minuscula_ascii =
            modificadores == ModificadoresTerminal::Alt && keystroke.key.is_ascii();
        let alt_mayuscula_ascii =
            keystroke.modifiers.alt && keystroke.modifiers.shift && keystroke.key.is_ascii();
        if alt_minuscula_ascii || alt_mayuscula_ascii {
            let tecla = if alt_mayuscula_ascii {
                keystroke.key.to_ascii_uppercase()
            } else {
                keystroke.key.clone()
            };
            return Some(Cow::Owned(format!("\x1b{tecla}")));
        }
    }

    None
}

/// Código de modificador xterm (2=Shift, 3=Alt, 4=Shift+Alt, 5=Ctrl, 6=Shift+Ctrl, 7=Alt+Ctrl,
/// 8=Shift+Alt+Ctrl). Ver https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-PC-Style-Function-Keys
fn codigo_modificador(keystroke: &Keystroke) -> u32 {
    let mut codigo = 0;
    if keystroke.modifiers.shift {
        codigo |= 1;
    }
    if keystroke.modifiers.alt {
        codigo |= 1 << 1;
    }
    if keystroke.modifiers.control {
        codigo |= 1 << 2;
    }
    codigo + 1
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    #[test]
    fn entradas_planas_sin_equivalente_no_generan_secuencia() {
        let ks = Keystroke {
            modifiers: Modifiers {
                control: false,
                alt: false,
                shift: false,
                platform: false,
                function: false,
            },
            key: "🖖🏻".to_string(),
            key_char: None,
        };
        assert_eq!(a_secuencia_esc(&ks, TermMode::NONE, false), None);
    }

    #[test]
    fn modo_aplicacion_cambia_flechas() {
        let app_cursor = TermMode::APP_CURSOR;
        let ninguno = TermMode::NONE;

        let up = Keystroke::parse("up").unwrap();
        let down = Keystroke::parse("down").unwrap();
        let left = Keystroke::parse("left").unwrap();
        let right = Keystroke::parse("right").unwrap();

        assert_eq!(a_secuencia_esc(&up, ninguno, false), Some("\x1b[A".into()));
        assert_eq!(a_secuencia_esc(&down, ninguno, false), Some("\x1b[B".into()));
        assert_eq!(a_secuencia_esc(&right, ninguno, false), Some("\x1b[C".into()));
        assert_eq!(a_secuencia_esc(&left, ninguno, false), Some("\x1b[D".into()));

        assert_eq!(a_secuencia_esc(&up, app_cursor, false), Some("\x1bOA".into()));
        assert_eq!(a_secuencia_esc(&down, app_cursor, false), Some("\x1bOB".into()));
        assert_eq!(
            a_secuencia_esc(&right, app_cursor, false),
            Some("\x1bOC".into())
        );
        assert_eq!(
            a_secuencia_esc(&left, app_cursor, false),
            Some("\x1bOD".into())
        );
    }

    #[test]
    fn shift_flechas_usa_codigo_de_modificador() {
        let ninguno = TermMode::NONE;
        let shift_up = Keystroke::parse("shift-up").unwrap();
        let shift_home = Keystroke::parse("shift-home").unwrap();
        assert_eq!(
            a_secuencia_esc(&shift_up, ninguno, false),
            Some("\x1b[1;2A".into())
        );
        assert_eq!(
            a_secuencia_esc(&shift_home, ninguno, false),
            Some("\x1b[1;2H".into())
        );
    }

    #[test]
    fn ctrl_shift_letra_igual_que_ctrl_mayuscula() {
        for (min, may) in ('a'..='z').zip('A'..='Z') {
            let modo = TermMode::APP_CURSOR;
            let ctrl_shift = a_secuencia_esc(
                &Keystroke::parse(&format!("ctrl-shift-{min}")).unwrap(),
                modo,
                false,
            );
            let ctrl_mayuscula = a_secuencia_esc(
                &Keystroke::parse(&format!("ctrl-{may}")).unwrap(),
                modo,
                false,
            );
            assert_eq!(ctrl_shift, ctrl_mayuscula, "letra {min}/{may}");
        }
    }

    #[test]
    fn shift_enter_es_salto_de_linea_enter_normal_es_cr() {
        let modo = TermMode::NONE;
        assert_eq!(
            a_secuencia_esc(&Keystroke::parse("shift-enter").unwrap(), modo, false),
            Some("\x0a".into())
        );
        assert_eq!(
            a_secuencia_esc(&Keystroke::parse("enter").unwrap(), modo, false),
            Some("\x0d".into())
        );
    }

    #[test]
    fn calculo_codigo_modificador() {
        assert_eq!(2, codigo_modificador(&Keystroke::parse("shift-a").unwrap()));
        assert_eq!(3, codigo_modificador(&Keystroke::parse("alt-a").unwrap()));
        assert_eq!(
            4,
            codigo_modificador(&Keystroke::parse("shift-alt-a").unwrap())
        );
        assert_eq!(5, codigo_modificador(&Keystroke::parse("ctrl-a").unwrap()));
        assert_eq!(
            6,
            codigo_modificador(&Keystroke::parse("shift-ctrl-a").unwrap())
        );
        assert_eq!(
            7,
            codigo_modificador(&Keystroke::parse("alt-ctrl-a").unwrap())
        );
        assert_eq!(
            8,
            codigo_modificador(&Keystroke::parse("shift-ctrl-alt-a").unwrap())
        );
    }
}
