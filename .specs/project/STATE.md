
## Blocker / Bug conocido (registrado 2026-07-02)
- **BUG CRÍTICO del broker — colisión de ID (mismo id a 2 procesos distintos).** Confirmado hoy
  en la revisión (tar-79): TOCTOU en `registrar` (main.rs:294) — el chequeo `id_ocupado_por_otro_vivo`
  y el `HSET`+`SADD` no son atómicos → dos peers que arrancan a la vez en el mismo dir (mismo
  id_preferido, PID vivo) obtienen ambos "libre" y se registran con el MISMO id base, pisándose
  la cola. El fix cross-host de hoy (hostname) cubrió el caso remoto, NO el same-host concurrente.
  Fix pendiente: hacer atómico el registro (lock/SETNX o Lua) + reservar `"broker"` como de_id
  no-suplantable + singleton de broker (SETNX broker_lock). Es un fix del backend claude-peers-rs,
  independiente de la app desktop. Vale arreglarlo para que no se repita el caos de identidad de hoy.
