# ADR 0043 — DiskReclaim (poda de disco irreversível) em F3-5, sob gate humano custodiado

- **Status:** 🟡 Proposto (2026-06-22, Arquiteto Terminal A na fundação F3-5) — gate da story F3-5-8
- **Onda/Story:** F3-5-8 (épico `39` §"Onda F3-5", spec `53 - Buffers Geridos` §11.C)
- **Data:** 2026-06-22
- **Fontes:** spec `53` §11.C (DiskBudget) · doc-fonte do fundador linha 60 ("limpeza de disco com autorização do usuário") · achado de dogfooding #25 (ENOSPC: ~30 GB de `target/`, disco do fundador ~95%) · invariante #6 (não-técnico-first; nunca perder em silêncio; apagar é irreversível) · família ADR 0007 (campo de agente nunca decide autoridade) · ADR 0004 (custódia inquebrável) · ADR 0021 §5 (gesto referencia `stable_id`, nunca texto/posição) · contrato: `crates/lina-core/src/events.rs` (variantes `DiskPressureSignaled`/`DiskReclaimProposed`/`DiskReclaimApproved`/`DiskReclaimExecuted` + `ReclaimCandidate`, fixadas na largada F3-5)

> **Gate:** F3-5-8 não inicia até este ADR ser aceito.

## Contexto

A spec 53 posicionava o `DiskBudget` em **F4/F5** (estoques maduros). O épico 39 o **puxou para F3-5** porque o ENOSPC é um risco **presente e vivido**: o achado #25 registrou ~30 GB de `target/` de cargo derrubando o orquestrador, com o disco do fundador a ~95%. Esperar F4/F5 deixa o produto vulnerável a travar em uso prolongado — exatamente o que a onda "buffers geridos" existe para impedir.

Mas a poda de disco é a capacidade **mais perigosa** do Lina até aqui: **apagar arquivos do disco do usuário é IRREVERSÍVEL**. Trazê-la para F3 fecha uma porta — introduz uma ação destrutiva no sistema — e por isso exige registro + salvaguarda inegociável (regra de processo do CLAUDE.md; precedente do git-de-runtime no ADR 0040 da F3-4).

## Decisão

### 1. O ciclo de 4 eventos separa DETECTAR de APAGAR (a porta destrutiva é uma só)

`DiskPressureSignaled` (detectar) → `DiskReclaimProposed` (propor candidatos) → **`DiskReclaimApproved`** (gate humano) → `DiskReclaimExecuted` (poda). Os três primeiros são **observabilidade/proposta** (livres ou propostos por autonomia); só `DiskReclaimExecuted` apaga bytes, e **só pode existir após `DiskReclaimApproved`**.

### 2. Salvaguarda INEGOCIÁVEL: zero `DiskReclaimExecuted` sem `DiskReclaimApproved`

Apagar disco é **gate humano em TODOS os níveis de autonomia** (manual/assistido/autônomo — igual `git push`/deploy/`rm -rf`, família gated-hard #19). O gate é o **gesto custodiado** (`AttentionKind::Custody`, attention.rs), com precedência sobre permissões — o canal já existente do ADR 0004. O gesto referencia o `stable_id` da proposta, **nunca** texto/posição (ADR 0021 §5).

### 3. `approved_by` é EXIBIÇÃO, jamais autoridade (família ADR 0007)

O campo `approved_by` de `DiskReclaimApproved` é para **humanizar** quem aprovou na tela — **não** é o que autoriza a poda. A autoridade é o **gesto custodiado** (o broker só executa após o gesto). Forjar `approved_by` no payload **não** dispara poda alguma: o `DiskReclaimExecuted` só nasce do caminho custodiado, não de um campo escrito por agente.

### 4. O core PROPÕE; a execução é do broker custodiado (camada PURA vs execução)

O core (determinístico, ZERO LLM): probe de disco → varre candidatos (`ReclaimCandidate{path, bytes, kind}`) → apenda `DiskReclaimProposed`. A **execução** (`rm`/`cargo clean`) é do broker, **após** o gesto — e separada da camada pura (lição de dogfooding #36: a frente prova a decisão com **disco simulado**, sem rodar a poda real; o `permission_prompt`/gate fica na execução custodiada).

### 5. Single-machine (a porta cross-machine fica fechada)

A poda opera no disco **local** (`workspace_target`/`app_target`/`disk_total`). Nada cross-machine (ADR 0034 intocado). O `DiskBudget` maduro (cron/autônomo) é F5; aqui é **sob gesto humano**, sempre.

## Consequências

- **Positivas:** o ENOSPC #25 deixa de poder derrubar o orquestrador em silêncio — o Lina **avisa** (pressão) e **propõe** (candidatos), e o humano **decide**. O trabalho nunca some sem gate.
- **Porta que se ABRE (registrada):** a primeira ação **destrutiva de disco** do produto. Confinada: 4 eventos com a execução atrás do gesto custodiado; `approved_by` nunca autoridade; single-machine.
- **Custo / superfície:** 4 variantes de evento + `ReclaimCandidate` (já fixados na largada) + a projeção `DiskBudget` + o probe + a fiação do gesto custodiado.
- **A provar (red-team):** (a) disco a 96% → `DiskPressureSignaled{critical}` + proposta; (b) **zero bytes apagados sem `DiskReclaimApproved`**; (c) `approved_by` forjado no payload NÃO dispara `DiskReclaimExecuted`; (d) com o gesto custodiado → `DiskReclaimExecuted{reclaimed_bytes>0}`; (e) replay reconstrói o `DiskBudget` byte-a-byte.

## Alternativas rejeitadas

- **Deixar o DiskBudget em F4/F5 (status quo da spec 53):** o ENOSPC é um risco presente (#25, disco a 95%); adiar deixa o produto vulnerável a travar agora. Trazer para F3-5 **sob gate custodiado** dá a proteção sem a autonomia plena (que fica para F5).
- **Poda automática quando o disco enche (sem gate):** apagar é irreversível (inv #6); um falso-positivo apagaria trabalho. Gate humano SEMPRE — a autonomia nunca afrouxa para ação destrutiva.
- **`approved_by` como autoridade (o agente "aprova" escrevendo o campo):** viola a família ADR 0007 (campo de agente nunca decide). A autoridade é o gesto custodiado, não o payload.
- **Core executa a poda direto (`rm` no caminho do evento):** acopla decisão e execução destrutiva, e roda no caminho crítico. O core PROPÕE; o broker custodiado EXECUTA após o gesto (separação pura/execução, lição #36).
