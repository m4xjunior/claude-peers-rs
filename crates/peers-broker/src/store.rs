//! `AlmacenRedis` — implementación del trait `Almacen` sobre Redis (backend por defecto).
//!
//! Namespace `cprs:` (claude-peers-rs) para no colisionar con otras bases del mismo Redis.
//! Async nativo vía deadpool-redis (pool sobre tokio). Toda la (de)serialización de structs
//! es JSON con serde — el wire interno del store es JSON, igual que el resto del sistema.
//!
//! Claves:
//!   cprs:instancia:{id}     HASH con los campos de la instancia
//!   cprs:instancias         SET con los ids registrados
//!   cprs:bandeja:{para_id}  ZSET (score=msgseq, member=msg_id) — bandeja ACTIVA por destinatario
//!   cprs:historial:{para_id} ZSET (score=msgseq, member=msg_id) — historial DURABLE (R2.1)
//!   cprs:msg:{msg_id}       HASH con el JSON del mensaje + estado + timestamps (fuente de verdad)
//!   cprs:msgseq             contador para asignar id incremental a mensajes
//!   cprs:outbox:{para_id}   LIST (JSON de ItemOutbox) — durable con ACK
//!   cprs:sesiones:{inst}    LIST (JSON de Sesion)
//!   cprs:tareas:{inst}      LIST (JSON de Tarea)  — fuente de la jornada
//!   cprs:tarea:{id}         STRING (JSON de Tarea) — índice directo por id de tarea
//!   cprs:factor_estimacion  HASH {muestras, factor, actualizado_en} — factor de corrección global
//!
//! DISEÑO bandeja/historial: el ZSET solo guarda `msg_id` (member) ordenado por `msgseq`
//! (score); el detalle del mensaje (JSON, estado, timestamps) vive en el HASH `cprs:msg:{id}`,
//! única fuente de verdad. Así `ZREM` por id y la actualización de estado son O(log n) y no
//! hay que reescribir el JSON dentro del ZSET (que exigiría conocer el valor exacto del member).

use async_trait::async_trait;
use deadpool_redis::redis::{cmd, AsyncCommands};
use deadpool_redis::{Config, Pool, Runtime};
use peers_core::{
    aplicar_media_movil, transicion_valida, Alcance, Alerta, Almacen, EstadoMensaje, EstadoTarea,
    FactorEstimacion, Instancia, ItemOutbox, Mensaje, Sesion, Tarea, TipoAlerta, MAX_ALERTAS,
};

const NS: &str = "cprs:";

pub struct AlmacenRedis {
    pool: Pool,
}

impl AlmacenRedis {
    /// Crea el pool a partir de la URL del Redis (ej. redis://127.0.0.1:6379).
    pub fn nuevo(url: &str) -> anyhow::Result<Self> {
        let cfg = Config::from_url(url);
        let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
        Ok(Self { pool })
    }

    async fn conn(&self) -> anyhow::Result<deadpool_redis::Connection> {
        Ok(self.pool.get().await?)
    }
}

fn k_instancia(id: &str) -> String {
    format!("{NS}instancia:{id}")
}
fn k_bandeja(para_id: &str) -> String {
    format!("{NS}bandeja:{para_id}")
}
fn k_historial(para_id: &str) -> String {
    format!("{NS}historial:{para_id}")
}
fn k_msg(msg_id: i64) -> String {
    format!("{NS}msg:{msg_id}")
}
fn k_outbox(para_id: &str) -> String {
    format!("{NS}outbox:{para_id}")
}
fn k_sesiones(inst: &str) -> String {
    format!("{NS}sesiones:{inst}")
}
fn k_tareas(inst: &str) -> String {
    format!("{NS}tareas:{inst}")
}
fn k_tarea(id: &str) -> String {
    format!("{NS}tarea:{id}")
}
fn k_reportes(tarea_id: &str) -> String {
    format!("{NS}reportes:{tarea_id}")
}
fn k_factor() -> String {
    format!("{NS}factor_estimacion")
}
fn k_alertas() -> String {
    format!("{NS}alertas")
}
fn k_alertas_activas() -> String {
    format!("{NS}alertas_activas")
}

/// Forma textual lowercase de un `TipoAlerta` (sin comillas JSON), para componer la clave
/// de idempotencia del SET de activas (`{tipo}:{sujeto}`). Coincide con el `rename_all`.
fn tipo_alerta_texto(t: TipoAlerta) -> &'static str {
    match t {
        TipoAlerta::Ocioso => "ocioso",
        TipoAlerta::Atascado => "atascado",
        TipoAlerta::Ghosteo => "ghosteo",
        // #8/#10: variantes compuestas. La cadena DEBE coincidir con el `#[serde(rename)]` del
        // enum en peers-core para que la clave de idempotencia {tipo}:{sujeto} sea consistente.
        TipoAlerta::CierreSospechoso => "cierre_sospechoso",
        TipoAlerta::CancelacionExcesiva => "cancelacion_excesiva",
    }
}

/// Clave del factor de corrección POR PEER (#9): `cprs:factor:{instancia_id}`. Distinta del
/// global `cprs:factor_estimacion` (k_factor), que sigue como fallback para peers sin historial.
fn k_factor_peer(instancia_id: &str) -> String {
    format!("{NS}factor:{instancia_id}")
}

#[async_trait]
impl Almacen for AlmacenRedis {
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
        let mut conn = self.conn().await?;
        let clave = k_instancia(id);
        // ¿Existe? Si sí, re-registro: UPDATE sin tocar registrada_en ni resumen (FIX #1).
        let existe: bool = conn.exists(&clave).await?;
        if existe {
            // Actualiza solo presencia; conserva registrada_en y resumen previos.
            cmd("HSET")
                .arg(&clave)
                .arg("pid").arg(pid)
                .arg("directorio").arg(directorio)
                .arg("repo_git").arg(repo_git.unwrap_or(""))
                .arg("repo_github").arg(repo_github.unwrap_or(""))
                .arg("tty").arg(tty.unwrap_or(""))
                .arg("visto_en").arg(ahora)
                .query_async::<()>(&mut conn)
                .await?;
        } else {
            cmd("HSET")
                .arg(&clave)
                .arg("id").arg(id)
                .arg("pid").arg(pid)
                .arg("directorio").arg(directorio)
                .arg("repo_git").arg(repo_git.unwrap_or(""))
                .arg("repo_github").arg(repo_github.unwrap_or(""))
                .arg("tty").arg(tty.unwrap_or(""))
                .arg("resumen").arg(resumen)
                .arg("registrada_en").arg(ahora)
                .arg("visto_en").arg(ahora)
                .query_async::<()>(&mut conn)
                .await?;
        }
        // SADD SIEMPRE (idempotente): garantiza que el id esté en el índice tras CUALQUIER
        // registro. BUG arreglado: antes el SADD solo estaba en el branch "nuevo"; si el HASH
        // sobrevivía pero el id se había quitado del SET (purga de un sufijo -2, limpieza de
        // vencidos a mitad), el re-registro repoblaba el HASH pero dejaba el id FUERA del SET
        // → instancia fantasma: con datos pero invisible en listar() (nadie la ve ni le envía).
        let _: () = conn.sadd(format!("{NS}instancias"), id).await?;
        Ok(())
    }

    async fn latido(&self, id: &str, ahora: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let _: () = conn.hset(k_instancia(id), "visto_en", ahora).await?;
        Ok(())
    }

    async fn definir_resumen(&self, id: &str, resumen: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let _: () = conn.hset(k_instancia(id), "resumen", resumen).await?;
        Ok(())
    }

    async fn salir(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let _: () = conn.del(k_instancia(id)).await?;
        let _: () = conn.srem(format!("{NS}instancias"), id).await?;
        Ok(())
    }

    async fn instancia_existe(&self, id: &str) -> anyhow::Result<bool> {
        let mut conn = self.conn().await?;
        Ok(conn.exists(k_instancia(id)).await?)
    }

    async fn contar_instancias(&self) -> anyhow::Result<usize> {
        let mut conn = self.conn().await?;
        let n: usize = conn.scard(format!("{NS}instancias")).await?;
        Ok(n)
    }

    async fn listar_ids(&self) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn().await?;
        // Estado crudo del almacén (sin filtro de liveness): el panel de admin los quiere
        // todos. Orden estable para que la TUI no "salte" filas entre refrescos.
        let mut ids: Vec<String> = conn.smembers(format!("{NS}instancias")).await?;
        ids.sort();
        Ok(ids)
    }

    async fn contar_mensajes_pendientes(&self, id: &str) -> anyhow::Result<usize> {
        let mut conn = self.conn().await?;
        // ZCARD no drena: solo cuenta la bandeja ACTIVA (los Procesado ya salieron via ZREM).
        let n: usize = conn.zcard(k_bandeja(id)).await?;
        Ok(n)
    }

    async fn purgar(&self, id: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        // DEL es idempotente (borra 0 o más claves). Borra la bandeja activa, el historial y
        // el outbox de ESTE id; no da de baja la instancia ni borra su jornada. Los HASH
        // cprs:msg:{id} sueltos (sin índice) caducan al podarse el historial, pero aquí
        // mantenemos la purga simple (DEL de las colas) como mantenimiento explícito.
        let _: () = conn.del(k_bandeja(id)).await?;
        let _: () = conn.del(k_historial(id)).await?;
        let _: () = conn.del(k_outbox(id)).await?;
        let _: () = conn.srem(format!("{NS}outbox_indice"), id).await?;
        Ok(())
    }

    async fn instancia_obtener(&self, id: &str) -> anyhow::Result<Option<Instancia>> {
        let mut conn = self.conn().await?;
        leer_instancia(&mut conn, id).await
    }

    async fn listar(
        &self,
        alcance: Alcance,
        directorio: &str,
        repo_git: Option<&str>,
        excluir_id: Option<&str>,
        vencidas_antes: &str,
    ) -> anyhow::Result<Vec<Instancia>> {
        let mut conn = self.conn().await?;
        let ids: Vec<String> = conn.smembers(format!("{NS}instancias")).await?;
        let mut out = Vec::new();
        for id in ids {
            if excluir_id.is_some_and(|ex| ex == id) {
                continue;
            }
            let inst = match leer_instancia(&mut conn, &id).await? {
                Some(i) => i,
                None => continue,
            };
            // Liveness por latido: solo vivas.
            if inst.visto_en.as_str() < vencidas_antes {
                continue;
            }
            // Filtro por alcance.
            let coincide = match alcance {
                Alcance::Maquina => true,
                Alcance::Directorio => inst.directorio == directorio,
                Alcance::Repo => match repo_git {
                    Some(rg) => inst.repo_git.as_deref() == Some(rg),
                    None => inst.directorio == directorio,
                },
            };
            if coincide {
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
        let mut conn = self.conn().await?;
        let id: i64 = conn.incr(format!("{NS}msgseq"), 1).await?;
        let msg = Mensaje {
            id,
            de_id: de_id.to_string(),
            para_id: para_id.to_string(),
            texto: texto.to_string(),
            enviado_en: ahora.to_string(),
            estado: EstadoMensaje::Enviado,
            entregado_en: None,
            leido_en: None,
            procesado_en: None,
            intentos: 0,
            reenviado_de: None,
            reenvios: 0,
        };
        // Fuente de verdad del mensaje (HASH). El ZSET de bandeja e historial solo indexan
        // el msg_id por msgseq (score) → ZADD/ZREM O(log n) sin tocar el JSON dentro del set.
        guardar_msg(&mut conn, &msg).await?;
        let _: () = conn.zadd(k_bandeja(para_id), id, id as f64).await?;
        let _: () = conn.zadd(k_historial(para_id), id, id as f64).await?;
        Ok(())
    }

    async fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>> {
        let mut conn = self.conn().await?;
        // PEEK no-destructivo (R1.1): leemos los ids de la bandeja activa por score (msgseq).
        // NO borramos nada: el borrado solo ocurre al confirmar `Procesado` (R1.5, transición).
        let clave = k_bandeja(id);
        let ids: Vec<i64> = conn.zrangebyscore(&clave, "-inf", "+inf").await?;
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut msgs = Vec::with_capacity(ids.len());
        for mid in ids {
            if let Some(m) = leer_msg(&mut conn, mid).await? {
                // La bandeja activa contiene Enviado/Entregado/Leido (no Procesado, que sale al
                // transicionar). Defensa: descartamos cualquier terminal que se hubiera colado.
                if m.estado != EstadoMensaje::Procesado
                    && m.estado != EstadoMensaje::DeadLetter
                {
                    msgs.push(m);
                }
            }
        }
        Ok(msgs)
    }

    async fn transicionar_mensaje(
        &self,
        msg_id: i64,
        nuevo: EstadoMensaje,
        ahora: &str,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn().await?;
        let clave = k_msg(msg_id);
        // Estado actual (fuente de verdad = HASH). Si no existe el mensaje, no-op.
        let estado_crudo: Option<String> = conn.hget(&clave, "estado").await?;
        let Some(estado_crudo) = estado_crudo else {
            return Ok(false);
        };
        let actual: EstadoMensaje = serde_json::from_str(&format!("\"{estado_crudo}\""))
            .unwrap_or(EstadoMensaje::Enviado);
        // Monótona: solo avanza si el nuevo rango es estrictamente mayor (idempotente si igual
        // o si retrocede). Fallido/DeadLetter tienen rango alto → siempre se aceptan.
        if nuevo.rango() <= actual.rango() {
            return Ok(false);
        }
        // Timbra el campo de tiempo SOLO la primera vez (HSETNX = el "COALESCE" de Redis, R1.3).
        let campo_tiempo = match nuevo {
            EstadoMensaje::Entregado => Some("entregado_en"),
            EstadoMensaje::Leido => Some("leido_en"),
            EstadoMensaje::Procesado => Some("procesado_en"),
            _ => None, // Fallido/DeadLetter no timbran campo dedicado (intentos/DLQ en fases futuras)
        };
        if let Some(campo) = campo_tiempo {
            let _: bool = conn.hset_nx(&clave, campo, ahora).await?;
        }
        // Estado avanza en el HASH (fuente de verdad).
        let nuevo_crudo = serde_json::to_string(&nuevo)?;
        let nuevo_crudo = nuevo_crudo.trim_matches('"');
        let _: () = conn.hset(&clave, "estado", nuevo_crudo).await?;
        // Al llegar a Procesado sale de la bandeja activa (R1.5); el HASH + historial persisten.
        if nuevo == EstadoMensaje::Procesado {
            if let Some(para_id) = conn.hget::<_, _, Option<String>>(&clave, "para_id").await? {
                let _: () = conn.zrem(k_bandeja(&para_id), msg_id).await?;
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
        let mut conn = self.conn().await?;
        // Cursor por score (msgseq == msg_id): desde exclusivo "(desde", o -inf si no hay.
        let min = match desde {
            Some(d) => format!("({d}"),
            None => "-inf".to_string(),
        };
        let ids: Vec<i64> = conn.zrangebyscore(k_historial(id), min, "+inf").await?;
        let mut out = Vec::with_capacity(ids.len());
        for mid in ids {
            if let Some(m) = leer_msg(&mut conn, mid).await? {
                if estado.is_none_or(|e| m.estado == e) {
                    out.push(m);
                }
            }
        }
        Ok(out)
    }

    async fn mensaje_obtener(&self, msg_id: i64) -> anyhow::Result<Option<Mensaje>> {
        let mut conn = self.conn().await?;
        leer_msg(&mut conn, msg_id).await
    }

    async fn encolar_reenvio(&self, original: &Mensaje, ahora: &str) -> anyhow::Result<i64> {
        let mut conn = self.conn().await?;
        // msgseq fresco: el reenvío es un mensaje NUEVO (id propio), no una mutación del original.
        let id: i64 = conn.incr(format!("{NS}msgseq"), 1).await?;
        let nuevo = Mensaje {
            id,
            de_id: original.de_id.clone(),
            para_id: original.para_id.clone(),
            texto: original.texto.clone(),
            enviado_en: ahora.to_string(),
            estado: EstadoMensaje::Enviado,
            entregado_en: None,
            leido_en: None,
            procesado_en: None,
            intentos: 0,
            reenviado_de: Some(original.id),
            reenvios: original.reenvios + 1,
        };
        // Mismo patrón que `encolar_mensaje`: HASH fuente de verdad + ZADD a bandeja e historial
        // del `para_id` original (R2.3).
        guardar_msg(&mut conn, &nuevo).await?;
        let _: () = conn.zadd(k_bandeja(&nuevo.para_id), id, id as f64).await?;
        let _: () = conn.zadd(k_historial(&nuevo.para_id), id, id as f64).await?;
        Ok(id)
    }

    async fn podar_historial(&self, retener: usize) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        // Recorta cada historial conocido a los últimos `retener` (R2.1). ZREMRANGEBYRANK
        // 0 -(N+1) elimina los más antiguos (rangos bajos) dejando los N de mayor score.
        let ids: Vec<String> = conn.smembers(format!("{NS}instancias")).await?;
        if retener == 0 {
            return Ok(());
        }
        let tope: isize = -(retener as isize) - 1;
        for id in ids {
            let _: () = conn
                .zremrangebyrank(k_historial(&id), 0, tope)
                .await?;
        }
        Ok(())
    }

    async fn limpiar_vencidas(&self, vencidas_antes: &str) -> anyhow::Result<usize> {
        let mut conn = self.conn().await?;
        let ids: Vec<String> = conn.smembers(format!("{NS}instancias")).await?;
        let mut n = 0;
        for id in ids {
            let inst = match leer_instancia(&mut conn, &id).await? {
                Some(i) => i,
                None => continue,
            };
            if inst.visto_en.as_str() < vencidas_antes {
                // #12: antes de borrar la instancia, bloquea sus tareas NO terminales (huérfanas
                // de peer caído) para que no queden con fin=None para siempre ni envenenen el
                // factor si algún día se cerraran. Bloqueada NO alimenta el factor (lo gestiona
                // el handler) y deja claro el motivo. Se hace sobre el índice cprs:tarea:{id} +
                // lista (mismo path que tarea_estado) — degrada por tarea: un fallo no aborta la
                // limpieza de la instancia.
                for id_t in self.tareas_no_terminales(&id).await?.into_iter().map(|t| t.id) {
                    if let Err(err) = self
                        .tarea_estado(&id_t, EstadoTarea::Bloqueada, Some("peer caído"), vencidas_antes)
                        .await
                    {
                        tracing::warn!("no se pudo bloquear la tarea huérfana '{id_t}': {err:#}");
                    }
                }
                let _: () = conn.del(k_instancia(&id)).await?;
                let _: () = conn.srem(format!("{NS}instancias"), &id).await?;
                // Purga su bandeja ACTIVA (no el historial durable ni el outbox): si vuelve,
                // los mensajes ya entregados se mantienen trazables en el historial (R2.1).
                let _: () = conn.del(k_bandeja(&id)).await?;
                n += 1;
            }
        }
        Ok(n)
    }

    // --- Outbox durable con ACK ---

    async fn outbox_encolar(&self, item: &ItemOutbox) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let _: () = conn
            .rpush(k_outbox(&item.para_id), serde_json::to_string(item)?)
            .await?;
        // Índice de para_id con outbox: permite a `outbox_confirmar` localizar la lista sin
        // un KEYS O(n) (R1.6). El destinatario puede no estar registrado como instancia, así
        // que NO basta el SET cprs:instancias: este índice es la fuente para iterar outboxes.
        let _: () = conn.sadd(format!("{NS}outbox_indice"), &item.para_id).await?;
        Ok(())
    }

    async fn outbox_pendientes(&self, para_id: &str) -> anyhow::Result<Vec<ItemOutbox>> {
        let mut conn = self.conn().await?;
        let crudos: Vec<String> = conn.lrange(k_outbox(para_id), 0, -1).await?;
        Ok(crudos
            .into_iter()
            .filter_map(|c| serde_json::from_str::<ItemOutbox>(&c).ok())
            .filter(|i| !i.confirmado)
            .collect())
    }

    async fn outbox_confirmar(&self, item_id: &str) -> anyhow::Result<()> {
        // El ACK reescribe el ítem como confirmado dentro de su lista. Recorremos las listas
        // de outbox indexadas en cprs:outbox_indice (poblado en outbox_encolar) en vez de un
        // KEYS O(n) sobre todo el keyspace (R1.6/AC4): grep KEYS debe quedar vacío en el store.
        let mut conn = self.conn().await?;
        let ids: Vec<String> = conn.smembers(format!("{NS}outbox_indice")).await?;
        let claves: Vec<String> = ids.iter().map(|id| k_outbox(id)).collect();
        for clave in claves {
            let crudos: Vec<String> = conn.lrange(&clave, 0, -1).await?;
            for (idx, c) in crudos.iter().enumerate() {
                if let Ok(mut item) = serde_json::from_str::<ItemOutbox>(c) {
                    if item.id == item_id && !item.confirmado {
                        item.confirmado = true;
                        let _: () = conn
                            .lset(&clave, idx as isize, serde_json::to_string(&item)?)
                            .await?;
                        return Ok(());
                    }
                }
            }
        }
        Ok(()) // idempotente: si no se encuentra, no es error
    }

    // --- Jornada ---

    async fn sesion_abrir(&self, sesion: &Sesion) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let _: () = conn
            .rpush(k_sesiones(&sesion.instancia_id), serde_json::to_string(sesion)?)
            .await?;
        Ok(())
    }

    async fn sesion_cerrar(&self, instancia_id: &str, fin: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let clave = k_sesiones(instancia_id);
        let crudos: Vec<String> = conn.lrange(&clave, 0, -1).await?;
        // Cierra la última sesión abierta (fin == None).
        for (idx, c) in crudos.iter().enumerate().rev() {
            if let Ok(mut s) = serde_json::from_str::<Sesion>(c) {
                if s.fin.is_none() {
                    s.duracion_seg = Some(crate::jornada::diferencia_seg(&s.inicio, fin));
                    s.fin = Some(fin.to_string());
                    let _: () = conn
                        .lset(&clave, idx as isize, serde_json::to_string(&s)?)
                        .await?;
                    break;
                }
            }
        }
        Ok(())
    }

    async fn tarea_guardar(&self, tarea: &Tarea) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        let json = serde_json::to_string(tarea)?;
        // Índice directo por id (para tarea_obtener) + append a la lista de la instancia.
        let _: () = conn.set(k_tarea(&tarea.id), &json).await?;
        // En la lista guardamos solo el id; el detalle vive en el índice (evita duplicar
        // versiones desactualizadas al re-guardar una tarea cerrada).
        let lista = k_tareas(&tarea.instancia_id);
        let ids: Vec<String> = conn.lrange(&lista, 0, -1).await?;
        if !ids.iter().any(|i| i == &tarea.id) {
            let _: () = conn.rpush(&lista, &tarea.id).await?;
        }
        Ok(())
    }

    async fn tarea_obtener(&self, tarea_id: &str) -> anyhow::Result<Option<Tarea>> {
        let mut conn = self.conn().await?;
        let json: Option<String> = conn.get(k_tarea(tarea_id)).await?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    async fn jornada(&self, instancia_id: &str) -> anyhow::Result<(Vec<Sesion>, Vec<Tarea>)> {
        let mut conn = self.conn().await?;
        let ses_crudas: Vec<String> = conn.lrange(k_sesiones(instancia_id), 0, -1).await?;
        let sesiones = ses_crudas
            .into_iter()
            .filter_map(|c| serde_json::from_str::<Sesion>(&c).ok())
            .collect();
        let ids: Vec<String> = conn.lrange(k_tareas(instancia_id), 0, -1).await?;
        let mut tareas = Vec::new();
        for id in ids {
            if let Some(t) = self.tarea_obtener(&id).await? {
                tareas.push(t);
            }
        }
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
        // Transición inválida: devuelve la tarea sin cambios (el handler decide el código). El
        // store nunca aplica una transición prohibida.
        if !transicion_valida(tarea.estado, estado) {
            return Ok(Some(tarea));
        }
        aplicar_transicion_tarea(&mut tarea, estado, motivo, ahora);
        self.tarea_guardar(&tarea).await?;
        Ok(Some(tarea))
    }

    async fn tarea_reportar(&self, tarea_id: &str, texto: &str, ahora: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        // Historial local del report (R9/AC5): el detalle de la TUI no depende de GitHub.
        let _: () = conn
            .rpush(k_reportes(tarea_id), format!("{ahora} — {texto}"))
            .await?;
        Ok(())
    }

    async fn tarea_reportes(&self, tarea_id: &str) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn().await?;
        let reportes: Vec<String> = conn.lrange(k_reportes(tarea_id), 0, -1).await?;
        Ok(reportes)
    }

    async fn factor_estimacion(&self) -> anyhow::Result<FactorEstimacion> {
        let mut conn = self.conn().await?;
        leer_factor_hash(&mut conn, &k_factor()).await
    }

    async fn actualizar_factor(&self, ratio: f64, ahora: &str) -> anyhow::Result<FactorEstimacion> {
        // Lee el actual, aplica la media móvil pura (clamp incluido), incrementa muestras y
        // persiste con el timbre del broker. El factor NUEVO se devuelve para que el handler
        // lo informe.
        let actual = self.factor_estimacion().await?;
        let nuevo = paso_factor(&actual, ratio, ahora);
        let mut conn = self.conn().await?;
        guardar_factor_hash(&mut conn, &k_factor(), &nuevo).await?;
        Ok(nuevo)
    }

    async fn factor_estimacion_peer(&self, instancia_id: &str) -> anyhow::Result<FactorEstimacion> {
        // #9: factor POR PEER. Mismo HASH-shape que el global, en clave cprs:factor:{inst}.
        let mut conn = self.conn().await?;
        leer_factor_hash(&mut conn, &k_factor_peer(instancia_id)).await
    }

    async fn actualizar_factor_peer(
        &self,
        instancia_id: &str,
        ratio: f64,
        ahora: &str,
    ) -> anyhow::Result<FactorEstimacion> {
        // #9: aprende el factor del peer (no toca el global). Misma media móvil que el global.
        let actual = self.factor_estimacion_peer(instancia_id).await?;
        let nuevo = paso_factor(&actual, ratio, ahora);
        let mut conn = self.conn().await?;
        guardar_factor_hash(&mut conn, &k_factor_peer(instancia_id), &nuevo).await?;
        Ok(nuevo)
    }

    async fn tarea_reasignar(
        &self,
        tarea_id: &str,
        nuevo_instancia_id: &str,
    ) -> anyhow::Result<Option<Tarea>> {
        // #11: quita el id de la lista del dueño viejo (LREM) y lo añade a la del nuevo (RPUSH),
        // cambia instancia_id y persiste el índice + la lista. Reasignar al mismo dueño es no-op.
        let Some(mut tarea) = self.tarea_obtener(tarea_id).await? else {
            return Ok(None);
        };
        let viejo = tarea.instancia_id.clone();
        if viejo == nuevo_instancia_id {
            return Ok(Some(tarea)); // no-op seguro: mismo dueño.
        }
        tarea.instancia_id = nuevo_instancia_id.to_string();
        let mut conn = self.conn().await?;
        // LREM borra TODAS las apariciones del id en la lista vieja (count 0); idempotente.
        let _: () = conn.lrem(k_tareas(&viejo), 0, tarea_id).await?;
        // RPUSH a la nueva lista solo si no estaba ya (evita duplicar al reasignar dos veces).
        let ids_nuevo: Vec<String> = conn.lrange(k_tareas(nuevo_instancia_id), 0, -1).await?;
        if !ids_nuevo.iter().any(|i| i == tarea_id) {
            let _: () = conn.rpush(k_tareas(nuevo_instancia_id), tarea_id).await?;
        }
        // Persiste el índice directo con el nuevo dueño. `tarea_guardar` ya hace el RPUSH defensivo
        // pero como el id YA está en la lista nueva, solo reescribe el índice (sin duplicar).
        let _: () = conn.set(k_tarea(tarea_id), serde_json::to_string(&tarea)?).await?;
        Ok(Some(tarea))
    }

    async fn tareas_no_terminales(&self, instancia_id: &str) -> anyhow::Result<Vec<Tarea>> {
        // #12: tareas vivas (no terminales) de la instancia, para bloquear las huérfanas.
        let (_, tareas) = self.jornada(instancia_id).await?;
        Ok(tareas
            .into_iter()
            .filter(|t| !t.estado.es_terminal())
            .collect())
    }

    // --- Supervisor (fase 5): alertas ---

    async fn alerta_emitir(&self, a: &Alerta) -> anyhow::Result<bool> {
        let mut conn = self.conn().await?;
        // Idempotencia (R7): SADD devuelve 1 si el miembro era NUEVO, 0 si ya estaba. La clave
        // es {tipo}:{sujeto} → una sola alerta activa por (tipo+sujeto) hasta que se resuelva.
        let clave_activa = format!("{}:{}", tipo_alerta_texto(a.tipo), a.sujeto);
        let nuevo: i64 = conn.sadd(k_alertas_activas(), &clave_activa).await?;
        if nuevo == 0 {
            // Ya estaba activa: no re-alertar (no toca la cola). AC1/AC4.
            return Ok(false);
        }
        // Encola la alerta y acota la LIST a las últimas MAX_ALERTAS (R5). LTRIM -N -1 conserva
        // los N más recientes (el extremo derecho son los nuevos por el RPUSH).
        let _: () = conn.rpush(k_alertas(), serde_json::to_string(a)?).await?;
        let _: () = conn
            .ltrim(k_alertas(), -(MAX_ALERTAS as isize), -1)
            .await?;
        Ok(true)
    }

    async fn alerta_resolver(&self, tipo: &str, sujeto: &str) -> anyhow::Result<()> {
        let mut conn = self.conn().await?;
        // SREM es idempotente (quita 0 o 1). Al resolverse la condición, el (tipo+sujeto) deja
        // de estar activo y podrá volver a alertarse si reaparece (AC2).
        let _: () = conn.srem(k_alertas_activas(), format!("{tipo}:{sujeto}")).await?;
        Ok(())
    }

    async fn alertas(&self) -> anyhow::Result<Vec<Alerta>> {
        let mut conn = self.conn().await?;
        let crudos: Vec<String> = conn.lrange(k_alertas(), 0, -1).await?;
        Ok(crudos
            .into_iter()
            .filter_map(|c| serde_json::from_str::<Alerta>(&c).ok())
            .collect())
    }

    async fn mensajes_en_estado(
        &self,
        estado: EstadoMensaje,
    ) -> anyhow::Result<Vec<(String, Mensaje)>> {
        let mut conn = self.conn().await?;
        // Itera el historial de cada instancia conocida (NUNCA KEYS): por cada cola, recorre sus
        // msg_id y lee el HASH fuente de verdad. Filtra por estado. El detector de ghosteo (R4)
        // pide los Leido (no Procesado).
        let ids: Vec<String> = conn.smembers(format!("{NS}instancias")).await?;
        let mut out = Vec::new();
        for id in ids {
            let msg_ids: Vec<i64> = conn.zrangebyscore(k_historial(&id), "-inf", "+inf").await?;
            for mid in msg_ids {
                if let Some(m) = leer_msg(&mut conn, mid).await? {
                    if m.estado == estado {
                        out.push((m.para_id.clone(), m));
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Aplica una transición de estado YA validada sobre una `Tarea` en memoria (R5/AC2). Centraliza
/// las reglas de timbrado para que Redis y SQLite no las dupliquen divergiendo:
///   - `Hecha` sin `fin` previo → timbra `fin = ahora` y mide `duracion_seg = ahora - inicio`
///     (el real lo pone SIEMPRE el broker vía `ahora`). IDEMPOTENTE (#1): si ya tenía `fin`, NO
///     re-timbra ni re-mide (un segundo Hecha no infla la duración). El aprendizaje del factor lo
///     hace el handler, no el store, y SOLO en `Hecha`.
///   - `Abierta` DESDE un estado terminal/parado (`Hecha`/`Cancelada`/`Bloqueada`) → REABRIR (#2):
///     la vida anterior se descarta. Limpia `fin = None`, `duracion_seg = None`, `bloqueo_motivo
///     = None` y RE-TIMBRA `inicio = ahora` (la nueva vida empieza ahora). Resetea también
///     `factor_aprendido = false` (#3): el segundo ciclo es trabajo nuevo y legítimo y podrá
///     contribuir una vez al factor cuando se cierre. Así el siguiente cierre vuelve a medir desde
///     cero y no arrastra el tiempo del ciclo anterior.
///   - `Bloqueada` → guarda `bloqueo_motivo = motivo`.
///   - cualquier otro estado → solo cambia `estado`.
pub(crate) fn aplicar_transicion_tarea(
    tarea: &mut Tarea,
    estado: EstadoTarea,
    motivo: Option<&str>,
    ahora: &str,
) {
    let venia_de_terminal = tarea.estado.es_terminal();
    tarea.estado = estado;
    match estado {
        EstadoTarea::Hecha => {
            // #1 idempotente: solo timbra/mide en el PRIMER cierre (fin aún vacío).
            if tarea.fin.is_none() {
                tarea.fin = Some(ahora.to_string());
                tarea.duracion_seg = Some(crate::jornada::diferencia_seg(&tarea.inicio, ahora));
            }
        }
        EstadoTarea::Abierta => {
            // #2 reabrir desde terminal: descarta la vida anterior y re-timbra inicio.
            if venia_de_terminal {
                tarea.fin = None;
                tarea.duracion_seg = None;
                tarea.bloqueo_motivo = None;
                tarea.inicio = ahora.to_string();
                tarea.factor_aprendido = false;
            }
        }
        EstadoTarea::Bloqueada => {
            tarea.bloqueo_motivo = motivo.map(|m| m.to_string());
        }
        _ => {}
    }
}

/// Calcula el siguiente `FactorEstimacion` desde el actual y un `ratio` (media móvil + clamp,
/// +1 muestra, timbre del broker). Pura: ni I/O ni clave. Compartida por el factor global y el
/// por-peer (#9) para que ambos apliquen EXACTAMENTE la misma fórmula.
fn paso_factor(actual: &FactorEstimacion, ratio: f64, ahora: &str) -> FactorEstimacion {
    FactorEstimacion {
        muestras: actual.muestras.saturating_add(1),
        factor: aplicar_media_movil(actual.factor, ratio),
        actualizado_en: ahora.to_string(),
    }
}

/// Lee un `FactorEstimacion` desde un HASH Redis (clave global o por-peer). Default neutro
/// `{ muestras: 0, factor: 1.0, actualizado_en: "" }` si la clave no existe.
async fn leer_factor_hash(
    conn: &mut deadpool_redis::Connection,
    clave: &str,
) -> anyhow::Result<FactorEstimacion> {
    let existe: bool = conn.exists(clave).await?;
    if !existe {
        return Ok(FactorEstimacion {
            muestras: 0,
            factor: 1.0,
            actualizado_en: String::new(),
        });
    }
    let muestras: Option<u32> = conn.hget(clave, "muestras").await?;
    let factor: Option<f64> = conn.hget(clave, "factor").await?;
    let actualizado_en: Option<String> = conn.hget(clave, "actualizado_en").await?;
    Ok(FactorEstimacion {
        muestras: muestras.unwrap_or(0),
        factor: factor.unwrap_or(1.0),
        actualizado_en: actualizado_en.unwrap_or_default(),
    })
}

/// Persiste un `FactorEstimacion` en un HASH Redis (clave global o por-peer).
async fn guardar_factor_hash(
    conn: &mut deadpool_redis::Connection,
    clave: &str,
    f: &FactorEstimacion,
) -> anyhow::Result<()> {
    cmd("HSET")
        .arg(clave)
        .arg("muestras").arg(f.muestras)
        .arg("factor").arg(f.factor)
        .arg("actualizado_en").arg(&f.actualizado_en)
        .query_async::<()>(conn)
        .await?;
    Ok(())
}

/// Guarda el mensaje completo en su HASH `cprs:msg:{id}` (fuente de verdad). Los campos van
/// como columnas del HASH para que `transicionar_mensaje` actualice `estado`/`*_en` con HSET/
/// HSETNX sin reescribir el blob entero. `leer_msg` lo reconstruye.
async fn guardar_msg(
    conn: &mut deadpool_redis::Connection,
    msg: &Mensaje,
) -> anyhow::Result<()> {
    let estado = serde_json::to_string(&msg.estado)?;
    let estado = estado.trim_matches('"');
    let mut c = cmd("HSET");
    c.arg(k_msg(msg.id))
        .arg("id").arg(msg.id)
        .arg("de_id").arg(&msg.de_id)
        .arg("para_id").arg(&msg.para_id)
        .arg("texto").arg(&msg.texto)
        .arg("enviado_en").arg(&msg.enviado_en)
        .arg("estado").arg(estado)
        .arg("intentos").arg(msg.intentos)
        .arg("reenvios").arg(msg.reenvios);
    if let Some(v) = &msg.entregado_en {
        c.arg("entregado_en").arg(v);
    }
    if let Some(v) = &msg.leido_en {
        c.arg("leido_en").arg(v);
    }
    if let Some(v) = &msg.procesado_en {
        c.arg("procesado_en").arg(v);
    }
    if let Some(v) = msg.reenviado_de {
        c.arg("reenviado_de").arg(v);
    }
    c.query_async::<()>(conn).await?;
    Ok(())
}

/// Reconstruye un Mensaje desde su HASH. None si la clave no existe.
async fn leer_msg(
    conn: &mut deadpool_redis::Connection,
    msg_id: i64,
) -> anyhow::Result<Option<Mensaje>> {
    use std::collections::HashMap;
    let h: HashMap<String, String> = conn.hgetall(k_msg(msg_id)).await?;
    if h.is_empty() {
        return Ok(None);
    }
    let estado = h
        .get("estado")
        .and_then(|s| serde_json::from_str(&format!("\"{s}\"")).ok())
        .unwrap_or(EstadoMensaje::Enviado);
    let opt = |k: &str| h.get(k).filter(|s| !s.is_empty()).cloned();
    Ok(Some(Mensaje {
        id: h.get("id").and_then(|s| s.parse().ok()).unwrap_or(msg_id),
        de_id: h.get("de_id").cloned().unwrap_or_default(),
        para_id: h.get("para_id").cloned().unwrap_or_default(),
        texto: h.get("texto").cloned().unwrap_or_default(),
        enviado_en: h.get("enviado_en").cloned().unwrap_or_default(),
        estado,
        entregado_en: opt("entregado_en"),
        leido_en: opt("leido_en"),
        procesado_en: opt("procesado_en"),
        intentos: h.get("intentos").and_then(|s| s.parse().ok()).unwrap_or(0),
        reenviado_de: h.get("reenviado_de").and_then(|s| s.parse().ok()),
        reenvios: h.get("reenvios").and_then(|s| s.parse().ok()).unwrap_or(0),
    }))
}

/// Lee una instancia del HASH y la reconstruye. None si la clave no existe.
async fn leer_instancia(
    conn: &mut deadpool_redis::Connection,
    id: &str,
) -> anyhow::Result<Option<Instancia>> {
    use std::collections::HashMap;
    let h: HashMap<String, String> = conn.hgetall(k_instancia(id)).await?;
    if h.is_empty() {
        return Ok(None);
    }
    let opt = |k: &str| h.get(k).filter(|s| !s.is_empty()).cloned();
    Ok(Some(Instancia {
        id: h.get("id").cloned().unwrap_or_else(|| id.to_string()),
        pid: h.get("pid").and_then(|s| s.parse().ok()).unwrap_or(0),
        directorio: h.get("directorio").cloned().unwrap_or_default(),
        repo_git: opt("repo_git"),
        repo_github: opt("repo_github"),
        tty: opt("tty"),
        resumen: h.get("resumen").cloned().unwrap_or_default(),
        registrada_en: h.get("registrada_en").cloned().unwrap_or_default(),
        visto_en: h.get("visto_en").cloned().unwrap_or_default(),
    }))
}
