# Despacho — Terminal I (Dev Core) · TRILHA B: Pertencimento, Conflito & Gatilho (eixo core/coordenação)

> Entregue via `lina handoff "@Terminal I"`. Marcador obrigatório no fim: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (onde o trabalho está — PUXE antes de codar)

- **Você trabalha numa git worktree ISOLADA, só sua:** `cd ../lina-f3-4-b` (irmã do repo principal; branch `lina/f3-4-b`). Mudar de branch ou commitar aqui **NÃO afeta** os colegas. **Você COMMITA na sua branch** (cada commit vira um teste vivo do `CodeChanged`). Não faça merge nem push.
- **LEIA primeiro (fonte da verdade):** vault `36 - Spec F3 - Coordenacao de Codigo Multi-Agente` (`lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/36 - Spec F3 - Coordenacao de Codigo Multi-Agente.md"`) §2 (sinal de mudança + pertencimento) e §3 (gatilho do DevOps). Depois o plano `tasks/epico-f3/onda-f3-4.md` e os ADRs propostos `docs/adr/0041-*` (paths no claim) e `docs/adr/0042-*` (gatilho DevOps). Você é o especialista em projeções (fez `mentality.rs`/`attention.rs` recebeu-não-começou) — este eixo é a sua praia.
- **A FUNDAÇÃO já está na sua base** (branch `develop`): `CodeChanged{branch, paths, author_node, commit}` e `BranchIntegrated{branch, into, commit}` já existem em `crates/lina-core/src/events.rs`. **NÃO toque `events.rs`** — é dono do Maestro; se faltar campo num evento, peça ao Maestro (A).
- **Mapa do código (já levantado — confirme com Read):**
  - `PlanItem` em `crates/lina-core/src/plan.rs:90-112` (campos `id`/`desc`/`owner`/`status`/`goal_id`/`parents`/`acceptance`/`budget_tokens`). **NÃO há campo de paths reservados** — o claim hoje reserva só o ID. Você adiciona `#[serde(default)] pub paths: Vec<String>` (ADR 0041) + parser (`parse_item`, plan.rs:446) + render (`@paths:`, perto de plan.rs:363-366, ao lado de `@parents:`). Round-trip byte-a-byte exato (plano antigo sem `paths` reserializa idêntico).
  - `PlanClaimed { id, item, by }` JÁ carrega o `item` completo (router.rs:1973-1980) → com `PlanItem.paths`, a projeção de claims já tem os paths. **Não precisa mudar o evento.**
  - Broadcast por pertencimento JÁ filtra vivos: `Supervisor::resolve` (lib.rs:1564, ramo Broadcast filtra `is_alive()` lib.rs:1577-1585). O gate de fan-out é só na CASCATA (router.rs:1089); a 1ª onda da origem entrega a todos.
  - `route_message` em `router.rs:855`; `handle_plan` em `router.rs:1950` (molde do seu handler novo de `CodeChanged`). O ramo de intent novo (`code.changed`) entra no dispatch de `route_message` (~router.rs:931, espelho de `handle_plan`).
  - `attention.rs`: enum `AttentionKind` em :103-129 (5 kinds: Custody/Permission/Spawn/GuardAsk/DeliveredNoProgress; NÃO é `non_exhaustive`). Precedência = ordem dos `out.extend(...)` em `items()` (attention.rs:763-767). `observe()` dispatch em :320. Struct de pendência a espelhar: `PendingGuardAsk` (:259) + `fold_guard_ask` (:533). `replay()`/`observe_records()` (:447/:458) reaplicam tudo via `from_record` — nada extra.
  - Projeções por replay (molde): `Mentality::replay` (mentality.rs:127, decodifica via `DomainEvent::from_record`, pula erro com `else { continue }` :141), `project_goals` (goal.rs:64), `CostLedger` (router.rs:3831). Lifecycle/status por nó: `ProjectedState.nodes` (events.rs:1397), alimentado por `NodeStatusChanged` (events.rs:1517); constantes de stall em `lifecycle.rs:46-51` (HEARTBEAT 120s, BREAKER 6 amostras).

## 2. FUNÇÃO

Você é o **dono do eixo core/coordenação**: o sinal de mudança roteado por pertencimento, a comparação `paths`×claims, o item de atenção de conflito e o gatilho determinístico do DevOps. Projeções puras sobre o event log — ZERO LLM (inv #1), molde `CostLedger`/`Mentality`.

## 3. DIRECIONAMENTO (as regras do jogo)

- **Mexa SÓ nos seus arquivos (DONO ÚNICO):** `crates/lina-core/src/router.rs`, `crates/lina-core/src/attention.rs`, `crates/lina-core/src/plan.rs`, `crates/lina-core/src/code.rs` (NOVO), `crates/lina-core/src/lib.rs` (só a linha `pub mod code;` + re-export). **NÃO toque** `events.rs` (Maestro), `guard.rs`/`workspace.rs`/`bridge.rs`/`lina.rs` (Trilha A — Terminal B).
- **`author_node`/`paths` é DADO, nunca autoridade (família ADR 0007):** ao processar `CodeChanged`, o `author_node` é o que o supervisor carimbou (não confie em payload). A comparação `paths`×claims só pode **abrir item de atenção** (PARA e pergunta) — NUNCA conceder posse nem autorizar. Sem interseção → ignora em silêncio (nó não muda de estado).
- **Comparação `paths`×claims:** interseção de conjuntos de caminhos (normalize: relativos ao repo). `CodeChanged.paths` ∩ (paths dos itens com claim de OUTRO nó) ≠ ∅ → o nó dono do claim recebe a notificação e PARA. Determinístico, sem LLM.
- **Item de atenção `CodeConflict`:** variante nova em `AttentionKind` (attention.rs:103); struct de pendência espelhando `PendingGuardAsk`; fold em `observe()`; projeção em `items()` com precedência da **família guard-ask** (entre `GuardAsk` e `DeliveredNoProgress`, ou acima se bloqueante — o conflito não-trivial PEDE direção humana, então trate como guard-ask: aprovável/escolha, com `stable_id`, nunca texto/posição). Detalhe leigo: "o colega mexeu em X que você reservou; rebase, renegociar ou desconflitar?".
- **Gatilho determinístico do DevOps (`code.rs`):** projeção pura — `branches_nao_integradas` = `{branch | ∃ CodeChanged{branch} ∧ ∄ BranchIntegrated{branch}}`; gatilho = `branches_nao_integradas ≠ ∅` E `todos os nós Idle/Dead por ≥ T` (lê `ProjectedState.nodes`/lifecycle). Quando dispara, **enfileira um SpawnRequested do papel DevOps** (origem com narração, via o caminho de spawn governado existente — NÃO crie nó você mesmo; o core só decide/dispara). O disparo automático sem humano amadurece depois; nesta onda a projeção + o gatilho são testados por unidade.
- **Replay idempotente:** toda projeção reconstrói por varredura de `EventStore::events()`; nada em `ProjectedState`/`apply()` (isso é META). `code.rs` segue o molde exato do `mentality.rs` (`replay(store) → from_records(&events)`, decodifica com `from_record`, `else { continue }`).
- Convenções: edition 2021; `cargo fmt`/`clippy --all-targets -D warnings` limpos em lina-core; formate só os SEUS arquivos (`rustfmt <arquivo>`, não `cargo fmt -p` que toca hunks de peer); eventos aditivos; sem `unwrap()` em prod.
- **Suíte de segurança do router (router.rs tests) VERDE** — você toca o router; o gate inclui-a. Prove por mutação onde tocar autoridade.

## 4. OBJETIVO (o porquê de negócio)

Quando várias IAs codam ao mesmo tempo, o time precisa saber quem mexeu em quê — e parar antes de duas reescreverem o mesmo arquivo. Esta trilha é o "sistema nervoso" da coordenação: o sinal de mudança chega a quem importa, quem reservou um arquivo é avisado e para, e quando todos terminam o sistema sabe que é hora de juntar. Sem isso, paralelismo vira colisão.

## 5. RESULTADO ESPERADO (formato exato da entrega)

Na sua worktree `../lina-f3-4-b`, commits atômicos por story na branch `lina/f3-4-b`, com:

- **F3-4-3:** `PlanItem.paths` (+ parser/render round-trip exato) + handler de `CodeChanged` no router (broadcast por pertencimento) + comparação `paths`×claims. Teste: nó com claim que intersecta os paths → PARA/atenção; nó sem interseção → estado inalterado.
- **F3-4-4:** `AttentionKind::CodeConflict` + fold + projeção em `items()`. Teste: interseção não-trivial → item de atenção com `stable_id` e precedência guard-ask; replay idempotente.
- **F3-4-5 (core):** `code.rs` — projeção `branches_nao_integradas` + gatilho determinístico (Idle/Dead ≥T + branches pendentes → enfileira SpawnRequested DevOps). Teste: `CodeChanged` sem `BranchIntegrated` ⇒ branch pendente; após `BranchIntegrated` ⇒ some; gatilho dispara só com todos Idle/Dead.
- **Gate verde:** `cargo test -p lina-core` (inclui suíte de segurança do router) + `cargo clippy -p lina-core --all-targets -D warnings` + `cargo fmt -p lina-core --check`. Rode e LEIA o exit code (sem pipe). Reporte os exit codes.

Termine com **`PRONTO: <o que entregou + exit codes observados>`** ou **`BLOCKED: <motivo + o que precisa do Maestro>`**.
