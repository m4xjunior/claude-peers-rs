# 🏛️ LexusFX · Empresa — Arquitectura de negocio (agentes de IA como empleados)

> Vault de la **arquitectura corporativa**: el modelo de empresa donde los peers son empleados, el app
> `peers-desktop` es el despacho del dueño, y el broker es RRHH + reloj de fichaje + oficina de correos.
> Traduce la VISÃO ("mi equipo en cualquier computador, inicio claude y salgo de perto") en una
> **corporación con lógica real de empresa**: proyectos, cargos, agentes, cadena de mando, delegación.
>
> Abrí esta carpeta (`.specs/empresa`) como vault en Obsidian; el grafo conecta con el vault
> `.specs/desktop` (las 9 pestañas + Lanzador + Política) porque la empresa **reusa** ese backlog.

---

## 📌 Empezar por aquí (orden de lectura)

1. [[01-modelo-corporativo]] — **la lógica de empresa** que Max no había estructurado: los 5 niveles
   (empresa→proyecto→equipo→cargo→agente), qué es un agente, el ciclo de operar la empresa, las decisiones
   abiertas con recomendación. **Este es el documento central.**
2. [[00-fundamentos-gpui]] — los **fundamentos de GPUI** (App/Entity/Context, render, async, la regla
   anti-SIGABRT) y por qué el modelo de negocio debe encajar en cómo GPUI posee el estado. La base técnica.
3. [[02-modelo-dominio]] — los **tipos** (`Cargo`, `Agente`, `Proyecto`, `Ubicacion`), el wire, la
   persistencia, y cómo se mapean a lo que YA existe en `peers-core`. El contrato de datos.

## 🔧 Ingeniería profunda (03–08) — "los varios pasos que el código tiene que tener"

> Segunda iteración (2026-07-03): tras la crítica de Max *("eso no fue una arquitectura; profundiza")*,
> estos 6 documentos bajan del concepto a la **ingeniería real**, verificados contra el código
> (`archivo:línea`). Decisiones de producto de Max que los fundan: conocimiento **rico** en el boot ·
> fuente **híbrida** (broker + repo) · **regla dura** (el broker valida) · entrega completa.

| Doc | Qué ingenieriza | Crítica de Max que resuelve |
|-----|-----------------|-----------------------------|
| [[03-pipeline-provision]] | El pipeline "de cero a agente operativo" como **máquina de estados** (precondición·acción·traza·fallo·idempotencia), cerrando los 9 huecos H1-H9 | *"hasta llegar al agente son varios pasos que el código tiene que tener"* |
| [[04-conocimiento-agente]] | La **base de conocimiento del agente** (2 canales A/B, modelo híbrido) + la **skill** para operar la empresa (tools MCP nuevas) | *"tienen que tener la base y conocimiento del negocio, de todos los proyectos… skill para operar el sistema"* |
| [[05-organigrama-visual-ethos]] | El **organigrama en detalle visual Ethos**: nodos, 3 aristas, estados vivos, interacción, impl GPUI | *"desenhar o organograma em detalhe visual Ethos"* |
| [[06-decisiones]] | **TODAS las decisiones resueltas** (ADR-es): las 4 de Max + los 9 huecos + store organizativo + anti-spoofing | *"detalhar a decisão #TODAS"* |
| [[07-workflow-trazable-164-features]] | El **workflow con cadena + trazabilidad** sobre las 164 features (7 eslabones, máquinas de estado reales) | *"ataque las 164 features con un workflow con cadena y trazabilidad"* |
| [[08-capa-ui]] | La **capa de UI** que expone las features: pantallas, wizard de contratación+lanzamiento, mapa feature→UI | *"no pensamos en las funcionalidades de UI que exponen estas features"* |

## 🧩 Las RFCs nuevas (la ingeniería de las features de empresa)

| RFC | Qué ingenieriza | Encargo de Max que resuelve |
|-----|-----------------|-----------------------------|
| [[rfc/RFC-proyectos]] | El "apartado de proyectos": workspaces aislados (carpeta local / SSH), equipo, tablero, política local, relanzar de un click | *"tengo apartado de proyectos… crear mis proyectos… controlar el servidor SSH o carpeta local"* |
| [[rfc/RFC-organigrama-roles]] | Cargos (system prompt = regla de negocio), contratar agentes (id `rol@proyecto`), organigrama vivo | *"delegar agentes que yo creo… un system prompt que es toda su regla de negocio"* |
| [[rfc/RFC-delegacion-cadena-mando]] | Delegar (hacia abajo), reportar (hacia arriba), escalar (alertas por la jerarquía) | *"cómo va a trabajar con los otros peers… corporación con lógica real de empresa"* |
| [[rfc/RFC-relectura-features-existentes]] | Reencuadre corporativo de las 164 features + RFCs grandes ya documentadas | *"hacer lo mismo para las que ya están documentadas"* |

## 🔗 Las RFCs existentes que la empresa REUSA (no se reescriben)

- [[../desktop/lanzador/RFC-lanzador]] — **motor de ejecución**: lanza el agente con su system prompt, chat
  privado (escribir sin que se vea en el TUI), terminal PTY, SSH/tmux. La empresa le aporta identidad
  (`rol@proyecto`), contenedor (proyecto) y significado (cargo).
- [[../desktop/politica-comunicacion/RFC-politica-comunicacion]] — **firewall organizativo** (eje
  habla-con); ya Fase 1. La empresa la usa para aislar proyectos/equipos (grupos).
- [[../desktop/registro-acciones/RFC-registro-acciones-jornada]] — **parte de trabajo / diario del
  empleado**; su identidad durable (`bitacora.db`) es el germen de la identidad organizativa de la empresa.
- [[../features/supervisor/spec]] — **mandos intermedios automáticos** (vigila 24/7); alimenta el escalado.
- [[../features/tareas-autogestionadas-aprendizaje/spec]] — **fichaje autónomo + productividad** (el
  empleado que trabaja solo y cuyo tiempo real mide el broker).
- Las 9 pestañas del [[../desktop/INDICE-RFCS|índice desktop]] — el backlog operativo, reencuadrado en
  [[rfc/RFC-relectura-features-existentes]].

---

## 🧠 La idea en una frase

> **La corporación NO es infraestructura nueva: es una capa de identidad organizativa (proyecto · cargo ·
> agente · cadena de mando) superpuesta a primitivas que YA existen** (peers, broker, jornada, tareas,
> política, supervisor, bitácora). El único backend imprescindible es el binding de id explícito
> (`CLAUDE_PEERS_ID = rol@proyecto`); todo lo demás es config del operador + vistas GPUI sobre endpoints
> existentes.

## 🗺️ El mapa metáfora ↔ sustrato real (resumen)

| Empresa | Sustrato real (ya existe) |
|---------|---------------------------|
| Empleado | sesión de Claude Code (`Instancia`/peer) |
| Descripción de puesto / contrato | system prompt (`--append-system-prompt`) + instrucciones MCP |
| Fichaje / horas | jornada timbrada por el broker (`Sesion`) |
| Órdenes de trabajo | tareas (`Tarea` + factor de estimación) |
| Correo interno + "tocar el hombro" | broker (`/enviar`, bandeja ZSET) + push `<channel>` |
| Quién habla con quién | política de comunicación (firewall, Fase 1) |
| Mandos intermedios | supervisor (alertas) |
| Parte de trabajo | registro de acciones (`bitacora.db`) |
| Expediente público | GitHub Issues |
| RRHH + nómina + verdad | el broker |
| Despacho del dueño | la app `peers-desktop` |
| Dueño (Max) | `ID_OPERADOR` (reservado, no-suplantable) |

## 🎨 Design System

Ethos (consistente con todo el vault desktop): TINTA `#100D0A` · TINTA2 `#1A1611` · PAPEL `#ECE5D7` ·
BRASA `#C9A96E` · HUMO `#938B7B` · LINEA `#2B271F`. Fraunces (títulos) / Inter (UI) / IBM Plex Mono (datos).
Radios card 14 · control 10 · pill 999.

## 🧭 Estado

- **Arquitectura, no código:** solo `.specs`. Las 4 decisiones de producto están **tomadas** por Max
  (conocimiento rico · fuente híbrida · regla dura · entrega completa); ver [[06-decisiones]].
- **Decisión #1 (cimiento):** provisión del id `rol@proyecto` (`CLAUDE_PEERS_ID`, el client ya lo respeta —
  `main.rs:85-88`; falta que la app lo provea). Cierra H1/H3/H7 y desbloquea el resto ([[03-pipeline-provision]]).
- **Prerequisito técnico más delicado (spike):** anti-spoofing del `de_id`↔conexión ([[06-decisiones]] E-10),
  necesario antes de la regla dura server-side.
- **Correcciones verificadas** propagadas a los docs: el MCP es JSON-RPC **a mano** (no rmcp);
  `--append-system-prompt` **no existe hoy** en el runtime (solo specs); la derivación de id saneala `@`→`-`.

#moc #empresa #arquitectura-corporativa #agentes #proyectos #peers-desktop

#moc #empresa #arquitectura-corporativa #agentes #proyectos #peers-desktop
