# COORDENAÇÃO — claude-peers-rs (time inteiro, sem se solapar)

> Ordem do Max: TODOS colaboram com o Jefin na MESMA pasta (/tmp/claude-peers-rs/
> no ai-studio), SEM se sobrepor. Cada peer tem uma FATIA própria (arquivo/doc
> diferente) — ninguém edita o arquivo do outro. O claude-peers (a própria
> ferramenta) é o canal de coordenação. Coordenador/arquiteto: Claudio.
> A ferramenta serve pra NÓS trabalharmos melhor — é a nossa infra de equipe.

## ACESSO (verificado)
- Peers que acessam o servidor por SSH = os CLIs REAIS no Mac do Max (Claudia,
  Front-QA, Claudio). Escrevem em /tmp/claude-peers-rs/ via `ssh servidor`.
- Peers que JÁ estão no ai-studio (Jefin, Aluísios, fsgegse0) = trabalham local lá.
- Todos convergem na MESMA pasta /tmp/claude-peers-rs/.

## REGRA ANTI-COLISÃO (sagrada)
1. Cada peer edita SÓ os arquivos da SUA fatia (lista abaixo). NUNCA o do outro.
2. Antes de criar/editar, anuncia no claude-peers: "pegando <arquivo>".
3. Implementadores: cada um num MÓDULO .rs separado → o Jefin faz o `mod` e integra.
4. Pesquisa/investigação: cada um num DOC separado em /tmp/claude-peers-rs/docs/.
5. Cargo.toml / main.rs (pontos de integração) = SÓ o Jefin mexe (evita conflito).

## FATIAS (quem faz o quê)

### IMPLEMENTAÇÃO (Rust, no ai-studio)
- **Jefin** (cgrx7bdk) — COORDENA + core: store.rs (Redis), integração, Cargo.toml,
  main.rs dos bins, `mod`s. Dono do repo. Integra o que os outros entregam.
- **Aluísio Back** (jopwh40d, rustáceo) — `crates/peers-broker/src/github.rs`
  (client de GitHub Issues: octocrab/reqwest; tarefa→issue, report→comentário,
  cerrar→fecha; degrada se GH fora). MÓDULO ISOLADO.
- **fsgegse0** (se Rust) OU Aluísio Back se sobra — `crates/peers-broker/src/jornada.rs`
  (sesion/tarea, broker carimba inicio/fin/duracion). MÓDULO ISOLADO.

### PESQUISA (docs, qualquer peer com acesso)
- **Front-QA** (djnewbwj, Mac) — `docs/stack-decisions.md`: crates corretos e por quê
  (Redis: deadpool-redis vs fred vs redis; GitHub: octocrab vs reqwest; MCP Rust: há
  crate bom ou implementar stdio JSON-RPC à mão?). Decisão fundamentada (context7/docs).
- **Aluísio Front** (5f1y1reo, ai-studio) — `docs/distribucion.md`: binário SEM deps
  externas — cross-compile musl (x86_64-unknown-linux-musl), static link, SQLite/Redis
  embutido vs remoto, e o TÚNEL (única dep externa aceita: como expor o broker por
  cloudflared pra "equipe em qualquer computador").

### INVESTIGAÇÃO (docs)
- **Claudia** (hsael830, Mac) — `docs/protocolo-mcp.md`: como o `claude` carrega/
  bootstrap um MCP (.mcp.json: command/args), o handshake (initialize/capabilities),
  e o contrato EXATO do push claude/channel (já temos: meta com from_id/from_summary/
  from_cwd/sent_at em INGLÊS — confirmar/detalhar). É o que faz "iniciar o claude e ter
  a equipe" funcionar num PC novo.

### ARQUITETO / REVISÃO
- **Claudio** (eu) — este doc, a partição, blueprint (/tmp/claude-peers-rs-fase2-empresa.md),
  e REVISÃO ADVERSARIAL do conjunto antes de publicar no repo privado do Max.

## VISÃO FINAL (o que estamos construindo)
O sistema pessoal LexusFX do Max: 2 binários Rust (broker + client) que, instalados,
dão a ELE a EQUIPE INTEIRA em QUALQUER computador — só inicia o claude e sai de perto
(autonomia: agentes pegam tarefas da fila Redis, trabalham, reportam em GitHub Issues,
batem ponto medido pelo broker). ZERO deps externas exceto rede (via túnel). É a infra
que nos faz trabalhar melhor e com mais eficiência.

## DECISÃO DE STORAGE (Claudio, líder — 2026-06-27, Max delegou ao time)
Redis vs SQLite RESOLVIDO: REDIS é o DEFAULT (Max aceita a dep de rede) + SQLite
atrás de FEATURE FLAG (--features sqlite, OFF por padrão). Via trait `Almacen`:
AlmacenRedis (default) + AlmacenSqlite (feature). O broker fala com o trait,
agnóstico. Honra o zero-deps compilável (catch do Front-QA) E a escolha Redis do Max.

## STACK FECHADA (do docs/stack-decisions.md do Front-QA)
- MCP: crate `rmcp` (SDK OFICIAL Anthropic, stdio nativo, tools por macro). NÃO à mão. Fixar versão (pre-1.0), isolar em mcp.rs.
- GitHub: `octocrab` default-features=false features=["rustls"]. github.rs DEGRADA se GH cair (espelho observacional, não fonte de verdade — fonte é o broker).
- Redis (se/quando): `fred` (pool+reconnect+cluster), NÃO deadpool-redis solto. Namespace "cprs:" no redis do ai-studio, NUNCA ethos-redis.

## SEQUÊNCIA (aprovada pelo arquiteto) — contrato ANTES de repartir
1) Jefin: peers-core (tipos ES Sesion/Tarea/ItemOutbox) + trait Almacen + esqueleto jornada.rs/github.rs com FIRMAS + todo!(). FIXA O CONTRATO. Claudio revisa o contrato.
2) Só então reparte: Aluísio Back→github.rs (firmas estáveis); jornada.rs→Jefin (toca store); fsgegse0 só se rustáceo.
3) Jefin integra (Cargo.toml/main.rs/mods) + cargo test+release. Claudio revisa adversarial o conjunto.
