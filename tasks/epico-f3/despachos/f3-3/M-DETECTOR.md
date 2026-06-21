# Despacho M-DETECTOR · Detector de correção + Refletor — Terminal B — Effort: Ultra Code (opus · high)

> Rodada **F3-3 Mentality**. Maestro desta rodada: **Terminal A** (reporte a ele).
> **Reatribuição:** esta frente passou de H para **você, Terminal B** — é a parte de backend mais sensível da onda (toca o caminho crítico `router.rs`/`a2a.rs`); o Maestro virou o A, então você (Ultra Code) assume a captação. O Refletor assíncrono fora do caminho crítico é o ponto mais delicado de toda a rodada — é seu.
> ⛔ **NÃO INICIE** até o Maestro avisar "contrato commitado" (variantes de crença em events.rs).
> Marcador OBRIGATÓRIO: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES:**
1. `tasks/epico-f3/onda-f3-3.md` — o plano (fronteira, invariante "crença ≠ autoridade", gate (d)/(g)).
2. **Vault (design fechado):** `35 - Proposta F3 - Mentalidade por Papel` §4.1 (Detector de correção — MVP por sentinela), §4.2 (Refletor — job assíncrono, 1 CLI-call), §6.3 (anti-poisoning). + `20 - Auto-Aprimoramento e Memoria` A.1 ("fork pós-turno FORA do caminho crítico" + o Cuidado: não é LLM no core).
3. **O molde de sentinela a espelhar:** `crates/lina-core/src/mailbox.rs:894` (`render` do bloco `[LINA::MSG]` — namespace `LINA`, terminador `[/LINA::MSG]`); e como o fim-de-resposta por sentinela é detectado em `crates/lina-core/src/a2a.rs:389-447`. O `[LINA::CORRECTION]` é da MESMA família — reuse o parser/estilo, não invente formato novo.
4. O contrato (LEITURA de `events.rs`): `CorrectionObserved` (campos: papel, resumo, refs) e `BeliefProposed` (belief_id, statement, proveniência). Confirme nomes com grep.

## 2. FUNÇÃO

Você é o dono do **caminho de captação**: detectar quando o usuário corrige um terminal (via sentinela `[LINA::CORRECTION ...]` que o agente emite, instruído pela doutrina do papel) → emitir `CorrectionObserved`; e o **Refletor** assíncrono que destila a correção em `BeliefProposed` com **1 CLI-call barata FORA do caminho crítico** (nunca um LLM dentro do router/guard).

## 3. DIRECIONAMENTO

- **Fronteira:** o caminho de detecção/reflexão em `crates/lina-core/src/{a2a.rs,mailbox.rs,router.rs}` (o que já parseia `[LINA::MSG]` e detecta sentinela de fim). Costura compartilhada `router.rs` — se precisar de mais que um ponto de hook, **mapeie o diff e proponha ao Maestro** (não monopolize router.rs). **NÃO toque:** `events.rs` (Maestro), `mentality.rs` (I), `bridge.rs` (J), app (G).
- **Detector (MVP, spec 35 §4.1):** o agente DECLARA via sentinela no PTY (`[LINA::CORRECTION ...]`), instruído pela doutrina do papel — o Lina já parseia `[LINA::MSG]`, reuse o mecanismo. Captura DETERMINÍSTICA e barata (detector estatístico de transcript = v2, fora do MVP, spec 35 §8). Ao detectar → emite `CorrectionObserved{papel, resumo, refs}` (papel/origem carimbados server-side, NUNCA do payload do agente — ADR 0007).
- **Refletor (spec 35 §4.2; Hermes 20 A.1):** job **assíncrono fora do caminho crítico** — 1 CLI-call neutra (no mesmo CLI/sessão para reusar o cache DELE, ou terminal-sombra) que transforma `CorrectionObserved` → `BeliefProposed` em formato FIXO. **NÃO é um LLM no core (inv #1)** — é uma chamada ao CLI de terceiro, como qualquer injeção. Se você não conseguir um caminho assíncrono limpo agora, entregue o **detector + o evento `CorrectionObserved`** e deixe o Refletor como hook nomeado (BLOCKED parcial documentado) — não force um LLM síncrono no router.
- **Regras duras do Refletor (spec 35 §4.2/§6.3):** 1 crença por correção; statement falseável (formato Why/How-to-apply, 1-2 frases); **sem PII**; **jamais instrução de segurança/permissão**; recusa crença que contradiga doutrina shipped (vira proposta narrada, não crença) — o **filtro anti-poisoning** vive aqui (o M-QA testa). Crença nascida em sessão que processou conteúdo externo não-confiável leva **flag de origem**.
- **Segurança (regra-mãe):** nada que você capta do PTY decide autoridade. `[LINA::CORRECTION]` é DADO; o papel/origem vêm do binding server-side, não do texto. A sentinela é gameável — aceita porque (a) mesmo trust boundary da doutrina, (b) a política exige situações distintas (M-PROMO), (c) o humano é árbitro (M-UI aposentar). Documente isso.
- Convenções: edition 2021; `fmt -p` só seu pacote; `clippy --all-targets -D warnings` 0; zero `unwrap()` em produção; sem erro engolido. **Você NÃO commita.**

## 4. OBJETIVO

Fechar a ponta de ENTRADA do aprendizado: quando o usuário corrige um terminal, a lição é capturada de forma barata e determinística e destilada por um reflexo assíncrono — sem nunca colocar um modelo no caminho crítico do core nem deixar um texto do agente virar autoridade.

## 5. RESULTADO ESPERADO

- O detector da sentinela `[LINA::CORRECTION]` emitindo `CorrectionObserved` + o Refletor (ou o hook nomeado, se assíncrono não fechar) produzindo `BeliefProposed`.
- Prova local (exits limpos): `cargo test -p lina-core 2>&1 | tail` + exit; `clippy --all-targets -D warnings`; `fmt -p lina-core`. Teste do parser da sentinela (positivo: extrai papel/statement; negativo: sentinela malformada/instrução de segurança é recusada).
- Reporte ao Maestro com **`PRONTO: M-DETECTOR — sentinela [LINA::CORRECTION]→CorrectionObserved + Refletor async→BeliefProposed (filtro anti-poisoning), fora do caminho crítico`** ou **`BLOCKED: <motivo>`**.

> Se o Refletor assíncrono exigir tocar o agendamento do pump (CORE sensível) ou um caminho que colida com outra frente → BLOCKED + mapa do diff ao Maestro. Entregar o detector + evento já destrava o M-PROMO/M-QA; o Refletor pode ser fatia 2.
