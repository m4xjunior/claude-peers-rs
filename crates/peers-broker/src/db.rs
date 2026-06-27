//! Capa de persistencia SQLite del broker.
//!
//! Encapsula todo el acceso a la base tras un `Mutex<Connection>`. rusqlite no es `Sync`
//! por conexión, así que serializamos el acceso; para esta carga (decenas de instancias,
//! mensajes esporádicos) sobra y evita la complejidad de un pool.

use peers_core::{Alcance, Instancia, Mensaje};
use rusqlite::{params, Connection};
use std::sync::Mutex;

/// Estado compartido del broker: la conexión SQLite serializada.
pub struct Base {
    conexion: Mutex<Connection>,
}

impl Base {
    /// Abre (o crea) la base en `ruta` y aplica el esquema. WAL + busy_timeout para
    /// tolerar accesos concurrentes del propio proceso.
    pub fn abrir(ruta: &str) -> anyhow::Result<Self> {
        let conexion = Connection::open(ruta)?;
        conexion.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 3000;

            CREATE TABLE IF NOT EXISTS instancias (
                id TEXT PRIMARY KEY,
                pid INTEGER NOT NULL,
                directorio TEXT NOT NULL,
                repo_git TEXT,
                tty TEXT,
                resumen TEXT NOT NULL DEFAULT '',
                registrada_en TEXT NOT NULL,
                visto_en TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS mensajes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                de_id TEXT NOT NULL,
                para_id TEXT NOT NULL,
                texto TEXT NOT NULL,
                enviado_en TEXT NOT NULL,
                entregado INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )?;
        Ok(Self {
            conexion: Mutex::new(conexion),
        })
    }

    fn bloquear(&self) -> std::sync::MutexGuard<'_, Connection> {
        // Si el Mutex queda envenenado el proceso ya está inconsistente; preferimos
        // recuperar el guard antes que propagar panic en cada handler.
        self.conexion.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registro con ID ESTABLE — el FIX #1 frente al TS.
    ///
    /// - Si `id` ya existe: UPDATE de sus datos (pid/directorio/tty/visto_en) SIN tocar la
    ///   tabla `mensajes`. La fila pendiente (entregado=0 hacia ese id) SOBREVIVE al reinicio.
    /// - Si no existe: INSERT nuevo.
    ///
    /// En el TS, el registro borraba la instancia por PID y creaba un id aleatorio nuevo,
    /// dejando la fila vieja huérfana. Aquí el id es la identidad estable del papel.
    pub fn registrar(
        &self,
        id: &str,
        pid: i64,
        directorio: &str,
        repo_git: Option<&str>,
        tty: Option<&str>,
        resumen: &str,
        ahora: &str,
    ) -> anyhow::Result<()> {
        let conexion = self.bloquear();
        let existe: bool = conexion
            .query_row("SELECT 1 FROM instancias WHERE id = ?1", params![id], |_| {
                Ok(true)
            })
            .unwrap_or(false);

        if existe {
            // UPDATE: conserva registrada_en original y, sobre todo, la fila de mensajes.
            conexion.execute(
                "UPDATE instancias SET pid = ?2, directorio = ?3, repo_git = ?4, tty = ?5, visto_en = ?6 WHERE id = ?1",
                params![id, pid, directorio, repo_git, tty, ahora],
            )?;
        } else {
            conexion.execute(
                "INSERT INTO instancias (id, pid, directorio, repo_git, tty, resumen, registrada_en, visto_en)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, pid, directorio, repo_git, tty, resumen, ahora],
            )?;
        }
        Ok(())
    }

    pub fn latido(&self, id: &str, ahora: &str) -> anyhow::Result<()> {
        self.bloquear().execute(
            "UPDATE instancias SET visto_en = ?2 WHERE id = ?1",
            params![id, ahora],
        )?;
        Ok(())
    }

    pub fn definir_resumen(&self, id: &str, resumen: &str) -> anyhow::Result<()> {
        self.bloquear().execute(
            "UPDATE instancias SET resumen = ?2 WHERE id = ?1",
            params![id, resumen],
        )?;
        Ok(())
    }

    pub fn salir(&self, id: &str) -> anyhow::Result<()> {
        self.bloquear()
            .execute("DELETE FROM instancias WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn instancia_existe(&self, id: &str) -> bool {
        self.bloquear()
            .query_row("SELECT 1 FROM instancias WHERE id = ?1", params![id], |_| {
                Ok(true)
            })
            .unwrap_or(false)
    }

    pub fn contar_instancias(&self) -> usize {
        self.bloquear()
            .query_row("SELECT COUNT(*) FROM instancias", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    /// Lista instancias según alcance, excluyendo al solicitante y a las muertas por latido.
    /// `vencidas_antes` es el timestamp ISO límite: viva si visto_en >= vencidas_antes.
    pub fn listar(
        &self,
        alcance: Alcance,
        directorio: &str,
        repo_git: Option<&str>,
        excluir_id: Option<&str>,
        vencidas_antes: &str,
    ) -> anyhow::Result<Vec<Instancia>> {
        let conexion = self.bloquear();
        // Cláusula según alcance. repo sin repo_git cae a directorio.
        let (sql, bind): (&str, Vec<String>) = match alcance {
            Alcance::Maquina => ("SELECT * FROM instancias", vec![]),
            Alcance::Directorio => (
                "SELECT * FROM instancias WHERE directorio = ?1",
                vec![directorio.to_string()],
            ),
            Alcance::Repo => match repo_git {
                Some(rg) => (
                    "SELECT * FROM instancias WHERE repo_git = ?1",
                    vec![rg.to_string()],
                ),
                None => (
                    "SELECT * FROM instancias WHERE directorio = ?1",
                    vec![directorio.to_string()],
                ),
            },
        };

        let mut stmt = conexion.prepare(sql)?;
        let filas = stmt.query_map(rusqlite::params_from_iter(bind.iter()), fila_a_instancia)?;

        let mut instancias = Vec::new();
        for fila in filas {
            let inst = fila?;
            if excluir_id.is_some_and(|ex| ex == inst.id) {
                continue;
            }
            // Liveness por latido: solo vivas. Las muertas se filtran (no se devuelven).
            if inst.visto_en.as_str() >= vencidas_antes {
                instancias.push(inst);
            }
        }
        Ok(instancias)
    }

    /// Encola un mensaje. El destino debe existir (lo verifica el handler antes).
    pub fn encolar_mensaje(
        &self,
        de_id: &str,
        para_id: &str,
        texto: &str,
        ahora: &str,
    ) -> anyhow::Result<()> {
        self.bloquear().execute(
            "INSERT INTO mensajes (de_id, para_id, texto, enviado_en, entregado)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![de_id, para_id, texto, ahora],
        )?;
        Ok(())
    }

    /// Devuelve los mensajes no entregados de `id` (orden FIFO) y los marca entregado=1
    /// en la MISMA transacción, para que un recibir concurrente no los duplique.
    pub fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>> {
        let mut conexion = self.bloquear();
        let tx = conexion.transaction()?;
        let mut mensajes = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id, de_id, para_id, texto, enviado_en, entregado
                 FROM mensajes WHERE para_id = ?1 AND entregado = 0 ORDER BY enviado_en ASC, id ASC",
            )?;
            let filas = stmt.query_map(params![id], |fila| {
                Ok(Mensaje {
                    id: fila.get(0)?,
                    de_id: fila.get(1)?,
                    para_id: fila.get(2)?,
                    texto: fila.get(3)?,
                    enviado_en: fila.get(4)?,
                    entregado: fila.get::<_, i64>(5)? != 0,
                })
            })?;
            for m in filas {
                mensajes.push(m?);
            }
            for m in &mensajes {
                tx.execute(
                    "UPDATE mensajes SET entregado = 1 WHERE id = ?1",
                    params![m.id],
                )?;
            }
        }
        tx.commit()?;
        Ok(mensajes)
    }

    /// Elimina instancias muertas (visto_en < vencidas_antes) y purga su fila pendiente.
    /// Devuelve cuántas se limpiaron (para log).
    pub fn limpiar_vencidas(&self, vencidas_antes: &str) -> anyhow::Result<usize> {
        let conexion = self.bloquear();
        let muertas: Vec<String> = {
            let mut stmt = conexion.prepare("SELECT id FROM instancias WHERE visto_en < ?1")?;
            let filas = stmt.query_map(params![vencidas_antes], |r| r.get::<_, String>(0))?;
            filas.filter_map(Result::ok).collect()
        };
        for id in &muertas {
            conexion.execute("DELETE FROM instancias WHERE id = ?1", params![id])?;
            conexion.execute(
                "DELETE FROM mensajes WHERE para_id = ?1 AND entregado = 0",
                params![id],
            )?;
        }
        Ok(muertas.len())
    }
}

/// Mapea una fila completa de `instancias` a un struct Instancia.
fn fila_a_instancia(fila: &rusqlite::Row<'_>) -> rusqlite::Result<Instancia> {
    Ok(Instancia {
        id: fila.get("id")?,
        pid: fila.get("pid")?,
        directorio: fila.get("directorio")?,
        repo_git: fila.get("repo_git")?,
        tty: fila.get("tty")?,
        resumen: fila.get("resumen")?,
        registrada_en: fila.get("registrada_en")?,
        visto_en: fila.get("visto_en")?,
    })
}
