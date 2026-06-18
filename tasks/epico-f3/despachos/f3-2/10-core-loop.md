# Despacho F3-2 · CORE — o loop do Maestro nativo (router.rs + goal.rs)
**Para:** Terminal B (BACKEND · Ultra Code) · **model·effort:** opus · High (Ultra) · **Dono de:** `crates/lina-core/src/router.rs` + `crates/lina-core/src/goal.rs`

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (caminho absoluto — não confie em relativo).
- **LEIA primeiro, nesta ordem:**
  1. `tasks/epico-f3/onda-f3-2.md` (a peça da onda — estado real + decisão de arquiteto + fronteiras + gate).
  2. Spec 52 no vault: `lina vault read` falha às vezes (achado #21) — use direto: `/Users/rafaelmelgaco/Documents/Obsidian Vault/Ecossistema Labs - Operação/Debriefing Vibe Coding/52 - SPEC Goal-and-Loop - A Meta como Primitiva.md` §3 (linhas 230-280, o laço calibrado), §4 (282-301, model/effort).
  3. O código que VOCÊ vai estender (NÃO re-implementar — já existe):
     - `goal.rs:19` (`GoalPhase`: Defined/Interpreted/Confirmed/Decomposed/InLoop/Achieved/Escalated), `:38` (`Goal`), `:62` (`project_goals`).
     - `router.rs:2452` (`handle_goal` — `goal.interpret` grava `proposed_team` em `:2501,2531`), `:2055` (`handle_plan_seed` — exige `Confirmed`), `:2132` (`review_and_advance` OUTER loop), `:2207` (`escalate_on_fail` — já re-spawna com effort maior), `:2580` (`handle_spawn`).
- **Contrato events.rs JÁ FIXO pelo Maestro (Rα-0)** — não toque `events.rs`. `SpawnRequested` já tem `model`/`effort`/`goal_id` (`events.rs:990`); `Effort` deriva `Ord` (`events.rs:90`).
- **Decisão de arquiteto (vinculante):** **NÃO crie `InterpretationProposed`/`InterpretationConfirmed`.** Reuse `GoalInterpreted` (proposta) + `GoalConfirmed` (gate) + re-`interpret` (correção). Ver onda-f3-2.md §"Decisão de arquiteto".

## FUNÇÃO
Você é o **dono do coração do loop**: a montagem do time a partir da interpretação confirmada, a formalização do ciclo interpretar→confirmar, e a distinção re-despacho-ao-mesmo vs novo-dev no escalonamento. Você é o único que toca `router.rs`/`goal.rs` nesta rodada.

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `crates/lina-core/src/router.rs` e `crates/lina-core/src/goal.rs`. Precisa de algo em `events.rs`/`plan.rs`/outra costura? **Peça ao Maestro (Terminal A)** via `lina ask "@Terminal A"` — nunca edite costura de peer.
- **Invariante #1 — ZERO LLM no core.** O juiz é `CheckKind` determinístico OU nó QA distinto. Você só observa/nomeia/dispara. Nenhum "motor de avaliação" novo.
- **Regra-mãe de segurança:** `interpretation`/`strategy`/`proposed_team`/`evidence` são **DADO transportado, JAMAIS autoridade**. `by`/`reviewer`/`origin` carimbados server-side (sender autenticado), nunca do payload. Um `ReviewVerdict{Pass}` não libera `GatedHard` (`guard.rs:209` soberano).
- **Aditividade:** replay de log F0/F1/F2 nunca quebra. Eventos são fato — config decide a próxima ação, não reescreve o passado.
- Convenções: `cargo fmt -p lina-core` (só seus arquivos), `clippy -D warnings` limpo, **sem `unwrap()` em produção**, testes que provam o critério **via `route_message`** (caminho real — não monte o evento à mão; lição registrada: teste à-mão esconde costura quebrada).

## OBJETIVO (o porquê de negócio)
O Lina deixa de executar pedidos e passa a perseguir metas: depois que o humano confirma o entendimento, o sistema **monta o time certo com o motor certo por papel** e, quando um dev falha, devolve a tratativa e — se persistir — chama um dev mais reforçado. É o método do fundador (doc-fonte 12) virando mecanismo.

## ESCOPO — 3 stories
- **F3-2-2 (loop interpretar→confirmar):** amarre o ciclo sobre os eventos existentes. Garanta, com teste, que **nenhuma `GoalDecomposed`/`SpawnRequested` de membro do time ocorre antes do `GoalConfirmed`** (gate). O "Quero ajustar" da UI re-envia `goal.interpret` → a projeção reflete a ÚLTIMA interpretação (corrige). Respeite o gate por autonomia (manual/assistido/autonomo) no confirm (spec 52 §327).
- **F3-2-3 (montar time por `proposed_team`):** ao `GoalConfirmed` (ou via verbo dedicado, alinhe com o Maestro), consuma o `proposed_team` do `GoalInterpreted` confirmado e emita **um `SpawnRequested` por papel** com `effort` resolvido contra os SystemParams (F3-0) e `goal_id` preenchido. Cascata sempre passa por aval humano (`SpawnGated{cascade}`, ADR 0019). **Mede-se:** 1 `SpawnRequested`/papel com `effort` no envelope.
- **F3-2-5 (novo-dev + breaker sticky):** no `escalate_on_fail`, distinga explicitamente: **N < teto do degrau** → re-despacho ao MESMO `target` (já faz); **atinge o teto** → **novo** `SpawnRequested` (dev novo) com `effort` estritamente maior + prompt carregando "tentativas anteriores". **Breaker sticky:** impeça re-spawn idêntico (mesma tarefa+effort) — após 2 falhas consecutivas do mesmo item, pare de auto-escalar (deixa para o `turn_budget_exhausted`/gate humano).

## RESULTADO ESPERADO (formato exato)
- Diffs em `router.rs`/`goal.rs` apenas; `events.rs` intocado.
- Testes novos via `route_message` provando: (b) zero decomposição/spawn antes do Confirmed; (c) N spawns com effort por papel; (d) re-mesmo vs novo-dev com effort `Ord` crescente + breaker sticky.
- `cargo test -p lina-core` verde (rode e cole a contagem); `clippy -D` 0; `fmt` limpo (só seus arquivos).
- **NÃO commite** (o Maestro commita por fatia). Reporte o 1º progresso assim que tocar o código (`lina ask "@Terminal A" "comecei o CORE F3-2" --intent status`).
- Termine com **`PRONTO: <resumo + contagem de testes + arquivos tocados>`** ou **`BLOCKED: <motivo + o que precisa do Maestro>`**.
