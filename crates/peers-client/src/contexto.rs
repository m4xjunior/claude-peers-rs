//! Detección del contexto de ejecución de la instancia: directorio, repo git, tty y
//! un resumen inicial. Todo con `Result`/`Option`; nada entra en pánico si git no existe.

use std::path::Path;
use std::process::Command;

/// Directorio de trabajo actual.
pub fn directorio_actual() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".into())
}

/// Raíz del repositorio git que contiene `directorio`, o None si no es un repo.
pub fn repo_git(directorio: &str) -> Option<String> {
    salida_git(directorio, &["rev-parse", "--show-toplevel"])
}

/// Repositorio GitHub ("owner/repo") derivado del remote `origin` del git_root.
///
/// INTENCIÓN: el client corre EN la máquina del peer, dentro del directorio de trabajo,
/// con gh/git autenticado — así que resolver el repo aquí es trivial y local. El broker
/// recibe ya el "owner/repo" y abre la issue en ESE repo (dinámico), sin GITHUB_REPO fijo.
///
/// Soporta los dos formatos de remote de GitHub:
///   - SSH:   git@github.com:owner/repo.git      → owner/repo
///   - HTTPS: https://github.com/owner/repo(.git) → owner/repo
/// Devuelve None si no hay remote, no es GitHub, o no se puede parsear (→ degradación).
pub fn repo_github(directorio: &str) -> Option<String> {
    let url = salida_git(directorio, &["remote", "get-url", "origin"])?;
    parsear_repo_github(&url)
}

/// Extrae "owner/repo" de una URL de remote de GitHub (SSH o HTTPS). None si no aplica.
/// Función pura (sin git) para poder testearla de forma determinista.
fn parsear_repo_github(url: &str) -> Option<String> {
    let url = url.trim();
    // Solo nos interesa github.com; otros hosts (gitlab, bitbucket) → None.
    if !url.contains("github.com") {
        return None;
    }
    // Aísla la parte "owner/repo" según el formato del remote.
    let resto = if let Some(idx) = url.find("github.com:") {
        // SSH: git@github.com:owner/repo.git
        &url[idx + "github.com:".len()..]
    } else if let Some(idx) = url.find("github.com/") {
        // HTTPS: https://github.com/owner/repo(.git)
        &url[idx + "github.com/".len()..]
    } else {
        return None;
    };
    // Quita el sufijo .git y barras sobrantes.
    let resto = resto.trim_end_matches('/').strip_suffix(".git").unwrap_or(resto.trim_end_matches('/'));
    let (owner, repo) = resto.split_once('/')?;
    let repo = repo.split('/').next().unwrap_or(repo);
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Rama git actual, o None. Reservada para enriquecer el resumen en el futuro
/// (hoy el resumen simplificado no la usa).
#[allow(dead_code)]
pub fn rama_git(directorio: &str) -> Option<String> {
    salida_git(directorio, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// tty del proceso padre (la terminal donde corre Claude). Best-effort.
///
/// PORTABILIDAD: el concepto de tty (y `os::unix::process::parent_id` + `ps`) es POSIX. En
/// Windows no aplica → devolvemos None (el campo tty es opcional; la TUI lo muestra vacío).
#[cfg(unix)]
pub fn tty() -> Option<String> {
    let ppid = std::os::unix::process::parent_id();
    let salida = Command::new("ps")
        .args(["-o", "tty=", "-p", &ppid.to_string()])
        .output()
        .ok()?;
    let t = String::from_utf8_lossy(&salida.stdout).trim().to_string();
    if t.is_empty() || t == "?" || t == "??" {
        None
    } else {
        Some(t)
    }
}

#[cfg(windows)]
pub fn tty() -> Option<String> {
    None
}

/// Hostname de la máquina donde corre el peer. El broker lo usa para la anti-colisión
/// cross-host (distinguir un peer remoto de uno local muerto; ver `Instancia::hostname`).
///
/// INTENCIÓN: con broker central (Mac) y peers en varios hosts, dos sesiones remotas del
/// mismo directorio pedían el mismo id y el broker no las distinguía (el PID remoto no es
/// verificable con kill -0 desde el Mac). Mandar el hostname cierra esa grieta. Sin deps
/// nuevas: `hostname` (portable Mac/Linux) y, si falla, las env HOSTNAME/HOST; "" como
/// último recurso → el broker degrada a la verificación por PID local (comportamiento previo).
pub fn hostname() -> String {
    if let Some(salida) = Command::new("hostname").output().ok() {
        let h = String::from_utf8_lossy(&salida.stdout).trim().to_string();
        if !h.is_empty() {
            return h;
        }
    }
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_default()
}

/// Id estable y ÚNICO POR TERMINAL, derivado del directorio + el TTY, para el caso "sin --id".
///
/// INTENCIÓN (solución definitiva a la colisión de ids): Max quiere lanzar `claude` sin pasar
/// CLAUDE_PEERS_ID ni --id y aun así tener un id ESTABLE (hereda la cola al reiniciar) y ÚNICO por
/// ventana. El problema anterior: dos sesiones en el MISMO directorio derivaban el MISMO id base y
/// dependían del sufijo del broker (-2) para diferenciarse — frágil bajo concurrencia (TOCTOU) y no
/// estable al reiniciar (la que era `-2` podía tomar el id base). AHORA el id incluye el TTY como
/// discriminador: `<nombre-dir>-<ttys>` (ej. `claude-peers-rs-s003` y `claude-peers-rs-s012`).
/// El TTY es estable por ventana de terminal y distinto entre ventanas → dos Claudes en el mismo
/// repo obtienen ids DISTINTOS y ESTABLES desde el arranque, sin depender de la carrera del broker.
/// La misma ventana conserva su id al reiniciar (mismo TTY) → hereda su cola. Esto es lo que Max
/// pidió: varias instancias por directorio, filtrables por nombre distinto (ejemplo-s003, …).
///
/// Sanea a [a-z0-9-] (el broker y el harness tratan el id como string opaco, pero un id limpio
/// evita sorpresas en logs/labels de GitHub).
///
/// FALLBACK sin nombre de carpeta usable (cwd `/`): base "peer" en vez de un genérico fijo tipo
/// "instancia" (ese colapsaba en masa las sesiones con cwd degradado). Sin TTY (proceso sin
/// terminal, p.ej. lanzado por un servicio): se usa solo la base + el broker sufija por colisión.
pub fn id_desde_directorio(directorio: &str) -> String {
    id_desde_directorio_y_tty(directorio, tty().as_deref())
}

/// Núcleo testeable: construye el id desde el directorio y un TTY opcional. Separado de
/// `id_desde_directorio` para poder probarlo sin depender del TTY real del proceso.
pub fn id_desde_directorio_y_tty(directorio: &str, tty: Option<&str>) -> String {
    let sanear = |s: &str| -> String {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    };

    let crudo = Path::new(directorio)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = {
        let b = sanear(&crudo);
        // Base neutra si no hay nombre de carpeta usable (cwd `/`). No "instancia" (colisionaba
        // en masa); "peer" + discriminador de TTY da identidad única sin colapso.
        if b.is_empty() { "peer".to_string() } else { b }
    };

    // Discriminador de sesión: sufijo compacto del TTY (ej. "ttys003" → "s003"). Hace el id ÚNICO
    // por ventana y estable al reiniciar la misma ventana. Sin TTY (proceso sin terminal), se deja
    // solo la base y el broker sufija por colisión si hiciera falta.
    match tty.map(sanear).filter(|t| !t.is_empty()) {
        Some(t) => {
            // "ttys003" → "s003"; cualquier otro formato → los últimos ~4 chars alfanuméricos.
            let disc = t.rsplit('-').next().unwrap_or(&t);
            let disc = disc.trim_start_matches("tty");
            format!("{base}-{disc}")
        }
        None => base,
    }
}

/// Resumen inicial de la instancia. A diferencia del TS (que llamaba a gpt-5.4-nano y
/// dependía de OPENAI_API_KEY), aquí lo construimos local y sin dependencias externas:
/// el papel (id) + el nombre del proyecto (carpeta del repo o del cwd). Es determinista,
/// instantáneo y nunca falla. La instancia puede refinarlo luego con la tool definir_resumen.
pub fn resumen_inicial(id: &str, directorio: &str, repo: Option<&str>) -> String {
    let proyecto = repo
        .or(Some(directorio))
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| directorio.to_string());
    format!("Instancia '{id}' trabajando en {proyecto}")
}

/// Ejecuta `git <args>` en `directorio` y devuelve stdout recortado si el comando tuvo éxito.
fn salida_git(directorio: &str, args: &[&str]) -> Option<String> {
    let salida = Command::new("git")
        .current_dir(directorio)
        .args(args)
        .output()
        .ok()?;
    if !salida.status.success() {
        return None;
    }
    let texto = String::from_utf8_lossy(&salida.stdout).trim().to_string();
    if texto.is_empty() {
        None
    } else {
        Some(texto)
    }
}


#[cfg(test)]
mod pruebas {
    use super::{id_desde_directorio_y_tty, parsear_repo_github};

    #[test]
    fn id_incluye_nombre_carpeta_y_tty() {
        // Con TTY, el id es <nombre-dir>-<disc>: único por ventana, estable al reiniciar.
        assert_eq!(id_desde_directorio_y_tty("/Users/max/claude-peers-rs", Some("ttys003")), "claude-peers-rs-s003");
        assert_eq!(id_desde_directorio_y_tty("/home/aistudio/ds", Some("ttys012")), "ds-s012");
    }

    #[test]
    fn mismo_dir_distinto_tty_da_ids_distintos() {
        // EL FIX DEFINITIVO: dos Claudes en el MISMO repo pero distintas ventanas → ids DISTINTOS
        // desde el arranque, sin depender del sufijo -2 del broker. Es lo que Max pidió.
        let a = id_desde_directorio_y_tty("/Users/max/claude-peers-rs", Some("ttys003"));
        let b = id_desde_directorio_y_tty("/Users/max/claude-peers-rs", Some("ttys012"));
        assert_ne!(a, b);
        assert_eq!(a, "claude-peers-rs-s003");
        assert_eq!(b, "claude-peers-rs-s012");
    }

    #[test]
    fn mismo_dir_mismo_tty_da_id_estable() {
        // La MISMA ventana (mismo TTY) conserva su id al reiniciar → hereda su cola de mensajes.
        let a = id_desde_directorio_y_tty("/Users/max/claude-peers-rs", Some("ttys003"));
        let b = id_desde_directorio_y_tty("/Users/max/claude-peers-rs", Some("ttys003"));
        assert_eq!(a, b);
    }

    #[test]
    fn id_sanea_y_minuscula() {
        assert_eq!(id_desde_directorio_y_tty("/tmp/P2V Grupo KH", Some("ttys001")), "p2v-grupo-kh-s001");
        assert_eq!(id_desde_directorio_y_tty("/x/Mi_Proyecto", None), "mi-proyecto");
    }

    #[test]
    fn id_fallback_si_vacio() {
        // Sin nombre de carpeta usable → base neutra "peer" (NO "instancia": ese colapsaba en masa).
        // Con TTY, cada sesión degradada sigue siendo única (peer-sNNN).
        assert_eq!(id_desde_directorio_y_tty("/", Some("ttys009")), "peer-s009");
        assert_eq!(id_desde_directorio_y_tty("", None), "peer");
    }

    #[test]
    fn ssh_estandar() {
        assert_eq!(
            parsear_repo_github("git@github.com:m4xjunior/lexusfx-emissor.git"),
            Some("m4xjunior/lexusfx-emissor".into())
        );
    }

    #[test]
    fn https_con_git() {
        assert_eq!(
            parsear_repo_github("https://github.com/m4xjunior/lexusfx-emissor.git"),
            Some("m4xjunior/lexusfx-emissor".into())
        );
    }

    #[test]
    fn https_sin_git() {
        assert_eq!(
            parsear_repo_github("https://github.com/owner/repo"),
            Some("owner/repo".into())
        );
    }

    #[test]
    fn https_con_barra_final() {
        assert_eq!(
            parsear_repo_github("https://github.com/owner/repo/"),
            Some("owner/repo".into())
        );
    }

    #[test]
    fn no_github_es_none() {
        assert_eq!(parsear_repo_github("git@gitlab.com:owner/repo.git"), None);
        assert_eq!(parsear_repo_github("https://bitbucket.org/owner/repo.git"), None);
    }

    #[test]
    fn vacio_o_basura_es_none() {
        assert_eq!(parsear_repo_github(""), None);
        assert_eq!(parsear_repo_github("github.com"), None);
    }
}
