# ADR 0048 — `situation_hash` determinístico + casamento correção→crença existente (sem LLM no core)

- **Status:** **Aceito** (decisão A2 do ADR-gate LOOP, fatia-2 Mentality; Arquiteto, 2026-06-24). Destrava `LP-CORE` (ELO3: `reflect_correction` + derivação) e fecha o gap de produção do `situation_hash` (hoje só literais de teste).
- **Escopo:** core puro (`mentality.rs`), ZERO LLM. DISJUNTO da Fase 4. Depende do ADR 0047 (o `shadow` que propõe o casamento) e costura com o `task_kind` da spec `0045-contrato-eventos-skill-routing`.
- **Relacionados:** spec 35 (Mentality), ADR 0045 (`task_kind`), ADR 0007/0026 (autoridade server-side).

## Contexto

Dois buracos impedem o motor de aprendizado de rodar em produção:

**(a) `situation_hash` não existe fora de teste.** O anti-gaming da promoção depende dele: `BeliefReinforced{situation_hash}` (`events.rs:1199`) só conta para promover se a situação for **distinta** (M-PROMO: N hashes distintos, default N=2; `mentality.rs:172-188`). Hoje os hashes são literais (`"situacao-A"`, `sit-{seed}`) injetados por testes. Em produção, **ninguém os deriva** — sem isso, ou tudo conta (gaming trivial: a mesma correção repetida promove) ou nada conta.

**(b) Casar uma nova correção a uma crença EXISTENTE.** Chega um `CorrectionObserved{role, summary}` (`events.rs:1167`). É uma crença **nova**, um **reforço** de uma que já existe para o papel, ou uma **contradição**? Disso depende qual evento emitir:
- nova → `belief_id` novo → `BeliefProposed`;
- reforço → MESMO `belief_id` → `BeliefReinforced{situation_hash}` (conta para promoção);
- contradição → `BeliefChallenged` (zera o progresso).

`reflect_correction(belief_id, candidate)` (`mentality.rs:753`) já RECEBE um `belief_id` pronto e o doc-comment já antecipa: *"belief_id é gerado pelo caller (determinístico — derivável da correção para idempotência sob replay)"*. **Falta especificar essa derivação e o casamento** — e fazê-lo **sem LLM no core** (inv #1).

## Decisão

### A2.a — `situation_hash` = hash determinístico ancorado no `task_kind`

`situation_hash = sha256_hex( canon(role) || "\x1f" || canon(task_kind) )[..16]` onde:
- `task_kind` é o **mesmo** da spec 0045 (emerge do retrieval: papel + termos dominantes da query). **A "situação" em que uma crença se aplicou É o tipo de tarefa corrente.**
- `canon(x)` = `x.trim().to_lowercase()` com espaços colapsados (normalização estável — a mesma situação semântica não vira hashes diferentes por capitalização/espaço).
- separador `\x1f` (unit separator) evita colisão por concatenação ambígua.
- função pura, **sem `now_ms`, sem random** → replay reproduz idêntico (inv #4). `sha2::Sha256` **já é dependência de `lina-core`** (`Cargo.toml`; padrão já usado em `approval.rs`/`lifecycle.rs`) — reusar, não adicionar. Qualquer hash estável serve; o contrato é "puro e determinístico", não o algoritmo.

**Consequência do design:** uma crença reforçada em **2 `task_kind` distintos** = 2 situações distintas = promove (evidência diversa real). Reforçada **2× no mesmo `task_kind`** = 1 situação = NÃO promove (anti-gaming §6.4). É exatamente a semântica que M-PROMO já espera — agora com uma fonte de hash legítima.

### A2.b — Casamento delegado ao sombra (DADO), AUTORIDADE re-validada no core

1. **O core monta o contexto — SNAPSHOT que viaja com o request:** no `enqueue` da `ReflectionRequest`, o caller captura um **snapshot** das crenças vivas do papel — `Mentality::top_k_for_role(role, k, now)` (`mentality.rs:283`) — como lista NUMERADA `{ #i → (belief_id, statement) }` e o **anexa à `ReflectionRequest`** (campo novo em `a2a.rs:573`). Só `#i` + `statement` vão no prompt do sombra (ADR 0047); **nunca** ids brutos forjáveis. O snapshot é a ÚNICA referência usada tanto para montar o prompt quanto para resolver `#i` na re-validação (passo 3). **Por que viajar, não re-ler:** entre `enqueue` → `drain` → re-validação a `Mentality` pode mudar (nova crença, retire); re-ler `top_k_for_role` na re-validação faria `#i` apontar para OUTRA crença — race de corretude/segurança. O snapshot congela a numeração (nota de impl do Maestro Loop, A2.b).
2. **O sombra propõe** (1 CLI-call, fora do caminho crítico): destila a lição via `[LINA::BELIEF] <statement>` e **acrescenta um marcador de casamento** — `reforça #i` | `contradiz #i` | `nova`. Tudo isso é **DADO não-confiável**.
3. **O core RE-VALIDA e decide a autoria:**
   - `reforça #i` / `contradiz #i` → o core resolve `#i` contra o **snapshot anexado à `ReflectionRequest`** (índice → `belief_id`, congelado no `enqueue` — passo 1), nunca contra uma releitura da projeção. Se `#i` está na faixa do snapshot E a crença ainda pertence ao papel server-side → emite `BeliefReinforced{belief_id, situation_hash}` (A2.a) ou `BeliefChallenged{belief_id}`. **O sombra nunca fornece um `belief_id` — só escolhe entre os que o core lhe mostrou.** Índice fora de faixa / papel divergente → trata como `nova` (degradação segura).
   - `nova` (ou casamento inválido) → `belief_id` **determinístico derivado da `correction_id`** (abaixo) → `reflect_correction(belief_id, candidate)` → `Propose(BeliefProposed)` | `Refuse(BeliefRetired{refuted})`.
4. **`belief_id` determinístico (idempotência sob replay):** `belief_id = "bel-" || sha256_hex(canon(role) || "\x1f" || correction_id)[..16]`. Como `correction_id` é único e estável (id do `CorrectionObserved`), replay do mesmo evento → mesmo `belief_id` → **não duplica crença** mesmo que o pump reprocesse. Fecha o "derivável da correção" que o `reflect_correction` já pressupõe.

## Segurança (doutrina inegociável)

- **Crença é DADO, jamais autoridade.** O casamento proposto pelo sombra nunca cria identidade: o core só aceita um `belief_id` que **ele mesmo** apresentou (via índice) e que existe na projeção do **papel certo**. O texto do CLI não nomeia ids.
- **`role` sempre server-side** (`CorrectionObserved.role` / `DistilledBelief.role`), nunca do payload (`events.rs:1168`, `mentality.rs:719`).
- **ZERO LLM no core:** o casamento *semântico* roda no sombra (CLI de terceiro); a *decisão* (id válido? papel certo? statement durável?) é determinística no core (`reflect_correction` + resolução de índice).
- **Anti-poisoning intacto:** `reflect_correction` segue barrando instrução de segurança/PII/comando/URL antes de materializar (`mentality.rs:744`); o casamento não abre desvio — um `reforça #i` malicioso só pode apontar para uma crença JÁ existente e validada.
- **Idempotência = anti-replay-gaming:** `belief_id` e `situation_hash` determinísticos impedem que reprocessar o log infle contagem de promoção.

## Por quê assim (alternativas descartadas)

- **Casamento por similaridade léxica no core (sem sombra):** tentador (BM25 sobre statements), mas frágil para paráfrase ("use pnpm" vs "evite npm") e reintroduz heurística rígida que vira dívida. O sombra já roda para destilar — reusá-lo para o casamento é custo marginal zero, e o core **re-valida** o que importa (id+papel). Rejeitado como mecanismo isolado; pode entrar como *fallback* determinístico se o sombra não opinar.
- **`situation_hash` = hash do `summary` cru:** a MESMA situação descrita com palavras diferentes geraria hashes distintos → gaming acidental (promove com evidência falsamente diversa). Ancorar no `task_kind` (já agrupado por retrieval) dá a unidade de situação certa. Rejeitado.
- **`belief_id` aleatório/UUID:** quebra idempotência sob replay (reprocessar duplica crença). Rejeitado — derivação determinística é requisito do event-sourcing (inv #4).

## Consequências

- **Abre:** `LP-CORE` implementa `housekeeping_tick` + a derivação + o casamento em `reflect_correction`; `LP-APP` fia o sombra carregando `top_k_for_role` no prompt.
- **Costura LOOP ↔ 0045:** um único `task_kind` serve de unidade de "situação" (Mentality) e de chave de agregação de outcome (skill routing). Evita dois vocabulários de contexto incompatíveis — **a porta que este ADR mantém aberta.**
- **Dependência de ordem:** o `task_kind` precisa estar disponível no momento do reforço. MVP: se o retrieval ainda não rodou no turno, `task_kind = role + ":" + termos_dominantes` derivado da própria query da tarefa corrente (mesma fórmula da spec 0045) — nunca vazio (vazio nunca promove, `mentality.rs:933`).
- **Porta que fecha se ignorado:** sem derivação determinística, o motor não roda em produção (só em teste); sem `belief_id` derivado, replay duplica crença.

## Verificação (observável)

- **Determinismo:** `situation_hash(role, tk)` e `belief_id(role, corr_id)` são puros — duas chamadas com os mesmos argumentos batem; replay do log reconstrói os mesmos ids (controle: reprocessar o mesmo `CorrectionObserved` 2× → 1 crença, não 2).
- **Anti-gaming:** mesma correção no mesmo `task_kind` 2× → 1 `situation_hash` → NÃO promove; em 2 `task_kind` distintos → 2 hashes → promove (default N=2) — exercita `mentality.rs` M-PROMO sem literais de teste.
- **Casamento re-validado:** um marcador `reforça #i` com `#i` fora de faixa OU de papel divergente cai em `nova` (não corrompe crença alheia) — prova por mutação (forjar índice inválido → degrada, não escala).
- **Snapshot congela a numeração (anti-race):** alterar a `Mentality(role)` (inserir/retirar crença) entre o `enqueue` e a re-validação NÃO muda a resolução de `#i` — o snapshot anexado ao request vence a projeção corrente (teste: enqueue → mutar a Mentality → re-validar → `#i` ainda resolve para o `belief_id` capturado no enqueue).
- **Segurança:** `reforça #i` não consegue apontar para `belief_id` inexistente (o sombra só vê índices; o core resolve contra a própria projeção) — mutação: injetar `belief_id` cru no texto não muda nada.
- **ZERO LLM (controle ±):** com o sombra mockado por um stub determinístico, todo o caminho casa/promove/recusa sem nenhuma chamada de modelo no core.
