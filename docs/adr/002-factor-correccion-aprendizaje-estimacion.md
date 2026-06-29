# ADR-002: Factor de corrección global como mecanismo de aprendizaje de estimación

- **Date**: 2026-06-29
- **Status**: Accepted
- **Deciders**: Max (LexusFX)
- **Tags**: arquitectura, jornada, aprendizaje, producto

## Context and Problem Statement

La IA, al planear una feature, estima tiempos de equipos humanos genéricos ("1 semana, semana
2, semana 3") cuando Max la implementa en **minutos o un día**. Esa alucinación de estimación
es un problema recurrente y concreto: las estimaciones no reflejan la velocidad real de Max
desarrollando con IA. El sistema ya mide el tiempo **real** de cada tarea (el broker timbra
inicio/fin con su reloj), pero no captura el **estimado** ni aprende de la diferencia. Había que
decidir CÓMO el sistema aprende para corregir estimaciones futuras.

## Decision Drivers

- Matar la alucinación de estimaciones de forma medible, no subjetiva.
- Aprender de los tiempos reales que el broker YA timbra (regla sagrada: la IA nunca estima el real).
- Que las IAs alimenten el aprendizaje **nativamente** (vía tools MCP), sin que Max meta datos a mano.
- Simplicidad y auditabilidad (no una caja negra de ML).

## Considered Options

- **A. Factor de corrección global** — un único ratio aprendido (`real ≈ estimado / factor`).
- **B. Promedio/mediana por tipo de tarea** — estadística por categoría de tarea.
- **C. Solo medir el real** — guardar tiempos, sin corregir la estimación (status quo + estimado).

## Decision Outcome

Chosen option: **"A. Factor de corrección global"**, porque mata el problema de raíz (el sesgo
de inflado de la IA) con un único número auditable que se ajusta con cada tarea cerrada, vía
**media móvil exponencial** (α≈0.3, pondera lo reciente) del ratio `estimado/real`, con clamp a
[0.5, 50] contra outliers. Es más simple que la opción B (que requiere clasificar tareas y
acumular suficientes muestras por tipo antes de ser útil) y funciona desde la 2ª tarea. La
opción C no corrige nada — la IA seguiría estimando mal. El estimado se captura porque las IAs
crean sus tareas vía tools MCP nativas (`crear_tarea` con estimado); al cerrar, el broker mide
el real y actualiza el factor. `crear_tarea` devuelve el estimado ya corregido, cerrando el
bucle de aprendizaje en vivo.

### Positive Consequences

- La IA recibe estimaciones realistas basadas en la velocidad real de Max ("dijiste 5d; ~6h").
- Cero intervención manual: los peers fichan tareas como funcionarios; el broker aprende solo.
- Auditable: un número + nº de muestras, no una caja negra.
- Reusa la jornada y el fichaje de tareas ya existentes.

### Negative Consequences

- Un factor global no distingue tipos de tarea (un fix trivial y una feature grande comparten
  factor). Aceptable hoy; si la varianza molesta, se evoluciona a la opción B (afinado por tipo).
- Depende de que las IAs creen las tareas con un estimado — si no lo hacen, no aprende (mitigado
  por la instrucción en el harness que las guía a hacerlo).

## Pros and Cons of the Options

### A. Factor global ✅ Chosen
- ✅ Mata el sesgo de raíz con un número simple y auditable.
- ✅ Útil desde la 2ª tarea; se adapta con media móvil.
- ❌ No distingue tipos de tarea.

### B. Promedio por tipo
- ✅ Más preciso por categoría.
- ❌ Requiere clasificar tareas y acumular muestras por tipo antes de servir.
- ❌ Más complejo de construir y mantener.

### C. Solo medir el real
- ✅ Mínimo esfuerzo.
- ❌ No corrige la estimación — el problema persiste.

## Links

- Spec: `.specs/features/tareas-autogestionadas-aprendizaje/spec.md`
- Relacionado: VISIÓN del proyecto ("matar o tempo inventado", jornada timbrada por el broker).
- Futuro: evolución a factor por tipo de tarea (opción B) si la varianza lo justifica.
