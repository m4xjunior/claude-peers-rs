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
use peers_core::{AccionRegistrada, DireccionWhatsapp, MensajeWhatsapp, PeerConocido, Tarea, TipoAccion};
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
        upsert_peer(&mut tx, &a.quien, &a.cuando, None, None).await?;
        let mut tarea_id: Option<&str> = None;
        if let Some(t) = tarea {
            upsert_peer(&mut tx, &t.instancia_id, &a.cuando, None, None).await?;
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

    /// Ancla la identidad DURABLE de un peer en cada REGISTRO (alta y re-registro), con su
    /// directorio y resumen. Es lo que permite al Chat global filtrar por directorio a peers que
    /// ya cerraron: `instancias` los borra al salir, esta tabla no.
    ///
    /// Se llama también en el RE-registro (a diferencia de `TipoAccion::Registrar`, que sólo se
    /// emite en el alta para no inundar la bitácora): aquí no se inserta un evento, sólo se
    /// refresca una fila — y un peer puede cambiar de directorio entre sesiones, así que el dato
    /// tiene que seguirle.
    pub async fn peer_visto(
        &self,
        id: &str,
        cuando: &str,
        resumen: Option<&str>,
        directorio: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        upsert_peer(&mut tx, id, cuando, resumen, directorio).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Todos los peers que el broker vio ALGUNA VEZ, con su directorio (identidad durable, R6).
    /// Lo consume el Chat global para poder filtrar por directorio incluyendo a los que ya no
    /// están vivos. Orden por id para que la UI no salte entre refrescos.
    pub async fn peers_conocidos(&self) -> anyhow::Result<Vec<PeerConocido>> {
        let filas: Vec<SqliteRow> = sqlx::query(
            "SELECT id, primer_visto, ultimo_visto, ultimo_resumen, directorio
             FROM peers_conocidos ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(filas
            .iter()
            .map(|f| PeerConocido {
                id: f.get("id"),
                primer_visto: f.get("primer_visto"),
                ultimo_visto: f.get("ultimo_visto"),
                ultimo_resumen: f.get("ultimo_resumen"),
                directorio: f.get("directorio"),
            })
            .collect())
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

    /// Chequeo barato de "¿ya vi este `evo_message_id`?" — usado para saltar trabajo caro (bajar
    /// y transcribir un adjunto) en mensajes que el webhook/polling ya reintentó, ANTES de pagar
    /// ese costo. `whatsapp_registrar_entrada` ya deduplica el INSERT, pero eso pasa DESPUÉS de
    /// resolver el adjunto — sin este chequeo previo, cada reintento re-descarga y re-transcribe.
    pub async fn whatsapp_ya_existe(&self, evo_message_id: &str) -> anyhow::Result<bool> {
        let fila: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM whatsapp_mensajes WHERE evo_message_id = ?1 LIMIT 1")
                .bind(evo_message_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(fila.is_some())
    }

    /// Registra un mensaje ENTRANTE del webhook de Evolution. Dedupe por `evo_message_id` vía el
    /// índice único parcial (0004): un reintento del webhook con el mismo `key.id` (at-least-once
    /// delivery es la norma) no duplica la fila — se ignora en silencio. Devuelve `false` cuando
    /// el mensaje ya existía, para que el caller sepa que NO debe re-empujarlo a los peers.
    pub async fn whatsapp_registrar_entrada(
        &self,
        texto: &str,
        nombre: Option<&str>,
        cuando: &str,
        evo_message_id: &str,
    ) -> anyhow::Result<bool> {
        // El índice único (0004) es PARCIAL (`WHERE evo_message_id IS NOT NULL`): SQLite exige
        // repetir esa misma condición en el `ON CONFLICT` para que el conflict-target resuelva al
        // índice parcial — sin ella, "no matching unique/primary key" (comprobado en vivo).
        let resultado = sqlx::query(
            "INSERT INTO whatsapp_mensajes (direccion, texto, nombre, cuando, evo_message_id)
             VALUES ('entrada', ?1, ?2, ?3, ?4)
             ON CONFLICT(evo_message_id) WHERE evo_message_id IS NOT NULL DO NOTHING",
        )
        .bind(texto)
        .bind(nombre)
        .bind(cuando)
        .bind(evo_message_id)
        .execute(&self.pool)
        .await?;
        Ok(resultado.rows_affected() > 0)
    }

    /// Registra un mensaje SALIENTE: un peer (o el operador, desde el desktop) le respondió.
    /// `autor_peer_id = None` significa "el operador" (vía la pantalla del desktop).
    /// `evo_message_id`: `Some` cuando el mensaje viene de Evolution (el hilo LID, "salida" =
    /// lo que Max escribió de verdad — el polling de respaldo lo relee cada 20s, así que sin
    /// dedupe duplicaría sin fin); `None` para el envío iniciado por un peer vía
    /// `/whatsapp/enviar` (no viene de Evolution, no lo necesita).
    pub async fn whatsapp_registrar_salida(
        &self,
        texto: &str,
        autor_peer_id: Option<&str>,
        cuando: &str,
        evo_message_id: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO whatsapp_mensajes (direccion, texto, autor_peer_id, cuando, evo_message_id)
             VALUES ('salida', ?1, ?2, ?3, ?4)
             ON CONFLICT(evo_message_id) WHERE evo_message_id IS NOT NULL DO NOTHING",
        )
        .bind(texto)
        .bind(autor_peer_id)
        .bind(cuando)
        .bind(evo_message_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Historial completo de la conversación (ambas direcciones), MÁS RECIENTE PRIMERO. Mismo
    /// contrato de paginación que `listar` (cursor `desde` = id exclusivo hacia atrás). Lo
    /// consume la pantalla del desktop (RFC "integrado en mi app").
    pub async fn whatsapp_historial(
        &self,
        desde: Option<i64>,
        limite: i64,
    ) -> anyhow::Result<Vec<MensajeWhatsapp>> {
        let limite = limite.clamp(1, 500);
        let filas: Vec<SqliteRow> = match desde {
            Some(d) => {
                sqlx::query(
                    "SELECT id, direccion, texto, nombre, autor_peer_id, cuando
                     FROM whatsapp_mensajes WHERE id < ?1 ORDER BY id DESC LIMIT ?2",
                )
                .bind(d)
                .bind(limite)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT id, direccion, texto, nombre, autor_peer_id, cuando
                     FROM whatsapp_mensajes ORDER BY id DESC LIMIT ?1",
                )
                .bind(limite)
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(filas.iter().filter_map(fila_a_mensaje_whatsapp).collect())
    }
}

/// Upsert de identidad durable (ADR-001): registra la primera vez y refresca la última.
/// JAMÁS se borra de `peers_conocidos` — ni kick ni vencimiento tocan esta tabla.
async fn upsert_peer(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    visto: &str,
    resumen: Option<&str>,
    directorio: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        // COALESCE en resumen y directorio: un caller que no los conoce (p. ej. `registrar` de una
        // acción, que sólo tiene el id del actor) pasa None y NO debe borrar lo que ya se sabía.
        // Sólo `peer_visto` — que viene del registro, donde el peer los declara — los refresca.
        "INSERT INTO peers_conocidos (id, primer_visto, ultimo_visto, ultimo_resumen, directorio)
         VALUES (?1, ?2, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
           ultimo_visto = excluded.ultimo_visto,
           ultimo_resumen = COALESCE(excluded.ultimo_resumen, peers_conocidos.ultimo_resumen),
           directorio = COALESCE(excluded.directorio, peers_conocidos.directorio)",
    )
    .bind(id)
    .bind(visto)
    .bind(resumen)
    .bind(directorio)
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

/// Reconstruye un `MensajeWhatsapp` desde una fila. `None` si la fila es ilegible o trae una
/// `direccion` que este binario no conoce — degradación por fila, igual que `fila_a_accion`.
fn fila_a_mensaje_whatsapp(f: &SqliteRow) -> Option<MensajeWhatsapp> {
    let direccion_txt: String = f.try_get("direccion").ok()?;
    let direccion = match direccion_txt.as_str() {
        "entrada" => DireccionWhatsapp::Entrada,
        "salida" => DireccionWhatsapp::Salida,
        _ => return None,
    };
    Some(MensajeWhatsapp {
        id: f.try_get("id").ok()?,
        direccion,
        texto: f.try_get("texto").ok()?,
        nombre: f.try_get::<Option<String>, _>("nombre").ok().flatten(),
        autor_peer_id: f.try_get::<Option<String>, _>("autor_peer_id").ok().flatten(),
        cuando: f.try_get("cuando").ok()?,
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

    /// El directorio se ancla en la identidad durable y se puede leer de vuelta. Es lo que permite
    /// al Chat global filtrar por carpeta; la migración 0003 lo añade nullable sin romper nada.
    #[tokio::test]
    async fn peer_visto_ancla_el_directorio() {
        let b = bitacora_en_memoria().await;
        b.peer_visto("pa", "2026-01-01T00:00:01Z", Some("hago cosas"), Some("/repo/a"))
            .await
            .expect("peer_visto");
        let conocidos = b.peers_conocidos().await.expect("peers_conocidos");
        assert_eq!(conocidos.len(), 1);
        assert_eq!(conocidos[0].directorio.as_deref(), Some("/repo/a"));
        assert_eq!(conocidos[0].ultimo_resumen.as_deref(), Some("hago cosas"));
    }

    /// Un peer que MUEVE de carpeta actualiza el directorio; pero una acción posterior (que no
    /// sabe el directorio y pasa None) NO puede borrarlo — de ahí el COALESCE. Sin él, el primer
    /// mensaje que enviara el peer dejaría su directorio a NULL y el filtro se vaciaría solo.
    #[tokio::test]
    async fn el_directorio_se_refresca_pero_una_accion_no_lo_borra() {
        let b = bitacora_en_memoria().await;
        b.peer_visto("pa", "2026-01-01T00:00:01Z", None, Some("/repo/viejo")).await.unwrap();
        b.peer_visto("pa", "2026-01-01T00:00:02Z", None, Some("/repo/nuevo")).await.unwrap();
        assert_eq!(
            b.peers_conocidos().await.unwrap()[0].directorio.as_deref(),
            Some("/repo/nuevo"),
            "un re-registro desde otra carpeta debe seguir al peer"
        );
        // Una acción cualquiera (upsert con directorio None) no debe pisar lo conocido.
        b.registrar(&accion("pa", "2026-01-01T00:00:03Z"), None).await.unwrap();
        assert_eq!(
            b.peers_conocidos().await.unwrap()[0].directorio.as_deref(),
            Some("/repo/nuevo"),
            "una acción sin directorio NO puede borrar el ya anclado"
        );
    }

    /// La identidad durable sobrevive a cualquier peer: `peers_conocidos` los devuelve todos,
    /// ordenados, hayan salido o no (aquí no hay `salir` — esa tabla sencillamente no se borra).
    #[tokio::test]
    async fn peers_conocidos_lista_todos_ordenados() {
        let b = bitacora_en_memoria().await;
        b.peer_visto("pz", "2026-01-01T00:00:01Z", None, Some("/z")).await.unwrap();
        b.peer_visto("pa", "2026-01-01T00:00:02Z", None, Some("/a")).await.unwrap();
        let ids: Vec<_> = b
            .peers_conocidos()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec!["pa", "pz"], "orden estable por id");
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

    /// Un reintento del webhook (mismo `evo_message_id`) NO duplica la fila — el índice único
    /// PARCIAL (0004) exige repetir el `WHERE` en el `ON CONFLICT`; sin eso, SQLite rechaza el
    /// INSERT entero con "no matching unique/primary key" (bug real, comprobado en vivo contra
    /// el broker corriendo antes de este test).
    #[tokio::test]
    async fn whatsapp_entrada_deduplica_por_evo_message_id() {
        let b = bitacora_en_memoria().await;
        let primero = b
            .whatsapp_registrar_entrada("oi", Some("Letícia"), "2026-01-01T00:00:01Z", "MSG-1")
            .await
            .expect("primer registro");
        assert!(primero, "primera vez: debe insertar");
        let repetido = b
            .whatsapp_registrar_entrada("oi", Some("Letícia"), "2026-01-01T00:00:02Z", "MSG-1")
            .await
            .expect("reintento no debe fallar");
        assert!(!repetido, "reintento con el mismo evo_message_id: no debe duplicar");
        let historial = b.whatsapp_historial(None, 100).await.expect("historial");
        assert_eq!(historial.len(), 1, "solo una fila pese al reintento");
    }

    /// Dos mensajes de ENTRADA sin `evo_message_id` en común (o de `SALIDA`, que nunca lo lleva)
    /// no chocan entre sí: el índice parcial permite múltiples NULL.
    #[tokio::test]
    async fn whatsapp_historial_mezcla_entrada_y_salida_en_orden() {
        let b = bitacora_en_memoria().await;
        b.whatsapp_registrar_entrada("oi", Some("Letícia"), "2026-01-01T00:00:01Z", "MSG-A")
            .await
            .expect("entrada");
        b.whatsapp_registrar_salida("oi de volta", Some("claude-peers-rs-s021"), "2026-01-01T00:00:02Z", None)
            .await
            .expect("salida");
        let historial = b.whatsapp_historial(None, 100).await.expect("historial");
        assert_eq!(historial.len(), 2);
        assert_eq!(historial[0].direccion, DireccionWhatsapp::Salida, "más reciente primero");
        assert_eq!(historial[1].direccion, DireccionWhatsapp::Entrada);
    }

    /// El hilo LID (diálogo humano real en ambas direcciones) también deduplica por
    /// `evo_message_id` del lado de SALIDA — sin esto, el polling de respaldo (cada 20s, relee
    /// todo el hilo) duplicaría cada mensaje de Max una vez por ciclo.
    #[tokio::test]
    async fn whatsapp_salida_con_evo_message_id_deduplica() {
        let b = bitacora_en_memoria().await;
        b.whatsapp_registrar_salida("mor vou dormir", None, "2026-01-01T00:00:01Z", Some("MSG-MAX-1"))
            .await
            .expect("primer registro");
        b.whatsapp_registrar_salida("mor vou dormir", None, "2026-01-01T00:00:02Z", Some("MSG-MAX-1"))
            .await
            .expect("reintento no debe fallar");
        let historial = b.whatsapp_historial(None, 100).await.expect("historial");
        assert_eq!(historial.len(), 1, "el polling repetido no debe duplicar los mensajes de Max");
    }
}
