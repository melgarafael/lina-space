# ADR 0007 — fan-out INICIAL do humano é livre; os freios anti-tempestade valem na CASCATA

- **Status:** Aceito (implementado no `Router::route_message` + verbo `lina broadcast` + SKILL `lina-agent-bus`)
- **Onda/Story:** Onda 5+ · fan-out-fix (bug de uso real validado em tela pelo fundador)
- **Data:** 2026-06-03

## Contexto

O fundador pediu a um agente **"manda oi pros 27 terminais"**. Só chegou ao terminal B. A causa raiz
(confirmada por evidência — **zero** `RouteBlocked` no log, pulsos só A↔B) tem duas camadas:

1. **Skill** (`lina-agent-bus/SKILL.md`): não tinha a receita "avisar TODOS os terminais" e ainda
   desencorajava ("se forem muitos, te peço o ok antes"). O agente mandou só ao B e narrou sucesso.
2. **Mecanismo** (`router.rs`): o **gate de fan-out** (`FANOUT_GATE = 3`: broadcast a > 3 alvos →
   confirmação humana, não entrega) barraria o `lina broadcast "*"` legítimo. O **orçamento**
   (`DELEGATION_BUDGET = 8`) é citado no diagnóstico, mas (ver abaixo) **já não barrava a origem**.

Esses freios existem por um bom motivo: barrar **tempestade** (broadcast que dispara broadcast →
amplificação N×N) e **loop**. Mas, do jeito que estavam, também barravam a 1ª onda **pedida pelo
humano** — que é exatamente o que o Maestri faz sem fricção. O produto precisa do fan-out 1-a-1 a
todos os terminais quando o usuário pede, **sem reabrir** a porta da tempestade.

## A chave: ORIGEM vs CASCATA, via `hops` NÃO-forjável

O roteador já distingue, de forma **à prova de forja**, a profundidade de uma mensagem na cadeia
causal — `derive_root_hops` (W1/A2) deriva o `(root_cause_id, hops)` EFETIVO do **binding interno**
que o supervisor carimba na ENTREGA (`delivered_root[sender]`), **nunca** do campo `msg.hops`/
`msg.root_cause_id` (que o outbox não-autenticado torna forjável):

- **ORIGEM (`hops == 0`):** o sender **não tem binding** — não recebeu nenhuma A2A. É o agente
  disparando **a mando do humano** (o prompt do usuário não é uma entrega A2A). A 1ª onda.
- **CASCATA (`hops >= 1`):** o sender **recebeu uma entrega** (tem binding) e está **re-espalhando**.
  É aqui que mora a tempestade.

Como `hops` vem do binding (carimbado numa entrega REAL), **um agente não compra fan-out livre
forjando `hops=0`** — a defesa anti-forja de W1 é preservada.

## Decisão

**O gate de fan-out e o orçamento de delegação aplicam-se à CASCATA (`hops >= 1`), não ao fan-out
INICIAL do humano (`hops == 0`).** Os backstops finais ficam **intactos**.

1. **`FANOUT_GATE` só na cascata.** `route_message`: o gate (`broadcast` a > `fanout_gate` alvos →
   `FanoutGated`) passou a exigir `hops >= 1`. Origem (`hops == 0`) entrega a TODOS os nós vivos.
2. **`DELEGATION_BUDGET` só na cascata.** O **enforcement** (CHECK) exige `hops >= 1`. Nota honesta:
   a origem **já escapava por construção** — cada msg de origem ganha um `root_cause_id` FRESCO
   (`msg.id`, W1), então `(root, sender)` nunca acumula. O `hops >= 1` no CHECK torna o invariante
   **explícito no ponto de decisão** (defesa em profundidade). O **incremento** do contador segue
   para toda delegação (bookkeeping/observabilidade + poda A5); a entrada de origem é inofensiva
   (root fresco → nunca re-checada) e é podada pela janela.
3. **Backstops anti-tempestade preservados:** `MAX_DEPTH` (`hops <= 4`), anti-loop por grafo de
   eventos (`MAX_CYCLE_REVISITS`, ping-pong apertado), a regra de skill **"nunca broadcast em
   resposta a broadcast"**, e **gate humano + custódia em ação irreversível (ADR 0004)** — nada
   disso muda. A tempestade é exatamente a CASCATA, que continua gateada.
4. **Verbo:** `lina broadcast "*" "<msg>"` (açúcar de `lina ask` com alvo `*` + `intent=broadcast`)
   avisa todos os vivos; `--role PAPEL` restringe a um papel.
5. **Autonomia respeitada:** em `manual`/`assistido` o agente PROPÕE/pede-ok como em qualquer ação
   (comportamento atual). O que muda é que, autorizado (ou em `autonomo`), a 1ª onda **não é mais
   barrada por um gate DURO de 3/8**.

## Mudança de invariante

| Antes | Depois (ADR 0007) |
|---|---|
| `FANOUT_GATE`/`DELEGATION_BUDGET` aplicavam a QUALQUER broadcast/delegação | Aplicam à **CASCATA** (`hops >= 1`); a **origem** (`hops == 0`) é livre |
| `lina broadcast "*"` de origem para > 3 alvos → `FanoutGated` (não entregava) | → entrega a TODOS os vivos, 0 `RouteBlocked` |
| Skill mandava pedir ok p/ "muitos" | Skill: fan-out inicial do humano é direto; só a cascata pede ok |

## Consequências

- **Positivas:** o caso de uso real ("avise o time todo") funciona em 1 comando, igual o Maestri,
  sem reabrir tempestade/loop. A distinção origem/cascata reaproveita um mecanismo **já testado e
  à prova de forja** (`hops` do binding) — sem novo estado, sem nova superfície de ataque.
- **Custo / risco:** a 1ª onda de origem é O(N) (N = nós vivos), limitada pela ação do próprio
  agente (o CLI não se auto-prompta infinitamente) e pelo fato de os destinatários **não**
  re-espalharem (cascata gateada + regra de skill). A amplificação exponencial exige cascata, que
  segue barrada.
- **Red-team (a provar):** (a) origem > gate entrega a todos, 0 RouteBlocked; (b) cascata > gate
  AINDA gated; (c) loop A→B→A AINDA cortado; (d) orçamento limita a cascata, não a origem. Cobertos
  pelos testes `user_origin_broadcast_fans_out_to_all_without_gate`,
  `cascade_broadcast_over_fanout_gate_still_gated`, `tight_loop_blocked_after_revisit_limit` /
  `forged_root_does_not_bypass_loop_detection`, `origin_delegations_are_not_limited_by_budget` /
  `delegation_budget_is_enforced_per_root_cause`.

## Alternativas descartadas

- **Um flag "is_user_initiated" na mensagem** → forjável pelo outbox não-autenticado (W1) — abriria
  exatamente o buraco que `derive_root_hops` fechou. `hops` derivado do binding é a fonte confiável.
- **Elevar `FANOUT_GATE`/`DELEGATION_BUDGET`** → trataria sintoma, não causa: a cascata grande
  continuaria capaz de tempestade, e a origem pequena continuaria gateada por um número arbitrário.
- **Remover os gates** → reabriria a tempestade/loop que eles existem para barrar (invariante de
  segurança do design §4).
