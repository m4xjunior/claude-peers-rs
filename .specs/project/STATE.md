
## Blocker / Bug conocido (registrado 2026-07-02)
- **BUG CRÍTICO del broker — colisión de ID (mismo id a 2 procesos distintos).** Confirmado hoy
  en la revisión (tar-79): TOCTOU en `registrar` (main.rs:294) — el chequeo `id_ocupado_por_otro_vivo`
  y el `HSET`+`SADD` no son atómicos → dos peers que arrancan a la vez en el mismo dir (mismo
  id_preferido, PID vivo) obtienen ambos "libre" y se registran con el MISMO id base, pisándose
  la cola. El fix cross-host de hoy (hostname) cubrió el caso remoto, NO el same-host concurrente.
  Fix pendiente: hacer atómico el registro (lock/SETNX o Lua) + reservar `"broker"` como de_id
  no-suplantable + singleton de broker (SETNX broker_lock). Es un fix del backend claude-peers-rs,
  independiente de la app desktop. Vale arreglarlo para que no se repita el caos de identidad de hoy.

### Actualización 2026-07-02 (verificación de identidad de app-planificacion)
Evidencia nueva del bug de ids: un peer con from_id="app-planificacion" me escribió, pero
listar_instancias NO lo mostraba con ese id — colapsaba a id="instancia", cwd="/". Hipótesis del
propio peer (sólida): el id se deriva del NOMBRE DEL DIRECTORIO del proyecto → dos instancias en
carpetas llamadas igual (una en el Mac local, otra en el servidor con el mismo proyecto) COLISIONAN
con el mismo id, y el registro las colapsa a un id genérico. Confirma el diagnóstico de la revisión
(tar-79): la derivación del id por cwd + el TOCTOU del registro producen colisión cross-máquina.
Refuerza la prioridad del fix: id estable NO derivado solo del cwd (o cwd+hostname), registro
atómico, y detección de colisión que sufije en vez de colapsar.

### RESUELTO 2026-07-02 (commit 1f4187f)
El bug de colisión de ids está ARREGLADO: (1) lock atómico en el registro del broker
(registro_lock) elimina el TOCTOU → varias instancias mismo dir se sufijan -2/-3; (2) fallback
del cliente pasó de "instancia" (colapsaba en masa) a "peer" (el broker lo sufija). Test
dos_instancias_mismo_dir_coexisten_con_sufijo. Broker recargado en vivo. Comportamiento pedido
por Max logrado: varias instancias por directorio, filtrables por nombre (ejemplo, ejemplo-2…).

### Pendiente de refinamiento (anotado 2026-07-02, para la fase Trazabilidad)
- alertas-02 "ir al sujeto": para GHOSTEO el sujeto es un msg-id, no un peer → hoy navega a
  Trazabilidad buscando el "historial del peer msg:NNNN" (inexistente → "0 mensajes"). Al
  implementar la pestaña Trazabilidad (trazabilidad-01: pop-up timeline del mensaje), hacer que
  "ir al sujeto" de un ghosteo abra el MENSAJE concreto (su timeline), no el historial de un peer.
- La trazabilidad rica de mensajes (timeline enviado→entregado→leído→procesado, abrir mensaje,
  reenviar) es la pestaña Trazabilidad — pendiente, fase futura de Fable.

## Sesión 2026-07-02 (noche) — Jefim s004 (dev) + Julio s003 (coord/QA)

### RESUELTO — P0 crash SIGABRT de peers-desktop al crear tareas
Causa raíz (crash reports .ips simbolizados, NO adivinada): el binario `target/release/peers-desktop`
que corría Max se compiló a las 16:27, ANTERIOR al fix anti-SIGABRT ee96677 (17:45) — contenía el
`.await` directo de reqwest en `cx.spawn` (tokio `Handle::current` → "no reactor running" → abort).
El código en HEAD ya estaba sano (grep: cero awaits de red fuera de `bloquear_en`). Fix = recompilar
release (21:56, UUID 099DF34F) y relanzar. Lección operativa reforzada: **tras cada fix, recompilar
TAMBIÉN el release que Max ejecuta** — el binario en disco no se actualiza solo.

### IMPLEMENTADO — Política de comunicación (RFC politica-comunicacion, Fase 1: motor+endpoints)
- peers-core: `Patron`/`AccionPolitica`/`ReglaComunicacion`/`Politica::evaluar` (primera regla gana,
  default Permitir) + `BloqueoComunicacion` + 6 tests. **Id del operador UNIFICADO** (§5.3 política +
  lanzador + colisión): `ID_BROKER`, `ID_OPERADOR` (reservado) y `REMITENTES_EXENTOS`
  (broker/operador/peers-tui/peers-desktop) con `remitente_exento()` — R3: jamás se bloquean.
- Trait `Almacen`: politica_leer/politica_guardar/registrar_bloqueo/bloqueos_recientes en AMBOS
  backends (Redis `cprs:politica_comunicacion` + `cprs:comunicacion_bloqueada` LPUSH/LTRIM 100;
  SQLite tablas espejo con poda) + 2 tests sqlite.
- Broker: `RwLock<Politica>` en memoria (R9, guard nunca cruza `.await`), carga fail-open al arrancar,
  gancho en `enviar()` (tras existencia, antes de encolar → `ok:false` "comunicación bloqueada por
  política: <motivo>", NO encola, NO 500), endpoints `GET/POST /admin/politica` (reemplazo completo,
  caliente) y `GET /admin/politica/bloqueos`. R5: forzar/asignar/reasignar/reenviar van con
  de="broker" → exentos. Limitación anotada: la exención confía en el `de_id` declarado
  (anti-spoofing = fix de colisión pendiente arriba).
- Verificación: build Redis+sqlite+workspace ✅, 89 tests ✅, E2E broker aislado :7898 AC1-AC6 ✅.
- Falta: UI (R10-R12 tabla/matriz desktop + TUI) y despliegue al broker de producción (autorizado
  por Max 2026-07-02 ~22:19, en curso).

### IMPLEMENTADO — Desktop: tema Ethos en el kit + crear tarea desde Jornada (pendiente QA visual)
- Contraste (bug Max "inputs ilegibles"): `tema::aplicar_tema_kit()` registra la paleta Ethos como
  Theme GLOBAL de gpui-component (base Dark + tokens pisados) en `main.rs` tras `init` — los Input/
  Select del kit pintaban su fondo BLANCO por el Theme default claro; envolver en divs NO bastaba.
- Jornada: botón "Crear tarea" en la cabecera reutilizando la Action `AbrirFormAsignar` y el overlay
  raíz del form de Tareas (cero duplicación); al abrir desde Jornada se preselecciona el peer enfocado.
- Build verde; QA visual de Julio + recompilación release de la desktop PENDIENTES.

### EN CURSO — Registro de acciones (RFC registro-acciones, decisión Max: SQLx+FK)
- Hecho: DTOs R1-R3 en peers-core (`AccionRegistrada`, `TipoAccion` #[non_exhaustive] snake_case,
  `RETENCION_ACCIONES=500`).
- Diseño en revisión con Julio ANTES de codificar el motor. Punto de arquitectura levantado por
  Jefim: FK `instancias(id) ON DELETE CASCADE` de la spec borraría la bitácora con cada
  `limpiar_vencidas`/kick (instancias = PRESENCIA efímera, no entidad durable) y en producción el
  backend es Redis (no existen esas tablas) → propuesta: .db PROPIO de bitácora vía SQLx (ambos
  backends) con tablas de identidad durable mínimas (`peers_conocidos`/`tareas_conocidas` upsert)
  y FK reales contra ESAS, sin CASCADE destructivo.
