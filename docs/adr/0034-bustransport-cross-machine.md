# ADR 0034 — `BusTransport`: a porta para o bus cross-machine (local-first preservado)

- **Status:** **Proposto (2026-06-16).** É **gate**: stories que dependem desta decisão não iniciam até este ADR ser aceito.
- **Onda/Story:** F4 entrega a porta (trait `BusTransport` + `InProcessTransport`, refactor sem comportamento novo — story F4-3-6); F5 entrega `RemoteOverSSH` (o transporte remoto real, multiplayer entre PCs · Lina no VPS). Nenhuma story de F5 ainda escrita.
- **Data:** 2026-06-16
- **Fontes:** doc-fonte de visão — túnel seguro entre Linas em PCs diferentes (linha 85) e Lina rodando no VPS (linha 55), ambos TOCAM o invariante #2 (local-first) · invariantes #2/#3/#5/#7 (`CLAUDE.md`) · âncora "Workspace Bus / Supervisor" (`CLAUDE.md` §Âncoras) · ADR 0006 (injeção default-deny por pertencimento) · ADR 0010 (multi-workspace: trust com namespace por Espaço) · ADR 0026 (identidade de terminal por env de spawn).

## Contexto

O **Workspace Bus / Supervisor** (`crates/lina-core/src/lib.rs:1300` — `pub struct Supervisor`) é
hoje, por construção, **in-process e single-machine**: o roster vive num `Mutex<HashMap<NodeId, …>>`
do processo, o pub/sub é `tokio::sync::broadcast` in-process (`lib.rs:1308`, comentário em `lib.rs:959`),
as filas seriais de escrita e os locks lógicos de PTY são estruturas de memória do mesmo processo. A
entrega A2A (`crates/lina-core/src/a2a.rs:311` — `deliver_a2a`) injeta bytes no PTY **vivo** de um nó
que está no MESMO processo. "Estar no Espaço = estar conectado" (invariante #5) é, hoje, "estar no mesmo
processo". Isso é o que torna o app local-first por default (invariante #2): **nada do bus sai da
máquina porque o bus nem sabe falar com outra máquina.**

O doc-fonte de visão pede dois cenários que rompem essa fronteira:
1. **Túnel seguro entre Linas em PCs diferentes** (linha 85) — dois operadores, duas máquinas, terminais
   de um visíveis/endereçáveis pelo bus do outro.
2. **Lina no VPS** (linha 55) — um nó (ou Espaço) que não roda na máquina do usuário.

Ambos **tocam diretamente o invariante #2**: o instante em que um `A2aEnvelope`
(`lib.rs:171`, versionado: `id/root_cause_id/from/to/intent/hops/await_reply/trace/ttl`) cruza o limite
da máquina, "nada sai por padrão" deixou de valer **por construção** — virou "sai, e o usuário precisa
saber e ter mandado sair". O risco não é implementar o remoto; é **deixá-lo barato de LIGAR sem ser caro
de AUTORIZAR**. Se o transporte remoto entrar acoplado ao `Supervisor`/`deliver_a2a` sem uma fronteira,
ele fecha portas de continuidade (invariante #7: core/shell split; #3: neutralidade) e regride a doutrina
de segurança que ADR 0006/0010 montaram em cima do roster.

A pergunta que este ADR fecha não é *como* fazer o remoto — é **onde fica a costura** para que abri-lo
seja barato de implementar e **caro/deliberado de autorizar**.

## Decisão

### 1. `BusTransport` — uma trait que abstrai POR ONDE o envelope viaja

Introduzir a trait **`BusTransport`** como fronteira entre o `Supervisor`/`Router` (a lógica de roteamento,
gate e entrega) e o **meio físico** por onde um `A2aEnvelope` chega ao nó destino. Duas implementações
PREVISTAS, **só a primeira existe**:

- **`InProcessTransport`** (atual, único implementado) — o que o `Supervisor` já faz: roster e pub/sub em
  memória, entrega via `deliver_a2a` no PTY local. Comportamento idêntico ao de hoje; nenhuma regressão.
- **`RemoteOverSSH`** (FUTURO, **NÃO implementado aqui**) — túnel seguro (SSH como base canônica:
  autenticado, criptografado, sem porta nova exposta) para os cenários PC↔PC (linha 85) e VPS (linha 55).

**Esta decisão entrega SÓ a porta — a trait e o split.** Nenhuma linha de `RemoteOverSSH` é escrita agora.
O objetivo é que, quando F5 chegar, abrir o remoto seja *implementar uma impl de trait*, não *re-arquitetar
o bus*.

### 2. Local-first é o DEFAULT por ausência, não por flag

O `Supervisor` nasce e permanece em `InProcessTransport`. Não existe transporte remoto a menos que o
usuário o crie explicitamente. **Nada sai da máquina porque não há para onde sair** — o invariante #2 é
preservado por construção, não por uma checagem que alguém pode esquecer. Um Espaço sem transporte remoto
ativo é, por definição, idêntico ao de hoje.

### 3. Cruzar máquina é opt-in sinalizado + gate humano + escopo explícito

Qualquer `BusTransport` que cruze o limite da máquina é, em camadas duras e cumulativas:

- **Opt-in sinalizado** — só existe se o humano o criar; e enquanto existir, a UI sinaliza
  visivelmente "este Espaço está exposto a / conectado a outra máquina" (espelho do "exposições são opt-in
  sinalizado" do invariante #2).
- **Gate humano** — abrir um túnel cross-machine é **ação irreversível de impacto externo** (expõe
  terminais a outra máquina): exige confirmação humana explícita, na classe do ADR 0004 (custódia de
  segredo) e da doutrina de gate de `CLAUDE.md` §6. Nenhum agente abre túnel.
- **Escopo explícito e mínimo** — **quem cria o túnel escolhe quais terminais ficam acessíveis** (default
  = nenhum). O escopo é decisão humana no momento da criação, não um campo que viaja no envelope.

### 4. A segurança continua nas DUAS camadas — o transporte não vira autoridade

`BusTransport` move **bytes**; não decide identidade, ordem nem autorização. A doutrina de admissão
permanece inteira do lado de cá da porta:

- A injeção A2A continua **default-deny por pertencimento** (ADR 0006): um envelope que chega por
  `RemoteOverSSH` só é entregue se o destino pertencer ao Espaço sob o **trust com namespace por Espaço**
  (ADR 0010) — o transporte remoto **não amplia** o conjunto de quem pode injetar em quem; ele só muda por
  onde os bytes chegam. Um nó remoto admitido entra no roster pelas mesmas regras de admissão canônica e
  identidade por env de spawn (ADR 0026), nunca por um campo do envelope.
- **Nenhum campo escrito por agente decide segurança.** `from`, `to`, `intent`, `payload`, `trace`, `ttl`
  do `A2aEnvelope` continuam sendo **dado transportado, jamais autoridade** — exatamente como hoje. Um
  `from` remoto não autentica nada; a autoridade é o canal autenticado (camada de transporte) + o trust do
  Espaço (camada de admissão), as mesmas duas camadas de sempre. A cadeia de validação do `Router`
  (`crates/lina-core/src/router.rs:9` — dedupe → anti-loop(hops) → remetente existe → alvo existe →
  autonomia → fan-out → orçamento → anti-deadlock) roda **idêntica** para envelope local ou remoto.

### 5. O event log permanece a fonte da verdade (invariante #4)

Abrir/fechar um transporte cross-machine e admitir/remover um nó remoto são **fatos do Espaço** → eventos
**aditivos** (`serde(default)`, como toda a série) numa story de F5, projetáveis e re-deriváveis por
replay. O transporte é um meio de entrega; ele não é dono de estado que escapa do log. Um envelope
entregue por `RemoteOverSSH` gera os mesmos eventos de entrega que um local.

## Limite explícito (o que este ADR NÃO faz)

- **Não implementa** `RemoteOverSSH` nem nenhum transporte remoto. Entrega a trait `BusTransport` e a impl
  `InProcessTransport` (extração do comportamento atual atrás da porta), nada mais.
- **Não escolhe** o wire-protocol nem o handshake do túnel (SSH é a base canônica de partida, não a
  decisão final do protocolo) — isso é ADR próprio quando F5 abrir.
- **Não decide** o modelo de descoberta/pareamento entre máquinas (como dois Linas se acham) — fica para a
  story de F5, sobre esta porta.

## Consequências

- **F5 fica barata de começar e cara de autorizar — exatamente o objetivo.** Quando o multiplayer/VPS
  chegar, é uma impl de trait sobre uma fronteira já desenhada e uma série de eventos aditivos; o
  `Supervisor`, o `Router` e `deliver_a2a` não são re-arquitetados.
- **Local-first é preservado como default por construção**, não como guardrail que pode falhar.
- **Custo agora:** uma refatoração contida — extrair a entrega in-process do `Supervisor` para trás de
  `BusTransport` sem mudar comportamento (suíte do router + bus verde como critério). É a refatoração de
  abrir a porta, deliberadamente paga antes de F5 para não pagá-la sob pressão dentro de F5.
- **Gate:** stories que dependem desta decisão (todo o eixo F5 de cross-machine/VPS) **não iniciam até este
  ADR ser aceito** — é gate porque mal-desenhado fecha portas de continuidade (#7) e regride a doutrina de
  segurança (#2 e a admissão de ADR 0006/0010).

## Alternativas rejeitadas

- **Implementar `RemoteOverSSH` agora, junto com a porta.** Viola "simplicidade primeiro" e o gate-discipline
  do projeto: remoto não tem story, não tem critério de aceite observável, e acoplar a implementação à porta
  é precisamente o risco que esta decisão existe para evitar. A porta vale por si; o remoto vem com sua story.
- **Sem fronteira — meter o remoto direto no `Supervisor`/`deliver_a2a` quando F5 chegar.** Fecha a porta de
  continuidade: o core passaria a conhecer transporte de rede, misturando roteamento com I/O remoto, e
  tornaria a refatoração inevitável sob pressão de feature. Contraria invariante #7 (core/shell split) e a
  regra de processo de `CLAUDE.md` ("isto fecha uma porta acima? → pare e registre ADR").
- **Transporte remoto ligado por default / como mero toggle de config.** Quebra o invariante #2: local-first
  deixaria de ser garantido por ausência e passaria a depender de uma flag certa — um default errado vaza a
  máquina. Aqui o remoto não existe sem criação humana explícita + gate + escopo.
- **Deixar o `from`/escopo do envelope decidir o que fica acessível remotamente.** Viola a doutrina de
  segurança (campo escrito por agente nunca decide autorização). O escopo é decisão humana no momento da
  criação do túnel; o envelope continua sendo dado, nunca autoridade.
- **Transporte que cruza máquina sem ser fato do log.** Quebra o invariante #4: abrir/fechar túnel e
  admitir nó remoto são fatos do Espaço e têm de ser eventos aditivos projetáveis, não estado opaco do
  transporte.
