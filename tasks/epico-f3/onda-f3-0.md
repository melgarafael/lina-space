# Onda F3-0 — Parâmetros & Cognição por Terminal · plano de execução

> **Peça-fonte story-a-story** da onda F3-0 do épico `39 - Epico Fase 3 — O Maestro Nativo` (vault). Em conflito, esta peça vence o épico; o ADR aceito vence ambos.
> **Fonte da verdade detalhada:** `51 - SPEC SystemParams` (vault) — schema, segurança, critérios integrais.
> **Maestro:** Terminal A (ARQUITETO). **Workers não commitam; o Maestro valida de fora e commita por fatia.**

## Objetivo da onda

Dar **casa estruturada e versionada** aos ~15 números que governam a orquestração (hoje 13 caem no `..RouterConfig::default()` de `app/lina-gpui/src/bridge.rs:717` — valor morto que parece configurável) **e criar o que não existe**: `model`/`effort` por terminal e por tarefa — a única etapa do loop do Maestro (doc-fonte linha 12/58) sem nenhuma fundação no código. Entrega o **knob**; o laço que o gira é F3-1/F3-2. Ponto **[12] Parâmetros** de Meadows (o mais fraco da escala), amarrado aos pontos altos que empurra ([7]/[8]/[9]).

## ⛔ Gate de entrada — ADR 0031 ANTES de codar o spawn

**`docs/adr/0031-model-effort-no-spawn.md` está `Proposto`.** As stories **F3-0-3, F3-0-4, F3-0-5, F3-0-6** tocam o envelope de spawn / a seleção de effort e **NÃO INICIAM** sem o ADR 0031 **aceito** pelo fundador. F3-0-1, F3-0-2 e F3-0-7 (estrutura/projeção/clamp) **podem começar antes** — não dependem da decisão de spawn.

**1ª ação do Maestro:** levar o ADR 0031 ao fundador (decisão sim/não em 1 frase: *"posso atribuir modelo e nível de raciocínio a cada terminal por tarefa, gravado como evento e nunca como autoridade de segurança?"*).

## DAG executável (deps) e rodadas de paralelização

```
F3-0-1 (SystemParams + Effort)  ─┬─► F3-0-2 (RouterConfig→projeção) ─┬─► F3-0-5 (verbos CLI)
                                 │                                   │
                                 └─► F3-0-3 (model/effort no spawn) ─┴─► F3-0-4 (EffortAssigned+ENV) ─► F3-0-6 (badge/sliders UI)
                                                                          │
F3-0-2 ∧ F3-0-4 ───────────────────────────────────────────────────────► F3-0-7 (clamp + suíte adversarial)
```

| Rodada | Stories em paralelo | Por quê não colidem |
|---|---|---|
| **R1** (pode começar já; ADR não bloqueia) | **F3-0-1** sozinha | cria os tipos (`SystemParams`, enum `Effort`) — raiz de tudo; dono único de `events.rs`/novo `params.rs` |
| **R2** (após R1) | **F3-0-2** *(livre — não depende do ADR)* → **F3-0-3** *(só com ADR 0031 aceito)*, sequenciais, mesmo dono | ambas tocam `router.rs`+`events.rs` (costuras) — **dono único por rodada**: Dev Core faz as duas em sequência, não em paralelo |
| **R3** (após F3-0-3/0-2) | **F3-0-4** (Dev Core: `bridge.rs`) ∥ **F3-0-5** (LLM Eng: `lina.rs`) | fronteiras disjuntas: ENV-injection na shell × verbos no binário CLI |
| **R4** (após F3-0-4) | **F3-0-6** (Frontend: render do card) ∥ **F3-0-7** (QA: arquivos de teste) | UI × testes — disjuntos |

## Fronteiras de arquivo por story (território disjunto)

| Story | Toca (dono) | Costura (dono único/rodada) |
|---|---|---|
| F3-0-1 | `crates/lina-core/src/params.rs` **(novo)**; enum `Effort` em `events.rs` | `events.rs` (R1: F3-0-1) |
| F3-0-2 | `params.rs`; `RouterConfig` em `router.rs:71`/`:133`; seam `app/lina-gpui/src/bridge.rs:714-718` | `router.rs` + `bridge.rs` (R2: Dev Core) |
| F3-0-3 | `SpawnRequested` `events.rs:712`; `SpawnApproved` `router.rs:228`/`:1946`; `handle_spawn` `router.rs:1833`; `app/lina-gpui/src/agent_modal.rs` | `router.rs` + `events.rs` (R2: Dev Core) |
| F3-0-4 | `EffortAssigned` (novo, `events.rs`); ENV-carimbo em `node_identity_env` (`app/lina-gpui/src/bridge.rs:2614`, invocada pelo funil `admit_node`), onde já entram `LINA_NODE_ID` (`:2632`)/`LINA_AUTONOMY` (`:2645`) | `bridge.rs` (R3: Dev Core) |
| F3-0-5 | `crates/lina-bootstrap/src/bin/lina.rs` (verbos `params`/`effort`); roteamento no supervisor (envelope, como `run_plan_intent`) | `lina.rs` (R3: LLM Eng) |
| F3-0-6 | render do card + preset/sliders em `app/lina-gpui/src/` (Frontend) | — (sem costura de core) |
| F3-0-7 | testes em `crates/lina-core/tests/` + `#[cfg(test)]` de `router.rs`/`params.rs` | lê, não altera lógica de produção |

> **Regra de costura (protocolo F0/F1):** `events.rs`, `router.rs`, `bridge.rs`, `plan.rs`, `lib.rs` têm **dono único por rodada**. Precisa de algo numa costura de outro dono? Peça ao Maestro — nunca edite/reverta linha de peer. `cargo fmt -p` toca só os próprios arquivos (memória: fmt em árvore compartilhada).

## Atribuição de modelo·effort (dogfooding da própria feature)

O Maestro casa dificuldade↔effort exatamente como a F3-0 vai automatizar:

| Story | Dificuldade | effort | Razão |
|---|---|---|---|
| F3-0-1 | arquitetura de API (overlay/resolve, compat) | **High** | decisão de tipos que tudo herda — errar aqui custa caro |
| F3-0-2 | projeção event-sourced (padrão CostLedger) | Medium | padrão já existe no código |
| F3-0-3 | campos aditivos no spawn | Medium | mecânico, com cuidado de compat |
| F3-0-4 | evento + ENV injection | Medium | espelha `LINA_NODE_ID` |
| F3-0-5 | verbos CLI gateados | Medium | padrão `run_plan_intent` |
| F3-0-6 | UI (badge + sliders pt-br) | Medium | render + naming |
| F3-0-7 | suíte adversarial de segurança | **High** | red-team de carimbo server-side — sutil |

## Despachos prontos (template `lina-dispatch`) — R1 e início de R2

> Fire-and-forget; acompanhar com `lina check`. Disparar **F3-0-1 já** (não depende do ADR); preparar F3-0-7 em paralelo (clamp não depende de spawn). F3-0-3 só após ADR 0031 aceito.

**→ F3-0-1 (Dev Core, effort High):**
> CONTEXTO: Onda F3-0 do épico 39; spec-fonte `51 - SPEC SystemParams` (vault). Hoje os parâmetros de orquestração são `pub const` + `RouterConfig` (`router.rs:71`), e 13 dos 15 caem no `..default()` de `bridge.rs:717`. FUNÇÃO: Dev Core. DIRECIONAMENTO: criar `crates/lina-core/src/params.rs` com `struct SystemParams` (todo campo `Option<T>` + `#[serde(default)]`), `overlay(self, over)` (fusão por `or`, over ganha) e `resolve(global, workspace, preset, terminal, autonomy) -> RouterConfig` com clamp de faixa; criar enum `Effort{Low,Medium,High}` (contrato neutro, mapeamento é do CLI Profile — invariante #3). Sem tocar `router.rs`/`bridge.rs` ainda (é da F3-0-2). OBJETIVO: a casa de tipos dos parâmetros, compatível com os defaults atuais. RESULTADO ESPERADO: `cargo test -p lina-core` verde incl. teste "4 camadas, só terminal opina fanout_gate → resolve usa o de terminal" e "sem camada → defaults atuais"; `cargo clippy -p lina-core -D warnings` limpo. Marcar `PRONTO:`/`BLOCKED:`.

**→ F3-0-7 (QA/Red-team, effort High) — pode adiantar a parte de clamp:**
> CONTEXTO: idem; §Segurança da spec 51. FUNÇÃO: QA/Red-team. DIRECIONAMENTO: escrever a suíte adversarial: (1) `.lina/params.json` com `delegation_budget=9999` → clamp para o teto + 1 WARN na fila, **zero panic**; (2) `SystemParamsChanged`/`EffortAssigned` com `by`/`task_difficulty` injetado no payload **não** altera o carimbo efetivo (server-side ignora campo de agente — família ADR 0007); (3) reduzir `delegation_budget`/`breaker_threshold` abaixo do default é `GatedHard`. OBJETIVO: provar que parâmetro nunca vira autoridade nem derruba o processo. RESULTADO ESPERADO: testes que **falham contra o código atual** (TDD) e passam após F3-0-1/0-2/0-4; a suíte de segurança do router existente (`delegation_budget_is_enforced_per_root_cause`, `spawn_cascade_requires_human_gate`) permanece verde. Marcar `PRONTO:`/`BLOCKED:`.

## GATE DE SAÍDA F3-0 (roda e se mede)

(a) `lina params set fanout_gate 8 --scope workspace` + recarregar → um `broadcast` em cascata p/ 7 alvos **entrega sem gate** (hoje, default 3 → `RouteBlocked{fanout_gated}`, teste `cascade_broadcast_over_fanout_gate_still_gated` `router.rs:2931`); reverter p/ 3 → volta a gatear; **medido contando `RouteBlocked{fanout_gated}` no log antes/depois**.
(b) Reconstruir `RouterConfig` por **replay do log** (sem reler arquivo) → `resolve_from_store(&store).router_config.fanout_gate == 8` (`resolve_from_store` é entrega da F3-0-2; não existe hoje — o gate mede a entrega, não pré-existência).
(c) `effort=High` injeta `LINA_EFFORT=high` no PTY filho (lido do ENV) + badge `modelo·effort` na tela.
(d) `grep -rniE '\beffort\b' crates/ app/` acha código real (`Effort::High`, `EffortAssigned`), não só "best-effort".
(e) Suíte de segurança do router **verde** com qualquer `params.json`.

## Conselho de gate (anti-deriva, read-only, não construiu a onda)

4 lentes antes de declarar o gate: (1) Visão/fios; (2) Arquitetura (ADR 0031 ↔ código re-derivado no fonte + invariantes 1-7); (3) Segurança (red-team: **0 ALTA para seguir**; carimbo server-side provado); (4) Specs (spot-check spec 51 vs implementado). A tela do badge `modelo·effort` (c) é **pendência de tela do fundador — bloqueante**.

## Riscos da onda

- 13 defaults presos no `..default()` viram "knob morto" se o seam de `bridge.rs` não fiar de fato → **critério (b) por replay**, não por presença.
- `model`/`effort` acoplarem a um CLI específico → contrato neutro `low/medium/high`; mapeamento é do CLI Profile (invariante #3).
- Preset do leigo expor jargão → sliders nomeados pt-br ("Quanto cada IA pensa antes de agir"), nunca a chave técnica.
- Colisão em `router.rs`/`events.rs` entre F3-0-2 e F3-0-3 → **dono único por rodada** (R2 sequencial), não paralelizar.
