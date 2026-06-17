# ADR 0032 — dependências (`parents`) no `PlanItem` e verbos de escrita do plano (`add`/`seed`)

- **Status:** Proposto (é GATE — ainda não aceito)
- **Onda/Story:** habilitador do Goal-and-Loop (spec `52`); pré-requisito de orquestração estruturada (skills `lina-orchestration`/`lina-dispatch`)
- **Data:** 2026-06-16
- **Fontes:** spec `52` (Goal-and-Loop) · invariante #4 (event log fonte da verdade; eventos aditivos `serde(default)`) · família ADR 0007 (campo escrito por agente nunca decide autoridade) · ADR 0001 (plano por event-sourcing; supervisor escritor único) · código: `crates/lina-core/src/plan.rs:88-97`, `crates/lina-core/src/router.rs:1767-1819`, `crates/lina-bootstrap/src/bin/lina.rs:1026-1036`

> **Gate:** stories que dependem desta decisão não iniciam até este ADR ser aceito.

## Contexto

As skills de orquestração **já assumem** que um item do plano declara suas dependências:
`lina-orchestration` instrui "decompor (`plan.md` com `parents:`)" e o método Goal-and-Loop
(spec `52`) precisa saber qual item **pode** começar (todos os pais `Done`) para distribuir
trabalho sem que um worker reivindique uma tarefa cujo pré-requisito ainda não existe. O
**código não tem isso**:

- `PlanItem` (`crates/lina-core/src/plan.rs:88-97`) carrega só `id`/`desc`/`owner`/`status`
  — nenhum campo de dependência, de critério de aceite ou de orçamento.
- `try_claim` (`plan.rs:185-200`) só valida existência e posse (`AlreadyOwned`); **não há
  guard de ordem** — qualquer item `Todo` é reivindicável, na ordem que for.
- O CLI **não tem verbo de escrita estrutural**: `run_plan` (`lina.rs:1026-1036`) roteia
  apenas `read | claim | check`. O core até possui `Plan::add_item` (`plan.rs:162-173`) e o
  evento `PlanItemAdded` (`crates/lina-core/src/events.rs:414`), mas **não há porta de CLI**
  para semear/adicionar item — hoje só nasce item por outro caminho (adoção/bootstrap).

Essa é uma **incoerência doutrina↔código**: a doutrina prescreve `parents:` e decomposição
pelo plano, mas um agente que siga a skill à risca escreveria um campo que o parser rígido
de `plan.md` rejeita e um guard que não existe. Fechar essa lacuna é **pré-requisito** do
Goal-and-Loop — por isso é GATE, não story livre.

## Decisão

### 1. Estender `PlanItem` com campos aditivos (replay de plano antigo não quebra)

Em `crates/lina-core/src/plan.rs:88-97`, acrescentar ao struct:

```rust
#[serde(default)] pub parents: Vec<String>,        // ids de itens-pai (dependências)
#[serde(default)] pub acceptance: Option<String>,  // critério de aceite observável (ADR 0019)
#[serde(default)] pub budget: Option<String>,      // orçamento/limite declarado do item
```

Os três são `#[serde(default)]` — **invariante #4**: replay de um event log/`plan.md`
anterior (sem esses campos) reconstrói com `parents: []`, `acceptance: None`, `budget: None`,
byte-compatível. O parser/serializer rígido de `plan.md` ganha um sufixo de campos
**opcionais** (ausência ≡ default), preservando o round-trip exato exigido pelo módulo.

`parents` é a única dependência que **muda comportamento** (guard, §3). `acceptance` e
`budget` são metadados projetados — informam o worker e o `lina retro`, não gateiam nada por
si (continuam sujeitos ao gate humano em ação irreversível e ao teto de custo do ADR 0005).

### 2. Verbos de ESCRITA do plano no CLI — o supervisor continua o escritor único

Adicionar a `run_plan` (`lina.rs:1026-1036`):

- `lina plan add <id> "<desc>" [--parent <id>]… [--acceptance "…"] [--budget "…"]` — semeia
  UM item.
- `lina plan seed <arquivo>` — semeia VÁRIOS itens de uma vez (decomposição inicial de um
  épico), cada um virando um `add`.

Ambos seguem **exatamente** o padrão de `claim`/`check` (`run_plan_intent`, `lina.rs:1060`):
o bin é processo SEPARADO, **não escreve `plan.md`** — deposita um envelope na mailbox por-nó
(`from` carimbado pelo supervisor, `ref=plan:<id>`, intent `plan.add`). O **supervisor**
intercepta por intent em `handle_plan` (`router.rs:1781`), valida via `Plan::add_item`
(estendido para receber os novos campos), loga `PlanItemAdded` (com `parents`/`acceptance`/
`budget` aditivos, `serde(default)` em `events.rs:414`) e só então reescreve a projeção. **O
CLI ganha o verbo; o supervisor continua o único escritor de `.lina/` (ADR 0001).**

### 3. Guard de ordem: `try_claim` recusa item com pai não-`Done`

Em `try_claim` (`plan.rs:185-200`), antes de aplicar o claim, verificar os `parents`: se
**algum** pai não estiver `ItemState::Done`, rejeitar com a **nova** variante
`PlanError::ParentsNotDone { id, pendentes: Vec<String> }` (adicionada ao enum de
`plan.rs:111-135`). É um guard de **estado projetado** — lê o status atual dos pais na mesma
projeção que já fundamenta o claim; é determinístico, sem LLM.

O roteador mapeia a rejeição: `handle_plan` (`router.rs:1788`) já converte
`Err(e) => RouteOutcome::PlanRejected(e.to_string())` — `ParentsNotDone` herda esse caminho
sem novo ramo no router, virando `PlanRejected` com a lista de pais pendentes. Ao usuário
leigo, isso narra como "essa tarefa depende de outra que ainda não terminou — vou pegar a que
está pronta" (TOM), nunca o erro cru.

### 4. Doutrina de segurança preservada (família ADR 0007)

`parents` é **dado transportado, não autoridade**: declara ORDEM (qual tarefa antes de qual),
**nunca** identidade, posse ou autorização. Um agente não compra privilégio escrevendo
`parents`/`acceptance`/`budget` — o que decide posse continua sendo `owner` validado por
`AlreadyOwned`, e o que decide a APLICAÇÃO no `plan.md` continua sendo o supervisor (escritor
único, `from` carimbado por ele, não pelo campo do envelope). O guard de §3 só pode **negar**
um claim (fail-safe restritivo), jamais conceder algo. Ação irreversível segue exigindo gate
humano; nada aqui o contorna.

## Consequências

- **Positivas:** a doutrina de orquestração passa a ter respaldo no código — `parents:` deixa
  de ser promessa de skill e vira invariante checado. O Goal-and-Loop (spec `52`) ganha o
  pré-requisito que faltava: "este item pode começar?" tem resposta determinística (todos os
  pais `Done`). O plano vira um grafo de dependências auditável (projeção do log), não uma
  lista plana.
- **Custo / superfície:** três campos novos no struct e no evento (aditivos, baixo risco de
  replay), uma variante de erro, dois verbos de CLI e um guard em `try_claim`. O parser rígido
  ganha campos opcionais — exige teste de round-trip exato (plano antigo sem os campos →
  reserializa idêntico; plano novo com os campos → idem) **e** teste do guard
  (`ParentsNotDone` quando pai `Todo`/`Doing`/`Blocked`; claim ok quando todos `Done`).
- **Migração:** zero. Plano/log pré-0032 reconstrói com defaults; nenhum item existente passa
  a ser bloqueado (sem `parents` ⇒ guard nunca morde).
- **A provar (red-team):** (a) `plan add`/`seed` de origem aplica e reprojeta byte-a-byte;
  (b) claim de item com pai não-`Done` → `PlanRejected`, 0 escrita no `plan.md`; (c) replay
  de `plan.md` antigo → defaults, round-trip exato; (d) `parents` forjado num envelope NÃO
  concede posse nem autoridade (a posse segue por `owner`/`AlreadyOwned`).

## Alternativas rejeitadas

- **Manter `parents:` só na skill (status quo)** — perpetua a incoerência: o agente escreve um
  campo que o parser rejeita e confia num guard inexistente; o Goal-and-Loop não tem como
  saber o que pode começar. É a lacuna que este ADR existe para fechar.
- **`parents` como campo NÃO-aditivo (sem `serde(default)`)** — quebraria o replay de todo
  event log/`plan.md` anterior (viola invariante #4). Rejeitado de saída.
- **Guard no router/handle_plan em vez de `try_claim`** — espalharia a regra de ordem para
  fora do modelo puro do plano. A ordem é propriedade do PLANO; `try_claim` é o ponto único
  onde claim é validado contra o estado — o guard mora lá, e o router só traduz o erro
  (`PlanRejected`), mantendo a separação comando-vs-evento de `plan.rs:8-13`.
- **Resolver dependências por LLM ("este item parece depender daquele?")** — não-determinismo
  num guardrail (viola invariante #1 e a doutrina dos ADRs 0007/0019). A dependência é
  **declarada** (`parents` explícito), checada por igualdade de estado, nunca inferida.
- **Escrita do `plan.md` direto pelo bin do CLI** — quebraria o escritor único (ADR 0001):
  dois escritores concorrentes corromperiam a projeção. O CLI deposita intent; o supervisor
  aplica — igual a `claim`/`check`.
