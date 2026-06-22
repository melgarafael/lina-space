# Pesquisa de estado (read-only) — F3-0 + F3-CONF-2/3 · para consolidar no épico 39

> Levantado por Terminal B (Ultra Code) a pedido do Maestro (Terminal A). Evidência: commit + teste/código. Read-only — nenhum doc/épico editado.

## Onda F3-0 — Parâmetros & Cognição por Terminal

**Entregue no commit `cc452cf`** (`feat(f3): fundacao — params/effort por terminal (F3-0) + contrato Goal-and-Loop (F3-1 Ra-0)`). Plano: `tasks/epico-f3/onda-f3-0.md`. ADR-gate 0031 (model/effort no spawn) **aceito** — as stories que dependiam dele (F3-0-3/4/5/6) estão entregues.

| Story | Status | Evidência |
|---|---|---|
| **F3-0-1** SystemParams + enum Effort | ✅ FEITO | `crates/lina-core/src/params.rs` (`struct SystemParams` campos `Option<T>`+`serde(default)`, `overlay`, `resolve` com clamp); enum `Effort` em `events.rs`; 21 `#[test]` em params.rs (verdes na suíte do core) |
| **F3-0-2** RouterConfig por projeção (replay) | ✅ FEITO | `params.rs:513 resolve_from_store` + teste `resolve_from_store_reconstroi_fanout_gate_8_so_do_log` (params.rs:850) |
| **F3-0-3** model/effort no spawn (aditivo) | ✅ FEITO | `SpawnRequested` ganhou `model`/`effort` (events.rs:3844, ADR 0031) + teste `spawn_requested_model_effort_are_additive` (events.rs:3849) |
| **F3-0-4** EffortAssigned + ENV | ✅ FEITO | evento `EffortAssigned` em events.rs; ENV `LINA_EFFORT` injetado por-spawn em `bridge.rs:2886-2890` (espelha `LINA_NODE_ID`) |
| **F3-0-5** verbos CLI params/effort | ✅ FEITO | verbos `params`/`effort` no `lina.rs`; `handle_params`/`handle_effort` no router (gated server-side); `crates/lina-bootstrap/tests/params_cli.rs` (10 testes) |
| **F3-0-6** badge/sliders UI | ✅ FEITO | `effort` integrado na UI (agent_modal, dashboard, attention_ui, goal_card); badge **modelo·effort** reforçado na F3-CONF-2 (CONF-UI, commit `5cf47ad`) |
| **F3-0-7** clamp + suíte adversarial | ✅ FEITO *(com nota)* | clamp/WARN em `params.rs` (`ParamWarning`, coberto nos 21 testes); adversarial server-side + GatedHard em `crates/lina-core/tests/f3_conf2_seguranca.rs` (afrouxar breaker→`GatedHard`:366; carimbo server-side ignora payload de agente). **Nota:** a "suíte adversarial" F3-0-7 ficou como SPEC `tests/f3_0_params_adversarial.md` + testes **distribuídos** (params.rs + f3_conf2_seguranca.rs) — não existe um `.rs` dedicado `f3_0_params_adversarial.rs`. A cobertura existe; o arquivo único não. |

**Veredito F3-0:** ✅ **FECHADA** (7/7 entregues). Única ressalva: F3-0-7 sem arquivo `.rs` dedicado (cobertura distribuída — decidir se vale consolidar).

## Rodada F3-CONF-2 — Confiabilidade da Orquestração v2 (Protocolo Anti-Engolimento)

**Status: FECHADA**, gate de onda **PASS** (commit `22095c8`). Plano: `onda-f3-conf-2.md`. Atacou o gargalo nº1 do dogfooding (#22/#23: *handoff confirmado ≠ trabalho feito*, 5 ocorrências incl. uso real com cliente).

**O que entregou:** mecanizou os passos 6-7 do loop do Maestro (monitorar→corrigir) como evento — re-despacho 1×→breaker sticky→`GoalEscalated{stalled}`, vigia delta O(1) (sem full-replay); `circuit_breaker` legível na tela + badge modelo·effort; `reason` do breaker flui core→host→UI (#21); botão "liberar"↔`reset_breaker`; CONF-HARNESS matou o flaky sistêmico `history_f1_5_8`; CONF-QA segurança por mutação verde.
**Commits:** `26d68a8` (contrato reason), `c113f36` (CORE-LOOP, B), `5cf47ad` (CONF-UI, G), `81f0422`+`1b4042b` (CONF-QA, R), `eb75e1e` (HARNESS, I), `a5e3041` (integração liberar↔reset, G).

## Rodada F3-CONF-3 — "O Maestro Enxerga o Time" (Observabilidade & Adoção)

**Status: FECHADA** (commit `d9dfd29`). Plano: `onda-f3-conf-3.md`. Rodada de consolidação entre F3-CONF-2 e F3-3 Mentality; atacou o top-risco §VI-3 do épico (*adoção 0% — Goal/effort/Tradutor construídos e nunca acionados*).

**O que entregou:** **R1+R3** (commit `d5977bf`, H+R) — #27 `lina history` aceita o `n-<uuid>` que o spawn carimba (o Maestro vê o worker no caminho real, não só no disco); #23c/#14 `lina list` vivo-vence-morto (não re-despacha quem está trabalhando); #21 `lina vault search` com timeout/índice/flush (deixa de pendurar em vault grande). **R2** (commit `5fece5c`, J) — bin auto-descoberto `intelligence_adoption` (read-only) que mede a adoção real da inteligência F3 no log (Goal/Tradutor/effort + slot da sentinela `[LINA::CORRECTION]`).

## Resumo de uma linha (para a tabela do épico)

- **F3-0** ✅ FECHADA (`cc452cf`) — params/effort por terminal, 7/7 stories; ressalva só na consolidação de testes da F3-0-7.
- **F3-CONF-2** ✅ FECHADA (`22095c8`) — protocolo anti-engolimento mecanizado + breaker honesto na tela.
- **F3-CONF-3** ✅ FECHADA (`d9dfd29`) — Maestro enxerga o time (history/list/vault) + métrica de adoção da inteligência F3.
