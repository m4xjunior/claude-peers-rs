
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
