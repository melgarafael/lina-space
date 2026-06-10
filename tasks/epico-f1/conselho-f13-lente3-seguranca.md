# Conselho de Gate F1-3 — LENTE 3 (SEGURANÇA): fecho dos commits pós-red-team

> **Re-verificação independente em linguagem de invariantes.** A Lente 3 já fez o red-team do
> `lina spawn` (`redteam-spawn-f1-3-6.md`, 0 ALTA / 4 MEDIA / 1 BAIXA). Este documento é o **FECHO**:
> os commits POSTERIORES (`76a3ccd..09876a4`) **não regridem** nenhum invariante de segurança e os
> MEDIA (M1-M4) **mantêm os bounds aceitos**.
> **Método:** cada invariante re-derivado no código (arquivo:linha) + **evidência observada** (gates
> rodados com exit code DIRETO, sem pipe-mask). READ-ONLY além deste relatório.

## VEREDITO: **OK-COM-RESSALVAS** · **0 ALTA novo** (critério do gate ATENDIDO)

Os 5 commits são seguros. **Nenhum** abre caminho de entrega sem prontidão, de escrita não-gateada,
de traversal, de contorno de gate, nem regride a suíte de segurança. **1 ressalva NOVA** de
**durabilidade/disponibilidade** (não-segurança) no ACHADO-2, fora do escopo-ALTA desta lente, sinalizada
para a lente de disponibilidade/Conselho.

| Commit | Superfície | Invariante | Veredito |
|---|---|---|---|
| `09876a4` | router/a2a (not-ready→retenção) | (1) entrega sem prontidão / DLQ-breaker / backstop 600s | ✅ 0 ALTA · **ressalva R1** (durabilidade no restart) |
| `bc58f0b` | instalador de skills | (2) só pastas `lina-*`, sem traversal, user-skill intacta | ✅ 0 ALTA |
| `1029151` | skill lina-spawn-terminal | (3) não ensina contornar gate | ✅ 0 ALTA |
| `988d1c7` | lina retro | (4) porta única, Refused antes de qualquer write | ✅ 0 ALTA |
| `dd32d46` | compile-fix app | só `requested_by: None`; spawn→admit_node ainda NÃO fiado | ✅ 0 ALTA (M4 inalterado) |
| (5) | gate_w34 | suíte de segurança intacta | ✅ EXIT=0 |

## Evidência observada (exit codes diretos)
- `cargo test -p lina-core --test gate_w34` → **EXIT=0** (1 passed) — segurança do router intacta.
- `cargo test -p lina-core --test gate_f1_0_7` → **EXIT=0** (7 passed) — retry/DLQ (área do ACHADO-2) intacta.
- `cargo test -p lina-core --lib` → **EXIT=0** (286 passed) — inclui os **7 spawn tests** (M1-M4 intactos)
  + os **4 ACHADO-2 não-vacuosos** (`delivery_not_ready_is_retained_not_failed`,
  `real_delivery_failure_still_strikes`, `retained_not_ready_delivers_when_ready_via_retry_engine`,
  `deliver_not_ready_never_injects`).
- `cargo test -p lina-bootstrap` → **EXIT=0** (todas as suítes; skills installer + retro_cli +
  doctrine_verbs verde — o `lina plan add` fantasma do red-team foi consertado por peer).

## Re-verificação por invariante (arquivo:linha)

### Invariante 1 — `09876a4`: not-ready vira RETENÇÃO, não cria entrega-sem-prontidão nem enfraquece DLQ/breaker
- **(a) Zero entrega sem checagem de prontidão REAL (anti-texto-colado vivo).** `deliver_a2a`
  (`a2a.rs`): se `!wait_ready(...)` → `return Ok(DeliveryOutcome::Injected{ready:false})` **ANTES de
  qualquer write** (o write só ocorre após `let ready=true`). O router reclassifica pelo RESULTADO via
  `DeliveryOutcome::injected()` (`a2a.rs` novo impl; `router.rs:1092` arm `Ok(outcome) if
  outcome.injected()`): `ready:false` → `injected()==false` → arm de RETENÇÃO (`router.rs:1138`,
  `single_target → retain_not_ready`), **jamais** o caminho de entregue. Re-checagem real da prontidão
  fica no motor de re-entrega (`process_due_retries` re-roda `wait_ready` com backoff). **Prova
  não-vacuosa:** `deliver_not_ready_never_injects` (`sup.applied_ops(target).is_empty()`) +
  `delivery_not_ready_is_retained_not_failed` (zero `MessageDeliveryFailed`/`MessageDeadLettered`/
  `CircuitOpened`/strike). ✅
- **(b) Falha REAL ainda strike (não enfraquece DLQ/breaker).** O arm `Err(e)` segue intacto →
  `on_delivery_failure` (strike/backoff/DLQ). `deliver_a2a` só devolve `Err` em falha REAL (regex
  inválida, allow-list, supervisor/PTY morto/lock — `a2a.rs` comentário do `A2aError`). **Teste
  `real_delivery_failure_still_strikes` é NÃO-VACUOSO:** `Err("escritor morto")` → `RetryBackoff` +
  `MessageDeliveryFailed=1` + `node_failures[b]=1` + `MessageRetained` vazio. Quebraria se o fix
  reclassificasse TODO problema como retenção (o oposto do bug). ✅
- **(c) Backstop 600s (sem fila infinita).** `retain_not_ready` enfileira `attempts:0` +
  `next_ms = now + NOT_READY_BACKOFF_MS(2s)`; o re-try (`router.rs:1504`, arm `Ok(_)`) checa
  `now - first >= retention_timeout_ms(600s)` → `dead_letter_now`. `MessageRetained` 1× (anti-A4 via
  `retained_since`). ✅
- **Segurança: 0 ALTA. FAIL-SAFE** — o caminho de erro nunca injeta sem prontidão; na dúvida, retém
  ou (no restart) descarta — **nunca** cola texto num alvo ocupado.

#### Ressalva R1 (DISPONIBILIDADE/durabilidade — NÃO segurança-ALTA; para a lente de disponibilidade)
A retenção por not-ready ocorre **DEPOIS** de `MessageRouted` ser logado (apêndice antes do loop de
entrega, `router.rs:1004`), mas guarda o estado **só em memória** (`self.retries`, `attempts:0`) +
`MessageRetained` — e `MessageRetained` **NÃO** é rastreado pelo `delivery_ledger` (`router.rs:1883`,
que só indexa `MessageRouted/Delivered/Failed/DeadLettered`). **No RESTART** durante a janela de
retenção, `self.retries` se perde e o ledger vê "roteada-sem-entrega" → arm conservador `Some(_)`
(`router.rs:599`) **assume entregue → ack → DESCARTA** a msg (o colega nunca recebe; o log lê
"assumida-entregue"). Pré-ACHADO-2 esse caso era DURÁVEL (`Err(NotReady)`→`MessageDeliveryFailed`,
rastreado pelo ledger → re-derivava o retry no restart). É a **mesma classe** do meu achado de
red-team **M3** (caminho de evento fora do `delivery_ledger` → sem durabilidade no restart).
**Security-wise é fail-safe** (perda, não injeção-insegura), por isso **não** é ALTA desta lente.
**Recomendação:** para paridade de durabilidade, o ledger deveria tratar
`MessageRetained{prompt_not_ready}` como "pendente, não-entregue" (restart re-checa prontidão) em vez
de o `Some(_)` assumir entregue — fix da mesma família do M3.

### Invariante 2 — `bc58f0b`: instalador escreve só em `lina-*`, sem traversal, preserva user-skill
- **Sem traversal — paths são literais de compilação.** `install_skills_into(root)` (`skills.rs`)
  itera `LINA_SKILLS` (catálogo `&'static`): `dir = root.join(skill.name)` com `skill.name` HARDCODED
  (`"lina-agent-bus"`…, todos `lina-*`); `dest = dir.join(rel)` com `rel` HARDCODED (`"SKILL.md"`,
  `"references/rubrica.md"`) e conteúdo via `include_str!`. **Nenhum input de runtime / nome de arquivo
  do disco entra no caminho** → traversal impossível por construção. ✅
- **Aditivo/idempotente, user-skill intacta.** Cada skill em pasta própria `lina-*`; idêntico não
  reescreve; `write_atomic` (tmp+rename). Teste `second_run_is_noop_and_preserves_user_skills`:
  `minha-skill/SKILL.md` do usuário **intocada**; 2ª rodada byte-idêntica. Único "overwrite" possível
  é se o usuário ocupar o namespace reservado `lina-*` (intencional, não vulnerabilidade). ✅
- **Bloco marcado da doutrina intacto.** `upsert_marked_block` (`LINA:START..LINA:END`) **inalterado**
  por este commit (só trocou `install_skill`→`install_skills_into`); conteúdo do usuário fora do bloco
  preservado. Anti-drift `catalog_matches_assets_dir` (catálogo == disco). Kit por-nó
  (`lib.rs:write_terminal_files_with_hooks` → `install_skills_into(claude.join("skills"))`) usa o
  MESMO caminho seguro. ✅

### Invariante 3 — `1029151`: skill lina-spawn-terminal NÃO ensina contornar gate
A skill ensina o **OPOSTO** (§"Os gates são física do mundo — não obstáculos", linhas 67-82):
*"Não há contorno — por construção. A distinção origem×cascata vem de um binding INTERNO do router,
nunca de campos da mensagem: forjar `hops`/root no envelope não muda nada. Pedir a um colega que
spawne por você também não funciona: o pedido viaja A2A, o colega herda a cadeia e cai no MESMO gate
de cascata. ... Tentar contorná-lo é violação de doutrina."* "Re-tentar após gate" está listado como
ERRO ("tempestade"). O conteúdo é **fiel ao código** (casa com o red-team) e **não revela bypass real**
(não menciona poda-60s/M1 nem spawnado-sem-binding/M4 — que são forward/seam, não exploráveis no core).
Cascata/cap/manual/custo descritos como física inviolável. ✅

### Invariante 4 — `988d1c7`: retro — porta única, Refused antes de qualquer write
- **Porta única:** `run_retro` (bin) → `classify_retro_args(args)` (`retro.rs:513`). Qualquer token de
  `MUTATION_TOKENS` (`apply/archive/pin/unpin/prune/delete/remove/consolidate/merge/write/fix` +
  variantes `--`) → `RetroInvocation::Refused{mutation:true}` **imediatamente**, antes de qualquer
  processamento. Só `--json` e `--now-ms <n>` → `Report`; posicional desconhecido → `Refused`
  (nunca degrada para ação). ✅
- **Verbo sem write:** o ramo `Report` apenas `read_to_string(event_log_path())` (espelho `log.jsonl`,
  SÓ-LEITURA) → `project_retro` (projeção pura sobre `&[EventRecord]`) → `render_report` (`&mut String`)
  → `println!`. **Zero `store.append`/`fs::write`** em todo o `retro.rs` (grep confirma: as únicas
  "escritas" são o buffer `&mut String` do render e `Vec<EventRecord>` em helpers de teste). "Por
  construção não existe aresta de um arg-de-mutação até um `append`." retro está **isolado** do
  router/gate (não toca `router.rs`/`events.rs`/`mailbox.rs`). Teste `gate_refuses_mutation_attempts`. ✅

### Invariante 5 — gate_w34 segue cobrindo
`gate_w34` EXIT=0 (1 passed) + `gate_f1_0_7` EXIT=0 (7) + 286 lib + 52+ bootstrap. Nenhum assert
afrouxado; a suíte de segurança do router está intacta. ✅

## MEDIA conhecidos (M1-M4 do red-team) — bounds INALTERADOS
- **M1 (cascata time-windowed, poda 60s):** `derive_root_hops`/`prune_expired` **não tocados** por
  nenhum commit; os 7 spawn tests passam. Bound inalterado.
- **M2 (manual desarmado em prod / autonomia não-fiada):** `app/lina-gpui/src/bridge.rs:653` segue
  `RouterConfig{ token_budget_day, ..default() }` → `autonomy=Assisted` (default). `dd32d46` tocou
  `bridge.rs` SÓ em `admission_events` (`requested_by:None`), não no `RouterConfig`. **Ainda
  forward/seam.** Bound inalterado.
- **M3 (spawn fora do delivery_ledger / sem dedupe durável):** `handle_spawn` **não tocado**; bound
  inalterado. **NOVO:** o ACHADO-2 introduz uma retenção da MESMA classe de durabilidade (ver R1) —
  recomendo tratar as duas juntas.
- **M4 (terminal spawnado nasce sem binding):** o caminho `SpawnApproved→admit_node→1º prompt`
  **continua NÃO fiado** — `dd32d46` confirma `admission_events` hardcodando `requested_by:None`
  (admissão de origem humana); nenhum consumidor do `pump` age no `SpawnApproved`. **Ainda forward/seam.**
  Bound inalterado.

## Notas de honestidade
- READ-ONLY: nenhum `.rs` modificado. Criei só este relatório + o sinal `.iniciado-lente3`.
- `git status` mostra relatórios de lentes-peer (`conselho-f13-lente1-visao.md`,
  `conselho-f13-lente4-pesquisa.md`) untracked — **não meus**, não toquei.
- O trabalho de F1-3-7 (`lina retro`) que estava não-commitado no meu red-team agora está **committed**
  (`988d1c7`); auditei a versão committed.

## Conclusão da Lente 3
**OK-COM-RESSALVAS · 0 ALTA novo.** Os 5 commits pós-red-team preservam todos os invariantes de
segurança (entrega-com-prontidão, escrita-gateada, sem-traversal, gate-inviolável do retro,
gate_w34 intacto) e os MEDIA M1-M4 mantêm seus bounds aceitos. A única ressalva (**R1**) é de
**durabilidade/disponibilidade** (retenção por not-ready não sobrevive ao restart → perda de msg
fail-safe), da mesma família do M3 — **fora do escopo-ALTA de segurança**, encaminhada à lente de
disponibilidade/Conselho com recomendação de fix (ledger rastrear `MessageRetained` como pendente).
Da ótica de segurança, o gate da onda F1-3 **passa**.
