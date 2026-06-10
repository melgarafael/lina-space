# SEAM — verbo `lina spawn` (F1-3-6 · agente-cria-terminal com guardrails)

> **Documento de design do ARQUITETO.** Não é código. O Dev 01 implementa lendo SÓ este doc —
> zero decisão arquitetural no impulso da story. Toda âncora é `arquivo:linha` do código em
> `HEAD = 9d4d336` (verificada na leitura, não inferida).
>
> **Fonte:** `tasks/epico-f1/onda-3.md:150-170` (F1-3-6) · ADR 0019 §6 (spawn caps) · ADR 0007
> (origem/cascata inforjável) · ADR 0006 (WorkspaceTrust) · ADR 0022 (admissão canônica de nó).

---

## 0. Decisão arquitetural central (leia primeiro)

**`lina spawn` é um envelope A2A `intent=spawn` interceptado no router como verbo estruturado —
exatamente o molde de `plan.claim` (`is_plan_intent` → `handle_plan`), NÃO uma entrega a PTY.**

Por quê isto e não um caminho novo:

1. O `(root_cause_id, hops)` EFETIVO **já é computado** para toda mensagem por
   `derive_root_hops(msg, sender)` (`router.rs:1433`), a partir do binding inforjável
   `delivered_root` (`router.rs:265`). O gate de spawn **reusa essa mesma função** — zero binding
   novo, zero superfície de ataque nova. É literalmente o que o ADR 0007 §Consequências prometeu
   ("reaproveita um mecanismo já testado e à prova de forja, sem novo estado").
2. O bin **nunca** toca o Supervisor: `run_ask` escreve um envelope no outbox por-nó
   (`.lina/outbox/<node>/`) e o `pump` do app drena (`router.rs:454`). `spawn` segue idêntico —
   só muda o `intent` e o handler no router. Mantém inv#1 (o bin é cliente burro do filesystem)
   e a doutrina de segurança (o `from` é autenticado pelo dir-dono, não por flag).
3. O Supervisor é a **autoridade única do NodeId** (`Uuid::now_v7()` em `register`,
   `lib.rs:1186`). O router **não** cunha id nem abre PTY (não tem o `writer` do master — isso é
   do app, fronteira core/shell inv#7). Logo `handle_spawn` **decide** (gate) e **devolve um
   outcome**; o app **executa** a criação via o funil `admit_node` (ADR 0022). Mesmo split do
   par detecção(core)→execução(app) já usado na aprovação (memória `lina-f1-1-8`).

**Fronteira:** o gate (decisão inforjável) mora no **core** (`lina-core`); a criação física
(PTY + `register` + bootstrap + append de `NodeAdded`) mora no **app** (`lina-gpui/bridge.rs`),
acionada por um `RouteOutcome` novo. O Dev 01 entrega o core; a fiação do app é a 2ª metade da
story (ver §8 — pode ir na mesma rodada se o Dev 01 tiver acesso ao `bridge.rs`, ou virar seam
de integração).

---

## 1. Fluxo end-to-end

### Caminho feliz (ORIGEM, `hops==0`, em `autonomo`)
```
agente roda:  lina spawn "@QA" --role qa --prompt "valide o checkout"
  │
  ▼  bin (lina-bootstrap/src/bin/lina.rs)
run_spawn → MailMessage::new(from=<eu>, to="@QA", intent="spawn", payload=<prompt>)
            + role no envelope (--role, já parseado por run_ask)  → enqueue_per_node(outbox)
  │
  ▼  app pump → router.route_message  (lina-core/src/router.rs:640)
validate_envelope_v2  (intent "spawn" ∈ CANONICAL_INTENTS_V2 ✔)
is_spawn_intent(msg.intent)  →  self.handle_spawn(msg, sender, store, now_ms)   ◄── NOVO, ~linha 713
  │
  ▼  handle_spawn  (NOVA fn, molde = handle_plan router.rs:1519)
(root, hops) = derive_root_hops(msg, sender)     ◄── INFORJÁVEL (reuso de router.rs:1433)
autonomia: blocks_delegation()? → não (autonomo)
cascata?  hops>=1 → não (origem)
cap:      spawn_count(root,sender) < max_spawns_per_turn (2)? → sim
custo:    CostLedger não-pausado? → sim
  → append SpawnRequested{requested_by=sender, name="@QA", role, root, hops}
  → incrementa spawn_count(root,sender)
  → return RouteOutcome::SpawnApproved{ name, role, prompt, requested_by:sender, root }
  │
  ▼  app (bridge.rs, consumidor do Vec<(id,RouteOutcome)> do pump)
admit_node(NodeAdmission{ name:"@QA", role:"qa", requested_by:Some(sender), engine:<default ws> })
  → wire_terminal → sup.register (NodeId nasce) → admission_events emite
       NodeAdded{requested_by:Some(sender)} + TerminalSpawned + NodeRoleAssigned
  → WorkspaceTrust re-derivada do roster vivo (nó entra automático — ADR 0006/inv#5)
  → enfileira o `prompt` na fila serial do novo nó (1º turno = template F1-3-5)
  → narra ao leigo (inv#6): "Trouxe um especialista de QA pro time porque…"
```

### Caminho gated (CASCATA, `hops>=1`)
```
worker que RECEBEU um handoff (tem binding delivered_root) roda:  lina spawn "@Helper" …
  │  handle_spawn: (root,hops)=derive_root_hops → hops>=1
  → append SpawnGated{requested_by=sender, reason:"cascade"}
  → return RouteOutcome::SpawnGated{ reason:"cascade", … }
  │
  ▼ app: banner de confirmação humana (pt-br). SEM confirmação → nada é criado.
        Com confirmação → o app chama admit_node (autoridade humana; requested_by preservado p/ auditoria).
```

---

## 2. Q1 — Entrada no bin + chamada ao core + alocação do NodeId

### 2.1 Bin (`crates/lina-bootstrap/src/bin/lina.rs`)
| Âncora | O que é | Edição |
|---|---|---|
| `lina.rs:29` `match args.first()` | dispatch manual (sem clap) | **+** `Some("spawn") => run_spawn(&args[1..])` |
| `lina.rs:113` `run_ask` | molde: parseia `--role`(144), `--intent`(124); monta `MailMessage::new`(187); `enqueue_and_report`(206) | **nova** `run_spawn`: parseia `@Nome` + `--role` + `--prompt`; `MailMessage::new(from, "@Nome", "spawn", prompt)`; reusa `enqueue_and_report` |
| `lina.rs:179` `from = load_input().terminal_name` | `from` autenticado pelo **dir-dono** do outbox (não por flag) | reusar — é a 1ª camada da auth de origem |
| `lina.rs:206` `enqueue_and_report` | escreve outbox + faz poll do desfecho real no `log.jsonl` | reusar; mapear os novos desfechos (`SpawnApproved`/`SpawnGated`/`SpawnBlocked`) para mensagens pt-br em `RouteConfirm`/`explain_block` |

**Manual local (doutrina bloco 5):** em `autonomy=manual`, `run_spawn` recusa **no próprio bin**
antes de enfileirar (mesmo mecanismo do handoff bloqueado) — UX imediata; o router é o backstop
durável (§4). O bin lê o nível de autonomia do bootstrap (`load_input`), como já faz hoje.

### 2.2 Alocação do NodeId — **autoridade única, não duplicar** (lição `lina-app-nodeid-autoridade-unica`)
| Âncora | Papel |
|---|---|
| `crates/lina-core/src/lib.rs:1186` `let id = Uuid::now_v7();` (em `Supervisor::register`, 1180) | **ÚNICO** ponto de produção que cunha NodeId. `handle_spawn` NÃO chama register (não tem o `writer`). |
| `app/lina-gpui/src/bridge.rs:3558` `NodeManager::admit_node` | **funil único** de admissão (ADR 0022 §1). É quem o app chama no `SpawnApproved`. |
| `app/lina-gpui/src/bridge.rs:1955` `wire_terminal` | `pty.spawn → take_writer → sup.register(name, role, writer)` → NodeId. |
| `app/lina-gpui/src/bridge.rs:3088` `admission_events` (fn PURA) | monta `NodeAdded + TerminalSpawned + NodeRoleAssigned`. **É aqui que `requested_by` entra** (novo parâmetro). |

> ⚠️ `PtyHost::spawn` (`lib.rs:379`) também cunha UUID mas é **teste-only**; o app usa
> `PtyManager` direto. Não criar um 2º caminho de cunhagem.

---

## 3. Q2 — Evento a REUSAR: `NodeAdded` + `requested_by` (campo aditivo)

**NÃO criar evento paralelo de "nó pedido por agente"** (lição de fidelidade de contrato
`contrato-evento-fidelidade-vs-contorno`). Estende-se `NodeAdded`.

### Enum atual (`crates/lina-core/src/events.rs:186`)
```rust
NodeAdded {
    node: NodeId,
    kind: String,
    x: f64,
    y: f64,
},
```

### Edição (aditiva via `serde(default)` — precedente EXATO: `TerminalSpawned.cwd` em events.rs:228)
```rust
NodeAdded {
    node: NodeId,
    kind: String,
    x: f64,
    y: f64,
    /// F1-3-6 (ADR 0019 §6): NodeId do agente que pediu o spawn via `lina spawn`
    /// (intent=spawn). `None` = criação direta pelo humano (UI/⌘T) — não passa pelo
    /// cap de agente. ADITIVO via `serde(default)`: logs antigos (sem o campo) replayam
    /// com `None`, SEM upcast (mesmo padrão de `TerminalSpawned.cwd`, ADR 0001 §2).
    #[serde(default)]
    requested_by: Option<NodeId>,
},
```

### Upcasting — **NÃO precisa**
- `current_version()` (`events.rs:704`): `NodeAdded` já está em **v2** (ganhou `kind`). Campo
  `Option<T>` com `serde(default)` desserializa de v2-sem-o-campo como `None` — **sem bump para
  v3, sem entrada em `upcast()`** (`events.rs:727`). O upcast só serve para campo *requerido*
  novo (caso `kind` v1→v2). Aditivo-Option dispensa.

### Projeção — toque mínimo
- `apply()` arm de `NodeAdded` (`events.rs:831`) hoje desestrutura `{ node, kind, x, y }`.
  Adicione `requested_by: _` ao padrão (**ignorado na projeção do canvas**). `requested_by` é
  **META/auditoria** (consumido por replay — `lina retro`, painel de custo), como `CostLedger`
  varre o log; **não** entra em `ProjectedNode` (`events.rs:757`) nesta story → mantém o
  `ProjectedNode` intocado (impacto mínimo). Se F1-1/retro precisar projetá-lo, é story própria.
- `kind()` (`events.rs:642`): `NodeAdded` já tem arm (`{ .. }`) — **nada muda** (o `..` cobre o
  campo novo).

### Onde `requested_by` é PRODUZIDO (app)
- `admission_events` (`bridge.rs:3088`) ganha o parâmetro `requested_by: Option<NodeId>` e o
  injeta no `NodeAdded`. `admit_node` (`bridge.rs:3558`) recebe via `NodeAdmission` (novo campo
  `requested_by`, default `None` na porta ⌘T/humana).

---

## 4. Q3 — Gate origem vs cascata pelo binding INFORJÁVEL (o coração da story)

**A regra de ouro (ADR 0007 + doutrina de segurança): nenhum campo escrito pelo agente decide
autorização.** O gate lê o binding interno, **nunca** `msg.hops`/`msg.root_cause_id`.

### O binding vive aqui
| Âncora | Conteúdo |
|---|---|
| `router.rs:265` | `delivered_root: HashMap<NodeId, (String, u8, u64)>` = `(root_cause_id, hops, last_ms)` |
| `router.rs:1433` `derive_root_hops(msg, sender) -> (String, u8)` | **com** binding → `(root, hops+1)`; **sem** binding → `(msg.id, 0)`. ESTA é a função que `handle_spawn` reusa. |
| `router.rs:1063` / `1366` | único ponto que CARIMBA `delivered_root` (insert), só após entrega real + guard P1 anti-envenenamento. É o que torna `hops` inforjável. |
| `router.rs:824` (fanout) / `868` (budget) | precedente vivo do `hops>=1` = cascata. O spawn segue o MESMO predicado. |

### Como o `handle_spawn` lê
```rust
// dentro de handle_spawn — UMA linha, reuso puro:
let (root, hops) = self.derive_root_hops(msg, sender);
// hops == 0  → ORIGEM  (agente a mando direto do humano; sem binding) → permitido + narração
// hops >= 1  → CASCATA (recebeu A2A e quer criar terminal) → SpawnGated SEMPRE (gate humano)
```

> **Não-forjabilidade (critério 3 da story, teste irmão do `f005324`):** um worker que forja
> `root_cause_id` fresco / `hops=0` no payload do outbox **continua gated** — `derive_root_hops`
> ignora o campo e lê `delivered_root[sender]` (que existe porque ele recebeu o handoff). O teste
> é não-vacuoso: quebra se o gate reler `msg.hops`.

### ⚠️ Ordem de interceptação (precisa exata)
`handle_spawn` deve ser interceptado **junto ao `is_plan_intent`** (`router.rs:710-712`),
**logo após** ele, e **antes** dos guardrails de target/hop/retenção — porque:
- o alvo `@Nome` do spawn **não existe ainda** no roster → cairia em `NoTarget` (`router.rs:801`);
- spawn não entrega a PTY (não passa por retenção `738`, close_await `780`, hop-limit `789`).

```rust
// router.rs ~712, IMEDIATAMENTE depois do bloco is_plan_intent:
if is_spawn_intent(&msg.intent) {
    return self.handle_spawn(msg, sender, store, now_ms);
}
```
`is_spawn_intent` = helper trivial ao lado de `is_plan_intent` (`router.rs:1613`):
```rust
fn is_spawn_intent(intent: &str) -> bool { intent == "spawn" }
```
`handle_spawn` chama `derive_root_hops` internamente (como o plano resolve `by`/`item` por conta).
`sender` já está resolvido/autenticado nesse ponto (`router.rs:688`).

---

## 5. Q4 — `max_spawns_per_turn=2`, autonomia e custo

### 5.1 ⚠️ DIVERGÊNCIA story↔código (ESCALAR — não adaptar em silêncio)
A story §3 diz "guardrail novo **no workspace.json** ao lado de `max_delegations_per_turn`".
**Não existe `workspace.json` parseado no core.** O cap de delegação hoje vive em **três** lugares:
- `lina-bootstrap/src/lib.rs:76` `Autonomy::max_delegations()` → `0` manual, `8` demais;
- `RouterConfig.delegation_budget` (`router.rs:69`) — construído em código, default `8`;
- placeholder de doutrina `{{max_delegations_per_turn}}` (`lib.rs:328`).

→ **Recomendação:** tratar "workspace.json" como atalho para "a config de autonomia/cap do
workspace" e plugar `max_spawns_per_turn` nos mesmos 3 pontos (abaixo). **Confirmar com o
Orquestrador** que essa é a leitura correta (a alternativa — criar um parser `workspace.json`
de verdade — é escopo maior e não está na story).

### 5.2 Onde plugar (3 pontos + 1 fix de gap)
| Âncora | Edição |
|---|---|
| `router.rs:67` `struct RouterConfig` | **+** `pub max_spawns_per_turn: u32,` (ao lado de `delegation_budget`) |
| `router.rs:118` `impl Default` | **+** `max_spawns_per_turn: MAX_SPAWNS_PER_TURN` (nova const `= 2`, ao lado de `DELEGATION_BUDGET`, `router.rs:30`) |
| `lina-bootstrap/src/lib.rs:76` `Autonomy::max_delegations()` | **+** `Autonomy::max_spawns()` espelhado (`0` manual, `2` demais) |
| `lina-bootstrap/src/lib.rs:328` `render_template` | **+** `.replace("{{max_spawns_per_turn}}", &max_spawns)` (a doutrina F1-3-6 da skill usa) |
| **GAP** `app/lina-gpui/src/bridge.rs:653` | hoje cria `RouterConfig { token_budget_day, ..default() }` — **sem `autonomy`!** A matriz de autonomia NÃO está fiada no router em produção. O spawn exige fiar `autonomy: AutonomyLevel` (convertido de `lina_bootstrap::Autonomy`) **e** `max_spawns_per_turn`. Escalar como dependência de integração. |

### 5.3 Contagem por `root_cause_id` — reusar a infra do budget
| Âncora | Reuso |
|---|---|
| `router.rs:257` `budget: HashMap<(String, NodeId), BudgetEntry>` (chave `(root, sender)`) | mesma chave do spawn |
| `router.rs:231` `struct BudgetEntry { count, last_ms }` | **+** campo `spawn_count: u32` |
| `router.rs:1003-1013` (increment) | template do increment do spawn |
| `router.rs:1397` `prune_expired` | **já poda `BudgetEntry`** — estender o campo evita um 2º mapa com poda própria |

**Recomendação:** estender `BudgetEntry` com `spawn_count` (reusa poda temporal A5), check/increment
**separados** do `count` de delegação (semânticas distintas: cap 2 vs budget 8). No `handle_spawn`:
```rust
let n = self.budget.get(&(root.clone(), sender)).map(|e| e.spawn_count).unwrap_or(0);
if n >= self.config.max_spawns_per_turn { /* append SpawnGated{over_cap} → SpawnGated */ }
// no ALLOW: budget.entry((root,sender)).or_default().spawn_count += 1;
```
> Nota: a origem (`hops==0`) ganha `root = msg.id` FRESCO a cada `lina spawn` → o cap conta
> spawns **por turno de origem**. 2 spawns no MESMO turno (mesmo root herdado/fresco) → 3º gated.

### 5.4 Custo (ADR 0005 / W3-7c)
- `handle_spawn` respeita o teto reusando a checagem de `CostLedger` já existente (`router.rs:842`,
  `CostLedger::replay` `router.rs:1847`): workspace `Paused` → `SpawnGated{reason:"cost"}`. O
  terminal novo conta no ledger naturalmente (seus `TokenUsageReported` somam) — **sem fiação de
  custo extra**.

### 5.5 Autonomia (`router.rs:48-62`)
- `AutonomyLevel::blocks_delegation()` (`router.rs:60`, só `Manual`) → `handle_spawn` em manual →
  `SpawnBlocked` (recusa terminal). `assistido`→propõe / `autonomo`→segue é doutrina da skill
  (camada do agente), o router só enforce o **manual-block** + **cascata-sempre-gate** + **cap**.

---

## 6. Q5 — Eventos novos da onda + `RouteOutcome` (campos mínimos)

### Eventos (`events.rs` — ambos META, vão no bloco de silêncio do `apply()` 919-978, e ganham arm em `kind()` 642)
```rust
/// F1-3-6: um agente PEDIU criar um terminal via `lina spawn`. META (livro-razão do pedido,
/// intent-vs-action 13.14 P1) — registra args reais + a cadeia. Precede a decisão do gate.
SpawnRequested {
    id: String,            // = msg.id (dedupe/correlação)
    requested_by: NodeId,  // sender AUTENTICADO (jamais o campo from)
    name: String,          // @Nome pedido
    role: String,          // papel pedido
    root_cause_id: String, // root EFETIVO (derive_root_hops)
    hops: u8,              // profundidade EFETIVA (0=origem, >=1=cascata)
},
/// F1-3-6: o gate barrou/adiou um spawn. META (livro-razão da decisão). reason ∈
/// {"cascade","over_cap","manual","cost"}.
SpawnGated {
    id: String,
    requested_by: NodeId,
    reason: String,
},
```
> `NodeAdded.requested_by` (§3) fecha o trio: REQUESTED (pedido) → GATED (barrado) **ou** ADDED
> (criado, com quem pediu). Auditoria intent-vs-action completa, tudo por replay.

### `RouteOutcome` (`router.rs:141`) — 3 desfechos novos
```rust
/// F1-3-6: spawn aprovado (origem, sob cap, custo ok). O APP executa a criação (admit_node).
SpawnApproved { name: String, role: String, prompt: String, requested_by: NodeId, root_cause_id: String },
/// F1-3-6: spawn precisa de confirmação humana (cascata / cap estourado / custo) — banner, nada criado sem o sim.
SpawnGated { reason: String },
/// F1-3-6: spawn recusado pela autonomia `manual` (paridade com handoff bloqueado).
SpawnBlocked,
```
> `enqueue_and_report`/`poll_route_outcome` no bin mapeiam os 3 para mensagens pt-br (reusa o
> padrão `RouteConfirm` + `explain_block`/`block_hint` de `lina.rs:206`).

---

## 7. Esqueleto de `handle_spawn` (molde = `handle_plan` router.rs:1519)

```rust
/// F1-3-6: gate inforjável do spawn. NÃO cunha NodeId nem abre PTY (autoridade do Supervisor/app);
/// DECIDE (origem/cascata/cap/custo/autonomia) e devolve o outcome que o app executa.
fn handle_spawn(&mut self, msg: &MailMessage, sender: NodeId,
                store: &mut EventStore, now_ms: u64) -> RouteOutcome {
    // 0) args (name/role/prompt vêm do envelope; validar não-vazios → SpawnBlocked com motivo legível)
    // 1) BINDING INFORJÁVEL (reuso puro):
    let (root, hops) = self.derive_root_hops(msg, sender);
    // 2) intent-vs-action: loga o PEDIDO sempre (antes da decisão)
    let _ = store.append(&DomainEvent::SpawnRequested { id: msg.id.clone(), requested_by: sender,
        name, role, root_cause_id: root.clone(), hops });
    // 3) autonomia manual → recusa
    if self.config.autonomy.blocks_delegation() { return RouteOutcome::SpawnBlocked; }
    // 4) cascata → gate humano SEMPRE
    if hops >= 1 { append SpawnGated{cascade}; return RouteOutcome::SpawnGated{reason:"cascade"}; }
    // 5) cap por (root,sender) → gate
    if spawn_count(root,sender) >= self.config.max_spawns_per_turn { append SpawnGated{over_cap}; return …; }
    // 6) custo (CostLedger::replay, igual router.rs:842) → gate
    if cost_paused { append SpawnGated{cost}; return RouteOutcome::SpawnGated{reason:"cost"}; }
    // 7) ALLOW: incrementa spawn_count; devolve SpawnApproved p/ o app criar
    budget.entry((root,sender)).or_default().spawn_count += 1;
    RouteOutcome::SpawnApproved { name, role, prompt, requested_by: sender, root_cause_id: root }
}
```
**Regra do livro-razão (igual handle_plan):** logar o evento ANTES do efeito; falha de `append`
→ `PersistFailed` (não decidir o que não logamos). `SpawnRequested` sempre; `SpawnGated` no ramo;
`NodeAdded` só na execução (app).

---

## 8. Fronteira de arquivos — o que o Dev 01 toca

### CORE (Dev 01 desta rodada — `lina-core` + `lina-bootstrap`)
- `crates/lina-core/src/mailbox.rs:33` — `CANONICAL_INTENTS_V2` **+** `"spawn"` (senão `validate_envelope_v2` recusa como `InvalidIntent`).
- `crates/lina-core/src/events.rs` — `NodeAdded.requested_by` + `SpawnRequested` + `SpawnGated` (enum, `kind()`, `apply()` silêncio). **COSTURA — ver §9.**
- `crates/lina-core/src/router.rs` — `is_spawn_intent` (~1615); interceptação (~713); `handle_spawn` (~perto de 1519); `RouterConfig.max_spawns_per_turn` (67/118) + const (30); `BudgetEntry.spawn_count` (231); `RouteOutcome::{SpawnApproved,SpawnGated,SpawnBlocked}` (141).
- `crates/lina-bootstrap/src/lib.rs:76,328` — `Autonomy::max_spawns()` + placeholder.
- `crates/lina-bootstrap/src/bin/lina.rs:29,113,206` — dispatch + `run_spawn` + mapeamento dos desfechos.

### APP (2ª metade — `lina-gpui`; mesma rodada se houver acesso, senão seam de integração)
- `app/lina-gpui/src/bridge.rs:3088` `admission_events` — parâmetro `requested_by`.
- `app/lina-gpui/src/bridge.rs:3558` `admit_node` / `NodeAdmission` — campo `requested_by`.
- `app/lina-gpui/src/bridge.rs:653` — **fix do gap**: passar `autonomy` + `max_spawns_per_turn` ao `RouterConfig`.
- consumidor do `pump` (onde inspeciona `Vec<(id,RouteOutcome)>`) — tratar `SpawnApproved` (→ `admit_node` + enfileira 1º prompt + narra) e `SpawnGated` (→ banner humano).

---

## 9. Q6 — Coordenação de costura `events.rs` (dono único por rodada)

`events.rs` é **dono único por rodada** (CLAUDE.md). Nesta onda **dois** querem mexer:
- **F1-3-6** (este): `NodeAdded.requested_by` + `SpawnRequested` + `SpawnGated`.
- **F1-3-7** (`lina retro`, onda-3.md:179): `SkillInvoked` + `SkillCreated`.

Ambos tocam os **mesmos 3 sítios exaustivos**: corpo do enum, `kind()` (`events.rs:642`),
`apply()` (`events.rs:815`). Todas as adições são **puramente aditivas** (variantes novas distintas
+ 1 campo Option) — sem colisão semântica, mas **colisão de merge** se editadas em paralelo
(lição `specs-irmaos-publicados-em-paralelo-divergem`).

**Proposta de sequenciamento (preferida):** cada story é dona de `events.rs` **na sua própria
rodada**. F1-3-6 roda primeiro (é a bloqueante — tem o gate + ADR 0019 como pré-requisito), lança
suas 3 adições; F1-3-7 lança `SkillInvoked/SkillCreated` na rodada seguinte, sem reabrir nada do
spawn (append-only em pontos distintos). Zero edição concorrente.

**Fallback (se o Orquestrador rodar as duas em paralelo):** **Dev 01 é o dono de `events.rs` nesta
rodada e batcheia AS QUATRO variantes + o campo num único commit** (todas append-only no fim do
enum + arms em `kind()`/`apply()`-silêncio), e o Dev 02 (F1-3-7) **não toca `events.rs`** — só
consome `SkillInvoked/SkillCreated` no `lina retro`. Decisão do Orquestrador; este doc só expõe a
costura para ela ser tomada antes do código, não no impulso.

---

## 10. Testes que provam o critério (gate `gate_f103`/`gate_onda3` — molde existente)
1. **Origem feliz:** `lina spawn` de nó sem binding → `SpawnApproved`; app cria; novo nó no roster + `NodeAdded{requested_by:Some}` + bootstrap íntegro + 1º prompt entregue. (critério 1)
2. **Cascata gated:** semear `delivered_root[sender]` (entrega real) → `lina spawn` → `SpawnGated{cascade}` + evento; nada criado sem confirmação. (critério 2)
3. **Não-forjabilidade (irmão do `f005324`):** com binding presente, forjar `hops=0`/root fresco no payload → AINDA `SpawnGated`. **Não-vacuoso:** quebra se o gate reler `msg.hops`. (critério 3)
4. **Cap:** 3º spawn no mesmo root/turno → `SpawnGated{over_cap}`; `manual` → `SpawnBlocked`. (critério 4)
5. **Replay/upcast:** log v2 antigo (`NodeAdded` sem `requested_by`) replaya com `None`; projeção não quebra. (inv#4)
6. **Red-team próprio ANTES do externo** (lição `gate-humano-campos-controlaveis-e-redteam`): nenhum campo escrito pelo agente (`from`/`name`/`role`/`hops`/`root`) decide origem/cap/autorização. (critério 5)

---

## 11. Escalações ao Orquestrador — ✅ RESOLVIDAS (ver §12)
1. **`workspace.json` (§5.1):** confirmar que "workspace.json" = a config de autonomia/cap (hoje `Autonomy` + `RouterConfig` + placeholder), não um parser novo.
2. **Gap de autonomia (§5.2):** `bridge.rs:653` não fia `autonomy` no `RouterConfig` — o spawn depende disso. Aprovar tocar `bridge.rs` nesta rodada (fronteira do app).
3. **Costura `events.rs` (§9):** sequenciar F1-3-6 antes de F1-3-7, ou batchear as 4 variantes com Dev 01 como dono único?
4. **Fronteira app:** a 2ª metade (admit_node/banner) entra nesta rodada (Dev 01 com acesso a `bridge.rs`) ou vira seam de integração separado?

---

## 12. Decisões do Maestro (2026-06-10) — VINCULANTES para a Rodada 3

O Orquestrador resolveu as 4 escalações. **Estas decisões vencem o corpo do doc onde houver
ambiguidade** e definem o escopo exato da Rodada 3.

### (1) `workspace.json` → NÃO criar parser novo (evita scope creep / porta nova)
- `max_spawns_per_turn` mora **onde `max_delegations` vive hoje**: `RouterConfig` (+ const) e
  `Autonomy::max_spawns()` — exatamente o "ao lado de `max_delegations`" do ADR 0019 §6.
- **Anula** qualquer leitura de §5.1 que sugira um arquivo `workspace.json` parseado. Plugue nos
  3 pontos da tabela §5.2 (`router.rs:67/118/30` + `bootstrap/lib.rs:76/328`). Sem novo arquivo.

### (2) Fiação da autonomia → PRÉ-REQUISITO do gate, dentro do CORE da Rodada 3
- Hoje `bridge.rs:653` cria `RouterConfig { token_budget_day, ..default() }` → `autonomy` fica no
  default `Assisted` (`router.rs:125`). **Sem fiar a autonomia real, o gate manual/cascata é
  fake.** Logo, **F1-3-6 fia a matriz como pré-requisito** (parte do core, pequeno).
- **Ponto de fiação a entregar:**
  - `app/lina-gpui/src/bridge.rs:653` — passar `autonomy: <conv>` (e `max_spawns_per_turn`) ao
    `RouterConfig` em vez de só `token_budget_day`.
  - **Conversão** `lina_bootstrap::Autonomy` → `lina_core::AutonomyLevel` (o core não depende do
    bootstrap — `router.rs:45-46` deixa explícito "o app traduz"): a tradução vive no app
    (`MailboxPump::new`, onde o `RouterConfig` nasce). Mapear `Manual→Manual`,
    `Assisted→Assisted`, `Autonomous→Autonomous`.
  - A origem do nível de autonomia do workspace é o `bootstrap.json`/`BootstrapInput.autonomy`
    (mesma fonte que hoje alimenta `{{max_delegations_per_turn}}`).
- **Nuance de Rodada:** a decisão (4) põe o `RouterConfig`-wiring (que toca `bridge.rs`) na
  fronteira do app. Como a fiação da autonomia É no `bridge.rs` mas é **pré-requisito do gate**,
  ela entra na Rodada 3 como **fatia mínima de core-enabling** (só o `RouterConfig {autonomy,
  max_spawns_per_turn}` + a conversão), separada do `admit_node`/banner (esses sim ficam no seam
  da tela). Provar por teste headless: `RouterConfig` com `autonomy=Manual` → `handle_spawn`
  retorna `SpawnBlocked` (não depende de PTY).

### (3) Costura `events.rs` → Dev 01 é DONO ÚNICO na Rodada 3, todas as variantes UP-FRONT
- **`events.rs` primeiro.** Dev 01 adiciona, numa única fatia, **TODAS** as variantes da onda —
  as de F1-3-6 **e** as de F1-3-7:
  - `NodeAdded.requested_by: Option<NodeId>` (§3)
  - `SpawnRequested`, `SpawnGated` (§6)
  - `SkillInvoked`, `SkillCreated` (de F1-3-7 — reserve já, append-only, ver onda-3.md:179)
  - `RouteOutcome::{SpawnApproved, SpawnGated, SpawnBlocked}` (§6)
- Todas append-only (variantes novas distintas) + arms em `kind()` (`events.rs:642`) e bloco-
  silêncio do `apply()` (`events.rs:919`); `NodeAdded` apply-arm (`831`) ganha `requested_by: _`.
- **Dev 02 (F1-3-7 `lina retro`) NÃO toca `events.rs`** — só projeção/verbo **consumindo**
  `SkillInvoked`/`SkillCreated`. Elimina a colisão de merge (lição
  `specs-irmaos-publicados-em-paralelo-divergem`). **Anula** a opção "sequenciar rodadas" de §9:
  a escolha é o batch up-front com dono único.
- `SkillInvoked`/`SkillCreated`: campos mínimos ficam a cargo do spec de F1-3-7, mas Dev 01
  precisa do **shape** para declará-las. Sugestão de placeholder (confirmar com LLM Engineer/Dev 02
  antes de cravar): `SkillInvoked { node: NodeId, skill: String }` · `SkillCreated { node: NodeId,
  skill: String }` — ambas META (silêncio no `apply`). Se o spec de F1-3-7 ainda não fixou os
  campos, **escalar para o Orquestrador casar os dois specs ANTES da fatia** (não cravar shape de
  peer no escuro).

### (4) Fronteira app → Rodada 3 é CORE-só; `admit_node`+banner = seam SEPARADO (casado à tela)
- **Entra na Rodada 3 (core, testável headless):**
  - `CANONICAL_INTENTS_V2 += "spawn"` (`mailbox.rs:33`).
  - `is_spawn_intent` + interceptação (`router.rs:~713`) + `handle_spawn` (gate completo:
    binding/autonomia/cascata/cap/custo) retornando os `RouteOutcome::Spawn*`.
  - eventos (§6) + `RouterConfig.max_spawns_per_turn` + `BudgetEntry.spawn_count`.
  - `Autonomy::max_spawns()` + placeholder de doutrina.
  - a fatia mínima de fiação da autonomia no `RouterConfig` (decisão 2).
  - `run_spawn` no bin (`lina.rs`).
  - **Testes headless** que provam o gate inforjável (irmão `f005324`), cap, cascata, manual —
    **sem tela** (o `RouteOutcome` é o ponto de prova; não precisa de `admit_node`).
- **Fica no seam SEPARADO (próxima rodada, casado com a tela do fundador):**
  - `admit_node`/`NodeAdmission.requested_by` + `admission_events(requested_by)` (`bridge.rs:3088/3558`).
  - consumidor do `pump` que age no `SpawnApproved` (cria) / `SpawnGated` (banner humano) + narração.
  - a entrega do 1º prompt na fila serial do novo nó.
- **Implicação de teste:** na Rodada 3, `SpawnApproved` é verificado como **valor de retorno**
  (o nó NÃO é criado de fato — isso é o seam da tela). Os testes asseguram a DECISÃO, não o efeito
  físico. Honestidade (lição `feature-ui-visibilidade-vs-logica`): o doc do gate diz explicitamente
  que "criou o terminal" só se prova no seam seguinte.

> **Resumo do escopo da Rodada 3 (Dev 01):** `events.rs` (todas as variantes, dono único) +
> `router.rs` (gate `handle_spawn` + config + outcomes) + `mailbox.rs` (intent canônico) +
> `bootstrap/lib.rs` (cap + placeholder) + `lina.rs` (`run_spawn`) + fatia mínima de fiação da
> autonomia no `RouterConfig`. **Tudo provado headless.** `admit_node`+banner+narração = seam da tela.
