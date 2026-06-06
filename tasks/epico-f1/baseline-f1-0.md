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

PRONTO desta story (resumo na resposta ao Maestro). Artefatos versionados: `tools/lina_watch.py`, `crates/lina-core/src/bin/baseline_f1_0.rs` (+ 6 testes), este `baseline-f1-0.md`. Dados crus da corrida: `<ws>/baseline-artifacts/` (não versionados; em `/tmp`).
