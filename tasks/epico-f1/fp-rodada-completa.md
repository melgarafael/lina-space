# Protocolo COMPLETO de FP — F1-1-6 (rodada R2) · REGISTRO DURÁVEL

Analista de Gates · sessão Lina Space · 2026-06-07 · **CONCLUÍDA**

> Versão durável de `crates/lina-core/.entrega-fpfull.md` (working copy gitignored). Os logs originais
> vivem em `/tmp` e **evaporam em reboot** — os trechos-chave estão transcritos na §Evidência ao final.

## Objetivo (pré-requisito do gate F1-1)

Protocolo completo: **10 rodadas × 15 min × 5 terminais PTY reais** (build-ansi, verbose-log, trap-busy, trap-idle, real-asker) com o harness commitado `src/bin/permission_fp.rs` (commit `99f6432`), medindo contra o **THRESHOLD ACORDADO (Maestro, 2026-06-07)**:

1. **Recall = 100%** dos pedidos reais roteirizados;
2. **FP ≤ 1,0/hora-terminal** nos NÃO-adversariais (build-ansi, verbose-log, trap-busy);
3. **Precisão ≥ 0,9 excluindo o trap deliberado** (trap-idle); precisão COM adversarial reportada só como observabilidade.

## Proveniência do binário (anomalia de partida — registrada)

- `cargo build -p lina-core --bin permission_fp` **falhou (exit 101)**: `events.rs:749` match não-exaustivo — variantes novas (`PermissionResolved`, `ApprovalInjected`, `ApprovalAborted`, `ApprovalDuplicateIgnored`, `PermissionDismissed`...) de **edição mid-flight de peer** (` M events.rs`, F1-1-7/8). Não é o harness; não toquei (fronteira: nenhum `.rs`). Evidência: `/tmp/fpfull-build.log` (transcrito na §Evidência).
- Medição roda com **binário congelado**: `target/debug/permission_fp` (mtime 12:08:25) compilado APÓS os fontes do harness/detector (12:02–12:03) e ANTES da edição do peer (19:26) ⇒ reflete o estado commitado da F1-1-6 (`99f6432`). Copiado para `/tmp/permission_fp-r2` — imune a rebuilds/`cargo clean` de peers durante as ~2,5h.
- **Smoke 1×45s: exit 0** (`/tmp/fpfull-smoke.log`): 2 TP real-asker, 2 FP trap-idle (1/isca, estrutural), 0 FP benignos, 209 suprimidos por busy, replay sem duplicar, stable_id 4/4 únicos. Harness validado antes de comprometer wall-clock.

## Aritmética do ground truth (derivada do script + confirmada no smoke)

- `real-asker`: ciclo ≈ 26s (read -t 18 + sleep 8) ⇒ **~34-35 pedidos reais/rodada de 900s**. Recall = TP/rodada estável nessa cadência.
- `trap-idle`: isca a cada ~25s ⇒ ~35-36 candidatos adversariais/rodada (FP 1-por-isca é o custo estrutural documentado da #28174).
- Limitação NOMEADA: ground truth é derivado da cadência roteirizada (mesmo método da rodada reduzida), não de contador independente dentro do script. Estabilidade inter-rodada é o cheque.

## Plano de blocos

| bloco | rodadas | duração | log | status |
|---|---|---|---|---|
| 1 | 3 × 900s | ~45 min | /tmp/fpfull-block1.log | ✅ exit 0, lido |
| 2 | 3 × 900s | ~45 min | /tmp/fpfull-block2.log | ✅ exit 0, lido |
| 3 | 2 × 900s | ~30 min | /tmp/fpfull-block3.log | ✅ exit 0, lido |
| 4 | 2 × 900s | ~30 min | /tmp/fpfull-block4.log | ✅ exit 0, lido |
| 11 (extra) | 1 × 900s · binário HEAD `986b6e8` | ~15 min | /tmp/fpfull-round11-head.log | ✅ exit 0, lido — SEM DRIFT |

Total: 10 rodadas × 15 min = 12,5 horas-terminal (5 terminais). Veredito só após ler TODOS os logs renderizados.

## Progresso (linha do tempo)

- [19:27] Início. Build quebrado por peer detectado e contornado (binário congelado). Smoke verde.
- [19:29] Bloco 1 disparado (3 × 900s).
- [20:15] Bloco 1 concluído (exit 0) e lido. Bloco 2 disparado.
- [20:2x] ANOMALIA da rodada longa identificada, mecanismo re-derivado no código e quantificado via event log (ver §Anomalia).
- [21:02] Bloco 2 concluído (exit 0) e lido — rodadas 4-6 idênticas às 1-3 (determinismo do harness). Event log re-verificado independentemente do stdout. Bloco 3 disparado.
- [21:33] Bloco 3 concluído (exit 0) e lido — rodadas 7-8 idênticas. Event log verificado (35,0 asks/rodada, 0 benignos). Bloco 4 (final) disparado.
- [22:04] Bloco 4 concluído (exit 0) e lido — rodadas 9-10 idênticas. Protocolo de 10 rodadas COMPLETO. Verificação final: exit codes 4/4 re-lidos, JSONL de todos os blocos conferido, pegada na árvore = só marcadores + entrega (gitignored por design).
- [22:0x] Detectado que o HEAD avançou 5 commits durante a medição (detector +857 linhas). Build no HEAD verde → rodada 11 de não-drift disparada com binário `986b6e8`.
- [22:3x] Rodada 11 concluída (exit 0) e lida — **SEM DRIFT**: 60/35/36/0 exatos, idênticos às rodadas 1-10 (ver §Rodada 11).
- [22:5x] Decisão de rotulagem do Maestro registrada (ver §Decisão de rotulagem). Entrega marcada CONCLUÍDA; este registro durável escrito.

## Resultados por rodada

Colunas: TP = emissões no real-asker (rótulo do protocolo) · asks = pedidos roteirizados distintos (agrupamento por gap>12s no event log; cadência ≈26s) · FP-ben = FP em build-ansi+verbose-log+trap-busy · FP-trap = FP no trap-idle (adversarial deliberado) · prec-excl = precisão excluindo trap-idle (rótulo do protocolo) · prec-c/adv = precisão com adversarial (observabilidade).

| rodada | TP | asks | recall | FP-ben | FP-ben/h-t | FP-trap | prec-excl | prec-c/adv | supr. busy |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4105 |
| 2 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4101 |
| 3 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4081 |
| 4 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4054 |
| 5 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4105 |
| 6 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4120 |
| 7 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4087 |
| 8 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4088 |
| 9 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4069 |
| 10 | 60 | 35 | 35/35 = 100% | 0 | 0,0 | 36 | 1,000 | 0,625 | 4069 |
| **11 (HEAD)** | **60** | **35** | **35/35 = 100%** | **0** | **0,0** | **36** | **1,000** | **0,625** | **4097** |

Sanidade bloco 1: replay 288→288 sem duplicar · stable_id 288/288 únicos · exit 0.
Sanidade bloco 2: replay 288→288 sem duplicar · stable_id 288/288 únicos · exit 0 · event log re-verificado (35,0 asks/rodada por gap>12s; razão 1,71; alternância 18s/8s; 0 emissões benignas no log JSONL — checagem independente do stdout).
Sanidade bloco 3: replay 192→192 sem duplicar · stable_id 192/192 únicos · exit 0 · event log re-verificado (35,0 asks/rodada; razão 1,71; 0 benignos no JSONL).
Sanidade bloco 4: replay 192→192 sem duplicar · stable_id 192/192 únicos · exit 0 · event log re-verificado (35,0 asks/rodada; razão 1,71; 0 benignos no JSONL).
Sanidade rodada 11: replay 96→96 sem duplicar · stable_id 96/96 únicos · exit 0 · event log re-verificado (35 asks por gap>12s; 60 PermissionAsked no real-asker + 36 no trap-idle = 96; 0 benignos).
Exit codes dos 4 blocos re-lidos das saídas reais: 0/0/0/0; rodada 11: 0. JSONL do bloco 1 re-conferido: só trap-idle (108) + real-asker (180) — 0 benignos em TODOS os blocos por checagem independente do stdout.

## Anomalia (nova vs rodada reduzida) — re-emissão de prompt OBSOLETO sob scroll

**Sintoma:** TP/rodada = 60, mas pedidos roteirizados = 35 (cadência 26s = read 18s + sleep 8s). Razão 1,71 emissões/pedido. Inter-arrival no event log: primeiras ~10 emissões a 26s (1/pedido — regime da reduzida); depois alternância **18s/8s** = 2 emissões por ciclo.

**Mecanismo (re-derivado no código, não inferido):** após o `read` expirar (T+18s), a linha `Continue? (y/n)` deixa de estar pendente mas continua sendo o match mais-ao-fundo da janela de 6 linhas (`scan_grid`, permission_detect.rs:322 — as linhas pós-timeout não casam a regex). Com o **viewport cheio (30 rows)**, o scroll muda a `row` da linha → o dedup `(line_hash,row)` re-arma (permission_detect.rs:255) → segunda emissão a T+19,5s, para pedido JÁ expirado. A reduzida (180s ≈ 28 rows impressas) nunca atingia o regime de scroll — por isso 7/7 exatos lá.

**Por que o trap-idle não dobra:** cada ciclo imprime isca NOVA mais abaixo; a mais nova sombreia a antiga (candidato único = match mais próximo do fundo). FP-trap = 36 = 900/25 = exatamente 1 por aparição da isca, igual à reduzida.

**Consequências honestas:**
- Pela rotulagem DO PROTOCOLO (emissão do real-asker = TP), recall/FP/precisão passam como na tabela. Pelo threshold acordado como escrito: **nada muda**.
- Sob rotulagem mais estrita (emissão p/ pedido já expirado = FP): ~25 re-emissões obsoletas/rodada ⇒ precisão-excl-adversarial cairia para 35/60 ≈ **0,583**.
- **Implicação de produto (F1-1-7):** o toast dispararia 2× por pedido sob scroll, e o dedup por `stable_id` NÃO funde as re-emissões (ts entra no hash — permission_detect.rs:286). Atenção: o re-arm por row foi fix deliberado da calibração (pedidos repetidos do mesmo texto precisam re-emitir); qualquer mitigação (ex.: janela temporal por line_hash, ou exigir cursor adjacente à linha-match) precisa preservar esse recall.

## Agregado final (10 rodadas × 900s × 5 terminais = 12,5 horas-terminal)

| métrica | valor |
|---|---|
| emissões totais | 960 (TP-protocolo 600 · FP 360, todos no trap-idle) |
| pedidos reais roteirizados (event log, gap>12s; = aritmética da cadência 26s) | 350 (35/rodada, exato em 10/10) |
| **recall** | **350/350 = 100%** |
| **FP nos NÃO-adversariais** (build-ansi + verbose-log + trap-busy, 7,5 h-t) | **0 ⇒ 0,00/h-t** |
| **precisão excl. trap deliberado** (rótulo do protocolo) | **600/600 = 1,000** |
| precisão COM adversarial (observabilidade) | 0,625 · FP/h-t mix completo: 28,80 |
| FP trap-idle | 360 = 36/rodada = exatamente 1 por aparição da isca (900/25) — custo estrutural #28174, idêntico à reduzida |
| candidatos suprimidos por busy | ~4,1k/rodada (~41k no total) |
| sanidade | replay sem duplicar e stable_id 100% únicos nos 4 blocos · exit 0 nos 4 |
| rotulagem ESTRITA (re-emissão p/ pedido expirado = FP) | precisão-excl cairia a 350/600 = 0,583 · 250 re-emissões obsoletas (25/rodada) — ver §Anomalia |

## Rodada 11 (não-drift no HEAD `986b6e8`) — RESULTADO

Contexto: durante as ~2,5h de protocolo o HEAD avançou 5 commits (F1-1-7/8, ADR 0021, R2b choice); `permission_detect.rs` ganhou +857/-19 linhas (camada choice + `ArmedPrompt`/`ClearedPrompt`); o harness `permission_fp.rs` INALTERADO. Mitigação: rodada extra 1×900s com binário compilado no HEAD (`/tmp/permission_fp-head`, build exit 0). O protocolo é determinístico (60/35/36/0) — qualquer divergência comportamental aparece nos números.

Exit 0 (`ROUND11_EXIT=0`). Números contra o esperado determinístico:

| métrica | esperado | medido | veredito |
|---|---|---|---|
| TP real-asker | 60 | 60 | ✓ |
| asks (event log, gap>12s — verificação independente do stdout) | 35 | 35 | ✓ |
| FP trap-idle | 36 | 36 | ✓ |
| FP benignos (build-ansi + verbose-log + trap-busy + real-asker) | 0 | 0 | ✓ |

Corroboração: suprimidos_busy = 4097 (faixa 4054–4120 das rodadas 1-10) · precisão-c/adv 0,625 idêntica · replay 96→96 sem duplicar · stable_id 96/96 únicos · inter-arrival com o MESMO perfil da anomalia (primeiras ~10 emissões a ~26s, depois alternância 18s/8s; 34 gaps>12s, média 20,1s) ⇒ a re-emissão obsoleta sob scroll **persiste no HEAD**, agora medida (não só lida no diff).

**VEREDITO: SEM DRIFT.** O caminho y/n do detector no HEAD `986b6e8` é comportamentalmente idêntico ao `99f6432` medido nas 10 rodadas — o PASS do gate transfere para o HEAD.

## Decisão de rotulagem (Maestro, 2026-06-07 — registrada, não re-litigar)

- **Rotulagem oficial = a do protocolo/threshold COMO ESCRITO** (emissão do real-asker = TP) ⇒ **GATE pré-F1-1 FP: PASS**.
- A rotulagem estrita (precisão-excl 0,583 sob "emissão p/ pedido expirado = FP") fica **REPORTADA como observabilidade** — não altera o gate.
- A anomalia de re-emissão obsoleta vira **PENDÊNCIA DE PRODUTO da F1-1-7**: toast 2× por pedido sob scroll; qualquer mitigação futura (janela temporal por line_hash, cursor adjacente etc.) **deve preservar o recall de pedidos repetidos reais** (o re-arm por row foi fix deliberado da calibração).

## VEREDITO contra o threshold acordado (Maestro, 2026-06-07)

| # | critério | medido | veredito |
|---|---|---|---|
| 1 | Recall = 100% dos pedidos reais roteirizados | 350/350 = 100% (10/10 rodadas) | ✅ **PASS** |
| 2 | FP ≤ 1,0/hora-terminal nos NÃO-adversariais | 0,00/h-t (0 FP em 7,5 h-t) | ✅ **PASS** |
| 3 | Precisão ≥ 0,9 excluindo o trap deliberado | 1,000 | ✅ **PASS** |
| obs | Precisão com adversarial (só observabilidade) | 0,625 (estável; trap-idle = 1 FP/isca por construção) | reportado |
| extra | Não-drift no HEAD `986b6e8` (rodada 11) | 60/35/36/0 exatos | ✅ **SEM DRIFT** |

**GATE pré-F1-1 (FP): PASS** — pela rotulagem oficial (decisão do Maestro acima), com a anomalia de re-emissão obsoleta registrada como pendência de produto da F1-1-7 e a rotulagem estrita (0,583) reportada como observabilidade.

## O que foi medido vs o que NÃO foi (honestidade)

**Medido:** o fallback de GRID (camada 2) do detector no estado `99f6432` (e, via rodada 11, o caminho y/n no HEAD `986b6e8`), sob as 5 cargas roteirizadas do protocolo, em binário debug (mesmo modo da rodada reduzida — comparável), nesta máquina (macOS, sessão Maestri ativa em paralelo — carga de fundo realista).

**NÃO medido (nomeado):**
1. **Camada 1 (hook)** — o protocolo de FP exercita só o caminho de grid (5 bash, sem Claude/hooks). A latência e correlação do hook foram medidas no probe da F1-1-6 (4 rodadas), não aqui.
2. **CLIs reais como carga** — as cargas são bash roteirizado por design do protocolo (rótulo a priori); um Claude Code real produzindo grids ricos (caixas, spinners, choice-chrome) não está coberto por ESTA medição.
3. **Ground truth de recall** — derivado da cadência roteirizada (26,0-26,2s medidos no event log, 35 pedidos/rodada exatos) + agrupamento por gap; não há contador independente dentro do script. A exatidão 35/35 em 11/11 rodadas é a evidência de que nada foi perdido.
4. **A camada choice do HEAD** — a rodada 11 prova não-drift do caminho **y/n**; a camada choice nova (+857 linhas) não é exercitada pelas 5 cargas deste protocolo e terá medição própria (R2b).

## Evidência (transcrição dos logs de /tmp — evaporam em reboot)

### Exit codes

```
bloco 1: exit 0 · bloco 2: exit 0 · bloco 3: exit 0 · bloco 4: exit 0
rodada 11: ROUND11_EXIT=0
```

### /tmp/fpfull-build.log (build quebrado por peer — proveniência do binário congelado)

```
error[E0004]: non-exhaustive patterns ... events.rs:749
For more information about this error, try `rustc --explain E0004`.
error: could not compile `lina-core` (lib) due to 1 previous error
(exit 101 — variantes novas PermissionResolved/ApprovalInjected/ApprovalAborted/
 ApprovalDuplicateIgnored/PermissionDismissed de edição mid-flight de peer, F1-1-7/8)
```

### /tmp/fpfull-smoke.log (smoke 1×45s, exit 0)

```
precisão: 0.500
  FP build-ansi: 0
  FP verbose-log: 0
  FP trap-busy: 0
  FP trap-idle: 2
  FP real-asker: 0
replay: PermissionAsked no log 4 → após 2 projeções 4 (sem duplicar)
stable_id únicos: 4/4 OK
```

### /tmp/fpfull-block1.log (rodadas 1-3, exit 0)

```
— rodada 1: emitidos=96 TP=60 FP=36 suprimidos_busy=4105 precisão=0.625
— rodada 2: emitidos=96 TP=60 FP=36 suprimidos_busy=4101 precisão=0.625
— rodada 3: emitidos=96 TP=60 FP=36 suprimidos_busy=4081 precisão=0.625
hora-terminal: 3.750
emitidos: 288 · TP: 180 · FP: 108
FP/hora-terminal: 28.80
precisão: 0.625
  FP build-ansi: 0 · FP verbose-log: 0 · FP trap-busy: 0 · FP trap-idle: 108 · FP real-asker: 0
replay: PermissionAsked no log 288 → após 2 projeções 288 (sem duplicar)
stable_id únicos: 288/288 OK
```

### /tmp/fpfull-block2.log (rodadas 4-6, exit 0)

```
— rodada 1: emitidos=96 TP=60 FP=36 suprimidos_busy=4054 precisão=0.625
— rodada 2: emitidos=96 TP=60 FP=36 suprimidos_busy=4105 precisão=0.625
— rodada 3: emitidos=96 TP=60 FP=36 suprimidos_busy=4120 precisão=0.625
hora-terminal: 3.750
emitidos: 288 · TP: 180 · FP: 108
FP/hora-terminal: 28.80
precisão: 0.625
  FP build-ansi: 0 · FP verbose-log: 0 · FP trap-busy: 0 · FP trap-idle: 108 · FP real-asker: 0
replay: PermissionAsked no log 288 → após 2 projeções 288 (sem duplicar)
stable_id únicos: 288/288 OK
```

### /tmp/fpfull-block3.log (rodadas 7-8, exit 0)

```
— rodada 1: emitidos=96 TP=60 FP=36 suprimidos_busy=4087 precisão=0.625
— rodada 2: emitidos=96 TP=60 FP=36 suprimidos_busy=4088 precisão=0.625
hora-terminal: 2.500
emitidos: 192 · TP: 120 · FP: 72
FP/hora-terminal: 28.80
precisão: 0.625
  FP build-ansi: 0 · FP verbose-log: 0 · FP trap-busy: 0 · FP trap-idle: 72 · FP real-asker: 0
replay: PermissionAsked no log 192 → após 2 projeções 192 (sem duplicar)
stable_id únicos: 192/192 OK
```

### /tmp/fpfull-block4.log (rodadas 9-10, exit 0)

```
— rodada 1: emitidos=96 TP=60 FP=36 suprimidos_busy=4069 precisão=0.625
— rodada 2: emitidos=96 TP=60 FP=36 suprimidos_busy=4069 precisão=0.625
hora-terminal: 2.500
emitidos: 192 · TP: 120 · FP: 72
FP/hora-terminal: 28.80
precisão: 0.625
  FP build-ansi: 0 · FP verbose-log: 0 · FP trap-busy: 0 · FP trap-idle: 72 · FP real-asker: 0
replay: PermissionAsked no log 192 → após 2 projeções 192 (sem duplicar)
stable_id únicos: 192/192 OK
```

### /tmp/fpfull-build-head.log (build no HEAD `986b6e8`, exit 0)

```
   Compiling lina-core v0.0.1 (.../lina-space/crates/lina-core)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.99s
```

### /tmp/fpfull-round11-head.log (rodada 11, binário HEAD, exit 0)

```
== F1-1-6 · protocolo de FP (fallback de grid) ==
rodadas=1 × 900s · terminais=5 · sample=250ms · idle=1500ms
event store: .../T/lina-fp-f116-019ea4c1-b613-7190-a439-6b92cd975bfc

— rodada 1: emitidos=96 TP=60 FP=36 suprimidos_busy=4097 precisão=0.625
  FP em trap-idle: 36

== AGREGADO ==
hora-terminal: 1.250
emitidos: 96 · TP: 60 · FP: 36
FP/hora-terminal: 28.80
precisão: 0.625
  FP build-ansi: 0
  FP verbose-log: 0
  FP trap-busy: 0
  FP trap-idle: 36
  FP real-asker: 0
replay: PermissionAsked no log 96 → após 2 projeções 96 (sem duplicar)
stable_id únicos: 96/96 OK
```

### Event log da rodada 11 (verificação independente do stdout — log.jsonl do event store)

```
tipos de evento: {'PermissionAsked': 96}
real-asker: 60 emissões · trap-idle: 36 emissões · benignos: 0
real-asker: asks (gap>12s) = 35
primeiros 12 gaps (s): 26.0, 26.1, 25.9, 26.1, 25.9, 26.2, 26.1, 26.0, 25.8, 18.1, 8.1, 17.9
últimos 12 gaps (s):   17.9, 8.0, 17.9, 8.1, 18.0, 7.9, 18.2, 8.0, 17.9, 8.2, 17.9, 8.0
(mesmo perfil das rodadas 1-10: ~26s no início, depois alternância 18s/8s = anomalia de
 re-emissão sob scroll, presente também no HEAD)
```
