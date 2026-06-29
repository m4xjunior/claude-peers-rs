//! `AlmacenSqlite` — implementación del trait `Almacen` sobre SQLite (tras feature `sqlite`).
//!
//! Backend alternativo al Redis por defecto: da el binario 100% autocontenido (SQLite va
//! embebido con rusqlite "bundled"). El trait es async pero rusqlite es síncrono; como el
//! broker es de instancia única y las operaciones son rápidas, ejecutamos inline tras un
//! `Mutex` (bloqueo breve, sin pool). Es la opción para "1 binario y corre, sin Redis".

use async_trait::async_trait;
use peers_core::{Alcance, Almacen, Instancia, ItemOutbox, Mensaje, Sesion, Tarea};
use rusqlite::{params, Connection};
use std::sync::Mutex;

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
                texto TEXT NOT NULL, enviado_en TEXT NOT NULL, entregado INTEGER NOT NULL DEFAULT 0
            );
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
                duracion_seg INTEGER, issue_number INTEGER
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
        // SOLO LECTURA: COUNT no consume (recibir_mensajes sí marca entregado=1).
        Ok(self
            .bloquear()
            .query_row(
                "SELECT COUNT(*) FROM mensajes WHERE para_id=?1 AND entregado=0",
                params![id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize)
    }

    async fn purgar(&self, id: &str) -> anyhow::Result<()> {
        let conexion = self.bloquear();
        // Borra la cola de mensajes y el outbox de ESTE destinatario. No da de baja la
        // instancia ni toca su jornada. Idempotente (borra 0 o más filas).
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
            "INSERT INTO mensajes (de_id,para_id,texto,enviado_en,entregado) VALUES (?1,?2,?3,?4,0)",
            params![de_id, para_id, texto, ahora],
        )?;
        Ok(())
    }

    async fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>> {
        let mut conexion = self.bloquear();
        let tx = conexion.transaction()?;
        let mut msgs = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id,de_id,para_id,texto,enviado_en,entregado FROM mensajes
                 WHERE para_id=?1 AND entregado=0 ORDER BY enviado_en ASC, id ASC",
            )?;
            let filas = stmt.query_map(params![id], |f| {
                Ok(Mensaje {
                    id: f.get(0)?,
                    de_id: f.get(1)?,
                    para_id: f.get(2)?,
                    texto: f.get(3)?,
                    enviado_en: f.get(4)?,
                    entregado: f.get::<_, i64>(5)? != 0,
                })
            })?;
            for m in filas {
                msgs.push(m?);
            }
            for m in &msgs {
                tx.execute("UPDATE mensajes SET entregado=1 WHERE id=?1", params![m.id])?;
            }
        }
        tx.commit()?;
        Ok(msgs)
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
            conexion.execute(
                "DELETE FROM mensajes WHERE para_id=?1 AND entregado=0",
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
             (id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                t.id, t.instancia_id, t.sesion_id, t.descripcion, t.inicio, t.fin,
                t.duracion_seg, t.issue_number.map(|n| n as i64)
            ],
        )?;
        Ok(())
    }

    async fn tarea_obtener(&self, tarea_id: &str) -> anyhow::Result<Option<Tarea>> {
        let conexion = self.bloquear();
        Ok(conexion
            .query_row(
                "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number
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
                "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number
                 FROM tareas WHERE instancia_id=?1 ORDER BY inicio ASC",
            )?;
            let filas = stmt.query_map(params![instancia_id], fila_a_tarea)?;
            filas.filter_map(Result::ok).collect()
        };
        Ok((sesiones, tareas))
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
    async fn recibir_no_duplica() {
        let b = base();
        b.registrar("a", 1, "/x", None, None, None, "", "2026-01-01T00:00:00Z").await.unwrap();
        b.encolar_mensaje("b", "a", "uno", "2026-01-01T00:00:01Z").await.unwrap();
        assert_eq!(b.recibir_mensajes("a").await.unwrap().len(), 1);
        assert_eq!(b.recibir_mensajes("a").await.unwrap().len(), 0);
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
}
