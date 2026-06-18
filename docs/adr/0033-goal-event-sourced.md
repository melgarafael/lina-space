# ADR 0033 — Goal como entidade event-sourced de primeira classe (loop-até-aceite com juiz separado)

- **Status:** ✅ Aceito (2026-06-18, pelo fundador via Maestro Terminal A) — destrava a onda F3-1 inteira (Goal-and-Loop)
- **Onda/Story:** spec 52 (objetivo como primitiva) · precede o loop autônomo da F5
- **Data:** 2026-06-16
- **Fontes:** spec `52` (Goal como primitiva; ponto 3 de Meadows — objetivo do sistema) · insumo Hermes (`hermes_cli/goals.py`: juiz LLM auxiliar separado do executor, fail-open, turn-budget, auto-pause por parse-failure) · ADR 0001 (plan event-sourced) · ADR 0020 (recovery por event log, sem durable execution) · ADR 0007 (campo de agente não decide segurança/autoridade) · ADR 0019 (definições mensuráveis, não impressões de LLM)

> **Gate: stories que dependem desta decisão não iniciam até este ADR ser aceito.**

## Contexto

A **Goal** (objetivo de uma sessão de trabalho — "deixe o app pronto pra subir", "feche os 3 bugs da tela X") hoje não existe como entidade no `lina-core`. O que existe é o **plano** (`Plan`, itens com owner/status — ADR 0001) e os eventos de roteamento/lifecycle. O objetivo vive implícito no prompt e na cabeça do agente; nada o materializa, projeta ou audita.

Isso fecha uma porta da F5: o **loop autônomo** ("trabalhe até o objetivo ser aceito, depois pare") precisa de uma primitiva durável que sobreviva a crash, seja reconstruível por replay e tenha um **critério de aceite verificável** — não um agente declarando "terminei". O ponto 3 de Meadows (spec 52) é literalmente *o objetivo do sistema*: é a alavanca mais alta que ainda cabe no produto sem virar autonomia plena, e por isso é onde a primitiva precisa nascer **antes** do loop, não junto com ele.

Três invariantes colidem com qualquer implementação ingênua:

1. **"O event log é a fonte da verdade"** (invariante #4) → a Goal não pode ser estado autoritativo num campo solto; tem de ser **projeção reconstruível por replay**, como o `plan.md` já é (ADR 0001).
2. **"Nenhum campo escrito por agente decide segurança/autoridade"** (ADR 0007, ADR 0019) → o veredito de aceite (PASS/FAIL) é produzido por LLM, e LLM **mente/alucina**. O veredito pode fechar o loop de trabalho, mas **nunca** pode decidir autorização, ordem ou identidade.
3. **"Sem LLM/harness próprios"** (invariante #1) → o juiz do aceite é um CLI/LLM de terceiro chamado pela borda, não um motor de avaliação interno do Lina.

O insumo Hermes (`hermes_cli/goals.py`) é a prova-de-conceito do mecanismo num produto real: um *Ralph loop* onde, a cada turno, um **juiz auxiliar separado** ("is this goal complete?") devolve `done`/`continue`, com **turn-budget** (`DEFAULT_MAX_TURNS`), **fail-open** (juiz quebrado faz `continue`, nunca trava o loop) e **auto-pause** após N parse-failures consecutivos. Adotamos o *padrão*, não o runtime — no Lina ele vira eventos sobre o log.

## Decisão

### 1. A Goal é entidade event-sourced — projeção, não estado autoritativo

A Goal é definida por uma sequência de `DomainEvent` **aditivos** e projetada como estado por replay, **herdando integralmente o padrão do ADR 0001** (supervisor escritor único; valida contra a projeção atual; `append` no log ANTES de tocar qualquer arquivo; render atômico). Replay do log reconstrói a Goal byte-a-byte; `open(dir) → project().goal` é a única fonte. A recuperação a crash **herda o ADR 0020**: idempotência durável + replay bastam — sem durable execution externo (ver §"Por que não Temporal").

Os eventos (todos novos, `event_version=1`, replay de log antigo nunca os vê — mesmo padrão de `TerminalRedimensionado`, `events.rs:214`; sem upcast):

| Evento | Significado | Fonte do dado |
|---|---|---|
| `GoalDefined { goal, raw }` | objetivo cru capturado (texto do humano) | humano |
| `GoalInterpreted { goal, statement, criteria }` | o agente reformula objetivo + critérios de aceite | agente (proposta, NÃO autoridade) |
| `GoalConfirmed { goal, by }` | **humano** confirma a interpretação (gate de partida) | humano |
| `GoalDecomposed { goal, items }` | objetivo quebrado em itens de plano (liga-se ao `Plan`, ADR 0001) | agente |
| `IterationOpened { goal, n }` | abre a iteração `n` do loop-até-aceite | supervisor |
| `IterationClosed { goal, n }` | fecha a iteração `n` (com ou sem veredito) | supervisor |
| `ReviewVerdict { goal, n, pass, by, reason }` | veredito do **juiz** para a iteração `n` | juiz LLM (separado) |
| `GoalAchieved { goal, by }` | objetivo aceito — loop para | projeção (PASS) + confirmação |
| `GoalEscalated { goal, reason }` | turn-budget estourado / juiz fail-open repetido / gate → humano | supervisor |

A projeção (`ProjectedState.goal`, espelhando `ProjectedState.plan` em `events.rs:1026`) deriva o estado corrente: objetivo, critérios, iteração atual, último veredito, status (`defining` → `confirmed` → `iterating` → `achieved`/`escalated`). Snapshots antigos desserializam com `goal` vazio (`#[serde(default)]`, herdando ADR 0001 §Consequências).

### 2. O loop-até-aceite tem JUIZ separado do executor — e turn-budget

O loop é: **executor trabalha a iteração → juiz avalia → PASS encerra / FAIL reabre**. Duas separações são inegociáveis:

- **Juiz ≠ executor.** Quem julga o aceite é um **LLM/CLI distinto** do que produziu o trabalho (insumo Hermes: "auxiliary model"). O executor não se auto-aprova — é o mesmo princípio do ADR 0019 (auto-report do agente é rejeitado: "agente pode mentir"). O veredito vira `ReviewVerdict`, atribuível e auditável por replay.
- **Turn-budget obrigatório.** Cada Goal tem `max_iterations` (default conservador, tunável por workspace, espelhando `RouterConfig` — ADR 0019/0020). Estourou → `GoalEscalated` + gate humano, **nunca** loop infinito. Isto **herda o circuit-breaker mandatório do ADR 0020** ("sem ele, retry storm consome quota e propaga erro"): o turn-budget é o breaker do loop de objetivo, assim como o breaker de entrega é o do pump A2A.

**Fail-open** (insumo Hermes): juiz quebrado/indisponível NÃO trava o loop nem fabrica um PASS — registra a falha e, persistindo, `GoalEscalated` ao humano (espelha o `auto-pause` após parse-failures consecutivos do Hermes; ADR 0020 §breaker). Falha do guardrail tende ao **pedir-ajuda-ao-humano**, nunca ao "achou que terminou".

### 3. O veredito PASS/FAIL NÃO decide segurança — só o avanço do trabalho (herda ADR 0007)

`ReviewVerdict.pass` é um **dado transportado, jamais autoridade**. Ele pode fechar o loop de *trabalho* (projeta `GoalAchieved`), mas **nunca** decide autorização, ordem de entrega, identidade de nó ou liberação de ação irreversível. A doutrina do `CLAUDE.md` é literal: "nenhum campo escrito por agente decide segurança". O juiz é um agente; seu veredito segue a mesma regra do `from`/`payload`/`contrato` do ADR 0007.

Consequência dura: **ação irreversível dentro do loop continua exigindo gate humano** (ADR 0004/0006/0021). Um PASS do juiz não autoriza `git push`, deploy ou envio real — o `GoalAchieved` encerra o ciclo de trabalho; o que o ciclo *executa* permanece sob os gates existentes. O loop-até-aceite acelera trabalho reversível e **para no gate** para o irreversível.

### 4. Por que isto é gate

Materializar o objetivo como primitiva **abre a porta** do loop autônomo da F5 (ponto 3 de Meadows, spec 52). Decidir errado aqui — Goal como estado mutável, juiz acoplado ao executor, veredito com poder de autoridade, loop sem budget — fragmenta um contrato que toda a F5 vai pendurar em cima. Por isso o ADR **precede** as stories e é **gate**: nada que dependa da entidade Goal inicia antes do aceite (mesma regra dos ADRs-gate do §V do épico F1).

## Por que NÃO durable execution externo (Temporal/Restate) — herda ADR 0020

O ADR 0020 já decidiu, para a entrega A2A, que **não** adotamos motor de durable execution; a Goal herda a mesma postura pelos mesmos motivos:

- **Local-first, processo único:** o event log (SQLite WAL + JSONL) já é a única fonte da verdade e sobrevive a `kill -9`. Um runtime de workflow adicionaria um **segundo log de verdade** (journal de atividades) — a tabela-autoridade paralela que o doc 01 §3 proíbe (invariante #4).
- **Os efeitos são injeções em PTY — irreversíveis:** nenhum replay de workflow torna um Enter "des-apertável". O ponto certo de idempotência é o event log + dedupe por `id` (ADR 0020), não o replay determinístico de side-effects.
- **Custo/complexidade:** Temporal-like exige versionamento de workflow code, non-determinism guards e infra distribuída — desproporcional para um loop single-thread cuja unidade é uma iteração de objetivo numa máquina.

Revisitar SE: (a) o Lina ganhar execução multi-máquina; (b) o loop precisar de saga/compensação multi-passo entre objetivos.

## Consequências

- **(+)** Goal 100% reconstruível do log; teste de replay prova reconstrução idêntica (gate herdado do ADR 0001). Zero formato-fio novo: reusa `EventStore`, `ProjectedState`, supervisor-escritor-único.
- **(+)** O loop autônomo da F5 encaixa sem refazer o core — só consome a projeção `goal` e os eventos já versionados.
- **(+)** Auditoria completa: cada veredito, iteração e escalada é evento atribuível; `lina retro`/painéis derivam do log (invariante #4).
- **(−)** `ProjectedState` ganha o campo `goal` (`#[serde(default)]`): o fingerprint determinístico passa a incluí-lo (esperado — a Goal é parte do estado).
- **(−)** Custo de LLM do juiz a cada iteração — conta no teto de custo (ADR 0005); o turn-budget o limita.
- **(risco a provar quando aceito)** (a) juiz fail-open não fabrica PASS sob falha; (b) turn-budget estourado escala, não loopa; (c) PASS não libera ação irreversível (gate humano permanece); (d) replay do log reconstrói a Goal byte-a-byte.

## Alternativas rejeitadas

- **Goal como estado mutável (campo num struct, fora do log)** — viola o invariante #4 e a decisão do ADR 0001 ("o plano *é* uma projeção do log"). Não sobrevive a crash de forma auditável. Rejeitada.
- **Juiz = executor (o agente se auto-aprova)** — agente mente/alucina (ADR 0019 §auto-report rejeitado); o aceite vira teatro. Rejeitada — o juiz é separado por princípio, não por implementação.
- **Veredito do juiz com poder de autoridade (PASS libera irreversível)** — viola ADR 0007/0004: campo escrito por agente jamais decide autorização; ação irreversível exige gate humano. Rejeitada.
- **Loop sem turn-budget ("trabalha até passar")** — retry storm que consome quota e nunca para se o juiz oscilar (ADR 0020 §breaker mandatório). Rejeitada.
- **Durable execution externo (Temporal/Restate)** — segundo log de verdade + infra distribuída num produto local-first de processo único (ADR 0020). Rejeitada.
- **Fail-closed no juiz (juiz quebrado trava/bloqueia)** — wedge do loop por falha de ferramenta auxiliar; o insumo Hermes documenta fail-open + auto-pause como a postura correta. Rejeitada — falha do guardrail escala ao humano, não congela o trabalho.
