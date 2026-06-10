# Red-team de INVARIANTES — verbo `lina spawn` (F1-3-6, commit `76a3ccd`)

> **Auditor de invariantes** (READ-ONLY no código de produção; nada editado/commitado).
> **Escopo auditado:** o commit `76a3ccd` (presente no `HEAD=1029151`). Rodada **CORE-só** (Maestro
> decisão 4): o gate **DECIDE** e devolve `RouteOutcome`; a criação física (`admit_node`/PTY/1º prompt)
> é o **seam da tela** (próxima rodada) e **NÃO está neste commit** (`bridge.rs`/`main.rs` não foram
> tocados — confirmado por `git show --stat 76a3ccd`).
> **Método:** cada invariante foi **re-derivado no código** (arquivo:linha), com **evidência observada**
> (`cargo test -p lina-core --lib spawn` → **9 passed, EXIT=0**), e cruzado com um **Workflow de 10
> céticos adversariais independentes** (read-only). Toda alegação de bypass dos céticos foi
> **re-derivada no código** antes de assinar severidade (céticos inflam/erram o mecanismo).

## Veredito

**0 ALTA aberto.** Critério de saída do gate da onda **ATENDIDO** para o código committed.
4 achados **MEDIA** (2 já escalados pelo autor; 2 **novos** desta auditoria) + 1 **BAIXA**. Os 4
"bypass" rateados pelos céticos foram re-derivados — **nenhum é bypass de campo-de-autorização /
identidade / cascata-viva-aprovada**: são divergência doc-vs-código, durabilidade e risco
estrutural do seam (detalhe na §Reconciliação).

## Tabela invariante → linha que garante → veredito

| # | Invariante | Linha que garante | Veredito |
|---|---|---|---|
| 1 | Nenhum campo escrito pelo agente (`msg.hops/root_cause_id/from/ref/payload`) decide o gate — só o binding + sender autenticado | `router.rs:720` (sender=`node_by_name`) + `mailbox.rs:519` (drain carimba `from`=dir-dono) + `router.rs:1657` (`derive_root_hops`) + `router.rs:1479-1483` (ignora `msg.hops/root`) | ✅ **VALE** |
| 2 | Cascata (`hops` efetivo ≥1) SEMPRE gate humano, sem caminho que aprove | `router.rs:1685` (`if hops>=1 → SpawnGated`, antes do ALLOW `1742`) | ⚠️ **VALE com ressalva** → **M1** (a *classificação* cascata depende de binding fresco; poda de 60s reabre como origem) |
| 3 | Cap `(root,sender)=2` consultado no gate, não-contornável por root forjado | `router.rs:1697-1709` (lê `spawn_count`) + `1479-1483` (root inforjável) + incremento atômico `1740` | ✅ **VALE** (origem-burst com root fresco = gap **aceito**) |
| 4 | Manual bloqueia no CORE (não só no bin) | `router.rs:1676` (`autonomy.blocks_delegation()→SpawnBlocked`) | ⚠️ **VALE no core / DESARMADO em prod** → **M2** (autonomia não-fiada no app) |
| 5 | Spawn conta no teto de custo / custo pausado gateia | `router.rs:1715-1728` (gate condicional, fail-closed) + `CostLedger::replay` `2064` | ✅ **VALE** (teto OFF por default = opt-in; ver **B1**) |
| 6 | NodeId nunca cunhado fora do Supervisor | `router.rs:1631-1748` (não cunha id/PTY; devolve strings) + `lib.rs:1186` (`Uuid::now_v7` só em `Supervisor::register`) | ✅ **VALE** |
| 7 | Replay de log antigo (`NodeAdded` v2 sem `requested_by`) não quebra | `events.rs:199-200` (`Option`+`serde(default)`) + `768-774` (v2 sem bump) + `apply` `902` (`requested_by:_`) + variantes novas no silêncio `1053-1058` | ✅ **VALE** (teste cru não-vacuoso `events.rs:2604`) |
| 8 | `gate_w34` cobre o que cobria antes (diff = só linha de compilação) | diff dos 3 gates = **só** `+ requested_by: None,` (compile-fix do campo aditivo); nenhum assert afrouxado; `gate_w34` verde | ✅ **VALE** |

## Achados (severidade + evidência arquivo:linha)

### M1 — `MEDIA` (doc-vs-código + binding efêmero p/ decisão que cria processo) — **NOVO**
**Invariante 2.** A guarda `hops>=1 → SpawnGated` (`router.rs:1685`) é incondicional e **não há
caminho que aprove uma cascata viva**. Mas a *classificação* origem×cascata vem de
`derive_root_hops` (`router.rs:1479-1483`), que lê `delivered_root[sender]`. O `prune_expired`
(`router.rs:1467-1469`, chamado no topo de `route_message:695`) **poda o binding** quando
`now-last ≥ 60s` (janela `DEDUPE_WINDOW_MS`) e o nó **não** tem await pendente. Sequência:
A→B entrega em T=1000 (binding[B]=(root_A,1,1000)); B ocioso ≥60s; no `lina spawn` de B em
T≥61000 o `prune_expired` remove binding[B]; `derive_root_hops` devolve `(msg.id,0)` → `hops=0`
→ **origem** → passa a guarda de cascata → `SpawnApproved` (`router.rs:1742`).

**Por que MEDIA e NÃO ALTA (re-derivação vs. cético que rateou ALTA):**
1. A alavanca é **tempo** (esperar), **não um campo forjável** — preserva a doutrina de segurança
   (nenhum campo do agente decide); a forja de `hops/root` continua inerte (INV3 + teste `4691`).
2. É a **poda Round 5 #8 herdada**, válida em TODO o router (também reseta o `DELEGATION_BUDGET` e
   a partição do anti-loop) — não é lógica nova/quebrada do spawn. Em T≥61000 o `hops` efetivo **é**
   0 pela própria semântica consistente do sistema (cadeia morta → o nó volta a ser sua raiz).
3. **Não há velocidade de fork-bomb:** ≥60s por nível ⇒ vazamento linear/lento, não exponencial.
4. **Converge com o gap de origem-burst já aceito:** após 60s, um ex-filho só alcança o que uma
   origem já tem livremente — ganho incremental ~nulo sobre a decisão aceita.

**Divergência real:** ADR 0007 e `seam-lina-spawn.md §4` dizem "cascata **SEMPRE** gate humano" sem
o caveat de janela de 60s. **Recomendação:** (a) documentar a janela de liveness do binding em
ADR 0007/seam; **ou** (b) para o gate de spawn (que cria PROCESSO, mais sensível que delegação),
endurecer a liveness (sinal durável "este nó já recebeu A2A nesta sessão" ou janela maior). Baixo
impacto prático dado (4); vale o registro.

### M2 — `MEDIA` (gate honesto, mas DESARMADO em produção) — **já escalado** (`.entrega-f1-3-6.md §1`)
**Invariante 4.** `handle_spawn:1676` recusa quando `autonomy=Manual` — **código real, provado
headless** (`spawn_manual_autonomy_blocks_in_core`, `router.rs:4770`). Porém a autonomia **nunca é
fiada** no `RouterConfig` de produção: `app/lina-gpui/src/main.rs:3418` hardcoda
`Autonomy::Assisted`; `MailboxPump::new`/`bridge.rs:653` constroem `RouterConfig { token_budget_day,
..default() }` → `autonomy=Assisted` (default `router.rs:135`). Logo o ramo manual do core fica
**inoperante em produção** (um agente que escreva o outbox direto, pulando a recusa local do bin em
`lina.rs:638`, chega ao router em `Assisted`). **Não é forja de campo** (gate ignorado, não
enganado). Escalado como pré-requisito (Maestro decisão 2). **Recomendação:** fiar a conversão
`lina_bootstrap::Autonomy → lina_core::AutonomyLevel` no `MailboxPump::new`/`bridge.rs:653` na
rodada do seam; o verificador do seam deve **provar em tela** que `manual` bloqueia de fato.

### M3 — `MEDIA` (durabilidade: spawn sem dedupe durável → dupla-criação no seam) — **NOVO**
**Caçador de bypass.** O `delivery_ledger` (`router.rs:1883-1951`) — o **dedupe DURÁVEL** à prova
de restart/ack-falho — só indexa `MessageRouted/MessageDelivered/MessageDeliveryFailed/
MessageDeadLettered`. **Spawn não escreve nenhum desses** (intercepta e retorna em `router.rs:756`,
antes da seção de roteamento que apenda `MessageRouted` em `1004`). O único dedupe é o `seen` **em
memória** (`router.rs:702`), perdido no restart. Como `SpawnApproved` não é `hold` (`router.rs:652`),
o caminho feliz dá `ack_inflight` e remove o `.inflight` (`664`) — OK. Mas se `ack_inflight`
**falhar** (erro logado, não propagado — `665`) **ou** houver crash entre o `SpawnApproved` e o ack,
a msg permanece em `.inflight`; no próximo tick `drain_to_inflight` (`500`) a re-drena, o
`ledger.get(id)=None` (`535`) cai em `route_message` de novo (`607`) → **2º `SpawnRequested` + 2º
`SpawnApproved`**. Delegação **não** sofre isso (o `MessageRouted` durável a protege); **spawn ficou
fora desse framework**.

**Por que MEDIA e NÃO ALTA (re-derivação vs. cético):** **não é bypass de gate** (o gate roda
correto, só roda 2×); o "double-count do cap" do cético é **red herring** (o cap é em-memória,
zerado no restart, e origem não é capada). O risco real é **dupla-admissão no seam** (dois terminais
de um `lina spawn`) e exige `ack_inflight` falho/crash na janela estreita. **Recomendação:** dar ao
spawn um dedupe **durável** — `handle_spawn` consulta o log por `SpawnRequested`/`NodeAdded` com o
mesmo `msg.id` antes de aprovar, **ou** o app admite idempotente keyed em `msg.id`.

### M4 — `MEDIA` (risco estrutural do seam: terminal spawnado sem binding derrota a cascata) — **NOVO**
**Caçador de bounds.** Terminais criados por `admit_node` (`bridge.rs:3558`) entram no roster do
Supervisor **sem passar por `route_message`** → **nunca** ganham binding em `delivered_root`. Se o
seam injetar o 1º prompt **direto no PTY** (como o design indica — `seam-lina-spawn.md:72` "enfileira
na fila serial"), um terminal spawnado B que rode `lina spawn @C` é **origem** (`hops=0`, sem
binding) → a guarda de cascata (`router.rs:1685`) nunca dispara → a cadeia B→C→D fica toda "origem"
e a defesa central anti-fork-bomb do ADR 0007 é **derrotada end-to-end**.

**Por que MEDIA-forward e NÃO ALTA committed:** o caminho `SpawnApproved→admit_node→1º prompt`
**não existe neste commit** (seam da próxima rodada; o app sequer compila — `.entrega §Seam`). Logo
**não é defeito do core committed** — é o **risco estrutural nº 1 que o seam DEVE resolver**. M1 e M4
são **a mesma propriedade**: a detecção de cascata exige um binding fresco, e há **duas** formas de
faltar (ocioso-podado M1; spawnado-nunca-entregue M4). **Recomendação (escolher antes do seam):**
(a) rotear o 1º prompt **pela via de `route_message`** (carimba binding) — preserva a axiomática;
**ou** (b) carimbar `delivered_root[B]` na admissão com o `(root,hops)` efetivo do spawn (filho nasce
cascata); **ou** (c) aceitar+documentar que terminais spawnados são "origem" (enfraquece a guarda).

### B1 — `BAIXA` (backstop nomeado da origem está OFF por default) — subconjunto do gap aceito
**Invariante 5.** O bound aceito de **origem-burst** nomeia "teto de custo" como backstop, mas
`token_budget_day` default = `0` (DESLIGADO, `router.rs:137`) e a autonomia não-fiada (M2) deixa
`Assisted`. Em `autonomo` + custo-off, o spawn de origem fica limitado **apenas** pelo arbítrio do
próprio agente (cascata/cap não mordem a origem). O custo **é** contabilizado sempre
(`TokenUsageReported`) e **gateia quando configurado+pausado** (`1715`), então o invariante 5 **vale**;
o ponto é só que o backstop nomeado é opt-in. Já coberto pela aceitação da origem-burst; registrado
para o painel de custo/seam decidir se liga um teto default.

## Reconciliação com os céticos adversariais (10 agentes, Explore/read-only)

| Probe | Veredito do cético | Veredito do auditor (re-derivado no código) |
|---|---|---|
| INV1 identidade | holds, none | ✅ **confirmo** none |
| INV2 cascata | **ALTA** (poda 60s vira origem) | **MEDIA (M1)** — tempo≠campo; poda herdada system-wide; sem velocidade de bomb; converge com origem-burst aceito |
| INV3 cap-forjado | holds, none | ✅ **confirmo** none |
| INV4 manual | MEDIA | ✅ **confirmo MEDIA (M2)** — já escalado |
| INV5 custo | holds, none | ✅ **confirmo** (vale); registro **B1** sobre o default OFF |
| INV6 nodeid | holds, none | ✅ **confirmo** none |
| INV7 replay | holds, none | ✅ **confirmo** none |
| INV8 gate_w34 | holds, none | ✅ **confirmo** none |
| HUNT-bypass idempotência | **ALTA** (double-count cap) | **MEDIA (M3)** — não é bypass de gate; cap-double-count é red herring; risco real = dupla-admissão no seam |
| HUNT-bounds spawnado-sem-binding | **ALTA** (cadeia toda origem) | **MEDIA-forward (M4)** — caminho do seam, **não committed**; risco estrutural nº1 do seam |

**Nenhuma das 3 ALTAs dos céticos sobrevive à re-derivação como ALTA do código committed:** todas são
ausência/efemeridade de binding (tempo ou criação) ou durabilidade — **não** um campo escrito pelo
agente decidindo autorização/identidade, nem uma cascata **viva** sendo aprovada.

## Notas de honestidade / estado da árvore
- **Árvore de trabalho (multi-terminal):** há trabalho **não-commitado de peer** (F1-3-7 `lina retro`:
  `?? crates/lina-bootstrap/src/retro.rs`, `retro_cli.rs`, `assets/lina-skills/lina-retro/`, e `M`
  em `lina-bootstrap/src/{bin/lina.rs,lib.rs}`). Confirmei por `git diff` que esse delta **NÃO toca**
  `run_spawn/handle_spawn/max_spawns/spawn_count/is_spawn` — é F1-3-7. Os arquivos do **gate**
  (`router.rs/events.rs/mailbox.rs`) estão **intocados** na árvore = exatamente o committed `76a3ccd`.
- **Meus subagentes** são `Explore` (read-only: sem Edit/Write) e **não escreveram** no repo; o único
  arquivo que criei é este relatório + o sinal `.iniciado-redteam-spawn`. Não toquei trabalho de peer.
- **Evidência observada:** `cargo test -p lina-core --lib spawn` → `9 passed; 0 failed` (EXIT=0).

## Conclusão
O gate inforjável de `lina spawn` no **core committed** é **sólido**: identidade/cascata/cap/custo/
NodeId/replay/não-regressão todos verificados em arquivo:linha, com testes não-vacuosos. **0 ALTA** —
critério do gate da onda atendido. Os 4 MEDIA + 1 BAIXA são **carry-forward para o seam da tela** —
com destaque para **M4** (terminal spawnado deve nascer com binding de cascata, senão a defesa
anti-fork-bomb cai end-to-end) e **M3** (dar idempotência durável ao spawn). M1 e B1 são divergências
doc-vs-código de baixo impacto; M2 já estava escalado.
