# 🗺️ claude-peers · desktop — Mapa de la Vault

> Vault de Obsidian de las RFCs de la app **peers-desktop** (GPUI, Design System Ethos).
> **164 features** en 9 pestañas que hoy faltan (CRUD, controles, pop-ups, trazabilidad).
> Abrí esta carpeta (`.specs/desktop`) como vault en Obsidian. El grafo muestra las conexiones.

---

## 📌 Empezar por aquí

- [[INDICE-RFCS|📇 Índice maestro]] — top-10 MVP, 5 olas de implementación, dependencias de backend.

## 📁 Las 9 pestañas (una RFC cada una)

| Pestaña | RFC | Features | Foco |
|---------|-----|---------:|------|
| Tareas | [[tareas/RFC-tareas\|RFC Tareas]] | 18 | abrir tarea (pop-up), CRUD, reasignar, estados, timeline |
| Peers | [[peers/RFC-peers\|RFC Peers]] | 18 | enviar mensaje, kick, jornada/trazabilidad del peer, estado |
| Alertas | [[alertas/RFC-alertas\|RFC Alertas]] | 18 | detalle, descartar, filtros, trazabilidad de emisión/resolución |
| Trazabilidad | [[trazabilidad/RFC-trazabilidad\|RFC Trazabilidad]] | 18 | abrir mensaje (timeline), reenviar, filtros, búsqueda |
| Redis | [[redis/RFC-redis\|RFC Redis]] | 20 | purgar, inspeccionar colas/outbox, reenviar pendientes |
| Broker | [[broker/RFC-broker\|RFC Broker]] | 20 | reiniciar, umbrales de liveness, health, admin |
| Config | [[config/RFC-config\|RFC Config]] | 18 | editar/guardar parámetros, validar, defaults, tema |
| Jornada | [[jornada/RFC-jornada\|RFC Jornada]] | 17 | detalle de sesión, tareas, totales, trazabilidad temporal |
| Acceso | [[acceso/RFC-acceso\|RFC Acceso]] | 17 | broker_url, token, probar conexión, allowlist, auth |

**Total: 164 features** · 44 alta · 74 media · 40 baja.

## 🥇 Top-10 MVP (recuperar el control cuanto antes)

Del [[INDICE-RFCS|índice]] — las de prioridad ALTA que desbloquean el uso real:

1. `tareas-01` — **Abrir tarea** (pop-up de detalle) — el bloqueo #1
2. `tareas-03` — Asignar tarea (formulario)
3. `tareas-04` — Reasignar con selector de peer
4. `peers-03` — Kick peer
5. `peers-02` — Enviar mensaje a un peer
6. `trazabilidad-05` — Reenviar mensaje
7. `redis-04` — Purgar cola de peer atascado
8. `alertas-02` — Descartar/resolver alerta
9. `acceso-05` — Probar conexión
10. `jornada-01` — Abrir tarea desde jornada

## 🎨 Design System Ethos (referencia visual de las propuestas)

Fondo tinta `#100D0A` · superficies `#1A1611` · texto papel `#ECE5D7` · acento **dorado brasa `#C9A96E`** · humo `#938B7B` · línea `#2B271F`. Tipografía Fraunces (títulos) / Inter (UI) / IBM Plex Mono (datos). Radios card 14 · control 10 · pill 999.

## 🧭 Estado

- App base: **funciona** (tema Ethos aplicado, datos cargando, navegación) — branch `feat/peers-desktop-gpui`.
- Estas RFCs son el **backlog** de lo que falta para que sea plenamente operable.
- Próximo paso: convertir la **Ola 1** del índice en spec+tasks ejecutables.

#moc #peers-desktop #rfc
