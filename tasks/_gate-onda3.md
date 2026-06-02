# Design executável — GATE DA ONDA 3 (teste headless final)

> **Contrato para o terminal implementador do teste.** Não é código; é o mapa preciso de _o que provar_, _qual mecanismo prova_, _onde ele vive (`arquivo:linha`)_ e _qual assert binário fecha a onda_. Fontes: `tasks/_design-onda3.md` (§ critério W3), `tasks/_design-w6-w7.md` (§6 consolidado + §3 ADR await/reply + §4.3 path do espelho), código real commitado até `97642b5`.
>
> **Invariantes que o gate materializa:** #1 (zero LLM — todo guardrail é contador/grafo/timer/pattern-match) · #4 (event log = fonte da verdade — **o gate assere do log, nunca de retorno em memória**) · #6 (estado salvo/visível — recusa é evento, pausa é projeção, nada é kill silencioso).
>
> **Trilho B → valida-se headless** sobre o event store: SQLite WAL (`events`) **+ espelho append-only `<store>/events/log.jsonl`** (nome reconciliado no código — ver §4.1). O gate lê o JSONL e assere `kind` + campos.

---

## 0) PREMISSA DO GATE — escritor único, sem instrução de orquestração

O gate roda num Espaço com **2 agentes + preset**. A única entrada humana permitida é o **turno-0** (o disparo que carimba o primeiro `root_cause_id`). **Nenhum evento do log pode ter origem em input humano além do turno-0** — é a prova de que a orquestração é automática (auto-orquestração W3-4, bloco 4 do `CLAUDE.md`). O pipeline `Router::route_message` é **escritor único single-thread** (`crates/lina-core/src/router.rs:255`), então a ordem dos eventos no log é determinística e asserível por replay.

---

## 1) SEQUÊNCIA DE EVENTOS ESPERADA NO LOG

### 1.1 Caminho feliz (2 agentes resolvem tudo sozinhos)
```
Handshake          (@A entra; "agent.joined" — ver nota)   events.rs:124
Handshake          (@B entra)
PlanClaimed        (@A reivindica item-1)                   events.rs:194
MessageRouted      (@A --ask/handoff--> @B; passou guardrails 0-4)   events.rs:130
[AwaitOpened]      (se a msg foi --await)                   events.rs:162
[AwaitClosed{Replied}]  (reply autenticado de @B)           events.rs:170
```

### 1.2 Caminhos de recusa (cada cenário injeta o gatilho; o gate assere o evento)
```
RouteBlocked{reason: hop_limit|no_target|blocked_by_autonomy|fanout_gated|budget_exceeded|deadlock|loop_detected}   events.rs:154
ActionGated{cmd, class, decision}   (gate de execução, fora do router)   events.rs:178
CostCeilingHit{...}                 (teto de custo — AINDA NÃO EXISTE; ver §5)
```

> **NOTA — `agent.joined` colapsa em `Handshake`.** O design fala "agent.joined → handshake" como dois passos; **no código é um só evento** `DomainEvent::Handshake` (events.rs:122-124), cujo doc-comment diz literalmente "registramos a entrada no log (`agent.joined`)". Presença/roster é **estado vivo do Supervisor**, NÃO event-sourced. O gate deve esperar **um** `Handshake` por agente, não dois eventos distintos.

> **REGRA DURA da sequência:** todo evento acima nasce de A2A roteada ou do gate de execução. Se o gate observar **qualquer** evento cujo `root_cause_id` não derive do turno-0 (ou um 2º `root_cause_id` sem novo turno humano), **FALHA** — significa orquestração manual vazando.

---

## 2) CENÁRIOS QUE O GATE PROVA

> Legenda: **dispara** = o que o teste injeta · **mecanismo** = a peça determinística · **evento** = assert no log · **vive em** = `arquivo:linha`.

### a. Auto-orquestração sem instrução (só turno-0)
- **dispara:** subir 2 agentes no preset; turno-0 humano único; ambos resolvem papel (W3-1) + bootstrap 8 blocos (W3-2) + `lina handshake` (W3-4) + `lina plan claim` (W3-5).
- **mecanismo:** roster/presença do Supervisor + `plan.md` event-sourced; A2A pela mailbox `.lina/`, roteada por `route_message`.
- **evento:** `Handshake` (×2) → `PlanClaimed` → `MessageRouted`, **todos** com `root_cause_id` derivado do turno-0.
- **vive em:** `events.rs:124` (Handshake), `events.rs:194` (PlanClaimed), `router.rs:255` (route), `router.rs:553-568` (`derive_root_hops` — root/hops vêm do binding interno, NUNCA dos campos forjáveis da mailbox).

### b. Gate duro morde (ação irreversível)
- **dispara:** `lina guard --check-action --cmd "git push --force origin main" --autonomy autonomo`; depois `--cmd "git push origin feature/x"` em `assistido` vs `autonomo`; depois `--cmd "cargo test"`.
- **mecanismo:** `classify` (routine/gated-soft/gated-hard, igualdade de token) → `decide` (matriz nível×classe). `gated-hard` é testado **antes** de `gated-soft` (force-push casa os dois; o piso destrutivo vence).
- **evento esperado:**
  - `git push --force origin main` → `decision=ask` em **qualquer** nível + `ActionGated{class:"gated-hard", decision:"ask"}`.
  - `git push origin feature/x` → `assistido`: `ask` + `ActionGated{class:"gated-soft"}`; `autonomo`: **`allow`** (afrouxa soft) **+ SEM evento** (ação permitida não polui o log).
  - `cargo test` → `allow`, sem evento.
- **vive em:** `guard.rs:134` (`classify`), `guard.rs:149` (`decide`, matriz), `guard.rs:155-156` (autônomo afrouxa soft mas hard segue `ask`), `events.rs:178` (ActionGated, apendado SÓ quando `decision != allow` — `events.rs:175-177`).
- **⚠ precisão:** a matriz **nunca retorna `deny`** — `gated-hard` é `ask` (piso confirmável), não `deny` (`guard.rs:62-64`). O `Decision::Deny` existe só para completude do contrato/hook. Logo `rm -rf /` via hook → **`ask`**, não `deny`. O AC histórico que dizia "deny|ask" deve assertar **"não-`allow`"**.

### c. Anti-loop A→B→A sob mesmo `root_cause_id`
- **dispara:** semear `MessageRouted A→B`; rotear `B→A` com **mesmo `root_cause_id`, hops<4, ids ULID distintos**; depois repetir trocando o `root_cause_id`.
- **mecanismo:** grafo de arestas `from→to_node` reconstruído dos `MessageRouted` filtrados por `root_cause_id`; se `sender→target` fecha ciclo → `LoopDetected`. **Ortogonal a `hops`/`trace`** (que só cortam profundidade/cadeia encaminhada).
- **evento:** 2ª msg mesmo root → `RouteBlocked{reason: loop_detected}`; trocar root → **passa** (`MessageRouted`), provando que o ciclo é por-turno.
- **vive em:** `router.rs:361-375` (checagem do grafo + `log_block(LoopDetected)`), `events.rs:130` (`MessageRouted` carrega `root_cause_id`/`hops`/`to_node` — `to_node` é a aresta; `None` em broadcast → broadcast não entra no anti-loop por ciclo, events.rs:143-145), `events.rs:154` (RouteBlocked).

### d. Teto de custo (token_budget_day) — ⚠ AINDA NÃO IMPLEMENTADO (ver §5)
- **dispara (quando existir):** semear eventos de uso somando `> token_budget_day`; tentar nova delegação; depois `lina resume --confirm`.
- **mecanismo planejado:** `CostLedger` por `(workspace, dia-UTC)` event-sourced; ao estourar, workspace → estado `Paused`; nova delegação retorna `BudgetExceeded` e **pede confirmação humana** (gate, não kill — invariante #6).
- **evento esperado:** `CostCeilingHit{workspace, day, tokens}` + projeção `Paused`; `lina resume --confirm` reabre.
- **vive em:** **NÃO EXISTE** — `grep CostLedger|CostCeilingHit|token_budget|Paused crates/lina-core/src/*.rs` = vazio. É W3-7c (em curso). Precisa: campo `RouterConfig.token_budget_day`, variante `DomainEvent::CostCeilingHit`, projeção `Paused`, verbo `lina resume`. **Bloqueia este cenário do gate até a peça existir.**

### e. Await/reply lifecycle (fechamento autenticado — Round 4)
- **dispara:** `@A --await--> @B`; depois reply **autenticado** de `@B` (`reply_to`=id, `from`=@B, `to`=@A); depois um reply de **não-target** (`@C`) com o mesmo `reply_to`.
- **mecanismo:** `begin_await` é a checagem+abertura (rollback se a rota falhar); o reply só fecha se `sender == ticket.target` **E** `to` resolve ao `waiter` (defesa em profundidade contra confused-deputy).
- **evento esperado:**
  - `--await` aprovado → `AwaitOpened{id, waiter, target, root_cause_id}`.
  - reply de @B → `AwaitClosed{reason: replied}` + `end_await` libera o wait-for-graph (verificável: novo `A→B --await` volta a ser permitido).
  - reply de @C (não-target) → **nada fecha**; o ticket persiste; o wait-for-graph segue travado.
- **vive em:** `router.rs:380-403` (abertura/`begin_await`/rollback + `AwaitOpened`), `router.rs:580-615` (`close_await_on_reply`: `sender != ticket.target` → `return false`, `router.rs:594-596`; checa `to_is_waiter`, `router.rs:598-602`; só então consome+`AwaitClosed{Replied}`), `events.rs:162` (AwaitOpened), `events.rs:170` (AwaitClosed), `events.rs:64-67` (`AwaitClosed{Deadlock}` **nunca** emitido — a pré-checagem recusa antes de abrir ticket). Timeout: `router.rs:617+` (`sweep_await_timeouts` → `AwaitClosed{Timeout}`).

### f. Blocks logados — o livro-razão das recusas
- **dispara:** cada caminho negativo de (b)(c)(e) + dedupe/hops/autonomia/fanout/budget/deadlock.
- **mecanismo:** `log_block(store, msg, BlockReason::X)` apenda `RouteBlocked` antes de retornar o `RouteOutcome` (ADR 0003).
- **evento esperado por recusa:**
  | recusa | BlockReason | vive em |
  |---|---|---|
  | profundidade | `hop_limit` | router.rs:307 |
  | autonomia (manual) | `blocked_by_autonomy` | router.rs:331 |
  | fan-out | `fanout_gated` | router.rs:337 |
  | orçamento | `budget_exceeded` | router.rs:355 |
  | anti-loop | `loop_detected` | router.rs:370 |
  | deadlock | `deadlock` | router.rs:391, 402 |
  | ação irreversível | `ActionGated` (não RouteBlocked) | events.rs:178 |
- **⚠ EXCEÇÃO HONESTA — `Duplicate` NÃO é logado.** Por design anti-amplificação (A4), dedupe **não** apenda `RouteBlocked` (`events.rs:152-153`: "`duplicate` NÃO é apendado"). O gate deve assertar **ausência** de evento para o caso dedupe — não tratar como falha. Idem `UnknownSender`/`NoTarget`: `BlockReason` os define (events.rs:55-56) mas confirme no replay se `route_message` os apenda (`router.rs:287`/`319`/`326` retornam sem garantia de `log_block` — **o implementador do gate deve ler esses 3 pontos e decidir o assert por inspeção, não por suposição**).
- **vive em:** `BlockReason` enum `events.rs:52-62`, `log_block` chamado em router.rs (linhas acima).

### g. Contrato congelado (envelope + terminador)
- **dispara:** roundtrip serialize→deserialize de uma `MailMessage` com **todos** os campos preenchidos (`schema/id/root_cause_id/from/to/intent/hops/reply_to/await_reply/payload`); render do bloco.
- **mecanismo:** `MailMessage` único tipo `lina/msg@1`; `render_message_block` achata cada campo em uma linha e sanitiza payload (defesa contra forja de `[LINA::MSG]`).
- **evento/assert:** roundtrip preserva todos os campos com `schema == "lina/msg@1"`; o bloco emite `[LINA::MSG]…[/LINA::MSG]` e **não contém `ESTUDIO`**.
- **vive em:** `mailbox.rs:17` (`MAIL_SCHEMA_V1="lina/msg@1"`), `mailbox.rs:58-138` (`MailMessage` + `new`/`awaiting`/`reply_to`), `mailbox.rs:529-561` (`render_message_block`, terminador `[/LINA::MSG]`, namespace `LINA` não `ESTUDIO`), `mailbox.rs:21` (`MAX_MSG_BYTES=256KiB`). Teste existente `message_block_has_sentinel_and_lina_terminator` cobre o terminador; **estender ao roundtrip completo**.

---

## 3) CRITÉRIO DE PASS (binário, observável no `events/log.jsonl`)

O gate **PASSA** se e somente se, num único Espaço de 2 agentes + preset, com turno-0 humano único:

1. **(a)** Log contém `Handshake`×2 → `PlanClaimed` → `MessageRouted`, todos com `root_cause_id` derivado do turno-0; **zero** evento com root estranho ao turno-0.
2. **(b)** `git push --force …` → `ActionGated{gated-hard, ask}` em `manual`/`assistido`/`autonomo`; `git push feature` → `ActionGated{gated-soft, ask}` em `assistido` e **`allow` sem evento** em `autonomo`; `cargo test` → `allow` sem evento.
3. **(c)** B→A mesmo `root_cause_id` (hops<4, ids distintos) → `RouteBlocked{loop_detected}`; trocar root → `MessageRouted`.
4. **(e)** `--await` → `AwaitOpened`; reply autenticado → `AwaitClosed{replied}` + aresta liberada; reply de não-target → ticket persiste (nada fecha).
5. **(f)** Cada recusa de (b)(c)(e)+hops/autonomia/fanout/budget/deadlock tem seu `DomainEvent`; **dedupe NÃO tem** (assert de ausência).
6. **(g)** Roundtrip do envelope preserva todos os campos; render usa `LINA`, nunca `ESTUDIO`.
7. **(d) — condicional:** se W3-7c estiver pronto, uso > `token_budget_day` → `CostCeilingHit` + `Paused`, `lina resume --confirm` reabre. **Enquanto não existir, este item fica fora do PASS e o gate roda com (d) explicitamente marcado SKIPPED** (não falha por ausência — falha só se existir e não funcionar).

> O assert é sempre **sobre o JSONL** (`kind` + campos), reaplicável por replay — nunca sobre o `RouteOutcome` em memória (que só um harness in-process lê). Veredito só após **ler o log renderizado em arquivo** (não inferir de stdout glitchado).

---

## 4) NOTA DE CONSOLIDAÇÃO

### 4.1 Path do espelho — RECONCILIADO no código (flag de §4.3 fechada)
O design `_design-w6-w7` §4.3 pedia reconciliar `bus.jsonl` (código antigo) vs `.lina/events/log.jsonl` (design). **Já está resolvido:** `const JSONL_FILE: &str = "log.jsonl"` (`events.rs:431`), aberto no diretório `events/` do store, com doc-comment registrando a decisão (`events.rs:426-431`): _"o espelho JSONL chama-se `log.jsonl` … nada de 3º nome. Stores de dev com `bus.jsonl` antigo ficam órfãos"_. **Ação do gate:** ler `<store>/events/log.jsonl`. Nenhuma reconciliação pendente — apenas confirme que nenhum teste legado ainda aponta para `bus.jsonl`.

### 4.2 ADR de teto-de-custo — A ESCREVER (`docs/adr/0005`)
Os ADRs `0001`-`0004` existem (`plan-event-sourcing`, `await-reply-lifecycle`, `blocks-logados`, `custodia-de-segredo`). O teto de custo (W3-7c) ainda **não tem ADR**. Antes de implementar `CostLedger`/`Paused`/`CostCeilingHit`, registrar **`docs/adr/0005-teto-de-custo.md`** decidindo: granularidade `(workspace, dia-UTC)`; fonte dos tokens (evento de uso por fim-de-resposta da CLI, W0); **pausa-com-gate vs kill** (decisão: pausa — invariante #6); reabertura via `lina resume --confirm`. Sem o ADR, o construtor pode inventar semântica divergente do "estado salvo e visível".

### 4.3 Reconciliação da sequência do design vs eventos reais
`agent.joined` e `handshake` do design são **um** `DomainEvent::Handshake`. Atualizar qualquer doc/teste que espere dois eventos distintos.

---

## 5) O QUE FALTA vs O QUE JÁ DÁ PARA PROVAR HOJE

### ✅ Provável HOJE (peças commitadas até `97642b5`)
- **(a)** auto-orquestração: handshake + plan.claim + MessageRouted — FEITO (W3-1/2/4/5).
- **(b)** gate duro determinístico: `classify`/`decide`/`ActionGated` + hook `PreToolUse` — FEITO (W3-6a `0bde89d`, W3-6b `97642b5`).
- **(c)** anti-loop por grafo de eventos — FEITO (W3-7a `516b7af`).
- **(e)** await/reply com fechamento autenticado (Round 4) — FEITO (`83da6fa`).
- **(f)** blocks logados (exceto dedupe, por design) — FEITO (ADR 0003).
- **(g)** contrato do envelope + terminador `LINA` — FEITO.

### ❌ Falta para o gate rodar 100%
1. **(d) Teto de custo (W3-7c)** — `CostLedger` + `token_budget_day` + `Paused` + `CostCeilingHit` + `lina resume`. **Em curso.** Sem isso, cenário (d) = SKIPPED.
2. **W3-6c — broker `lina do` + custódia de segredo + A3 auth (outbox por-nó).** Hoje o reply é autenticado por `sender==target` (router.rs:594) e root/hops vêm do binding interno (router.rs:553), mas o **outbox em si é canal não-autenticado** (memória de projeto confirma: campos do outbox não decidem cadeia). A custódia de segredo (AC-6.5: agente sem token → ação vira `lina do deploy` brokerada) **não existe** — `lina do` não está implementado. Logo a parte "deploy/pagamento bloqueado por ausência de credencial" do cenário (b) **não é provável hoje**; só a classificação `gated-hard → ask` é.
3. **Verbo `lina resume`** — necessário para fechar o loop de (d).

### Recomendação de fechamento
Rodar o gate **agora** com (a)(b)(c)(e)(f)(g) como PASS obrigatório e (d) + custódia como **SKIPPED rastreado** (logar o que foi pulado — sem cap silencioso). Quando W3-7c + W3-6c entrarem, promover (d) e a custódia a obrigatórios e re-rodar. O gate é incremental por construção: cada peça que chega vira um assert que sai do SKIP.
