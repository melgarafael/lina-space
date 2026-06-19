# Despacho R2 · ADOÇÃO da inteligência F3 — Terminal J (opus · medium)

> Rodada **F3-CONF-3 — "O Maestro Enxerga o Time"**. Maestro: Terminal B (Ultra Code).
> Marcador OBRIGATÓRIO ao terminar: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Sem marcador = falha de protocolo.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES de começar:**
1. `tasks/epico-f3/onda-f3-conf-3.md` — o plano (sua fronteira, o gate item (d)).
2. **O molde a copiar:** `crates/lina-core/src/bin/plan_adoption.rs` — leia INTEIRO. É um **bin offline auto-descoberto** que mede adoção do PLANO (claims vs ad-hoc) lendo o `log.jsonl`. Você vai escrever um **irmão** dele para a inteligência F3. Note o padrão registrado no topo dele: "Bin auto-descoberto (`src/bin/`): zero edição em `lib.rs`/`Cargo.toml`/módulos com dono"; "É MÉTRICA ACOMPANHADA, NUNCA GATE"; taxa `null` em vez de panic/0-0.
3. **Vault (norte):** `lina vault index` → épico `39 - Epico Fase 3` §VI-3 (o **top-risco da fase é adoção 0%**: Goal/effort/Tradutor construídos e nunca acionados — "medir uso real no log desde o dia 1, não assumir"). Caminho: `Ecossistema Labs - Operação/Debriefing Vibe Coding/39 - Epico Fase 3 — O Maestro Nativo.md`. (Se `lina vault search` pendurar, é o bug #21 de outra frente — use `lina vault index`.)
4. **Os eventos que provam uso da inteligência F3** (já existem no log — confirme os nomes em `crates/lina-core/src/events.rs`, **SÓ LEITURA**, é território do Terminal A): `GoalDefined`, `GoalInterpreted`, `GoalConfirmed`, `GoalDecomposed`, `GoalAchieved`, `GoalEscalated`, `InterpretationProposed`/`InterpretationConfirmed`, `EffortAssigned`, `ReviewVerdict`, e o origin `@Tradutor` em `GoalInterpreted`. (A sentinela `[LINA::CORRECTION]` da Mentality ainda NÃO existe — deixe o slot previsto, contando 0, comentado como "futuro F3-3".)

## 2. FUNÇÃO

Você é o **autor de um novo leitor offline de adoção** — um bin read-only que responde, a partir do event log: *"a inteligência da F3 está sendo USADA de verdade, ou foi construída e nunca acionada?"*. É a métrica que o Maestro consulta no gate de cada onda nova para não repetir o erro do `plan.md` (adoção 0% no gate F1-3).

## 3. DIRECIONAMENTO

- **Crie UM arquivo novo:** `crates/lina-core/src/bin/intelligence_adoption.rs`. **NÃO toque** `lib.rs`, `Cargo.toml`, `events.rs` (território do A) nem qualquer módulo com dono. O bin é auto-descoberto pelo cargo (igual `plan_adoption`/`baseline_f1_0`) — zero costura.
- **100% read-only:** lê o `log.jsonl` (ou o events-dir) e IMPRIME. Não apenda evento, não muta nada. Use `lina_core::EventRecord` para desserializar (veja como `plan_adoption.rs` faz). Se um evento não desserializa numa versão, **conte e siga** (tolerante a log sob append/versão antiga) — nunca panic.
- **O que medir (relatório):** contagens por categoria do uso da inteligência F3:
  - **Goal:** quantos `GoalDefined`, quantos chegaram a `GoalConfirmed` (gate humano cruzado), quantos a `GoalAchieved` vs `GoalEscalated` (e por `reason`). Uma taxa "metas que fecharam" = `Achieved / (Achieved + Escalated)` com `None` se 0.
  - **Tradutor:** quantos `GoalInterpreted` com `origin:"@Tradutor"` vs degradados ao Maestro (mede a adoção do papel Tradutor da F3-2).
  - **Effort:** quantos `EffortAssigned` e a distribuição por `effort` (low/medium/high) e por `origin` (observed/assigned) — mede se o motor por papel da F3-0 está vivo.
  - **Correção/Mentality (futuro):** slot previsto contando `[LINA::CORRECTION]` (hoje 0; deixe o gancho comentado "F3-3").
  - Onde fizer sentido, agrupe por `root_cause_id` para não inflar (uma cadeia = 1, igual `plan_adoption` faz com ask-chains).
- **Honestidade do número (regra dura, herdada do `plan_adoption`):** é **MÉTRICA ACOMPANHADA, NUNCA GATE**. A saída deve dizer isso explicitamente; taxas viram `null` (N/A) quando não há denominador — nunca `0/0`, nunca panic. Adoção é comportamento de LLM (não-determinístico) — você REPORTA, não reprova.
- **Saída:** humana por padrão + `--json` (struct `serde::Serialize`, igual `AdoptionReport`). Uso: `intelligence_adoption <events-dir> [--json]`.
- Convenções: edition 2021; `cargo fmt -p lina-core` (só seu pacote); `clippy --all-targets -D warnings` 0; zero `unwrap()` em produção; doc-comment `//!` no topo explicando o que mede e a regra "métrica nunca gate" (espelhe o tom do `plan_adoption`).
- **Você NÃO commita.** Valide local e reporte; o Maestro commita por fatia.

## 4. OBJETIVO

O top-risco da Fase 3 é construir inteligência que ninguém usa (Goal/effort/Tradutor viram enfeite). O fundador precisa de um número honesto, derivado do log real, que diga se a orquestração inteligente está sendo acionada — para o gate de cada onda nova declarar com DADO, não com suposição. Você constrói esse termômetro.

## 5. RESULTADO ESPERADO

- O arquivo novo `crates/lina-core/src/bin/intelligence_adoption.rs` compilando e rodando.
- Prova local (rode e LEIA o exit, sem pipe que mascara):
  - `cargo build -p lina-core --bin intelligence_adoption 2>&1 | tail -5` + exit.
  - Rodar contra um events-dir real do Espaço (ex.: o do `$LINA_HOME/events` se existir, ou um fixture pequeno que você monta) e colar a saída no reporte.
  - `cargo clippy -p lina-core --bin intelligence_adoption --all-targets -- -D warnings 2>&1 | tail`.
  - `cargo fmt -p lina-core` (só seu pacote).
  - Se você escrever um teste unitário do parser/agrupador (recomendado — `#[cfg(test)]` no próprio bin, como `plan_adoption` tem), `cargo test -p lina-core --bin intelligence_adoption`.
- Reporte: `lina ask "@Terminal B - Effort: Ultra Code" "R2: <o que mede + saída de exemplo>" --intent status` terminando com **`PRONTO: R2 — bin intelligence_adoption read-only, build/clippy verdes, saída de exemplo anexa`** ou **`BLOCKED: <motivo>`**.

> Princípio anti-slop: um relatório que sempre diz "tudo ótimo" é pior que vermelho. Mostre o número cru (inclusive 0) e a regra de que ele NÃO é gate. Se um evento que você esperava não existe no log de teste, diga "0 observado" — não invente.
