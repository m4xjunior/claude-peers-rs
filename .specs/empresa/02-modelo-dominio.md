# Modelo de dominio corporativo — tipos, wire y persistencia

> ⬆ [[_MOC|Mapa]] · Lógica de negocio: [[01-modelo-corporativo]] · Base técnica: [[00-fundamentos-gpui]]
>
> Fecha: 2026-07-02. Estado: **PROPUESTA DE DISEÑO — para revisar antes de codificar.**
> Traduce el modelo corporativo a **DTOs en `peers-core`**, endpoints del broker y persistencia, siguiendo
> las convenciones verificadas del código (`crates/peers-core/src/lib.rs`, `almacen.rs`; el trait
> `Almacen` con backends Redis + SQLite; ADR-001 sobre `bitacora.db` e identidad durable).
> **No define UI** (eso son las RFCs). Define el contrato de datos que sostiene la empresa.

---

## 1. Principios de diseño (heredados del código, no negociables)

Verificados en `peers-core/lib.rs` y las specs existentes:

1. **DTO único en `peers-core`.** Los tipos corporativos se definen UNA vez aquí; broker, TUI y desktop
   los consumen. La desktop NUNCA redefine DTOs ([[00-fundamentos-gpui]] §7).
2. **Español salvo protocolo.** Structs, campos, variantes en español (como `Instancia`, `Tarea`,
   `EstadoTarea`, `Politica`, `AccionRegistrada`). Excepción: las 4 claves del push del harness.
3. **Compat por `#[serde(default)]`.** Todo campo nuevo lleva `#[serde(default)]`; todo enum de estado que
   pueda crecer es `#[non_exhaustive]` (como `TipoAccion`, `TipoAlerta`). JSON viejo debe deserializar.
4. **Retención acotada.** Todo lo que crece se poda a una constante (`RETENCION_*`), patrón LIST+LTRIM en
   Redis / poda en SQLite (como `RETENCION_HISTORIAL=500`, `MAX_BLOQUEOS=100`).
5. **El tiempo lo timbra el broker.** Ningún timestamp corporativo lo pone la IA ni la UI (`inicio`/`fin`
   de jornada, `cuando` de acciones — regla sagrada).
6. **Trait `Almacen` en AMBOS backends.** Cada operación de persistencia nueva se implementa en Redis
   (default) y SQLite (feature). La identidad durable va en `bitacora.db` vía SQLx (ADR-001).

---

## 2. Los tipos nuevos (borrador de `peers-core`)

> Firmas orientativas para fijar el contrato; los detalles finales se cierran en revisión (como se hizo
> con `Politica`/`AccionRegistrada`). Se marcan `#[serde(default)]` los campos que lo requieren para compat.

### 2.1 Identidad organizativa

```rust
/// La empresa: una sola, la constante del sistema. Casi todo son valores globales.
pub struct Empresa {
    pub nombre: String,                 // "LexusFX"
    pub operador_id: String,            // = ID_OPERADOR (reservado, ya existe)
    // política por defecto, principios, etc. viven en sus propios stores (no duplicar aquí)
}

/// Plantilla de puesto reutilizable. NO es un empleado vivo: es la "descripción de cargo".
pub struct Cargo {
    pub id: String,                     // id de rol estable: "backend", "qa", "coordinador"
    pub nombre: String,                 // legible: "Backend Rust"
    pub system_prompt: String,          // la regla de negocio inyectada (--append-system-prompt)
    pub departamento: Option<String>,   // agrupación opcional (v1: etiqueta)
    pub reporta_a: Option<String>,      // id de cargo superior en la cadena de mando (§4)
    pub puede_delegar_a: Vec<String>,   // ids de cargo a los que este puede asignar/reasignar
    pub capacidades: Vec<Capacidad>,    // v1: informativas/como texto; v2: permisos duros (§6)
    #[serde(default)]
    pub notas: Option<String>,
}

/// v1 = etiquetas informativas que además se vuelcan al system prompt; v2 = permisos reales en el broker.
#[non_exhaustive]
pub enum Capacidad {
    CrearTareas, AsignarTareas, ReasignarTareas, ForzarTareas,
    LanzarSubAgentes, TocarProduccion, RevisarSolo, // …
}

/// Un empleado vivo: instancia de un Cargo en un Proyecto. Identidad durable (rol@proyecto),
/// presencia efímera (puede estar vivo o no según el peer registrado).
pub struct Agente {
    pub id: String,                     // "backend@proyecto-x" — derivado de rol+proyecto (§5, resuelve colisión)
    pub cargo_id: String,               // FK lógica a Cargo
    pub proyecto_id: String,            // FK lógica a Proyecto
    pub ubicacion: Ubicacion,           // dónde corre (heredado del proyecto, override posible)
    #[serde(default)]
    pub instancia_id: Option<String>,   // el id del peer vivo si está registrado (cruce con /listar)
    #[serde(default)]
    pub estado: EstadoAgente,           // contratado | lanzado | vivo | pausado (derivado + persistido)
}

#[non_exhaustive]
pub enum EstadoAgente { Contratado, Lanzado, Vivo, Ocioso, Pausado }
impl Default for EstadoAgente { fn default() -> Self { EstadoAgente::Contratado } }
```

### 2.2 Proyecto (el "apartado")

```rust
pub struct Proyecto {
    pub id: String,                     // slug estable: "proyecto-x"
    pub nombre: String,
    pub ubicacion: Ubicacion,           // carpeta local o host SSH por defecto para su equipo
    #[serde(default)]
    pub agentes: Vec<String>,           // ids de Agente del equipo
    #[serde(default)]
    pub creado_en: Option<String>,      // ISO, timbrado por el broker
    #[serde(default)]
    pub archivado: bool,
}

/// Dónde corre físicamente un agente. Reusa el multi-host de RFC Acceso.
pub enum Ubicacion {
    Local { carpeta: String },          // ruta elegida con el file picker nativo
    Ssh { host: String, carpeta: String }, // host de la lista configurable; carpeta remota
    Tmux { host: Option<String>, carpeta: String, sesion: String }, // destino tmux (RFC Lanzador R4.3)
}
```

### 2.3 Perfil de lanzamiento (ya previsto en RFC Lanzador R9, se formaliza)

```rust
/// Combinación reutilizable para desplegar un equipo/agente con un click.
pub struct PerfilLanzamiento {
    pub nombre: String,
    pub proyecto_id: String,
    pub agentes: Vec<AgentePlan>,       // qué cargos instanciar y dónde
    #[serde(default)]
    pub tareas_iniciales: Vec<TareaInicial>, // se materializan al lanzar (R3.1 broker / R3.2 prompt)
    #[serde(default)]
    pub flags: FlagsLanzamiento,        // skip-permissions (off), canal (siempre on), etc.
}
```

> **Nota de alcance:** `Cargo`, `Proyecto`, `PerfilLanzamiento` y las plantillas de system prompt se
> persisten en **`config.toml`** (mismo archivo que ya usan TUI y desktop, `config.rs`) —son configuración
> del operador, no estado vivo del broker. El estado **vivo** (qué agente está registrado, su jornada, sus
> tareas) viene del broker por HTTP y se cruza por `instancia_id`. Esta separación (config del operador vs
> verdad del broker) es la misma que ya respeta la app.

---

## 3. Mapa a tipos existentes (qué se reusa, qué se añade)

| Necesidad corporativa | Tipo existente que se reusa | Tipo nuevo |
|-----------------------|-----------------------------|-----------|
| Empleado vivo | `Instancia` (presencia: id, directorio, resumen, visto_en) | `Agente` (identidad durable, envuelve la Instancia por `instancia_id`) |
| Fichaje/horas | `Sesion`, `RespuestaJornada` | — (se filtra por agente/proyecto) |
| Órdenes de trabajo | `Tarea`, `EstadoTarea`, `FactorEstimacion` | — (se etiquetan por proyecto, §7) |
| Correo interno | `Mensaje`, `EstadoMensaje`, bandeja ZSET | — |
| Quién habla con quién | `Politica`, `ReglaComunicacion`, `Patron` (incl. `Grupo`) | — (los grupos modelan proyecto/equipo) |
| Supervisión | `Alerta`, `TipoAlerta`, umbrales | — (alertas filtrables por proyecto/agente) |
| Parte de trabajo | `AccionRegistrada`, `TipoAccion`, `bitacora.db` | — |
| Identidad reservada del dueño | `ID_OPERADOR`, `REMITENTES_EXENTOS`, `remitente_exento()` | — |
| Cargo / Proyecto / organigrama | — | `Cargo`, `Agente`, `Proyecto`, `Capacidad`, `Ubicacion` |

**Lectura clave:** el 80% del dominio ya existe. Lo genuinamente nuevo es la **capa de identidad
organizativa** (`Cargo`, `Agente`, `Proyecto`) que **envuelve y etiqueta** las primitivas de presencia y
trabajo. No se reescribe nada; se añade una capa de estructura encima.

---

## 4. La cadena de mando como datos

Las tres relaciones del organigrama ([[01-modelo-corporativo]] §6) se codifican así:

- **Puede-hablar-con** → NO va en `Cargo`; va en la **política de comunicación** (ya existe). El
  organigrama la lee de `GET /admin/politica` y la pinta como aristas.
- **Reporta-a / delega-en** → campos `reporta_a` y `puede_delegar_a` en `Cargo`. Son la cadena de mando.
  La app los usa para (v1) guiar el system prompt y validar en UI qué asignaciones ofrecer; (v2) el broker
  podría rechazar asignaciones que las violan (RFC Delegación §riesgos).
- **Supervisa-a** → derivado: el `reporta_a` invertido + el supervisor (broker) + el operador (todo).

> **Por qué la cadena de mando vive en `Cargo` y no en `Agente`:** es una propiedad del **puesto**, no de
> la persona. "El backend reporta al coordinador" vale para cualquier agente que ocupe ese cargo, en
> cualquier proyecto. Mantiene el modelo DRY y las plantillas reutilizables.

---

## 5. Identidad de agente: `rol@proyecto` (resuelve el binding y la colisión)

Problema abierto en 3 specs (Lanzador §6.2, política §5.3, registro-acciones §5.3) + `STATE.md`. Solución
de dominio:

- El id de un agente = **`<cargo_id>@<proyecto_id>`** (ej. `backend@proyecto-x`, `qa-2@proyecto-x` si hay
  dos QA). Legible, durable, único por proyecto.
- Al **lanzar**, la app pasa ese id explícito a la sesión vía `CLAUDE_PEERS_ID` (env/flag **que el
  `peers-client` ya respeta** — está en el README: *"`--id` o `CLAUDE_PEERS_ID` es el papel estable"*).
  Esto elimina la correlación frágil por cwd: la app **fija** la identidad, el broker la honra.
- El registro atómico ya arreglado (commit 1f4187f) sufija en colisión (-2/-3); combinado con
  `rol@proyecto`, el sufijo casi nunca se necesita, y cuando ocurre es legible.
- El `instancia_id` del `Agente` se rellena cuando ese id aparece en `/listar` → así la app cruza
  "agente contratado" ↔ "peer vivo" sin heurística.

**Impacto backend (pequeño, ya casi listo):** el `peers-client` ya acepta `CLAUDE_PEERS_ID`; solo hay que
asegurar que NO lo re-deriva de la carpeta cuando viene explícito. Es la única pieza de backend
imprescindible para el modelo corporativo. Todo lo demás (Cargo/Proyecto/Perfil) es config del operador +
UI.

---

## 6. Capacidades: v1 texto, v2 permisos duros (decisión explícita)

- **v1 (recomendado):** `Capacidad` es informativa. La app **vuelca** las capacidades y la cadena de mando
  al **system prompt** del cargo al lanzar ("Eres el coordinador; puedes asignar tareas a backend y qa;
  reportas a Max"). El cumplimiento es por comportamiento del agente + firewall de política para lo
  prohibido. Cero cambios de backend más allá del id.
- **v2 (evolución):** el broker valida capacidades reales — p.ej. rechaza `/tarea/asignar` si el `de` no
  tiene `AsignarTareas`, o expone herramientas MCP distintas por rol. Depende de qué exponga el harness y
  de endurecer el broker. NO bloquea v1.

Esto responde el *"puede ser otra cosa"* de Max: el mecanismo HOY es el system prompt (garantizado);
el endurecimiento es futuro y opcional.

---

## 7. Etiquetado por proyecto (aislamiento sin infra nueva)

Para que tareas/mensajes/alertas se filtren por proyecto sin cambiar el core:

- **v1 (recomendado): por convención de id.** Como el id de agente es `rol@proyecto`, TODO lo atribuible a
  un agente (tareas asignadas, jornada, acciones, alertas por sujeto) es filtrable por el sufijo
  `@proyecto` en el cliente. Cero cambios de esquema. La app agrupa por proyecto al pintar.
- **v2 (si Max quiere aislamiento fuerte):** un campo `proyecto_id` opcional en `Tarea`/`Mensaje`
  (`#[serde(default)]`) y filtros server-side. Más limpio, pero toca el core. Diferible.

**Recomendación:** v1 por convención de id desbloquea el "apartado de proyectos" HOY; v2 se hace si el
filtrado por string se queda corto.

---

## 8. Persistencia (dónde vive cada cosa)

| Dato | Store | Backend | Justificación |
|------|-------|---------|---------------|
| `Cargo`, `Proyecto`, `PerfilLanzamiento`, plantillas de system prompt | `config.toml` | fichero local (toml+dirs) | config del operador, no estado vivo; ya es el patrón de la app |
| Estado vivo de agentes (registrado, jornada, tareas) | broker | Redis / SQLite | verdad del broker; la app lo espeja por HTTP |
| Política de comunicación | broker | Redis `cprs:politica_comunicacion` / SQLite | ya existe (Fase 1) |
| Bitácora / registro de acciones + identidad durable (`peers_conocidos`, `tareas_conocidas`) | `bitacora.db` | SQLx (ADR-001) | rastreabilidad FK; ya decidido por Max |
| Alertas del supervisor | broker | Redis `cprs:alertas` / SQLite | ya existe |

> **Coherencia con ADR-001:** la identidad durable (que un proyecto/cargo sobreviva al peer) encaja con la
> decisión ya tomada de `bitacora.db` con `peers_conocidos`. Si el modelo corporativo necesita persistir
> agentes conocidos server-side (para trazabilidad histórica cross-reinicio), `bitacora.db` es su hogar
> natural — misma decisión, extendida. Config del operador (plantillas de cargo) NO va ahí: va a `config.toml`.

---

## 9. Endpoints nuevos (mínimos) y reusados

**Reusados tal cual** (cero backend nuevo): `/listar`, `/enviar`, `/salir`, `/jornada`, `/listar-tareas`,
`/tarea/asignar`, `/tarea/reasignar`, `/tarea/estado`, `/tarea/forzar`, `/tarea/reportes`,
`/factor-estimacion-peer`, `/admin/alertas`, `/admin/alerta-resolver`, `/admin/politica*`,
`/admin/historial`, `/acciones` (registro-acciones).

**Nuevo imprescindible (backend):** que `peers-client` respete `CLAUDE_PEERS_ID` explícito sin
re-derivarlo de la carpeta (§5). Es lo ÚNICO bloqueante.

**Nuevos opcionales (evolución, decidir con Max):**
- `POST /tarea/asignar` que valide capacidad/cadena de mando (v2, §6).
- Campo `proyecto_id` en tareas/mensajes para filtrado server-side (v2, §7).
- Persistir "agentes conocidos" en `bitacora.db` para trazabilidad histórica (v2, §8).

---

## 10. Verificación / criterios de que el dominio es correcto

- **DTOs compilan con `#[serde(default)]`** y JSON viejo (sin secciones corporativas) deserializa sin error.
- **Cero DTOs duplicados:** ningún tipo corporativo redefine `Instancia`/`Tarea`/`Mensaje`; los envuelve.
- **El id `rol@proyecto` es estable** entre reinicios del peer (mismo rol+proyecto → mismo id) y legible.
- **Todo lo persistido es acotado** (config del operador es finito; lo del broker respeta `RETENCION_*`).
- **La app funciona sin la capa corporativa** (compat): sin proyectos/cargos definidos, se comporta como
  hoy (9 pestañas + lanzador). La empresa es opt-in.

---
#empresa #modelo-dominio #peers-core #dto #persistencia #identidad
