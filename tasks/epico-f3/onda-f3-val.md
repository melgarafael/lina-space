# Onda F3-VAL — "Provar a Meta na Tela" · validação ao vivo + loop de correção de F3-1 e F3-2

> **Maestro:** Terminal B — Effort: Ultra Code. **Não é uma rodada de implementação — é o fechamento do gate (h) BLOQUEANTE** que ficou diferido em F3-1 e F3-2: a *validação do fundador na tela*. O fundador delegou o olho ao Maestro: subir um **Lina paralelo isolado** e validar com **computer use** (screencapture + cliclick + osascript — todos presentes na máquina), em **loop até 100% funcional**.

## Estado de entrada (verificado por git log + build, não pelo board)

- **F3-1 (Goal-and-Loop): código FECHADO e commitado** — cadeia `cc452cf`→`7c9bd0c` (CORE-Goal `0b3cc13`, CORE-Plan `c097ab7`, CLI `541e9bc`, UI `d40b929`, suíte adversarial `4d658ec`, gestos humanos ADR 0036 `5a999af`..`1060b36`, fixes GL-1..GL-5 `316d67b`). 480 testes lina-core. ADRs 0032/0033 aceitos.
- **F3-2 (Loop do Maestro + Tradutor): código FECHADO e commitado por fatia** — `24f6d90` core / `6cc3a3b` briefing+history / `0a803b0` Tradutor / `8abea53` UI / `260aa8a` QA + fiações `2b7f566`/`ada696c`/`854eda2`. Revisão cega PASS score 96, 0 ALTA. 413 testes lina-core.
- **Build verde** no HEAD atual **com o WIP não-commitado do Terminal A** (ADR 0037 `NodeCwdSet` + 0038 kit-lina) na árvore: workspace `cargo build` exit 0; `app/lina-gpui` `cargo build` exit 0.
- **Único pendente das duas ondas:** gate (h) — *"validação na TELA do cenário do loop … gpui não roda headless"* (onda-f3-1.md:123-124; onda-f3-2.md:115,149). **É esta rodada.**

## Fatos técnicos da validação (do reconhecimento)

- **O app É o supervisor.** Os verbos `lina goal define/interpret/confirm` **enfileiram** intent; quem cunha `goal_id`, valida o ciclo e emite os eventos é o `MailboxPump`/`Router::handle_goal` (`router.rs:959`), que roda **dentro do app gpui**. Sem app de pé, o CLI só enfileira → validar o **caminho real** exige o app rodando.
- **Eventos reais** (NÃO existem `Interpretation*` — reusam Goal): `GoalDefined`→`GoalInterpreted`→`GoalConfirmed`→`GoalDecomposed`+`PlanItemAttributed`×N→ (loop) `ReviewVerdict{Pass|Fail}` → `GoalAchieved` | `GoalEscalated`.
- **Números cravados:** turn-budget `N=3` falhas → `GoalEscalated{turn_budget_exhausted}` (`router.rs:56,2255`); `EFFORT_LADDER=[Medium,High]` (`router.rs:66`); juiz≠executor enforçado (`reviewer != target`); breaker sticky morde já na 2ª falha do topo da escada.
- **Isolamento:** `LINA_WS_ROOT=/tmp/lina-f3val LINA_DEMO=1` → `.lina/` em `/tmp/lina-f3val`, **sem poluir** o registro global `~/.lina/workspaces.json` (a auto-inscrição em `runtime.rs:371` é pulada por `LINA_DEMO=1`). CLI dirige o mesmo estado via `LINA_HOME=/tmp/lina-f3val/.lina`.
- **Card da Goal:** `goal_card.rs` + `main.rs:2123` `render_goals_panel` varre `lina_core::project_goals` (projeção VIVA, não mock).

## Estratégia: validar em CAMADAS — da determinística à visual

A prova de "100% funcional" é construída de baixo para cima; cada camada que falha vira despacho ao **dono da fronteira** e re-valida (loop). O Maestro conduz a subida do app e o computer use (serial, indelegável); o que se paraleliza são as **provas independentes** e as **correções**.

| Camada | O que prova | Como | Quem conduz |
|---|---|---|---|
| **C1 — Mecanismo (dados)** | gates (a)-(g) de F3-1/F3-2 por **evento no log** ao vivo (caminho real CLI→supervisor→eventos→`status`) | app isolado + CLI dirige + ler `events/` e `lina goal status --json` | Maestro |
| **C2 — Visual** | card F3-1 + estado do loop F3-2: **zero jargão**, identidade da casa, gestos confirmar/ajustar | `screencapture` da janela + olho do Maestro | Maestro |
| **C3 — Leigo e2e** | cenário "sem ensaio": pedido leigo→interpretação→time c/ effort→`Fail`→re-despacho→escala→`Pass`→narração | 1-2 terminais Claude Code reais no Lina paralelo | Maestro |
| **Provas paralelas** | cenário-completo por teste (R), zero-jargão no render por grep+leitura (G), output limpo dos verbos (I), parents/round-trip (H) | suíte/auditoria isoladas, **sem o app** | R · G · I · H |
| **Correções** | cada gap → verde + re-teste ao vivo | despacho especialista ao dono | dono da fronteira |

## Donos de fronteira (idênticos a F3-1/F3-2 — territórios disjuntos preservados)

| Frente | Dono | Fronteira (dono único) |
|---|---|---|
| CORE-Goal / loop / router | **B (Maestro)** | `crates/lina-core/src/goal.rs`, OUTER loop + `handle_goal` em `router.rs` |
| CORE-Plan | **H** | `crates/lina-core/src/plan.rs` (parents/acceptance/`try_claim`) |
| CLI | **I** | `crates/lina-bootstrap/src/bin/lina.rs` (verbos `goal`/`plan`) |
| UI / card | **G** | `app/lina-gpui/src/goal_card.rs` + `render_goals_panel` em `main.rs` |
| QA / red-team | **R** | `crates/**/tests/` + `#[cfg(test)]` |

> Costura `events.rs` está **suja pelo A** (WIP 0037/0038). NINGUÉM toca `events.rs`/`bridge.rs`/`main.rs` do A nesta rodada sem o Maestro coordenar. As correções de F3-VAL são aditivas e disjuntas do território do A.

## Protocolo de loop (até 100%)

1. Maestro sobe o app isolado e percorre C1→C2→C3, registrando **cada gap** em `tasks/epico-f3/despachos/f3-val/GAPS.md` (id, sintoma observado, evento/screenshot que prova, dono).
2. Cada gap → `lina handoff @<dono> "<gap>" --ref plan:F3VAL-<id>` com o despacho especialista (5 campos da skill `lina-dispatch`).
3. Worker corrige na **sua** fronteira (não commita), reporta `PRONTO`; Maestro valida de fora (exit codes diretos), cold-review se não-trivial, rebuilda o app, **re-testa ao vivo**.
4. Breaker sticky: gap que reincide 2× → escala ao fundador em pt-br, não vai a 3ª tentativa automática.

## GATE DE SAÍDA F3-VAL (o que destrava "100% funcional")

- **(C1)** ciclo F3-1 completo provado por evento ao vivo + replay byte-a-byte (mata/reabre → `status` idêntico); freios F3-2 provados: 3×`Fail`→`Escalated`, re-despacho com effort `Ord` crescente, juiz≠executor, breaker sticky, segurança (forja de `by`/`reviewer`/`origin` rejeitada).
- **(C2)** screenshots do card F3-1 e do loop F3-2 **aprovados**: zero jargão técnico, identidade da casa (fonte destaque≠corpo, tokens), gestos confirmar/ajustar presentes.
- **(C3)** ao menos UM cenário leigo ponta-a-ponta com terminal real fechando em `GoalAchieved` (ou `GoalEscalated` narrado), com narração leiga sem jargão.
- **Evidência anexada** (screenshots + trechos de event log), não auto-relato. O fundador dá o ok final sobre as evidências.
