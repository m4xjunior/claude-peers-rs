//! Integración con GitHub Issues — cada tarea del equipo es una issue rastreable.
//!
//! Reglas FIJAS (de Max):
//! 1. Solo se usa contra un REPO DE PRUEBA descartable, NUNCA un repo real.
//! 2. DEGRADACIÓN GRACIOSA: si GitHub no responde (o no hay token), la tarea sigue en
//!    local — la integración NUNCA bloquea el trabajo. Todos los métodos devuelven Result
//!    y el broker IGNORA el error (lo loguea con warn!).
//! 3. Sin GITHUB_TOKEN/GITHUB_REPO → `desde_entorno` devuelve None y el broker opera sin GH.
//!
//! Stack: octocrab con rustls (sin OpenSSL nativo), coherente con "zero deps nativas".

use octocrab::models::IssueState;
use octocrab::Octocrab;

/// Cliente de GitHub Issues. Construido solo si hay token + repo (owner/repo) en el entorno.
pub struct GitHub {
    cliente: Octocrab,
    owner: String,
    repo: String,
}

impl GitHub {
    /// Construye desde el entorno: GITHUB_TOKEN + GITHUB_REPO ("owner/repo").
    /// None si falta cualquiera de los dos → el broker opera sin GitHub (degradación).
    ///
    /// RECORDATORIO: GITHUB_REPO debe apuntar a un repo de PRUEBA descartable.
    pub fn desde_entorno() -> Option<Self> {
        let token = std::env::var("GITHUB_TOKEN").ok()?;
        let repo_full = std::env::var("GITHUB_REPO").ok()?;
        let (owner, repo) = repo_full.split_once('/')?;
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        let cliente = Octocrab::builder().personal_token(token).build().ok()?;
        Some(Self {
            cliente,
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }

    /// Crea una issue para una tarea. title = descripción; labels = [instancia_id, área].
    /// Devuelve el número de issue. Err si GitHub falló (el broker lo ignora → degradación).
    pub async fn crear_issue(&self, titulo: &str, labels: &[String]) -> anyhow::Result<u64> {
        let issue = self
            .cliente
            .issues(&self.owner, &self.repo)
            .create(titulo)
            .labels(labels.to_vec())
            .send()
            .await?;
        Ok(issue.number)
    }

    /// Añade un comentario a una issue (cada report de la tarea).
    pub async fn comentar_issue(&self, numero: u64, texto: &str) -> anyhow::Result<()> {
        self.cliente
            .issues(&self.owner, &self.repo)
            .create_comment(numero, texto)
            .await?;
        Ok(())
    }

    /// Cierra una issue (al cerrar la tarea); comenta la conclusión si se provee.
    pub async fn cerrar_issue(
        &self,
        numero: u64,
        comentario_final: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(c) = comentario_final {
            // El comentario es best-effort: si falla, igual intentamos cerrar.
            let _ = self
                .cliente
                .issues(&self.owner, &self.repo)
                .create_comment(numero, c)
                .await;
        }
        self.cliente
            .issues(&self.owner, &self.repo)
            .update(numero)
            .state(IssueState::Closed)
            .send()
            .await?;
        Ok(())
    }
}
