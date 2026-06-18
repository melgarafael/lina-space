# Onda F3-1 — Goal-and-Loop, a Meta como Primitiva · plano de execução

> **Peça-fonte story-a-story** da onda F3-1 do épico `39 - Epico Fase 3 — O Maestro Nativo` (vault). Em conflito, esta peça vence o épico; o ADR aceito vence ambos.
> **Fonte da verdade detalhada:** `52 - SPEC Goal-and-Loop - A Meta como Primitiva` (vault) — schema, segurança, critérios integrais. **Todo worker LÊ a spec 52 antes de codar sua fatia** (o despacho aponta a seção exata).
> **Maestro:** Terminal A (ARQUITETO). **Workers não commitam; o Maestro fixa o contrato, valida de fora (exit codes diretos) e commita por fatia.**

## Objetivo da onda

Transformar a Goal de **"frase no prompt"** em **entidade event-sourced de primeira classe** e mecanizar o **loop-até-aceite** com o **juiz separado do executor**. É onde o Lina deixa de **executar pedidos** e passa a **perseguir metas** — ponto **[3] Objetivos** (a alavanca mais alta que ainda cabe no produto sem virar autonomia plena) + **[8] Feedback negativo** (o `ReviewVerdict` é o termostato de qualidade) de Meadows; doc-fonte linhas 10-12. O Lina implementa **só o OUTER loop** (observa terminal idle, roda o juiz, re-injeta a continuação ou fecha); o INNER loop é o CLI de terceiro (**invariante #1: sem LLM próprio no core**).

Materializa o método do fundador (doc-fonte 12): *"executam até atingir a meta → o Maestro analisa e devolve a tratativa se há erro/qualidade/segurança → se o dev não consegue, cria um novo com effort maior → entrega."*

## ⛔ Gate de entrada — ADR 0033 (+ 0032) ANTES de codar

- **`docs/adr/0033-goal-event-sourced.md` está `Proposto`.** A onda F3-1 **INTEIRA não inicia** sem o ADR 0033 **aceito** pelo fundador — ele decide a forma dos eventos `GoalDefined`/`GoalInterpreted`/`ReviewVerdict` que **todas** as frentes consomem.
- **`docs/adr/0032-parents-no-plano.md` está `Proposto`.** A fatia **CORE-Plan (F3-1-3, `parents`)** e o bloqueio anti-race **não iniciam** sem o ADR 0032 aceito. As demais fatias não dependem dele.
- **1ª ação do Maestro:** levar ambos ao fundador (decisão sim/não em 1 frase):
  - 0033 — *"a Goal (objetivo de uma sessão de trabalho) vira entidade durável no log, com critério de aceite verificável e loop-até-aceite com juiz separado do executor?"*
  - 0032 — *"itens do plano ganham dependências estruturadas (`parents:`) que bloqueiam o despacho até os pais ficarem prontos?"*

## A jogada de arquiteto: CONTRATO primeiro, depois fan-out

F3-1 é acoplada em poucas costuras (`events.rs`, `router.rs`, `plan.rs`) — costuras de **dono único por rodada**. Para paralelizar de verdade sem colisão, o **Maestro (Terminal A, ARQUITETO) fixa o contrato ANTES das frentes**: todas as variantes de evento + os tipos compartilhados, compilando com aplicadores-stub. Commitado o contrato, as 5 frentes codam contra arquivos **disjuntos**.

**Rα-0 (Maestro, effort High) — o contrato em `events.rs` (dono único, fixado 1×):**
- Variantes novas: `GoalDefined`, `GoalInterpreted`, `GoalConfirmed`, `GoalDecomposed`, `GoalAchieved`, `GoalEscalated`, `PlanItemAttributed`, `ReviewVerdict` (schema integral da spec 52 §1/§2/§3 — sem `serde(default)` nos campos da variante nova; precedente `SpawnRequested`).
- Campos aditivos em `SpawnRequested`/`SpawnAdmitted`: `model: Option<String>`, `effort: Option<Effort>`, `goal_id: Option<String>` (todos `#[serde(default)]`) — **se já não landados pela F3-0-3** (reconciliar: F3-0 já tem `Effort`/`EffortAssigned`).
- Tipos compartilhados: `AcceptanceCriterion{desc, check_kind, check_arg}`, `enum CheckKind{HumanReview(default), Command, FileExists, TestPass}`, `enum Verdict{Pass, Fail}`. (`Effort` já existe da F3-0.)
- Registry de nomes (`events.rs:881`) + braços de `apply` (`events.rs:1179`) chamando **assinaturas** de projeção que as frentes implementam (`goal.rs`/`plan.rs`).
- **Critério:** `cargo build -p lina-core` verde com a árvore de eventos completa e `todo!()`/no-op nos aplicadores que as frentes preencherão. Replay de log F0/F1/F2 carrega sem erro (campos `serde(default)`). **Commit do contrato é o sinal de largada das frentes.**

## DAG executável (deps) e frentes de paralelização

```
[GATE: ADR 0033 (+ 0032) aceitos]
        │
        ▼
   Rα-0  CONTRATO (Maestro/A) ── events.rs fixo + commit ──┐
        │                                                  │
        ├─► CORE-Goal (B·Ultra)  goal.rs + router.rs  ─────┤  F3-1-1(proj) · F3-1-2(exec) · F3-1-4 · F3-1-5
        ├─► CORE-Plan (H)        plan.rs              ─────┤  F3-1-3 [ADR 0032]
        ├─► CLI       (I)        lina.rs              ─────┤  F3-1-6
        ├─► UI        (G)        app/lina-gpui        ─────┤  F3-1-7
        └─► QA        (R)        tests/ + gate F3-0   ─────┘  F3-1-8 + validação gate F3-0
                │
                ▼
   Maestro (A): integração e2e + gate de onda (4 lentes) + commit por fatia + tela do fundador
```

| Frente | Stories | Por que não colidem |
|---|---|---|
| **CORE-Goal** (Terminal B · Ultra Code) | F3-1-1 (projeção `Goal`) · F3-1-2 (executar `acceptance[]` no laço) · F3-1-4 (`ReviewVerdict` + re-despacho informado + escala effort) · F3-1-5 (turn-budget + fail-open + parse-breaker) | dono de `goal.rs` (novo) + lógica de loop em `router.rs` — events.rs já está fixo pelo contrato |
| **CORE-Plan** (Terminal H) | F3-1-3 (`PlanItemAttributed` aplicado + `parents`/`acceptance` no `PlanItem` + `try_claim`→`ParentsNotDone`) | dono de `plan.rs` — disjunto de `router.rs`/`goal.rs` |
| **CLI** (Terminal I) | F3-1-6 (verbos `lina goal define/interpret/status` + `lina plan add/seed`) | dono de `lina.rs` (bin) + `doctrine_verbs.rs` — enfileira intent, não toca core |
| **UI** (Terminal G) | F3-1-7 (card da Goal: meta + "o que entendi" + critérios + barra por iteração) | dono de `app/lina-gpui/src/` (canvas/card) — projeção, sem costura de core |
| **QA/Red-team** (Terminal R) | F3-1-8 (forja de `by`/`reviewer` + `ReviewVerdict{Pass}` não libera `GatedHard`) · validação do **gate F3-0** (badge + teste C + suíte) | testes em `crates/**/tests/` + `#[cfg(test)]` — lê, não altera produção |

## Fronteiras de arquivo por frente (território disjunto)

| Frente | Toca (dono) | Costura (dono único/rodada) |
|---|---|---|
| **Maestro/Contrato** | `crates/lina-core/src/events.rs` (variantes + tipos + registry + apply-stub) | `events.rs` (Rα-0: Maestro — fixo antes das frentes) |
| **CORE-Goal** | `crates/lina-core/src/goal.rs` **(novo)**; OUTER loop + handlers `goal.*` em `router.rs` | `router.rs` (CORE-Goal) |
| **CORE-Plan** | `PlanItem` + `apply_item_attributed` + `try_claim`/`ParentsNotDone` em `crates/lina-core/src/plan.rs` | `plan.rs` (CORE-Plan) |
| **CLI** | `crates/lina-bootstrap/src/bin/lina.rs` (verbos `goal`/`plan add`/`plan seed`); `tests/doctrine_verbs.rs` | `lina.rs` (CLI) |
| **UI** | card da Goal em `app/lina-gpui/src/` (canvas/novo módulo `goal_card.rs`) | — (sem costura de core) |
| **QA** | `crates/lina-core/tests/` + `#[cfg(test)]` de `router.rs`/`goal.rs` | lê, não altera lógica de produção |

> **Regra de costura (protocolo F0/F1):** `events.rs` é fixado pelo Maestro no Rα-0; depois disso **ninguém mais o toca nesta rodada**. `router.rs`=CORE-Goal, `plan.rs`=CORE-Plan, `lina.rs`=CLI são donos únicos. Precisa de algo numa costura de outro dono? **Peça ao Maestro** — nunca edite/reverta linha de peer. `cargo fmt -p` toca só os próprios arquivos (memória: *fmt em árvore compartilhada*).

## Atribuição modelo·effort (dogfooding da própria F3-0 — doc-fonte 12)

O Maestro casa dificuldade↔effort exatamente como a F3-0 automatiza — *"define o melhor modelo e effort por desenvolvedor"*:

| Frente | Terminal | Dificuldade | model · effort |
|---|---|---|---|
| Contrato | A (Maestro) | tipos de evento que tudo herda — errar aqui custa caro | **opus · High** |
| CORE-Goal | B · Ultra Code | projeção + loop juiz≠executor + escalonamento — o coração | **opus · High (Ultra)** |
| CORE-Plan | H | campos aditivos + guarda `try_claim` (round-trip byte-a-byte) | opus · Medium |
| CLI | I | verbos gateados, padrão `run_plan_intent` | opus · Medium |
| UI | G | render do card + naming pt-br do leigo | opus · Medium |
| QA/Red-team | R | red-team de carimbo server-side (forja `by`/`reviewer`) — sutil | **opus · High** |

## GATE DE SAÍDA F3-1 (roda e se mede — do épico §V)

(a) `lina goal define` → `interpret` → `GoalConfirmed` → `lina plan seed` gera ≥1 `PlanItemAttributed` com `goal_id`; **mata o processo, reabre, replay → `Plan` reconstrói byte-a-byte e o estado da Goal idêntico** (`event_count` e `Plan` iguais antes/depois).
(b) Goal com 2 itens, `AcceptanceCriterion`=comando exit 0: ao 2º `ReviewVerdict::Pass` → exatamente um `GoalAchieved` e **nenhum** despacho depois.
(c) Goal com critério que SEMPRE falha (exit 1): após 3 `ReviewVerdict::Fail` → `GoalEscalated{turn_budget_exhausted}` e **zero iteração a mais**.
(d) Todo `ReviewVerdict` tem `reviewer != target`; o forjado (`reviewer==target`) é **recusado**.
(e) Re-spawn no Fail carrega effort **estritamente maior** (comparação `Ord` no log).
(f) `parents:["T1"]` bloqueia `claim T2` até `T1` virar `Done` (`PlanError::ParentsNotDone`). [requer ADR 0032]
(g) Suíte de segurança do Router **verde** (`delegation_budget_is_enforced_per_root_cause`, `spawn_cascade_requires_human_gate`); `GoalConfirmed` forjado com `by` de outro nó **ignorado**.
**(h) Validação do fundador na tela (card da Goal) é parte do gate — BLOQUEANTE.**

## Conselho de gate (anti-deriva, read-only, não construiu a onda)

4 lentes antes de declarar o gate: (1) Visão/fios; (2) Arquitetura (ADR 0033/0032 ↔ código re-derivado no fonte + invariantes 1-7 + **risco #1: nenhum "motor de avaliação" que não seja `CheckKind` determinístico ou nó QA; zero LLM no core**); (3) Segurança (red-team: **0 ALTA para seguir**; `interpretation`/`evidence`/`effort` provados como dado, nunca autoridade); (4) Specs (spot-check spec 52 vs implementado). A **tela do card (h)** é pendência de tela do fundador — bloqueante.

## Riscos da onda (do épico + spec 52)

- **O Maestro nativo recriar um harness** (motor de avaliação próprio / LLM no core) → viola inv #1. *Mitigação:* juiz é `CheckKind` determinístico OU nó QA distinto; o core só observa/nomeia/dispara.
- **Loop infinito** se o turn-budget não freiar → critério (c) mede **exatamente 3 Fail + silêncio**.
- **Executor auto-avaliar** enviesado a "pronto" → `reviewer != target` enforçado + recusa do forjado (d).
- **`CheckKind::Command` virar portal de execução arbitrária** → roda sob `guard::decide`; `GatedHard` (deploy/pay/`rm -rf`) pede `Ask`, o juiz não roda ação irreversível para "verificar".
- **Adoção 0%** (como o `plan.md` no gate F1-3) → medir uso real do `lina goal` no log desde o dia 1, não assumir.
- **Colisão em `events.rs`** entre frentes → **contrato fixado pelo Maestro no Rα-0**, ninguém mais toca.

## Rodada α (recorte desta sessão)

Escopo recomendado: **a onda F3-1 inteira** nas 5 frentes + contrato do Maestro — entrega o loop COMPLETO do doc-fonte 12 (Goal nasce → interpreta → confirma → decompõe → executa → juiz → escala effort → fecha). Precede F3-2 (o loop do Maestro nativo + papel Tradutor costura tudo numa cena real de ponta a ponta).
Alternativa conservadora (se preferir menor superfície por rodada): **α-focada** = só a Goal durável + verbos + card (F3-1-1/2/3/6/7), deferindo o termostato (F3-1-4/5/8) para a rodada β.

---

## STATUS DA EXECUÇÃO — 2026-06-18 (rodada α COMPLETA, escopo cheio)

✅ **F3-1 implementada, auditada e commitada** — 11 commits (`cc452cf`→`7c9bd0c`):
- CORE-Plan `c097ab7` · CORE-Goal `0b3cc13` · CLI verbos `541e9bc` + confirm `7c9bd0c` · QA segurança `4d658ec` · UI card-vivo `d40b929` · fix-forward Ord+doc `4a0eb7a` · ADR 0033/0032 aceitos (`0943674`) + ADR 0036 (`ea3f016`).
- **Gate de onda: PASSOU** — 2 auditores independentes (read-only, não construíram), **0 ALTA**. Confirmados por re-derivação no fonte: invariante #1 (ZERO LLM no core; juiz = `std::process::Command`/exit code) e a regra-mãe (`by`/`reviewer`/`effort` nunca autoridade — impossível por *tipo*, `guard.rs::decide` não recebe verdict). Achados BAIXA (Effort sem `Ord`; doc-comments stale) corrigidos no fix-forward.
- 480 testes lina-core / 0 falhas (gate_d **por mutação**, integração `route_message` zero-mock); clippy 0; fmt limpo; token_ratchet intacto; suíte de segurança do Router 4/4.

⏳ **PENDENTE (porta desenhada, não dívida cega):**
- **Gate (h) — validação de tela do fundador:** código provado; e2e AO VIVO precisa do Lina.app rebuildado (binário/supervisor em execução ainda são pré-rodada).
- **Confirm 1-toque pela UI:** deferido sob **ADR 0036** (identidade do gesto humano na UI); por ora confirm via `lina goal confirm` (CLI).

➡️ **Próximo:** F3-2 (loop do Maestro nativo + papel Tradutor — cenário real de ponta a ponta na tela).

> Atualizar também o §II do épico `39` (vault) e o `_HANDOFF` quando possível — fora do alcance de escrita do Maestro (vault é read-only exceto `Lina/`).
