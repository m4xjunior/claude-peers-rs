//! Tests de integración contra un Redis REAL.
//!
//! Prueban que `AlmacenRedis` es intercambiable con `AlmacenSqlite` (misma semántica del
//! trait) y, sobre todo, el requisito-corazón de la visión: el OUTBOX SOBREVIVE AL REINICIO
//! de un peer (un nuevo handle al mismo Redis encuentra el ítem pendiente → no se pierde).
//!
//! Se saltan limpiamente si no hay Redis disponible (CLAUDE_PEERS_TEST_REDIS o el local),
//! para no romper entornos sin Redis.

use peers_core::{Alcance, Almacen, ItemOutbox};

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
async fn limpiar(alm: &AlmacenRedis, ids: &[&str]) {
    for id in ids {
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
        para_id: "it-jefin".into(),
        texto: "tarea a medio hacer".into(),
        creado_en: "2026-01-01T00:00:00Z".into(),
        confirmado: false,
    };
    // Limpieza previa + encolar con el primer handle.
    let _ = alm1.outbox_confirmar("it-ob-restart").await;
    alm1.outbox_encolar(&item).await.unwrap();

    // "El peer se reinicia": creamos un handle NUEVO al mismo Redis.
    let alm2 = AlmacenRedis::nuevo(&url).unwrap();
    let pendientes = alm2.outbox_pendientes("it-jefin").await.unwrap();
    assert!(
        pendientes.iter().any(|i| i.id == "it-ob-restart"),
        "el ítem del outbox debe sobrevivir al reinicio (lo ve un handle nuevo)"
    );

    // ACK con el segundo handle → ya no está pendiente.
    alm2.outbox_confirmar("it-ob-restart").await.unwrap();
    let tras_ack = alm2.outbox_pendientes("it-jefin").await.unwrap();
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
