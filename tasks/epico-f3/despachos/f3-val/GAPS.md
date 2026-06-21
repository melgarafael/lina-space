# F3-VAL · matriz de validação ao vivo (atualizada pelo Maestro a cada marco)

Lina de teste isolado: `LINA_WS_ROOT=/tmp/lina-f3val LINA_DEMO=1` · GUI debug · supervisor vivo (PID descobrir via `pgrep -x lina-gpui`).
Dirigir o teste: `export LINA_HOME=/tmp/lina-f3val/.lina LINA_NODE_NAME="Terminal A"; unset LINA_NODE_ID`.
Falar com a equipe (produção): `export LINA_HOME="/Users/rafaelmelgaco/Library/Application Support/Lina/Lina/.lina"` (NUNCA `unset`).

## Camada 1 — Mecanismo (eu, ao vivo no caminho real)

| Gate | Estado | Evidência |
|---|---|---|
| F3-1 ciclo `define→interpret→confirm→seed→status` | ✅ GREEN | eventos seq 67-72 no log de teste; `goal status` em pt-br, fase Decomposed |
| `origin:@Maestro` server-side (degrada sem Tradutor) | ✅ GREEN | GoalDefined seq 67 `origin:'@Maestro'` |
| `by:` NodeId server-side (não-forjável) | ✅ GREEN | GoalConfirmed seq 69 `by:'019ee7a1-159b-...'` |
| Replay determinístico | ✅ GREEN | 2 leituras de `goal status --json` byte-idênticas |
| Segurança de borda: sender desconhecido barrado | ✅ GREEN | `RouteBlocked unknown_sender` + dead-letter ao usar identidade de produção contra ws de teste (doutrina, não bug) |
| F3-2 `proposed_team` registrado no `GoalInterpreted` (`--team`) | ✅ GREEN | G2 seq GoalInterpreted `proposed_team=['Terminal B','Terminal A']` + strategy |
| Freios: turn-budget=3→Escalated, escala effort, juiz≠executor, breaker sticky | ✅ GREEN | `goal_loop_aceite_f3val.rs` 5/5 testes, `cargo test` exit 0 (validado de fora); R não tocou produção (router.rs/goal.rs diff vazio) |
| F3-2 time montado com effort (SpawnRequested pós-confirm) | ⏳ pendente (spawna terminais reais — C3) | — |

## Camada 2 — Visual (card)

| Gate | Estado | Evidência |
|---|---|---|
| zero-jargão / identidade / gestos (por código) | ✅ GREEN | Maestro reassumiu (G engoliu #22/#23); `goal_card.rs` auditado: jargão barrado por testes internos, Fraunces/Plex+tokens, gestos via cx.listener, 7 fases cobertas; fiação viva confirmada em `main.rs:2139 render_goals_panel` (project_goals, não mock) |
| screenshot do card (pixel) | 🔒 TCC: o `Lina.app` (host do meu processo) não está autorizado p/ Gravação de Tela; rebuild muda assinatura. Via colaborativa: fundador captura, Maestro lê. | — |

## Camada 3 — Leigo e2e

| Gate | Estado |
|---|---|
| cenário pedido→interpretação→time→FAIL→re-despacho→escala→PASS→narração com terminal real | ✅ DISPENSADO pelo fundador (2026-06-20): "prova atual basta" — mecanismo 100% por testes no caminho real (R) + ciclo ao vivo (Maestro); não gastar API |

## Status final F3-VAL (2026-06-20)
- **Critério central F3-1 + F3-2: 100% provado por evidência.** Mecanismo (ciclo, origin/by, replay, segurança, proposed_team) + freios (5/5 testes, caminho real) + card (auditoria de código G+Maestro).
- **GAP-1** → story residual (`STORY-RESIDUAL-escalada-manual.md`), porta aberta por decisão do fundador.
- **C3** → dispensado (prova peça-por-peça basta).
- **C2-pixel** → aguardando o olho do fundador no card (gate h é dele por design); auditoria de código já PASS.
- **Pronto p/ commit (quando o fundador pedir):** `crates/lina-core/tests/goal_loop_aceite_f3val.rs` (5 testes) + docs F3-VAL. NÃO commitado (sem pedido; árvore tem WIP do A).

## Gaps reais encontrados até agora
- **GAP-1 (achado do G, validado de fora) — fidelidade label↔ação na escalada manual.** Na fase `Escalated`, os botões "Reforçar o time"/"Olhar com você" (`goal_card.rs cta_labels`) reusam `on_confirm`/`on_correct` (`main.rs:2178-2183`). Efeito real: "Reforçar o time"→`goal.confirm` (re-dispara a montagem do time, router.rs:116, mas **não sobe effort**); "Olhar com você"→abre re-interpretação. **Label promete o que a ação não faz.** Escopo: é a **recuperação MANUAL pós-escalada** — o critério central F3-2 (loop automático + escala de effort por `Fail`) está 100% provado (R). Fix cruza `main.rs` (WIP sujo do A) + provável intent novo no core (`goal.reinforce` = re-arma loop com effort↑) → **decisão de escopo/design do fundador** (não atravessar o WIP do A nem inventar design no impulso). **Porta aberta:** story F3-2 residual.
- **Não-bug:** o `unknown_sender` foi artefato de setup do Maestro (identidade de produção vs ws de teste) — na verdade **prova a doutrina de segurança** funcionando.
- **Obstáculo (não-bug):** `lina vault index` stale não lista notas 39/52 — workers leem por path direto. (candidato a fix futuro; não bloqueia F3-VAL)
- **Obstáculo (ambiente, não-bug):** computer use por pixel bloqueado por TCC (o `Lina.app` host do meu processo não está autorizado p/ Gravação de Tela; rebuild muda assinatura). Contornado pela via colaborativa (fundador captura).

## Despachos ativos
- R → `tasks/epico-f3/despachos/f3-val/R-freios.md` (msg `019ee7ae-45d5`)
- G → `tasks/epico-f3/despachos/f3-val/G-card.md` (msg `019ee7ae-47e1`)
