# Onda F3-CONF-2 — Confiabilidade da Orquestração v2 (o Protocolo Anti-Engolimento) · plano de execução

> **Rodada DEDICADA** ao gargalo nº1 da orquestração (família dogfooding #22/#23: *handoff confirmado ≠ trabalho feito* — 5 ocorrências, incl. uso real com cliente). O fix foi conscientemente DIFERIDO em F3-2 para uma rodada própria (não cabia em `router.rs` sem colidir com o CORE da F3-2). Esta é essa rodada.
> **Fonte da verdade:** `tasks/epico-f3/repro-22-23.md` (taxonomia A/B/C + recomendação §6) · `tasks/despachos/achados-dogfooding-sessao.md` (#21, #22*, #23*, #15) · épico `39 - Epico Fase 3 — O Maestro Nativo` (vault) §I "orquestração como mecanismo, não skill", §VI riscos 1-3 · ADR 0019 (definições de progresso/travamento) · ADR 0007 (carimbo server-side; regra-mãe).
> **Maestro:** Terminal A (ARQUITETO). **Workers NÃO commitam; o Maestro fixa o contrato, valida de fora (exit codes diretos — pipe mascara), e commita por fatia.**

## Por que esta rodada é "avanço significativo na inteligência do Lina"

A F3 é o salto de **orquestração-por-skill** para **orquestração-como-mecanismo**. F3-0/F3-1/F3-2 entregaram as primitivas (SystemParams, Goal, loop interpretar→confirmar, Tradutor). Mas o loop do Maestro nativo **não fecha na vida real**: o despacho é entregue e some (zero trabalho, zero `DomainEvent`). Hoje a única defesa é DETECÇÃO (alarme `DeliveredNoProgress`, W3) — nada CONSOME o alarme. Esta rodada mecaniza o **passo 6 (monitorar) + passo 7 (corrigir trajeto)** do loop do orquestrador como mecanismo event-sourced, e torna o estado mais crítico (`circuit_breaker`) honesto na tela. É a fundação sem a qual F3-3 (Mentality) / F3-4 / F3-5 não se sustentam (risco §VI-3 do épico: adoção 0% — Goal/effort construídos e nunca acionados porque o despacho não vira trabalho).

## Invariante que ATRAVESSA a rodada (não regredir)

**Reason / interpretação / effort / veredito é DADO transportado, JAMAIS autoridade.** O protocolo anti-engolimento re-despacha e escala recurso, mas **nunca** decide segurança/identidade/ordem. `by`/`reviewer`/`origin`/`reason` são carimbados **server-side** (família ADR 0007). Zero LLM no core (inv #1): o "juiz de engolimento" é estrutural (ausência de `DomainEvent` de progresso na janela), não um modelo. Toda story que toca `router.rs`/`deliver_a2a` tem como critério implícito a **suíte de segurança do router verde por mutação**.

## DAG executável e frentes (fronteiras DISJUNTAS — dono único por costura)

```
[Rβ-0: árvore limpa (já) + CONTRATO do reason surfacado, commitado (Maestro/A)]
        │
        ├─► CORE-LOOP  (B·Ultra)  router.rs + attention.rs ──┐  protocolo anti-engolimento + loop_detected + reset breaker
        ├─► UI-HONESTA (G)        app/lina-gpui ─────────────┤  circuit_breaker legível + badge modelo·effort (F3-0-6)
        ├─► QA/RED     (R)        crates/**/tests (novos) ───┤  mutação do protocolo + segurança router + repro Modo C + auditar F3-0-7
        └─► HARNESS    (I)        crates/lina-core/tests ────┘  tmpdir thread::id (mata o flaky sistêmico)
                │
                ▼
   Maestro (A): integração final (botão "liberar" ↔ verbo de reset) + gate de onda (cold-review + 4 lentes) + commit por fatia
```

| Frente | Terminal | Toca (dono único) | model·effort | Por que não colide |
|---|---|---|---|---|
| **Contrato/integração** | A (Maestro) | `lina-host/lib.rs`, `lina-core/lib.rs` (BusEvent/set_status), `events.rs`, `router.rs:1537-1541` (1 site do breaker) | opus · High | fixo e commito ANTES do fan-out; depois ninguém toca |
| **CORE-LOOP** | B · Ultra Code | `crates/lina-core/src/router.rs` + `attention.rs` | **opus · High (Ultra)** | dono único de router.rs/attention.rs APÓS o commit do contrato |
| **UI-HONESTA** | G | `app/lina-gpui/src/{bridge.rs,canvas.rs,main.rs,dashboard.rs}` | opus · Medium | só projeção/render — consome o contrato; sem costura de core |
| **QA/RED-TEAM** | R | `crates/**/tests/` (arquivos NOVOS) + `#[cfg(test)]` read-only | **opus · High** | lê produção, escreve só testes |
| **HARNESS/FLAKY** | I | `crates/lina-core/tests/history_f1_5_8.rs` (+ opcionais gate_w34/scrollback_cable_w52) | opus · Medium | só arquivos tests/ existentes; disjunto dos novos do R |

**Reserva (escalada):** H, J, K — o próprio mecanismo que estamos construindo (novo-dev com effort maior) se uma frente travar 2×.

## Rβ-0 (Maestro/A) — o contrato (setup, ANTES do fan-out)

1. **Árvore limpa:** confirmado — `git status` vazio, HEAD `6503b05`. Nada a commitar antes.
2. **Fixar o contrato do `reason` surfacado** (o que UI consome e CORE propaga) — todos ADITIVOS:
   - `crates/lina-host/src/lib.rs`: `NodeStatus::Blocked { reason: String }` (vira `Clone`, não `Copy`; `#[non_exhaustive]` já permite o braço novo). Atualizo os matches afetados (status_dot/aggregate_badge ficam com stub que G refina).
   - `crates/lina-core/src/lib.rs`: `BusEvent::NodeStatus` ganha `reason: Option<String>` (`#[serde(default)]`); `Supervisor::set_status` propaga o reason (assinatura nova OU método irmão).
   - `crates/lina-core/src/router.rs:1533-1541`: o site do breaker passa `reason:"circuit_breaker"` ao set_status (1 linha — fica no contrato p/ não colidir com B).
   - (se necessário p/ a projeção) `events.rs`: `ProjectedNode.reason` + reducer guarda o reason de `NodeStatusChanged`.
3. **Commit do contrato = sinal de largada.** Replay F0/F1/F2 carrega sem erro (campos default).

## GATE DE SAÍDA F3-CONF-2 — RODA e se MEDE

- **(a) Protocolo anti-engolimento:** um despacho ENTREGUE (`MessageDelivered`) que volta a `Idle` sem nenhum `DomainEvent` de progresso na janela → o OUTER loop **re-despacha 1× informado** (payload com "não recebi sinal de progresso; reassuma a tarefa") e, persistindo, **breaker sticky** após 2 → `GoalEscalated{reason:"stalled"}`. Provado por teste do caminho real (`route_message`), RED→GREEN, no `repro_22_23_residual.rs`-style.
- **(b) loop_detected calibrado:** re-despacho ao MESMO worker engolido (zero progresso desde a entrega) **NÃO** é barrado como `LoopDetected`; cascata-tempestade real (saltos repetidos COM atividade no meio) **continua** barrada. Teste por contraste.
- **(c) circuit_breaker legível:** um nó pausado pelo breaker pinta como **"pausado por segurança — clique para liberar"** (não âmbar "trabalhando"); o reason flui core→host→render; zero jargão (`STATE_JARGON` verde). O "clique para liberar" chama um **verbo de reset** real do core (reset sob gesto humano, `by` server-side).
- **(d) Badge modelo·effort (F3-0-6):** o header do card do canvas mostra `opus · caprichoso` (effort em pt-br), reusando `dashboard::effort_badges`.
- **(e) Segurança:** suíte do router verde **por mutação**; o re-despacho/escala não vira portal (`GoalEscalated` forjado ignorado; reason carimbado server-side; reset do breaker exige gesto humano, não auto-reset por agente). 0 ALTA.
- **(f) Harness:** `cargo test` paralelo do `lina-core` deixa de piscar vermelho em `history_f1_5_8` (tmpdir por-thread); o gate do Maestro para de ter falso-vermelho.
- **(g) [DIFERIDO ao fundador]** validação na tela ao vivo (o breaker legível + o badge na tela) — junto da pendência de tela de F3-1/F3-2. gpui não roda headless.

## Conselho de gate de onda (4 lentes, read-only)

(1) Visão/fios: o protocolo mecaniza o passo 6/7 do loop sem virar harness (inv #1). (2) Arquitetura: ZERO LLM no core (juiz de engolimento = ausência de evento, estrutural); reason aditivo; replay íntegro. (3) Segurança (red-team, **0 ALTA p/ seguir**): reason/escala/reset nunca autoridade; reset do breaker = gesto humano. (4) Specs: spot-check repro-22-23 §6 vs implementado.

## Pendências tratadas / deferidas

- **Embutidas:** #22/#23 família (Modo B mecanizado; Modo C como vigia/teste), #21 circuit_breaker legível, #15 (já fechado em F3-2 — só reuso), F3-0-6 badge, flaky sistêmico.
- **Auditada (já verde):** F3-0-7 Teste C (`weakening_a_balancing_loop_is_gated_hard` já implementado — o Maestro confirma no gate, não re-implementa).
- **Candidata oportunista (se folga):** #17b residual (roster/vault/plano ainda do bootstrap.json por-cwd — já env-first p/ nome).
- **Deferida com porta:** validação na tela do fundador (gate g) + Modo C repro determinístico do pump (toca agendamento do pump = CORE, encaixe se B tiver folga).
- **Fora do escopo:** #25 ENOSPC → F3-5-8; F3-3 Mentality → próxima rodada (sobre esta fundação).
