# F3-1 · Roteiro de integração & gate (Maestro Terminal A)

> Como o Maestro valida CADA fatia **de fora** (exit codes diretos — pipe mascara; `touch` fura cache de lint) e conduz o gate de onda. Workers **não commitam**; o Maestro commita por fatia. Estado vivo desta rodada.

## Ordem de integração (deps reais)
1. **Contrato (B)** → valida → **commita** (base) → libera as 4 frentes.
2. **CORE-Plan (H)** ∥ **CLI (I)** ∥ **UI (G)** ∥ **CORE-Goal-lógica (B)** ∥ **QA-segurança (R)** — fronteiras disjuntas, validadas e commitadas por fatia conforme chegam.
3. Maestro: **integração e2e** + **gate de onda (4 lentes)** + **sessão de tela**.

## Validação por fatia (rodar com exit code DIRETO, sem pipe)
| Fatia | Comando de gate | Verde = |
|---|---|---|
| Contrato (B) | `cargo build -p lina-core` ; `cargo test -p lina-core` ; `cargo clippy -p lina-core -- -D warnings` | compila + replay F0/F1/F2 ok + 0 warning |
| CORE-Plan (H) | `cargo test -p lina-core` (parents + round-trip) ; clippy | `ParentsNotDone` bloqueia claim; round-trip byte-a-byte |
| CLI (I) | `cargo test -p lina-bootstrap` (doctrine_verbs) ; clippy | verbos enfileiram intent certo; **zero verbo fantasma** |
| UI (G) | `cargo test --manifest-path app/lina-gpui/Cargo.toml` ; clippy do app | card compila; catraca token/a11y verde (memória) |
| CORE-Goal (B) | `cargo test -p lina-core` (loop a-e) ; clippy | Pass fecha; 3 Fail → Escalated; reviewer≠target; effort↑ |
| QA (R) | `cargo test -p lina-core` (forja) ; suíte router | forja recusada; suíte segurança verde |

> **Memória aplicável:** validar fatia que toca app/bridge/admit exige `cargo test` do manifest do app (não só clippy); `cargo fmt -p` só nos arquivos da fatia (árvore compartilhada); o token_ratchet é teste global (gate de UI roda suíte completa).

## Cenário e2e do gate de saída F3-1 (o circuito do doc-fonte 12)
```
lina goal define "<meta leiga>" --accept "<criterio exit-0>"
  → GoalDefined (goal_id cunhado server-side)
lina goal interpret <id> --understanding "..." --strategy "..." --team A,B
  → GoalInterpreted
[gate humano: GoalConfirmed]            # assistido: propõe→confirma
lina plan seed <id>                      → GoalDecomposed + N PlanItemAttributed (goal_id)
--- matar o processo, reabrir, replay ---
  → (a) Plan + estado da Goal reconstroem BYTE-A-BYTE (event_count e Plan iguais)
loop: PlanChecked → acceptance[] (exit 0) → ReviewVerdict{Pass}
  → (b) ao 2º Pass: exatamente um GoalAchieved, zero despacho depois
loop com criterio exit 1: 3× ReviewVerdict{Fail} → (c) GoalEscalated{turn_budget_exhausted}
  → (d) todo ReviewVerdict reviewer≠target; (e) re-spawn effort estritamente maior
parents:["T1"] → claim T2 recusado até T1 Done   # (f) [ADR 0032]
GoalConfirmed forjado com `by` de outro nó → IGNORADO  # (g)
```

## Gate de onda — 4 lentes (read-only, agentes que NÃO construíram a fatia)
1. **Visão/fios** — Goal materializa "perseguir metas, não executar pedidos"?
2. **Arquitetura** — ADR 0033/0032 ↔ código re-derivado no fonte + invariantes 1-7. **Risco #1: nenhum "motor de avaliação" que não seja `CheckKind` determinístico ou nó QA; ZERO LLM no core.**
3. **Segurança** — red-team: **0 ALTA para seguir**; `interpretation`/`evidence`/`effort`/`by` provados como dado, nunca autoridade.
4. **Specs** — spot-check spec 52 vs implementado; nada refutado re-entrou.

## Gate `h` (BLOQUEANTE) — sessão de tela do fundador
Card da Goal na tela: meta + "o que entendi" + critérios + barra por iteração; confirmar 1 toque; aviso de escalada narrado. Sem jargão. **"validei tudo visualmente"** = gate fechado.
