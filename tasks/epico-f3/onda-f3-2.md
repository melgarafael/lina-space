# Onda F3-2 — O Loop do Maestro Nativo + papel Tradutor · plano de execução

> **Peça-fonte story-a-story** da onda F3-2 do épico `39 - Epico Fase 3 — O Maestro Nativo` (vault). Em conflito, esta peça vence o épico; o ADR aceito vence ambos.
> **Fonte da verdade detalhada:** `52 - SPEC Goal-and-Loop` (vault) §3 (loop), §4 (model/effort), §Superfície, §"Portas que NÃO fecha" (Tradutor); doc-fonte linhas 10-12, 83; `10 - Arquitetura e Runtime do Agente` §6.1/§6.2 (briefing em camadas). **Todo worker LÊ a seção citada da spec antes de codar.**
> **Maestro:** Terminal A (ARQUITETO). **Workers não commitam; o Maestro fixa o contrato, valida de fora (exit codes diretos) e commita por fatia.**

## Objetivo da onda

Costurar o **loop completo do doc-fonte (linhas 10-12)** sobre as primitivas já entregues em F3-0/F3-1: o Maestro **interpreta** o pedido leigo → **devolve a interpretação** → espera **confirmação** → monta o **time com model/effort por papel** → despacha com prompt impecável → recebe o trabalho → roda o **juiz** → devolve a **tratativa no FAIL** → **escala effort** se o dev não consegue → entrega → **narra ao leigo**. Cria o papel **Tradutor** (doc-fonte 83) — o gap nomeado da spec 52. É o ponto **[6] Fluxo de informação** fechando o circuito de volta ao humano, sobre o termostato **[8]** de F3-1.

O Lina implementa **só o OUTER loop** (observa idle, roda o juiz, re-injeta ou fecha); o INNER loop é o CLI de terceiro (**invariante #1: zero LLM no core**).

## Estado real — o que F3-1 JÁ ENTREGOU (não re-implementar)

A rodada F3-1 (α, 11 commits, gate PASS) adiantou a maior parte do **termostato**. Confirmado por re-derivação no fonte:

| Já pronto (reusar) | Onde |
|---|---|
| Projeção `Goal`/`GoalPhase`/`project_goals` (replay) | `crates/lina-core/src/goal.rs:19,38,62` |
| OUTER loop (`review_and_advance`, juiz estrutural ZERO LLM, `reviewer != target`) | `router.rs:2132,2869,2887` |
| `ReviewVerdict` com `evidence`/`defect_class`/`iteration` | `events.rs:667` |
| Re-despacho informado no FAIL (`escalate_on_fail` monta prompt com evidência) | `router.rs:2207,2234` |
| Escalonamento de effort (`effort_ladder`, `Effort: Ord`) | `router.rs:66,2217`; `events.rs:90` |
| Turn-budget + `GoalEscalated{turn_budget_exhausted}` | `router.rs:56,2179` |
| Handlers `handle_goal`/`handle_plan`/`handle_plan_seed`/`human_intent`/`handle_spawn` | `router.rs:2452,1858,2055,2433,2580` |
| `SpawnRequested` com `model`/`effort`/`goal_id` | `events.rs:963,990` |
| Verbos `lina goal define/interpret/confirm/status` + `plan add/seed` | `lina.rs` |
| Card da Goal + botões (confirm/ajustar/recolher/arrastar/fechar) | `app/lina-gpui/src/goal_card.rs`, `main.rs` |

**O gap REAL de F3-2** (o que esta onda constrói):

1. **Papel Tradutor** — não existe no role-discovery (`default-roles.yaml` tem 12 papéis + DEVELOPER fallback; nenhum Tradutor). Os doc-comments já reservam `@Tradutor` em `events.rs:591,597`.
2. **Loop interpretar→propor→confirmar/corrigir formalizado** — hoje `GoalInterpreted` (sugere) + `GoalConfirmed` (gate) + re-`interpret` (corrige) existem isolados; falta o ciclo amarrado e a garantia testável de "ZERO decomposição/spawn antes do `GoalConfirmed`".
3. **Montagem do time INICIAL por `proposed_team`** — hoje `proposed_team` é gravado em `GoalInterpreted` (`router.rs:2501,2531`) mas **nada o consome** para spawnar o time. (No FAIL, `escalate_on_fail` já spawna — falta a montagem da PRIMEIRA leva.)
4. **Escalonamento "novo dev" vs "re-despacho ao mesmo"** — `escalate_on_fail` já re-spawna com effort maior; falta distinguir explicitamente "re-despacha ao MESMO" (N < teto do degrau) de "spawna um NOVO dev" (atinge o teto) + breaker sticky anti-re-spawn-idêntico.
5. **Briefing em camadas** (stable/context/volatile) + verify-loop no boot — inexistente no runtime (só há o bootstrap de turno-0 `whoami`). Trabalho novo.
6. **Observabilidade do Maestro (achado dogfooding #15):** `lina history` existe no core (`history.rs` + `HistoryRead` + leitura cross-terminal por pertencimento) mas **não é verbo exposto no bin** — o Maestro não consegue "ver a tela do worker" para monitorar trajeto (passo 6 do loop). Fiar isto é [6] Fluxo de informação puro.

## ⛔ Decisão de arquiteto (Rα-0) — a registrar no kickoff

**`InterpretationProposed`/`InterpretationConfirmed` NÃO viram eventos novos.** A spec 52 §198 crava *"InterpretationConfirmed (= GoalConfirmed quando há Goal)"* e modela todo o gate como `GoalConfirmed`. `GoalInterpreted` (proposta) + `GoalConfirmed` (confirmação) + re-`interpret` (correção) **já realizam** o loop de devolver-o-entendimento. Criar variantes paralelas seria **duplicação de contrato** (lina-code-doctrine: duplicação é dívida) sem caso de uso real — o caso "pré-Goal / gerar N Goals" é porta aberta F3+ (spec 52 §"Portas que NÃO fecha", interpretação automática profunda). **F3-2-2 reusa os eventos de Goal**; o que adiciona é o ciclo de correção formalizado + a garantia testável do gate. *Levar ao fundador no kickoff como ponto de decisão; reusar não fecha porta, então nota na peça basta (sem ADR pesado) salvo objeção.*

## A jogada de arquiteto: CONTRATO primeiro, depois fan-out

A costura crítica é `router.rs` (OUTER loop) — **dono único = CORE (B/Ultra)**. O contrato em `events.rs` é fixado pelo Maestro no **Rα-0** (variantes/campos novos + stubs compilando); commitado o contrato, as frentes codam contra arquivos **disjuntos**.

**Rα-0 (Maestro/A, effort High) — setup + contrato:**
1. **Limpar a árvore:** validar e commitar o fix de freeze do render (`try_with_store` não-bloqueante em `bridge.rs`/`main.rs`, JÁ não-commitado — compila + `try_with_store_is_nonblocking_under_contention` verde). Continuação do 2º freeze do `lina ask` (memória). É pré-requisito da rodada multi-terminal (working tree compartilhada).
2. **Fixar o contrato** dos campos novos que as frentes consomem (sem eventos `Interpretation*` — ver decisão acima): se F3-2-3 precisar de campo aditivo em algum evento, fixá-lo `#[serde(default)]` aqui com aplicador stub. Replay F0/F1/F2 carrega sem erro.
3. **Commit do contrato = sinal de largada.**

## DAG executável (deps) e frentes de paralelização

```
[Rα-0: fix-freeze commitado + contrato fixo (Maestro/A)]
        │
        ├─► CORE     (B·Ultra)  router.rs + goal.rs ──┐  F3-2-2 · F3-2-3 · F3-2-5 (+integra F3-2-1 origin)
        ├─► TRADUTOR (H)        role-discovery+yaml ──┤  F3-2-1
        ├─► BRIEFING (I)        briefing.rs + lina.rs ─┤  F3-2-6 + #15 (lina history)
        ├─► UI       (G)        app/lina-gpui ─────────┤  F3-2-7 (parte visual)
        └─► QA       (R)        tests/ + investigação ─┘  F3-2 segurança+adoção + repro #22/#23 (read-only)
                │
                ▼
   Maestro (A): integração e2e + gate de onda (4 lentes/cold-review) + commit por fatia + roteiro da tela do fundador
```

| Frente | Stories | Toca (dono) | Por que não colide |
|---|---|---|---|
| **CORE** (B · Ultra Code) | F3-2-2 (loop interpretar→confirmar) · F3-2-3 (montar time por `proposed_team`) · F3-2-5 (novo-dev + breaker sticky) | `router.rs` + `goal.rs` | dono único de `router.rs`/`goal.rs` — events.rs já fixo no Rα-0 |
| **TRADUTOR** (H) | F3-2-1 (papel Tradutor no role-discovery + skill + degradação ao Maestro) | `crates/lina-role-discovery/src/default-roles.yaml` + crate role-discovery + `assets/` skill | disjunto; o **roteamento** da 1ª msg ao Tradutor é costura de `router.rs` → fica com CORE (B integra o `origin`) |
| **BRIEFING** (I) | F3-2-6 (briefing em camadas — módulo puro) · #15 (`lina history` verbo + doutrina) | `crates/lina-core/src/briefing.rs` **(novo, função pura)** + `crates/lina-bootstrap/src/bin/lina.rs` (verbo) | módulo pub de lib (sem chamador interno é OK em lib) + verbo CLI — disjuntos de router.rs |
| **UI** (G) | F3-2-7 parte visual (interpretação proposta→confirmar/corrigir; estado do loop por iteração; escalada narrada) | `app/lina-gpui/src/` (goal_card/canvas) | projeção, sem costura de core |
| **QA** (R) | F3-2 segurança (interpretation/proposed_team/by/reviewer nunca autoridade; `reviewer != target` no caminho novo) · métrica de adoção · **investigação read-only da família #22/#23** | `crates/**/tests/` + `#[cfg(test)]`; relatório em `tasks/` | lê, não altera produção |

> **Regra de costura (protocolo F0/F1):** `events.rs` fixado no Rα-0, depois ninguém toca. `router.rs`+`goal.rs`=CORE, `briefing.rs`+`lina.rs`=BRIEFING, `default-roles.yaml`=TRADUTOR são donos únicos. Precisa de algo numa costura de outro dono? **Peça ao Maestro.** `cargo fmt -p` só nos próprios arquivos (memória: *fmt em árvore compartilhada*).

## Atribuição modelo·effort (dogfooding da própria F3-0)

| Frente | Terminal | Dificuldade | model · effort |
|---|---|---|---|
| Contrato/integração | A (Maestro) | tipos + costura e2e | **opus · High** |
| CORE | B · Ultra Code | loop + montagem de time + escalonamento — o coração | **opus · High (Ultra)** |
| TRADUTOR | H | papel + degradação (rótulo de proveniência, não credencial) | opus · Medium |
| BRIEFING | I | módulo de camadas + verbo de leitura cross-terminal | opus · Medium |
| UI | G | render do loop + naming pt-br do leigo | opus · Medium |
| QA/Red-team | R | red-team de "interpretação não é autoridade" + repro de bug intermitente | **opus · High** |

## GATE DE SAÍDA F3-2 (roda e se mede — do épico §V)

(a) **Tradutor intercepta:** Espaço com Tradutor no roster → 1ª mensagem do leigo roteia primeiro a ele (`GoalInterpreted{origin:"@Tradutor"}`); **sem** Tradutor → degrada ao Maestro (rótulo de proveniência, nunca credencial). Provado por teste de role-discovery + roteamento.
(b) **Gate antes de agir:** nenhuma `GoalDecomposed`/`SpawnRequested` de membro do time aparece no log **antes** do `GoalConfirmed` (teste do ciclo).
(c) **Time montado por effort:** após `GoalConfirmed`, a montagem do `proposed_team` emite um `SpawnRequested` por papel **com `effort` no envelope** (resolvido contra SystemParams); ENV `LINA_EFFORT` no PTY de cada filho.
(d) **Correção informada:** `ReviewVerdict{Fail}` → re-despacho ao MESMO dev com `evidence` não-vazio; ao esgotar o degrau → **novo** `SpawnRequested` com `effort` **estritamente maior** (`Ord`) + prompt com "tentativas anteriores"; **breaker sticky** impede re-spawn idêntico.
(e) **Briefing em camadas:** o briefing de um worker é montado em stable/context/volatile, **gated por papel** (FRONTEND não recebe protocolo de orquestração) — provado por teste de unidade do módulo puro.
(f) **Observabilidade do Maestro (#15):** `lina history @worker` devolve o último estado/saída do colega (leitura cross-terminal por pertencimento, ADR 0006) — o Maestro vê a tela sem perguntar ao humano.
(g) **Segurança:** suíte do Router verde; `interpretation`/`strategy`/`proposed_team`/`evidence` provados como DADO (um `ReviewVerdict{Pass}` não libera `GatedHard`; `by`/`reviewer`/`origin` carimbados server-side; `GoalConfirmed` forjado ignorado).
**(h) Validação do fundador na tela (cenário real F3-2-7) é parte do gate — BLOQUEANTE e DIFERIDA** (precisa do Lina.app rebuildado + olho do fundador; preparada nesta rodada, fechada com ele).

## Conselho de gate de onda (4 lentes, read-only, não construiu)

(1) Visão/fios; (2) Arquitetura (ADR 0033 ↔ código + invariantes 1-7 + **risco #1: zero "motor de avaliação" que não seja `CheckKind` ou nó QA; zero LLM no core**); (3) Segurança (red-team: **0 ALTA para seguir**; interpretação/proposed_team/effort provados como dado, nunca autoridade); (4) Specs (spot-check spec 52 §3/§4 vs implementado). A tela (h) é pendência do fundador.

## Riscos da onda

- **Recriar um harness** (motor de avaliação próprio / LLM no core) → inv #1. *Mitigação:* juiz é `CheckKind` ou nó QA; o core só observa/nomeia/dispara.
- **Tradutor virar gargalo único** → degrada ao Maestro sem Tradutor (rótulo, não credencial).
- **Escalonamento explodir custo** → teto de `effort_ladder` + breaker sticky + cost ceiling F1.
- **Adoção 0%** → métrica de uso de `lina goal`/Tradutor no log desde o dia 1.
- **🔴 Risco operacional nº1 — família dogfooding #22/#23 (handoff confirmado ≠ trabalho feito; 5 ocorrências).** Despachar a 5 workers pode "engolir" despachos. *Mitigação do Maestro:* confirmar o **1º evento de progresso** de cada worker (não o "ok" do verbo); re-despacho informado 1×; breaker sticky após 2 falhas → escala ao fundador. **O FIX do #22/#23 NÃO entra nesta rodada** (toca `router.rs`/`mailbox.rs` → colidiria com o CORE) — R produz repro+relatório read-only; o fix vira rodada dedicada.

## Pendências de rodadas anteriores

- **Embutida nesta rodada:** #15 (observabilidade do Maestro — `lina history` verbo).
- **Investigada (read-only) nesta rodada, fix diferido:** família #22/#23 (gargalo de confiabilidade da orquestração).
- **Diferida com porta:** F3-1 gate (h) tela do fundador + F3-2-7 (cenário real) → fecham juntas na validação ao vivo com o fundador.
- **Fora do escopo (outras ondas):** #25 ENOSPC → F3-5-8; #17/#17b identidade por cwd no spawn/ask → candidata; F3-0-6 badge UI / F3-0-7 Teste C → encaixe oportunista se houver folga em UI/QA.

---

## STATUS DA EXECUÇÃO — (a preencher pelo Maestro durante a rodada)
