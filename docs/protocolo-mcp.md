# Protocolo MCP — como o `claude` carrega a equipe num PC novo

> Fatia da Claudia (lado cliente MCP). Fonte de verdade do "iniciar o claude e a
> equipe aparecer". Cruzado com **4 fontes verificadas, ZERO memória**:
> 1. **Spec oficial MCP** (`modelcontextprotocol`, via context7) — handshake, lifecycle, stdio.
> 2. **Ambiente real** — o `.mcp.json` (`~/.claude.json`) deste Mac.
> 3. **Prova viva** — um `<channel>` que chega de verdade na minha sessão (§3.3).
> 4. **Código-fonte do server atual** — `~/claude-peers-mcp/server.ts` (o `bun` que o broker
>    Rust vai substituir): os literais do push/capability/tools saíram DAQUI, com nº de linha.
>
> Onde digo "spec/observado/prova viva/server.ts" → é a fonte. Nada deduzido sem marcar.

---

## 0. O problema que este doc resolve

A VISÃO diz: "iniciar o claude em qualquer PC e ter a equipe". Para isso o `claude`
(o host MCP) precisa, no boot, **descobrir e subir o servidor de peers**, fazer o
**handshake**, e a partir daí **receber pushes** que viram o `<channel>` na sessão.
Este doc documenta as 3 peças do lado cliente:

1. **Bootstrap** — onde o `claude` acha o servidor e como o sobe (`.mcp.json`).
2. **Handshake** — o aperto de mão JSON-RPC que negocia versão + capabilities.
3. **Push `claude/channel`** — a notification servidor→cliente que faz a equipe "aparecer".

O broker Rust da missão substitui o servidor atual (`bun server.ts`) por um **binário**
— mas o CONTRATO abaixo (handshake + push) tem que ser idêntico, senão o `claude` não
reconhece o servidor. Este doc é esse contrato.

---

## 1. Bootstrap — como o `claude` descobre e sobe o MCP

### 1.1 Onde mora a config (observado)

O `claude` lê os servidores MCP de arquivos de config JSON. No ambiente real deste Mac,
o `claude-peers` está registrado em `~/.claude.json`, na chave `mcpServers`:

```json
"mcpServers": {
  "claude-peers": {
    "type": "stdio",
    "command": "bun",
    "args": ["/Users/maxmeireles/claude-peers-mcp/server.ts"],
    "env": {}
  }
}
```

Campos (observados em TODOS os servers do meu `.claude.json` — context7, github, n8n, peers):
- **`type`**: `"stdio"` — transporte por stdin/stdout (ver §4). É o tipo que o broker Rust usa.
- **`command`**: o executável que o `claude` SPAWNA. Hoje é `"bun"` (a dep frágil que a
  missão mata). Com o binário Rust vira `"command": "/caminho/peers-client"` (ou só
  `"peers-client"` se estiver no PATH) — **é exatamente esta linha que torna "1 binário
  em qualquer PC" real**: sem `bun`/`node`, sem `server.ts`.
- **`args`**: array de argumentos passados ao `command`. Hoje aponta pro `server.ts`;
  com o binário, pode ser vazio `[]` ou flags (ex.: `["--broker", "wss://..."]`).
- **`env`**: variáveis de ambiente injetadas no processo do server (ex.: token, broker URL).

### 1.2 Escopos de config (spec + prática do claude)

O `claude` resolve MCP de mais de um lugar, com precedência. Os escopos relevantes:
- **Projeto** (`./.mcp.json` na raiz do repo) — versionável, compartilhado pelo time do repo.
- **User/global** (`~/.claude.json` → `mcpServers`) — vale pra TODAS as sessões do usuário.
  É onde o `claude-peers` está hoje (observado) → por isso a equipe aparece em qualquer
  diretório que o Max abrir o `claude`.

> **Implicação pra missão:** pra "a equipe em QUALQUER computador", o registro do
> `peers-client` deve ir no escopo **user** (`~/.claude.json`). Assim, instalado o binário
> + 1 entrada em `~/.claude.json`, o Max inicia o `claude` em qualquer pasta e a equipe sobe.
> O instalador do binário pode escrever essa entrada automaticamente (idempotente).

### 1.3 Ciclo de vida do processo

1. `claude` inicia → lê `mcpServers` → para cada server, **spawna** `command + args` com `env`.
2. O processo do server fica vivo enquanto a sessão do `claude` viver (stdio aberto).
3. `claude` fala com o server por **stdin** (envia) e lê **stdout** (recebe). `stderr` é log.
4. Ao fechar a sessão, o `claude` fecha o stdin → o server deve encerrar limpo.

> **Regra de ouro do stdio (spec):** o server **NUNCA** escreve nada que não seja JSON-RPC
> em **stdout** — um `print`/log perdido em stdout corrompe o stream e o `claude` desconecta
> o MCP. Logs vão em **stderr**. (No broker Rust: `tracing` → stderr, JSON-RPC → stdout.)

---

## 2. Handshake — o aperto de mão (spec oficial, verificado)

JSON-RPC 2.0. Sequência obrigatória ANTES de qualquer uso:

### 2.1 Cliente → Servidor: `initialize` (request, tem `id`)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2025-06-18",
    "capabilities": { },
    "clientInfo": { "name": "claude-code", "version": "x.y.z" }
  }
}
```

- **`protocolVersion`**: a versão que o CLIENTE suporta. O server responde com a que ELE
  suporta; se incompatível, o cliente desconecta. (Versões atuais da spec: `2025-06-18`,
  `2025-11-25`.) O broker Rust deve **ecoar uma versão que o `claude` aceite** — não
  inventar; responder a mesma que o cliente mandou se a suportar.
- **`capabilities`**: o que o CLIENTE oferece (roots, sampling, elicitation…). Pro push
  servidor→cliente, o que importa é o SERVER declarar a capability (§3).
- **`clientInfo`**: nome/versão do host (o `claude`). Informativo.

### 2.2 Servidor → Cliente: resposta do `initialize` (mesmo `id`)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2025-06-18",
    "capabilities": {
      "tools": { "listChanged": true },
      "experimental": { "claude/channel": {} }
    },
    "serverInfo": { "name": "claude-peers", "version": "x.y.z" }
  }
}
```

- **`result.protocolVersion`**: versão acordada (o broker devolve a sua).
- **`result.capabilities`**: o que o SERVER oferece. Aqui o broker declara:
  - `tools` (as ferramentas: `send_message`, `set_summary`, `list_peers`, `check_messages`).
  - **`experimental.claude/channel`** — ver §3. **É O QUE HABILITA O PUSH.** Sem isto, o
    `claude` não espera notifications de canal e a equipe não "aparece" sozinha.
- **`serverInfo`**: nome/versão do broker (aparece pro `claude` como o nome do MCP).

### 2.3 Cliente → Servidor: `notifications/initialized` (notification, SEM `id`)

```json
{ "jsonrpc": "2.0", "method": "notifications/initialized" }
```

Sinaliza "handshake completo, pode operar". Depois disto o `claude` lista tools
(`tools/list`) e o canal está vivo: o server pode começar a empurrar pushes.

---

## 3. O push `claude/channel` — o que faz a equipe "aparecer"

Esta é a peça central da VISÃO ("equipe viva"). É uma **notification servidor→cliente**
(o servidor INICIA a mensagem; não é resposta a request do cliente).

### 3.1 Declarar a capability (no `initialize` result) — VERIFICADO no server.ts

O server DEVE declarar, em `result.capabilities`, a capability experimental que diz ao
`claude` "eu vou te mandar pushes de canal". **Literal exato extraído do `server.ts`
atual (`server.ts:148`)**:

```js
// server.ts:144-150 (server bun atual)
const mcp = new Server(
  { name: "claude-peers", version: "0.1.0" },        // ← serverInfo (define o source do <channel>)
  { capabilities: {
      experimental: { "claude/channel": {} },         // ← server.ts:148 — HABILITA o push
      tools: {},                                       // ← server.ts:149
  }, instructions: "…" }
);
```

→ Não é dedução: a chave é **`experimental: { "claude/channel": {} }`**, e o `name`
do serverInfo é **`"claude-peers"`** (é o que vira o `source="claude-peers"` do `<channel>`).
O broker Rust replica 1:1 — mesma chave, mesmo name.

### 3.2 O formato da notification (servidor → cliente) — VERIFICADO no server.ts

JSON-RPC 2.0 notification (sem `id`, não espera resposta — spec §4). **Literal exato do
`server.ts:429-441`** (o código que produz o push hoje):

```js
// server.ts:430-441 — "this is what makes it immediate"
await mcp.notification({
  method: "notifications/claude/channel",     // ← literal, server.ts:431
  params: {
    content: msg.text,                        // ← o corpo do <channel>
    meta: {
      from_id:      msg.from_id,              // ← chaves EM INGLÊS, server.ts:435-438
      from_summary: fromSummary,
      from_cwd:     fromCwd,
      sent_at:      msg.sent_at,
    },
  },
});
```

- **`method`**: **`"notifications/claude/channel"`** — confirmado literal (server.ts:431),
  não deduzido. O harness do `claude` reconhece esse method e renderiza o `<channel>`.
- **`params.content`**: o corpo da mensagem (`msg.text`). Vira o conteúdo dentro do `<channel>`.
- **`params.meta`**: os 4 campos de identidade. **CRÍTICO e CONFIRMADO: as CHAVES são em
  INGLÊS** — `from_id`, `from_summary`, `from_cwd`, `sent_at` (server.ts:435-438). Se vierem
  em ES/PT, o harness do `claude` NÃO parseia e o `<channel>` quebra. O broker Rust DEVE
  emitir as chaves exatamente assim. (O `fromSummary`/`fromCwd` o server resolve buscando o
  peer remetente na lista — server.ts:420-423 — então o broker Rust precisa do mesmo lookup.)

### 3.3 PROVA VIVA (observação direta, não memória)

Um push real que chegou NA MINHA SESSÃO durante esta missão renderizou como:

```
<channel source="claude-peers"
         from_id="uqqiif82"
         from_summary="CONCLUIDO Fase 2: CambioGateway…"
         from_cwd="/Users/maxmeireles/EmissorNFE"
         sent_at="2026-06-27T01:51:41.946Z">
  Boa, Claudia — captaste exatamente o sentido…  ← isto é o params.content
</channel>
```

Mapeamento confirmado (notification JSON-RPC → atributos do `<channel>`):
| `<channel>` atributo | vem de | exemplo observado |
|---|---|---|
| `source` | fixo do MCP | `"claude-peers"` (nome do server no `.mcp.json`) |
| `from_id` | `params.meta.from_id` | `"uqqiif82"` |
| `from_summary` | `params.meta.from_summary` | summary do peer |
| `from_cwd` | `params.meta.from_cwd` | `"/Users/maxmeireles/EmissorNFE"` |
| `sent_at` | `params.meta.sent_at` | ISO-8601 UTC `"…Z"` |
| (corpo) | `params.content` | o texto |

→ Os 4 do `meta` em INGLÊS estão CONFIRMADOS pela prova viva. `sent_at` é ISO-8601 UTC.
`source` = o nome da chave do server em `mcpServers` (`"claude-peers"`) — então o broker
Rust deve ser registrado com esse mesmo nome pra o `source` bater.

### 3.4 Como o broker entrega o push (lado server, pro Jefin integrar)

No stdio, o server escreve a notification (linha JSON) em **stdout** a qualquer momento
após o `initialized` — não precisa de request do cliente. É isto que difere de uma tool:
a tool RESPONDE; o push é EMPURRADO. O broker Rust:
1. Recebe a mensagem de outro peer (via Redis/fila — fatia do Jefin).
2. Monta a notification `notifications/claude/channel` com `content` + `meta` (4 chaves EN).
3. Escreve a linha JSON em stdout do processo MCP daquela sessão.
→ O `claude` lê, reconhece o method, renderiza o `<channel>`. Equipe "viva".

---

## 4. Transport stdio (spec)

- **Framing:** uma mensagem JSON-RPC **por linha** (delimitada por `\n`), UTF-8, em stdout.
  Sem cabeçalhos Content-Length (isso é do transport HTTP/SSE; stdio é newline-delimited).
- **Direções:** cliente→server por **stdin**; server→cliente por **stdout**.
- **stderr = logs**, livre. **stdout = SÓ JSON-RPC** (ver regra de ouro §1.3).
- **Notifications** (push e `initialized`) = JSON-RPC SEM `id`, sem resposta esperada (spec).
- **Requests** (initialize, tools/call) = COM `id`; a resposta ecoa o mesmo `id`.

---

## 5. Checklist pro broker Rust (o que ele DEVE cumprir pra "a equipe aparecer")

- [ ] Registrável em `~/.claude.json` (escopo user) com `type:"stdio"`, `command` = caminho
      do binário, `source`/nome = `"claude-peers"` (pro `source` do `<channel>` bater).
- [ ] Responder `initialize` ecoando uma `protocolVersion` que o `claude` aceite.
- [ ] Declarar `experimental.claude/channel` (nome EXATO extraído do `server.ts` atual) +
      as tools (`send_message`, `set_summary`, `list_peers`, `check_messages`).
- [ ] Aceitar `notifications/initialized`.
- [ ] Empurrar push como notification (sem `id`): method `notifications/claude/channel`,
      `params.content` + `params.meta` com as 4 chaves **em inglês** + `sent_at` ISO-8601 UTC.
- [ ] stdout = só JSON-RPC newline-delimited; logs (`tracing`) só em stderr.
- [ ] Encerrar limpo quando o `claude` fechar stdin.
- [ ] Verificar a flag de carga do canal: o `server.ts:10` documenta
      `claude --dangerously-load-development-channels server:claude-peers`. O push de canal
      é um recurso EXPERIMENTAL do `claude` — confirmar se ele exige essa flag pra entregar
      o `<channel>`, ou se o registro em `~/.claude.json` basta. Isto é o teste final da
      VISÃO ("iniciar o claude num PC limpo e a equipe aparecer") — se precisar da flag, o
      instalador/wrapper tem que pô-la, senão a equipe NÃO aparece sozinha.

## 6. O que ficou RESOLVIDO vs a confirmar (honestidade total)

**RESOLVIDO (verificado no `server.ts` atual, com nº de linha — não é mais dedução):**
1. ✅ Method do push = `"notifications/claude/channel"` (server.ts:431).
2. ✅ Capability = `experimental: { "claude/channel": {} }` (server.ts:148).
3. ✅ serverInfo name = `"claude-peers"` (server.ts:145) → é o `source` do `<channel>`.
4. ✅ params do push = `{content, meta:{from_id,from_summary,from_cwd,sent_at}}` chaves EN
   (server.ts:432-441).
5. ✅ As 4 tools = `list_peers`, `send_message`, `set_summary`, `check_messages`
   (server.ts:158-161, def em 169-231). Schemas de args estão no array `TOOLS` (server.ts:169).

**A CONFIRMAR (depende do runtime, não do `server.ts`):**
1. **`protocolVersion` que o `claude` exige**: o broker deve ecoar a versão que o cliente
   manda no `initialize` se a suportar (a spec lista `2025-06-18` / `2025-11-25`). Testar
   com a versão real que o `claude` instalado envia — não cravar um literal sem testar.
2. **Flag de boot — CONFIRMADO (Claudio validou em runtime, 2026-06-27):** o canal só é
   entregue se o `claude` subir com `--dangerously-load-development-channels server:claude-peers`
   (visto nos terminais reais do Max; documentado em `server.ts:10`). NÃO é hipótese: é
   OBRIGATÓRIA. Sem ela, o broker Rust pode estar 100% correto e a equipe AINDA ASSIM não
   aparece. → O instalador/wrapper do binário TEM que iniciar o `claude` com essa flag (ou o
   equivalente estável quando o recurso sair de experimental). É a condição que torna a VISÃO real.

> Tudo em §1–§5 = verificado (spec oficial + ambiente real + prova viva + server.ts com
> linha). Só o protocolVersion e a flag de boot ficam pra teste em runtime — marcados pra
> NÃO virar "tempo/dado inventado".
