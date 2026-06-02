# Design executável — W3-6 (Autonomia + gate DURO) & W3-7 (Guardrails P0)

> Extração densa para o Master Maestro distribuir a construção do **sorvedouro da Onda 3** sem reler docs gigantes. Fontes: `tasks/_design-onda3.md` (§4 ORDEM + tabela "Guardrails P0" + critério W3); `crates/lina-core/src/{router.rs, mailbox.rs, lib.rs, events.rs}` (estado real). Sem código.
> **Princípio duro (invariante #1):** o app **NUNCA chama LLM**. Todo guardrail é **contador/grafo/timer/pattern-match**. A inteligência vive no agente; o Lina é roteador + supervisor com estado.
> **Trilho B → consome o core W0.** Tudo aqui se valida **headless** via o event log (`DomainEvent` no SQLite + espelho JSONL). Ver §6.0 sobre o **path do espelho** (`bus.jsonl` no código vs `.lina/events/log.jsonl` no design — reconciliar).

---

## 0) ESTADO REAL — o que JÁ existe vs o que falta (não reconstruir o que está pronto)

O pipeline `Router::route_message` (`router.rs`) é **escritor único**, single-thread, sem locks — já roda os guardrails 0-4 + dedupe/hops/fanout/budget/deadlock. Mapa preciso EXISTS/MISSING:

| Guardrail (tabela design §4) | Status | Onde / por quê |
|---|---|---|
| **Dedupe** (`id`, janela 60s) | ✅ **EXISTS** | `router.seen: HashMap<id,ts>` + `prune_seen`; `RouteOutcome::Duplicate`. Teste `duplicate_id_within_window_is_ignored`. |
| **Profundidade** (`hops`, max=4) | ✅ **EXISTS** | `msg.hops > config.max_depth` → `HopLimit`. Teste `hops_over_max_depth_is_anti_loop_dropped`. + `A2aEnvelope.ttl`/`trace` (anti-loop **de cadeia única**). |
| **Anti-deadlock** (wait-for graph, só `--await`) | ✅ **EXISTS** | `WaitForGraph{awaiting: HashMap<NodeId,NodeId>}` + `begin_await`/`end_await`; checagem pura com rollback no router (Guardrail 2) → `Deadlock`. Teste `await_into_a_waiting_target_is_a_deadlock`. |
| **Orçamento de delegação** (8/turno por `root_cause_id`) | ✅ **EXISTS** | `router.budget: HashMap<(root,sender),u32>`; incrementa **só após persist** (delegação efetivada, não tentada). Teste `delegation_budget_is_enforced_per_root_cause`. |
| **Gate de fan-out** (`broadcast` > 3) | ✅ **EXISTS** | `FanoutGated{count}` quando `Recipient::Broadcast && preview.len() > fanout_gate`. Teste `broadcast_over_fanout_gate_needs_confirmation`. |
| **Autonomia** (manual bloqueia delegação) | ⚠️ **PARCIAL** | Só `Manual` muda roteamento (`blocks_delegation` → `BlockedByAutonomy`). `Assisted` e `Autonomous` são **idênticos no router** — a diferença vive no **ponto de execução** (W3-6), que **não existe**. |
| **Anti-loop ciclo A→B→A** (grafo de eventos handoff, por item+`root_cause_id`, independe do texto) | ❌ **MISSING (como especificado)** | O `trace`/`ttl` só pega loop **dentro de um envelope encaminhado**. Mensagens **independentes** que fecham ciclo sob o mesmo `root_cause_id` nascem com `trace=[from]` → **escapam**. O grafo `handoff from→to` lido do log é mecanismo distinto e ausente. |
| **Gate ação irreversível** (`PreToolUse`/wrapper no comando real) | ❌ **MISSING** | Não existe. É **W3-6**, fora do router (ponto de execução do agente, não roteamento A2A). |
| **Teto de custo** (`token_budget_day`) | ❌ **MISSING** | Nenhum contador de tokens. `RouterConfig` não tem o campo. |

> **Leitura para o construtor:** W3-7 **NÃO** reescreve o router — ele **adiciona 3 peças** (anti-loop de grafo de eventos; teto de custo; e — transversal — **logar os blocks**, §6.1) e W3-6 **adiciona uma camada nova** (o gate de execução). O resto é endurecer/conectar o que já passa nos testes.

---

## 1) W3-6 · AUTONOMIA (3 níveis) + GATE DURO de ação irreversível

### 1.1 Os 3 níveis — semântica PRECISA (quem delega, quem confirma, o que roda sozinho)

A autonomia governa **delegação + trabalho rotineiro**. O **gate duro** (ações irreversíveis/credenciais) é um **piso** que a autonomia pode **afrouxar de "confirma" para "auto"**, **exceto o conjunto hard-gated não-negociável**, que confirma sempre. Determinístico: um **ruleset TOML** classifica cada padrão de comando em `{routine | gated-soft | gated-hard}`; `(nível) × (classe) → {auto | confirm | block}`. Zero LLM.

**Classes de ação** (do ponto de vista do agente):
- `delegation` — A2A `ask`/`handoff`/`broadcast`/`review` (sai pela mailbox → router).
- `local-reversible` — ler/editar arquivo no workspace, rodar teste, build.
- `gated-soft` — irreversível mas recuperável/escopo-feature: `git push` (branch feature), `rm` em path do workspace, `git commit --amend`.
- `gated-hard` — irreversível destrutivo OU com custódia de segredo: `git push --force`/push em `main`, `rm -rf` **fora** do workspace, `deploy`/`terraform apply` em prod, **pagamento**, envio externo (e-mail/DM/webhook a terceiro), `DROP`/migração destrutiva.

**Matriz canônica** (preenche o **bloco 5** do `CLAUDE.md`):

| Classe \ Nível | `manual` | `assistido` (default) | `autônomo` |
|---|---|---|---|
| `delegation` | **BLOCK** (router já faz) | ALLOW | ALLOW |
| `local-reversible` | ALLOW | ALLOW | ALLOW |
| `gated-soft` | CONFIRM | CONFIRM | **AUTO** (afrouxado) |
| `gated-hard` | CONFIRM | CONFIRM | **CONFIRM** (piso — nunca afrouxa) |
| fan-out `broadcast` > N | CONFIRM | CONFIRM | CONFIRM |

> **O que `autônomo` realmente libera:** move `gated-soft` de *confirma* → *auto* e dispensa o "proponha primeiro" da delegação. **`gated-hard` confirma em TODOS os níveis** — autonomia não é licença para deletar prod ou pagar sem humano. `manual` ≠ "nada roda": trabalho local rotineiro segue; o que trava é **delegação automática** (no router) e tudo que muta o mundo (confirma).

### 1.2 Como o nível é lido e propagado

- **Leitura:** `workspace.json → autonomy: "manual"|"assistido"|"autonomo"`. O app traduz para `router::AutonomyLevel` (espelho leve; o **core não depende do bootstrap**, conforme o doc do `AutonomyLevel`).
- **Propagação ao agente:** a cada mudança de roster/autonomia, o app **reescreve `<workspace>/CLAUDE.md`** (canal à prova de falhas, W3-2). O **bloco 5** recebe: o nível vigente **+ a linha da matriz** ("neste nível: delego sozinho? sim/não; `git push` pede confirmação? sim/não; pagamento/deploy: sempre confirma"). Em CLI com hook, `lina whoami --bootstrap` reimprime o nível vivo.
- **Dois enforcement, dois lugares:** (a) `delegation` em `manual` é barrado **no router** (já existe, `BlockedByAutonomy`); (b) `gated-*` é barrado **no ponto de execução** (§1.3, novo). O bloco 5 é **doutrina** (o agente sabe), mas o **enforcement real** é o gate de execução — não se confia que o LLM "respeite" o texto.

### 1.3 GATE DURO no PONTO DE EXECUÇÃO — não "grep de intenção"

O gate **NÃO** lê o texto do agente procurando "vou dar deploy". Ele intercepta a **ação real** (o comando prestes a rodar) **antes** de executar e pede **sim/não** ao humano. Mecanismo difere por CLI:

| CLI | Mecanismo de intercepção | Natureza do gate |
|---|---|---|
| **Claude Code** (tier 1) | hook **`PreToolUse`** → roda `lina guard --check-action --cmd "<comando>" --autonomy <nível>` → devolve JSON `hookSpecificOutput.permissionDecision = allow\|ask\|deny` | **HARD real** — o harness garante que **toda** chamada de tool (Bash/Write/…) passa pelo hook antes de executar. |
| **Codex / Gemini / shell** (tier 2, **sem hook**) | **PATH-shim**: o app prepõe `.lina/bin/` ao `PATH` do agente com wrappers (`git`, `rm`, `kubectl`, `terraform`, `gh`…) que chamam `lina guard --check-action` e só fazem `exec` do binário real se `allow` | **HARD no limite do binário** — mas **bypassável por caminho absoluto** (`/usr/bin/git`). |

**Trade-off honesto:** só o **hook** é gate verdadeiramente duro (o harness obriga a passagem). O **PATH-shim** é duro-no-comum mas furável (`/usr/bin/git push --force`), e cobre só binários conhecidos. Sandbox por SO (seatbelt/bubblewrap/token restrito) fecha o furo, mas é **pesado e frágil cross-platform** → fora do MVP.

> **★ Recomendação (ADR-C) — custódia de segredo é o gate duro REAL para `gated-hard` externo.** Deploy/pagamento/envio externo dependem de um **segredo** (deploy key, API key de pagamento, token de webhook). Se o **Lina** detém esse segredo no keyring (**Secret Vault W0-7, token-por-nó**) e **o agente não o tem**, o agente **não consegue** executar a ação sozinha — por construção, ele **precisa pedir** ao Lina, que aí roda o gate. **Pattern-match de comando é a camada soft; custódia de credencial é a camada inquebrável.** Recomendação composta:
> 1. **Claude Code:** `PreToolUse` hook (gate duro nativo) para `gated-soft` + `gated-hard` locais.
> 2. **Tier 2:** PATH-shim para `gated-soft`/`gated-hard` locais (cobre o caso comum, ruidoso quando furado).
> 3. **`gated-hard` externo (deploy/pagamento), TODOS os CLIs:** **custódia de segredo** — o agente nunca recebe o token; a ação vira `lina do deploy`/`lina do pay` que o app **broker**a com o gate humano. Inquebrável independente de hook.

### 1.4 Critérios de aceite W3-6 (headless, observáveis no log)

- **AC-6.1 (matriz/propagação):** setar `workspace.json autonomy=autonomo` e adicionar `@Dev` → `<workspace>/CLAUDE.md` **bloco 5** contém o nível `autonomo` **e a linha da matriz** (`git push` = auto; pagamento/deploy = sempre confirma). Trocar para `manual` **reescreve** o bloco (diff observável) sem reiniciar o app. `lina whoami --bootstrap` imprime o nível vivo.
- **AC-6.2 (gate duro — verbo determinístico):** `lina guard --check-action --cmd "git push --force origin main" --autonomy autonomo` → saída `permissionDecision=ask` (nunca `allow`) **e** evento `ActionGated{cmd, class:"gated-hard", decision:"ask"}` apendado ao log. `--cmd "cargo test"` → `allow`, sem evento. `--cmd "git push origin feature/x" --autonomy assistido` → `ask`; `--autonomy autonomo` → `allow` (afrouxa `gated-soft`).
- **AC-6.3 (hook Claude Code — golden):** dado o JSON de entrada do `PreToolUse` para um Bash `rm -rf /` , o hook produz o JSON `{hookSpecificOutput:{permissionDecision:"deny"|"ask", permissionDecisionReason:"…"}}` **bem-formado e estável** (golden-file), e **nunca** stdout cru (mesma regra de robustez do `SessionStart`, W3-2).
- **AC-6.4 (PATH-shim — não-execução):** com `.lina/bin/git` no `PATH` e um canal de confirmação stubado para **"não"**, invocar `git push --force` → o binário real **NÃO** é exec'd (sentinela: o wrapper apenda `ActionGated{decision:"deny"}` e sai != 0 sem chamar o `git` real). Furo documentado: `/usr/bin/git push --force` **passa** (limite conhecido do shim).
- **AC-6.5 (custódia de segredo):** com o token de deploy só no keyring do Lina (não no env do agente), `lina do deploy --env prod` dispara o gate humano e **bloqueia** sem confirmação; o agente, sem o token, não tem caminho alternativo de execução (verificável: o env do PTY do agente **não** contém a chave; evento `ActionGated{class:"gated-hard-external"}`).

---

## 2) W3-7 · GUARDRAILS P0 — mecanismo, casa, critério headless (por linha)

Os ✅ já passam em teste (§0). Abaixo, os **3 itens faltantes** + endurecimentos, cada um com mecanismo determinístico e AC headless.

### 2.1 Anti-loop ciclo A→B→A — grafo de eventos handoff (FALTANTE)

- **Mecanismo:** ao rotear um `handoff`/`ask`, o router consulta o **grafo de arestas `from→to`** reconstruído dos eventos `MessageRouted` do log **filtrados por `(root_cause_id, ref do plano)`**. Se adicionar `sender→target` **fecha um ciclo** nesse grafo (mesma lógica de detecção do `WaitForGraph.add`, mas sobre o histórico de handoffs, **não** sobre `--await`), recusa com `LoopDetected`. **Independe do texto** (é topologia de eventos).
- **Por que não basta o que existe:** `hops`/`max_depth` corta **profundidade**; `trace`/`ttl` corta **loop de cadeia encaminhada**. Nenhum pega A→B→A formado por **mensagens novas** sob o mesmo `root_cause_id` (cada `MailMessage::new` reinicia `trace=[from]`, `hops=0`). Este grafo é o que fecha esse furo.
- **Casa:** `lina-core` (novo módulo/append no `router.rs`); estado derivável do log (replay) → coerente com invariante #4 (log = fonte da verdade).
- **AC-7.1 (headless):** semear o log com `MessageRouted A→B` e depois rotear `B→A` **mesmo `root_cause_id`, hops<4, ids distintos** → `LoopDetected` + evento `RouteBlocked{reason:"loop", id}`; trocar o `root_cause_id` da 2ª → **passa** (ciclo é por-turno). Prova que o mecanismo é ortogonal a hops/ttl.

### 2.2 Teto de custo — `token_budget_day` (FALTANTE)

- **Mecanismo:** contador de tokens por `(workspace, dia-UTC)` alimentado pelos eventos de uso (token-usage por turno, emitidos quando o app observa o fim-de-resposta da CLI — W0). Ao atingir `token_budget_day`, o workspace entra em **estado `Paused`**: novas delegações/spawns retornam `BudgetExceeded{tokens}` e **pedem confirmação humana** para retomar (gate, não kill — invariante #6 "estado sempre salvo e visível"). **Timer/contador**, zero LLM.
- **Casa:** `lina-core` (`RouterConfig.token_budget_day` + um `CostLedger` por dia, event-sourced).
- **AC-7.2 (headless):** semear eventos de uso somando `> token_budget_day` → a próxima delegação retorna `BudgetExceeded` **e** apenda `CostCeilingHit{workspace, day, tokens}`; o workspace fica `Paused` (projeção observável); `lina resume --confirm` reabre. Antes do teto, delega normal.

### 2.3 Endurecimentos dos guardrails que JÁ existem (não reconstruir — conectar)

| Item | Endurecimento | AC headless |
|---|---|---|
| Autonomia `assistido`/`autonomo` | Hoje idênticos no router. **Conectar** ao gate de execução (§1) — o router não muda; o gate de execução lê o nível. | AC-6.2 cobre. |
| Dedupe/hops/budget/fanout/deadlock | **Logar os blocks** (§6.1): hoje o `RouteOutcome` morre em memória. | §6.1 AC. |
| Envelope final completo | Teste de **serialização** do envelope canônico com **todos** os campos (`id/root_cause_id/from/to/intent/hops/await` + sentinela/terminador `LINA`). | AC-7.3. |

- **AC-7.3 (contrato congelado):** serializar→desserializar uma `MailMessage` com todos os campos preenchidos preserva tudo (`schema=="lina/msg@1"`); `render_message_block` emite `[LINA::MSG]…[/LINA::MSG]` e **não contém `ESTUDIO`** (§5). (O teste `message_block_has_sentinel_and_lina_terminator` já cobre o terminador; estender para o roundtrip completo do envelope.)

### 2.4 Tabela-resumo W3-7 (defaults canônicos)

| Guardrail | Mecanismo | Default | Status |
|---|---|---|---|
| Anti-deadlock | wait-for graph (`--await`) | imediato | ✅ |
| Profundidade | `hops` + cadeia `--await` | `max_depth=4` | ✅ |
| Dedupe | guarda `id` entregues | `60s` | ✅ |
| **Anti-loop ciclo** | **grafo de eventos `handoff from→to` por `root_cause_id`** | imediato | ❌→§2.1 |
| Orçamento delegação | conta eventos por `root_cause_id`/agente | `8`/turno | ✅ |
| Gate de fan-out | `broadcast` > N alvos confirma | `3` | ✅ |
| Gate ação irreversível | `PreToolUse`/shim **no comando real** (+custódia segredo) | sempre (`gated-hard`) | ❌→§1 |
| **Teto de custo** | contador tokens por workspace/dia → pausa+gate | `token_budget_day` | ❌→§2.2 |

---

## 3) ADR PENDENTE — `--await/reply` lifecycle: alargar `BusEvent::Message` vs envelope completo fora da bus

**Contexto/estado real:** `BusEvent::Message { id, from, to }` é **enxuto** (só a notificação de roteamento). O **payload** viaja **fora da bus**, pela closure `deliver` → injeção no PTY. O await é manual: `begin_await`/`end_await` no `WaitForGraph`, **sem** correlação de `reply` modelada (quando B responde, nada hoje casa a resposta ao await de A nem chama `end_await`). O `bus.jsonl` é, por comentário do próprio `events.rs`, **"espelho redundante do log"** — i.e., **a bus não é a fonte da verdade; o `DomainEvent` log é**.

### Opção A — alargar `BusEvent::Message` para carregar o envelope inteiro
- **Prós:** um lugar só na bus com tudo; subscribers (UI/observabilidade) recebem o envelope completo; reply vira `BusEvent::Reply{reply_to,from}`.
- **Contras:** a bus é `broadcast::Sender<BusEvent>` (clona a **todos** os subscribers a cada msg). Payload vai até **256 KiB** (`MAX_MSG_BYTES`) → broadcastar isso a N subscribers é **caro e vaza payload** (possivelmente sensível) a todo subscriber/espelho. **Funde dois contratos que devem versionar separados:** o `BusEvent` (pub/sub interno) e o `A2aEnvelope`/`lina/msg@1` (wire A2A). Mudar um quebra o outro.

### Opção B — envelope completo fora da bus; bus fica enxuta ✅ **RECOMENDADA**
- **Mecanismo:** `BusEvent::Message{id,from,to}` permanece como **notificação/chave de correlação**. O lifecycle de await/reply é modelado como **dois novos `DomainEvent`** — `AwaitOpened{id, waiter, target, root_cause_id}` e `AwaitClosed{id, reason: replied|timeout|deadlock}` — apendados ao **log** (fonte da verdade). O supervisor ganha uma **tabela tipada `pending: HashMap<id, AwaitTicket>`**; o `reply` (envelope com `reply_to=id`) acha o `waiter`, chama `end_await` e apenda `AwaitClosed`. Payload continua **só no envelope/transcript** (PTY), nunca duplicado na bus nem no log.
- **Justificativa ligada aos invariantes:**
  1. **#4 (event log = fonte da verdade):** o lifecycle vira **eventos no log** (observável headless, reconstruível por replay), não estado efêmero de bus. A bus é transporte/projeção (o próprio código diz que o `bus.jsonl` é espelho).
  2. **Envelope versionado (âncora de continuidade):** mantém `BusEvent` e `lina/msg@1` **versionando em separado** — não funde o pub/sub interno com o wire A2A. Disciplina "não fragmentar **nem** fundir contratos".
  3. **Local-first/performance:** não broadcasta 256 KiB a cada subscriber; bus O(1). Payload sensível não fan-out.
- **Custo:** subscribers que querem contexto completo casam a notificação (`id`) com o envelope da origem (store/registro) — plumbing pequeno, já que a entrega **já** ocorre fora da bus.

> **★ Recomendação:** **Opção B.** Formalizar em `docs/adr/` como **ADR — "await/reply lifecycle é event-log-sourced; bus permanece enxuta; 2 novos `DomainEvent` (`AwaitOpened`/`AwaitClosed`)"**, com **rejeição explícita** de alargar `BusEvent`.
> **AC-ADR (headless):** `A --await--> B` apenda `AwaitOpened`; o `reply` de B (`reply_to`) apenda `AwaitClosed{reason:"replied"}` e `end_await` zera a aresta do wait-for-graph (verificável: novo `A→B --await` volta a ser permitido). Timeout sem reply → `AwaitClosed{reason:"timeout"}`.

---

## 4) DECISÕES TRANSVERSAIS (viram ADR/consolidação)

### 4.1 ADR-B — todo guardrail bloqueado **emite um `DomainEvent`**
Hoje os blocks (`Duplicate`/`HopLimit`/`BlockedByAutonomy`/`FanoutGated`/`BudgetExceeded`/`Deadlock`) só existem como `RouteOutcome` **em memória**. Para os critérios "validável headless via log.jsonl" valerem nos **caminhos negativos**, cada recusa apenda `RouteBlocked{id, reason, from, to}` (router) / `ActionGated{cmd, class, decision}` (gate) / `CostCeilingHit{…}`. **O log vira o livro-razão das recusas** (invariante #4), e os ACs assertam do log, não de um retorno que só um harness in-process lê.
- **AC-4.1:** cada teste de block de §0/§1/§2 assere o `DomainEvent` correspondente no log (não só o enum de retorno).

### 4.2 Consolidação de naming — terminador `[/LINA::MSG]`
O terminador canônico A2A é **`[/LINA::MSG]`** (namespace `LINA`). **Já está correto no código** (`mailbox.rs::render_message_block` emite `[/LINA::MSG]`; o teste `message_block_has_sentinel_and_lina_terminator` **assere ausência de `ESTUDIO`**). O resíduo `[/ESTUDIO::MSG]` está **só no vault doc 21 §3.4** (rename "Estúdio"→"Lina Space" não propagado). **Regra para o contrato versionado: usar `LINA`, NUNCA `ESTUDIO`** — não copiar o exemplo do doc 21. (Não é ADR; é nota de consolidação no contrato.)

### 4.3 Reconciliação de path do espelho (flag)
O design diz "headless via `.lina/events/log.jsonl`"; o código (`events.rs`) escreve o espelho em **`<store>/bus.jsonl`**. **Reconciliar**: ou renomear o espelho para `events/log.jsonl`, ou o design adota `bus.jsonl`. Todos os ACs deste doc referenciam "**o event log** (`DomainEvent` kinds via store/JSONL)" para ficarem corretos sob qualquer escolha — mas a decisão precisa ser registrada para o construtor não criar um 3º nome.

---

## 5) ORDEM DE CONSTRUÇÃO & DEPENDÊNCIAS (recorte W6/W7)

Da sequência canônica do Trilho B (design §4): `…W3-5 plan.md → **W3-6 + W3-7** → W3-4/W3-3`. Ambos são **supervisor (lina-core), escritor único** → **sequenciais entre si**, não paralelizáveis (mutam o mesmo `.lina/`).

1. **Pré-req:** envelope canônico congelado (`id/root_cause_id/from/to/intent/hops/await` — já no `MailMessage`/`A2aEnvelope`) + W3-5 `plan.md` (o `ref` do plano alimenta o grafo anti-loop §2.1) + W0-7 Secret Vault (custódia §1.3).
2. **W3-7 primeiro a parte de roteamento** (mesma cadeia do router): §2.1 anti-loop de grafo + §2.2 teto de custo + §4.1 logar blocks. Tudo testável no `router.rs` headless, sem CLI viva.
3. **W3-6 depois** (camada nova, ponto de execução): verbo `lina guard --check-action` + ruleset TOML + hook `PreToolUse` (Claude Code) + PATH-shim (tier 2) + broker `lina do` (custódia). Depende do nível de autonomia propagado (W3-2 bloco 5) e do Secret Vault.
4. **Fecho:** §3 ADR await/reply (2 `DomainEvent`) — pode entrar junto de W3-7 (mesma cadeia de eventos).

---

## 6) CRITÉRIO DE ACEITE CONSOLIDADO W6+W7 (gate, headless via event log)

Num Espaço com 2 agentes e preset, **sem instrução de orquestração do usuário** além do turno-0:
1. **Autonomia propaga (AC-6.1):** `autonomy` no `workspace.json` aparece no **bloco 5** do `CLAUDE.md` com a linha da matriz; mudar reescreve sem reiniciar.
2. **Gate duro morde (AC-6.2/6.4/6.5):** `git push --force` → `ask` em qualquer nível; `git push feature` → `auto` só em `autonomo`; deploy/pagamento → bloqueado por custódia de segredo; tudo apenda `ActionGated`.
3. **Anti-loop de grafo (AC-7.1):** A→B→A sob mesmo `root_cause_id` (hops<4, ids distintos) → `RouteBlocked{reason:"loop"}`; muda root → passa.
4. **Teto de custo (AC-7.2):** uso > `token_budget_day` → `CostCeilingHit` + workspace `Paused`; `lina resume --confirm` reabre.
5. **Blocks logados (AC-4.1):** toda recusa (dedupe/hops/autonomia/fanout/budget/deadlock/loop/custo/ação) tem seu `DomainEvent` no log — o livro-razão das recusas é completo.
6. **Await/reply (AC-ADR):** `--await` apenda `AwaitOpened`; reply apenda `AwaitClosed{replied}` e libera o wait-for-graph.
7. **Contrato congelado (AC-7.3):** roundtrip do envelope completo preserva todos os campos; bloco usa terminador `LINA`, nunca `ESTUDIO`.

> **Sequência esperada no log (caminho feliz + recusas):** `agent.joined → handshake → plan.claim → MessageRouted → (RouteBlocked|ActionGated|CostCeilingHit conforme o cenário)` — nenhum evento originado em input humano além do turno-0; toda recusa é um evento, não um silêncio.
