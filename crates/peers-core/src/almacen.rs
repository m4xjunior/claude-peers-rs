//! Trait `Almacen` — la interfaz de persistencia del broker.
//!
//! Abstrae el motor de almacenamiento para que el resto del broker (handlers, jornada,
//! github) programe contra esta interfaz y no contra SQLite ni Redis directamente. Es el
//! CONTRATO de coordinación de la fase 2: store.rs (Redis) lo implementa; jornada.rs lo
//! consume; los tests corren contra cualquier implementación.
//!
//! Todos los métodos son async (Redis lo es) y devuelven `anyhow::Result` — nada entra
//! en pánico; el handler traduce el error a un 500 con JSON.

use crate::{Alcance, Instancia, ItemOutbox, Mensaje, Sesion, Tarea};
use async_trait::async_trait;

/// Alcance de listado, reexportado para que las firmas no dependan del módulo concreto.
pub use crate::Alcance as AlcanceListado;

#[async_trait]
pub trait Almacen: Send + Sync {
    // --- Instancias (fase 1) ---

    /// Registra o re-registra (UPDATE sin perder fila) una instancia con id estable.
    async fn registrar(
        &self,
        id: &str,
        pid: i64,
        directorio: &str,
        repo_git: Option<&str>,
        tty: Option<&str>,
        resumen: &str,
        ahora: &str,
    ) -> anyhow::Result<()>;

    async fn latido(&self, id: &str, ahora: &str) -> anyhow::Result<()>;
    async fn definir_resumen(&self, id: &str, resumen: &str) -> anyhow::Result<()>;
    async fn salir(&self, id: &str) -> anyhow::Result<()>;
    async fn instancia_existe(&self, id: &str) -> anyhow::Result<bool>;
    async fn contar_instancias(&self) -> anyhow::Result<usize>;

    async fn listar(
        &self,
        alcance: Alcance,
        directorio: &str,
        repo_git: Option<&str>,
        excluir_id: Option<&str>,
        vencidas_antes: &str,
    ) -> anyhow::Result<Vec<Instancia>>;

    // --- Mensajes (fase 1) ---

    async fn encolar_mensaje(
        &self,
        de_id: &str,
        para_id: &str,
        texto: &str,
        ahora: &str,
    ) -> anyhow::Result<()>;

    async fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>>;

    async fn limpiar_vencidas(&self, vencidas_antes: &str) -> anyhow::Result<usize>;

    // --- Outbox durable con ACK (fase 2) ---

    /// Encola un ítem durable que debe sobrevivir a un reinicio del destinatario.
    async fn outbox_encolar(&self, item: &ItemOutbox) -> anyhow::Result<()>;

    /// Lista los ítems del outbox de `para_id` aún sin confirmar (pendientes).
    async fn outbox_pendientes(&self, para_id: &str) -> anyhow::Result<Vec<ItemOutbox>>;

    /// Marca un ítem como confirmado (ACK). Idempotente.
    async fn outbox_confirmar(&self, item_id: &str) -> anyhow::Result<()>;

    // --- Jornada (fase 2) ---

    async fn sesion_abrir(&self, sesion: &Sesion) -> anyhow::Result<()>;
    async fn sesion_cerrar(&self, instancia_id: &str, fin: &str) -> anyhow::Result<()>;
    async fn tarea_guardar(&self, tarea: &Tarea) -> anyhow::Result<()>;
    async fn tarea_obtener(&self, tarea_id: &str) -> anyhow::Result<Option<Tarea>>;
    async fn jornada(&self, instancia_id: &str) -> anyhow::Result<(Vec<Sesion>, Vec<Tarea>)>;
}
