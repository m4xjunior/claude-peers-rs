//! Integración con GitHub Issues — cada tarea del equipo es una issue rastreable.
//!
//! ESQUELETO/CONTRATO — lo rellena Aluísio Back (jopwh40d). Reglas FIJAS (de Max):
//!
//! 1. Solo se prueba contra un REPO DE PRUEBA descartable, NUNCA contra un repo real.
//! 2. DEGRADACIÓN GRACIOSA: si GitHub no responde (o no hay token), la tarea sigue
//!    funcionando en local (Redis) — la integración NUNCA bloquea ni tumba el trabajo.
//!    Por eso todos los métodos devuelven Result y el broker IGNORA el error (lo loguea).
//! 3. Sin token configurado (GITHUB_TOKEN ausente) → `GitHub::desde_entorno` devuelve
//!    None y el broker opera sin tocar GitHub.
//!
//! Sugerencia de stack (decidir con docs/stack-decisions.md): `octocrab` (cliente GitHub
//! idiomático) o `reqwest` directo a la API REST. Cualquiera sirve mientras respete estas
//! firmas y la degradación graciosa.

#![allow(dead_code)] // se activa al cablear en main.rs

/// Cliente de GitHub Issues. Construido solo si hay token + repo configurados.
pub struct GitHub {
    // TODO(aluisio-back): token, owner, repo, cliente http/octocrab.
}

impl GitHub {
    /// Construye el cliente desde el entorno (GITHUB_TOKEN, GITHUB_REPO="owner/repo").
    /// Devuelve None si falta configuración → el broker opera sin GitHub (degradación).
    pub fn desde_entorno() -> Option<Self> {
        // TODO(aluisio-back): leer GITHUB_TOKEN y GITHUB_REPO; None si falta alguno.
        // RECORDATORIO: GITHUB_REPO debe apuntar a un repo de PRUEBA descartable.
        None
    }

    /// Crea una issue para una tarea. title = descripción, labels = [instancia_id, área].
    /// Devuelve el número de issue creado, o Err si GitHub falló (el broker lo ignora).
    pub async fn crear_issue(
        &self,
        _titulo: &str,
        _labels: &[String],
    ) -> anyhow::Result<u64> {
        todo!("aluisio-back: POST /repos/{{owner}}/{{repo}}/issues; devolver issue.number")
    }

    /// Añade un comentario a una issue (cada report de la tarea).
    pub async fn comentar_issue(&self, _numero: u64, _texto: &str) -> anyhow::Result<()> {
        todo!("aluisio-back: POST /repos/{{owner}}/{{repo}}/issues/{{n}}/comments")
    }

    /// Cierra una issue (al cerrar la tarea), opcionalmente con comentario de conclusión.
    pub async fn cerrar_issue(
        &self,
        _numero: u64,
        _comentario_final: Option<&str>,
    ) -> anyhow::Result<()> {
        todo!("aluisio-back: comentar conclusión si hay + PATCH issue state=closed")
    }
}
