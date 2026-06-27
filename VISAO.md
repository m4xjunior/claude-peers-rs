# VISÃO — Sistema Pessoal de Agentes LexusFX (a missão do time)

> Esta é a VISÃO COMPLETA do que estamos construindo, direto do Max. TODO peer
> deve ler e entender o PORQUÊ antes da sua fatia. Não somos despachados em
> tarefas soltas — somos um TIME construindo a própria infra que nos torna melhores.

## O QUE ESTAMOS CONSTRUINDO (o destino)

O **sistema pessoal de agentes do Max** — a ferramenta que ELE vai usar **TODO DIA
pra desenvolver**. Não é um projeto a mais: é a INFRA que potencializa todo o resto.

O sonho concreto do Max, nas palavras dele:
> "quero ter minha equipe em QUALQUER computador, apenas iniciando o claude e
> saindo de perto."

Ou seja: o Max chega em qualquer máquina, inicia o `claude`, e **a equipe inteira
está lá** — nós, os peers, prontos pra trabalhar, com autonomia. Ele não precisa
configurar nada, não precisa microgerenciar. Inicia e sai de perto. A equipe roda.

## OS PRINCÍPIOS (as constraints que dão forma)

1. **ZERO dependências externas** — exceto conexões públicas, que se resolvem com
   TÚNEL. Por isso é RUST, BINÁRIO: o Max baixa 1 binário em qualquer PC e funciona.
   Nada de bun/node/instalações frágeis (foi o que travou no ai-studio). Auto-contido.
2. **Autonomia ("sair de perto")** — os agentes pegam tarefas de uma FILA, trabalham,
   reportam, batem ponto — sem o Max no meio o tempo todo.
3. **A ferramenta é pra NÓS** — ela existe pra o TIME trabalhar melhor e com mais
   eficiência. Estamos construindo a nossa própria casa.

## POR QUE CADA PEÇA EXISTE (o sentido)

- **Broker em Redis (não perder solicitação):** com tantas iterações entre nós, NENHUMA
  solicitação pode se perder. Redis + outbox com ACK = se um peer reinicia no meio de
  uma tarefa, ela SOBREVIVE e ele retoma. (O bug que o Max sentiu: "não vi as mensagens
  chegarem" — isso morre aqui.)
- **Jornada medida pelo broker (matar o "tempo inventado"):** o defeito que mais
  irrita o Max é a IA INVENTAR tempos ("levou X horas"). O BROKER carimba início/fim/
  duração com o relógio DELE — a IA nunca mais estima, o número é MEDIDO. Como funcionários
  de verdade que batem ponto real.
- **GitHub Issues (empresa de verdade):** cada tarefa vira uma issue; cada report, um
  comentário; fechar a tarefa fecha a issue. O Max acompanha TODO o trabalho do time no
  GitHub, rastreável — como uma empresa com funcionários.
- **Push MCP claude/channel (equipe viva):** é o que faz a mensagem APARECER na sessão
  do peer (o <channel>), sem ele ter que checar. É o que faz a equipe parecer VIVA.
- **ID estável + binário universal:** reiniciar um peer mantém a identidade e a fila;
  o mesmo binário roda em qualquer Linux/Mac. É o que torna "a equipe em qualquer PC" real.

## COMO TRABALHAMOS (somos um time, não silos)

Todos colaboram na MESMA pasta (/tmp/claude-peers-rs/ no ai-studio), cada um na sua
FATIA (arquivo/módulo próprio) pra não se solapar — coordenados pelo próprio
claude-peers (a ferramenta que nos une). Jefin coordena a implementação Rust; Claudio
é o arquiteto/revisor; cada peer contribui no que é forte. A partição está em
COORDENACAO.md. Mas a VISÃO vem primeiro: entenda o destino, depois pegue sua parte.
