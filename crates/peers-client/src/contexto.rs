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

/// Id estable derivado del directorio de trabajo, para el caso "sin --id".
///
/// INTENCIÓN: Max quiere lanzar `claude` sin pasar CLAUDE_PEERS_ID ni --id y aun así
/// tener un id ESTABLE por terminal (para heredar la cola al reiniciar). El nombre de la
/// carpeta es el id BASE (rol implícito por proyecto). VARIAS instancias en el mismo
/// directorio están PERMITIDAS: comparten el id base, y el broker las diferencia con sufijo
/// (`ejemplo`, `ejemplo-2`, `ejemplo-3`…) al detectar la colisión en el registro. Así se puede
/// filtrar por nombre distinto sin que Max escriba nada.
///
/// Sanea el nombre a [a-z0-9-] (el broker y el harness tratan el id como string opaco,
/// pero un id limpio evita sorpresas en logs/labels de GitHub).
///
/// FALLBACK (sin nombre de carpeta usable, p.ej. cwd `/`): NO se cae a un genérico fijo como
/// "instancia" — eso hacía que TODAS las sesiones con cwd degradado colapsaran al mismo id y se
/// pisaran (bug real observado: id="instancia", cwd="/"). En su lugar se usa "peer" como base y
/// se deja que el broker sufije por colisión, de modo que cada sesión conserve una identidad única.
pub fn id_desde_directorio(directorio: &str) -> String {
    let crudo = Path::new(directorio)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let limpio: String = crudo
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if limpio.is_empty() {
        // Base neutra; el broker la sufija (-2, -3…) si varias sesiones caen aquí, evitando el
        // colapso a un id compartido. No es "instancia" a propósito (ese colisionaba en masa).
        "peer".into()
    } else {
        limpio
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
    use super::{id_desde_directorio, parsear_repo_github};

    #[test]
    fn id_desde_directorio_usa_nombre_carpeta() {
        assert_eq!(id_desde_directorio("/Users/max/claude-peers-rs"), "claude-peers-rs");
        assert_eq!(id_desde_directorio("/home/aistudio/ds"), "ds");
    }

    #[test]
    fn id_desde_directorio_sanea_y_minuscula() {
        assert_eq!(id_desde_directorio("/tmp/P2V Grupo KH"), "p2v-grupo-kh");
        assert_eq!(id_desde_directorio("/x/Mi_Proyecto"), "mi-proyecto");
    }

    #[test]
    fn id_desde_directorio_fallback_si_vacio() {
        // Sin nombre de carpeta usable → base neutra "peer" (NO "instancia": ese colapsaba
        // en masa todas las sesiones con cwd degradado a un id compartido). El broker la sufija.
        assert_eq!(id_desde_directorio("/"), "peer");
        assert_eq!(id_desde_directorio(""), "peer");
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
