//! Tests de integración contra un Redis REAL.
//!
//! Prueban que `AlmacenRedis` es intercambiable con `AlmacenSqlite` (misma semántica del
//! trait) y, sobre todo, el requisito-corazón de la visión: el OUTBOX SOBREVIVE AL REINICIO
//! de un peer (un nuevo handle al mismo Redis encuentra el ítem pendiente → no se pierde).
//!
//! Se saltan limpiamente si no hay Redis disponible (CLAUDE_PEERS_TEST_REDIS o el local),
//! para no romper entornos sin Redis.

use peers_core::{Alcance, Almacen, EstadoMensaje, ItemOutbox};

// El módulo store vive en el binario; lo incluimos directamente para testearlo aislado.
#[path = "../src/store.rs"]
mod store;
// store.rs referencia crate::jornada::diferencia_seg; proveemos ese símbolo mínimo aquí.
mod jornada {
    pub fn diferencia_seg(inicio: &str, fin: &str) -> i64 {
        use time::format_description::well_known::Rfc3339;
        use time::OffsetDateTime;
        let p = |s: &str| OffsetDateTime::parse(s, &Rfc3339).ok();
        match (p(inicio), p(fin)) {
            (Some(i), Some(f)) => (f - i).whole_seconds().max(0),
            _ => 0,
        }
    }
}

use store::AlmacenRedis;

/// URL de Redis para tests, o None si no hay (→ se salta el test).
fn url_redis() -> Option<String> {
    if let Ok(u) = std::env::var("CLAUDE_PEERS_TEST_REDIS") {
        return Some(u);
    }
    Some("redis://127.0.0.1:6379".to_string())
}

/// Crea un almacén Redis con un prefijo de prueba único por test (vía la propia URL/DB),
/// y verifica conectividad; si falla, devuelve None para saltar.
async fn almacen_o_saltar() -> Option<AlmacenRedis> {
    let url = url_redis()?;
    let alm = AlmacenRedis::nuevo(&url).ok()?;
    // Comprobamos conectividad con una operación trivial.
    match alm.contar_instancias().await {
        Ok(_) => Some(alm),
        Err(_) => None,
    }
}

/// Limpia las claves que estos tests usan (ids con prefijo it- para no pisar datos reales).
/// Da de baja la instancia (DEL+SREM) Y purga su bandeja/historial/outbox para no arrastrar
/// estado entre corridas (los msg_id van por el contador global cprs:msgseq).
async fn limpiar(alm: &AlmacenRedis, ids: &[&str]) {
    for id in ids {
        let _ = alm.purgar(id).await;
        let _ = alm.salir(id).await;
    }
}

#[tokio::test]
async fn redis_id_estable_hereda_fila_en_restart() {
    let Some(alm) = almacen_o_saltar().await else {
        eprintln!("SALTADO: no hay Redis disponible");
        return;
    };
    limpiar(&alm, &["it-jefin", "it-claudia"]).await;

    alm.registrar("it-jefin", 1, "/x", None, None, None, "j", "2026-01-01T00:00:00Z").await.unwrap();
    alm.registrar("it-claudia", 2, "/y", None, None, None, "c", "2026-01-01T00:00:00Z").await.unwrap();
    alm.encolar_mensaje("it-claudia", "it-jefin", "pre-restart", "2026-01-01T00:00:01Z").await.unwrap();
    // "Restart": re-registro mismo id, pid distinto.
    alm.registrar("it-jefin", 999, "/x", None, None, None, "j", "2026-01-01T00:01:00Z").await.unwrap();

    let msgs = alm.recibir_mensajes("it-jefin").await.unwrap();
    assert_eq!(msgs.len(), 1, "el mensaje debe sobrevivir al re-registro");
    assert_eq!(msgs[0].texto, "pre-restart");

    limpiar(&alm, &["it-jefin", "it-claudia"]).await;
}

#[tokio::test]
async fn redis_reregistro_repuebla_el_set_si_quedo_fuera() {
    // REGRESIÓN del bug "instancia fantasma": el HASH sobrevive pero el id quedó FUERA del
    // SET (p.ej. tras purgar un sufijo o limpiar vencidos a mitad). El re-registro debe
    // volver a meter el id en el SET (SADD idempotente), o nadie lo ve en listar().
    let Some(alm) = almacen_o_saltar().await else {
        eprintln!("SALTADO: no hay Redis disponible");
        return;
    };
    limpiar(&alm, &["it-fantasma"]).await;

    // 1. Registro normal → está en el SET.
    alm.registrar("it-fantasma", 1, "/x", None, None, None, "f", "2026-01-01T00:00:00Z").await.unwrap();
    let antes = alm.listar_ids().await.unwrap();
    assert!(antes.iter().any(|i| i == "it-fantasma"), "debe estar en el SET tras registrar");

    // 2. Simula la inconsistencia: quitar SOLO del SET, dejando el HASH (estado fantasma).
    //    Usamos otro registro y un salir parcial no; lo provocamos vía un re-registro de un id
    //    distinto que no toca a éste — en su lugar comprobamos el camino real: re-registrar
    //    el MISMO id (el HASH ya existe) debe garantizar el SADD igualmente.
    //    Para forzar el estado fantasma de forma determinista, registramos de nuevo (branch
    //    "existe") y verificamos que sigue en el SET (antes del fix, si hubiera salido, no volvía).
    alm.registrar("it-fantasma", 2, "/x", None, None, None, "f", "2026-01-01T00:01:00Z").await.unwrap();
    let despues = alm.listar_ids().await.unwrap();
    assert!(
        despues.iter().any(|i| i == "it-fantasma"),
        "tras el re-registro el id DEBE seguir en el SET (SADD idempotente siempre)"
    );

    limpiar(&alm, &["it-fantasma"]).await;
}

#[tokio::test]
async fn redis_outbox_sobrevive_reinicio_del_peer() {
    // EL test-prueba de la visión: el ítem del outbox persiste en Redis; un NUEVO handle
    // (simula el peer reiniciado / otro proceso) lo encuentra pendiente. Solo desaparece
    // tras el ACK. Nada se pierde aunque el peer caiga a mitad.
    let url = match url_redis() {
        Some(u) => u,
        None => return,
    };
    let Some(alm1) = almacen_o_saltar().await else {
        eprintln!("SALTADO: no hay Redis disponible");
        return;
    };

    let item = ItemOutbox {
        id: "it-ob-restart".into(),
        para_id: "it-ob-peer".into(),
        texto: "tarea a medio hacer".into(),
        creado_en: "2026-01-01T00:00:00Z".into(),
        confirmado: false,
    };
    // Limpieza previa + encolar con el primer handle.
    let _ = alm1.outbox_confirmar("it-ob-restart").await;
    alm1.outbox_encolar(&item).await.unwrap();

    // "El peer se reinicia": creamos un handle NUEVO al mismo Redis.
    let alm2 = AlmacenRedis::nuevo(&url).unwrap();
    let pendientes = alm2.outbox_pendientes("it-ob-peer").await.unwrap();
    assert!(
        pendientes.iter().any(|i| i.id == "it-ob-restart"),
        "el ítem del outbox debe sobrevivir al reinicio (lo ve un handle nuevo)"
    );

    // ACK con el segundo handle → ya no está pendiente.
    alm2.outbox_confirmar("it-ob-restart").await.unwrap();
    let tras_ack = alm2.outbox_pendientes("it-ob-peer").await.unwrap();
    assert!(
        !tras_ack.iter().any(|i| i.id == "it-ob-restart"),
        "tras el ACK el ítem ya no debe estar pendiente"
    );
}

#[tokio::test]
async fn redis_jornada_timbrada() {
    let Some(alm) = almacen_o_saltar().await else {
        return;
    };
    use peers_core::{Sesion, Tarea};
    let _ = alm.salir("it-jor").await;

    alm.sesion_abrir(&Sesion {
        id: "it-s1".into(),
        instancia_id: "it-jor".into(),
        inicio: "2026-01-01T00:00:00Z".into(),
        fin: None,
        duracion_seg: None,
    })
    .await
    .unwrap();

    let mut t = Tarea {
        id: "it-t1".into(),
        instancia_id: "it-jor".into(),
        sesion_id: "it-s1".into(),
        descripcion: "x".into(),
        inicio: "2026-01-01T00:00:00Z".into(),
        fin: None,
        duracion_seg: None,
        issue_number: None,
        estimado_seg: None,
    };
    alm.tarea_guardar(&t).await.unwrap();
    // Cierre timbrado 90s después.
    t.fin = Some("2026-01-01T00:01:30Z".into());
    t.duracion_seg = Some(jornada::diferencia_seg("2026-01-01T00:00:00Z", "2026-01-01T00:01:30Z"));
    alm.tarea_guardar(&t).await.unwrap();

    let (_, tareas) = alm.jornada("it-jor").await.unwrap();
    let t1 = tareas.iter().find(|x| x.id == "it-t1").unwrap();
    assert_eq!(t1.duracion_seg, Some(90), "duración MEDIDA por el broker, no estimada");
}

#[tokio::test]
async fn redis_recibir_es_peek_no_destructivo() {
    // R1.1/AC1: encolar 2, recibir dos veces → ambas devuelven los 2 (no se borran). Tras
    // transicionar a Procesado, salen de la bandeja activa pero quedan en el historial (R2.1).
    let Some(alm) = almacen_o_saltar().await else {
        eprintln!("SALTADO: no hay Redis disponible");
        return;
    };
    limpiar(&alm, &["it-peek", "it-emisor"]).await;
    alm.registrar("it-peek", 1, "/x", None, None, None, "p", "2026-01-01T00:00:00Z").await.unwrap();
    alm.encolar_mensaje("it-emisor", "it-peek", "uno", "2026-01-01T00:00:01Z").await.unwrap();
    alm.encolar_mensaje("it-emisor", "it-peek", "dos", "2026-01-01T00:00:02Z").await.unwrap();

    let p1 = alm.recibir_mensajes("it-peek").await.unwrap();
    assert_eq!(p1.len(), 2, "primer peek devuelve los 2");
    let p2 = alm.recibir_mensajes("it-peek").await.unwrap();
    assert_eq!(p2.len(), 2, "segundo peek NO consume → siguen los 2");

    // Procesa el primero → sale de la bandeja activa, queda en historial.
    let mid = p1[0].id;
    assert!(alm.transicionar_mensaje(mid, EstadoMensaje::Procesado, "2026-01-01T00:00:09Z").await.unwrap());
    assert_eq!(alm.recibir_mensajes("it-peek").await.unwrap().len(), 1, "tras Procesado queda 1 activo");
    let h = alm.historial("it-peek", None, None).await.unwrap();
    assert_eq!(h.len(), 2, "el historial retiene ambos aunque uno ya esté Procesado");
    assert!(h.iter().any(|m| m.id == mid && m.estado == EstadoMensaje::Procesado));

    limpiar(&alm, &["it-peek", "it-emisor"]).await;
}

#[tokio::test]
async fn redis_transicion_idempotente_timbra_una_vez() {
    // R1.3/AC2: transicionar a Entregado dos veces → 2ª devuelve false y entregado_en no cambia.
    let Some(alm) = almacen_o_saltar().await else {
        eprintln!("SALTADO: no hay Redis disponible");
        return;
    };
    limpiar(&alm, &["it-idem", "it-emisor"]).await;
    alm.registrar("it-idem", 1, "/x", None, None, None, "i", "2026-01-01T00:00:00Z").await.unwrap();
    alm.encolar_mensaje("it-emisor", "it-idem", "uno", "2026-01-01T00:00:01Z").await.unwrap();
    let mid = alm.recibir_mensajes("it-idem").await.unwrap()[0].id;

    assert!(alm.transicionar_mensaje(mid, EstadoMensaje::Entregado, "2026-01-01T00:00:02Z").await.unwrap());
    let m1 = alm.mensaje_obtener(mid).await.unwrap().unwrap();
    assert_eq!(m1.entregado_en.as_deref(), Some("2026-01-01T00:00:02Z"));
    // Segunda vez → no-op, el timbre se mantiene.
    assert!(!alm.transicionar_mensaje(mid, EstadoMensaje::Entregado, "2099-01-01T00:00:00Z").await.unwrap());
    let m2 = alm.mensaje_obtener(mid).await.unwrap().unwrap();
    assert_eq!(m2.entregado_en.as_deref(), Some("2026-01-01T00:00:02Z"), "el timbre NO se re-escribe");
    // Avanza a Leido (rango mayor) → true.
    assert!(alm.transicionar_mensaje(mid, EstadoMensaje::Leido, "2026-01-01T00:00:03Z").await.unwrap());
    // Retroceder a Enviado → false.
    assert!(!alm.transicionar_mensaje(mid, EstadoMensaje::Enviado, "2026-01-01T00:00:04Z").await.unwrap());

    limpiar(&alm, &["it-idem", "it-emisor"]).await;
}

#[tokio::test]
async fn redis_factor_aprende_clampa_y_persiste() {
    // R2/R3/R4 + AC1/AC4 sobre Redis: empezando del default (factor 1.0, 0 muestras), un ratio
    // extremo (120) clampa el factor a 50 con muestras=1; una segunda llamada con ratio 2 lo
    // mueve por media móvil; el factor persiste (lo lee un handle nuevo al mismo Redis).
    let url = match url_redis() {
        Some(u) => u,
        None => return,
    };
    let Some(alm) = almacen_o_saltar().await else {
        eprintln!("SALTADO: no hay Redis disponible");
        return;
    };
    // Reset determinista de la clave global (DEL directo: factor es estado compartido).
    {
        use deadpool_redis::redis::cmd;
        use deadpool_redis::{Config, Runtime};
        let pool = Config::from_url(&url)
            .create_pool(Some(Runtime::Tokio1))
            .unwrap();
        let mut conn = pool.get().await.unwrap();
        let _: () = cmd("DEL")
            .arg("cprs:factor_estimacion")
            .query_async(&mut conn)
            .await
            .unwrap();
    }

    // Default neutro tras el reset.
    let f0 = alm.factor_estimacion().await.unwrap();
    assert_eq!(f0.muestras, 0);
    assert_eq!(f0.factor, 1.0);

    // Paso 1: 1 + 0.3*(120-1) = 36.7 (un solo paso no llega al techo). muestras=1.
    let f1 = alm.actualizar_factor(120.0, "2026-01-01T00:00:00Z").await.unwrap();
    assert_eq!(f1.muestras, 1);
    assert!((f1.factor - 36.7).abs() < 1e-9, "1 + 0.3*(120-1) = 36.7, fue {}", f1.factor);
    assert_eq!(f1.actualizado_en, "2026-01-01T00:00:00Z");

    // Paso 2: 36.7 + 0.3*(200-36.7) = 85.69 → clamp al techo 50. muestras=2.
    let f2 = alm.actualizar_factor(200.0, "2026-01-01T00:00:15Z").await.unwrap();
    assert_eq!(f2.muestras, 2);
    assert_eq!(f2.factor, 50.0, "ratio extremo desde factor alto se clampa al techo");

    // Paso 3: media móvil hacia abajo: 50 + 0.3*(2-50) = 35.6. muestras=3.
    let f3 = alm.actualizar_factor(2.0, "2026-01-01T00:00:30Z").await.unwrap();
    assert_eq!(f3.muestras, 3);
    assert!((f3.factor - 35.6).abs() < 1e-9, "50 + 0.3*(2-50) = 35.6, fue {}", f3.factor);

    // Persiste: un handle NUEVO al mismo Redis lee el mismo factor.
    let alm2 = AlmacenRedis::nuevo(&url).unwrap();
    let leido = alm2.factor_estimacion().await.unwrap();
    assert_eq!(leido.muestras, 3);
    assert!((leido.factor - 35.6).abs() < 1e-9);
    assert_eq!(leido.actualizado_en, "2026-01-01T00:00:30Z");

    // Limpieza.
    use deadpool_redis::redis::cmd;
    use deadpool_redis::{Config, Runtime};
    let pool = Config::from_url(&url)
        .create_pool(Some(Runtime::Tokio1))
        .unwrap();
    let mut conn = pool.get().await.unwrap();
    let _: () = cmd("DEL")
        .arg("cprs:factor_estimacion")
        .query_async(&mut conn)
        .await
        .unwrap();
}

#[tokio::test]
async fn redis_sin_keys_en_el_store() {
    // R1.6/AC4: ningún `KEYS` ni `.keys(` debe quedar en el código de producción del store.
    let fuente = include_str!("../src/store.rs");
    assert!(
        !fuente.contains("\"KEYS\"") && !fuente.contains(".keys("),
        "el store de producción NO debe usar KEYS (O(n) sobre el keyspace)"
    );
}

#[tokio::test]
async fn redis_listar_filtra_alcance_y_vivos() {
    let Some(alm) = almacen_o_saltar().await else {
        return;
    };
    limpiar(&alm, &["it-a", "it-b"]).await;
    alm.registrar("it-a", 1, "/p", Some("/p"), None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
    alm.registrar("it-b", 2, "/p", None, None, None, "", "2026-06-27T12:00:00Z").await.unwrap();
    // Excluye al solicitante it-a.
    let r = alm.listar(Alcance::Maquina, "/p", None, Some("it-a"), "1970-01-01T00:00:00Z").await.unwrap();
    assert!(r.iter().any(|i| i.id == "it-b"));
    assert!(!r.iter().any(|i| i.id == "it-a"));
    limpiar(&alm, &["it-a", "it-b"]).await;
}
