# UX de las vistas de Configuración — inspiración de mercado adaptada a peers-desktop

> ⬆ [[_MOC|Mapa]] · Capa UI: [[08-capa-ui]] · GPUI: [[00-fundamentos-gpui]]
>
> Fecha: 2026-07-03. Estado: **INSPIRACIÓN DE UX — para el implementador de las vistas config (#8 del paquete de Max).**
> Max pidió: *"pesquise en context7 cómo deberían ser las vistas de configuración de softwares... como
> en otros mercados hacen funciones semejantes, sabiendo que el mío es totalmente diferente."*
> context7 sólo devolvió extensiones/MCP de VS Code (no UX conceptual), así que esto se construye desde
> patrones REALES de mercado que conozco, mapeados 1:1 a lo que peers-desktop ES (un panel de control de
> agentes IA, NO un SaaS web). Tema **Ethos**, gpui-component real, cero suposiciones sobre el kit.

---

## 0. El problema con la Config de HOY

Hoy `Config` (config.rs) es **minimalista y plana**: 3 campos (`broker_url`, `token`, `refresh_ms`) + botones
(Guardar / Probar conexión / Recargar del disco / Restablecer). Funciona, pero:

1. **No escala.** El proyecto YA tiene mucho más configurable que NO está expuesto: umbrales del supervisor
   (ocioso 600s / atasco 1800s / ghosteo 300s — `peers-core/src/lib.rs:712-718`), vencimiento de liveness
   (45s, `lib.rs:21`), umbral real mínimo (30s), ratio de cancelación (0.4), la política de comunicación.
2. **Va a crecer MUCHO** con la arquitectura empresa: cargos, proyectos, plantillas de system prompt,
   ubicaciones SSH. Una lista plana de inputs no aguanta eso.
3. **Mezcla dos naturalezas distintas** que el mercado separa: config de **conexión** (broker_url/token —
   "cómo me conecto") vs config de **comportamiento** (umbrales, política — "cómo opera el sistema") vs
   entidades **vivas** (peers, cargos, proyectos — "qué existe").

---

## 1. Qué hacen OTROS MERCADOS (referentes reales, no genéricos)

Cuatro patrones de mercado, cada uno resuelve un problema que peers-desktop tiene:

| Referente | Patrón que aporta | Qué copiar para peers-desktop |
|-----------|-------------------|-------------------------------|
| **VS Code — Settings** | Búsqueda en vivo + árbol de categorías a la izquierda + settings agrupados a la derecha + **badge "modified"** en lo cambiado + **scope** (User / Workspace) | La **búsqueda de settings** (cuando haya 20+ opciones) y el **badge de "modificado sin guardar"** por campo. El scope User/Workspace mapea a **Global / por-Proyecto** (encaja con la arquitectura empresa). |
| **Linear — Settings** | Sidebar de secciones + una **página por sección** + **guardado inline** (sin botón "Guardar" global; cada cambio se persiste o se confirma en su sitio) + estética sobria | La **navegación por secciones** (no todo en una pantalla) y el **guardado por sección/campo**, no un botón global que guarda todo a ciegas. |
| **Vercel / Stripe — Dashboard settings** | **Cards por área** con inline-edit + **estado de conexión visible** (verde/rojo) arriba + acciones destructivas separadas y confirmadas ("Danger zone") | Las **cards por área** (ya las tienes con `superficie_card`), el **estado de conexión** siempre visible (ya lo tienes en Broker/Acceso), y una **zona de acciones peligrosas** separada (Restablecer, purgar). |
| **Tailscale / 1Password — gestión de entidades vivas** | Config que NO son campos sino **cosas vivas** (dispositivos/conexiones): lista con **estado + identidad copiable + acciones por fila** | Esto es CLAVE para peers-desktop: los **peers/cargos/proyectos** son entidades vivas, no settings. Se gestionan como Tailscale gestiona dispositivos: fila con estado ● + id copiable + acciones. (Conecta con #4 "copiar id" y el organigrama.) |

**La conclusión de mercado:** ninguno mete todo en una lista de inputs. **Separan config de conexión, config
de comportamiento, y entidades vivas** — tres modos de UI distintos. peers-desktop debe hacer lo mismo.

---

## 2. Propuesta para peers-desktop (Ethos, GPUI) — 3 modos, no 1

peers-desktop es **distinto** de todos ellos (no es web, no es SaaS, es un panel de control local de agentes).
Pero los 3 modos aplican. Reorganizar "Config" en **secciones navegables** dentro de la pestaña, con un
sub-sidebar o tabs (patrón Linear), agrupadas por naturaleza:

```
┌ Configuración ─────────────────────────────────────────────────────────────┐
│ ┌──────────────┐  ● conectado · broker 0.0.0.0:7899 · 9 instancias          │  ← estado siempre visible (Vercel)
│ │ CONEXIÓN     │  ┌────────────────────────────────────────────────────┐    │
│ │ COMPORTAMIENTO│ │  (contenido de la sección activa)                   │    │
│ │ POLÍTICA     │  │                                                     │    │
│ │ ─────────    │  │                                                     │    │
│ │ ⚠ Avanzado   │  └────────────────────────────────────────────────────┘    │
│ └──────────────┘                                                             │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 2.1 — CONEXIÓN (lo que hoy es toda la Config)
`broker_url`, `token` (con máscara), `refresh_ms`. Estado de conexión arriba (verde/rojo, patrón Vercel).
Guardar + Probar conexión. **Es lo que ya existe** — solo se le añade el badge de "modificado" por campo
(patrón VS Code) para que Max vea qué tocó antes de guardar. **Ya casi está hecho** (config.rs).

### 2.2 — COMPORTAMIENTO (nuevo, expone lo que hoy está oculto)
Los umbrales del supervisor que hoy son constantes de compilación (`lib.rs`): ocioso, atasco, ghosteo,
vencimiento de liveness, real-mínimo, ratio-cancelación. Cada uno un input numérico con su **unidad** (seg)
y su **default** mostrado (patrón VS Code: "default: 600"). **Requisito de backend**: hoy son `const`; para
editarlos hace falta `GET/POST /admin/umbrales` (ya está documentado como pendiente de backend en
[[../desktop/broker/RFC-broker]] y la memoria operativa). Mientras no exista, esta sección los muestra
**read-only con nota "valor de compilación, edición pendiente de backend"** (honestidad — no fingir que se
guardan).

### 2.3 — POLÍTICA (nuevo, ya tiene backend)
La política de comunicación (firewall peer↔peer) YA existe (`GET/POST /admin/politica`). Aquí se listan las
reglas `(de, para) → permitir|bloquear` como **filas editables** (patrón Tailscale: entidades vivas con
acciones por fila), con "añadir regla" y el orden (primera que casa gana). Estado de bloqueos activos.

### 2.4 — ⚠ AVANZADO / Danger zone (patrón Stripe)
Acciones destructivas separadas y confirmadas en 2 pasos: Restablecer config, purgar colas, kick masivo.
Visualmente apartadas (borde/acento de peligro `boton_peligro`) para no tocarlas por error.

---

## 3. Principios de mercado que peers-desktop debe adoptar (checklist para el implementador)

1. **Estado de conexión SIEMPRE visible** (arriba, verde/rojo) — patrón Vercel/Stripe. Ya lo tienes en
   Broker/Acceso; llévalo al header de Config.
2. **Badge "modificado sin guardar" por campo** — patrón VS Code. config.rs ya tiene el snapshot de
   dirty-state (`config-12`); solo falta pintar el badge junto al campo cambiado.
3. **Agrupar por naturaleza, no lista plana** — patrón Linear. Conexión / Comportamiento / Política /
   Avanzado como secciones navegables (sub-tabs o sub-sidebar dentro de la pestaña Config).
4. **Guardado con scope explícito** — patrón VS Code User/Workspace → aquí **Global / por-Proyecto**
   (encaja con la arquitectura empresa; hoy todo es Global, se prepara el terreno).
5. **Entidades vivas ≠ settings** — patrón Tailscale. Peers/cargos/proyectos se gestionan como filas con
   estado + id copiable + acciones, NO como campos de texto. (Conecta con #4 copiar-id y el organigrama.)
6. **Danger zone separada y confirmada** — patrón Stripe. Nunca una acción destructiva junto a un input normal.
7. **Búsqueda de settings** cuando haya 15+ opciones — patrón VS Code. No hoy (hay pocas), pero dejar el
   hueco en el header de la sección Comportamiento cuando crezca.
8. **Honestidad de lo no-editable** — lo que aún no tiene backend (umbrales) se muestra read-only con nota,
   NO como si se guardara. (Es la regla del proyecto: no fingir estado.)

---

## 4. Qué NO copiar (peers-desktop es distinto)

- **No** un dashboard web con gráficas de métricas (es un panel local, no analytics).
- **No** login/cuentas/organizaciones multi-usuario (un solo operador = Max, `ID_OPERADOR`).
- **No** wizards de onboarding de SaaS (Max ya sabe lo que hace; nada de tours).
- **No** guardar-en-la-nube ni sync (config local en `config.toml`, filosofía del proyecto).

---

## 5. Alcance para HOY (mínimo viable, el resto es evolución)

Max quiere terminar hoy. El **mínimo de valor inmediato** sin backend nuevo:
1. Añadir a la Config actual el **badge de "modificado"** por campo (dirty-state ya existe) + el **estado de
   conexión visible** en el header. Barato, alto valor. (patrones VS Code + Vercel)
2. Reagrupar visualmente en secciones (aunque sea con eyebrows/cards separadas: Conexión / Comportamiento /
   Política) — sin sub-router aún si no da tiempo. (patrón Linear, versión ligera)
3. Sección Comportamiento **read-only** mostrando los umbrales reales de compilación (transparencia — hoy
   Max ni los ve). Editarlos = cuando exista `/admin/umbrales`.

Lo demás (sub-sidebar completo, scope por proyecto, política editable inline, búsqueda) es evolución que se
construye encima sin retrabajo.

---
#empresa #ux #config #mercado #ethos #gpui #inspiracion
