# Bug del round-trip: identidad + colisión de id (resuelto 2026-06-29)

#cutover #protocolo #bug

## Síntoma
Claudio → asset funcionaba (a asset le llegaba el <channel>), pero asset → Claudio fallaba:
la respuesta de asset rebotaba. El transporte estaba bien; el problema era de IDENTIDAD.

## Causa raíz (workflow debug: reproducir + diagnosticar + verificar adversarial)

**Bug 1 — el agente no sabía su propio id.** Sin --id, el cliente deriva el id del nombre
de la carpeta (`id_desde_directorio`). Claudio se registró como `claude-peers-rs` pero creía
ser "claudio" (su rol). Le pidió a asset responder a `para_id=claudio` → no existe → rebote.
La grieta: ni las instrucciones del initialize ni el meta del push exponían el id PROPIO.

**Bug 2 — colisión de id (latente, lo destapó el verificador adversarial).** Dos Claude en
la MISMA carpeta sin --id derivaban el MISMO id → compartían `cprs:mensajes:{id}` (drenado
destructivo: el primero que revisa roba los mensajes del otro), outbox, jornada; y `/salir`
de uno borraba el registro del otro. El id-auto por carpeta era el detonante.

## Arreglo (commit 7976424)
- **Identidad:** `mcp.rs::instrucciones(id_efectivo)` — el initialize arranca anunciando
  "Tu id en la red claude-peers es: '<id>'". `main.rs` rama initialize usa el id REAL
  (estado.id, ya con sufijo si hubo colisión), fallback id_efectivo.
- **Colisión:** `broker/main.rs::resolver_id_sin_colision` — si el id_preferido ya está
  registrado por otro PID VIVO (`pid_vivo` = kill(pid,0)), sufija -2/-3. Mismo PID o PID
  muerto = re-registro legítimo → reusa el id (conserva herencia de cola). Dep nueva: libc.
- Decisión de Max: "carpeta + sufijo único si colisiona" (id legible + estable + sin colisión).

## Verificado E2E
2 peers en /tmp/colision-test sin --id → `colision-test` y `colision-test-2` (no colisión).
initialize de peer1 anuncia "Tu id en la red claude-peers es: 'colision-test'". 15 tests verdes.

## Limitación conocida (honestidad)
`pid_vivo` solo funciona para peers LOCALES (kill(pid,0) sobre PID local). Un peer remoto
(cross-host por túnel) tiene un PID que aquí no existe → se trataría como "muerto". Para
cross-host, distinguir por id de rol explícito (CLAUDE_PEERS_ID). Aceptable: la colisión
por carpeta es un problema local (dos terminales en la misma máquina).

## Pendiente de paridad con el TS (no bloqueante)
Falta el CLI de inspección (el TS tiene cli.ts: status/peers/send/kill-broker). El Rust no
tiene un `peers-cli` equivalente — útil para debugar la red sin abrir Claude. Propuesto a Max.
