# Despacho M-PROMO · Projeção Mentality + Política de Promoção — Terminal I (opus · high)

> Rodada **F3-3 Mentality — "A Lina aprende com você"**. Maestro desta rodada: **Terminal A** (reporte a ele; o Terminal B agora é worker da M-DETECTOR).
> ⛔ **NÃO INICIE** até o Maestro avisar "contrato commitado" (as 6 variantes de crença em events.rs + `mod mentality` já no lib.rs). Você consome o contrato; sem ele, `mentality.rs` não compila.
> Marcador OBRIGATÓRIO ao terminar: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES, nesta ordem:**
1. `tasks/epico-f3/onda-f3-3.md` — o plano (sua fronteira, o invariante "crença ≠ autoridade", o gate (b)/(f)).
2. **Vault (design FECHADO — não redesenhe):** `lina vault index` → `35 - Proposta F3 - Mentalidade por Papel (Agent Primitives)` §3 (modelo de eventos: ciclo de vida da crença), §4.3 (política de promoção), §7 (eval-first). Caminho: `Ecossistema Labs - Operação/Debriefing Vibe Coding/35 - Proposta F3 - Mentalidade por Papel (Agent Primitives).md`.
3. Vault `20 - Auto-Aprimoramento e Memoria` cluster A.2 (a **lista de NÃO-capturar**: claims negativos sobre CLIs/ambiente do usuário NÃO viram crença durável).
4. ADR 0005 (projeção por replay). **O molde de projeção-por-replay a espelhar:** `crates/lina-core/src/bin/intelligence_adoption.rs` (o J fez na rodada passada — lê o log, agrupa por chave, taxa `None` sem denominador) e o `CostLedger` (busque-o no código).
5. O contrato de eventos (LEITURA de `events.rs`, território do Maestro — NÃO edite): as 6 variantes `CorrectionObserved`/`BeliefProposed`/`BeliefReinforced`/`BeliefChallenged`/`BeliefEstablished`/`BeliefRetired` e seus campos. Confirme os nomes exatos com `grep -nE 'CorrectionObserved|BeliefProposed' crates/lina-core/src/events.rs`.

## 2. FUNÇÃO

Você é o dono do **cérebro determinístico da Mentality**: a projeção `Mentality(papel)` (estado de leitura, reconstruído por replay) e a **política de promoção SEM LLM** (contadores sobre o log). É o coração que decide quando uma correção vira crença estabelecida — e jamais um modelo decide isso.

## 3. DIRECIONAMENTO

- **Crie UM arquivo novo:** `crates/lina-core/src/mentality.rs`. O Maestro já adicionou `pub mod mentality;` ao `lib.rs` no setup — **você NÃO toca `lib.rs`** (costura, dono do Maestro). Se precisar exportar algo, exporte de `mentality.rs` (o `pub` no módulo basta).
- **Projeção `Mentality(papel)` por REPLAY** (ADR 0005, padrão `CostLedger`/`intelligence_adoption`): varra `EventStore::events()` (ou os `EventRecord`) e reconstrua o estado das crenças por PAPEL. Crença **nunca deletada** — `BeliefRetired`/`BeliefChallenged` rebaixam/aposentam, mas o histórico permanece reconstruível. Teste: replay reconstrói a projeção **idêntica** (byte-a-byte / igualdade estrutural).
- **Política de promoção DETERMINÍSTICA (ZERO LLM — regra dura, inv #1):**
  - `BeliefEstablished` sse **N situações DISTINTAS** confirmaram (default **N=2**), distinção por **hash-de-situação** (a MESMA situação 2× **não** conta — anti-gaming, spec 35 §4.3/§6.4).
  - `BeliefChallenged` (usuário corrige de novo no mesmo tema) **ZERA** o progresso de promoção.
  - Provisória sem reforço em **TTL** (default 30d — mesmo vocabulário stale do `lina retro`) → expira (`BeliefRetired{reason:expired}`). Use o `now_ms` injetado (NÃO `Date::now()`/wall-clock embutido — protocolo time-based mede contra o now_ms passado; ver memória do time).
  - **Cap top-K** por recência/uso na seleção para injeção (a M-INJETOR consome isto): exponha uma função que devolve as top-K crenças do papel (estabelecidas primeiro, depois provisórias marcadas). K é critério de aceite (gate c), não opção.
- **Faixa do que é capturável (Hermes 20 A.2):** a política NÃO promove claims negativos sobre CLIs/ambiente a crença durável (isso é ruído, não lição). Statement falseável, sem PII (padrões, não pessoas — LGPD, spec 35 §6.6). Esses filtros estruturais que NÃO exigem LLM ficam aqui; o filtro semântico anti-poisoning é do Refletor (M-DETECTOR) — você cobre o que é determinístico.
- **NÃO toque:** `events.rs` (Maestro), `router.rs`/`a2a.rs` (M-DETECTOR), `bridge.rs` (M-INJETOR), app (M-UI). Se precisar de um campo a mais numa variante de evento → **BLOCKED + escala ao Maestro** (ele é dono do contrato).
- Convenções: edition 2021; `cargo fmt -p lina-core` (só seu pacote); `clippy --all-targets -D warnings` 0; zero `unwrap()` em produção; doc-comments `//!`/`///` explicando o PORQUÊ (cite spec 35 §X). Testes `#[cfg(test)]` no próprio módulo com **controle positivo E negativo** (gate b: mesma situação 2× NÃO promove; situações distintas promovem; challenge zera).
- **Você NÃO commita.** Valide local e reporte.

## 4. OBJETIVO

Materializar "corrigiu uma vez, nunca mais repete" como mecanismo confiável e auditável: a correção do usuário vira crença do papel SÓ com evidência diversa real (não por gaming), sem nenhum LLM decidir o guardrail, e o histórico é reconstruível por replay. É a peça de MEMÓRIA da personalidade da Lina.

## 5. RESULTADO ESPERADO

- `crates/lina-core/src/mentality.rs`: projeção `Mentality(papel)` + política de promoção + seleção top-K, com testes de mecanismo (controle +/-).
- Prova local (exits limpos, sem pipe mascarando): `cargo test -p lina-core --lib mentality 2>&1 | tail` + exit; `cargo clippy -p lina-core --all-targets -- -D warnings`; `cargo fmt -p lina-core`.
- Reporte: `lina ask "@Terminal A" "M-PROMO: <o que entregou + exits>" --intent status` terminando com **`PRONTO: M-PROMO — projeção+promoção determinística, N-distintos/challenge-zera/TTL/cap-K provados por contraste, replay idêntico`** ou **`BLOCKED: <motivo>`**.

> Anti-vacuosidade (spec 35 §7): cada teste com controle positivo E negativo. Um teste que só prova "promove" sem provar "mesma situação NÃO promove" é vacuoso. Prove o mecanismo, não a feliz coincidência.
