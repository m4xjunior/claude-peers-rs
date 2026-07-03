//! Bitácora de acciones — RFC registro-acciones / ADR-001-bitacora-sqlx-identidad-durable.
//!
//! Componente TRANSVERSAL a ambos backends del store (Redis y rusqlite): vive en un fichero
//! SQLite PROPIO (`--bitacora-db`), con SQLx (pool async) + WAL + `foreign_keys=ON`. Está
//! deliberadamente FUERA del trait `Almacen` (desviación de R8b aprobada): dentro del trait
//! habría dos implementaciones idénticas compartiendo el mismo pool — duplicación sin valor.
//!
//! Identidad DURABLE (el corazón del ADR): `peers_conocidos` se upserta en cada registro y
//! NUNCA se borra — el histórico sobrevive a kicks y vencimientos de liveness (a diferencia
//! de `instancias`, que es presencia efímera). `tareas_conocidas` es un ANCLA de FK, no la
//! fuente de verdad del estado de la tarea (esa sigue siendo el store vía `tarea_guardar`).
//!
//! Sin macros compile-time de sqlx (queries dinámicas con `bind`): nadie necesita
//! `DATABASE_URL` ni `.sqlx/` para compilar. R9: quien llama decide degradar — un fallo aquí
//! JAMÁS tumba la mutación de negocio (la bitácora es observabilidad).

use anyhow::Context as _;
use peers_core::{AccionRegistrada, Tarea, TipoAccion};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row as _, SqlitePool};
use std::time::Duration;

pub struct Bitacora {
    pool: SqlitePool,
}

impl Bitacora {
    /// Abre (o crea) el fichero de bitácora y aplica las migraciones embebidas
    /// (`migrations/`). Crea el directorio padre si no existe.
    pub async fn abrir(ruta: &str) -> anyhow::Result<Self> {
        if let Some(dir) = std::path::Path::new(ruta).parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creando el directorio de la bitácora {}", dir.display()))?;
            }
        }
        let opciones = SqliteConnectOptions::new()
            .filename(ruta)
            .create_if_missing(true)
            // WAL: lecturas del endpoint concurrentes con las escrituras de los handlers.
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(3))
            // SQLite NO aplica FK por defecto: se activa POR CONEXIÓN (todo el pool).
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opciones)
            .await
            .with_context(|| format!("abriendo la bitácora en {ruta}"))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("aplicando las migraciones de la bitácora")?;
        Ok(Self { pool })
    }

    /// Registra una acción (R4) con sus upserts de identidad durable, en UNA transacción:
    /// actor → `peers_conocidos`; si la acción versa sobre una tarea, también su dueño (puede
    /// no ser el actor: el operador asigna a un peer) y el ancla en `tareas_conocidas`; por
    /// último el evento en `acciones`. Así las FK SIEMPRE resuelven.
    pub async fn registrar(&self, a: &AccionRegistrada, tarea: Option<&Tarea>) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        upsert_peer(&mut tx, &a.quien, &a.cuando, None).await?;
        let mut tarea_id: Option<&str> = None;
        if let Some(t) = tarea {
            upsert_peer(&mut tx, &t.instancia_id, &a.cuando, None).await?;
            sqlx::query(
                "INSERT INTO tareas_conocidas (id, instancia_id, descripcion, creada_en)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET descripcion = excluded.descripcion",
            )
            .bind(&t.id)
            .bind(&t.instancia_id)
            .bind(&t.descripcion)
            .bind(&t.inicio)
            .execute(&mut *tx)
            .await?;
            tarea_id = Some(t.id.as_str());
        }
        sqlx::query(
            "INSERT INTO acciones (instancia_id, accion, sujeto, tarea_id, detalle, cuando, evidencia)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&a.quien)
        .bind(accion_texto(a.accion))
        .bind(&a.sujeto)
        .bind(tarea_id)
        .bind(&a.detalle)
        .bind(&a.cuando)
        .bind(&a.evidencia)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Últimas acciones del peer, MÁS RECIENTE PRIMERO (R6). `desde` = cursor ISO exclusivo
    /// hacia atrás (paginación). `limite` se acota a [1, 500] (RETENCION_ACCIONES es el techo
    /// natural). Una fila con un `TipoAccion` desconocido (broker más nuevo escribió) se salta
    /// sin abortar el listado — degradación por fila, como el resto del proyecto.
    pub async fn listar(
        &self,
        instancia_id: &str,
        desde: Option<&str>,
        limite: i64,
    ) -> anyhow::Result<Vec<AccionRegistrada>> {
        let limite = limite.clamp(1, 500);
        let filas: Vec<SqliteRow> = match desde {
            Some(d) => {
                sqlx::query(
                    "SELECT instancia_id, accion, sujeto, detalle, cuando, evidencia FROM acciones
                     WHERE instancia_id = ?1 AND cuando < ?2
                     ORDER BY cuando DESC, id DESC LIMIT ?3",
                )
                .bind(instancia_id)
                .bind(d)
                .bind(limite)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT instancia_id, accion, sujeto, detalle, cuando, evidencia FROM acciones
                     WHERE instancia_id = ?1
                     ORDER BY cuando DESC, id DESC LIMIT ?2",
                )
                .bind(instancia_id)
                .bind(limite)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(filas.iter().filter_map(fila_a_accion).collect())
    }

    /// Poda la bitácora a las últimas `retener` acciones POR PEER (R7). Corre en el barrido
    /// periódico del broker (mismo ciclo que podar_historial/podar_tareas).
    pub async fn podar(&self, retener: i64) -> anyhow::Result<()> {
        if retener <= 0 {
            return Ok(()); // no-op defensivo, igual que podar_historial(0)
        }
        sqlx::query(
            "DELETE FROM acciones WHERE id IN (
                SELECT a.id FROM acciones a
                WHERE (SELECT COUNT(*) FROM acciones a2
                       WHERE a2.instancia_id = a.instancia_id AND a2.id > a.id) >= ?1)",
        )
        .bind(retener)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Upsert de identidad durable (ADR-001): registra la primera vez y refresca la última.
/// JAMÁS se borra de `peers_conocidos` — ni kick ni vencimiento tocan esta tabla.
async fn upsert_peer(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    visto: &str,
    resumen: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO peers_conocidos (id, primer_visto, ultimo_visto, ultimo_resumen)
         VALUES (?1, ?2, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
           ultimo_visto = excluded.ultimo_visto,
           ultimo_resumen = COALESCE(excluded.ultimo_resumen, peers_conocidos.ultimo_resumen)",
    )
    .bind(id)
    .bind(visto)
    .bind(resumen)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Forma de wire (snake_case) de un `TipoAccion` para la columna `accion` — la MISMA cadena
/// que emite serde en el protocolo, así el JSON del endpoint y la columna coinciden siempre.
fn accion_texto(a: TipoAccion) -> String {
    serde_json::to_string(&a)
        .unwrap_or_else(|_| "\"desconocida\"".to_string())
        .trim_matches('"')
        .to_string()
}

/// Reconstruye una `AccionRegistrada` desde una fila. `None` si la fila es ilegible o trae un
/// `TipoAccion` que este binario no conoce (se salta, no aborta — degradación por fila).
fn fila_a_accion(f: &SqliteRow) -> Option<AccionRegistrada> {
    let accion_txt: String = f.try_get("accion").ok()?;
    let accion: TipoAccion = serde_json::from_str(&format!("\"{accion_txt}\"")).ok()?;
    Some(AccionRegistrada {
        quien: f.try_get("instancia_id").ok()?,
        accion,
        sujeto: f.try_get::<Option<String>, _>("sujeto").ok().flatten(),
        detalle: f.try_get::<Option<String>, _>("detalle").ok().flatten(),
        cuando: f.try_get("cuando").ok()?,
        evidencia: f.try_get::<Option<String>, _>("evidencia").ok().flatten(),
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use peers_core::EstadoTarea;

    async fn bitacora_en_memoria() -> Bitacora {
        // ":memory:" con pool >1 daría una BD por conexión; para tests basta 1 conexión.
        let opciones = SqliteConnectOptions::new()
            .filename(":memory:")
            .journal_mode(SqliteJournalMode::Memory)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opciones)
            .await
            .expect("pool en memoria");
        sqlx::migrate!("./migrations").run(&pool).await.expect("migraciones");
        Bitacora { pool }
    }

    fn accion(quien: &str, cuando: &str) -> AccionRegistrada {
        AccionRegistrada {
            quien: quien.to_string(),
            accion: TipoAccion::EnviarMensaje,
            sujeto: Some("destino".into()),
            detalle: None,
            cuando: cuando.to_string(),
            evidencia: None,
        }
    }

    /// AC1/R4: registrar + listar devuelve el evento con su cuando y sujeto; más reciente primero.
    #[tokio::test]
    async fn registrar_y_listar_reciente_primero() {
        let b = bitacora_en_memoria().await;
        b.registrar(&accion("pa", "2026-01-01T00:00:01Z"), None).await.expect("registrar 1");
        b.registrar(&accion("pa", "2026-01-01T00:00:02Z"), None).await.expect("registrar 2");
        b.registrar(&accion("pb", "2026-01-01T00:00:03Z"), None).await.expect("otro peer");
        let acciones = b.listar("pa", None, 100).await.expect("listar");
        assert_eq!(acciones.len(), 2, "solo las del peer pedido (AC2)");
        assert_eq!(acciones[0].cuando, "2026-01-01T00:00:02Z", "más reciente primero");
        assert_eq!(acciones[0].sujeto.as_deref(), Some("destino"));
    }

    /// #11: registrar una acción CON evidencia la persiste y `listar` la devuelve intacta;
    /// una acción SIN evidencia (la mayoría, y toda la bitácora previa a este campo) sigue
    /// deserializando con `evidencia: None` — la columna `ADD COLUMN` es nullable, sin romper
    /// filas existentes ni el resto del roundtrip.
    #[tokio::test]
    async fn evidencia_se_persiste_y_se_lee_de_vuelta() {
        let b = bitacora_en_memoria().await;
        let mut con_evidencia = accion("pa", "2026-01-01T00:00:01Z");
        con_evidencia.evidencia = Some("commit abc123, ver PR #42".to_string());
        b.registrar(&con_evidencia, None).await.expect("registrar con evidencia");
        b.registrar(&accion("pa", "2026-01-01T00:00:02Z"), None)
            .await
            .expect("registrar sin evidencia");

        let acciones = b.listar("pa", None, 100).await.expect("listar");
        assert_eq!(acciones.len(), 2);
        // Más reciente primero (sin evidencia) → la de evidencia queda en [1].
        assert_eq!(acciones[1].evidencia.as_deref(), Some("commit abc123, ver PR #42"));
        assert!(acciones[0].evidencia.is_none(), "sin evidencia sigue siendo None, no vacío");
    }

    /// ADR-001: la acción sobre una tarea siembra el ancla (tareas_conocidas) y la FK resuelve;
    /// el dueño de la tarea (distinto del actor) también queda en peers_conocidos.
    #[tokio::test]
    async fn accion_con_tarea_siembra_anclas_fk() {
        let b = bitacora_en_memoria().await;
        let tarea = Tarea {
            id: "tar-9".into(),
            instancia_id: "peer-dueno".into(),
            sesion_id: "ses-1".into(),
            descripcion: "hacer algo".into(),
            inicio: "2026-01-01T00:00:00Z".into(),
            fin: None,
            duracion_seg: None,
            estimado_seg: None,
            estado: EstadoTarea::Abierta,
            bloqueo_motivo: None,
            issue_number: None,
            factor_aprendido: false,
            evidencia: None,
        };
        let mut a = accion("operador", "2026-01-01T00:00:05Z");
        a.accion = TipoAccion::CrearTarea;
        a.sujeto = Some(tarea.id.clone());
        b.registrar(&a, Some(&tarea)).await.expect("registrar con tarea");

        // La FK resolvió (no falló el INSERT) y el JOIN devuelve la descripción del ancla.
        let fila = sqlx::query(
            "SELECT t.descripcion FROM acciones ac JOIN tareas_conocidas t ON ac.tarea_id = t.id
             WHERE ac.instancia_id = 'operador'",
        )
        .fetch_one(&b.pool)
        .await
        .expect("join accion↔tarea");
        let desc: String = fila.try_get("descripcion").expect("descripcion");
        assert_eq!(desc, "hacer algo");
    }

    /// R7/AC4: la poda deja como mucho `retener` por peer (los más recientes) y respeta a otros.
    #[tokio::test]
    async fn poda_por_peer_conserva_recientes() {
        let b = bitacora_en_memoria().await;
        for i in 0..7 {
            b.registrar(&accion("pa", &format!("2026-01-01T00:00:0{i}Z")), None)
                .await
                .expect("registrar");
        }
        b.registrar(&accion("pb", "2026-01-01T00:00:09Z"), None).await.expect("pb");
        b.podar(5).await.expect("podar");
        let pa = b.listar("pa", None, 100).await.expect("listar pa");
        let pb = b.listar("pb", None, 100).await.expect("listar pb");
        assert_eq!(pa.len(), 5, "pa podada a 5");
        assert_eq!(pa[0].cuando, "2026-01-01T00:00:06Z", "conserva las recientes");
        assert_eq!(pb.len(), 1, "pb intacta (poda por peer)");
    }

    /// R6: `desde` pagina hacia atrás (exclusivo) y `limite` se respeta.
    #[tokio::test]
    async fn listar_con_cursor_y_limite() {
        let b = bitacora_en_memoria().await;
        for i in 1..=4 {
            b.registrar(&accion("pa", &format!("2026-01-01T00:00:0{i}Z")), None)
                .await
                .expect("registrar");
        }
        let pagina = b.listar("pa", Some("2026-01-01T00:00:04Z"), 2).await.expect("pagina");
        assert_eq!(pagina.len(), 2);
        assert_eq!(pagina[0].cuando, "2026-01-01T00:00:03Z", "exclusivo hacia atrás");
        assert_eq!(pagina[1].cuando, "2026-01-01T00:00:02Z");
    }
}
