//! Trait `Almacen` — la interfaz de persistencia del broker.
//!
//! Abstrae el motor de almacenamiento para que el resto del broker (handlers, jornada,
//! github) programe contra esta interfaz y no contra SQLite ni Redis directamente. Es el
//! CONTRATO de coordinación de la fase 2: store.rs (Redis) lo implementa; jornada.rs lo
//! consume; los tests corren contra cualquier implementación.
//!
//! Todos los métodos son async (Redis lo es) y devuelven `anyhow::Result` — nada entra
//! en pánico; el handler traduce el error a un 500 con JSON.

use crate::{
    Alcance, Alerta, EstadoMensaje, EstadoTarea, FactorEstimacion, Instancia, ItemOutbox, Mensaje,
    Sesion, Tarea,
};
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
        repo_github: Option<&str>,
        tty: Option<&str>,
        resumen: &str,
        ahora: &str,
    ) -> anyhow::Result<()>;

    async fn latido(&self, id: &str, ahora: &str) -> anyhow::Result<()>;
    async fn definir_resumen(&self, id: &str, resumen: &str) -> anyhow::Result<()>;
    async fn salir(&self, id: &str) -> anyhow::Result<()>;
    async fn instancia_existe(&self, id: &str) -> anyhow::Result<bool>;
    async fn contar_instancias(&self) -> anyhow::Result<usize>;

    // --- Admin / introspección (SOLO LECTURA salvo `purgar`) ---

    /// Lista TODOS los ids registrados (sin filtro de liveness ni alcance). Lo usa el panel
    /// de admin (TUI) para enumerar colas y outbox por instancia. A diferencia de `listar`,
    /// no descarta vencidas: el admin quiere ver el estado crudo del almacén.
    async fn listar_ids(&self) -> anyhow::Result<Vec<String>>;

    /// Cuenta los mensajes de la bandeja activa de `id` SIN drenarlos. SOLO LECTURA: el panel
    /// de admin solo observa. Redis: ZCARD cprs:bandeja:{id}. SQLite: COUNT(*) en bandeja activa
    /// (estado != 'procesado'/'fallido'/'deadletter').
    async fn contar_mensajes_pendientes(&self, id: &str) -> anyhow::Result<usize>;

    /// Purga la fila de mensajes y el outbox de `id` (DEL/DELETE). Operación de admin
    /// explícita (la TUI la dispara desde la pantalla Redis). Idempotente.
    async fn purgar(&self, id: &str) -> anyhow::Result<()>;

    /// Lee una instancia por id (sin filtro de liveness). None si no existe.
    /// Lo usa el broker para resolver el repo_github de la instancia dueña de una tarea
    /// y abrir la issue en ESE repo (dinámico).
    async fn instancia_obtener(&self, id: &str) -> anyhow::Result<Option<Instancia>>;

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

    /// PEEK no-destructivo (R1.1): devuelve los mensajes de la bandeja activa de `id` que
    /// aún NO están `Procesado` (es decir, en `Enviado`/`Entregado`/`Leido`), SIN borrar nada.
    /// El borrado de la bandeja activa solo ocurre al confirmar `Procesado` (R1.5). Idempotente:
    /// `recibir` repetido devuelve los mismos mensajes hasta que se procesen.
    async fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>>;

    /// Transiciona un mensaje a `nuevo` estado timbrando el tiempo con `ahora` (R1.2/R1.3).
    /// Idempotente y monótona: solo avanza si el nuevo rango es mayor que el actual (ver
    /// `EstadoMensaje::rango`); el timbre del campo de tiempo (`entregado_en`/`leido_en`/
    /// `procesado_en`) es "solo la primera vez" (HSETNX / COALESCE). Al llegar a `Procesado`
    /// el mensaje sale de la bandeja activa (R1.5) pero persiste en el historial (R2.1).
    /// Devuelve `true` si transicionó, `false` si fue no-op (idempotente o ya en estado igual/mayor).
    async fn transicionar_mensaje(
        &self,
        msg_id: i64,
        nuevo: EstadoMensaje,
        ahora: &str,
    ) -> anyhow::Result<bool>;

    /// Historial durable de la cola `id` (R2.1/R2.2): mensajes con su estado final aunque ya
    /// se procesaran. `desde` filtra por cursor (msg_id > desde); `estado` filtra por estado.
    async fn historial(
        &self,
        id: &str,
        desde: Option<i64>,
        estado: Option<EstadoMensaje>,
    ) -> anyhow::Result<Vec<Mensaje>>;

    /// Lee un mensaje por su id (de cualquier bandeja/historial). None si no existe.
    /// Lo usa `/admin/reenviar` para clonar el mensaje original (R2.3).
    async fn mensaje_obtener(&self, msg_id: i64) -> anyhow::Result<Option<Mensaje>>;

    /// Re-encola un mensaje a partir de `original` (R2.3): crea uno NUEVO con `msgseq`/id
    /// fresco, estado `Enviado`, `reenviado_de = Some(original.id)` y `reenvios =
    /// original.reenvios + 1`. Reusa `de_id`/`para_id`/`texto` del original y lo deposita en
    /// la bandeja activa + historial del `para_id` original. `ahora` es el `enviado_en`
    /// timbrado por el broker. Devuelve el `msg_id` del nuevo mensaje.
    async fn encolar_reenvio(&self, original: &Mensaje, ahora: &str) -> anyhow::Result<i64>;

    async fn limpiar_vencidas(&self, vencidas_antes: &str) -> anyhow::Result<usize>;

    /// Aplica la retención del historial (R2.1): recorta cada cola a los últimos N mensajes
    /// (`RETENCION_HISTORIAL`). Se llama desde la limpieza periódica de 30s del broker.
    async fn podar_historial(&self, retener: usize) -> anyhow::Result<()>;

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

    // --- Gestión interactiva de tareas (R4/R5/R9) ---

    /// Edita campos de una tarea (R4/AC1): lee la tarea, aplica solo los campos `Some`
    /// (`descripcion`/`estimado_seg`), reusa `tarea_guardar` para persistir y devuelve la tarea
    /// actualizada. `None` si la tarea no existe (el handler responde 404). NO toca estado,
    /// tiempos ni factor: editar es solo metadatos.
    async fn tarea_editar(
        &self,
        tarea_id: &str,
        descripcion: Option<&str>,
        estimado_seg: Option<i64>,
    ) -> anyhow::Result<Option<Tarea>>;

    /// Transiciona el estado de una tarea (R5/AC2). Valida con `transicion_valida`; si la
    /// transición es inválida devuelve la tarea SIN cambios (el handler decide el código). Reglas
    /// de timbrado (el reloj lo pone el broker vía `ahora`):
    ///   - `Hecha` y la tarea aún no tenía `fin` → timbra `fin = ahora` y mide
    ///     `duracion_seg = ahora - inicio` (el aprendizaje del factor lo hace el handler, NUNCA
    ///     el store, y SOLO en `Hecha`).
    ///   - `Bloqueada` → guarda `bloqueo_motivo = motivo`.
    ///   - otros estados → solo cambia `estado`.
    /// Devuelve la tarea ya actualizada; `None` si no existe.
    async fn tarea_estado(
        &self,
        tarea_id: &str,
        estado: EstadoTarea,
        motivo: Option<&str>,
        ahora: &str,
    ) -> anyhow::Result<Option<Tarea>>;

    /// Persiste una nota de progreso (report) de una tarea y devuelve el historial actualizado
    /// (R9/AC5). Lo llama `tarea_reportar` además de comentar en GitHub: el historial local es la
    /// fuente para el detalle de la TUI (no depende de GitHub). Redis: `RPUSH cprs:reportes:{id}`.
    /// SQLite: `INSERT INTO reportes`.
    async fn tarea_reportar(&self, tarea_id: &str, texto: &str, ahora: &str) -> anyhow::Result<()>;

    /// Historial de reportes de progreso de una tarea, en orden cronológico (R9/AC5). Cada
    /// entrada es una línea `"<ahora> — <texto>"`. Vacío si la tarea no tiene reportes.
    async fn tarea_reportes(&self, tarea_id: &str) -> anyhow::Result<Vec<String>>;

    // --- Aprendizaje de estimación (factor global) ---

    /// Lee el factor de corrección global (R2/R4). Si nunca se ha actualizado, devuelve el
    /// default neutro `{ muestras: 0, factor: 1.0, actualizado_en: "" }` (sin corrección).
    /// Redis: HASH `cprs:factor_estimacion`. SQLite: tabla `factor_estimacion(id=1)`.
    async fn factor_estimacion(&self) -> anyhow::Result<FactorEstimacion>;

    /// Aprende de UNA tarea cerrada con estimado+real válidos (R3): lee el factor actual,
    /// aplica la media móvil exponencial (`peers_core::aplicar_media_movil`) con `ratio`,
    /// incrementa `muestras`, timbra `actualizado_en = ahora` (el reloj lo pone el broker,
    /// nunca la IA) y persiste. Devuelve el factor ya actualizado.
    async fn actualizar_factor(&self, ratio: f64, ahora: &str) -> anyhow::Result<FactorEstimacion>;

    // --- Supervisor (fase 5): alertas de ocioso/atascado/ghosteo ---

    /// Emite una alerta a la cola `cprs:alertas` (LIST acotada a `MAX_ALERTAS` con LTRIM)
    /// SOLO si `(tipo+sujeto)` no está ya en el SET de activas `cprs:alertas_activas` (R7).
    /// Devuelve `true` si la emitió, `false` si ya estaba activa (idempotencia: no re-alertar
    /// lo mismo cada 30s). El `creada_en` de la alerta lo timbra el broker (su reloj), aquí
    /// solo se persiste. Redis: SADD condicional + RPUSH + LTRIM. SQLite: INSERT en `alertas_activas`
    /// (PK tipo+sujeto) + INSERT en `alertas` con poda a las últimas `MAX_ALERTAS`.
    async fn alerta_emitir(&self, a: &Alerta) -> anyhow::Result<bool>;

    /// Quita `(tipo+sujeto)` del SET de activas cuando la condición se resuelve (AC2: al pasar
    /// un mensaje a `Procesado`, su ghosteo deja de estar activo y puede volver a alertarse si
    /// reaparece). Idempotente: si no estaba, no es error.
    async fn alerta_resolver(&self, tipo: &str, sujeto: &str) -> anyhow::Result<()>;

    /// Lee la cola de alertas `cprs:alertas` (R6, para `GET /admin/alertas`). Las últimas
    /// `MAX_ALERTAS` como mucho (la cola ya está acotada por `alerta_emitir`).
    async fn alertas(&self) -> anyhow::Result<Vec<Alerta>>;

    /// Devuelve TODOS los mensajes (de cualquier cola/historial) que están en `estado`, junto
    /// a su `para_id`. Lo usa el detector de ghosteo (R4): busca los `Leido` no `Procesado`.
    /// Redis: itera el historial de cada instancia (vía `listar_ids`); NUNCA usa KEYS.
    /// SQLite: `SELECT ... WHERE estado = ?`.
    async fn mensajes_en_estado(
        &self,
        estado: EstadoMensaje,
    ) -> anyhow::Result<Vec<(String, Mensaje)>>;
}
