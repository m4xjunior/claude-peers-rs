# Deep-research "Criador de Producto" (agente PM-IA) — Síntesis + Índice maestro

> **Coste real: 4.8M tokens de sesión de Max (~80% de su cuota semanal en una tacada).** Por eso este
> workflow queda documentado EXHAUSTIVAMENTE: ninguna información se desperdicia.
> Ejecutado por el peer research (s006, Fable 5) el 2026-07-02: 17 skills GTM cargadas + 105 agentes de
> research, 23 fuentes, 108 claims → **21 confirmados / 4 refutados** (verificación adversarial 3 votos).
> Documentado y curado por Julio (s003, coord/QA).

---

## 📇 Índice de esta carpeta (todo permanente, nada efímero)

| Fichero | Qué es |
|---------|--------|
| `00-INFORME-COMPLETO-deep-research.md` | **El informe bruto ÍNTEGRO** (2577 líneas, 188KB) — los 21 claims confirmados con sus fuentes, los 4 refutados, la cadena de verificación adversarial. **Este es el core de los 4.8M tokens.** |
| `SINTESIS-Y-INDICE.md` | Este documento — síntesis ejecutable + índice + próximos pasos. |
| `fuentes-recogidas/metagpt-paper-arxiv-2308.00352.pdf` | Paper primario MetaGPT (ICLR 2024) — base de H1/H2 (rol PM como prompt+SOP). |
| `fuentes-recogidas/paper.txt` | Extracto de texto del paper MetaGPT (legible/grepeable). |
| `fuentes-recogidas/spec-driven.md`, `sdd.md` | Spec-kit (GitHub) — loop Specify→Plan→Tasks→Implement (base de H3). |
| `fuentes-recogidas/pm-skills-readme.md`, `readme.md` | Product-Manager-Skills (deanpeters, 52 frameworks) — base de H4. ⚠️ licencia CC BY-NC-SA (no comercial). |
| `fuentes-recogidas/ibm-prd.txt` | Estructura de PRD (IBM) — base del artefacto PRD 10 secciones. |
| `fuentes-recogidas/metagpt_readme.md`, `agents.md`, `augment.md`, `ps.md` | Fuentes secundarias sobre agentes PM/SOP/prompts. |
| `fuentes-recogidas/marketplace.json`, `catalog.md` | Catálogo de skills/frameworks PM disponibles. |

---

## 🎯 Los 8 hallazgos clave (verificados)

**H1 — El PM-IA NO es un chatbot: su output son SPECS ESTRUCTURADAS** que actúan como contratos
verificables entre agentes. MetaGPT (arXiv 2308.00352) formaliza el PM como prompt+SOP: input 1 línea
→ PRD + user stories + análisis competitivo. *Probado en benchmarks, no en producción.* [3-0, high]

**H2 — Comunicación por DOCUMENTOS, no diálogo.** Outputs modulares validados en cada handoff =
mecanismo anti-alucinación en cascada. Define el formato de salida obligatorio PM-IA → devs. [3-0, high]

**H3 — El loop más reutilizable es spec-kit: Specify→Plan→Tasks→Implement.** La spec es el artefacto
primario persistente; el código es su expresión generada. Mecanismos verificados:
(a) separación QUÉ/POR QUÉ vs CÓMO (la spec nunca prescribe stack),
(b) marcadores `[NEEDS CLARIFICATION: pregunta]` obligatorios para toda ambigüedad — **nunca inventar**,
(c) checklists como gates antes de pasar de fase. [3-0, high]

**H4 — El conocimiento PM ya existe empaquetado** (Product-Manager-Skills, deanpeters, 52 frameworks):
PRD 10 secciones, user stories Cohn + Gherkin, "prioritization advisor" que elige RICE/ICE/Kano
DINÁMICAMENTE vía 3-5 preguntas (Adaptive Decision Ladder): pre-PMF sin datos → ICE/Value-Effort; con
datos ricos → RICE. ⚠️ **licencia CC BY-NC-SA (no comercial)** — usar como referencia, no copiar tal cual. [3-0, high]

**H5 — ⚠️ ADVERTENCIA ESTRUCTURAL (MAST, Berkeley/ICML 2025):** multi-agente NO supera a agente único
por defecto. La 1ª categoría de fallos multi-agente es la ESPECIFICACIÓN (terreno del PM), pero mejores
specs solo dieron +14% en ChatDev (25%→40.6%, sigue fallando ~60%). El PM-IA se justifica por atacar esa
clase de fallo + paralelización + verificación cruzada, pero exige **rediseño estructural** (gates,
topología, agentes verificadores), no solo un buen system prompt. [3-0, high]

**H6 — Límite al reemplazo total:** el claim de sustitución total del PM humano fue **REFUTADO 0-3**. El
marco 2024-2026 (arXiv 2507.01069) propone CO-EVOLUCIÓN con humano en gobernanza. Teresa Torres: los
resúmenes IA pierden 20-40% del detalle en discovery con clientes. → **Max queda como interfaz humana de
discovery/gobernanza; el PM-IA ejecuta la mecánica** (síntesis, opportunity solution tree, assumption tests).

**H7 — El cierre del loop métricas→spec NO está documentado en ninguna fuente** (el claim de spec-kit sobre
feedback bidireccional producción→spec fue refutado 1-2). **Hay que diseñarlo ex novo:** percepción de
métricas reales (NRR, activation, TTFV, PQL) → revisión de roadmap/spec. **Es nuestra pieza original.**

**H8 — GAP GTM:** la dimensión GTM no produjo claims verificados. Se cubre inyectando las 17 skills GTM
de Max como conocimiento del agente: positioning-icp (four-layer stack de Dunford: category→wedge→proof
vector→alternative framing, revalidación 90 días), ai-pricing (consumption/workflow/outcome, híbrido 41%),
gtm-metrics (NRR>106%, TTFV, PQL 5-15% conv), sales-motion-design (PLG vs sales-led por ACV×complejidad),
expansion-retention (NRR como motor).

---

## 🤖 Propuesta: SYSTEM PROMPT del agente Criador de Producto

```
Eres el CRIADOR DE PRODUCTO del equipo peers [id]. NO escribes código. Tu output son
especificaciones estructuradas que actúan como contratos ejecutables para los devs-IA.
REGLAS DURAS:
(1) Toda ambigüedad → marcador [NEEDS CLARIFICATION: pregunta específica] dirigido a
    Max/orquestador — PROHIBIDO inventar supuestos plausibles.
(2) Tus specs definen QUÉ y POR QUÉ; nunca CÓMO — el stack lo deciden arquitecto/devs.
(3) Ningún artefacto pasa de fase sin su checklist-gate completo (incluye "0 marcadores
    NEEDS CLARIFICATION pendientes").
(4) Eliges el framework de priorización según contexto (etapa, datos, equipo) — no
    hardcodees uno.
(5) Cada feature liga a UNA métrica de producto (activation/TTFV/NRR/PQL) y a la capa GTM
    (positioning four-layer, pricing, motion).
(6) Te comunicas por documentos versionados, nunca por diálogo libre.
(7) Discovery con usuarios reales y decisiones de gobernanza → escalas a Max (human-in-the-loop).
```

## 🔁 Propuesta: LOOP (percepción → decisión → acción → feedback)

1. **PERCIBIR** — inputs: visión/órdenes de Max, feedback del orquestador y devs, métricas de
   producto/GTM disponibles, backlog, señales de mercado (skills GTM).
2. **DECIDIR** — prioriza con framework elegido por contexto; actualiza roadmap now/next/later ligado a outcomes.
3. **ESPECIFICAR** — pipeline spec-kit: Specify (idea→PRD iterativo) → Plan (PRD→plan) → Tasks (tareas
   atómicas con acceptance criteria Gherkin). Gates de checklist entre fases.
4. **DESPACHAR** — entrega specs al orquestador (Julio), que asigna a devs-IA. El PM no implementa.
5. **VERIFICAR** — revisa entregas contra acceptance criteria (PM como agente verificador: ataca FC1/FC3 de MAST).
6. **FEEDBACK** — métricas y resultados → revisión de spec/roadmap. Cadencias: Sean Ellis + PMF trimestral;
   positioning/ICP cada 90 días. **(Pieza a diseñar ex novo, ver H7.)**

## 📦 Artefactos que debe producir

- Documento de visión + positioning four-layer (Dunford) con ICP scoring (fit+intent separados)
- Roadmap now/next/later ligado a outcomes medibles (no fechas)
- PRD estructurado: problema→personas→solución→métricas→stories (10 secciones)
- User stories Cohn + acceptance criteria Gherkin (Given/When/Then); anti-patrones vetados
- **"AI Spec" para devs-IA** — documento de EJECUCIÓN distinto del PRD, 6 elementos: outcomes verificables,
  in/out-of-scope explícito, constraints+assumptions, decisiones ya tomadas, task breakdown, criterios de verificación
- ADRs (decisión + por qué)
- Matriz de priorización documentada (RICE/ICE/Kano según contexto)
- Dashboard métricas producto+GTM: activation, TTFV, NRR, PQL/PQA, Sean Ellis (trimestral)
- Opportunity Solution Tree (Torres) para discovery — con Max como fuente de señal de usuarios reales

---

## ⚠️ Caveats (honestidad intelectual)

- **NO existe ningún caso documentado de PM-IA completo en producción** sustituyendo a un PM humano.
- Validación de MetaGPT es en benchmarks; techo ~40% éxito en ChatDev tras mejoras (H5).
- El reemplazo TOTAL del PM humano fue refutado (H6): el modelo viable es **co-evolución** (Max en gobernanza).
- El cierre métricas→spec (H7) no tiene precedente documentado: es diseño original nuestro.

## 📚 Fuentes primarias clave
- arXiv 2308.00352 — MetaGPT (PM como prompt+SOP) → `fuentes-recogidas/metagpt-paper-*.pdf`
- github/spec-kit — loop Specify→Plan→Tasks→Implement → `fuentes-recogidas/spec-driven.md`, `sdd.md`
- deanpeters/Product-Manager-Skills — 52 frameworks (CC BY-NC-SA) → `fuentes-recogidas/pm-skills-readme.md`
- arXiv 2503.13657 — MAST (fallos multi-agente) → ver informe completo
- arXiv 2507.01069 — co-evolución humano-IA en gobernanza → ver informe completo
- producttalk.org (Teresa Torres) — discovery, opportunity solution tree → ver informe completo

---

## ➡️ Próximo paso (pendiente de Max)

Diseñar el agente Criador de Producto como **peer Claude persistente** (tmux) a partir de esta síntesis:
system prompt final + fichero de agente / arranque + su lugar en la topología (PM-IA → Julio → devs).
**Julio NO lanza más research** (orden de Max: research congelado por coste). Este diseño se hace CON Max
cuando él lo decida, sobre esta documentación ya persistida.

#deep-research #criador-producto #pm-ia #documentado #4.8M-tokens
