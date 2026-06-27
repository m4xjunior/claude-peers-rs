//! `AlmacenRedis` — implementación del trait `Almacen` sobre Redis (backend por defecto).
//!
//! Namespace `cprs:` (claude-peers-rs) para no colisionar con otras bases del mismo Redis.
//! Async nativo vía deadpool-redis (pool sobre tokio). Toda la (de)serialización de structs
//! es JSON con serde — el wire interno del store es JSON, igual que el resto del sistema.
//!
//! Claves:
//!   cprs:instancia:{id}     HASH con los campos de la instancia
//!   cprs:instancias         SET con los ids registrados
//!   cprs:mensajes:{para_id} LIST (JSON de Mensaje) — fila FIFO por destinatario
//!   cprs:msgseq             contador para asignar id incremental a mensajes
//!   cprs:outbox:{para_id}   LIST (JSON de ItemOutbox) — durable con ACK
//!   cprs:sesiones:{inst}    LIST (JSON de Sesion)
//!   cprs:tareas:{inst}      LIST (JSON de Tarea)  — fuente de la jornada
//!   cprs:tarea:{id}         STRING (JSON de Tarea) — índice directo por id de tarea

use async_trait::async_trait;
use deadpool_redis::redis::{cmd, AsyncCommands};
use deadpool_redis::{Config, Pool, Runtime};
use peers_core::{Alcance, Almacen, Instancia, ItemOutbox, Mensaje, Sesion, Tarea};

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
fn k_mensajes(para_id: &str) -> String {
    format!("{NS}mensajes:{para_id}")
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
            let _: () = conn.sadd(format!("{NS}instancias"), id).await?;
        }
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
            entregado: false,
        };
        let _: () = conn
            .rpush(k_mensajes(para_id), serde_json::to_string(&msg)?)
            .await?;
        Ok(())
    }

    async fn recibir_mensajes(&self, id: &str) -> anyhow::Result<Vec<Mensaje>> {
        let mut conn = self.conn().await?;
        // Drena la lista entera de forma atómica: LRANGE + DEL en pipeline.
        let clave = k_mensajes(id);
        let crudos: Vec<String> = conn.lrange(&clave, 0, -1).await?;
        if crudos.is_empty() {
            return Ok(vec![]);
        }
        let _: () = conn.del(&clave).await?;
        let mut msgs = Vec::with_capacity(crudos.len());
        for c in crudos {
            if let Ok(mut m) = serde_json::from_str::<Mensaje>(&c) {
                m.entregado = true;
                msgs.push(m);
            }
        }
        Ok(msgs)
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
                let _: () = conn.del(k_instancia(&id)).await?;
                let _: () = conn.srem(format!("{NS}instancias"), &id).await?;
                // Purga su fila de mensajes pendientes (no su outbox durable).
                let _: () = conn.del(k_mensajes(&id)).await?;
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
        // El ACK reescribe el ítem como confirmado dentro de su lista. Recorremos las
        // listas de outbox; como el id es único, basta encontrarlo y marcarlo.
        let mut conn = self.conn().await?;
        let claves: Vec<String> = conn.keys(format!("{NS}outbox:*")).await?;
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
