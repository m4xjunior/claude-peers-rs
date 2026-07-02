# RFC — Proyectos (el "apartado de proyectos": workspaces aislados de la empresa)

> ⬆ [[../_MOC|Mapa]] · Modelo: [[../01-modelo-corporativo]] · Dominio: [[../02-modelo-dominio]] ·
> GPUI: [[../00-fundamentos-gpui]]
>
> Fecha: 2026-07-02. Estado: **PROPUESTO — NO implementar hasta aprobar.**
> Pantalla nueva: `crates/peers-desktop/src/vista/proyectos.rs` (pestaña del sidebar).
> Design System: **Ethos** (TINTA `#100D0A`, TINTA2 `#1A1611`, PAPEL `#ECE5D7`, BRASA `#C9A96E`,
> HUMO `#938B7B`, LINEA `#2B271F`; Fraunces/Inter/IBM Plex Mono; radios card 14 / control 10 / pill 999).
> Relacionada con: [[../../desktop/lanzador/RFC-lanzador]] (lanza el equipo del proyecto),
> [[RFC-organigrama-roles]] (define su equipo), [[../../desktop/acceso/RFC-acceso]] (hosts SSH).

---

## 1. El problema (en palabras de Max)

> "Tengo apartado de proyectos… tiene que haber cómo yo desarrollar y crear mis proyectos… controlar el
> servidor SSH o carpeta local donde estarán [los agentes]."

Hoy `peers-desktop` observa **un** broker con **todos** los peers mezclados. No hay noción de "proyecto":
no se pueden agrupar agentes por trabajo, ni aislar equipos, ni ver el tablero de UN proyecto, ni relanzar
un equipo. Max no puede "crear un proyecto" ni tener su equipo dentro. Todo es una sola lista plana de
instancias. El "apartado de proyectos" —la unidad organizativa básica de la empresa— no existe como
entidad.

## 2. La solución

Una pestaña **Proyectos** que introduce el **contenedor aislado** del modelo corporativo
([[../01-modelo-corporativo]] §4): cada proyecto tiene su **ubicación** (carpeta local o host SSH), su
**equipo** de agentes, su **tablero** de tareas, su **política** local y su **registro de acciones**. Los
proyectos se crean, editan, archivan y **relanzan** (perfil de lanzamiento). El aislamiento se logra por
convención de id `rol@proyecto` (sin infra nueva, [[../02-modelo-dominio]] §7).

**Sin infraestructura nueva:** un proyecto es (a) config del operador en `config.toml` (`Proyecto`,
[[../02-modelo-dominio]] §2.2) + (b) un **filtro/vista** sobre el estado vivo que ya trae la app del broker.

---

## 3. Requisitos (trazables)

### Modelo y persistencia

- **R1 — Entidad `Proyecto`.** `{ id (slug), nombre, ubicacion (Local|Ssh|Tmux), agentes: Vec<id>,
  creado_en, archivado }` ([[../02-modelo-dominio]] §2.2). Persistida en `config.toml`, sección
  `[[proyecto]]`, con `#[serde(default)]` para compat (config viejo sin proyectos deserializa).
- **R2 — Aislamiento por convención de id.** El proyecto NO cambia el esquema del broker: agrupa por el
  sufijo `@<proyecto_id>` de los ids de agente ([[../02-modelo-dominio]] §5, §7). La app filtra
  `instancias`, tareas, jornada, alertas y acciones por ese sufijo. (v2: `proyecto_id` server-side si hace
  falta — fuera de v1.)
- **R3 — Ubicación.** Carpeta local (elegida con `cx.prompt_for_paths`, picker nativo, cero deps) **o**
  host SSH (de la lista de RFC Acceso) **o** tmux. Validar que la carpeta local existe antes de habilitar
  "Lanzar".

### CRUD de proyectos

- **R4 — Crear proyecto.** `Dialog` Ethos: nombre + elegir ubicación (botón "Elegir carpeta" → picker
  nativo, o `Select` de host SSH + ruta remota). Al crear, se timbra `creado_en` (broker) y se persiste.
  Opcional: clonar el equipo de un preset/otro proyecto.
- **R5 — Editar / renombrar / cambiar ubicación.** Sobre un proyecto existente. Cambiar la ubicación
  reubica el equipo por defecto (los agentes heredan la nueva salvo override).
- **R6 — Archivar / reactivar.** `archivado=true` lo saca de la vista activa sin borrar su historia
  (bitácora, tareas cerradas). Reactivable. Borrado real = acción destructiva con confirmación.
- **R7 — Duplicar proyecto (plantilla).** Copia estructura de equipo (cargos) y perfil de lanzamiento a un
  nuevo proyecto con otra ubicación. Acelera montar proyectos parecidos.

### Vista del proyecto (el tablero)

- **R8 — Ficha de proyecto.** Al abrir un proyecto: cabecera (nombre BRASA + ubicación mono + estado) y
  secciones: **Equipo** (agentes del proyecto con su estado vivo/ocioso/pausado), **Tablero** (tareas del
  proyecto: abiertas/en curso/bloqueadas/hechas, estimado vs real), **Actividad** (últimas acciones del
  equipo, registro-acciones filtrado), **Alertas** (del supervisor, filtradas por proyecto).
- **R9 — Lanzar el equipo del proyecto (un click).** Botón "Lanzar" que despliega el perfil de lanzamiento
  del proyecto vía la RFC Lanzador (una sesión `claude` por agente, id `rol@proyecto`, en su
  carpeta/host/tmux). Ver [[RFC-organigrama-roles]] para la definición del equipo y [[../../desktop/lanzador/RFC-lanzador]]
  para el mecanismo. Lanzar por-agente o todo-el-equipo.
- **R10 — Estado vivo del proyecto.** Cruza los ids `rol@proyecto` contra `/listar` para mostrar cuántos
  agentes del equipo están vivos, ociosos o caídos. Refresco reusando el patrón `desktop-carga-datos` (D2).

### Navegación e integración

- **R11 — Selector de proyecto global.** Un `Select`/breadcrumb en la cabecera de la app que fija el
  "proyecto activo"; las demás pestañas (Peers, Tareas, Jornada, Alertas…) pueden **filtrar por el proyecto
  activo** (toggle "todos / este proyecto"). Da el "apartado" real sin duplicar pantallas.
- **R12 — Política local del proyecto (preset).** Botón "Aislar este proyecto" que añade a la política
  global la regla `otros/* → <proyecto>/*: bloquear` (y viceversa), usando el motor existente (Fase 1). Es
  opt-in; por defecto los proyectos no se aíslan.

### Transversal

- **R13 — Todo degrada.** Broker offline, carpeta inexistente, host SSH caído → banner Ethos
  (`banner_error`), la app sigue viva. Sin `.unwrap()`/`.expect()` en prod. Config viejo sin proyectos
  deserializa (`#[serde(default)]`).
- **R14 — Red en background.** Todo cruce con el broker (listar, jornada, tareas, alertas por proyecto) va
  por `background_executor` + `bloquear_en` (regla anti-SIGABRT, [[../00-fundamentos-gpui]] §5).

---

## 4. Diseño de UI (gpui-component, tema Ethos)

Componentes del kit: `Table`/lista de cards, `Select`, `Button`, `Dialog`, `Tabs`, `Badge`, `Breadcrumb`,
`Tooltip`. Helpers Ethos de `tema.rs`: `superficie_card`, `eyebrow`, `titulo`, `chip_estado`,
`boton_primario/secundario`, `fila_seleccionable`, `banner_error`.

```
┌ Proyectos ──────────────────────────────────────────────────────────────┐
│  [ + Nuevo proyecto ]              activo: proyecto-x ▾   [ ● Lanzar equipo ]│
│ ┌───────────────┐ ┌───────────────┐ ┌───────────────┐                     │
│ │ proyecto-x    │ │ proyecto-y    │ │ (archivados)  │                     │
│ │ /Users/max/px │ │ ssh: otus     │ │  …            │                     │
│ │ 4 agentes ●●●○│ │ 2 agentes ●○  │ │               │                     │
│ └───────────────┘ └───────────────┘ └───────────────┘                     │
├──────────────────────── proyecto-x (ficha) ──────────────────────────────┤
│ [Equipo] [Tablero] [Actividad] [Alertas]                                  │
│  Equipo:  backend@proyecto-x ●vivo · qa@proyecto-x ○ocioso · …            │
│  Tablero: ▸ 3 abiertas · 1 bloqueada · 8 hechas (est ×1.4 → 12 muestras)  │
└───────────────────────────────────────────────────────────────────────────┘
```

Variantes a decidir en Design: grid de cards vs tabla; ficha como pestaña vs drawer lateral; el selector de
proyecto activo como breadcrumb vs `Select` fijo.

---

## 5. Criterios de aceptación

- **AC1 (R4/R1)** — Crear un proyecto con carpeta local elegida en el picker persiste en `config.toml`;
  reabrir la app lo muestra. Cancelar el picker (None) no rompe nada.
- **AC2 (R2)** — Con dos proyectos, el filtro por proyecto activo muestra solo los agentes/tareas cuyo id
  termina en `@<proyecto>`; "todos" los muestra mezclados como hoy.
- **AC3 (R9)** — "Lanzar equipo" arranca una sesión por agente del proyecto con id `rol@proyecto` en su
  ubicación (verificable: aparecen en `/listar` con ese id). Un host caído → banner, sin crash (R13).
- **AC4 (R10)** — La ficha muestra cuántos agentes del equipo están vivos cruzando con `/listar`; al morir
  uno, se refleja al refrescar.
- **AC5 (R12)** — "Aislar este proyecto" añade la regla de política y un `/enviar` cross-proyecto recibe
  `ok:false`; quitarla lo restaura. (Reusa el motor de política Fase 1.)
- **AC6 (R6 compat)** — Archivar un proyecto lo oculta sin perder su bitácora; reactivar lo restaura.
- **AC7 (compat)** — `config.toml` sin sección `[[proyecto]]` deserializa; la app opera como hoy (sin
  proyectos = lista plana, comportamiento actual).

---

## 6. Riesgos y decisiones abiertas (Design)

1. **Aislamiento v1 (convención de id) vs v2 (server-side).** v1 filtra en cliente por `@proyecto`; es
   suficiente y cero-backend. Si Max quiere garantías duras (que el broker no ruteé cross-proyecto sin
   política), es v2 (`proyecto_id` en tareas/mensajes). **Recomendación: v1**, endurecer si hace falta.
2. **Id `rol@proyecto` depende del binding del Lanzador.** Requiere que `peers-client` respete
   `CLAUDE_PEERS_ID` explícito ([[../02-modelo-dominio]] §5). Es la única dependencia de backend; resolver
   antes de R9.
3. **¿Proyecto multi-host?** El modelo lo permite (cada agente su ubicación). Decidir si la UI v1 asume una
   ubicación por proyecto (simple) o permite override por agente desde el principio. **Recomendación:** una
   por proyecto + override opcional en la ficha del agente.
4. **Borrado real vs archivado.** El archivado preserva la bitácora (trazabilidad). El borrado real debe
   ser explícito y confirmado, y NO debería borrar la historia en `bitacora.db` (coherente con ADR-001,
   `ON DELETE SET NULL`/`RESTRICT`, sin cascade destructivo).

---

## 7. Constraints

- Sin `.unwrap()`/`.expect()` en prod; `Result`/`anyhow`. Red SIEMPRE en `background_executor` +
  `bloquear_en`. Español salvo protocolo. Reusa DTOs de `peers-core`; NO redefinir. Reusa endpoints
  existentes (`/listar`, `/jornada`, `/listar-tareas`, `/admin/alertas`, `/admin/politica`, `/acciones`) y
  el file picker nativo (cero deps). Config en el mismo `config.toml`. Versionar plugin solo si se toca
  algún binario (aquí, idealmente no: proyecto = config + vista). NUNCA `Co-Authored-By`. Jornada en el commit.

## 8. Fuera de alcance (v1)

- Filtrado server-side por `proyecto_id` (v2).
- Métricas agregadas cross-proyecto ("toda la empresa de un vistazo") — v2, encaja con un dashboard global.
- Permisos duros por proyecto/rol en el broker (v2, [[RFC-organigrama-roles]] §capacidades).
- Editor de código / árbol de archivos (no es un IDE — igual que RFC Lanzador §9).

## 9. Dependencias

- **[[RFC-organigrama-roles]]** — define el equipo (cargos/agentes) que un proyecto contiene y lanza.
- **[[../../desktop/lanzador/RFC-lanzador]]** — el mecanismo de lanzamiento (PTY/SSH/tmux) del equipo (R9).
- **Binding id↔sesión** ([[../02-modelo-dominio]] §5) — `peers-client` respeta `CLAUDE_PEERS_ID` explícito.
- **[[../../desktop/politica-comunicacion/RFC-politica-comunicacion]]** — motor de aislamiento (R12).
- **[[../../features/desktop-carga-datos/spec]]** — patrón de refresco para el estado vivo del proyecto.

---
#rfc #empresa #proyectos #workspace #aislamiento #peers-desktop
