# Índice de capturas — peers-desktop (validación visual)

Sesión de capturas iniciada: 2026-07-03 11:40 (España).
App: peers-desktop (Rust/GPUI, tema oscuro "Ethos" tinta/dorado).

Capturas organizadas en este directorio, renombradas con formato `NN-<pantalla>[-<detalle>].png`.

| Archivo renombrado | Pantalla | Qué se ve | Nombre original |
|---|---|---|---|
| `01-jornada-sesiones.png` | Jornada (Fichaje) | Título "Jornada · app-planificacion-servidor". KPIs: Sesiones 23, Total trabajado 0s, badge "en curso". Tabla Sesiones (23) con columnas Inicio / Fin / Duración — las 23 filas con Fin "(abierta)" y Duración "—". Al pie, inicio de tabla Tareas (3). | Captura de pantalla 2026-07-03 a las 11.46.39.png |
| `02-peers-lista.png` | Peers | Título "Peers (10) · vivo · 10 instancia(s)". Tabla con columnas ID / Directorio / Resumen / Visto / Estado. Filas: app-planificacion-servidor, peer, claude-peers-rs-s005 (con resumen larguísimo sin truncar), etc. Todos en estado "trabajando". | Captura de pantalla 2026-07-03 a las 11.47.10.png |
| `03-peers-detalle-modal.png` | Peers (modal) | Modal "Detalle del peer" (badge "ocioso") sobre la lista de Peers atenuada. Muestra Directorio, Repo Git (—), GitHub (—), Proceso (pid 1559171 · host — · pts/5), Resumen, Registrada (2026-07-03T07:18:43.453626Z), Visto (2026-07-03T09:47:21.481188Z), Métricas (1 alerta viva · 0 tareas). Botones: Cerrar, Enviar mensaje, Ver jornada, Expulsar. | Captura de pantalla 2026-07-03 a las 11.47.27.png |

---

## Referencia de pantallas de peers-desktop

| Pantalla | Cómo identificarla |
|---|---|
| Peers | Sidebar "Peers" resaltado; lista de peers de la red |
| Alertas | Sidebar "Alertas" |
| Broker | Sidebar "Broker" |
| Config | Sidebar "Config"; inputs de configuración |
| Jornada | Título "Jornada · &lt;peer&gt;"; registro de jornada/sesiones |
| Redis | Título "Colas de mensajes" |
| Tareas | Título "Tareas · GLOBAL" |
| Trazabilidad | Sidebar "Trazabilidad" |
| Acceso | Título "Red / Acceso" |
| Lanzador | Título "Configurar sesión" |
