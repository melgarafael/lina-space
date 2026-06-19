# Onda F3-CONF-3 — "O Maestro Enxerga o Time" (Observabilidade & Adoção da Orquestração) · plano de execução

> **Rodada de CONSOLIDAÇÃO** entre F3-2/F3-CONF-2 (fechadas) e F3-3 Mentality (próxima onda nova). Ataca o **top-risco da fase** (épico `39` §VI-3: *adoção 0% — Goal/effort/Tradutor construídos e nunca acionados*) fechando os achados de dogfooding ALTA/MEDIA-ALTA que tornam a inteligência da F3 **vaporware na prática**: o Maestro não consegue VER o worker (#27 quebra `lina history` no caminho real do spawn), a observabilidade MENTE (#23c/#14 `list` vs `check` divergem → re-despacha quem trabalha), e o contexto TRAVA (#21 `lina vault search` pendura 50 min).
> **Por que agora (e não a Mentality):** a F3-3 (Mentality) tem como alicerce 6 eventos novos em `events.rs` — arquivo que o **Terminal A está editando agora** (ADR 0037 `NodeCwdSet`, não commitado, + `bridge.rs`/`main.rs`/`agent_modal.rs`). Lançar Mentality sobre uma árvore compartilhada com `events.rs` sujo = risco de regressão silenciosa (lição `working-tree-compartilhado-reverte-edicao`). Esta rodada tem **território 100% disjunto do A** e prepara o terreno: a Mentality precisa de observabilidade confiável (o detector de correção + a métrica de adoção da sentinela, §Risco-4 da spec 35). Mentality vira a rodada seguinte, com `events.rs` limpo.
> **Fonte da verdade:** `tasks/despachos/achados-dogfooding-sessao.md` (#27, #23c/#23b/#14, #21/#21c, #15, #26) · épico `39 - Epico Fase 3 — O Maestro Nativo` (vault) §I "orquestração como mecanismo", §VI-3 risco de adoção · ADR 0019 (definições de progresso/travamento) · ADR 0026 (identidade por env de spawn `n-<uuid>`) · ADR 0006 (fronteira de pertencimento) · ADR 0007 (carimbo server-side; regra-mãe).
> **Maestro:** Terminal B — Effort: Ultra Code. **Workers NÃO commitam; o Maestro fixa a fronteira, valida de fora (exit codes diretos — pipe mascara), cold-review isolado, e commita por fatia.**

## Por que esta rodada é "avanço na inteligência do Lina"

A inteligência de um orquestrador é **proporcional à sua observabilidade**. A F3-0/1/2 deu ao Maestro os mecanismos (Goal, loop interpretar→confirmar, Tradutor, effort por papel, anti-engolimento). Mas o **passo 6 do loop (monitorar) depende de o Maestro VER o worker** — e hoje ele é cego no caminho real:
- **#27:** a F3-2 entregou `lina history @colega` para "o Maestro ver a tela do worker"; mas o verbo **rejeita o `LINA_NODE_ID` que o spawn de fato carimba** (`n-<uuid>`) → o caminho oficial de observabilidade está mudo; o Maestro cai no fallback de ler o disco à mão.
- **#23c/#14:** `lina list` e `lina check` dão **leituras divergentes do mesmo nome** (list mostra homônimo de sessão morta; check já resolve o vivo) → o Maestro que confia no `list` re-despacha trabalho de um worker que está TRABALHANDO.
- **#21:** `lina vault search` **pendura sem timeout nem output** em vault grande (o cenário real do fundador) → o verbo de contexto do segundo cérebro fica inutilizável; o agente trava o turno ou fica cego.

Fechar isso é Meadows **[6] Fluxo de Informação** completando o circuito de volta ao orquestrador — sem o qual o termostato [8] (F3-1) e os objetivos [3] não fecham na vida real. **Mais a métrica de adoção** (§VI-3): medir uso real de `lina goal`/Tradutor/effort no log, para o próximo gate declarar com dado, não com suposição.

## Invariante que ATRAVESSA a rodada (não regredir)

**Observabilidade NÃO é autoridade.** Nenhum fix desta rodada pode deixar um campo escrito por agente decidir identidade, ordem ou autorização (família ADR 0007). Em particular: a resolução **vivo-vence-morto** é da **superfície de OBSERVABILIDADE** (`list`/`check`/`history`) — **NÃO** se toca `Supervisor::node_by_name` (lib.rs:1534), que resolve remetente/alvo do **router** e já ignora mortos (`is_alive()`). Tocar a autoridade do router = **escalar ao Maestro**, não decidir. Toda story que roça o router tem como critério implícito a **suíte de segurança do router verde por mutação**.

## DAG executável e frentes (fronteiras DISJUNTAS — dono único por arquivo)

```
[Setup Maestro (B): fixa a fronteira + cria a pasta de despachos + semeia o plano]
        │
        ├─► R1 · RESOLUÇÃO   (Terminal H)  crates/lina-bootstrap/src/bin/lina.rs ──┐  #27 reader_node_id n- · #23c list vivo-vence-morto · #21 vault timeout+índice+flush
        ├─► R2 · ADOÇÃO      (Terminal J)  crates/lina-core/src/bin/<novo>.rs ──────┤  bin auto-descoberto read-only: adoção da inteligência F3 (Goal/Tradutor/effort/sentinela)
        └─► R3 · QA/RED      (Terminal R)  crates/**/tests/ (NOVOS) ───────────────┘  repro RED→GREEN de #27/#23c/#21 + mutação + suíte de segurança do router VERDE
        │
        ▼
   Maestro (B): valida de fora (exit codes diretos) + cold-review isolado + gate de onda (4 lentes) + commit por fatia
```

| Frente | Terminal | Toca (DONO ÚNICO) | model·effort | Por que NÃO colide |
|---|---|---|---|---|
| **R1 · RESOLUÇÃO** | **H** | `crates/lina-bootstrap/src/bin/lina.rs` SOMENTE | opus · **high** | H já trabalhou neste bin (W2: report honesto + `resolve_check_node` anti-nó-morto). Único dono do `lina.rs` na rodada. NÃO toca `lib.rs`/router (autoridade). Disjunto do A. |
| **R2 · ADOÇÃO** | **J** | `crates/lina-core/src/bin/intelligence_adoption.rs` (**NOVO**) | opus · medium | Bin auto-descoberto (`src/bin/`) = zero edição em `lib.rs`/`Cargo.toml`/módulos com dono (padrão `plan_adoption.rs`). Read-only do log. Disjunto de tudo. |
| **R3 · QA/RED** | **R** | `crates/**/tests/` (arquivos **NOVOS**) + `#[cfg(test)]` read-only | opus · **high** | Lê produção, escreve só testes novos. QA consistente do time (CONF-QA/F2QA). Disjunto do A e dos workers. |

**Reserva (escalada por breaker sticky 2×):** I, G, K.
**Terminal A:** OFF-LIMITS — está no ADR 0037 (`events.rs`, `bridge.rs`, `main.rs`, `agent_modal.rs`). Nenhuma frente toca esses arquivos.

## Setup do Maestro (B) — ANTES do fan-out

1. **Fronteira fixada (este doc).** Cada frente tem 1 arquivo/área DONO ÚNICO; zero sobreposição entre frentes e zero interseção com os 5 arquivos do A.
2. **Contrato de segurança (a linha que ninguém cruza):** a resolução vivo-vence-morto vive SÓ na superfície de observabilidade (`lina.rs`). `node_by_name` (lib.rs) e o router **não são tocados**. Se R1 descobrir que `lina list` lê um estado que SÓ pode ser consertado em `lib.rs`/supervisor → **BLOCKED + escala ao Maestro** (eu decido o contrato; possível ADR curto).
3. **Sem `events.rs`/`bridge.rs`/`agent_modal.rs`/`main.rs`** em NENHUMA frente (território do A).

## GATE DE SAÍDA F3-CONF-3 — RODA e se MEDE

- **(a) #27 — `lina history` no caminho real:** com `LINA_NODE_ID=n-<uuid>` no env (o formato que `bridge.rs:2760`/`admit_node` carimba), `lina history "@colega" --tail N` **resolve e imprime** (não mais `LINA_NODE_ID invalido`). Teste de paridade: o parser do leitor aceita exatamente o formato que o spawn produz (`n-{uuidv7}`) E o uuid puro. Repro RED no HEAD → GREEN com o fix.
- **(b) #23c/#14 — `list` para de mentir:** com um nome reusado entre sessões (nó morto + nó vivo homônimo), `lina list` mostra o **VIVO** (mesma semântica de `resolve_check_node`), batendo com `lina check`. Teste por contraste: log com homônimo morto+vivo → `list` e `check` concordam no `NodeId` vivo.
- **(c) #21 — `vault search` deixa de pendurar:** em vault grande (simulado com N arquivos), `lina vault search` (i) faz **flush incremental** (resultados saem em streaming, não no fim), (ii) respeita um **teto de tempo/arquivos** com saída parcial honesta ("parei em Xs — refine ou use `lina vault index`"), (iii) nunca fica >Ts sem 1 byte. Medido com um diretório de teste grande: o comando RETORNA em tempo limitado.
- **(d) Adoção (R2):** um bin read-only deriva do `log.jsonl` quantos `GoalDefined`/`GoalConfirmed`/`InterpretationProposed`/`EffortAssigned`/`[LINA::CORRECTION]` ocorreram (uso REAL da inteligência F3), agrupando por `root_cause_id` onde fizer sentido. Saída honesta ("baseline: N" — métrica acompanhada, NUNCA gate, igual `plan_adoption`). Replay reproduz idêntico.
- **(e) Segurança:** suíte do router verde **por mutação**; `node_by_name`/autoridade do router **intactos** (a resolução vivo-vence-morto da observabilidade não vazou para o roteamento); nenhum campo de agente decide identidade. 0 ALTA.
- **(f) Higiene:** `cargo fmt --all --check` 0; `clippy --all-targets -D warnings` 0 (workspace + app); a suíte do `lina-core` não pisca falso-vermelho (reusar o tmpdir-por-thread do CONF-HARNESS se algum teste novo criar tmpdir).
- **(g) [DIFERIDO ao fundador]** validação na tela — esta rodada é toda de bin/verbo/projeção (sem UI nova); o efeito é observável por CLI/teste, mas o fundador pode validar `lina history`/`lina list`/`lina vault search` ao vivo após o rebuild.

## Conselho de gate de onda (4 lentes, read-only)

(1) Visão/fios: fecha o passo 6 do loop (monitorar) sem virar harness (inv #1) — só leitura/projeção. (2) Arquitetura: resolução de observabilidade ≠ resolução de autoridade; bin de adoção auto-descoberto (zero costura); reuso da semântica de morte já provada (`resolve_check_node`). (3) Segurança (red-team, **0 ALTA p/ seguir**): `node_by_name`/router intactos; `n-` parse não vira bypass de identidade (continua exigindo o env do app, ADR 0026); leitura cross só por pertencimento (ADR 0006). (4) Specs: spot-check dos achados #27/#23c/#21 vs implementado; nada "adaptado em silêncio".

## Pendências tratadas / deferidas

- **Embutidas:** #27 (history n-), #23c/#14 (list vivo-vence-morto), #21/#21c (vault search timeout/índice/flush), métrica de adoção (§VI-3).
- **Deferida (não nesta rodada):** #26 (permission_prompt surfacar o comando ao orquestrador) — toca o circuito de attention/host; candidata da rodada de UI ou junto da Mentality. #15 (briefing/observabilidade ampla) — núcleo já existe; #27 era o gargalo real.
- **Fora do escopo:** F3-3 Mentality (próxima onda, exige `events.rs` limpo); #25 ENOSPC → F3-5-8; qualquer toque em `events.rs`/`bridge.rs` (território do A).

---

## STATUS DA EXECUÇÃO — F3-CONF-3 ✅ FECHADA EM CÓDIGO + GATE DE ONDA PASS (2026-06-19)

**Gate de código FECHADO ✅ + Gate de onda (4 lentes) PASS:**
- **Validação final consolidada (de fora, exits diretos):** `cargo test -p lina-bootstrap` 0 · `cargo test -p lina-core` 0 (inclui `f3_conf3_router_autoridade` + suíte de segurança 420 `--lib` + `f3_conf2`/`f3_2`/`goal_adversarial`) · `clippy -p lina-bootstrap -p lina-core --all-targets -D warnings` 0 · `fmt --check` 0.
- **Cold-review:** R1 (lina.rs) PASS — #27 seguro (autoridade do env), #23c `resolve_check_node` idêntico + `reconcile_roster_status` não inventa morte nem toca `node_by_name`/router, #21 nunca pendura; `LifecycleProjection` DRY justificado. R3 (testes) PASS — segurança do router **provada por mutação** (desligar `is_alive()` derruba 3/4, caminho-feliz segue = não-vacuoso), recency-trap, 3 formas de morte. **0 ALTA.**
- **Gate de onda (4 lentes):** (1) fecha o passo-6 do loop sem virar harness; (2) observabilidade ≠ autoridade, sem evento novo, replay intacto; (3) red-team 0 ALTA (parse `n-` não é bypass; reconcile não decide membresia; `node_by_name` intacto por mutação); (4) #27/#23c/#21 atacados, divergência do Tradutor documentada (não adaptada em silêncio).

**Commits por fatia:** `5fece5c` R2 ADOÇÃO (J, read-only) · `d5977bf` R1+R3 (lina.rs do H + 4 testes do R).

**Achado de produto p/ o épico (evidência §VI-3, do termômetro do R2):** no log real a inteligência F3 tem adoção **PARCIAL** — metas são definidas e confirmadas, mas **0 decompostas/fechadas** e **0 via `@Tradutor`**. A próxima onda (Mentality, F3-3) deve medir adoção desde o dia 1 (a sentinela `[LINA::CORRECTION]` já tem slot no `intelligence_adoption`).

**Coordenação com o Terminal A:** rodada inteira correu **sem tocar** nenhum dos arquivos do A (events.rs, bridge.rs, agent_modal.rs, main.rs, runtime.rs, ADRs 0037/0038). Mentality (F3-3) engatilhada para quando o A commitar e `events.rs` ficar livre.

**[DIFERIDO ao fundador]** validação na tela ao vivo (gate g): após rebuild, conferir `lina history`/`lina list`/`lina vault search` no caminho real.

---

## STATUS DA EXECUÇÃO — F3-CONF-3 (LANÇADA, 2026-06-19)

- **Plano:** este doc. **Despachos:** `tasks/epico-f3/despachos/conf-v3/{R1-resolucao,R2-adocao,R3-qa}.md`.
- **Itens no plano:** `CV3R1` (@Terminal H), `CV3R2` (@Terminal J), `CV3R3` (@Terminal R) — semeados via `lina plan add` com `--accept`.
- **Disparo (fire-and-forget):** os 3 handoffs entregues; **arranque confirmado** — H/J/R todos em `Busy (a2a_delivery)` (não engoliram o 1º despacho; passo-6 do loop OK no marco inicial).
- **Próximo marco (Maestro/B):** confirmar 1º DomainEvent de progresso (claim/edição) de cada um → validar de fora (exit codes diretos) → cold-review isolado (PASS) → commit por fatia → gate de onda (4 lentes). Breaker sticky: item que falhar 2× → escala narrada ao fundador. Reserva: I, G, K.

### Progresso por fatia
- **R2 · ADOÇÃO (J) — ✅ FECHADA e COMMITADA (`5fece5c`).** Validado de fora: build/clippy `--all-targets -D warnings`/test (5/5) verdes. Cold-review PASS: read-only confirmado (zero `append`/`write`), taxa `None` sem denominador (nunca 0/0), `unreachable!` provado inalcançável (braço externo lista os 6 kinds), divergência despacho→código documentada no código (Tradutor via `GoalDefined.origin`). Fronteira respeitada (só o bin novo). **Achado p/ o gate (evidência do risco §VI-3):** no log real o funil de Goal trava em "confirmadas" (0 decompostas/fechadas) e 0 metas via `@Tradutor` — adoção PARCIAL medida, não assumida.
- **R1 · RESOLUÇÃO (H) — ✅ ENTREGUE + cold-review PASS (aguardando commit conjunto c/ R3).** `lina.rs` (+488/−136, só esse arquivo). #27: `parse_node_id_lenient` aceita `n-<uuid>` E uuid puro, rejeita lixo (autoridade segue do env do app). #23c: extraída `LifecycleProjection` (def única de morte `node_is_dead`); `resolve_check_node` virou wrapper IDÊNTICO; `reconcile_roster_status` faz `lina list` casar `lina check` SEM inventar morte e SEM tocar `node_by_name`/router (linha vermelha respeitada). #21: `vault_search_into` (deadline 8s + teto 4000 arquivos + flush por hit, parametrizado/testável) nunca pendura. Validado de fora: build/clippy `--bins`/fmt 0. **Cold-review PASS** (segurança + DRY justificado + anti-slop). 7 testes inline. ⚠️ `clippy --all-targets` RED por 2 `doc_lazy_continuation` num teste do R (não-H; roteado ao R).
- **R3 · QA/RED (R) — 🔄 em curso.** 2 repro criados (`repro_conf3_history_n_prefix.rs` #27, `repro_conf3_list_vivo_vence_morto.rs` #23c). Roteado: corrigir os 2 doc-comments do próprio teste de vault + provar segurança do router verde por mutação. Aguardando PRONTO.
- **Decisão de commit:** R1 (código) será commitado JUNTO com R3 (testes que o provam) — a suíte completa do R é a prova de integração de R1; commitar antes seria confiança parcial. R2 já foi commitado isolado (read-only, sem teste do R cobrindo).
- **Coordenação c/ Terminal A:** A expandiu para ADR 0038 (kit/skills no spawn — `bridge.rs`/`main.rs`/`runtime.rs`) + 0037. **Verificado: 0038 NÃO toca `lina.rs`** (território do H) nem `tests/` (R). Fronteira da rodada intacta.
