# Organigrama — diseño visual Ethos (detalle completo)

> ⬆ [[_MOC|Mapa]] · Modelo: [[01-modelo-corporativo]] · Roles: [[rfc/RFC-organigrama-roles]] ·
> Delegación: [[rfc/RFC-delegacion-cadena-mando]] · GPUI: [[00-fundamentos-gpui]] · UI: [[08-capa-ui]]
>
> Fecha: 2026-07-03. Estado: **DISEÑO VISUAL — para revisar/decidir en Design.**
> El "desenhar o organograma em detalhe visual" que pidió Max. Diseño pixel-level en tema **Ethos**,
> implementable en GPUI (div+flexbox, verificado en [[00-fundamentos-gpui]]). Dibuja las **tres
> relaciones** del organigrama ([[01-modelo-corporativo]] §6) con el **estado vivo** del equipo.

---

## 0. Qué dibuja el organigrama

Un grafo del equipo (de un proyecto o de toda la empresa) con **nodos** (operador / cargo / agente) y
**tres tipos de arista**: **reporta-a** (cadena de mando), **habla-con** (política de comunicación) y
**supervisa-a** (derivada). Cada nodo-agente muestra su **estado vivo** cruzando `/listar` +
`/admin/alertas`. Es el hub de operación: de un vistazo, Max ve quién trabaja, quién manda a quién, y quién
está atascado.

---

## 1. Tokens Ethos (la base — copiar exacto)

Consistentes con todo el vault. Fuente: `_MOC` y `tema.rs`.

```
Fondos          TINTA   #100D0A   (fondo app, overlay al 60%)
                TINTA2  #1A1611   (superficies: nodos, cards)
Texto           PAPEL   #ECE5D7   (texto principal, nombres)
                HUMO    #938B7B   (texto terciario, labels, mono)
Acento          BRASA   #C9A96E   (dorado brasa: foco, vivo, acento primario)
Líneas          LINEA   #2B271F   (bordes, aristas neutras, separadores)

Tipografía      Fraunces      títulos de sección, nombre del proyecto/empresa
                Inter         UI, nombres de agente, labels
                IBM Plex Mono datos: id rol@proyecto, timestamps, contadores

Radios          card   14px    (nodos, contenedor)
                control 10px    (chips, botones)
                pill    999px   (badges, estado)

Estados (paleta de dominio, reusada de la TUI)
                vivo       BRASA    #C9A96E   (latió < 45s, trabajando)
                ocioso     HUMO     #938B7B   (vivo sin tarea)
                atascado   ÁMBAR    #C77D3C   (tarea sin avance / bloqueada)
                pausado    GRIS     #5B554B   (kick / no lanzado)
                caído      ROJO-ap  #7F1D1D   (no late / error)
```

---

## 2. Layout general (v1 = árbol vertical con aristas de política superpuestas)

Decisión de diseño (ver [[06-decisiones]]): **v1 = árbol vertical** de `reporta-a` (simple en flexbox) con
las aristas `habla-con` **superpuestas** como líneas punteadas; grafo libre con layout automático = v2.
El **Operador (Max)** es siempre la raíz.

```
┌─ Organigrama ─────────────────────────────────────────────────  [ proyecto-x ▾ ] [ empresa ⌂ ] ─┐
│  ◈ LexusFX · proyecto-x                                    4 agentes · 3 vivos · 1 atascado        │
│  Leyenda:  ──── reporta-a    ┈┈┈┈ habla-con    ◦ vivo  ◦ ocioso  ◦ atascado  ◦ pausado           │
│                                                                                                    │
│                              ┌───────────────────────────┐                                        │
│                              │  ◆  OPERADOR              │   ← raíz; borde BRASA doble; no-nodo-IA │
│                              │     Max · dueño            │                                        │
│                              └─────────────┬─────────────┘                                        │
│                                            │ (reporta-a)                                           │
│                              ┌─────────────┴─────────────┐                                        │
│                              │  ●  coordinador@proyecto-x │   ● vivo (punto BRASA)                 │
│                              │     Coordinador · PM       │                                        │
│                              │     ▸ 2 tareas · est ×1.3  │                                        │
│                              └──────┬───────────────┬─────┘                                        │
│                    (reporta-a)      │               │      (reporta-a)                             │
│              ┌──────────────────────┴──┐         ┌──┴────────────────────────┐                     │
│              │  ●  backend@proyecto-x   │┈┈┈┈┈┈┈┈┈│  ◐  qa@proyecto-x         │  ← ┈ habla-con      │
│              │     Backend Rust         │(habla)  │     QA visual             │                     │
│              │     ▸ 3 tareas ·  ●vivo  │         │     ▸ 1 tarea · ◐atascado │                     │
│              └──────────────────────────┘         └───────────────────────────┘                     │
│                                                                                                    │
│  [ + Contratar ]  [ Editar cargos ]  [ Aislar proyecto ]              actualizado hace 4s ↻        │
└────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Contenedor:** `fondo_app` TINTA; padding 24px; título Fraunces PAPEL con eyebrow mono/HUMO
  ("◈ LexusFX · proyecto-x").
- **Cabecera derecha:** selector de ámbito (`proyecto-x ▾` / `empresa ⌂`) — alterna entre el organigrama de
  un proyecto y el de toda la empresa. Contador vivo (`4 agentes · 3 vivos · 1 atascado`), mono/HUMO.
- **Leyenda** (segunda línea): tipos de arista + estados, en mono/HUMO 12px. Siempre visible (accesibilidad:
  el color no es el único canal — hay glifos `● ◐ ◦`).
- **Aristas reporta-a:** líneas sólidas LINEA de 1.5px, verticales, con codo en L (flexbox: columnas + un
  conector). **Aristas habla-con:** punteadas HUMO 1px, horizontales entre pares. **Supervisa-a:** no se
  dibuja como arista (sería ruido); se expresa con el badge de alerta en el nodo + el escalado (§4).
- **Pie:** acciones (`+ Contratar`, `Editar cargos`, `Aislar proyecto`) + sello de refresco mono/HUMO con
  spinner BRASA (patrón `desktop-carga-datos` D2).

---

## 3. Anatomía del nodo (tres variantes)

Cada nodo es una `superficie_card` (TINTA2, radio 14, borde LINEA 1px, padding 12–14px). Las tres
variantes:

### 3.1 — Nodo OPERADOR (la raíz, Max)

```
┌───────────────────────────┐   · borde: BRASA doble (2px) — se distingue de los agentes IA
│  ◆  OPERADOR              │   · glifo ◆ BRASA (rombo = humano/dueño)
│     Max · dueño            │   · nombre Inter PAPEL 15px semibold; "dueño" HUMO 12px
└───────────────────────────┘   · sin estado vivo (siempre presente); sin métricas
```

### 3.2 — Nodo AGENTE (empleado vivo)

```
┌──────────────────────────────┐   · borde LINEA 1px; al foco → BRASA 1.5px (anillo)
│  ●  backend@proyecto-x       │   · ● = punto de estado (color por estado §1), 10px, izquierda
│     Backend Rust              │   · línea 1: id rol@proyecto  → IBM Plex Mono 13px, BRASA
│  ┌────────┐                   │   · línea 2: nombre del cargo → Inter PAPEL 14px
│  │▸3 tareas│ est ×1.4  ⚠1     │   · fila de métricas: chips pill 999
│  └────────┘                   │      - "▸3 tareas" chip TINTA/HUMO
└──────────────────────────────┘      - "est ×1.4" factor (verde ~1.0 / ámbar >1.3 / naranja <0.7)
                                       - "⚠1" badge de alertas vivas (ámbar), oculto si 0
```

Anatomía por capas (de arriba a abajo dentro del nodo):
1. **Fila identidad:** `●` estado + id mono BRASA + (opcional) glifo de cargo.
2. **Nombre del cargo:** Inter PAPEL.
3. **Fila de métricas (chips):** nº tareas abiertas · factor de estimación · badge de alertas. Cada chip
   es pill 999, altura 20px, texto mono 11px. Badge de alerta solo si >0 (ámbar), abre el panel de alertas
   del agente al click ([[rfc/RFC-organigrama-roles]] R12).

### 3.3 — Nodo CARGO vacío (contratado sin lanzar / plantilla)

```
┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐   · borde LINEA PUNTEADO (no sólido) = "sin presencia"
   ◦  qa@proyecto-x               · punto ◦ gris (pausado/no-lanzado)
      QA visual · contratado       · "contratado" en HUMO 11px
   [ ▶ Lanzar ]                    · botón "Lanzar" inline (dispara el pipeline, doc 03)
└ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┘
```

Distingue **identidad durable sin presencia** (contratado, apagado, borde punteado) de **presencia viva**
(borde sólido, punto de color). Es la traducción visual de la máquina de estados del agente
([[03-pipeline-provision]] §2).

---

## 4. Estados vivos y su semántica visual

El punto `●`/`◦`/`◐` a la izquierda del id codifica el estado (cruzando `/listar` + `/admin/alertas`):

| Glifo | Color | Estado | Origen del dato |
|-------|-------|--------|-----------------|
| `●` | BRASA | **vivo** trabajando (tiene tarea en curso, latió <45s) | `/listar` + `/listar-tareas` |
| `◦` | HUMO | **ocioso** (vivo sin tarea) | alerta `TipoAlerta::Ocioso` (`lib.rs:677`) |
| `◐` | ÁMBAR | **atascado** (tarea sin avance / bloqueada) | alerta `Atascado` / `EstadoTarea::Bloqueada` |
| `◦` | GRIS | **pausado** (kick) / **contratado** (no lanzado) | ausente de `/listar` |
| `✕` | ROJO-ap | **caído** (no late, error) | vencido `VENCIMIENTO_MS` sin baja limpia |

- **Doble canal (accesibilidad):** el estado se lee por **color Y glifo** (`● ◦ ◐ ✕`), nunca solo color
  (peers-18: la queja transversal de accesibilidad). El tooltip del punto da el texto ("atascado desde
  hace 12m") con la antigüedad relativa (mismo cálculo `time` que las alertas).
- **Halo de foco:** el nodo seleccionado gana un anillo BRASA (1.5px) + fondo TINTA2 un punto más claro
  (`fila_seleccionable` ya existe en `tema.rs`).

---

## 5. Las tres aristas (detalle de trazo)

| Relación | Trazo | Color | Grosor | Dirección | Fuente del dato |
|----------|-------|-------|--------|-----------|-----------------|
| **reporta-a** | sólida | LINEA (neutra) | 1.5px | vertical, codo en L (superior→inferior) | `cargo.reporta_a` |
| **habla-con** | punteada | HUMO | 1px | horizontal entre pares | política (`GET /admin/politica`) |
| **supervisa-a** | (no dibujada) | — | — | — | derivada de reporta-a invertido + supervisor |

- **reporta-a** es el esqueleto del árbol (define la posición vertical de cada nodo). El operador arriba;
  cada nivel de mando, una fila.
- **habla-con** se **superpone**: por defecto todos pueden hablar (no se dibujan aristas si la política es
  "todo permitido"); solo se dibujan las **restricciones** (un bloqueo = arista punteada ROJO-ap tachada
  entre dos nodos) o, en modo "ver quién habla con quién", las permitidas explícitas. Alterna con un toggle
  "Ver comunicación".
- **supervisa-a** no ensucia el grafo: se expresa con el badge de alerta en el nodo supervisado + la
  acción de **escalado** (una alerta de `backend` muestra "→ escalada a coordinador", RFC Delegación R6).

---

## 6. Interacción

| Gesto | Efecto |
|-------|--------|
| **Hover** sobre nodo | eleva la card (sombra sutil), muestra tooltip con id completo + resumen + antigüedad de estado |
| **Click** en nodo-agente | abre la **ficha de empleado** (drawer derecho): identidad, cargo, jornada, tareas, alertas, chat privado ([[08-capa-ui]]) |
| **Click** en nodo-cargo vacío | abre el **editor de cargo** (system prompt + cadena + capacidades) |
| **Click** "▶ Lanzar" | dispara el pipeline de provisión+lanzamiento ([[03-pipeline-provision]]) desde el nodo |
| **Click** en badge ⚠ | abre el panel de alertas del agente (resolver/escalar) |
| **Toggle** "Ver comunicación" | superpone/oculta las aristas `habla-con` |
| **Selector** proyecto ▾ / empresa ⌂ | conmuta el ámbito del grafo |
| **Teclado** ↑↓←→ | mueve el foco entre nodos (anillo BRASA); Enter abre ficha; `l` lanza; `e` edita cargo (paridad peers-18) |

Refresco: reusa el patrón de `desktop-carga-datos` (D2) — auto-refresh con sello "actualizado hace Ns" y
spinner BRASA mientras carga. Red SIEMPRE en `background_executor` + `bloquear_en`
([[00-fundamentos-gpui]] §5).

---

## 7. Variantes de diseño (a decidir en Design)

1. **Árbol vertical (v1, recomendada)** — simple en flexbox (columnas + conectores), legible hasta ~15
   nodos. La de los mockups de arriba.
2. **Grafo libre (v2)** — nodos posicionables con layout automático (force-directed) y las 3 aristas como
   tipos visuales distintos. Más potente para equipos grandes / relaciones densas, pero complejo en GPUI
   (requiere cálculo de layout propio o un elemento custom). Diferida.
3. **Matriz peer×peer (complementaria)** — para la política de comunicación, una rejilla donde cada celda
   es permitir (BRASA tenue) / bloquear (ROJO-ap), clicable (ya propuesta en RFC política R11). Vista
   alternativa al toggle "Ver comunicación", útil con muchos peers.

**Ámbito:** por-proyecto (default, equipo de un proyecto) vs empresa (todos los proyectos como super-nodos
que se expanden). v1: por-proyecto + un modo empresa que agrupa por proyecto.

---

## 8. Implementación GPUI (verificada, sin deps nuevas)

- **El organigrama es una vista `Render`** (`impl Render for Organigrama`) que **observa** las entidades
  `Proyecto`/`Cargo`/`Agente` + el estado vivo (`instancias`, `alertas`) — `cx.observe` + `cx.notify`
  redibuja al cambiar ([[00-fundamentos-gpui]] §4).
- **Nodos = `div()` + flexbox** con los helpers Ethos de `tema.rs` (`superficie_card`, `chip_estado`,
  `fila_seleccionable`, `eyebrow`, `titulo`). Nada custom para el nodo.
- **Aristas reporta-a:** columnas flexbox anidadas (un contenedor por subárbol) + un `div` conector de
  1.5px BRASA/LINEA entre padre e hijos (borde/altura fija). Es el enfoque "árbol vertical" que evita
  cálculo de coordenadas.
- **Aristas habla-con:** en v1, se dibujan como `div`s punteados posicionados con flexbox entre pares del
  mismo nivel; si se vuelve complejo, se difiere al modo matriz (§7.3). En v2 (grafo libre) sería un
  `Element` custom con paint de líneas.
- **Estado vivo:** el color del punto sale de una función `color_estado(agente, instancias, alertas)`
  reusando la paleta de la TUI. Cruce por el id `rol@proyecto` contra `/listar`.
- **Foco/teclado:** anillo BRASA con el sistema de foco de GPUI (acciones + keybindings,
  [[00-fundamentos-gpui]] §3), paridad peers-18.
- **Degradación:** broker offline → el grafo pinta la **estructura** (del store organizativo / config) con
  todos los nodos en gris + banner "estado en vivo no disponible", sin crash. La estructura no depende del
  estado vivo.

---

## 9. Criterios de aceptación

- **AC1** — El organigrama dibuja el árbol `reporta-a` con el Operador como raíz y cada cargo en su nivel;
  el layout no se rompe con 1, 4 o 15 nodos.
- **AC2** — Cada nodo-agente muestra su estado vivo por **color + glifo** (`● ◦ ◐ ✕`), cruzando `/listar` +
  `/admin/alertas`; cambiar el estado de un peer se refleja al refrescar.
- **AC3** — Un nodo-cargo contratado-sin-lanzar se distingue (borde punteado, gris, botón "Lanzar"); al
  lanzar, pasa a nodo-agente vivo (máquina de estados doc 03).
- **AC4** — El toggle "Ver comunicación" superpone/oculta las aristas `habla-con` derivadas de la política;
  un bloqueo se ve como arista tachada ROJO-ap.
- **AC5** — Click en un agente abre su ficha (drawer); click en un cargo abre el editor; click en ⚠ abre
  alertas; teclado mueve el foco con anillo BRASA.
- **AC6 (degradación)** — Broker caído → estructura en gris + banner, sin crash, sin `.unwrap()`.
- **AC7 (accesibilidad)** — El estado nunca depende solo del color (glifo + tooltip textual); foco visible;
  navegable por teclado.

---

## 10. Constraints

- Ethos exacto (§1). Sin `.unwrap()`/`.expect()` en prod. Cero deps nuevas (div+flexbox+helpers `tema.rs`).
  Red en `background_executor` + `bloquear_en`. El estado vivo se **lee** del broker (verdad), no se
  inventa. Reusa `Proyecto`/`Cargo`/`Agente` de `peers-core` ([[02-modelo-dominio]]) y la paleta de estados
  de la TUI (coherencia). Español salvo protocolo. NUNCA `Co-Authored-By`. Jornada en el commit.

## 11. Fuera de alcance (v1)

- Grafo libre con layout automático (v2). Drag & drop de nodos para reorganizar la jerarquía (v2; en v1 la
  cadena se edita en el cargo). Animaciones de transición de estado (nice-to-have). Zoom/pan del lienzo
  (v2, útil solo en grafo grande).

---
#empresa #organigrama #ethos #diseño-visual #gpui #ui
