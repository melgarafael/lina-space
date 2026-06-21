# Despacho F3-VAL · R (QA/red-team) — prova REPETÍVEL dos freios do loop (caminho real)

**LEIA ANTES (puxe o contexto):**
- `tasks/epico-f3/onda-f3-val.md` (plano desta rodada — estado, estratégia em camadas, gate de saída).
- Spec 52 §3.2 (turn-budget) e §Aceite (gates b/c/d/e). O `lina vault index` está **stale** para a nota 52 — leia pelo path direto: `/Users/rafaelmelgaco/Documents/Obsidian Vault/Ecossistema Labs - Operação/Debriefing Vibe Coding/52 - SPEC Goal-and-Loop - A Meta como Primitiva.md`.
- Código do loop+juiz: `crates/lina-core/src/router.rs` → `GOAL_MAX_ITERATIONS=3` (:56), `EFFORT_LADDER=[Medium,High]` (:66), `escalate_on_fail` (:2301-2348), juiz estrutural `STRUCTURAL_JUDGE` (:2227), `handle_goal` (:2802). Suíte adversarial existente: `crates/lina-core/tests/` (commits `4d658ec`, `260aa8a`).

## 1. CONTEXTO
F3-1/F3-2 têm o **código fechado e verde** (480/413 testes), mas o gate (h) ao vivo está aberto. O Maestro já provou ao vivo o **ciclo** F3-1 (define→interpret→confirm→seed→status) e o **replay**. Falta a prova **repetível, no caminho do código real**, dos **FREIOS** — o termostato de qualidade. Hoje os freios estão cobertos por testes pontuais; quero um **teste de ACEITE e2e nomeado** que encene o cenário completo de ponta a ponta e assira a sequência **exata** de eventos.

## 2. FUNÇÃO
Você é o **QA/red-team**, dono de `crates/**/tests/`. Escreve teste de aceite; **não** altera produção.

## 3. DIRECIONAMENTO (fronteira + regras)
- Mexa **SÓ** em `crates/lina-core/tests/` (arquivo novo, ex.: `goal_loop_aceite_f3val.rs`) + `#[cfg(test)]`. **NÃO** toque `router.rs`/`goal.rs`/`events.rs` (produção — donos B/A).
- **Dirija o CAMINHO REAL**: encene via `route_message`/`handle_goal` + o loop OUTER, como os testes de integração existentes. **NÃO** monte eventos à mão com `store.append` pulando o handler (lição paga: "teste à-mão não prova a costura intent→handler"). O veredito tem que nascer do juiz real.
- Juiz = `CheckKind::Command` (exit code) — **ZERO LLM** no core (inv #1).
- `cargo fmt` toca **só** o seu arquivo novo; `clippy --all-targets -D warnings` limpo.

## 4. OBJETIVO
Provar, de forma que rode no CI e não dependa de olho humano, que o loop-até-aceite **freia exatamente em N=3** e **escala o effort** — é o que separa "executar pedido" de "perseguir meta" (Meadows [8] feedback negativo).

## 5. RESULTADO ESPERADO (formato exato)
Arquivo de teste novo cobrindo, cada um por evento no log reconstruído:
- **(c) turn-budget:** acceptance `Command` que SEMPRE falha (exit 1) → **exatamente 3** `ReviewVerdict{Fail}` → `GoalEscalated{reason:"turn_budget_exhausted"}` → **zero** iteração/despacho depois.
- **(b) caminho feliz:** acceptance `Command` exit 0 → ao Pass de todos os itens → **exatamente 1** `GoalAchieved` → nada depois.
- **(d) juiz≠executor:** todo `ReviewVerdict` tem `reviewer != target`; veredito forjado com `reviewer==target` é **recusado**.
- **(e) escala de effort:** re-spawn no Fail carrega `effort` **estritamente maior** por `Ord` no log (degrau 0=`Medium` → `High`); **breaker sticky** impede re-spawn idêntico (mesma tarefa+effort) — morde já na 2ª falha do topo.
- Rode `cargo test -p lina-core <arquivo>` e confirme **exit 0** (leia o código de saída direto, sem pipe que mascara).

Termine com **`PRONTO: <nº de testes verdes + 1 linha por gate provado>`** ou **`BLOCKED: <motivo + arquivo:linha>`**.
