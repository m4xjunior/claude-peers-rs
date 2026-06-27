//! Integración con GitHub Issues — cada tarea del equipo es una issue rastreable.
//!
//! Reglas FIJAS (de Max):
//! 1. El repo es DINÁMICO: las issues van al repo donde el peer trabaja ("owner/repo"),
//!    resuelto por el client desde el remote origin y pasado por parámetro. Ya NO hay
//!    GITHUB_REPO fijo en el entorno — solo el token (credencial de la máquina del broker).
//! 2. DEGRADACIÓN GRACIOSA: si GitHub no responde (o no hay token), la tarea sigue en
//!    local — la integración NUNCA bloquea el trabajo. Todos los métodos devuelven Result
//!    y el broker IGNORA el error (lo loguea con warn!).
//! 3. Sin GITHUB_TOKEN → `desde_entorno` devuelve None y el broker opera sin GH.
//!    Sin repo_github en la instancia dueña → el broker tampoco crea issue (degradación).
//!
//! Stack: octocrab con rustls (sin OpenSSL nativo), coherente con "zero deps nativas".

use octocrab::models::IssueState;
use octocrab::Octocrab;

/// Cliente de GitHub Issues. Construido solo si hay token en el entorno. El repo destino
/// (owner/repo) NO se guarda aquí: llega por parámetro en cada método, porque es dinámico
/// (el repo donde trabaja el peer dueño de la tarea).
pub struct GitHub {
    cliente: Octocrab,
}

impl GitHub {
    /// Construye desde el entorno: SOLO GITHUB_TOKEN (la credencial, por-máquina-del-broker).
    /// None si falta el token → el broker opera sin GitHub (degradación graciosa).
    ///
    /// El repo destino ya NO viene del entorno: es dinámico, derivado del peer (owner/repo).
    pub fn desde_entorno() -> Option<Self> {
        let token = std::env::var("GITHUB_TOKEN").ok()?;
        if token.is_empty() {
            return None;
        }
        let cliente = Octocrab::builder().personal_token(token).build().ok()?;
        Some(Self { cliente })
    }

    /// Crea una issue en `owner/repo` (dinámico, del peer dueño de la tarea).
    /// title = descripción; labels = [instancia_id, área]. Devuelve el número de issue.
    /// Err si GitHub falló (el broker lo ignora → degradación).
    pub async fn crear_issue(
        &self,
        owner: &str,
        repo: &str,
        titulo: &str,
        labels: &[String],
    ) -> anyhow::Result<u64> {
        let issue = self
            .cliente
            .issues(owner, repo)
            .create(titulo)
            .labels(labels.to_vec())
            .send()
            .await?;
        Ok(issue.number)
    }

    /// Añade un comentario a una issue de `owner/repo` (cada report de la tarea).
    pub async fn comentar_issue(
        &self,
        owner: &str,
        repo: &str,
        numero: u64,
        texto: &str,
    ) -> anyhow::Result<()> {
        self.cliente
            .issues(owner, repo)
            .create_comment(numero, texto)
            .await?;
        Ok(())
    }

    /// Cierra una issue de `owner/repo` (al cerrar la tarea); comenta la conclusión si se provee.
    pub async fn cerrar_issue(
        &self,
        owner: &str,
        repo: &str,
        numero: u64,
        comentario_final: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(c) = comentario_final {
            // El comentario es best-effort: si falla, igual intentamos cerrar.
            let _ = self
                .cliente
                .issues(owner, repo)
                .create_comment(numero, c)
                .await;
        }
        self.cliente
            .issues(owner, repo)
            .update(numero)
            .state(IssueState::Closed)
            .send()
            .await?;
        Ok(())
    }
}
