# F3-0-7 — Suíte adversarial de SystemParams (spec de teste, pt-br)

> **Story:** F3-0-7 (onda F3-0, épico F3 "O Maestro Nativo"). **Papel:** QA/Red-team.
> **Status deste arquivo:** SPEC de teste — **não é `.rs` ainda** de propósito. Os 3 testes
> dependem de tipos que a F3-0-1 (`params.rs`: `SystemParams`/`Effort`/`resolve`/clamp/`ParamWarning`)
> e a F3-0-2 (`resolve_from_store` + projeção do `RouterConfig` por replay) e a F3-0-4
> (`EffortAssigned` + carimbo server-side) ainda não mergearam. Escrever `.rs` agora quebraria a
> compilação. **Um dev implementa direto a partir daqui** assim que F3-0-1/0-2/0-4 existirem.
> **Quem escreveu não commita** (protocolo F3-0). 
>
> **Fonte da verdade:** `51 - SPEC SystemParams` (vault), §Segurança e §Critérios de aceite.
> **Regra-mãe (a coisa que estes testes provam):** *parâmetro é DADO transportado, JAMAIS
> autoridade* — nenhum campo escrito por um agente decide identidade, ordem, autorização **nem
> magnitude de segurança**, e nenhum valor de `params.json` derruba o processo ou rebaixa um gate
> humano (família ADR 0007; doutrina de segurança do `CLAUDE.md` do projeto).

---

## 0. Baseline de segurança — VERDE (não pode regredir)

A entrega 1 da F3-0-7 é confirmar que a suíte de segurança **existente** do router está verde
**antes** de qualquer parâmetro novo entrar. Rodado em `HEAD = bebdc1b`:

```
$ cargo test -p lina-core --lib -- \
    delegation_budget_is_enforced_per_root_cause \
    cascade_broadcast_over_fanout_gate_still_gated \
    spawn_cascade_requires_human_gate

running 3 tests
test router::tests::spawn_cascade_requires_human_gate ... ok
test router::tests::cascade_broadcast_over_fanout_gate_still_gated ... ok
test router::tests::delegation_budget_is_enforced_per_root_cause ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 323 filtered out; finished in 0.04s
```

**Contrato de não-regressão:** estes 3 testes (e os irmãos `spawn_gate_ignores_forged_hops_and_root`
`router.rs:5106`, `forged_hops_are_ignored` `:2720`, `origin_delegations_are_not_limited_by_budget`
`:2970`) **continuam verdes com QUALQUER `params.json`**. Nenhum override pode transformar cascata
em origem, dispensar o aval humano de spawn, nem afrouxar o binding inforjável. Os 3 testes novos
abaixo são **aditivos** — não tocam a lógica de produção, só leem.

---

## 1. Convenções de teste (helpers já existentes no módulo `router::tests`)

Os testes novos reusam o ferramental do `#[cfg(test)]` de `router.rs` (e de `params.rs` quando F3-0-1
existir). Inventário confirmado no código atual:

| Helper | Assinatura / efeito | Origem |
|---|---|---|
| `router_with("slug")` | `-> (Router, Arc<Supervisor>, PathBuf)` — router com `RouterConfig::default()` | `router.rs` tests |
| `Router::with_config(sup, mailbox, cfg)` | injeta um `RouterConfig` arbitrário (usado p/ autonomia/params) | `router.rs:398` |
| `sup.register("@X", None, sink())` | `-> NodeId` (binding server-side; a autoridade real) | `router.rs` tests |
| `TmpStore::new("slug")` | event store temporário (`ts.store`) | `router.rs` tests |
| `recorder()` | `-> (rec: Rc<RefCell<Vec<_>>>, deliver)` — conta entregas REAIS | `router.rs` tests |
| `MailMessage::new(from,to,intent,body)` | + `.with_ref("role:helper")`, campos públicos `.hops`/`.root_cause_id`/`.id` **(forjáveis — é o ponto)** | `router.rs` |
| `router.route_message(&m, &mut store, t_ms, &mut deliver)` | `-> RouteOutcome` | `router.rs` |
| `blocked_reasons(&store)` | `-> Vec<String>` (os `reason` snake_case de todo `RouteBlocked`) | `router.rs:2420` |
| `spawn_payloads(&store, "Kind")` | `-> Vec<serde_json::Value>` (payloads de eventos por kind) | `router.rs` tests |
| `router.budget` | `HashMap<(root:String, NodeId), BudgetEntry{count, spawn_count, last_ms}>` | `router.rs` |
| `classify(cmd) -> ActionClass` | `Routine < GatedSoft < GatedHard`; matriz `GatedHard -> Decision::Ask` | `guard.rs:148,204` |

**Faixas canônicas** (spec 51 §1, a fonte do clamp): `delegation_budget` default 8, **faixa `1..=64`**;
`breaker_threshold` default 5, **faixa `1..=20`**; `stall_breaker_samples` default 6, **faixa `2..=40`**;
`fanout_gate` default 3, faixa `1..=16`.

**Doutrina de não-vacuosidade (obrigatória em cada teste):** todo teste adversarial tem de **falhar
contra uma implementação ingênua** — exatamente como os testes baseline declaram *"NÃO-VACUOSO:
quebraria se o gate relesse `msg.hops`"*. Cada caso abaixo traz a sua cláusula "quebra se…".

---

## 2. Teste A — `params_json_out_of_range_clamps_with_warning_no_panic`

**Arquivo destino:** `crates/lina-core/tests/f3_0_params_adversarial.rs` (teste de integração) **ou**
`#[cfg(test)]` de `params.rs`. Depende de **F3-0-1** (`SystemParams::resolve`/clamp/`ParamWarning`).

**Regra-mãe provada:** §Segurança invariante 4 + Critério de aceite 3 — *faixa enforçada sem panic:
valor fora da faixa no load → clamp + WARN, o processo sobe e roteia* (postura best-effort do código).

**Por que prova a regra-mãe:** um número escrito por quem editou o `params.json` (expert OU um agente
que conseguisse escrever no arquivo) **nunca** pode derrubar o processo nem entrar cru no
`RouterConfig` — o valor é saneado (clamp à faixa) e a anomalia é registrada (WARN), jamais fatal.

### Setup
1. `params.json` (camada workspace) com `delegation_budget = 9999` (acima do teto 64).
2. `let (cfg, warns) = SystemParams::resolve(global, workspace, preset, terminal, Autonomous);`
   onde `workspace.delegation_budget = Some(9999)` e as demais camadas vazias.
3. Montar `Router::with_config(sup, mailbox, cfg)` e registrar `@A`,`@B`.

### Asserção exata
```text
assert_eq!(cfg.delegation_budget, 64, "9999 clampa para o teto da faixa, não entra cru");
assert_eq!(warns.len(), 1, "exatamente 1 WARN — o clamp é registrado, não silencioso");
assert_eq!(warns[0].key, "delegation_budget");
assert!(warns[0].to_string().contains("9999") && warns[0].to_string().contains("64"),
        "o WARN nomeia o valor pedido e o aplicado (auditável)");
// ZERO panic, prova viva: o router construído com cfg clampado ROTEIA uma msg normal.
let m = MailMessage::new("@A", "@B", "ask", "vivo");
assert!(matches!(router.route_message(&m, &mut ts.store, 1, &mut deliver),
                 RouteOutcome::Delivered { .. }),
        "o processo sobe e roteia com o valor clampado — sem panic");
```

### Vetores de borda (red-team — incluir como sub-casos `#[test]` ou asserts no mesmo teste)
- **Piso, não só teto:** `delegation_budget = 0` → clamp para **1** (faixa `1..=64`) + 1 WARN.
  *Desligar o freio do laço por baixo é tão perigoso quanto estourá-lo por cima.*
- **Exatamente no teto não gera ruído:** `delegation_budget = 64` → `cfg == 64`, **`warns.len() == 0`**
  (clamp é idempotente na borda; WARN só quando o valor REALMENTE muda — espelha o anti-amplificação
  do `RouteBlocked`).
- **Várias chaves fora da faixa → várias WARN distintas, nenhuma fatal:** `delegation_budget=9999`
  + `breaker_threshold=0` (faixa `1..=20`) → `cfg.delegation_budget==64`, `cfg.breaker_threshold==1`,
  `warns.len()==2`. O load NUNCA aborta no primeiro erro.
- **JSON corrompido/campo desconhecido:** graças a `#[serde(default)]` (aditivo), um `params.json`
  com chave inexistente **carrega** (campo ignorado), não quebra — replay de config antiga é seguro.

**Quebra se…** a implementação ingênua fizer `cfg.delegation_budget = raw` (sem clamp) → `9999` vaza;
ou der `panic!`/`unwrap()` no valor fora da faixa; ou clampar **em silêncio** (`warns.len()==0`).

> **Nota de canal (decisão de F3-0-1):** a spec 51 fala em "1 item WARN na **fila de atenção**".
> A `AttentionQueue` atual (`attention.rs:90`) tem kinds `Custody/Permission/Spawn/GuardAsk` — o WARN
> de parâmetro precisa de um canal (novo `AttentionKind::ParamWarn` **ou** `ParamWarning` retornado
> por `resolve` e empurrado na fila pelo seam `bridge.rs`). O **contrato observável** deste teste é a
> contagem de WARN + o valor clampado + zero panic; o veículo exato é escolha do implementador de
> F3-0-1, e o teste deve assertar pelo contrato, não pelo veículo.

---

## 3. Teste B — `agent_written_authority_fields_are_ignored_server_side`

**Arquivo destino:** `#[cfg(test)]` de `router.rs` ou `events.rs` (onde mora o carimbo). Depende de
**F3-0-4** (`EffortAssigned` + carimbo) e **F3-0-2** (projeção `SystemParamsChanged`). **É o teste
central da regra-mãe.**

**Regra-mãe provada:** §Segurança invariante 1 + Critério de aceite 6 — *`model`/`effort`/`scope`/`by`/
`task_difficulty` escritos no payload por um agente são dado transportado, jamais autoridade; o
carimbo efetivo vem do binding server-side, como `requested_by`/`root_cause_id` em `SpawnRequested`
já fazem (`events.rs:716,722`).*

**Por que prova a regra-mãe (e é o irmão direto de `spawn_cascade_requires_human_gate` `:5093` e
`spawn_gate_ignores_forged_hops_and_root` `:5129`):** um agente em cascata escreve campos PÚBLICOS
no envelope tentando (i) **forjar autoridade** (`by:"human"` — passar-se por humano) e (ii) **forjar
magnitude** (`task_difficulty:"hard"`/`effort:"high"` — auto-atribuir mais raciocínio/tokens). Ambos
têm de ser **descartados**; o evento EFETIVO no log carrega o carimbo derivado do binding.

### Vetor B1 — forja de AUTORIDADE (`by`)
**Setup:** `@A → @B` (fixa o binding de B; B agora está em cadeia). B emite uma mudança de parâmetro
com `by` forjado no payload:
```text
let m_ab = MailMessage::new("@A","@B","ask","cadeia");
router.route_message(&m_ab, &mut ts.store, 1, &mut deliver);   // binding de B

// B tenta aplicar um SystemParamsChanged se declarando "human".
let mut m = MailMessage::new("@B","@workspace","param_set","fanout_gate=8")
              .with_ref("by:human");          // ← FORJADO no payload
// (ou o campo equivalente do envelope/evento que F3-0-2 definir)
let out = router.route_message(&m, &mut ts.store, 2, &mut deliver);
```
**Asserção exata:**
```text
let ev = spawn_payloads(&ts.store, "SystemParamsChanged").pop().expect("evento");
assert_eq!(ev["by"].as_str(), Some("maestro"),
    "by EFETIVO = carimbo server-side derivado do sender (B, em cascata), NUNCA o 'human' forjado");
assert_ne!(ev["by"].as_str(), Some("human"),
    "o campo do payload do agente é IGNORADO — não vira autoridade");
```
*(O valor canônico do carimbo — `"maestro"` p/ um agente, `"human"` só p/ o gesto humano real, ou
`by:Some("preset:<slug>")` — é o que F3-0-2/0-4 fixarem; a asserção dura é: **`by` ≠ o que o agente
escreveu**, e **`by` = a identidade autenticada do sender**.)*

### Vetor B2 — forja de MAGNITUDE (`effort`/`task_difficulty`)
**Setup:** mesmo binding. Um `EffortAssigned` com `task_difficulty`/`effort` auto-declarado pelo nó:
```text
let mut m = MailMessage::new("@B","@B","effort_self","high")
              .with_ref("task_difficulty:hard");   // ← agente se auto-atribui effort alto
router.route_message(&m, &mut ts.store, 3, &mut deliver);
```
**Asserção exata:**
```text
let ev = spawn_payloads(&ts.store, "EffortAssigned").pop().expect("evento");
// Auto-atribuição por um agente NÃO produz origin:"assigned" com autoridade — ou é rejeitada,
// ou é carimbada origin:"observed" (leitura), nunca uma atribuição forjada que gaste tokens.
assert_ne!(ev["origin"].as_str(), Some("assigned"),
    "um nó não pode se AUTO-ATRIBUIR effort (gastar mais) — assigned exige carimbo do Maestro/humano");
assert!(ev.get("task_difficulty").is_none() || ev["task_difficulty"].is_null(),
    "task_difficulty é carimbo do lado da atribuição (server-side), não lido do payload do agente");
```

### Vetor B3 — `scope`/`target` forjado (escalada de camada)
Um agente tenta aplicar um param na camada `global`/`workspace` (precedência baixa, afeta todos)
forjando `scope:"workspace"` num set que deveria ser, no máximo, da sua própria camada `terminal`.
**Asserção:** `ev["scope"]` e `ev["target"]` vêm do binding (o nó só pode opinar na própria camada
`terminal`), nunca do `scope` escrito no payload. *Espelha `gated[0]["requested_by"] == B` do baseline.*

**Quebra se…** o projetor de `SystemParamsChanged`/`EffortAssigned` fizer
`by = payload.by` / `task_difficulty = payload.task_difficulty` / `scope = payload.scope` (ler o campo
do agente) em vez de carimbar a partir do binding autenticado. **Não-vacuoso por construção:** com o
carimbo correto o teste passa; com a leitura ingênua, falha em todos os 3 vetores.

---

## 4. Teste C — `weakening_a_balancing_loop_is_gated_hard`

**Arquivo destino:** `#[cfg(test)]` de `params.rs` (a classificação) + cobertura do caminho
`lina params set`. Depende de **F3-0-1** (a função de classificação) e da fiação do gate.

**Regra-mãe provada:** §Segurança invariante 3 — *reduzir `delegation_budget`, `breaker_threshold`
ou `stall_breaker_samples` abaixo do default é `GatedHard` (gate humano com aviso de consequência —
herda `guard.rs`); o sistema resiste à tentação de Meadows de desligar o termostato que "nunca dispara".*

**Por que prova a regra-mãe:** afrouxar um laço **balanceador** (os breakers/anti-loop que protegem
o sistema) é uma ação de impacto de segurança disfarçada de "ajuste de número". Tem de subir ao mesmo
gate humano de uma ação destrutiva (`GatedHard → Decision::Ask`, `guard.rs:209`) — **mesmo em
autônomo** —, com aviso de consequência. Não é `Deny` (o expert PODE reduzir, conscientemente); é
`Ask` (o piso confirmável).

### Setup + Asserção exata
A F3-0-1/0-2 expõe uma classificação de mudança de parâmetro (nome a fixar pelo implementador, ex.:
`param_change_action_class(key, new, default) -> ActionClass`). As asserções:
```text
// REDUZIR abaixo do default → GatedHard (gate humano, mesmo em autônomo).
assert_eq!(param_change_action_class("delegation_budget", 4, 8),  ActionClass::GatedHard);
assert_eq!(param_change_action_class("breaker_threshold", 2, 5),  ActionClass::GatedHard);
assert_eq!(param_change_action_class("stall_breaker_samples",4,6), ActionClass::GatedHard);
// e a matriz já existente transforma isso em ASK (não Deny), em QUALQUER autonomia:
assert_eq!(decide(ActionClass::GatedHard, AutonomyLevel::Autonomous), Decision::Ask);

// O evento de gate é logado quando aplicado via params set sob esse caminho:
//   ActionGated{ class:"gated-hard", decision:"ask" }   (guard.rs:55-61)
```

### Vetores de borda (red-team — a não-vacuosidade vive aqui)
- **AUMENTAR/afrouxar acima do default NÃO é GatedHard:** `param_change_action_class("delegation_budget",
  16, 8) == Routine` (ou `GatedSoft`, conforme a doutrina), **nunca `GatedHard`**. *Subir o teto do
  laço não é desligar o breaker.* — sem este caso, um teste que retornasse `GatedHard` para tudo
  passaria vacuamente.
- **Parâmetro NÃO-balanceador fora da lista dos 3 não escala:** mudar `scrollback_retention_days` ou
  `token_budget_day` **não** é `GatedHard` por esta regra (eles têm seus próprios gates — custódia de
  custo, ADR 0005). Prova que a regra é **cirúrgica** (só os 3 laços balanceadores), não um gate
  preguiçoso que barra qualquer `set`.
- **No default exato não é redução:** `param_change_action_class("breaker_threshold", 5, 5)` não é
  `GatedHard` (não houve enfraquecimento).
- **`params set` via CLI RECUSA fora-da-faixa (≠ load):** distinção da spec 51 §Superfície/§Seg 4 —
  o `set` do expert **recusa** o valor inválido com a faixa na mensagem (feedback explícito), enquanto
  o **load** de `params.json` clampa (Teste A). São caminhos diferentes; não confundir clamp com recusa.

**Quebra se…** a classificação devolver `Routine`/`GatedSoft` para uma redução de breaker (o gate
some), **ou** devolver `GatedHard` para um aumento/para parâmetros fora dos 3 (gate preguiçoso que
trava o uso normal e treina o usuário a clicar "sim" no automático — pior que não ter gate).

---

## 5. Matriz de rastreabilidade (teste ↔ spec 51 ↔ critério)

| Teste | Prova (regra-mãe) | spec 51 | Critério de aceite | Depende de | Molde herdado |
|---|---|---|---|---|---|
| **A** clamp+WARN sem panic | parâmetro não derruba o processo nem entra cru | §Seg 4 | Critério 3 | F3-0-1 | `history.rs:116` (clamp), best-effort |
| **B** carimbo server-side | campo de agente ≠ autoridade/magnitude | §Seg 1 | Critério 6 | F3-0-2/0-4 | `spawn_cascade…:5093`, `spawn_gate_ignores_forged…:5129` |
| **C** weakening = GatedHard | afrouxar laço balanceador sobe ao gate humano | §Seg 3 | (gate de saída F3-0 §e) | F3-0-1 | `guard.rs:classify/decide:148,204` |

**Gate de saída F3-0 (e):** "suíte de segurança do router **verde** com qualquer `params.json`" — os
3 testes acima, mais o baseline da §0, são a evidência desse gate. **0 ALTA de segurança para seguir.**

---

## 6. Ordem de implementação (para o dev pós-F3-0-1)

1. **A** primeiro (só precisa de `SystemParams::resolve`/clamp/`ParamWarning` de F3-0-1) — é o mais
   isolado e o que mais cedo destrava confiança no load.
2. **C** em seguida (classificação + matriz do `guard.rs` já existe; só falta a função de classe).
3. **B** por último (precisa do carimbo de `SystemParamsChanged`/`EffortAssigned` de F3-0-2/0-4) — é o
   mais valioso e o que mais espelha os testes baseline; escreva-o lado a lado com
   `spawn_gate_ignores_forged_hops_and_root` para reusar o padrão de prova não-vacuosa.

Todos devem **falhar contra o código atual** (TDD — não há `params.rs` ainda) e passar após o merge
das dependências, **sem** nunca tornar vermelho qualquer teste da §0.
