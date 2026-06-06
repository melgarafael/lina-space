# Baseline F1-0 — bugs P0/P1 reproduzidos ANTES dos fixes

> **Story:** F1-0-1 (Harness de observação + baseline P0/P1) · **Onda:** F1-0 (Coordenação Confiável) · **Dono:** dev01 (QA/red-team).
> **Metade (i) do GATE F1-0** ([[ondas-0-1]] §GATE DE SAÍDA F1-0): a baseline **antes do primeiro merge de fix**. Sem esta metade o gate não pode ser declarado. O DEPOIS re-roda o **mesmo** roteiro e compara taxa-a-taxa.
> **Data da captura:** 2026-06-06, 12:53–12:58 (run de 5 claudes reais headless). **Status:** ✅ baseline capturada com evidência de log + grid.
> **Fontes:** `docs/DIRECIONAMENTO-coordenacao-a2a.md` §2 (P0/P1) e §4 (protocolo) · pesquisa 13.2 item 6 (baseline mandatória, por taxa). **Tudo 100% local, workspace ISOLADO** (`LINA_WS_ROOT=/tmp/lina-baseline-f1-0`) — o canvas Maestri vivo nunca foi tocado.

---

## Como reproduzir (ferramentas versionadas do repo)

Duas ferramentas novas, ambas **leitoras** do event log (`.lina/events/log.jsonl` — o espelho JSONL canônico; nenhum canal paralelo de verdade, âncora Event Store / invariante #4):

```sh
# 1) Runner headless da baseline — sobe N claudes REAIS via o core (PtyManager +
#    Supervisor + reader-thread → grid), replica o MailboxPump de produção (tick 120ms,
#    deliver_a2a com WorkspaceTrust + o demo_profile() de produção) e mede as 2 taxas.
cargo run --release -p lina-core --bin baseline_f1_0 -- \
  --ws /tmp/lina-baseline-f1-0 --claudes 5 --attempts 20 --rounds 5

# 2) Observador legível do log (filtra TokenUsageReported, imprime A2A/lifecycle com
#    nomes; --stats analisa atropelamento por alvo). Acompanha o stream vivo OU re-lê:
python3 tools/lina_watch.py --follow "/tmp/lina-baseline-f1-0/.lina/events/log.jsonl"
python3 tools/lina_watch.py --stats --replay "/tmp/lina-baseline-f1-0/.lina/events/log.jsonl"
```

Artefatos crus desta corrida (anexos da baseline): `<ws>/baseline-artifacts/summary.json`, `stuck.json` (detalhe por tentativa + região de input em t+3s/t+8s), `collision.json` (cada entrega com seq+ts), `grids/stuck-*.txt` (grid completo no momento do "preso").

---

## Condição de ambiente (vigência registrada — aviso do Maestro 2026-06-06)

O **ANTES e o DEPOIS do gate F1-0 devem rodar com o MESMO timing-base**, e o fix correto faz parte da base. Vigente nesta baseline:

- **`profiles/claude-code.toml` pós-fix F1-1-1 (Dev 02):** `idle_ms` e `ask_timeout_ms` agora são parseados (estavam sendo engolidos por estarem depois de `[end_signal]`; o fallback de idle do `EndDetector` rodava com DEFAULT=500ms). **Valores efetivos lidos em runtime pelo harness:** `idle_ms=1500`, `ask_timeout_ms=120000`, `prompt_ready_regex='(?m)^\s*[>❯]\s'`, `submit_delay_ms=300`. O harness **imprime esses valores no início de cada corrida** (carregados via `CliProfile::load_file`), então o DEPOIS prova vigência do mesmo timing-base por inspeção do próprio log/stderr.
- **Esta baseline NÃO rodou pré-fix.** Nada foi medido antes das ~12:30; a primeira (e única) corrida foi às 12:53, já com o `claude-code.toml` corrigido. Logo, **não há tentativas pré-fix a marcar ou re-rodar** — toda a baseline está sobre o timing-base correto. (Nota importante: o `claude-code.toml` é o perfil de **fim-de-resposta/lifecycle**; o perfil de **entrega A2A** medido aqui é o `demo_profile()` de produção — ver abaixo. O fix do Dev 02 afeta a detecção de idle, que é timing-base do lifecycle; mantê-lo fixo nas duas metades é exatamente o pedido do Maestro.)

---

## O que o harness replica de produção (fidelidade — não um mock)

| Caminho de produção | Onde (referência) | No harness |
|---|---|---|
| Tick do pump | `app/lina-gpui/src/bridge.rs:443-528` (~120ms) | `spawn_pump` (mesmo tick, mesmo `refresh_agents` + `Router::pump`) |
| Entrega A2A | `deliver_fn` (bridge.rs:406-440): grid do alvo + `WorkspaceTrust::from_members` + `deliver_a2a` | idêntico (closure `deliver` no pump) |
| **Perfil de entrega** | **`demo_profile()` (bridge.rs:327): `prompt_ready_regex='.'`, `submit_delay_ms=150`** | **réplica testada** (`tests::demo_profile_replica_fields` falha se produção divergir) |
| Canal CLI→supervisor | `lina ask` deposita no outbox por-nó; `from` autenticado pela origem no drain | `Mailbox::enqueue_as` (mesmo canal por-nó) |
| Fonte da verdade | event log SQLite + espelho `log.jsonl` | harness lê só o `log.jsonl` (`LogTail`) |

> **A causa-raiz hipotetizada do P0 (DIRECIONAMENTO §P0) é exatamente o `demo_profile()` ativo em produção** (`regex='.'` casa qualquer caractere → `wait_ready` nunca espera o prompt real; `submit_delay=150ms` curto). O harness entrega com ESSE perfil — por isso a baseline mede o bug REAL, não um artefato.

---

## Parâmetros desta corrida

- **5 claudes reais** headless: `Maestro` (orquestrador) + `W1..W4` (workers). Cada um é um processo `claude` real, cwd isolado em `<ws>/agents/<nome>`, que chegou ao prompt real (regex do `claude-code.toml` casou — confirmado no log por terminal).
- **Cenário 1 (texto-colado / P0):** 20 tentativas de `lina ask` (Maestro→W1) com **W1 forçado ocupado** (streaming) no instante da entrega — a condição do bug (§4.4). Marcador único por tentativa (`PROBE-<k>-<pid>`).
- **Cenário 2 (atropelamento / P1):** 5 rodadas; em cada uma os **4 workers** depositam um "RESULTADO" ao Maestro no mesmo instante (fan-out de tarefa curta que termina junto — §P1).
- Log final: 95 eventos (5×{NodeAdded,NodeRenamed,TerminalSpawned} + 40 MessageRouted + 40 MessageDelivered), seq 1→95, 12:53:32→12:58:47.

---

## Resultado P0 — texto-colado (taxa em ≥20 tentativas)

**TAXA DE BASELINE: `18/20` presos (90%).** Demais: 1 submetido, 1 "tardio" (limpou só em t+8s), 0 não-entregues. Alvo estava **ocupado em 20/20** tentativas (condição do bug garantida).

**Regra de detecção (determinística, testada em `tests::input_region_finds_live_prompt`):** após o `MessageDelivered` no log, amostra a **região de input vivo** do grid do alvo em **t+3s e t+8s**. Região = da última linha-prompt (`> `/`❯ `, com/sem borda `│`) até o fim do grid (calibrada contra o grid REAL, Claude Code v2.1.166). Marcador presente em ambas (ou persistente) = **preso**; ausente em ambas = **submetido**.

**Desfechos:** `{stuck: 18, submitted: 1, late_clear: 1}`. A intermitência (1 submeteu, 1 limpou tarde) é o **mesmo comportamento timing-dependente** que o fundador relatou ("às vezes vira texto colado") — por isso a baseline é por **taxa**, não single-shot.

**Evidência de grid (tentativa 2, marcador `PROBE-2-98138`, `MessageDelivered` seq 19, região em t+8s):**

```
  ❯ [LINA::MSG]
    id: msg_019e9da4-1aea-74e2-8b85-da2a9bec74a3
    ...
    payload: PROBE-2-98138 — mensagem de teste do harness; NÃO responda, ignore.
    [EXPECTED] responda ao colega no formato pedido; narre ao usuario so o resultado em pt-br.
    [/LINA::MSG]
  ────────────────────────────────────────────────────────────────────────
  ❯ Press up to edit queued messages
```

**Mecanismo observado:** com `regex='.'` + `submit_delay=150ms`, a colagem bracketed de um bloco grande num Claude **ocupado** cai no estado de paste-colapsado (`paste again to expand`) / mensagem-enfileirada (`Press up to edit queued messages`) — o Enter (0x0D) separado **não** produz um turno limpo. O texto fica visível no input, não submetido. *(Observação adicional: o paste preso é um chip colapsado que resiste a `Ctrl+U` — em várias tentativas o input seguiu sujo após 3×Ctrl+U, registrado no stderr; cada tentativa usa marcador único, então isso não contamina a contagem por-tentativa.)*

**Controle (dicotomia que prova a causa):** no smoke prévio com o alvo **ocioso** (Maestro na tela de boas-vindas), a entrega única foi **limpa** (`ROUTED→DELIVERED`, 0 preso). Ocupado → 90% preso; ocioso → limpo. O bug é função de o alvo estar ocupado no instante da entrega — exatamente a intermitência relatada.

---

## Resultado P1 — atropelamento (retornos ao mesmo alvo com <2s)

**BASELINE: `15/15` pares consecutivos ao Maestro DENTRO da mesma rodada com Δt < 2s.** Δ mínimo **155 ms**, Δ mediano **160 ms** (`collision.json`, autoritativo — filtra por rodada e só conta as entregas do cenário de colisão).

**Trecho do log (rodada 1, 4 retornos ao Maestro, com seq + timestamp absoluto):**

| seq | ts (ms) | hora | remetente | Δ vs anterior |
|---:|---:|:--|:--|---:|
| 57 | 1780761495399 | 12:58:15.399 | W1 | — |
| 59 | 1780761495555 | 12:58:15.555 | W2 | **156 ms** |
| 61 | 1780761496024 | 12:58:16.024 | W3 | **469 ms** |
| 63 | 1780761496183 | 12:58:16.183 | W4 | **159 ms** |

Os 4 retornos da rodada chegam ao input do Maestro em **~784 ms** (3 colisões <2s). O padrão se repete nas 5 rodadas (os gaps de ~7s entre rodadas são o *settle* proposital do harness, não entregas). Bate com a evidência original do DIRECIONAMENTO (seq 1853 G→A, +0.6s seq 1860 C→A): **entregas ao mesmo alvo colidem quando as tarefas terminam juntas**, porque `deliver_fn` injeta sem checar o estado (Busy/Idle) do alvo.

Cross-check legível (`lina_watch --stats`, sobre o log inteiro — mistura as entregas do cenário 1 (Maestro→W1, espaçadas pelos 3s+samples) com as do cenário 2; a separação autoritativa por rodada está em `collision.json`):

```
alvo Maestro: 20 entregas
  seq 57→59  Δ 156 ms  (W1 → W2)  ⚠ COLISÃO <2s
  seq 59→61  Δ 469 ms  (W2 → W3)  ⚠ COLISÃO <2s
  seq 61→63  Δ 159 ms  (W3 → W4)  ⚠ COLISÃO <2s
  seq 63→65  Δ 7355 ms (W4 → W1)            ← settle entre rodadas (não-colisão)
  ... (padrão idêntico nas rodadas 2–5: 3 colisões intra-rodada + 1 gap de settle)
```

---

## Resumo das taxas (o que o DEPOIS compara)

| Métrica | Baseline ANTES (esta corrida) | Alvo do DEPOIS (gate F1-0) |
|---|---|---|
| **Texto-colado (P0)** | **18/20 presos (90%)** com alvo ocupado | **0/N** nas mesmas N tentativas (F1-0-2: carregar `claude-code.toml` real na entrega) |
| **Atropelamento (P1)** | **15/15 pares <2s** (Δ min 155ms, med 160ms) | **0 injeção concorrente** — entregas ao mesmo alvo serializadas por estado (F1-0-4) |
| `RouteBlocked` | **0** nesta corrida (40 roteadas → 40 entregues) | **≈0 mantido** (não regredir o retry transiente `f353838`) |
| Nós-fantasma | n/a aqui (sem reabertura) | replay = roster vivo (F1-0-8) |

> **Protocolo do DEPOIS:** re-rodar `baseline_f1_0` com os **mesmos** `--claudes 5 --attempts 20 --rounds 5`, no mesmo timing-base (mesmo `claude-code.toml`), e comparar `summary.json` lado a lado. O entregável do gate é o **relatório com trechos de log antes/depois** (DIRECIONAMENTO §4.6), não "os testes passaram".

## Ressalvas honestas (metodologia)

1. **Force-busy é o protocolo (§4.4), e infla a taxa de propósito:** ao garantir o alvo ocupado em 20/20, medimos o pior caso (90% preso). Em uso real a intermitência vem de o alvo **às vezes** estar ocupado — a taxa real-mundo é menor, mas o que o fix precisa zerar é precisamente o caso ocupado. O DEPOIS roda o mesmo force-busy.
2. **`demo_profile()` é o perfil de entrega medido** (réplica testada de bridge.rs:327). Se o F1-0-2 trocar a entrega para o `claude-code.toml` real, o harness do DEPOIS deve continuar replicando **o que produção usa naquele momento** — a réplica e o teste `demo_profile_replica_fields` são o lembrete de re-sincronizar a fidelidade.
3. **A regra de "preso" é por região de input vivo**, calibrada contra o grid real (v2.1.166). Mudança grande na TUI do Claude Code pode exigir recalibrar `is_prompt_row`/`input_region` (cobertos por testes determinísticos).
4. **`collision.json` é a fonte autoritativa do 15/15** (filtra por rodada e por msg_id do cenário); o `lina_watch --stats` é cross-check legível sobre o log inteiro e por isso mistura os dois cenários — não usar o total bruto do watch como a métrica.

---

## DEPOIS — F1-0-4 (entrega ciente de estado / retenção) — atropelamento P1 ZERADO

> Corrida do **mesmo roteiro** de colisão (5 claudes reais, 5 rodadas, fan-out simultâneo ao Maestro) com a **retenção por estado ativa** (F1-0-4) sobre o stack de produção atual (F1-0-2: `claude-code.toml` real — `submit_delay_ms=400`, `idle_ms=1500`). Workspace isolado `/tmp/lina-depois-f1-0-4`. **Data:** 2026-06-06.

| Métrica P1 (atropelamento) | ANTES (baseline) | DEPOIS (F1-0-4) |
|---|---|---|
| Pares consecutivos ao mesmo alvo **<2s** | **15/15** | **0/15** |
| Δ **mínimo** entre entregas (mesma rodada) | 155 ms | **3194 ms** |
| Δ **mediano** | 160 ms | **9634 ms** |
| Entregas totais ao Maestro | 20 | 20 (nenhuma perdida) |

**A serialização é da RETENÇÃO, não de claudes lentos** (prova no log `/tmp/lina-depois-f1-0-4/.lina/events/log.jsonl`):
- **19 `MessageRetained{reason:target_busy}`** — quase toda mensagem que chegou com o Maestro ocupado foi retida (não injetada) em vez de atropelar.
- **18 `NodeStatusChanged{reason:a2a_delivery}`** — cada entrega marcou o alvo `Busy` (o que serializa a próxima); as que acharam o alvo já Busy são no-op (por isso 18, não 20).
- **20 `MessageDelivered`** — todas entregues exatamente 1× (0 perda, 0 duplicata), agora espaçadas pelo ciclo real de cada turno (Busy→Idle).

**Trecho do log (rodada 1, retornos ao Maestro — comparar com a tabela do ANTES, onde os 4 chegavam em ~784 ms):**

| seq | ts (ms) | remetente | Δ vs anterior |
|---:|---:|:--|---:|
| 18 | 1780771834793 | W1 | — |
| 25 | 1780771837987 | W2 | **3194 ms** |
| 29 | 1780771853893 | W3 | **15906 ms** |
| 33 | 1780771861748 | W4 | **7855 ms** |

Reproduzir: `cargo run --release -p lina-core --bin baseline_f1_0 -- --ws <novo-dir> --claudes 5 --scenario collision --rounds 5` (DEPOIS, produção atual) · adicione `--legacy-demo-delivery` para reproduzir o caminho de entrega do ANTES.

> **Ressalva honesta da comparação:** o DEPOIS roda com DUAS mudanças concomitantes vs o ANTES — o perfil de entrega real (F1-0-2) **e** a retenção (F1-0-4). A serialização (Δ >> 2s, 19 retenções) é atribuível **à retenção** — o perfil de entrega não serializa nada, só calibra o submit; o `RouteOutcome::Retained` + os `MessageRetained` no log isolam a causa. O P0 (texto-colado) DEPOIS é mérito do F1-0-2 (dono separado) e não foi re-medido aqui (esta rodada cobriu só o cenário de colisão / P1, que é o escopo da F1-0-4).

Cobertura headless (determinística, no crate — `cargo test -p lina-core`): `retencao_alvo_busy_nao_injeta_e_entrega_no_idle` (critério 1), `retencao_serializa_rajada_ao_mesmo_alvo` (critério 2), `retencao_estoura_teto_e_entrega_mesmo_busy` (critério 4 anti-deadlock), `retencao_via_pump_sobrevive_restart_fifo_exactly_once` (inv #4 + FIFO), `reply_retida_fecha_await_so_na_entrega` (reply retida ≠ loop).

---

PRONTO desta story (resumo na resposta ao Maestro). Artefatos versionados: `tools/lina_watch.py`, `crates/lina-core/src/bin/baseline_f1_0.rs` (+ testes), este `baseline-f1-0.md`. Dados crus das corridas: `<ws>/baseline-artifacts/` (não versionados; em `/tmp`).

---

# DEPOIS FORMAL (gate) — medição independente · 2026-06-06

> **Medidor:** Analista (construtor≠validador — não construiu nenhum fix da onda). **HEAD medido:** `51649c0` (F1-0-2/3/4/5/6/7/8/9 mergeados). **Workspaces ISOLADOS** em `/tmp/lina-gate-f1-0/{main,dlq,reopen,handoff}` — o canvas Maestri vivo nunca foi tocado. **Timing-base idêntico ao ANTES:** `profiles/claude-code.toml` vigente impresso pelo harness em cada corrida (`regex='(?m)^\s*[>❯]\s'`, `submit_delay_ms=400`, `ready_timeout_ms=30000`, `busy_markers=["esc to interrupt","Press up to edit queued messages"]`, `idle_ms=1500`). Claude Code v2.1.166→167 nas corridas. Exit code de TODAS as corridas: 0 (direto, sem pipe).

## Re-sincronização da réplica (ressalva 2 — deltas reportados)

O runner `baseline_f1_0` foi re-sincronizado com o core de HEAD (fronteira: bin do harness apenas; zero mudança em `src/` de produção):

| Delta réplica × produção | Ação |
|---|---|
| Pump narra `Retained/RetryBackoff/DeadLettered` no stderr (produção narra `other` em bridge.rs `tick`) | réplica igualada à produção |
| Boot do app chama `close_previous_generation` (F1-0-8, `main.rs:2464`) | replicado no boot do harness (no-op em ws fresco; medido no cenário reopen) |
| `Summary.delivery_profile` dizia "demo_profile" mesmo em modo produção | corrigido (string dinâmica) |
| Cenários novos: `dlq`, `reopen-check`, `handoff` + flags `--busy-count`/`--lina-bin` | adicionados p/ o roteiro do gate |
| **⚠ Sincronizador grid-quiet→roster-Idle do harness NÃO tem equivalente em produção** | ver ACHADO 1 — é a divergência que importa |

## ⚠ ACHADOS DE GATE (divergências — reportadas, não consertadas)

> **Atualização 2026-06-06 (Tarefa S — re-medição direcionada):** os achados 1 e 2 foram **CORRIGIDOS** na tree e re-medidos — ver §"RE-MEDIÇÃO DIRECIONADA" ao fim. Mantidos abaixo no texto original para a trilha de auditoria.

1. **[ALTO — gap de integração no app] ✅ CORRIGIDO (Dev 03, re-medido)** Produção não devolve o roster a `Idle`. O router marca o alvo `Busy` na entrega (`router.rs:1163`, reason `a2a_delivery`) e a retenção lê o ROSTER (`router.rs:1137`). Mas **nenhum call-site de produção** faz `sup.set_status(_, Idle)`: o medidor de turno do app (bridge.rs:2598-2615) emite Busy/Idle **só para o host de UI** (`bridge.on_event`), nunca para o roster; o `LifecycleEngine` do core não é instanciado pelo app (único uso: `close_previous_generation` em `main.rs:2464`); os testes do core simulam o retorno com `turn_done()` ("como o lifecycle real faria" — router.rs:1953-1957). **Consequência em produção real:** após a 1ª entrega A2A a um nó, toda mensagem seguinte ao mesmo alvo retém até o teto (30s) e cai na DLQ. **As taxas DEPOIS abaixo foram medidas com o sincronizador do harness fazendo o papel da peça que falta no app** (grid quieto por `idle_ms` → roster `Idle`, espelhando a intenção do EndDetector). O core está pronto; falta a fiação no app.
2. **[MÉDIO — calibração] ✅ CORRIGIDO (Dev 01, re-medido)** O teto de retenção default (30s) dead-letterou 1 retorno legítimo sob rajada. Na rodada 5 do cenário P1 (alvo saturado processando 3 turnos), a 20ª mensagem estourou a retenção → `MessageDeadLettered{reason:"retencao estourou o teto (30000 ms)…"}` (seq 188) + carta em `dead-letter/` + alerta ☠ no stderr. **Nada se perdeu em silêncio — é o comportamento do ADR 0020** — mas o default pode ser apertado para turnos reais de rajada; decidir calibração (`retention_timeout_ms`) com o time.
3. **[BAIXO — medição] A regra de "preso" do ANTES tinha 3 cegueiras no DEPOIS**, todas corrigidas no harness com evidência de bytes (recalibração prevista na ressalva 3 da baseline): (i) o ECO `❯ [LINA::MSG]` de mensagem JÁ submetida lia como "presa" durante o turno; (ii) o prompt vivo da TUI usa `❯`+**NBSP (U+00A0)** — sem normalizar, o prompt vivo nunca casava e a região caía no eco; (iii) o texto preso REAL do caminho legado colapsa em **chip** ("paste again to expand") com o marcador INVISÍVEL — detecção por marcador é cega; a assinatura do chip entrou na regra (conservadora: conta contra o DEPOIS). **Anti-vácuo provado:** corrida-controle com `--legacy-demo-delivery` detectou o chip (late_clear/1ª tentativa) + teste unitário `empty_prompt_row_discriminates_echo_and_stuck` com os bytes reais.
4. **[BAIXO] Regra do DEPOIS:** veredito por tentativa amostrado **após o turno aquietar** (prompt vivo vazio, sem "esc to interrupt", fila consumida ≤20s) — equivalente ao protocolo do ANTES (o preso nunca aquieta: timeout 90s → amostra acha chip/marcador). `--busy-count 300` no DEPOIS (vs 2500 do ANTES): com a entrega agora ESPERANDO prontidão, busy infinito estouraria o orçamento de retry por design — a condição do bug (alvo ocupado NO INSTANTE do envio) foi garantida em 20/20 (`busy_no_envio=true`).

## Taxas lado a lado — ANTES × DEPOIS (mesmo roteiro: 5 claudes reais, 20 tentativas, 5 rodadas)

| Métrica | ANTES (baseline 12:53) | DEPOIS (gate, ws `main`) | Critério |
|---|---|---|---|
| **Texto-colado (P0)** | **18/20 presos (90%)**, alvo ocupado 20/20 | **0/20 presos** (20 submetidos limpos), alvo ocupado 20/20 | (a) **PASS** |
| **Atropelamento (P1)** — pares <2s ao mesmo alvo | **15/15** (Δ min 155 ms, med 160 ms) | **0/14** (Δ min **3158 ms**, med **9539 ms**) | (b) **PASS** |
| Entregas da rajada | 20/20 | 19/20 entregues + **1 na DLQ com motivo** (retenção 30s estourada — ACHADO 2; nada sumiu) | (b) nota |
| `MessageRetained{target_busy}` | n/a (sem retenção) | **17** (serialização visível no log) | (b) prova |
| `RouteBlocked` | 0 (40 rotas) | **0** (39 rotas legítimas; ws `dlq` tem 100 **por design** — livro-razão do retry transiente de alvos órfãos, ADR 0003) | (c) **PASS** |
| `MessageDeliveryFailed` / `CircuitOpened` | n/a | 0 / 0 | — |

## Cenários novos da onda

- **(d) DLQ visível — PASS** (ws `dlq`, 2 modos): (i) alvo inexistente `@Fantasma` → `MessageDeadLettered{reason:"destino indisponivel (NoTarget) apos 50 re-tentativas"}`; (ii) **destino real derrubado** (`Supervisor::mark_dead` — caminho canônico; resolução passa a ignorá-lo) → mesma DLQ com motivo. Ambos: alerta `☠ DLQ ← <id> … retry MANUAL` no stderr + projeção `dead-letter/` com as 2 cartas (`Mailbox::dead_letters()`). Somado ao caso de retenção da rajada (ACHADO 2): 3 caminhos à DLQ exercitados ao vivo, 0 mensagens sumidas em silêncio.
- **(e) Replay sem nós-fantasma pós `kill -9` — PASS** (ws `reopen`): 2 gerações de runner mortas com **`kill -9` real** no meio da corrida (PIDs 36893/37059); cada boot seguinte fechou a geração anterior (réplica de `main.rs:2464`); corrida final: pré-close 2 fantasmas → pós-close **0** → spawna geração nova → **replay do log == roster vivo (2 nós, IDs idênticos)** e aritmética por evento `NodeAdded(6) − mortes(4) = 2 vivos ✓` (`reopen.json`).
- **(f) Telemetria de adoção do plano — APURADA (métrica, não critério):** `plan_adoption` sobre o log do teste: `{plan_claims: 0, plan_checks: 0, ask_chains: 39, rate: 0,0%}`. Esperado para este roteiro: o harness coordena por enqueue direto (probes), não por plano — o número de referência segue a baseline de 14h (0%) e passa a ser acompanhado nos workspaces reais.

## Golden transcript — `lina handoff --context` (F1-0-6 critério 1) — ✓ TRANSCRIPT OK

Corrida ws `handoff` (2 claudes reais; identidade `.lina/bootstrap.json` por agente + allow `Bash(lina:*)`; `LINA_HOME` apontado ao ws): `lina handoff "@W1" "<tarefa>" --context plano.md` rodado do cwd do Maestro →
1. CLI exit 0: `ok: <NodeId do W1> recebeu a mensagem (id msg_…8aeb)`.
2. Log: `MessageRouted{intent=handoff}` (seq 7) → `MessageDelivered{to=W1}` (seq 9) — contrato `lina/msg@2` validado pelo router.
3. **Grid do W1**: bloco `[LINA::MSG]` com o payload + `--- contexto anexado (plano.md) ---` (marcador `PLANO-CTX` visível) + **contrato legível** (`contract.input_schema/output_schema/error_codes/timeout_sec/retry_policy`, `constraint.autonomy`) + `[EXPECTED]`.
4. O W1 (claude real) executou `lina ask "@Maestro" "HANDOFF-OK-… plano recebido" --reply-to msg_…8aeb` e narrou `PRONTO: handoff F1-0 validado…` em pt-br, sem vazar o bloco técnico.
5. Log: reply roteado **herdando a cadeia** (`intent=ask`, `hops=1`, `root_cause_id = id do handoff`) → `MessageDelivered{to=Maestro}` → marcador `HANDOFF-OK` visível no grid do Maestro.
Transcript integral (grids + eventos + comando): `/tmp/lina-gate-f1-0/handoff/baseline-artifacts/handoff-transcript.md`.

## Veredito por critério do GATE F1-0 (metade DEPOIS)

| Critério | Veredito | Evidência |
|---|---|---|
| (a) texto-colado 0/N nas mesmas N tentativas | ✅ **PASS** (0/20; antes 18/20) | `main/baseline-artifacts/stuck.json` + grids |
| (b) 0 injeção concorrente, serialização por estado | ✅ **PASS** (0/14 <2s; Δmin 3158ms; 17 retenções) | `main/baseline-artifacts/collision.json` + log |
| (c) RouteBlocked ≈ 0 mantido | ✅ **PASS** (0 em 39 rotas legítimas) | `main/.lina/events/log.jsonl` |
| (d) DLQ com motivo + alerta visível | ✅ **PASS** (3 caminhos: órfão, derrubado, retenção) | `dlq/baseline-artifacts/dlq.json` + ☠ stderr |
| (e) replay = roster vivo (0 fantasmas) | ✅ **PASS** (2× kill -9; 6−4=2; IDs idênticos) | `reopen/baseline-artifacts/reopen.json` |
| (f) telemetria plan-vs-ask reportada | ✅ **APURADA** (0,0% — métrica acompanhada) | `plan_adoption --json` |
| F1-0-6 crit. 1 — golden transcript handoff | ✅ **PASS** (ciclo completo com reply no formato) | `handoff/baseline-artifacts/handoff-transcript.md` |

**GATE DEPOIS: PASS COM RESSALVA** — as taxas foram medidas com o harness suprindo a peça de roster-Idle que **faltava no app de produção** (ACHADO 1, ALTO). Recomendação ao Maestro: fiar o retorno a `Idle` no roster do app (EndDetector/meter → `set_status`) **antes de declarar o gate da onda fechado em produção**, e decidir a calibração do `retention_timeout_ms` (ACHADO 2). Reproduzir: comandos das 5 fases registrados nos stderr `/tmp/gate-{main,dlq,reopen-*,handoff*}.err`.

> **⤷ Ressalva RESOLVIDA na re-medição abaixo (Tarefa S):** com os 2 fixes na tree, a ressalva do PASS cai — ver §"RE-MEDIÇÃO DIRECIONADA".

---

# RE-MEDIÇÃO DIRECIONADA (Tarefa S) — achados 1 e 2 CORRIGIDOS · 2026-06-06

> **Escopo (ordem do Maestro):** re-medir APENAS (a) a rajada do critério (b) com o teto novo e (b) a paridade da réplica com a fiação real do app. Demais critérios: já PASS, não re-medidos. **Fixes medidos (não-commitados na tree):** fiação roster-Idle no app (Dev 03) + `RETENTION_TIMEOUT_MS` 30s→600s com trava anti-drift (Dev 01). Workspace isolado `/tmp/lina-gate-f1-0/remed`; mesmo timing-base; exit codes diretos (todos 0).

## (b) Paridade da réplica × fiação real do app — CONFIRMADA (com 2 deltas residuais aceitos)

**O caminho real do app (lido no diff do Dev 03):** `spawn_pump` (bridge.rs) agora recebe `Arc<Supervisor>` (main.rs:2649+ passa na construção) e instancia `LifecycleEngine`; no fim-de-turno do medidor (`CostMeter::poll_finished_turns` — silêncio de output por `idle_ms`), chama **`lifecycle.on_end_of_response(&sup, &mut store, node)`** → transição event-sourced para `Idle` no SUPERVISOR (`NodeStatusChanged{reason:"end_of_response"}` no log + roster) — exatamente a peça que o ACHADO 1 apontou ausente. **Teste guardião do app observado verde por este medidor:** `bridge::tests::end_of_response_returns_node_to_idle_and_unblocks_second_delivery ... ok` (exercita o `spawn_pump` real; exit 0). **Trava do teto observada verde:** `router::tests::retention_default_e_da_ordem_de_minutos ... ok` (default ≥600s e ≤1h; ADR 0020 §Calibração).

**Réplica atualizada para a MESMA API:** o pump do harness agora chama `on_end_of_response` (antes: `transition(Idle, "idle_grid")` — reason divergente). Prova no log da re-medição: **19 `NodeStatusChanged{end_of_response}`** (o reason de produção). Deltas residuais, reportados e aceitos como equivalentes:
1. **Sinal de fim-de-turno:** app = silêncio do `CostMeter` sobre `GridDelta.bytes` por `idle_ms`; harness = hash do grid estável por `idle_ms`. Mesmo contrato ("sem output novo pela janela do perfil"); fontes de medição distintas.
2. **Seleção de nós:** app usa o active-set do medidor (só nós que produziram output no turno); harness usa guard `status == Busy`. Equivalentes para o caminho da retenção (só alvos Busy retêm).

## (a) Rajada re-medida com o teto novo (600s) — 20/20 ENTREGUES, 0 DLQ ✅

Mesmo roteiro do cenário de colisão (5 claudes reais, 5 rodadas, fan-out simultâneo ao Maestro), ws `remed`:

| Métrica | DEPOIS original (teto 30s) | RE-MEDIÇÃO (teto 600s + fiação app) | Esperado |
|---|---|---|---|
| `MessageDelivered` (log, fonte da verdade) | 19/20 | **20/20** | 20/20 ✅ |
| `MessageDeadLettered` por retenção | **1** (seq 188, 30s estourados) | **0** | 0 ✅ |
| Pares <2s ao mesmo alvo | 0/14 | **0/14** (Δmin 3203ms, med 7495ms) | 0 ✅ |
| `MessageRetained{target_busy}` | 17 | 18 (serialização intacta) | >0 ✅ |
| `RouteBlocked` / `MessageDeliveryFailed` | 0 / 0 | **0 / 0** | ≈0 ✅ |
| `NodeStatusChanged{end_of_response}` | n/a (reason antigo `idle_grid`) | **19** (reason do caminho real do app) | >0 ✅ |

*Nota de estatística do coletor:* o `collision.json` registrou 19 das 20 entregas na janela por-rodada (a 4ª entrega da rodada 1 saiu da retenção durante a janela de coleta da rodada 2 — **entregue no log**, fora da estatística de pares). A fonte da verdade (`log.jsonl`) é inequívoca: 20 roteadas → 20 entregues → 0 DLQ.

## Veredito da re-medição

| Item | Veredito |
|---|---|
| ACHADO 1 (roster-Idle no app) | ✅ **CORRIGIDO** — fiação real lida e comparada; teste guardião do app verde (observado); réplica usa a mesma API (`on_end_of_response`); paridade confirmada com 2 deltas residuais equivalentes |
| ACHADO 2 (teto de retenção 30s) | ✅ **CORRIGIDO** — default 600s + trava anti-drift [600s, 1h] verde; rajada re-medida: **20/20 entregues, 0 DLQ por retenção**, serialização intacta (0 pares <2s) |
| Ressalva do "PASS COM RESSALVA" | **CAI** — com os 2 fixes, o gate F1-0 (metade DEPOIS) é **PASS** sem ressalva de produção. Os fixes foram commitados durante a medição (`4321e04` teto + `1b6f573` fiação) — verificado no disco ao fechar: teto 600s e `on_end_of_response` presentes em HEAD |
