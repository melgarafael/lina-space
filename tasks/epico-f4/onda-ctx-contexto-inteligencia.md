# Onda F4-CTX — Contexto & Inteligência (port deliberado do ruflo) — peça executável

> **Fonte da verdade story-a-story** da onda F4-CTX. Consolida o épico [[41 - Epico Fase 4 — Integracoes Canais e Contexto]] §III (pilar "Contexto") + os ADRs **0057** (memória de trajetória), **0058** (roteamento tarefa→terminal) e **0059** (gate de auto-aprimoramento). **Precedência:** o ADR aceito vence tudo; o épico dá os critérios observáveis; esta peça detalha território/ordem; em conflito de detalhe, o épico vence esta peça. **Maestro da rodada:** a definir.
> **Meta da onda:** dar ao Lina a **memória de RESULTADO** que nenhum CLI mono-processo tem — "da última vez que enfrentei uma tarefa como esta, o caminho X funcionou" — e usá-la para **rotear melhor** e **propor melhorias medidas (nunca aplicadas sozinhas)**. Tudo como **projeção sobre o event log** (inv #4), local-first (inv #2). É a maturação da C2 do ADR 0045 com o pipeline de recall do ruflo como referência.

## Origem (análise do ruflo — 2026-06-29)

Clonamos `ruvnet/ruflo`, rodamos o **graphify** nos 9 plugins relevantes (382 nós / 549 arestas / 37 comunidades — `tasks/epico-f4/refs/ruflo-graphify/graph.html`) e lemos a lógica real (não o README). Conclusão que orienta esta onda: **o que vale do ruflo não é o que ele anuncia.** Consenso (Byzantine/Raft) é branding não-implementado; "witness verification" é afirmado mas não existe no código; "neural/SONA" é heurística + outcome. **O que é real e forte** — e que o Lina já tem substrato para fazer melhor — é o **pipeline de recall (SmartRetrieval)** e o **auto-evolve human-gated**. Detalhe em `docs/adr/0057..0059`.

## Estado de partida (reconhecido no código, 2026-06-29)

### Já existe (REUSAR, não reinventar)
- **`crates/lina-core/src/skill_index.rs`** — índice BM25 léxico (C1 do ADR 0045). É a base do recall; a onda **estende** o ranking, não troca o índice.
- **`events.rs:1387/1431`** — `SkillSelected { …, selection_id, by_role, … }` + `SkillOutcome { selection_ref, signal: Success|Failure|Reworked|HumanOverride }`. O sinal de outcome **já é logado** (decisão passiva, 0045 §4). A trajetória é **derivável** disso — nenhum evento novo no MVP.
- **`crates/lina-core/src/mentality.rs`** + eventos `BeliefProposed→BeliefReinforced→BeliefEstablished→BeliefRetired` + `CorrectionObserved` — a máquina de aprendizado por evidência com **gate humano** (`BeliefEstablished`). O ADR 0059 **alimenta** este gate; não cria outro.
- **`crates/lina-core/src/powers.rs`** (ADR 0052, Área de Poderes) — capacidade determinística por terminal. Insumo do roteador (ADR 0058).
- **`lina list`** / supervisor — disponibilidade dos terminais.

### NÃO existe (o delta que esta onda constrói)
1. **Projeção de trajetória** — agregar `SkillSelected`/`SkillOutcome`/`SkillInvoked`/`CorrectionObserved` em "trajetória T (papel+task_kind+capacidades) → veredito".
2. **Pipeline de recall em 5 fases** — RRF + boost de recência + MMR sobre o BM25 (hoje é BM25 puro).
3. **Consolidação** — dedup/poda de stale da projeção ativa.
4. **Roteador tarefa→terminal** — score capacidade × outcome × disponibilidade → sugestão com motivo legível.
5. **Guardrail anti-drift** — checagem "entrega casa com objetivo?" ao fechar item.
6. **Score + bench** — cartão de prontidão + corpus fixo de avaliação.
7. **Proposta medida** — variante medida no bench → `BeliefProposed{ delta de score }` (nunca aplica).

## LARGADA (a fazer pelo Maestro da rodada — dono único das costuras)
- **Costuras de dono único:** `events.rs`/`lib.rs` — **só** se a granularidade exigir um envelope de trajetória explícito (porta D1 do 0057; **evitar no MVP**, a projeção deriva dos eventos existentes). No MVP, **nenhuma costura** — tudo é módulo novo + projeção.
- **Stubs sugeridos:** `crates/lina-core/src/trajectory.rs` (projeção + recall), `crates/lina-core/src/task_router.rs` (roteador), `crates/lina-core/src/bench.rs` (score+bench). `pub mod` no `lib.rs` = dono único Maestro.

## As 7 stories — território de arquivo DISJUNTO por frente

### F4-CTX-1 — Projeção de trajetória · ADR 0057 D1 · TAG [core]
- **Território:** `crates/lina-core/src/trajectory.rs` (NOVO) + testes inline.
- **Entrega:** projeção pura que reidrata, por replay, `Trajectory { task_kind, by_role, candidates[], verdict }` a partir de `SkillSelected`/`SkillOutcome`/`SkillInvoked`/`CorrectionObserved`. `task_kind` **emerge do retrieval** (0045 §3), sem classificador. Mapeamento de veredito `Success→pass / Failure|Reworked→fail / HumanOverride→partial`.
- **Observável:** injetar uma sequência de eventos de uma tarefa → a projeção devolve a trajetória com o veredito correto; **replay reconstrói idêntico**; log sem os eventos reidrata vazio sem erro (aditividade).
- **Invariante:** projeção pura — só lê `EventRecord`; nenhum estado fora do log (inv #4).

### F4-CTX-2 — Recall em 5 fases (SmartRetrieval-lite) · ADR 0057 D2 · TAG [core]
- **Território:** `trajectory.rs` (fn de recall) + estende o consumo de `skill_index.rs` (coordenar ponto de leitura com o Maestro; **não** reescrever o índice).
- **Entrega:** `recall(task) → [Trajectory]` ranqueado por **expansão de query → multi-query+RRF → boost de recência (decai ~0,95/dia, calibrável) → MMR (Jaccard) → score composto (relevância × outcome × diversidade × recência)**. 100% léxico/determinístico; **sem embeddings** (porta D4).
- **Observável:** top-k recupera a trajetória de maior `Success` para o `task_kind`; `SkillOutcome{Failure}` na campeã a rebaixa; uma sub-query mal-formulada **não** colapsa o top-k (RRF); recente-com-`Failure` **não** vence antiga-com-muitos-`Success` (recência modula).
- **Invariante:** recência informa, **nunca** decide (a causa-raiz do 0045).

### F4-CTX-3 — Consolidação (dedup + poda) · ADR 0057 D3 · TAG [core]
- **Território:** `trajectory.rs` (fn de manutenção) + teste.
- **Entrega:** dedup de trajetórias quase idênticas; poda de stale (antigas + zero recuperações úteis) da projeção **ativa** — nunca do log.
- **Observável:** reprojetar do log inteiro pós-poda reproduz a projeção ativa (idempotência); a trajetória stale **continua no log** (varredura prova).
- **Invariante:** consolidação muda a projeção, jamais o log (inv #4).

### F4-CTX-4 — Roteador tarefa→terminal · ADR 0058 R1/R2 · TAG [core]
- **Território:** `crates/lina-core/src/task_router.rs` (NOVO) — lê `powers.rs` (capacidade) + `trajectory.rs` (outcome) + disponibilidade.
- **Entrega:** `suggest(task) → [RouteSuggestion{ terminal, score, motivo_pt_br, tier_effort }]`. Score = capacidade × outcome × disponibilidade. Tier de effort por complexidade (tamanho da fronteira, ação irreversível, task_kind).
- **Observável:** @A com histórico `Success` em K é sugerido sobre @B com `Failure`; inverter outcomes inverte a sugestão; terminal sem a capacidade do scan **não** aparece por melhor que seja o histórico genérico.
- **Invariante:** sugestão é DADO; capacidade vem do **scan** (0052), não auto-declarável; não toca `Router`/`deliver_a2a`.

### F4-CTX-5 — Guardrail anti-drift · ADR 0058 R3 · TAG [core]
- **Território:** `task_router.rs` (fn `check_drift`) — reusa cold-review/verification existentes.
- **Entrega:** ao fechar um item do plano, checagem barata "entrega casa com o objetivo declarado?". Divergiu → não fecha, devolve ao Maestro.
- **Observável:** uma entrega propositalmente fora do objetivo **não fecha**.
- **Invariante:** porta o **único** pedaço de "swarm" real do ruflo; **não** importa consenso (branding).

### F4-CTX-6 — Score + bench · ADR 0059 G1/G2 · TAG [core]
- **Território:** `crates/lina-core/src/bench.rs` (NOVO) + corpus em `bench/` (versionado).
- **Entrega:** `score(capability) → Card{ fit, gate_confidence, task_coverage, tool_safety, memory_usefulness }` (projeção read-only) + `bench(variant) → resultado` contra corpus fixo `{entrada, esperado, peso}`.
- **Observável:** duas variantes com cold-review PASS rate diferente produzem scores ordenados certo; uma variante mede contra o corpus de bench, não contra o repo.
- **Invariante:** score **informa** o humano, não barra merge sozinho (não vira gate duro de CI).

### F4-CTX-7 — Proposta medida (alimenta o gate humano) · ADR 0059 G3/G4 · TAG [core]
- **Território:** `bench.rs` (fn de proposta) → emite `BeliefProposed` (reusa `mentality.rs`).
- **Entrega:** variante (uma mutação) → mede no bench → ganho medido → `BeliefProposed{ proposta, evidência: delta de score }`. **Nunca** aplica; só após `BeliefEstablished` humano a mudança vale. Tripwire: variante que toca segredo/egress é **desqualificada** antes de medir (reusa a guarda existente).
- **Observável (teste-chave):** force uma variante a vencer o bench com folga → **nada muda em produção**; só após `BeliefEstablished` simulado a mudança passa a valer. **Não existe** caminho medição→produção sem aval.
- **Invariante:** auto-aplicação proibida por construção (a linha vermelha do fundador).

### Frentes de borda (acompanham, não bloqueiam)
- **UI:** narrar ao leigo "lembrei que da última vez X funcionou" / "sugiro o @Fulano porque…" em pt-br simples (TOM) — sem jargão de score/RRF. Dono: Especialista em Telas.
- **QA-RED:** o teste de mutação de segurança de cada ADR (gate humano não-furável) entregue como `#[ignore]` que o dono liga. Dono: Caça Problemas.

## ADR-GATE
F4-CTX-1..3 não iniciam sem **ADR 0057** aceito; F4-CTX-4/5 sem **0058**; F4-CTX-6/7 sem **0059**. Os três estão **Aceitos** (ratificados em sessão direta com o fundador, 2026-06-29). A projeção (F4-CTX-1) pode começar a modelagem com os stubs — o 0057 está aceito.

## Notas de processo
- **Sem evento novo no MVP** (D1 do 0057): a trajetória deriva dos eventos existentes → replay de log antigo nunca quebra.
- **Throttle/delta obrigatório** no recall (hot-path de chegada de tarefa) — lição `fiar-projecao-no-pump-exige-throttle`.
- **Workers não commitam**; o Maestro valida de fora (exit codes diretos) e commita por fatia.
