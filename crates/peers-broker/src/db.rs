//! `AlmacenSqlite` — implementación del trait `Almacen` sobre SQLite (tras feature `sqlite`).
//!
//! Backend alternativo al Redis por defecto: da el binario 100% autocontenido (SQLite va
//! embebido con rusqlite "bundled"). El trait es async pero rusqlite es síncrono; como el
//! broker es de instancia única y las operaciones son rápidas, ejecutamos inline tras un
//! `Mutex` (bloqueo breve, sin pool). Es la opción para "1 binario y corre, sin Redis".

use async_trait::async_trait;
use peers_core::{
    aplicar_media_movil, transicion_valida, Alcance, Alerta, Almacen, BloqueoComunicacion,
    DireccionChatPrivado, EstadoMensaje, EstadoTarea, FactorEstimacion, Instancia, ItemOutbox,
    Mensaje, MensajeChatPrivado, Politica, Sesion, Tarea, TipoAlerta, MAX_ALERTAS, MAX_BLOQUEOS,
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
        // #8/#10: variantes compuestas; cadena = `#[serde(rename)]` del enum en peers-core.
        TipoAlerta::CierreSospechoso => "cierre_sospechoso",
        TipoAlerta::CancelacionExcesiva => "cancelacion_excesiva",
    }
}

/// Parsea la columna `tipo` a `TipoAlerta` (cae a `Ocioso` si es desconocida — defensivo).
fn texto_a_tipo_alerta(s: &str) -> TipoAlerta {
    match s {
        "atascado" => TipoAlerta::Atascado,
        "ghosteo" => TipoAlerta::Ghosteo,
        "cierre_sospechoso" => TipoAlerta::CierreSospechoso,
        "cancelacion_excesiva" => TipoAlerta::CancelacionExcesiva,
        _ => TipoAlerta::Ocioso,
    }
}

/// Serializa un `EstadoTarea` a su forma textual (lowercase) para la columna `estado` (coincide
/// con el `rename_all` de serde del enum en peers-core).
fn estado_tarea_a_texto(e: EstadoTarea) -> &'static str {
    match e {
        EstadoTarea::Abierta => "abierta",
        EstadoTarea::EnCurso => "encurso",
        EstadoTarea::Bloqueada => "bloqueada",
        EstadoTarea::Hecha => "hecha",
        EstadoTarea::Cancelada => "cancelada",
    }
}

/// Parsea la columna `estado` a `EstadoTarea` (cae a `Abierta` si es desconocida/NULL — sostiene
/// la compat AC6: una fila vieja sin estado se lee como Abierta).
fn texto_a_estado_tarea(s: &str) -> EstadoTarea {
    match s {
        "encurso" => EstadoTarea::EnCurso,
        "bloqueada" => EstadoTarea::Bloqueada,
        "hecha" => EstadoTarea::Hecha,
        "cancelada" => EstadoTarea::Cancelada,
        _ => EstadoTarea::Abierta,
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
                id TEXT PRIMARY KEY, pid INTEGER NOT NULL,
                hostname TEXT NOT NULL DEFAULT '', directorio TEXT NOT NULL,
                repo_git TEXT, repo_github TEXT, tty TEXT, resumen TEXT NOT NULL DEFAULT '',
                registrada_en TEXT NOT NULL, visto_en TEXT NOT NULL,
                ultima_actividad_en TEXT NOT NULL DEFAULT '',
                secreto TEXT
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
                duracion_seg INTEGER, issue_number INTEGER, estimado_seg INTEGER,
                estado TEXT NOT NULL DEFAULT 'abierta', bloqueo_motivo TEXT,
                -- #3 idempotencia del aprendizaje (0/1) y #7 prueba de trabajo (nullable).
                factor_aprendido INTEGER NOT NULL DEFAULT 0, evidencia TEXT
            );
            -- #9 factor de corrección POR PEER (espejo del global, particionado por instancia_id).
            CREATE TABLE IF NOT EXISTS factor_peer (
                instancia_id TEXT PRIMARY KEY,
                muestras INTEGER NOT NULL DEFAULT 0,
                factor REAL NOT NULL DEFAULT 1.0,
                actualizado_en TEXT NOT NULL DEFAULT ''
            );
            -- Historial de reportes de progreso por tarea (R9/AC5): el detalle de la TUI los lee
            -- de aquí (local), no de GitHub. `tarea_reportar` escribe; `tarea_reportes` lee.
            CREATE TABLE IF NOT EXISTS reportes (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                tarea_id TEXT NOT NULL, texto TEXT NOT NULL, creado_en TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_reportes_tarea ON reportes(tarea_id, seq);
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
            -- #15/R5: contador transaccional para ids de tarea (espejo de cprs:tareaseq en Redis).
            -- Fila única id=1; `siguiente_tarea_seq` hace UPDATE atómico + lectura bajo el mismo
            -- lock del Mutex → dos crear_tarea simultáneas obtienen seqs distintas (sin colisión).
            CREATE TABLE IF NOT EXISTS tareaseq (
                id INTEGER PRIMARY KEY CHECK(id=1),
                valor INTEGER NOT NULL DEFAULT 0
            );
            INSERT OR IGNORE INTO tareaseq (id, valor) VALUES (1, 0);
            -- Política de comunicación (RFC politica-comunicacion R8): fila única con el JSON
            -- completo. Blob A PROPÓSITO (no columnas): la política se reemplaza entera
            -- (POST /admin/politica) y su modelo aún crece (Patron::Grupo futuro) — un blob
            -- evita una migración por cada variante nueva. Ausente → default Permitir (AC6).
            CREATE TABLE IF NOT EXISTS politica_comunicacion (
                id INTEGER PRIMARY KEY CHECK(id=1),
                json TEXT NOT NULL
            );
            -- Intentos de envío bloqueados por la política (R7), cola acotada a MAX_BLOQUEOS
            -- (mismo patrón de poda que `alertas`).
            CREATE TABLE IF NOT EXISTS comunicacion_bloqueada (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                de_id TEXT NOT NULL, para_id TEXT NOT NULL,
                motivo TEXT NOT NULL, cuando TEXT NOT NULL
            );
            -- Chat privado dueño↔peer (RFC-lanzador §7). v1 SIMPLE: cola drenar-al-recibir. La fila
            -- se BORRA al drenar (entrega única, sin estados). `sesion_id` = id del peer. `direccion`
            -- discrimina las dos colas por peer: 'in' (operador→peer) y 'out' (peer→operador).
            CREATE TABLE IF NOT EXISTS chat_privado (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sesion_id TEXT NOT NULL, direccion TEXT NOT NULL DEFAULT 'in', de TEXT NOT NULL,
                texto TEXT NOT NULL, enviado_en TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chat_privado_sesion ON chat_privado(sesion_id, direccion, id);
            "#,
        )?;
        // Migración para bases YA existentes (R10/AC6): `CREATE TABLE IF NOT EXISTS` no añade
        // columnas a una tabla `tareas` antigua. Añadimos `estado`/`bloqueo_motivo` con ALTER e
        // ignoramos el error "duplicate column name" (idempotente: ya migrada). Las filas viejas
        // toman el DEFAULT 'abierta' → se leen como EstadoTarea::Abierta (compat AC6).
        let _ = conexion.execute(
            "ALTER TABLE tareas ADD COLUMN estado TEXT NOT NULL DEFAULT 'abierta'",
            [],
        );
        let _ = conexion.execute("ALTER TABLE tareas ADD COLUMN bloqueo_motivo TEXT", []);
        // Anti-colisión cross-host: `hostname` en bases YA existentes. Idempotente (ignora
        // "duplicate column"). Filas viejas → "" → la lógica de colisión las trata como
        // host-desconocido (degradación a la verificación por PID local, comportamiento previo).
        let _ = conexion.execute(
            "ALTER TABLE instancias ADD COLUMN hostname TEXT NOT NULL DEFAULT ''",
            [],
        );
        // #3/#7: columnas nuevas en bases YA existentes. Idempotente (ignora "duplicate column").
        // Filas viejas → factor_aprendido=0 (no aprendida) y evidencia=NULL (no-verificada).
        let _ = conexion.execute(
            "ALTER TABLE tareas ADD COLUMN factor_aprendido INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conexion.execute("ALTER TABLE tareas ADD COLUMN evidencia TEXT", []);
        // E-10: `secreto` en bases YA existentes. Idempotente (ignora "duplicate column"). Filas
        // viejas → NULL → None → el broker les emite un secreto al primer re-registro (compat).
        let _ = conexion.execute("ALTER TABLE instancias ADD COLUMN secreto TEXT", []);
        // RFC actividad-real: `ultima_actividad_en` en bases YA existentes. Idempotente (ignora
        // "duplicate column"). Filas viejas → "" → el detector de ociosos las trata como "sin
        // actividad conocida" (degrada a solo mirar tarea abierta, no rompe).
        let _ = conexion.execute(
            "ALTER TABLE instancias ADD COLUMN ultima_actividad_en TEXT NOT NULL DEFAULT ''",
            [],
        );
        // Índice para la búsqueda inversa `id_por_secreto` (O(log n) en vez de escaneo). IF NOT
        // EXISTS lo hace idempotente; la columna ya existe por el ALTER anterior.
        let _ = conexion.execute(
            "CREATE INDEX IF NOT EXISTS idx_instancias_secreto ON instancias(secreto)",
            [],
        );
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
        hostname: &str,
        directorio: &str,
        repo_git: Option<&str>,
        repo_github: Option<&str>,
        tty: Option<&str>,
        resumen: &str,
        ahora: &str,
        secreto: &str,
    ) -> anyhow::Result<bool> {
        let conexion = self.bloquear();
        let existe: bool = conexion
            .query_row("SELECT 1 FROM instancias WHERE id = ?1", params![id], |_| Ok(true))
            .unwrap_or(false);
        if existe {
            // Re-registro: UPDATE sin tocar la fila de mensajes ni registrada_en/resumen/
            // ultima_actividad_en — el re-registro (reconexión SSH/VPN) no es en sí una llamada de
            // tool, así que no cuenta como "actividad" (mismo criterio que no tocar registrada_en).
            // E-10: el secreto SÍ se rota (sesión nueva = credencial fresca), espejo de la impl Redis.
            conexion.execute(
                "UPDATE instancias SET pid=?2, hostname=?3, directorio=?4, repo_git=?5, repo_github=?6, tty=?7, visto_en=?8, secreto=?9 WHERE id=?1",
                params![id, pid, hostname, directorio, repo_git, repo_github, tty, ahora, secreto],
            )?;
        } else {
            // Alta nueva: ultima_actividad_en arranca en `ahora` (== registrada_en) — recién
            // llegado, se asume activo (decisión de Max/s000).
            conexion.execute(
                "INSERT INTO instancias (id,pid,hostname,directorio,repo_git,repo_github,tty,resumen,registrada_en,visto_en,ultima_actividad_en,secreto)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?9,?10)",
                params![id, pid, hostname, directorio, repo_git, repo_github, tty, resumen, ahora, secreto],
            )?;
        }
        Ok(!existe)
    }

    async fn latido(&self, id: &str, ahora: &str) -> anyhow::Result<()> {
        self.bloquear()
            .execute("UPDATE instancias SET visto_en=?2 WHERE id=?1", params![id, ahora])?;
        Ok(())
    }

    async fn actualizar_actividad(&self, id: &str, ahora: &str) -> anyhow::Result<()> {
        // `UPDATE ... WHERE id=?1` ya es no-op silencioso si `id` no existe (0 filas afectadas,
        // sin error) — cubre el contrato de "instancia ya dada de baja" sin lógica extra.
        self.bloquear().execute(
            "UPDATE instancias SET ultima_actividad_en=?2 WHERE id=?1",
            params![id, ahora],
        )?;
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

    async fn id_por_secreto(&self, secreto: &str) -> anyhow::Result<Option<String>> {
        // Búsqueda inversa por la columna `secreto` (índice SQL `idx_instancias_secreto` la hace
        // O(log n)). `query_row` → Err si no hay fila → `.ok()` = None. Secreto vacío nunca matchea
        // (no lo guardamos vacío), pero filtramos por si acaso.
        if secreto.is_empty() {
            return Ok(None);
        }
        let id: Option<String> = self
            .bloquear()
            .query_row(
                "SELECT id FROM instancias WHERE secreto=?1",
                params![secreto],
                |f| f.get::<_, String>(0),
            )
            .ok();
        Ok(id)
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

    async fn chat_privado_encolar(
        &self,
        sesion_id: &str,
        direccion: DireccionChatPrivado,
        de: &str,
        texto: &str,
        ahora: &str,
    ) -> anyhow::Result<MensajeChatPrivado> {
        let conexion = self.bloquear();
        conexion.execute(
            "INSERT INTO chat_privado (sesion_id, direccion, de, texto, enviado_en) VALUES (?1,?2,?3,?4,?5)",
            params![sesion_id, direccion.sufijo(), de, texto, ahora],
        )?;
        // El id lo asigna AUTOINCREMENT: lo recuperamos para devolver el mensaje completo.
        let id = conexion.last_insert_rowid();
        Ok(MensajeChatPrivado {
            id,
            de: de.to_string(),
            texto: texto.to_string(),
            enviado_en: ahora.to_string(),
        })
    }

    async fn chat_privado_drenar(
        &self,
        sesion_id: &str,
        direccion: DireccionChatPrivado,
    ) -> anyhow::Result<Vec<MensajeChatPrivado>> {
        let conexion = self.bloquear();
        // Lee todo lo pendiente de ESA dirección ordenado por id, luego BORRA (entrega única, v1).
        // El lock del Mutex serializa lectura+borrado: sin ventana para que un drenar concurrente duplique.
        let mut stmt = conexion.prepare(
            "SELECT id, de, texto, enviado_en FROM chat_privado WHERE sesion_id=?1 AND direccion=?2 ORDER BY id ASC",
        )?;
        let filas = stmt.query_map(params![sesion_id, direccion.sufijo()], |f| {
            Ok(MensajeChatPrivado {
                id: f.get(0)?,
                de: f.get(1)?,
                texto: f.get(2)?,
                enviado_en: f.get(3)?,
            })
        })?;
        let msgs: Vec<MensajeChatPrivado> = filas.filter_map(Result::ok).collect();
        if !msgs.is_empty() {
            conexion.execute(
                "DELETE FROM chat_privado WHERE sesion_id=?1 AND direccion=?2",
                params![sesion_id, direccion.sufijo()],
            )?;
        }
        Ok(msgs)
    }

    async fn chat_privado_pendientes(
        &self,
        sesion_id: &str,
        direccion: DireccionChatPrivado,
    ) -> anyhow::Result<usize> {
        let conexion = self.bloquear();
        // PEEK: cuenta sin borrar (no drena).
        let n: i64 = conexion.query_row(
            "SELECT COUNT(*) FROM chat_privado WHERE sesion_id=?1 AND direccion=?2",
            params![sesion_id, direccion.sufijo()],
            |f| f.get(0),
        )?;
        Ok(n as usize)
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
            // #12: bloquea las tareas NO terminales del peer caído (huérfanas) ANTES de borrar la
            // instancia, para que no queden con fin=None ni envenenen el factor. Bloqueada NO
            // alimenta el factor (lo gestiona el handler). Se hace inline en la MISMA conexión
            // (re-lockear self.bloquear() aquí sería un deadlock del Mutex). NO toca fin/duracion
            // (las parques quedan medibles si se reabren). Espeja `aplicar_transicion_tarea(Bloqueada)`.
            conexion.execute(
                "UPDATE tareas SET estado='bloqueada', bloqueo_motivo='peer caído'
                 WHERE instancia_id=?1 AND estado NOT IN ('hecha','cancelada','bloqueada')",
                params![id],
            )?;
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
             (id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg,estado,bloqueo_motivo,factor_aprendido,evidencia)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                t.id, t.instancia_id, t.sesion_id, t.descripcion, t.inicio, t.fin,
                t.duracion_seg, t.issue_number.map(|n| n as i64), t.estimado_seg,
                estado_tarea_a_texto(t.estado), t.bloqueo_motivo,
                t.factor_aprendido as i64, t.evidencia
            ],
        )?;
        Ok(())
    }

    async fn tarea_obtener(&self, tarea_id: &str) -> anyhow::Result<Option<Tarea>> {
        let conexion = self.bloquear();
        Ok(conexion
            .query_row(
                "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg,estado,bloqueo_motivo,factor_aprendido,evidencia
                 FROM tareas WHERE id=?1",
                params![tarea_id],
                fila_a_tarea,
            )
            .ok())
    }

    async fn tareas_todas(&self) -> anyhow::Result<Vec<Tarea>> {
        // #14/R1: vista global. Una sola consulta a la tabla `tareas` (cada fila ya tiene su
        // instancia_id) ordenada por inicio DESC (más reciente primero). Degrada por fila:
        // una que no parsea se descarta.
        let conexion = self.bloquear();
        let mut stmt = conexion.prepare(
            "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg,estado,bloqueo_motivo,factor_aprendido,evidencia
             FROM tareas ORDER BY inicio DESC",
        )?;
        let filas = stmt.query_map([], fila_a_tarea)?;
        Ok(filas.filter_map(Result::ok).collect())
    }

    async fn siguiente_tarea_seq(&self) -> anyhow::Result<i64> {
        // #15/R5: contador transaccional. UPDATE + lectura bajo el MISMO lock del Mutex
        // (self.bloquear() entrega exclusión mutua) → dos llamadas concurrentes serializan y
        // obtienen valores distintos. Espejo de INCR cprs:tareaseq en Redis.
        let conexion = self.bloquear();
        conexion.execute("UPDATE tareaseq SET valor = valor + 1 WHERE id = 1", [])?;
        let seq: i64 = conexion.query_row("SELECT valor FROM tareaseq WHERE id = 1", [], |r| r.get(0))?;
        Ok(seq)
    }

    async fn podar_tareas(&self, retener: usize) -> anyhow::Result<()> {
        // #15/R6: espejo de podar_historial sobre tareas. Por cada instancia, conserva las
        // `retener` más recientes por inicio y borra el resto. Subconsulta: cuenta cuántas tareas
        // del MISMO peer son más recientes (inicio mayor, id como desempate determinista); las que
        // tienen >= `retener` por delante son las viejas a purgar.
        if retener == 0 {
            return Ok(()); // no-op, igual que podar_historial(0).
        }
        let conexion = self.bloquear();
        conexion.execute(
            "DELETE FROM tareas WHERE id IN (
                SELECT id FROM tareas t
                WHERE (
                    SELECT COUNT(*) FROM tareas t2
                    WHERE t2.instancia_id = t.instancia_id
                      AND (t2.inicio > t.inicio OR (t2.inicio = t.inicio AND t2.id > t.id))
                ) >= ?1
            )",
            params![retener as i64],
        )?;
        Ok(())
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
                "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg,estado,bloqueo_motivo,factor_aprendido,evidencia
                 FROM tareas WHERE instancia_id=?1 ORDER BY inicio ASC",
            )?;
            let filas = stmt.query_map(params![instancia_id], fila_a_tarea)?;
            filas.filter_map(Result::ok).collect()
        };
        Ok((sesiones, tareas))
    }

    async fn tarea_editar(
        &self,
        tarea_id: &str,
        descripcion: Option<&str>,
        estimado_seg: Option<i64>,
    ) -> anyhow::Result<Option<Tarea>> {
        // Lee, aplica solo los campos Some, reusa tarea_guardar para persistir (R4/AC1).
        let Some(mut tarea) = self.tarea_obtener(tarea_id).await? else {
            return Ok(None);
        };
        if let Some(d) = descripcion {
            tarea.descripcion = d.to_string();
        }
        if let Some(est) = estimado_seg {
            tarea.estimado_seg = Some(est);
        }
        self.tarea_guardar(&tarea).await?;
        Ok(Some(tarea))
    }

    async fn tarea_estado(
        &self,
        tarea_id: &str,
        estado: EstadoTarea,
        motivo: Option<&str>,
        ahora: &str,
    ) -> anyhow::Result<Option<Tarea>> {
        let Some(mut tarea) = self.tarea_obtener(tarea_id).await? else {
            return Ok(None);
        };
        // Transición inválida: devuelve la tarea sin cambios (el handler decide el código).
        if !transicion_valida(tarea.estado, estado) {
            return Ok(Some(tarea));
        }
        // Misma regla de timbrado que Redis (helper compartido del módulo store): el real lo pone
        // SIEMPRE el broker vía `ahora`; el aprendizaje del factor lo hace el handler, no el store.
        crate::store::aplicar_transicion_tarea(&mut tarea, estado, motivo, ahora);
        self.tarea_guardar(&tarea).await?;
        Ok(Some(tarea))
    }

    async fn tarea_reportar(&self, tarea_id: &str, texto: &str, ahora: &str) -> anyhow::Result<()> {
        self.bloquear().execute(
            "INSERT INTO reportes (tarea_id,texto,creado_en) VALUES (?1,?2,?3)",
            params![tarea_id, texto, ahora],
        )?;
        Ok(())
    }

    async fn tarea_reportes(&self, tarea_id: &str) -> anyhow::Result<Vec<String>> {
        let conexion = self.bloquear();
        // Mismo formato textual que Redis ("<creado_en> — <texto>") para que la TUI no distinga
        // el backend. Orden cronológico por seq.
        let mut stmt = conexion
            .prepare("SELECT creado_en,texto FROM reportes WHERE tarea_id=?1 ORDER BY seq ASC")?;
        let filas = stmt.query_map(params![tarea_id], |r| {
            Ok(format!("{} — {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(filas.filter_map(Result::ok).collect())
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

    async fn factor_estimacion_peer(&self, instancia_id: &str) -> anyhow::Result<FactorEstimacion> {
        // #9: factor POR PEER. Tabla factor_peer (PK instancia_id). Default neutro si no hay fila.
        let conexion = self.bloquear();
        let fila: Option<(i64, f64, String)> = conexion
            .query_row(
                "SELECT muestras,factor,actualizado_en FROM factor_peer WHERE instancia_id=?1",
                params![instancia_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        Ok(match fila {
            Some((muestras, factor, actualizado_en)) => FactorEstimacion {
                muestras: muestras.max(0) as u32,
                factor,
                actualizado_en,
            },
            None => FactorEstimacion {
                muestras: 0,
                factor: 1.0,
                actualizado_en: String::new(),
            },
        })
    }

    async fn actualizar_factor_peer(
        &self,
        instancia_id: &str,
        ratio: f64,
        ahora: &str,
    ) -> anyhow::Result<FactorEstimacion> {
        // #9: misma media móvil que el global, sobre la fila del peer (UPSERT por instancia_id).
        let actual = self.factor_estimacion_peer(instancia_id).await?;
        let nuevo = FactorEstimacion {
            muestras: actual.muestras.saturating_add(1),
            factor: aplicar_media_movil(actual.factor, ratio),
            actualizado_en: ahora.to_string(),
        };
        self.bloquear().execute(
            "INSERT INTO factor_peer (instancia_id,muestras,factor,actualizado_en)
             VALUES (?1,?2,?3,?4)
             ON CONFLICT(instancia_id) DO UPDATE SET
               muestras=excluded.muestras, factor=excluded.factor,
               actualizado_en=excluded.actualizado_en",
            params![instancia_id, nuevo.muestras as i64, nuevo.factor, nuevo.actualizado_en],
        )?;
        Ok(nuevo)
    }

    async fn tarea_reasignar(
        &self,
        tarea_id: &str,
        nuevo_instancia_id: &str,
    ) -> anyhow::Result<Option<Tarea>> {
        // #11: en SQLite la "lista del dueño" se deriva de la columna instancia_id (la jornada
        // filtra WHERE instancia_id=?). Cambiar la columna ya quita la tarea de la jornada vieja
        // y la mete en la nueva — no hay lista que limpiar aparte. UPDATE atómico de una fila.
        let Some(mut tarea) = self.tarea_obtener(tarea_id).await? else {
            return Ok(None);
        };
        if tarea.instancia_id == nuevo_instancia_id {
            return Ok(Some(tarea)); // no-op: mismo dueño.
        }
        tarea.instancia_id = nuevo_instancia_id.to_string();
        self.bloquear().execute(
            "UPDATE tareas SET instancia_id=?2 WHERE id=?1",
            params![tarea_id, nuevo_instancia_id],
        )?;
        Ok(Some(tarea))
    }

    async fn tareas_no_terminales(&self, instancia_id: &str) -> anyhow::Result<Vec<Tarea>> {
        // #12: tareas vivas (no terminales) de la instancia. Filtra por los estados terminales
        // en SQL para no traer las cerradas; el filtro final reusa `es_terminal` por coherencia.
        let conexion = self.bloquear();
        let mut stmt = conexion.prepare(
            "SELECT id,instancia_id,sesion_id,descripcion,inicio,fin,duracion_seg,issue_number,estimado_seg,estado,bloqueo_motivo,factor_aprendido,evidencia
             FROM tareas WHERE instancia_id=?1 AND estado NOT IN ('hecha','cancelada','bloqueada') ORDER BY inicio ASC",
        )?;
        let filas = stmt.query_map(params![instancia_id], fila_a_tarea)?;
        Ok(filas
            .filter_map(Result::ok)
            .filter(|t| !t.estado.es_terminal())
            .collect())
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

    // --- Política de comunicación (RFC politica-comunicacion R8) ---

    async fn politica_leer(&self) -> anyhow::Result<Politica> {
        let json: Option<String> = {
            let conexion = self.bloquear();
            conexion
                .query_row("SELECT json FROM politica_comunicacion WHERE id=1", [], |r| r.get(0))
                .ok()
        };
        // Fila ausente O JSON corrupto → default Permitir (AC6, fail-open: una política
        // ilegible no deja sin habla al equipo). Espejo exacto del backend Redis.
        Ok(match json {
            Some(j) => match serde_json::from_str(&j) {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!(
                        "política de comunicación corrupta en SQLite (se usa default Permitir): {err:#}"
                    );
                    Politica::default()
                }
            },
            None => Politica::default(),
        })
    }

    async fn politica_guardar(&self, politica: &Politica) -> anyhow::Result<()> {
        // UPSERT sobre la fila única id=1 (reemplazo completo, R6).
        self.bloquear().execute(
            "INSERT INTO politica_comunicacion (id,json) VALUES (1,?1)
             ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            params![serde_json::to_string(politica)?],
        )?;
        Ok(())
    }

    async fn registrar_bloqueo(&self, bloqueo: &BloqueoComunicacion) -> anyhow::Result<()> {
        let conexion = self.bloquear();
        conexion.execute(
            "INSERT INTO comunicacion_bloqueada (de_id,para_id,motivo,cuando) VALUES (?1,?2,?3,?4)",
            params![bloqueo.de_id, bloqueo.para_id, bloqueo.motivo, bloqueo.cuando],
        )?;
        // Poda a los últimos MAX_BLOQUEOS (R7): conserva los seq más altos (los recientes),
        // mismo patrón que la poda de `alertas`.
        conexion.execute(
            "DELETE FROM comunicacion_bloqueada WHERE seq IN (
                SELECT seq FROM comunicacion_bloqueada a
                WHERE (SELECT COUNT(*) FROM comunicacion_bloqueada a2 WHERE a2.seq > a.seq) >= ?1
            )",
            params![MAX_BLOQUEOS as i64],
        )?;
        Ok(())
    }

    async fn bloqueos_recientes(&self) -> anyhow::Result<Vec<BloqueoComunicacion>> {
        let conexion = self.bloquear();
        // seq DESC = más reciente primero (contrato del trait, igual que el LPUSH de Redis).
        let mut stmt = conexion.prepare(
            "SELECT de_id,para_id,motivo,cuando FROM comunicacion_bloqueada ORDER BY seq DESC",
        )?;
        let filas = stmt.query_map([], |f| {
            Ok(BloqueoComunicacion {
                de_id: f.get(0)?,
                para_id: f.get(1)?,
                motivo: f.get(2)?,
                cuando: f.get(3)?,
            })
        })?;
        Ok(filas.filter_map(Result::ok).collect())
    }
}

fn fila_a_instancia(f: &rusqlite::Row<'_>) -> rusqlite::Result<Instancia> {
    Ok(Instancia {
        id: f.get("id")?,
        pid: f.get("pid")?,
        // `unwrap_or_default`: BD migrada de un schema sin la columna (ver ALTER en init) o fila
        // escrita por un broker viejo → "" en vez de fallar la lectura completa.
        hostname: f.get("hostname").unwrap_or_default(),
        directorio: f.get("directorio")?,
        repo_git: f.get("repo_git")?,
        repo_github: f.get("repo_github")?,
        tty: f.get("tty")?,
        resumen: f.get("resumen")?,
        registrada_en: f.get("registrada_en")?,
        visto_en: f.get("visto_en")?,
        // Mismo criterio que `hostname`: BD migrada de un schema sin la columna → "" en vez de
        // fallar la lectura completa.
        ultima_actividad_en: f.get("ultima_actividad_en").unwrap_or_default(),
        // E-10: `Option<String>` → NULL o columna ausente (BD pre-migración) → None. `ok()`
        // absorbe ambos casos sin fallar la lectura de la instancia entera.
        secreto: f.get("secreto").ok(),
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
        // Columnas nuevas (R2): NULL/desconocido → Abierta (compat AC6). Posición 9/10 según el
        // orden de los SELECT de tareas.
        estado: texto_a_estado_tarea(&f.get::<_, String>(9)?),
        bloqueo_motivo: f.get(10)?,
        // #3/#7: factor_aprendido (0/1 → bool) y evidencia (TEXT nullable). Posiciones 11/12.
        factor_aprendido: f.get::<_, i64>(11)? != 0,
        evidencia: f.get(12)?,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// R8/AC6: sin fila guardada, `politica_leer` devuelve el default Permitir; el roundtrip
    /// conserva reglas/orden/default; guardar dos veces REEMPLAZA (fila única, no acumula).
    #[tokio::test]
    async fn politica_default_roundtrip_y_reemplazo() {
        use peers_core::{AccionPolitica, DecisionPolitica, Patron, ReglaComunicacion};
        let b = base();

        // Sin configurar → default: todo permitido (AC6).
        let p0 = b.politica_leer().await.unwrap();
        assert!(p0.reglas.is_empty());
        assert_eq!(p0.evaluar("a", "b"), DecisionPolitica::Permitida);

        // Guardar una política con 1 bloqueo → roundtrip fiel.
        let p1 = Politica {
            reglas: vec![ReglaComunicacion {
                de: Patron::try_from("a".to_string()).expect("patrón"),
                para: Patron::try_from("b".to_string()).expect("patrón"),
                accion: AccionPolitica::Bloquear,
                motivo: Some("prueba".into()),
            }],
            accion_por_defecto: AccionPolitica::Permitir,
        };
        b.politica_guardar(&p1).await.unwrap();
        assert_eq!(b.politica_leer().await.unwrap(), p1);

        // Reemplazo completo (R6): guardar la vacía deja la vacía, no acumula reglas.
        b.politica_guardar(&Politica::default()).await.unwrap();
        assert!(b.politica_leer().await.unwrap().reglas.is_empty());
    }

    /// R7: los bloqueos se registran, salen MÁS RECIENTE PRIMERO y la cola queda acotada a
    /// MAX_BLOQUEOS (los viejos se podan).
    #[tokio::test]
    async fn bloqueos_acotados_y_recientes_primero() {
        let b = base();
        for i in 0..(MAX_BLOQUEOS + 5) {
            b.registrar_bloqueo(&BloqueoComunicacion {
                de_id: format!("de-{i}"),
                para_id: "para".into(),
                motivo: "prueba".into(),
                cuando: format!("2026-01-01T00:00:{:02}Z", i % 60),
            })
            .await
            .unwrap();
        }
        let recientes = b.bloqueos_recientes().await.unwrap();
        assert_eq!(recientes.len(), MAX_BLOQUEOS, "la cola debe podarse a MAX_BLOQUEOS");
        // El último registrado es el primero de la lista (más reciente primero).
        assert_eq!(recientes[0].de_id, format!("de-{}", MAX_BLOQUEOS + 4));
    }

    fn base() -> AlmacenSqlite {
        AlmacenSqlite::abrir(":memory:").expect("base en memoria")
    }

    #[tokio::test]
    async fn id_estable_reregistro_hereda_la_fila() {
        let b = base();
        b.registrar("jefin", 111, "testhost", "/x", None, None, None, "papel", "2026-01-01T00:00:00Z", "sec-jefin").await.unwrap();
        b.registrar("claudia", 222, "testhost", "/y", None, None, None, "papel", "2026-01-01T00:00:00Z", "sec-claudia").await.unwrap();
        b.encolar_mensaje("claudia", "jefin", "hola pre-restart", "2026-01-01T00:00:01Z").await.unwrap();
        // Restart: re-registro mismo id, pid distinto.
        b.registrar("jefin", 999, "testhost", "/x", None, None, None, "papel", "2026-01-01T00:01:00Z", "sec-jefin").await.unwrap();
        let msgs = b.recibir_mensajes("jefin").await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].texto, "hola pre-restart");
    }

    /// Chat privado (paridad SQLite): las dos colas (Entrada/Salida) por peer son independientes y
    /// la entrega es única (drenar borra). Espejo del test Redis (AC7).
    #[tokio::test]
    async fn chat_privado_dos_colas_independientes() {
        use peers_core::DireccionChatPrivado::{Entrada, Salida};
        let b = base();
        b.chat_privado_encolar("p", Entrada, "operador", "orden", "2026-07-06T00:00:00Z").await.unwrap();
        b.chat_privado_encolar("p", Salida, "p", "respuesta", "2026-07-06T00:00:01Z").await.unwrap();

        let entrada = b.chat_privado_drenar("p", Entrada).await.unwrap();
        assert_eq!(entrada.len(), 1);
        assert_eq!(entrada[0].de, "operador");
        // Drenar Entrada no toca Salida.
        let salida = b.chat_privado_drenar("p", Salida).await.unwrap();
        assert_eq!(salida.len(), 1);
        assert_eq!(salida[0].de, "p");
        // Entrega única.
        assert!(b.chat_privado_drenar("p", Entrada).await.unwrap().is_empty());
        assert!(b.chat_privado_drenar("p", Salida).await.unwrap().is_empty());
    }

    /// Chat privado — PEEK (`chat_privado_pendientes`): cuenta SIN drenar, para el aviso push.
    #[tokio::test]
    async fn chat_privado_peek_no_drena() {
        use peers_core::DireccionChatPrivado::Entrada;
        let b = base();
        assert_eq!(b.chat_privado_pendientes("p", Entrada).await.unwrap(), 0);
        b.chat_privado_encolar("p", Entrada, "operador", "a", "2026-07-06T00:00:00Z").await.unwrap();
        b.chat_privado_encolar("p", Entrada, "operador", "b", "2026-07-06T00:00:01Z").await.unwrap();
        // El peek ve 2 y NO consume (dos llamadas seguidas dan lo mismo).
        assert_eq!(b.chat_privado_pendientes("p", Entrada).await.unwrap(), 2);
        assert_eq!(b.chat_privado_pendientes("p", Entrada).await.unwrap(), 2);
        // Drenar sí consume → el peek vuelve a 0.
        assert_eq!(b.chat_privado_drenar("p", Entrada).await.unwrap().len(), 2);
        assert_eq!(b.chat_privado_pendientes("p", Entrada).await.unwrap(), 0);
    }

    /// E-10 (paridad SQLite): `id_por_secreto` resuelve el id real desde el secreto, el re-registro
    /// lo ROTA (el viejo deja de resolver) y `salir` limpia. Espejo del test Redis (AC7). Sin la
    /// columna/ALTER/índice, esto fallaría — guardarraíl de la migración SQLite del secreto.
    #[tokio::test]
    async fn secreto_resuelve_emisor_y_rota() {
        let b = base();
        b.registrar("s", 1, "testhost", "/x", None, None, None, "", "2026-01-01T00:00:00Z", "sec-uno")
            .await
            .unwrap();
        assert_eq!(b.id_por_secreto("sec-uno").await.unwrap().as_deref(), Some("s"));
        // Secreto inventado → None (el broker rechazaría, Invalido).
        assert_eq!(b.id_por_secreto("sec-inventado").await.unwrap(), None);
        // Re-registro rota: el viejo ya no resuelve, el nuevo sí.
        b.registrar("s", 2, "testhost", "/x", None, None, None, "", "2026-01-01T00:01:00Z", "sec-dos")
            .await
            .unwrap();
        assert_eq!(b.id_por_secreto("sec-uno").await.unwrap(), None);
        assert_eq!(b.id_por_secreto("sec-dos").await.unwrap().as_deref(), Some("s"));
        // Baja limpia la resolución.
        b.salir("s").await.unwrap();
        assert_eq!(b.id_por_secreto("sec-dos").await.unwrap(), None);
    }

    /// Anti-colisión cross-host (fix 2026-06-30): el almacén debe persistir y devolver el
    /// `hostname` para que el broker pueda distinguir un peer remoto de uno local muerto.
    /// Antes faltaba el campo → dos sesiones remotas del mismo dir pisaban el mismo id.
    #[tokio::test]
    async fn registrar_persiste_hostname_para_anticolision_crosshost() {
        let b = base();
        b.registrar("aistudio", 555, "ai-studio", "/home/aistudio", None, None, None, "", "2026-06-30T10:00:00Z", "sec-aistudio").await.unwrap();
        let inst = b.instancia_obtener("aistudio").await.unwrap().unwrap();
        assert_eq!(inst.hostname, "ai-studio", "el hostname debe round-trippear");
        // Re-registro desde el MISMO host conserva el hostname actualizado.
        b.registrar("aistudio", 777, "ai-studio", "/home/aistudio", None, None, None, "", "2026-06-30T10:01:00Z", "sec-aistudio").await.unwrap();
        let inst2 = b.instancia_obtener("aistudio").await.unwrap().unwrap();
        assert_eq!(inst2.hostname, "ai-studio");
        assert_eq!(inst2.pid, 777, "re-registro actualiza el pid");
    }

    #[tokio::test]
    async fn dos_instancias_mismo_dir_coexisten_con_sufijo() {
        // Escenario que Max pidió: varias instancias en el MISMO directorio deben PERMITIRSE y
        // filtrarse por nombre distinto (ejemplo, ejemplo-2…). Aquí se simula lo que hace el
        // registro del broker: la 1ª toma el id base; la 2ª (mismo dir, otro pid, ambas vivas)
        // se registra bajo el sufijo -2. Las dos deben coexistir en el índice, no pisarse.
        let b = base();
        b.registrar("ejemplo", 1001, "host", "/proyecto/ejemplo", None, None, None, "primera", "2026-07-02T10:00:00Z", "sec-ejemplo").await.unwrap();
        b.registrar("ejemplo-2", 1002, "host", "/proyecto/ejemplo", None, None, None, "segunda", "2026-07-02T10:00:01Z", "sec-ejemplo-2").await.unwrap();
        // Ambas existen con identidad propia.
        let uno = b.instancia_obtener("ejemplo").await.unwrap().expect("ejemplo debe existir");
        let dos = b.instancia_obtener("ejemplo-2").await.unwrap().expect("ejemplo-2 debe existir");
        assert_eq!(uno.pid, 1001);
        assert_eq!(dos.pid, 1002);
        assert_eq!(uno.directorio, dos.directorio, "comparten dir (misma estación), distinto id");
        assert_ne!(uno.id, dos.id, "ids DISTINTOS: no colapsan al mismo");
    }

    #[tokio::test]
    async fn recibir_es_peek_no_destructivo() {
        // R1.1: recibir NO borra. Dos peeks seguidos devuelven el mismo mensaje. Solo sale de
        // la bandeja activa al transicionar a Procesado (R1.5), pero queda en historial (R2.1).
        let b = base();
        b.registrar("a", 1, "testhost", "/x", None, None, None, "", "2026-01-01T00:00:00Z", "sec-a").await.unwrap();
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
        b.registrar("a", 1, "testhost", "/x", None, None, None, "", "2026-01-01T00:00:00Z", "sec-a").await.unwrap();
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
        b.registrar("a", 1, "testhost", "/x", None, None, None, "", "2026-01-01T00:00:00Z", "sec-a").await.unwrap();
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
        b.registrar("x", 1, "testhost", "/d", None, None, None, "resumen original", "2026-01-01T00:00:00Z", "sec-x").await.unwrap();
        b.registrar("x", 2, "testhost", "/d", None, None, None, "ignorado", "2026-02-02T00:00:00Z", "sec-x").await.unwrap();
        let inst = &b.listar(Alcance::Maquina, "/d", None, None, "1970-01-01T00:00:00Z").await.unwrap()[0];
        assert_eq!(inst.resumen, "resumen original");
        assert_eq!(inst.registrada_en, "2026-01-01T00:00:00Z");
        assert_eq!(inst.pid, 2);
    }

    #[tokio::test]
    async fn liveness_filtra_vencidas() {
        let b = base();
        b.registrar("viva", 1, "testhost", "/d", None, None, None, "", "2026-06-27T12:00:00Z", "sec-viva").await.unwrap();
        b.registrar("muerta", 2, "testhost", "/d", None, None, None, "", "2020-01-01T00:00:00Z", "sec-muerta").await.unwrap();
        let vivas = b.listar(Alcance::Maquina, "/d", None, None, "2026-06-27T11:59:00Z").await.unwrap();
        assert_eq!(vivas.len(), 1);
        assert_eq!(vivas[0].id, "viva");
    }

    #[tokio::test]
    async fn limpiar_purga_instancia_y_fila() {
        let b = base();
        b.registrar("zombie", 1, "testhost", "/d", None, None, None, "", "2020-01-01T00:00:00Z", "sec-zombie").await.unwrap();
        b.encolar_mensaje("otro", "zombie", "x", "2020-01-01T00:00:01Z").await.unwrap();
        assert_eq!(b.limpiar_vencidas("2026-01-01T00:00:00Z").await.unwrap(), 1);
        assert!(!b.instancia_existe("zombie").await.unwrap());
        assert_eq!(b.recibir_mensajes("zombie").await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn listar_excluye_solicitante() {
        let b = base();
        b.registrar("yo", 1, "testhost", "/d", None, None, None, "", "2026-06-27T12:00:00Z", "sec-yo").await.unwrap();
        b.registrar("otro", 2, "testhost", "/d", None, None, None, "", "2026-06-27T12:00:00Z", "sec-otro").await.unwrap();
        let r = b.listar(Alcance::Maquina, "/d", None, Some("yo"), "1970-01-01T00:00:00Z").await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "otro");
    }

    #[tokio::test]
    async fn repo_sin_git_cae_a_directorio() {
        let b = base();
        b.registrar("a", 1, "testhost", "/proj", Some("/proj"), None, None, "", "2026-06-27T12:00:00Z", "sec-a").await.unwrap();
        b.registrar("b", 2, "testhost", "/proj", None, None, None, "", "2026-06-27T12:00:00Z", "sec-b").await.unwrap();
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
            estado: EstadoTarea::Abierta,
            bloqueo_motivo: None,
            factor_aprendido: false,
            evidencia: None,
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
        b.registrar("a", 1, "testhost", "/x", None, None, None, "", "2026-01-01T00:00:00Z", "sec-a").await.unwrap();
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
            estado: EstadoTarea::Abierta,
            bloqueo_motivo: None,
            factor_aprendido: false,
            evidencia: None,
        };
        b.tarea_guardar(&t).await.unwrap();
        let leida = b.tarea_obtener("te").await.unwrap().unwrap();
        assert_eq!(leida.estimado_seg, Some(432000));
    }

    fn tarea_base(id: &str) -> Tarea {
        Tarea {
            id: id.into(),
            instancia_id: "jefin".into(),
            sesion_id: "s1".into(),
            descripcion: "original".into(),
            inicio: "2026-01-01T00:00:00Z".into(),
            fin: None,
            duracion_seg: None,
            issue_number: None,
            estimado_seg: Some(600),
            estado: EstadoTarea::Abierta,
            bloqueo_motivo: None,
            factor_aprendido: false,
            evidencia: None,
        }
    }

    #[tokio::test]
    async fn tarea_editar_aplica_solo_campos_some() {
        // R4/AC1: editar persiste descripción/estimado; los None no tocan nada; None si no existe.
        let b = base();
        b.tarea_guardar(&tarea_base("t1")).await.unwrap();
        // Solo descripción.
        let e = b.tarea_editar("t1", Some("nueva desc"), None).await.unwrap().unwrap();
        assert_eq!(e.descripcion, "nueva desc");
        assert_eq!(e.estimado_seg, Some(600), "estimado intacto");
        // Solo estimado.
        let e = b.tarea_editar("t1", None, Some(1200)).await.unwrap().unwrap();
        assert_eq!(e.descripcion, "nueva desc", "descripción intacta");
        assert_eq!(e.estimado_seg, Some(1200));
        // Relee de la base.
        let leida = b.tarea_obtener("t1").await.unwrap().unwrap();
        assert_eq!(leida.descripcion, "nueva desc");
        assert_eq!(leida.estimado_seg, Some(1200));
        // Inexistente → None.
        assert!(b.tarea_editar("fantasma", Some("x"), None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tareas_todas_junta_varios_peers_orden_inicio_desc() {
        // #14/R1/AC1: vista global = tareas de >1 peer, cada una con su instancia_id, ordenadas
        // por inicio DESC (la más reciente primero).
        let b = base();
        let mut t_a = tarea_base("ta");
        t_a.instancia_id = "peerA".into();
        t_a.inicio = "2026-01-01T00:00:00Z".into();
        let mut t_b = tarea_base("tb");
        t_b.instancia_id = "peerB".into();
        t_b.inicio = "2026-01-03T00:00:00Z".into();
        let mut t_c = tarea_base("tc");
        t_c.instancia_id = "peerA".into();
        t_c.inicio = "2026-01-02T00:00:00Z".into();
        b.tarea_guardar(&t_a).await.unwrap();
        b.tarea_guardar(&t_b).await.unwrap();
        b.tarea_guardar(&t_c).await.unwrap();
        let todas = b.tareas_todas().await.unwrap();
        let ids: Vec<&str> = todas.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["tb", "tc", "ta"], "inicio DESC: más reciente primero");
        // Cada tarea conserva su instancia_id (la TUI pinta la columna PEER).
        let peers: std::collections::HashSet<&str> =
            todas.iter().map(|t| t.instancia_id.as_str()).collect();
        assert!(peers.contains("peerA") && peers.contains("peerB"));
    }

    #[tokio::test]
    async fn siguiente_tarea_seq_incrementa_sin_colision() {
        // #15/R5/AC3: cada llamada devuelve un valor mayor → ids tar-{seq} únicos.
        let b = base();
        let s1 = b.siguiente_tarea_seq().await.unwrap();
        let s2 = b.siguiente_tarea_seq().await.unwrap();
        let s3 = b.siguiente_tarea_seq().await.unwrap();
        assert!(s2 > s1 && s3 > s2, "seq monótono creciente: {s1} {s2} {s3}");
        // Distintos entre sí (no colisionan aunque sea el "mismo peer").
        let set: std::collections::HashSet<i64> = [s1, s2, s3].into_iter().collect();
        assert_eq!(set.len(), 3);
    }

    #[tokio::test]
    async fn podar_tareas_conserva_n_mas_recientes_por_peer() {
        // #15/R6/AC4: conserva las RETENCION más recientes por peer; purga las viejas. retener=0 no-op.
        let b = base();
        // peerA: 5 tareas con inicios crecientes. peerB: 2 (no debe tocarse si retener>=2).
        for i in 0..5 {
            let mut t = tarea_base(&format!("a{i}"));
            t.instancia_id = "peerA".into();
            t.inicio = format!("2026-01-0{}T00:00:00Z", i + 1);
            b.tarea_guardar(&t).await.unwrap();
        }
        for i in 0..2 {
            let mut t = tarea_base(&format!("b{i}"));
            t.instancia_id = "peerB".into();
            t.inicio = format!("2026-02-0{}T00:00:00Z", i + 1);
            b.tarea_guardar(&t).await.unwrap();
        }
        // retener=0 es no-op (no purga nada).
        b.podar_tareas(0).await.unwrap();
        assert_eq!(b.tareas_todas().await.unwrap().len(), 7, "retener=0 no purga");
        // Conserva 3 por peer: peerA pasa de 5→3 (las 3 más recientes), peerB intacto (2<=3).
        b.podar_tareas(3).await.unwrap();
        let (_, ta) = b.jornada("peerA").await.unwrap();
        let ids_a: std::collections::HashSet<String> = ta.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ta.len(), 3, "peerA conserva 3");
        assert!(ids_a.contains("a4") && ids_a.contains("a3") && ids_a.contains("a2"));
        assert!(!ids_a.contains("a0") && !ids_a.contains("a1"), "las 2 viejas purgadas");
        // Las purgadas también desaparecen del índice (tarea_obtener None).
        assert!(b.tarea_obtener("a0").await.unwrap().is_none());
        let (_, tb) = b.jornada("peerB").await.unwrap();
        assert_eq!(tb.len(), 2, "peerB intacto (2<=3)");
    }

    #[tokio::test]
    async fn tarea_estado_hecha_mide_real_y_bloqueada_guarda_motivo() {
        // R5/AC2: Hecha timbra fin + mide real (broker); Bloqueada guarda motivo. El factor NO se
        // toca aquí (lo hace el handler).
        let b = base();
        b.tarea_guardar(&tarea_base("t1")).await.unwrap();
        // Abierta → EnCurso (válida).
        let t = b.tarea_estado("t1", EstadoTarea::EnCurso, None, "2026-01-01T00:00:10Z").await.unwrap().unwrap();
        assert_eq!(t.estado, EstadoTarea::EnCurso);
        assert!(t.fin.is_none());
        // EnCurso → Hecha: timbra fin y mide real = 90s desde inicio.
        let t = b.tarea_estado("t1", EstadoTarea::Hecha, None, "2026-01-01T00:01:30Z").await.unwrap().unwrap();
        assert_eq!(t.estado, EstadoTarea::Hecha);
        assert_eq!(t.fin.as_deref(), Some("2026-01-01T00:01:30Z"));
        assert_eq!(t.duracion_seg, Some(90), "real MEDIDO por el broker, no estimado");

        // Otra tarea: Bloqueada guarda motivo.
        b.tarea_guardar(&tarea_base("t2")).await.unwrap();
        let t = b.tarea_estado("t2", EstadoTarea::Bloqueada, Some("falta credencial"), "2026-01-01T00:02:00Z").await.unwrap().unwrap();
        assert_eq!(t.estado, EstadoTarea::Bloqueada);
        assert_eq!(t.bloqueo_motivo.as_deref(), Some("falta credencial"));
        // Persiste al releer.
        let leida = b.tarea_obtener("t2").await.unwrap().unwrap();
        assert_eq!(leida.estado, EstadoTarea::Bloqueada);
        assert_eq!(leida.bloqueo_motivo.as_deref(), Some("falta credencial"));
    }

    #[tokio::test]
    async fn tarea_estado_transicion_invalida_no_cambia() {
        // R3/R5: Hecha → EnCurso es inválida (hay que reabrir a Abierta primero). Devuelve la
        // tarea SIN cambios.
        let b = base();
        b.tarea_guardar(&tarea_base("t1")).await.unwrap();
        b.tarea_estado("t1", EstadoTarea::Hecha, None, "2026-01-01T00:01:00Z").await.unwrap();
        let t = b.tarea_estado("t1", EstadoTarea::EnCurso, None, "2026-01-01T00:02:00Z").await.unwrap().unwrap();
        assert_eq!(t.estado, EstadoTarea::Hecha, "transición inválida no aplica");
        // Reabrir a Abierta sí es válida.
        let t = b.tarea_estado("t1", EstadoTarea::Abierta, None, "2026-01-01T00:03:00Z").await.unwrap().unwrap();
        assert_eq!(t.estado, EstadoTarea::Abierta);
    }

    #[tokio::test]
    async fn reportes_persisten_y_se_leen_en_orden() {
        // R9/AC5: los reportes se persisten localmente y se leen en orden cronológico.
        let b = base();
        b.tarea_guardar(&tarea_base("t1")).await.unwrap();
        assert!(b.tarea_reportes("t1").await.unwrap().is_empty());
        b.tarea_reportar("t1", "arranqué", "2026-01-01T00:00:01Z").await.unwrap();
        b.tarea_reportar("t1", "voy por la mitad", "2026-01-01T00:05:00Z").await.unwrap();
        let reportes = b.tarea_reportes("t1").await.unwrap();
        assert_eq!(reportes.len(), 2);
        assert_eq!(reportes[0], "2026-01-01T00:00:01Z — arranqué");
        assert_eq!(reportes[1], "2026-01-01T00:05:00Z — voy por la mitad");
    }

    #[tokio::test]
    async fn tarea_vieja_sin_estado_se_lee_abierta() {
        // AC6: una fila escrita por el esquema viejo (sin columnas estado/bloqueo_motivo) se lee
        // como Abierta. Simulamos insertando directo sin esas columnas (DEFAULT 'abierta').
        let b = base();
        {
            let c = b.bloquear();
            c.execute(
                "INSERT INTO tareas (id,instancia_id,sesion_id,descripcion,inicio,estimado_seg)
                 VALUES ('vieja','jefin','s1','legacy','2026-01-01T00:00:00Z',300)",
                [],
            ).unwrap();
        }
        let t = b.tarea_obtener("vieja").await.unwrap().unwrap();
        assert_eq!(t.estado, EstadoTarea::Abierta);
        assert!(t.bloqueo_motivo.is_none());
        assert_eq!(t.estimado_seg, Some(300));
        // #3/#7 compat: una fila vieja sin esas columnas se lee con los DEFAULT (no aprendida,
        // sin evidencia) gracias al ALTER ... DEFAULT 0 y a evidencia NULL.
        assert!(!t.factor_aprendido);
        assert!(t.evidencia.is_none());
    }

    #[tokio::test]
    async fn factor_aprendido_y_evidencia_roundtrip() {
        // #3/#7: ambos campos se persisten y se releen (paridad con Redis vía JSON).
        let b = base();
        let mut t = tarea_base("t1");
        t.factor_aprendido = true;
        t.evidencia = Some("commit abc123".into());
        b.tarea_guardar(&t).await.unwrap();
        let leida = b.tarea_obtener("t1").await.unwrap().unwrap();
        assert!(leida.factor_aprendido);
        assert_eq!(leida.evidencia.as_deref(), Some("commit abc123"));
    }

    #[tokio::test]
    async fn cierre_idempotente_no_remide_duracion() {
        // #1: jornada::cerrar_tarea es idempotente. Primer cierre mide; un segundo cierre (ACK
        // perdido) NO re-timbra ni re-mide aunque pase tiempo (no infla la duración).
        use std::sync::Arc;
        let almacen: Arc<dyn Almacen> = Arc::new(base());
        almacen.tarea_guardar(&tarea_base("t1")).await.unwrap();
        let c1 = crate::jornada::cerrar_tarea(&almacen, "t1", "2026-01-01T00:01:30Z").await.unwrap();
        assert_eq!(c1.duracion_seg, Some(90));
        assert_eq!(c1.estado, EstadoTarea::Hecha);
        // Segundo cierre mucho más tarde → misma duración, mismo fin (idempotente).
        let c2 = crate::jornada::cerrar_tarea(&almacen, "t1", "2026-01-01T05:00:00Z").await.unwrap();
        assert_eq!(c2.duracion_seg, Some(90), "el segundo cierre NO re-mide");
        assert_eq!(c2.fin.as_deref(), Some("2026-01-01T00:01:30Z"), "el fin no cambia");
    }

    #[tokio::test]
    async fn reabrir_desde_terminal_limpia_y_retimbra() {
        // #2: reabrir Hecha→Abierta limpia fin/duracion, re-timbra inicio y resetea factor_aprendido.
        let b = base();
        let mut t = tarea_base("t1");
        t.factor_aprendido = true; // simula que ya contribuyó en el ciclo anterior.
        b.tarea_guardar(&t).await.unwrap();
        // Cierra (mide 90s).
        b.tarea_estado("t1", EstadoTarea::Hecha, None, "2026-01-01T00:01:30Z").await.unwrap();
        // Reabre: la vida anterior se descarta.
        let r = b.tarea_estado("t1", EstadoTarea::Abierta, None, "2026-01-01T02:00:00Z").await.unwrap().unwrap();
        assert_eq!(r.estado, EstadoTarea::Abierta);
        assert!(r.fin.is_none(), "reabrir limpia fin");
        assert!(r.duracion_seg.is_none(), "reabrir limpia duracion");
        assert_eq!(r.inicio, "2026-01-01T02:00:00Z", "reabrir re-timbra inicio");
        assert!(!r.factor_aprendido, "reabrir resetea factor_aprendido (#3)");
        // El segundo cierre mide desde el NUEVO inicio (60s después), no arrastra el viejo.
        let c = b.tarea_estado("t1", EstadoTarea::Hecha, None, "2026-01-01T02:01:00Z").await.unwrap().unwrap();
        assert_eq!(c.duracion_seg, Some(60), "el segundo ciclo se mide desde cero");
    }

    #[tokio::test]
    async fn hecha_idempotente_en_store() {
        // #1 a nivel store: un segundo tarea_estado→Hecha no re-timbra (fin ya presente).
        let b = base();
        b.tarea_guardar(&tarea_base("t1")).await.unwrap();
        b.tarea_estado("t1", EstadoTarea::Hecha, None, "2026-01-01T00:01:30Z").await.unwrap();
        // Re-aplicar Hecha (idempotente por transicion_valida) con otro reloj → no re-mide.
        let t = b.tarea_estado("t1", EstadoTarea::Hecha, None, "2026-01-01T09:00:00Z").await.unwrap().unwrap();
        assert_eq!(t.duracion_seg, Some(90));
        assert_eq!(t.fin.as_deref(), Some("2026-01-01T00:01:30Z"));
    }

    #[tokio::test]
    async fn factor_por_peer_es_independiente_del_global() {
        // #9: aprender en un peer no toca el global ni a otro peer.
        let b = base();
        let pa = b.actualizar_factor_peer("alice", 4.0, "2026-01-01T00:00:00Z").await.unwrap();
        assert_eq!(pa.muestras, 1);
        // El global sigue neutro.
        assert_eq!(b.factor_estimacion().await.unwrap().muestras, 0);
        // Otro peer también neutro.
        assert_eq!(b.factor_estimacion_peer("bob").await.unwrap().muestras, 0);
        // Releer el de alice: persiste.
        let leido = b.factor_estimacion_peer("alice").await.unwrap();
        assert_eq!(leido.muestras, 1);
        assert!((leido.factor - pa.factor).abs() < 1e-9);
    }

    #[tokio::test]
    async fn reasignar_mueve_de_jornada_y_no_duplica() {
        // #11: reasignar cambia el dueño → desaparece de la jornada vieja y aparece en la nueva.
        let b = base();
        let mut t = tarea_base("t1");
        t.instancia_id = "viejo".into();
        b.tarea_guardar(&t).await.unwrap();
        assert_eq!(b.jornada("viejo").await.unwrap().1.len(), 1);
        let r = b.tarea_reasignar("t1", "nuevo").await.unwrap().unwrap();
        assert_eq!(r.instancia_id, "nuevo");
        assert_eq!(b.jornada("viejo").await.unwrap().1.len(), 0, "ya no está en el dueño viejo");
        assert_eq!(b.jornada("nuevo").await.unwrap().1.len(), 1, "aparece en el nuevo");
        // Reasignar al mismo dueño es no-op (no duplica).
        b.tarea_reasignar("t1", "nuevo").await.unwrap();
        assert_eq!(b.jornada("nuevo").await.unwrap().1.len(), 1);
        // Inexistente → None.
        assert!(b.tarea_reasignar("fantasma", "x").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tareas_no_terminales_filtra() {
        // #12: solo Abierta/EnCurso (las "vivas"). Hecha/Cancelada/Bloqueada cuentan como
        // terminal/parado (`es_terminal`) → fuera: ya están paradas, no hay que re-bloquearlas.
        let b = base();
        for (id, est) in [
            ("abierta", EstadoTarea::Abierta),
            ("encurso", EstadoTarea::EnCurso),
            ("bloq", EstadoTarea::Bloqueada),
            ("hecha", EstadoTarea::Hecha),
            ("canc", EstadoTarea::Cancelada),
        ] {
            let mut t = tarea_base(id);
            t.estado = est;
            b.tarea_guardar(&t).await.unwrap();
        }
        let vivas: Vec<String> = b.tareas_no_terminales("jefin").await.unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(vivas.len(), 2, "solo Abierta y EnCurso son 'vivas'");
        assert!(vivas.contains(&"abierta".to_string()));
        assert!(vivas.contains(&"encurso".to_string()));
        assert!(!vivas.contains(&"bloq".to_string()), "una ya Bloqueada no se re-bloquea");
    }

    #[tokio::test]
    async fn limpiar_vencidas_bloquea_huerfanas() {
        // #12: al limpiar un peer caído, sus tareas vivas pasan a Bloqueada('peer caído') y NO
        // se quedan con fin=None apuntando a un peer que ya no existe. Las terminales no se tocan.
        let b = base();
        b.registrar("muerto", 1, "testhost", "/d", None, None, None, "", "2020-01-01T00:00:00Z", "sec-muerto").await.unwrap();
        let mut viva = tarea_base("viva");
        viva.instancia_id = "muerto".into();
        viva.estado = EstadoTarea::EnCurso;
        b.tarea_guardar(&viva).await.unwrap();
        let mut hecha = tarea_base("ya_hecha");
        hecha.instancia_id = "muerto".into();
        hecha.estado = EstadoTarea::Hecha;
        hecha.fin = Some("2020-01-01T00:00:30Z".into());
        b.tarea_guardar(&hecha).await.unwrap();

        assert_eq!(b.limpiar_vencidas("2026-01-01T00:00:00Z").await.unwrap(), 1);
        let viva2 = b.tarea_obtener("viva").await.unwrap().unwrap();
        assert_eq!(viva2.estado, EstadoTarea::Bloqueada);
        assert_eq!(viva2.bloqueo_motivo.as_deref(), Some("peer caído"));
        // La que ya estaba Hecha no se toca.
        let hecha2 = b.tarea_obtener("ya_hecha").await.unwrap().unwrap();
        assert_eq!(hecha2.estado, EstadoTarea::Hecha);
    }
}
