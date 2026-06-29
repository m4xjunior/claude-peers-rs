# Roadmap de features — claude-peers-rs (orden acordado con Max, 2026-06-29)

> Orden de trabajo definido por Max. Cada feature: estado y dónde está su diseño.

## ✅ Hecho y desplegado
- **Cutover TS→Rust** (broker LaunchAgent, MCP, plugin, install.sh).
- **Conectividad cross-host** con token (RFC-001 fase previa).
- **TUI base** (ratatui): Peers, Acceso, Redis, Broker, Config, Trazabilidad.
- **Entrega durable + trazabilidad** (RFC-001/ADR-001): bandeja ZSET no-destructiva, estados
  de mensaje timbrados, historial, reenvío. Pantalla Trazabilidad.

## 🔨 En curso
- **1. Aprendizaje de estimación** (ADR-002 + TDD-001 + spec) — workflow de implementación
  corriendo. Factor de corrección: los peers fichan tareas vía tools MCP, el broker mide el
  real y aprende a corregir las estimaciones infladas de la IA.

## ⏭️ Siguiente (en orden)
- **2. Supervisor** (Fase 5 del consejo-roadmap) — detector de ociosos / tareas atascadas /
  ghosteo (mensaje leído pero no procesado). Alerta en la TUI. PENDIENTE DE DISEÑO (brainstorming).
- **3. TUI completa con mouse** — al final, cuando 1 y 2 estén listos:
  - Navegación con MOUSE (crossterm captura click/scroll) en todas las pantallas.
  - Exponer TODAS las features: + pantalla Jornada (fichaje/tiempos por peer), + pantalla
    Tareas (estados, estimado vs real), + pantalla Factor (el aprendido), + pantalla Supervisor.
  - Decisión tomada: ratatui (NO GPUI), para que corra por SSH en servers. (ADR pendiente.)

## Aprendizajes operativos (en memoria: claude-peers-rs-operativa)
- Cambiar protocolo del broker (LIST→ZSET) o el token → reiniciar TODOS los peers vivos.
- Cambiar binarios del plugin → bump de versión (cache por versión).
- Recargar LaunchAgent tras recompilar el broker.
- El flag --dangerously-load-development-channels es obligatorio para el <channel>.
