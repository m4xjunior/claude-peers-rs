# Mudança — GitHub Issues no repo DINÂMICO do peer (decisão do Max)

> Decisão do Max (2026-06-27): as issues NÃO vão pra um repo fixo (GITHUB_REPO env).
> Vão pro REPO ONDE O CLAUDE ESTÁ TRABALHANDO. "Se o diretório tiver git e repo
> criado, as issues devem ir pra lá." Cada peer registra suas tarefas no repo do
> projeto que está tocando naquele momento — contextual e automático.
> Substitui a regra antiga "repo de prueba fixo" (essa era só pra TESTE).

## Arquitetura (simplificada — esclarecimento do Max 2026-06-27)
Não há "problema" de o broker estar noutra máquina: o Max esclareceu que (a) os
diretórios de trabalho são os MESMOS/espelhados em cada máquina, e (b) o GitHub
está LOGADO em todas as máquinas (gh auth presente no servidor também). Logo:
- O **peers-client** roda SEMPRE na máquina do peer, DENTRO do diretório git de
  trabalho, com gh/git autenticado. Ele resolve o `owner/repo` LOCALMENTE (trivial)
  e manda pro broker. O broker só usa o valor recebido.
- Como o gh está logado em toda máquina, o TOKEN de criar issue também pode ser
  resolvido por máquina (gh auth token) — não precisa de um GITHUB_TOKEN central
  único; cada client/broker usa o gh da sua máquina. (Decidir no impl: token via
  gh auth token da máquina OU GITHUB_TOKEN env — ambos viáveis já que gh está logado.)

## Como o client resolve o repo (no boot / por tarea)
1. O client já descobre git_root (faz hoje pro registro). ADICIONAR: resolver o
   `owner/repo` do GitHub a partir do remote origin do git_root:
   `git -C <git_root> remote get-url origin` → parse:
   - `git@github.com:owner/repo.git` → owner/repo
   - `https://github.com/owner/repo(.git)` → owner/repo
2. Manda o `repo_github: Option<String>` ("owner/repo") no /registrar (campo novo
   na Instancia) OU na /tarea/abrir. Se o dir não tem git/remote GitHub → None.

## Mudança no broker
- A `Instancia` ganha `repo_github: Option<String>`.
- Ao abrir uma tarea (tarea_abrir), o broker usa o `repo_github` da instancia dona:
  - Se Some E há GITHUB_TOKEN → cria a issue NESSE repo (owner/repo do peer).
  - Se None (dir sem git/remote) OU sem token → degrada (sem issue, opera local).
- O github.rs deixa de depender de GITHUB_REPO fixo: o `crear_issue` recebe
  owner+repo como PARÂMETRO (vindos da instancia), não do env. GITHUB_TOKEN
  continua do env (a credencial é por-máquina-do-broker, global).

## Ajuste no github.rs
- `GitHub::desde_entorno()` → só lê GITHUB_TOKEN (a credencial). NÃO lê mais GITHUB_REPO.
- Os métodos `crear_issue/comentar_issue/cerrar_issue` recebem `owner: &str, repo: &str`
  como params (o repo alvo vem da tarea/instancia, dinâmico).
- Degradação intacta: sem token → None; instancia sem repo_github → broker não cria issue.

## Regra de segurança (mantida)
- Durante TESTE: o client pode rodar num dir cujo git_root é um repo de prueba
  descartável → as issues vão pra lá (seguro). NÃO é hardcoded; é o dir de teste.
- Em USO REAL: o peer trabalha no repo real → issues vão pro real. É o que o Max quer.
- O token GITHUB_TOKEN precisa de permissão de issues nos repos onde os peers atuam.

## Critério de pronto
1. Client resolve owner/repo do remote origin do git_root (git@ e https://). None se não-GitHub.
2. Instancia carrega repo_github: Option<String>; broker usa o da instancia dona da tarea.
3. github.rs sem GITHUB_REPO fixo; crear_issue recebe owner+repo por param.
4. Degradação: sem token / sem repo_github → opera local, sem issue (warn!).
5. Teste: peer num dir com git de teste → issue criada no repo de teste; peer num dir sem git → sem issue, broker OK.
