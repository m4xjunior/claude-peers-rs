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
    use super::parsear_repo_github;

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
