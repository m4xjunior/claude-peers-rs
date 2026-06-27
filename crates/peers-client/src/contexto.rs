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
