# Conselho de Gate F1-3 — LENTE 2: ARQUITETURA (ADR↔CÓDIGO)

> **Auditoria independente read-only** sobre `HEAD=09876a4` (árvore limpa exceto marcadores
> `.iniciado-*` e relatórios de outras lentes — não tocados). Auditor **não construiu nada da
> onda** (F1-3-5 anterior travou e foi reatribuída — limpo de auto-endosso).
> **Método:** cada check re-derivado **no fonte** (arquivo:linha do HEAD atual — não no relato
> dos workers); varredura ampla por 4 agentes `Explore` (read-only, sem Edit/Write) cujos achados
> foram **re-verificados linha a linha** antes de assinar (1 falso-positivo de finder derrubado —
> ver §Falso-positivo). **Evidência executável:** `cargo test -p lina-core` → **EXIT=0, 18 suítes,
> 0 falhas**; `cargo test -p lina-bootstrap` → **EXIT=0, 12 suítes, 0 falhas** (exit codes
> diretos, sem pipe; logs em `/tmp/lente2-{core,bootstrap}-tests.log`).

## VEREDITO: **OK-COM-RESSALVAS**

Zero violação bloqueante ADR↔código no committed da onda. Todas as divergências encontradas são
(i) **documentadas no próprio código e formalizadas no red-team** (`redteam-spawn-f1-3-6.md`,
0 ALTA / 4 MEDIA / 1 BAIXA), (ii) **pré-existentes à onda** (herdadas, não regressões), ou
(iii) **forwards declarados do seam da tela** (rodada CORE-só por decisão registrada do Maestro).
Ressalvas nomeadas com linha em §Ressalvas.

---

## Check (a) — ADR 0019 §6 spawn caps ↔ `router.rs::handle_spawn` ✅ (com R1/R2)

| Promessa do ADR | Código (re-derivado) | Veredito |
|---|---|---|
| `max_spawns_per_turn = 2` | `router.rs:34` (`MAX_SPAWNS_PER_TURN: u32 = 2`) + default `router.rs:139` | ✅ |
| origem×cascata via `derive_root_hops`, NUNCA `msg.hops` | `handle_spawn` lê `self.derive_root_hops(msg, sender)` em `router.rs:1777`; `derive_root_hops` (`router.rs:1599-1604`) lê **só** `delivered_root[sender]` — `msg.hops`/`msg.root_cause_id` ignorados por construção (nenhum uso decisório no arquivo; testes `router.rs:2590`, `2623` provam forja inerte) | ✅ |
| cascata gateada | `hops >= 1` → `SpawnGated{cascade}` em `router.rs:1805-1812`, **ANTES** do cap (`1814`) — a defesa anti-fork-bomb não depende do contador | ✅ (mais restritivo que o ADR — ver R2) |
| `requested_by` autenticado | `SpawnRequested{requested_by: sender}` (`router.rs:1783`) — sender resolvido pelo supervisor, jamais `msg.from`; logado **antes** da decisão (`1781`, intent-vs-action) | ✅ |
| spawn conta no teto de custo (ADR 0005) | gate de custo fail-closed `router.rs:1835-1848` (`CostLedger::replay`; falha de leitura → `PersistFailed`, não aprova) | ✅ |
| interceptação | `route_message` `router.rs:767-768`, molde do `is_plan_intent` (`756`), pós-autenticação do sender e `validate_envelope_v2` | ✅ |

## Check (b) — ADR 0007: binding inforjável REUSADO, não duplicado ✅

- `handle_spawn` chama `self.derive_root_hops` (`router.rs:1777`) — **reuso puro**. Varredura de
  `delivered_root` no arquivo: os únicos acessos são o carimbo na entrega (`1125-1126`,
  `1500`), a consulta anti-envenenamento (`1121`, `1495-1497`), a poda (`1587-1589`) e o próprio
  `derive_root_hops` (`1600`). **Nenhuma segunda derivação de hops existe.**
- Propriedade do ADR 0007 preservada: nenhum campo escrito pelo agente decide origem/cap/autorização
  (cruzado com red-team INV1/INV3: "holds, none").

## Check (c) — `lina retro`: mutação Refused-por-construção + zero LLM ✅

> Nota de path: `retro.rs` vive em `crates/lina-bootstrap/src/retro.rs` (não em lina-core,
> como o despacho sugeria). Sem impacto — é a casca correta: o bin lê o espelho `log.jsonl`.

- **Porta única:** `classify_retro_args` (`retro.rs:513-549`) é whitelist estrita — só `--json` e
  `--now-ms <n>`. `MUTATION_TOKENS` (`482-505`: apply/archive/pin/prune/… + flags) →
  `Refused{mutation:true}` (`519-523`) **antes de qualquer caminho de escrita**; posicional
  desconhecido → `Refused{mutation:false}` (`539-544`) — **nunca degrada para ação**. Não existe,
  por construção, aresta de arg-de-mutação até um `append`. O bin (`bin/lina.rs:891-896`) passa
  pela porta única.
- **Zero LLM (inv#1):** varredura de `retro.rs` inteiro + handler do bin — zero import/chamada de
  rede/HTTP/SDK de LLM; `project_retro` é pura sobre `&[EventRecord]` + `now_ms` injetado
  (`retro.rs:1-20`), `BTreeMap` p/ determinismo; teste `determinism_byte_identical_across_runs`.
- **"Sugere, nunca aplica" (fio do fundador):** o verbo só observa/relata; as sugestões
  (stale/archive/consolidar) são texto para o humano decidir (`retro.rs:14-20`).

## Check (d) — eventos aditivos `serde(default)` ✅

- `NodeAdded.requested_by: Option<NodeId>` com `#[serde(default)]` (`events.rs:199-200`); doc
  explícita: log antigo (shape v2 `{node,kind,x,y}`) replaya com `None` **sem bump de versão**;
  campo META — não entra em `ProjectedNode`.
- Teste cru não-vacuoso: `node_added_requested_by_is_additive` (`events.rs:~2590-2644`) usa
  `insert_raw` com JSON **sem** o campo (tag `"event": "NodeAdded"` correto — o gotcha do
  serde(tag) foi evitado).
- Call-sites: 19 no workspace, todos com `requested_by` explícito (8 do app fixados em `dd32d46`).
  Compilação verde do workspace prova a totalidade.
- Eventos novos da onda (`SpawnRequested`/`SpawnGated`/`SkillInvoked`/`SkillCreated`,
  `events.rs:652-692`) são **variantes novas** (aditivas por natureza; log antigo não as contém);
  `apply` as absorve sem efeito de projeção (META).

## Check (e) — NodeId só no Supervisor ✅ (com nota)

- Caminho da onda: `handle_spawn` **não cunha NodeId nem abre PTY** (`router.rs:1739-1741` doc +
  corpo `1751-1869` sem nenhum `Uuid::now_v7`) — devolve `SpawnApproved` com strings; a autoridade
  é `Supervisor::register` (`lib.rs:1186`), e no app o NodeId canônico nasce em `sup.register`
  (`bridge.rs:1969`, via `wire_terminal`).
- **Nota (fora do escopo da onda):** `PtyHost::spawn` (`lib.rs:380`) cunha `Uuid::now_v7()` como
  chave do **seu** mapa de terminais — código da **Onda 0** (`e95e2bf`, 2026-05-30, pré-onda),
  caminho headless que o app **não usa** (`bridge.rs:3853-3854`: app roda PtyManager-direto,
  "zero PtyHost"). Não é regressão da onda; registrado para a arrumação futura da migração
  app→PtyHost (já mapeada em F1-1-8).

## Check (f) — Âncoras do CLAUDE.md intactas ✅

- **Prova direta:** `git show --stat` dos 12 commits da onda (`7d2c198..09876a4`) — **nenhum**
  toca `lina-host`, `lina-vt` ou `lina-cli-profiles`.
- `UiHost` (`lina-host/src/lib.rs:136`) e `VtBackend` vivos; `lina-host` depende só de `uuid`;
  `lina-core/Cargo.toml` sem gpui/wgpu/slint (grep `use gpui` no core: zero) — inv#7 mantido.
- **Envelope A2A:** `A2aEnvelope` (`lib.rs:149-162`) com `id/root_cause_id/from/to/intent/hops/
  await` — nenhum campo removido/renomeado na série.
- **Event Store:** append-only intacto (só INSERT; diff da onda em `events.rs` é aditivo);
  upcasting preservado (campos novos = `serde(default)`, sem entrada em `upcast`).
- **Bus/Supervisor:** orquestração nova (spawn) pendurada no broker existente, sem formato paralelo.

## Check (g) — os 2 fixes do gate ✅

### `bc58f0b` — instalador de skills (ACHADO-1)
- **Aditivo:** cada skill em pasta própria; skill estrangeira do usuário no mesmo root permanece
  intocada — teste `second_run_is_noop_and_preserves_user_skills` (`skills.rs:190-216`).
- **Idempotente:** conteúdo idêntico → skip (`skills.rs:76-78`); 2ª rodada = árvore byte-idêntica
  (mesmo teste); escrita atômica tmp+rename (`skills.rs:88-92`).
- **Bloco marcado preservado:** `upsert_marked_block` (`global_install.rs:100-129`) — cria/anexa/
  substitui **só o miolo** `LINA:START..LINA:END`; testes `preserves_user_content_and_is_idempotent`
  (conteúdo do usuário + 1 único bloco) e `updates_block_in_place_when_doctrine_changes` (moldura
  do usuário intacta, miolo antigo substituído) (`global_install.rs:186-246`).
- **Anti-drift:** paridade catálogo×disco (`catalog_matches_assets_dir`, `skills.rs:134-162`) —
  fecha a **classe** do ACHADO-1, não só a safra de hoje. Safra completa nos 2 caminhos: por-nó
  (`lib.rs:468`) e global por-CLI (`global_install.rs:82`).

### `09876a4` — retenção not-ready (ACHADO-2)
- **Anti-texto-colado preservado:** `a2a.rs:343-348` — prontidão falhou → `Ok(Injected{ready:false})`
  **antes** de qualquer `lock_pty`/`enqueue_write` (zero write por construção; o write só existe
  após `ready=true`, `a2a.rs:349-360`). `A2aError::NotReady` removido (`a2a.rs:59-61`).
- **Strikes reservados a falha real:** tripartição no router — `Ok(injected())` = entregue
  (`router.rs:1095`); `Ok(!injected)` = **RETÉM** via `retain_not_ready` (`1146-1152`);
  `Err` = strike via `on_delivery_failure` (`1153-1163`). `retain_not_ready` (`1309-1350`):
  `MessageRetained` 1× (anti-amplificação via `retained_since`) + entrada no motor com
  `attempts: 0` (`1343`) — retenção, não falha. `process_due_retries` re-checa prontidão com
  backoff `NOT_READY_BACKOFF_MS = 2_000` (`131`) **sem** strike nem incremento de attempts
  (`1533-1537`), interceptado pré-ledger (sem `MessageRouted` spam — doc `1304-1308`).
- **Backstop 600s vivo:** `RETENTION_TIMEOUT_MS = 600_000` (`router.rs:119`); estouro → DLQ com
  motivo nomeado (`1512-1532`).
- **Não-vacuosidade:** `real_delivery_failure_still_strikes` (`router.rs:5088-5112`) prova que
  escritor morto **ainda** strike; `retained_then_ready_delivers_once` (`5122+`) prova entrega 1×
  pós-backoff. Suíte verde observada (EXIT=0).

## Invariantes 1-7 (CLAUDE.md)

| # | Invariante | Veredito | Evidência-chave |
|---|---|---|---|
| 1 | Sem LLM próprio | ✅ | retro determinístico (check c); gate de spawn por regras, não LLM |
| 2 | Local-first | ✅ | instaladores escrevem só em disco local (`~/.claude` etc.); zero rede nova |
| 3 | Neutralidade multi-CLI | ✅ | `global_install.rs:47-60` cobre os 3 CLIs; profiles TOML intocados |
| 4 | Event log = verdade | ✅ | `SpawnRequested`/`SpawnGated` ANTES do efeito (`1781`, `não decidir o que não logamos`); retro = projeção pura |
| 5 | Pertencimento = conexão | ✅ | nada na onda introduz topologia por cabo |
| 6 | Não-técnico-first | ✅ | skills/doutrina em pt-br; outcomes narrados, nunca silêncio (`bridge.rs:833`) |
| 7 | Core/shell split | ✅ | core decide, app executa; zero import de toolkit no core |

## Ressalvas (nenhuma bloqueante — todas registradas/datadas)

- **R1 — cap de origem não morde rajada** (`router.rs:1814-1816`, comentário honesto no código):
  keyed `(root, sender)` com root **fresco** por `lina spawn` de origem → o cap=2 do ADR 0019 §6
  ("por nó solicitante") só morde dentro de root compartilhado. **Aceito e formalizado** no
  red-team (INV3, "gap aceito") — mas o **texto do ADR 0019 §6 não registra a aceitação**.
  Recomendação: 1 linha no ADR.
- **R2 — código mais restritivo que o ADR 0019 §6 na cascata:** o ADR diz "cascata conta contra o
  cap e o DELEGATION_BUDGET"; o código **recusa cascata sempre** (`SpawnGated{cascade}`,
  `router.rs:1805`) — nem chega ao cap. Mais seguro, porém divergência textual; alinhar o ADR.
- **R3 — autonomia não fiada em produção** (= M2 do red-team, **pré-existente**): `bridge.rs:653-656`
  constrói `RouterConfig{token_budget_day, ..default()}` → `autonomy=Assisted` fixo
  (`router.rs:141`). O ramo manual do core (`1796`) é real e testado (`spawn_manual_autonomy_blocks_
  in_core`, `4890`) mas **inoperante em prod**. Herdado de `f3cd2a6` (2026-06-03, Onda 4 — pré-onda);
  escalado na entrega; pré-requisito do seam.
- **R4 — seam da tela pendente (declarado):** `SpawnApproved` **não tem consumidor no app** (grep
  zero em `app/`); cai no `other => eprintln!` (`bridge.rs:833` — visível, nunca silencioso). A
  execução física (`admit_node` + 1º prompt + banner humano de cascata/cap) é a rodada do seam,
  por decisão registrada (doc do `handle_spawn`, `router.rs:1741-1743`). Ao fiar o seam, resolver
  **M4** (terminal spawnado nasce sem binding — rotear o 1º prompt por `route_message` ou carimbar
  binding na admissão, senão a guarda de cascata é derrotada end-to-end) e **M3** (dedupe durável
  do spawn — fora do `delivery_ledger`).
- **R5 — M1 do red-team (doc-vs-código ADR 0007):** a poda de 60s do binding
  (`prune_expired`, `router.rs:1587-1589`) faz ex-filho ocioso voltar a "origem" — o "cascata
  SEMPRE gate humano" do ADR vale **enquanto o binding vive**. MEDIA justificada (alavanca é tempo,
  não campo forjável; sem velocidade de fork-bomb); documentar a janela no ADR 0007.
- **R6 — nota PtyHost** (check e): cunhagem de uuid pré-onda no caminho headless; zero impacto no
  funil do app; arrumar na migração app→PtyHost.

## Falso-positivo de finder (derrubado por re-derivação)

Um agente de varredura alegou "violação: `PtyHost::spawn` cunha uuid órfão descartado em
`bridge.rs:1966`". **Errado no mecanismo:** `bridge.rs:1966` chama `PtyManager::spawn(key, …)`
(4 args, keyed por **string** — crate `lina-pty`, zero cunhagem de NodeId), não `PtyHost::spawn`
(3 args, `lib.rs:379`). O NodeId canônico nasce em `sup.register` (`bridge.rs:1969`). Sobrou só a
nota R6 (pré-onda, caminho não usado pelo app).

## Honestidade / escopo do que foi comparado

- **Read-only cumprido:** os únicos arquivos que criei são este relatório e `.iniciado-lente2`.
  Subagentes da varredura: `Explore` (sem Edit/Write); `git status` re-checado — sem mudanças além
  dos marcadores pré-existentes de outras lentes.
- **Escopo da prova executável:** as 30 suítes verdes (core+bootstrap) provam o comportamento dos
  módulos auditados nos checks (a)-(d) e (g) **no commit HEAD**; não provam o seam do app (R4, não
  fiado) nem comportamento em tela (fora do escopo desta lente).
- Linhas citadas referem-se ao **HEAD `09876a4`** (as linhas do red-team referem-se a `76a3ccd` e
  estão deslocadas ~+70 no router — reconciliadas nesta auditoria).
