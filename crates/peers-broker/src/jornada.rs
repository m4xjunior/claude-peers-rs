//! Jornada — control de tiempo MEDIDO por el broker (la IA nunca estima).
//!
//! ESQUELETO/CONTRATO (lo rellena su responsable). El broker timbra inicio/fin/duración
//! con SU reloj: por eso todas las funciones reciben `ahora: &str` (timestamp del servidor)
//! y NUNCA un tiempo provisto por el cliente.
//!
//! Depende solo del trait `Almacen` (peers-core) — no de Redis ni SQLite directamente,
//! para integrar sin colisión.

use peers_core::{Almacen, RespuestaJornada, Sesion, Tarea};
use std::sync::Arc;

/// Abre una sesión para la instancia, timbrada por el broker. Se llama en /registrar.
pub async fn abrir_sesion(
    almacen: &Arc<dyn Almacen>,
    sesion_id: &str,
    instancia_id: &str,
    ahora: &str,
) -> anyhow::Result<()> {
    let sesion = Sesion {
        id: sesion_id.to_string(),
        instancia_id: instancia_id.to_string(),
        inicio: ahora.to_string(),
        fin: None,
        duracion_seg: None,
    };
    almacen.sesion_abrir(&sesion).await
}

/// Cierra la sesión abierta de la instancia, calculando la duración con el reloj del broker.
pub async fn cerrar_sesion(
    almacen: &Arc<dyn Almacen>,
    instancia_id: &str,
    ahora: &str,
) -> anyhow::Result<()> {
    almacen.sesion_cerrar(instancia_id, ahora).await
}

/// Abre una tarea dentro de la sesión activa de la instancia. Devuelve la tarea creada
/// (sin issue todavía; la integración GitHub la rellena después si está activa).
pub async fn abrir_tarea(
    almacen: &Arc<dyn Almacen>,
    tarea_id: &str,
    instancia_id: &str,
    sesion_id: &str,
    descripcion: &str,
    ahora: &str,
) -> anyhow::Result<Tarea> {
    let tarea = Tarea {
        id: tarea_id.to_string(),
        instancia_id: instancia_id.to_string(),
        sesion_id: sesion_id.to_string(),
        descripcion: descripcion.to_string(),
        inicio: ahora.to_string(),
        fin: None,
        duracion_seg: None,
        issue_number: None,
        // El estimado lo wirea el handler /crear-tarea (Fase 3); aquí queda neutro para no
        // alterar el comportamiento del fichaje existente.
        estimado_seg: None,
    };
    almacen.tarea_guardar(&tarea).await?;
    Ok(tarea)
}

/// Cierra una tarea: timbra fin y calcula duración (fin - inicio) con el reloj del broker.
/// Devuelve la tarea cerrada (para que github.rs cierre la issue espejo con la duración).
pub async fn cerrar_tarea(
    almacen: &Arc<dyn Almacen>,
    tarea_id: &str,
    ahora: &str,
) -> anyhow::Result<Tarea> {
    let mut tarea = almacen
        .tarea_obtener(tarea_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("la tarea '{tarea_id}' no existe"))?;
    tarea.fin = Some(ahora.to_string());
    tarea.duracion_seg = Some(diferencia_seg(&tarea.inicio, ahora));
    almacen.tarea_guardar(&tarea).await?;
    Ok(tarea)
}

/// Vista consolidada de la jornada de una instancia.
pub async fn consolidar(
    almacen: &Arc<dyn Almacen>,
    instancia_id: &str,
) -> anyhow::Result<RespuestaJornada> {
    let (sesiones, tareas) = almacen.jornada(instancia_id).await?;
    Ok(RespuestaJornada { sesiones, tareas })
}

/// Diferencia en segundos entre dos timestamps RFC3339. Si algo no parsea, devuelve 0
/// (nunca entra en pánico). El cálculo lo hace el broker, no el cliente.
pub fn diferencia_seg(inicio: &str, fin: &str) -> i64 {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;
    let parse = |s: &str| OffsetDateTime::parse(s, &Rfc3339).ok();
    match (parse(inicio), parse(fin)) {
        (Some(i), Some(f)) => (f - i).whole_seconds().max(0),
        _ => 0,
    }
}
