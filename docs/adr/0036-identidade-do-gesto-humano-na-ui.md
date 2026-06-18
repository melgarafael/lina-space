# ADR 0036 — Identidade do gesto humano direto na UI (confirm/correct da Goal)

- **Status:** Proposto (decisão de PORTA; a IMPLEMENTAÇÃO do disparo-UI vem em fatia dedicada).
- **Onda/Story:** F3-1-7 (card da Goal) — escalado pelo Terminal G no wiring (recusou-se a forjar `by`, escalou).
- **Data:** 2026-06-18
- **Fontes:** spec 52 §Seg2 (proíbe escolher `by` no cliente) · família ADR 0007 (campo escrito por agente nunca decide autoridade) · ADR 0021 (aprovação custodiada) · invariante de stack (processo único, GPU-first) · código: `app/lina-gpui/src/goal_card.rs` (`on_confirm`), `crates/lina-core/src/router.rs` (handler `goal.confirm`, `derive_root_hops`), `mailbox.rs` (auth por dir-dono).

> **Gate:** a fatia que IMPLEMENTA o disparo do confirm-humano na UI não inicia sem este ADR aceito.

## Contexto
O card da Goal (F3-1-7) tem um botão "Confirmar" (1 toque). Mas o **workspace-shell não é um nó A2A**: não tem um outbox autenticado por dir-dono de onde o supervisor carimbe `by`. A doutrina (spec 52 §Seg2 + ADR 0007) **proíbe escolher `by` no cliente**. O Terminal G ligou os botões à view mas **recusou-se a forjar `by`** e escalou — corretamente: forjar furaria a segurança da Goal (blindada em 2 camadas nesta rodada — auditoria do gate de onda confirmou).

O handler `goal.confirm` carimba `by` server-side via a identidade autenticada do remetente (`node_by_name(msg.from)`, com `msg.from` reescrito pela origem do outbox). Esse caminho serve **agentes** (que escrevem no outbox). O gesto **humano direto** via UI é diferente: não é um agente — é o **dono do app**.

## Decisão
O **gesto humano direto via UI** (confirmar/corrigir uma Goal — e gestos diretos futuros) carimba `by = "human"` (proveniência de gesto direto, `hops == 0`) por ser uma **chamada LOCAL in-process da UI** — não via outbox. A autenticação **não é uma string forjável**: é o **fato de ser a chamada in-process** (processo único, GPU-first — a UI e o supervisor compartilham o processo; nenhum remoto alcança esse canal).

Concretamente: o supervisor expõe um caminho `human_intent(intent, payload)` (ou reusa o canal de aprovação custodiada, ADR 0021) que a view chama no `on_confirm`; o supervisor carimba `by="human"`, `hops=0`, e emite `GoalConfirmed`. O `by="human"` **só** nasce desse canal local — um agente (via outbox) nunca o produz.

Isto **preserva a doutrina**: `by` continua server-side; a view não ESCOLHE `by` — apenas DISPARA o intent pelo canal local, e o supervisor carimba pela **natureza do canal**.

## Alternativas consideradas
- **(A) Nó "@User" com outbox especial** — cerimônia extra (criar um nó para o humano) sem ganho de segurança; o gesto humano não é um agente.
- **(B) Aprovação custodiada plena (ApprovalWiring, ADR 0021)** — adequada para gestos `GatedHard` (deploy/pay/irreversível externo); o confirm da Goal NÃO é `GatedHard`, então a custódia plena é desproporcional — mas o **princípio do canal in-process autenticado é o mesmo**.

## Consequências
- A fatia de implementação expõe `human_intent` no supervisor + liga `on_confirm`/`on_correct` do card. Consumidor inicial: o card da Goal; a porta serve gestos humanos diretos futuros (corrigir, aprovar leve).
- **Por ora (F3-1):** o confirm da Goal é disparável via `lina goal confirm` (CLI) — autenticado pela origem do outbox do terminal que roda o comando. O **1-toque-UI** vem com esta porta aceita.
