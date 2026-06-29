//! `AlmacenSqlite` — implementación del trait `Almacen` sobre SQLite (tras feature `sqlite`).
//!
//! Backend alternativo al Redis por defecto: da el binario 100% autocontenido (SQLite va
//! embebido con rusqlite "bundled"). El trait es async pero rusqlite es síncrono; como el
//! broker es de instancia única y las operaciones son rápidas, ejecutamos inline tras un
//! `Mutex` (bloqueo breve, sin pool). Es la opción para "1 binario y corre, sin Redis".

use async_trait::async_trait;
use peers_core::{
    aplicar_media_movil, Alcance, Alerta, Almacen, EstadoMensaje, FactorEstimacion, Instancia,
    ItemOutbox, Mensaje, Sesion, Tarea, TipoAlerta, MAX_ALERTAS,
};
use rusqlite::{params, Connection};
use std::sync::Mutex;

/// Serializa un `EstadoMensaje` a su forma textual (lowercase) para la columna `estado`.
fn estado_a_texto(e: EstadoMensaje) -> &'static str {
    match e {
        EstadoMensaje::Enviado => "enviado",
        EstadoMensaje::Entregado => "entregado",
        EstadoMensaje::Leido => "leido",
        EstadoMensaje::Procesado => "procesado",
        EstadoMensaje::Fallido => "fallido",
        EstadoMensaje::DeadLetter => "deadletter",
    }
}

/// Parsea la columna `estado` a `EstadoMensaje` (cae a `Enviado` si es desconocida).
fn texto_a_estado(s: &str) -> EstadoMensaje {
    match s {
        "entregado" => EstadoMensaje::Entregado,
        "leido" => EstadoMensaje::Leido,
        "procesado" => EstadoMensaje::Procesado,
        "fallido" => EstadoMensaje::Fallido,
        "deadletter" => EstadoMensaje::DeadLetter,
        _ => EstadoMensaje::Enviado,
    }
}

/// Forma textual lowercase de un `TipoAlerta` (coincide con el `rename_all` de serde).
fn tipo_alerta_a_texto(t: TipoAlerta) -> &'static str {
    match t {
        TipoAlerta::Ocioso => "ocioso",
        TipoAlerta::Atascado => "atascado",
        TipoAlerta::Ghosteo => "ghosteo",
    }
}

/// Parsea la columna `tipo` a `TipoAlerta` (cae a `Ocioso` si es desconocida — defensivo).
fn texto_a_tipo_alerta(s: &str) -> TipoAlerta {
    match s {
        "atascado" => TipoAlerta::Atascado,
        "ghosteo" => TipoAlerta::Ghosteo,
        _ => TipoAlerta::Ocioso,
    }
}

/// Reconstruye un Mensaje desde una fila con el orden de columnas:
/// id, de_id, para_id, texto, enviado_en, estado, entregado_en, leido_en, procesado_en,
/// intentos, reenviado_de, reenvios.
fn fila_a_mensaje(f: &rusqlite::Row<'_>) -> rusqlite::Result<Mensaje> {
    Ok(Mensaje {
        id: f.get(0)?,
        de_id: f.get(1)?,
        para_id: f.get(2)?,
        texto: f.get(3)?,
        enviado_en: f.get(4)?,
        estado: texto_a_estado(&f.get::<_, String>(5)?),
        entregado_en: f.get(6)?,
        leido_en: f.get(7)?,
        procesado_en: f.get(8)?,
        intentos: f.get::<_, i64>(9)? as u32,
        reenviado_de: f.get::<_, Option<i64>>(10)?,
        reenvios: f.get::<_, i64>(11)? as u32,
    })
}

/// Columnas del SELECT de mensajes en el orden que espera `fila_a_mensaje`.
const COLS_MSG: &str = "id,de_id,para_id,texto,enviado_en,estado,entregado_en,leido_en,procesado_en,intentos,reenviado_de,reenvios";

pub struct AlmacenSqlite {
    conexion: Mutex<Connection>,
}

impl AlmacenSqlite {
    /// Abre (o crea) la base y aplica el esquema (instancias + mensajes + outbox + jornada).
    pub fn abrir(ruta: &str) -> anyhow::Result<Self> {
        let conexion = Connection::open(ruta)?;
        conexion.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 3000;

            CREATE TABLE IF NOT EXISTS instancias (
                id TEXT PRIMARY KEY, pid INTEGER NOT NULL, directorio TEXT NOT NULL,
                repo_git TEXT, repo_github TEXT, tty TEXT, resumen TEXT NOT NULL DEFAULT '',
                registrada_en TEXT NOT NULL, visto_en TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS mensajes (
                id INTEGER PRIMARY KEY AUTOINCREMENT, de_id TEXT NOT NULL, para_id TEXT NOT NULL,
                texto TEXT NOT NULL, enviado_en TEXT NOT NULL,
                estado TEXT NOT NULL DEFAULT 'enviado',
                entregado_en TEXT, leido_en TEXT, procesado_en TEXT,
                intentos INTEGER NOT NULL DEFAULT 0,
                reenviado_de INTEGER, reenvios INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_mensajes_para ON mensajes(para_id, id);
            CREATE TABLE IF NOT EXISTS outbox (
                id TEXT PRIMARY KEY, para_id TEXT NOT NULL, texto TEXT NOT NULL,
                creado_en TEXT NOT NULL, confirmado INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS sesiones (
                id TEXT PRIMARY KEY, instancia_id TEXT NOT NULL, inicio TEXT NOT NULL,
                fin TEXT, duracion_seg INTEGER
            );
            CREATE TABLE IF NOT EXISTS tareas (
                id TEXT PRIMARY KEY, instancia_id TEXT NOT NULL, sesion_id TEXT NOT NULL,
                descripcion TEXT NOT NULL, inicio TEXT NOT NULL, fin TEXT,
                duracion_seg INTEGER, issue_number INTEGER, estimado_seg INTEGER
            );
            CREATE TABLE IF NOT EXISTS factor_estimacion (
                id INTEGER PRIMARY KEY CHECK(id=1),
                muestras INTEGER NOT NULL DEFAULT 0,
                factor REAL NOT NULL DEFAULT 1.0,
                actualizado_en TEXT NOT NULL DEFAULT ''
            );
            -- Supervisor (fase 5): cola de alertas + set de activas para idempotencia (R5/R7).
            -- `alertas` es la LIST acotada (rowid asc = orden de emisión, se poda a MAX_ALERTAS).
            -- `alertas_activas` es el SET (PK tipo+sujeto): una alerta viva por par hasta resolver.
            CREATE TABLE IF NOT EXISTS alertas (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                tipo TEXT NOT NULL, sujeto TEXT NOT NULL,
                detalle TEXT NOT NULL, creada_en TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS alertas_activas (
                tipo TEXT NOT NULL, sujeto TEXT NOT NULL,
                PRIMARY KEY (tipo, sujeto)
            );
            "#,
        )?;
        Ok(Self {
            conexion: Mutex::new(conexion),
        })
    }

    fn bloquear(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conexion.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl Almacen for AlmacenSqlite {
    async fn registrar(
        &self,
        id: &str,
        pid: i64,
        directorio: &str,
        repo_git: Option<&str>,
        repo_github: Option<&str>,
        tty: Option<&str>,
        resumen: &str,
        ahora: &str,
    ) -> anyhow::Result<()> {
        let conexion = self.bloquear();
        let existe: bool = conexion
            .query_row("SELECT 1 FROM instancias WHERE id = ?1", params![id], |_| Ok(true))
            .unwrap_or(false);
        if existe {
            // Re-registro: UPDATE sin tocar la fila de mensajes ni registrada_en/resumen.
            conexion.execute(
                "UPDATE instancias SET pid=?2, directorio=?3, repo_git=?4, repo_github=?5, tty=?6, visto_en=?7 WHERE id=?1",
                params![id, pid, directorio, repo_git, repo_github, tty, ahora],
            )?;
        } else {
            conexion.execute(
                "INSERT INTO instancias (id,pid,directorio,repo_git,repo_github,tty,resumen,registrada_en,visto_en)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",
                params![id, pid, directorio, repo_git, repo_github, tty, resumen, ahora],
            )?;
        }
        Ok(())
    }

    async fn latido(&self, id: &str, ahora: &str) -> anyhow::Result<()> {
        self.bloquear()
            .execute("UPDATE instancias SET visto_en=?2 WHERE id=?1", params![id, ahora])?;
        Ok(())
    }

    async fn definir_resumen(&self, id: &str, resumen: &str) -> anyhow::Result<()> {
        self.bloquear()
            .execute("UPDATE instancias SET resumen=?2 WHERE id=?1", params![id, resumen])?;
        Ok(())
    }

    async fn salir(&self, id: &str) -> anyhow::Result<()> {
        self.bloquear()
            .execute("DELETE FROM instancias WHERE id=?1", params![id])?;
        Ok(())
    }

    async fn instancia_existe(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self
            .bloquear()
            .query_row("SELECT 1 FROM instancias WHERE id=?1", params![id], |_| Ok(true))
            .unwrap_or(false))
    }

    async fn contar_instancias(&self) -> anyhow::Result<usize> {
        Ok(self
            .bloquear()
            .query_row("SELECT COUNT(*) FROM instancias", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize)
    }

    async fn listar_ids(&self) -> anyhow::Result<Vec<String>> {
        let conexion = self.bloquear();
        // Estado crudo (sin liveness): el panel de admin los quiere todos. Orden estable.
        let mut stmt = conexion.prepare("SELECT id FROM instancias ORDER BY id ASC")?;
        let filas = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(filas.filter_map(Result::ok).collect())
    }

    async fn contar_mensajes_pendientes(&self, id: &str) -> anyhow::Result<usize> {
        // SOLO LECTURA: cuenta la bandeja ACTIVA (estado no terminal). Los Procesado/DeadLetter
        // siguen en la tabla (historial durable) pero ya NO están activos.
        Ok(self
            .bloquear()
            .query_row(
                "SELECT COUNT(*) FROM mensajes WHERE para_id=?1 AND estado NOT IN ('procesado','deadletter')",
                params![id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize)
    }

    async fn purgar(&self, id: &str) -> anyhow::Result<()> {
        let conexion = self.bloquear();
        // Borra los mensajes (bandeja + historial, misma tabla) y el outbox de ESTE
        // destinatario. No da de baja la instancia ni toca su jornada. Idempotente.
        conexion.execute("DELETE FROM mensajes WHERE para_id=?1", params![id])?;
        conexion.execute("DELETE FROM outbox WHERE para_id=?1", params![id])?;
        Ok(())
    }

    async fn instancia_obtener(&self, id: &str) -> anyhow::Result<Option<Instancia>> {
        Ok(self
            .bloquear()
            .query_row(
                "SELECT * FROM instancias WHERE id=?1",
                params![id],
                fila_a_instancia,
            )
            .ok())
    }

    async fn listar(
        &self,
        alcance: Alcance,
        directorio: &str,
        repo_git: Option<&str>,
        excluir_id: Option<&str>,
        vencidas_antes: &str,
    ) -> anyhow::Result<Vec<Instancia>> {
        let conexion = self.bloquear();
        let (sql, bind): (&str, Vec<String>) = match alcance {
            Alcance::Maquina => ("SELECT * FROM instancias", vec![]),
            Alcance::Directorio => (
                "SELECT * FROM instancias WHERE directorio = ?1",
                vec![directorio.to_string()],
            ),
            Alcance::Repo => match repo_git {
                Some(rg) => ("SELECT * FROM instancias WHERE repo_git = ?1", vec![rg.to_string()]),
                None => (
                    "SELECT * FROM instancias WHERE directorio = ?1",
                    vec![directorio.to_string()],
                ),
            },
        };
        let mut stmt = conexion.prepare(sql)?;
        let filas = stmt.query_map(rusqlite::params_from_iter(bind.iter()), fila_a_instancia)?;
        let mut out = Vec::new();
        for f in filas {
            let inst = f?;
            if excluir_id.is_some_and(|ex| ex == inst.id) {
                continue;
            }
            if inst.visto_en.as_str() >= vencidas_antes {
                out.push(inst);
            }
        }
        Ok(out)
    }

    async fn encolar_mensaje(
        &self,
        de_id: &str,
        para_id: &str,
        texto: &str,
        ahora: &str,
    ) -> anyhow::Result<()> {
        self.bloquear().execute(
            "INSERT INTO mensajes (de_id,para_id,texto,enviado_en,estado) VALUES (?1,?2,?3,?4,'enviado')",
            params![de_id, para_id, texto, ahora],
        )?;
        Ok(())
    }

    async fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>> {
        // PEEK no-destructivo (R1.1): la bandeja ACTIVA son los mensajes no terminales.
        // NO se marca ni se borra nada aquí; las transiciones las hace transicionar_mensaje.
        let conexion = self.bloquear();
        let sql = format!(
            "SELECT {COLS_MSG} FROM mensajes
             WHERE para_id=?1 AND estado NOT IN ('procesado','deadletter')
             ORDER BY id ASC"
        );
        let mut stmt = conexion.prepare(&sql)?;
        let filas = stmt.query_map(params![id], fila_a_mensaje)?;
        Ok(filas.filter_map(Result::ok).collect())
    }

    async fn transicionar_mensaje(
        &self,
        msg_id: i64,
        nuevo: EstadoMensaje,
        ahora: &str,
    ) -> anyhow::Result<bool> {
        let conexion = self.bloquear();
        // Estado actual (None si no existe el mensaje → no-op).
        let actual: Option<String> = conexion
            .query_row("SELECT estado FROM mensajes WHERE id=?1", params![msg_id], |r| r.get(0))
            .ok();
        let Some(actual) = actual else {
            return Ok(false);
        };
        let actual = texto_a_estado(&actual);
        // Monótona: solo avanza si el rango nuevo es estrictamente mayor (idempotente si no).
        if nuevo.rango() <= actual.rango() {
            return Ok(false);
        }
        // Timbra el campo de tiempo SOLO la primera vez (COALESCE = el HSETNX de SQLite, R1.3).
        let col_tiempo = match nuevo {
            EstadoMensaje::Entregado => Some("entregado_en"),
            EstadoMensaje::Leido => Some("leido_en"),
            EstadoMensaje::Procesado => Some("procesado_en"),
            _ => None,
        };
        let estado_txt = estado_a_texto(nuevo);
        match col_tiempo {
            Some(col) => {
                // COALESCE preserva el primer timbre si ya existía (idempotencia del tiempo).
                let sql = format!(
                    "UPDATE mensajes SET estado=?2, {col}=COALESCE({col}, ?3) WHERE id=?1"
                );
                conexion.execute(&sql, params![msg_id, estado_txt, ahora])?;
            }
            None => {
                conexion.execute(
                    "UPDATE mensajes SET estado=?2 WHERE id=?1",
                    params![msg_id, estado_txt],
                )?;
            }
        }
        Ok(true)
    }

    async fn historial(
        &self,
        id: &str,
        desde: Option<i64>,
        estado: Option<EstadoMensaje>,
    ) -> anyhow::Result<Vec<Mensaje>> {
        let conexion = self.bloquear();
        // Historial durable = TODAS las filas de la cola (no se borran al procesar). Filtros
        // opcionales por cursor (id > desde) y por estado. Vincula los binds dinámicamente.
        let mut sql = format!("SELECT {COLS_MSG} FROM mensajes WHERE para_id=?1");
        if desde.is_some() {
            sql.push_str(" AND id > ?2");
        }
        let estado_txt = estado.map(estado_a_texto);
        if estado_txt.is_some() {
            // El índice del bind depende de si hubo `desde`.
            sql.push_str(if desde.is_some() { " AND estado = ?3" } else { " AND estado = ?2" });
        }
        sql.push_str(" ORDER BY id ASC");
        let mut stmt = conexion.prepare(&sql)?;
        // Construye los binds en orden.
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(id.to_string())];
        if let Some(d) = desde {
            binds.push(Box::new(d));
        }
        if let Some(e) = estado_txt {
            binds.push(Box::new(e.to_string()));
        }
        let filas = stmt.query_map(rusqlite::params_from_iter(binds.iter()), fila_a_mensaje)?;
        Ok(filas.filter_map(Result::ok).collect())
    }

    async fn mensaje_obtener(&self, msg_id: i64) -> anyhow::Result<Option<Mensaje>> {
        let conexion = self.bloquear();
        let sql = format!("SELECT {COLS_MSG} FROM mensajes WHERE id=?1");
        Ok(conexion.query_row(&sql, params![msg_id], fila_a_mensaje).ok())
    }

    async fn encolar_reenvio(&self, original: &Mensaje, ahora: &str) -> anyhow::Result<i64> {
        // Fila NUEVA (id autoincrement fresco) con estado 'enviado' y la traza de reenvío (R2.3).
        // La bandeja activa y el historial son la misma tabla; al nacer 'enviado' entra a ambas.
        let reenvios = original.reenvios + 1;
        let conexion = self.bloquear();
        conexion.execute(
            "INSERT INTO mensajes (de_id,para_id,texto,enviado_en,estado,intentos,reenviado_de,reenvios)
             VALUES (?1,?2,?3,?4,'enviado',0,?5,?6)",
            params![original.de_id, original.para_id, original.texto, ahora, original.id, reenvios],
        )?;
        Ok(conexion.last_insert_rowid())
    }

    async fn podar_historial(&self, retener: usize) -> anyhow::Result<()> {
        if retener == 0 {
            return Ok(());
        }
        let conexion = self.bloquear();
        // Por cada cola (para_id), borra todo lo que NO esté entre los últimos `retener` por id.
        // Subconsulta: los ids a CONSERVAR son los `retener` más recientes de cada para_id.
        conexion.execute(
            "DELETE FROM mensajes WHERE id IN (
                SELECT id FROM mensajes m
                WHERE (
                    SELECT COUNT(*) FROM mensajes m2
                    WHERE m2.para_id = m.para_id AND m2.id > m.id
                ) >= ?1
            )",
            params![retener as i64],
        )?;
        Ok(())
    }

    async fn limpiar_vencidas(&self, vencidas_antes: &str) -> anyhow::Result<usize> {
        let conexion = self.bloquear();
        let muertas: Vec<String> = {
            let mut stmt = conexion.prepare("SELECT id FROM instancias WHERE visto_en < ?1")?;
            let filas = stmt.query_map(params![vencidas_antes], |r| r.get::<_, String>(0))?;
            filas.filter_map(Result::ok).collect()
        };
        for id in &muertas {
            conexion.execute("DELETE FROM instancias WHERE id=?1", params![id])?;
            // Saca de la bandeja ACTIVA los mensajes aún no procesados de la instancia muerta
            // (→ deadletter) pero los CONSERVA en la tabla = historial durable (R2.1), en vez
            // de borrarlos. La poda por retención se ocupa luego del recorte.
            conexion.execute(
                "UPDATE mensajes SET estado='deadletter'
                 WHERE para_id=?1 AND estado NOT IN ('procesado','deadletter')",
                params![id],
            )?;
        }
        Ok(muertas.len())
    }

    // --- Outbox durable con ACK ---

    async fn outbox_encolar(&self, item: &ItemOutbox) -> anyhow::Result<()> {
        self.bloquear().execute(
            "INSERT OR REPLACE INTO outbox (id,para_id,texto,creado_en,confirmado)
             VALUES (?1,?2,?3,?4,?5)",
            params![item.id, item.para_id, item.texto, item.creado_en, item.confirmado as i64],
        )?;
        Ok(())
    }

    async fn outbox_pendientes(&self, para_id: &str) -> anyhow::Result<Vec<ItemOutbox>> {
        let conexion = self.bloquear();
        let mut stmt = conexion.prepare(
            "SELECT id,para_id,texto,creado_en,confirmado FROM outbox
             WHERE para_id=?1 AND confirmado=0 ORDER BY creado_en ASC",
        )?;
        let filas = stmt.query_map(params![para_id], |f| {
            Ok(ItemOutbox {
                id: f.get(0)?,
                para_id: f.get(1)?,
                texto: f.get(2)?,
                creado_en: f.get(3)?,
                confirmado: f.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(filas.filter_map(Result::ok).collect())
    }

    async fn outbox_confirmar(&self, item_id: &str) -> anyhow::Result<()> {
        self.bloquear()
            .execute("UPDATE outbox SET confirmado=1 WHERE id=?1", params![item_id])?;
        Ok(())
    }

    // --- Jornada ---

    async fn sesion_abrir(&self, s: &Sesion) -> anyhow::Result<()> {
        self.bloquear().execute(
            "INSERT OR REPLACE INTO sesiones (id,instancia_id,inicio,fin,duracion_seg)
             VALUES (?1,?2,?3,?4,?5)",
            params![s.id, s.instancia_id, s.inicio, s.fin, s.duracion_seg],
        )?;
        Ok(())
    }

    async fn sesion_cerrar(&self, instancia_id: &str, fin: &str) -> anyhow::Result<()> {
        let conexion = self.bloquear();
        // Última sesión abierta de la instancia.
        let fila: Option<(String, String)> = conexion
            .query_row(
                "SELECT id,inicio FROM sesiones WHERE instancia_id=?1 AND fin IS NULL
                 ORDER BY inicio DESC LIMIT 1",
                params![instancia_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        if let Some((id, inicio)) = fila {
            let dur = crate::jornada::diferencia_seg(&inicio, fin);
            conexion.execute(
                "UPDATE sesiones SET fin=?2, duracion_seg=?3 WHERE id=?1",
                params![id, fin, dur],
            )?;
        }
        Ok(())
    }

    async fn tarea_guardar(&self, t: &Tarea) -> anyhow::Result<()> {
        self.bloquear().execute(
            "INSERT OR REPLACE INTO tareas
             (id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                t.id, t.instancia_id, t.sesion_id, t.descripcion, t.inicio, t.fin,
                t.duracion_seg, t.issue_number.map(|n| n as i64), t.estimado_seg
            ],
        )?;
        Ok(())
    }

    async fn tarea_obtener(&self, tarea_id: &str) -> anyhow::Result<Option<Tarea>> {
        let conexion = self.bloquear();
        Ok(conexion
            .query_row(
                "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg
                 FROM tareas WHERE id=?1",
                params![tarea_id],
                fila_a_tarea,
            )
            .ok())
    }

    async fn jornada(&self, instancia_id: &str) -> anyhow::Result<(Vec<Sesion>, Vec<Tarea>)> {
        let conexion = self.bloquear();
        let sesiones = {
            let mut stmt = conexion
                .prepare("SELECT id,instancia_id,inicio,fin,duracion_seg FROM sesiones WHERE instancia_id=?1 ORDER BY inicio ASC")?;
            let filas = stmt.query_map(params![instancia_id], |r| {
                Ok(Sesion {
                    id: r.get(0)?,
                    instancia_id: r.get(1)?,
                    inicio: r.get(2)?,
                    fin: r.get(3)?,
                    duracion_seg: r.get(4)?,
                })
            })?;
            filas.filter_map(Result::ok).collect()
        };
        let tareas = {
            let mut stmt = conexion.prepare(
                "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg
                 FROM tareas WHERE instancia_id=?1 ORDER BY inicio ASC",
            )?;
            let filas = stmt.query_map(params![instancia_id], fila_a_tarea)?;
            filas.filter_map(Result::ok).collect()
        };
        Ok((sesiones, tareas))
    }

    async fn factor_estimacion(&self) -> anyhow::Result<FactorEstimacion> {
        let conexion = self.bloquear();
        let fila: Option<(i64, f64, String)> = conexion
            .query_row(
                "SELECT muestras,factor,actualizado_en FROM factor_estimacion WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        Ok(match fila {
            Some((muestras, factor, actualizado_en)) => FactorEstimacion {
                muestras: muestras.max(0) as u32,
                factor,
                actualizado_en,
            },
            // Default neutro: sin corrección, 0 muestras.
            None => FactorEstimacion {
                muestras: 0,
                factor: 1.0,
                actualizado_en: String::new(),
            },
        })
    }

    async fn actualizar_factor(&self, ratio: f64, ahora: &str) -> anyhow::Result<FactorEstimacion> {
        let actual = self.factor_estimacion().await?;
        let nuevo = FactorEstimacion {
            muestras: actual.muestras.saturating_add(1),
            factor: aplicar_media_movil(actual.factor, ratio),
            actualizado_en: ahora.to_string(),
        };
        // UPSERT sobre la fila única id=1 (el reloj lo pone el broker).
        self.bloquear().execute(
            "INSERT INTO factor_estimacion (id,muestras,factor,actualizado_en)
             VALUES (1,?1,?2,?3)
             ON CONFLICT(id) DO UPDATE SET
               muestras=excluded.muestras, factor=excluded.factor,
               actualizado_en=excluded.actualizado_en",
            params![nuevo.muestras as i64, nuevo.factor, nuevo.actualizado_en],
        )?;
        Ok(nuevo)
    }

    // --- Supervisor (fase 5): alertas ---

    async fn alerta_emitir(&self, a: &Alerta) -> anyhow::Result<bool> {
        let conexion = self.bloquear();
        let tipo = tipo_alerta_a_texto(a.tipo);
        // Idempotencia (R7): INSERT en el SET de activas; si el par (tipo,sujeto) ya existe, la
        // restricción de PK lo rechaza → 0 filas afectadas → no re-alertamos (AC1/AC4).
        let insertadas = conexion.execute(
            "INSERT OR IGNORE INTO alertas_activas (tipo,sujeto) VALUES (?1,?2)",
            params![tipo, a.sujeto],
        )?;
        if insertadas == 0 {
            return Ok(false);
        }
        // Encola la alerta y poda a las últimas MAX_ALERTAS (R5): borra todo lo que tenga al
        // menos MAX_ALERTAS filas con seq mayor (las más nuevas se conservan).
        conexion.execute(
            "INSERT INTO alertas (tipo,sujeto,detalle,creada_en) VALUES (?1,?2,?3,?4)",
            params![tipo, a.sujeto, a.detalle, a.creada_en],
        )?;
        conexion.execute(
            "DELETE FROM alertas WHERE seq IN (
                SELECT seq FROM alertas a
                WHERE (SELECT COUNT(*) FROM alertas a2 WHERE a2.seq > a.seq) >= ?1
            )",
            params![MAX_ALERTAS as i64],
        )?;
        Ok(true)
    }

    async fn alerta_resolver(&self, tipo: &str, sujeto: &str) -> anyhow::Result<()> {
        // Idempotente: DELETE de 0 o 1 filas. Al resolverse, el par vuelve a poder alertarse (AC2).
        self.bloquear().execute(
            "DELETE FROM alertas_activas WHERE tipo=?1 AND sujeto=?2",
            params![tipo, sujeto],
        )?;
        Ok(())
    }

    async fn alertas(&self) -> anyhow::Result<Vec<Alerta>> {
        let conexion = self.bloquear();
        let mut stmt = conexion
            .prepare("SELECT tipo,sujeto,detalle,creada_en FROM alertas ORDER BY seq ASC")?;
        let filas = stmt.query_map([], |f| {
            Ok(Alerta {
                tipo: texto_a_tipo_alerta(&f.get::<_, String>(0)?),
                sujeto: f.get(1)?,
                detalle: f.get(2)?,
                creada_en: f.get(3)?,
            })
        })?;
        Ok(filas.filter_map(Result::ok).collect())
    }

    async fn mensajes_en_estado(
        &self,
        estado: EstadoMensaje,
    ) -> anyhow::Result<Vec<(String, Mensaje)>> {
        let conexion = self.bloquear();
        // La tabla `mensajes` es bandeja + historial; filtramos por estado. El detector de
        // ghosteo (R4) pide los `Leido` no `Procesado`.
        let sql = format!("SELECT {COLS_MSG} FROM mensajes WHERE estado=?1 ORDER BY id ASC");
        let mut stmt = conexion.prepare(&sql)?;
        let estado_txt = estado_a_texto(estado);
        let filas = stmt.query_map(params![estado_txt], fila_a_mensaje)?;
        Ok(filas
            .filter_map(Result::ok)
            .map(|m| (m.para_id.clone(), m))
            .collect())
    }
}

fn fila_a_instancia(f: &rusqlite::Row<'_>) -> rusqlite::Result<Instancia> {
    Ok(Instancia {
        id: f.get("id")?,
        pid: f.get("pid")?,
        directorio: f.get("directorio")?,
        repo_git: f.get("repo_git")?,
        repo_github: f.get("repo_github")?,
        tty: f.get("tty")?,
        resumen: f.get("resumen")?,
        registrada_en: f.get("registrada_en")?,
        visto_en: f.get("visto_en")?,
    })
}

fn fila_a_tarea(f: &rusqlite::Row<'_>) -> rusqlite::Result<Tarea> {
    Ok(Tarea {
        id: f.get(0)?,
        instancia_id: f.get(1)?,
        sesion_id: f.get(2)?,
        descripcion: f.get(3)?,
        inicio: f.get(4)?,
        fin: f.get(5)?,
        duracion_seg: f.get(6)?,
        issue_number: f.get::<_, Option<i64>>(7)?.map(|n| n as u64),
        estimado_seg: f.get::<_, Option<i64>>(8)?,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn base() -> AlmacenSqlite {
        AlmacenSqlite::abrir(":memory:").expect("base en memoria")
    }

    #[tokio::test]
    async fn id_estable_reregistro_hereda_la_fila() {
        let b = base();
        b.registrar("jefin", 111, "/x", None, None, None, "papel", "2026-01-01T00:00:00Z").await.unwrap();
        b.registrar("claudia", 222, "/y", None, None, None, "papel", "2026-01-01T00:00:00Z").await.unwrap();
        b.encolar_mensaje("claudia", "jefin", "hola pre-restart", "2026-01-01T00:00:01Z").await.unwrap();
        // Restart: re-registro mismo id, pid distinto.
        b.registrar("jefin", 999, "/x", None, None, None, "papel", "2026-01-01T00:01:00Z").await.unwrap();
        let msgs = b.recibir_mensajes("jefin").await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].texto, "hola pre-restart");
    }

    #[tokio::test]
    async fn recibir_es_peek_no_destructivo() {
        // R1.1: recibir NO borra. Dos peeks seguidos devuelven el mismo mensaje. Solo sale de
        // la bandeja activa al transicionar a Procesado (R1.5), pero queda en historial (R2.1).
        let b = base();
        b.registrar("a", 1, "/x", None, None, None, "", "2026-01-01T00:00:00Z").await.unwrap();
        b.encolar_mensaje("b", "a", "uno", "2026-01-01T00:00:01Z").await.unwrap();
        let primera = b.recibir_mensajes("a").await.unwrap();
        assert_eq!(primera.len(), 1);
        assert_eq!(b.recibir_mensajes("a").await.unwrap().len(), 1, "peek no consume");
        let mid = primera[0].id;
        // Procesado → sale de la bandeja activa.
        assert!(b.transicionar_mensaje(mid, EstadoMensaje::Procesado, "2026-01-01T00:00:05Z").await.unwrap());
        assert_eq!(b.recibir_mensajes("a").await.unwrap().len(), 0, "tras Procesado sale de la bandeja");
        // Pero sigue en el historial durable.
        let h = b.historial("a", None, None).await.unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].estado, EstadoMensaje::Procesado);
    }

    #[tokio::test]
    async fn transicion_idempotente_y_monotona() {
        // R1.2/R1.3: el timbre del tiempo es solo la primera vez; no retrocede.
        let b = base();
        b.registrar("a", 1, "/x", None, None, None, "", "2026-01-01T00:00:00Z").await.unwrap();
        b.encolar_mensaje("b", "a", "uno", "2026-01-01T00:00:01Z").await.unwrap();
        let mid = b.recibir_mensajes("a").await.unwrap()[0].id;
        // Primera transición a Entregado → true, timbra entregado_en.
        assert!(b.transicionar_mensaje(mid, EstadoMensaje::Entregado, "2026-01-01T00:00:02Z").await.unwrap());
        let m1 = b.mensaje_obtener(mid).await.unwrap().unwrap();
        assert_eq!(m1.entregado_en.as_deref(), Some("2026-01-01T00:00:02Z"));
        // Segunda a Entregado → false (idempotente), no re-timbra.
        assert!(!b.transicionar_mensaje(mid, EstadoMensaje::Entregado, "2026-01-01T09:99:99Z").await.unwrap());
        let m2 = b.mensaje_obtener(mid).await.unwrap().unwrap();
        assert_eq!(m2.entregado_en.as_deref(), Some("2026-01-01T00:00:02Z"), "el timbre no cambia");
        // Retroceder a Enviado → false (no retrocede).
        assert!(!b.transicionar_mensaje(mid, EstadoMensaje::Enviado, "2026-01-01T00:00:03Z").await.unwrap());
    }

    #[tokio::test]
    async fn historial_filtra_y_retencion_recorta() {
        let b = base();
        b.registrar("a", 1, "/x", None, None, None, "", "2026-01-01T00:00:00Z").await.unwrap();
        for i in 0..5 {
            b.encolar_mensaje("b", "a", &format!("m{i}"), "2026-01-01T00:00:01Z").await.unwrap();
        }
        assert_eq!(b.historial("a", None, None).await.unwrap().len(), 5);
        // Retención a 3 → recorta a los 3 más recientes.
        b.podar_historial(3).await.unwrap();
        let h = b.historial("a", None, None).await.unwrap();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].texto, "m2");
        assert_eq!(h[2].texto, "m4");
        // Filtro por estado.
        let mid = h[0].id;
        b.transicionar_mensaje(mid, EstadoMensaje::Procesado, "2026-01-01T00:00:09Z").await.unwrap();
        let procesados = b.historial("a", None, Some(EstadoMensaje::Procesado)).await.unwrap();
        assert_eq!(procesados.len(), 1);
        assert_eq!(procesados[0].id, mid);
    }

    #[tokio::test]
    async fn reregistro_conserva_registrada_en_y_resumen() {
        let b = base();
        b.registrar("x", 1, "/d", None, None, None, "resumen original", "2026-01-01T00:00:00Z").await.unwrap();
        b.registrar("x", 2, "/d", None, None, None, "ignorado", "2026-02-02T00:00:00Z").await.unwrap();
        let inst = &b.listar(Alcance::Maquina, "/d", None, None, "1970-01-01T00:00:00Z").await.unwrap()[0];
        assert_eq!(inst.resumen, "resumen original");
        assert_eq!(inst.registrada_en, "2026-01-01T00:00:00Z");
        assert_eq!(inst.pid, 2);
    }

    #[tokio::test]
    async fn liveness_filtra_vencidas() {
        let b = base();
        b.registrar("viva", 1, "/d", None, None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
        b.registrar("muerta", 2, "/d", None, None, None, "", "2020-01-01T00:00:00Z").await.unwrap();
        let vivas = b.listar(Alcance::Maquina, "/d", None, None, "2026-06-27T11:59:00Z").await.unwrap();
        assert_eq!(vivas.len(), 1);
        assert_eq!(vivas[0].id, "viva");
    }

    #[tokio::test]
    async fn limpiar_purga_instancia_y_fila() {
        let b = base();
        b.registrar("zombie", 1, "/d", None, None, None, "", "2020-01-01T00:00:00Z").await.unwrap();
        b.encolar_mensaje("otro", "zombie", "x", "2020-01-01T00:00:01Z").await.unwrap();
        assert_eq!(b.limpiar_vencidas("2026-01-01T00:00:00Z").await.unwrap(), 1);
        assert!(!b.instancia_existe("zombie").await.unwrap());
        assert_eq!(b.recibir_mensajes("zombie").await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn listar_excluye_solicitante() {
        let b = base();
        b.registrar("yo", 1, "/d", None, None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
        b.registrar("otro", 2, "/d", None, None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
        let r = b.listar(Alcance::Maquina, "/d", None, Some("yo"), "1970-01-01T00:00:00Z").await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "otro");
    }

    #[tokio::test]
    async fn repo_sin_git_cae_a_directorio() {
        let b = base();
        b.registrar("a", 1, "/proj", Some("/proj"), None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
        b.registrar("b", 2, "/proj", None, None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
        let r = b.listar(Alcance::Repo, "/proj", None, None, "1970-01-01T00:00:00Z").await.unwrap();
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn outbox_sobrevive_y_ack() {
        // El item del outbox sobrevive (no se borra) hasta el ACK — base del "no perder
        // solicitud al reiniciar un peer".
        let b = base();
        let item = ItemOutbox {
            id: "ob1".into(),
            para_id: "jefin".into(),
            texto: "tarea a medio hacer".into(),
            creado_en: "2026-01-01T00:00:00Z".into(),
            confirmado: false,
        };
        b.outbox_encolar(&item).await.unwrap();
        // Sigue pendiente tras un "reinicio" (releer).
        assert_eq!(b.outbox_pendientes("jefin").await.unwrap().len(), 1);
        b.outbox_confirmar("ob1").await.unwrap();
        // Tras el ACK, ya no está pendiente.
        assert_eq!(b.outbox_pendientes("jefin").await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn jornada_timbrada_por_el_broker() {
        let b = base();
        let s = Sesion {
            id: "s1".into(),
            instancia_id: "jefin".into(),
            inicio: "2026-01-01T00:00:00Z".into(),
            fin: None,
            duracion_seg: None,
        };
        b.sesion_abrir(&s).await.unwrap();
        let t = Tarea {
            id: "t1".into(),
            instancia_id: "jefin".into(),
            sesion_id: "s1".into(),
            descripcion: "algo".into(),
            inicio: "2026-01-01T00:00:00Z".into(),
            fin: None,
            duracion_seg: None,
            issue_number: None,
            estimado_seg: None,
        };
        b.tarea_guardar(&t).await.unwrap();
        // Cierre con tiempo del "broker": 90s después.
        let mut cerrada = b.tarea_obtener("t1").await.unwrap().unwrap();
        cerrada.fin = Some("2026-01-01T00:01:30Z".into());
        cerrada.duracion_seg = Some(crate::jornada::diferencia_seg("2026-01-01T00:00:00Z", "2026-01-01T00:01:30Z"));
        b.tarea_guardar(&cerrada).await.unwrap();
        let (_, tareas) = b.jornada("jefin").await.unwrap();
        assert_eq!(tareas.len(), 1);
        assert_eq!(tareas[0].duracion_seg, Some(90)); // MEDIDO, no estimado
    }

    #[tokio::test]
    async fn factor_default_aprende_clampa_y_upsert() {
        // R2/R3/R4 + AC1/AC4 sobre SQLite: default neutro → un paso desde 1.0 con ratio 120 da
        // 36.7 (un solo paso no llega al techo, fórmula real); un segundo ratio enorme (200)
        // empuja por encima de 50 → clamp; un ratio 2 mueve por media móvil. UPSERT id=1.
        let b = base();
        let f0 = b.factor_estimacion().await.unwrap();
        assert_eq!(f0.muestras, 0);
        assert_eq!(f0.factor, 1.0);
        assert_eq!(f0.actualizado_en, "");

        // Paso 1: 1 + 0.3*(120-1) = 36.7. muestras=1.
        let f1 = b.actualizar_factor(120.0, "2026-01-01T00:00:00Z").await.unwrap();
        assert_eq!(f1.muestras, 1);
        assert!((f1.factor - 36.7).abs() < 1e-9, "1 + 0.3*(120-1) = 36.7, fue {}", f1.factor);
        assert_eq!(f1.actualizado_en, "2026-01-01T00:00:00Z");

        // Paso 2: 36.7 + 0.3*(200-36.7) = 85.69 → clamp al techo 50. muestras=2.
        let f2 = b.actualizar_factor(200.0, "2026-01-01T00:00:15Z").await.unwrap();
        assert_eq!(f2.muestras, 2);
        assert_eq!(f2.factor, 50.0, "ratio extremo desde factor alto se clampa al techo");

        // Paso 3: media móvil hacia abajo: 50 + 0.3*(2-50) = 35.6. muestras=3.
        let f3 = b.actualizar_factor(2.0, "2026-01-01T00:00:30Z").await.unwrap();
        assert_eq!(f3.muestras, 3);
        assert!((f3.factor - 35.6).abs() < 1e-9, "50 + 0.3*(2-50) = 35.6, fue {}", f3.factor);

        // Relee de la base (no del valor devuelto): la fila persiste vía UPSERT.
        let leido = b.factor_estimacion().await.unwrap();
        assert_eq!(leido.muestras, 3);
        assert!((leido.factor - 35.6).abs() < 1e-9);
        assert_eq!(leido.actualizado_en, "2026-01-01T00:00:30Z");
    }

    #[tokio::test]
    async fn alerta_emitir_es_idempotente_por_tipo_y_sujeto() {
        // R7/AC4: la primera emisión de (ghosteo, msg:42) devuelve true y encola; una segunda
        // emisión del MISMO par devuelve false y NO duplica en la cola. Tras resolver, vuelve
        // a poder emitir (AC2). Una alerta de OTRO sujeto sí se emite.
        let b = base();
        let a1 = Alerta {
            tipo: TipoAlerta::Ghosteo,
            sujeto: "msg:42".into(),
            detalle: "leído sin procesar".into(),
            creada_en: "2026-06-29T15:00:00Z".into(),
        };
        assert!(b.alerta_emitir(&a1).await.unwrap(), "primera emisión");
        assert!(!b.alerta_emitir(&a1).await.unwrap(), "duplicado no re-alerta");
        assert_eq!(b.alertas().await.unwrap().len(), 1, "cola no duplica");

        // Otro sujeto → alerta distinta, sí se emite.
        let a2 = Alerta {
            tipo: TipoAlerta::Ocioso,
            sujeto: "claudia".into(),
            detalle: "10min sin tarea".into(),
            creada_en: "2026-06-29T15:01:00Z".into(),
        };
        assert!(b.alerta_emitir(&a2).await.unwrap());
        assert_eq!(b.alertas().await.unwrap().len(), 2);

        // Resolver el ghosteo → vuelve a poder alertarse (AC2).
        b.alerta_resolver("ghosteo", "msg:42").await.unwrap();
        assert!(b.alerta_emitir(&a1).await.unwrap(), "tras resolver re-alerta");
        assert_eq!(b.alertas().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn mensajes_en_estado_filtra_leido() {
        // R4: el detector de ghosteo pide los Leido (no Procesado). Devuelve (para_id, Mensaje).
        let b = base();
        b.registrar("a", 1, "/x", None, None, None, "", "2026-01-01T00:00:00Z").await.unwrap();
        b.encolar_mensaje("b", "a", "uno", "2026-01-01T00:00:01Z").await.unwrap();
        b.encolar_mensaje("b", "a", "dos", "2026-01-01T00:00:02Z").await.unwrap();
        let ids: Vec<i64> = b.recibir_mensajes("a").await.unwrap().iter().map(|m| m.id).collect();
        // Sube el primero a Leido; el segundo se queda Enviado.
        b.transicionar_mensaje(ids[0], EstadoMensaje::Leido, "2026-01-01T00:00:03Z").await.unwrap();
        let leidos = b.mensajes_en_estado(EstadoMensaje::Leido).await.unwrap();
        assert_eq!(leidos.len(), 1);
        assert_eq!(leidos[0].0, "a");
        assert_eq!(leidos[0].1.id, ids[0]);
    }

    #[tokio::test]
    async fn tarea_roundtrip_conserva_estimado_seg() {
        // R1 en SQLite: el estimado se persiste y se relee (la columna nueva no se pierde).
        let b = base();
        let t = Tarea {
            id: "te".into(),
            instancia_id: "jefin".into(),
            sesion_id: "s1".into(),
            descripcion: "con estimado".into(),
            inicio: "2026-01-01T00:00:00Z".into(),
            fin: None,
            duracion_seg: None,
            issue_number: None,
            estimado_seg: Some(432000),
        };
        b.tarea_guardar(&t).await.unwrap();
        let leida = b.tarea_obtener("te").await.unwrap().unwrap();
        assert_eq!(leida.estimado_seg, Some(432000));
    }
}
