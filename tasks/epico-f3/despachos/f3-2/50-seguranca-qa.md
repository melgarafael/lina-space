# Despacho F3-2 · QA/Red-team — segurança do loop + adoção + repro do gargalo #22/#23
**Para:** Terminal R · **model·effort:** opus · High · **Dono de:** `crates/**/tests/` + `#[cfg(test)]` (lê produção, não altera) + relatório em `tasks/`

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (caminho absoluto).
- **LEIA primeiro:**
  1. `tasks/epico-f3/onda-f3-2.md` §gate (g) + §riscos (o risco operacional nº1).
  2. Spec 52 §Segurança (linhas 345-356) — as 5 invariantes que a onda não pode quebrar; §Critérios (359-370).
  3. A suíte de segurança do Router existente (o padrão a seguir): `crates/lina-core/src/router.rs` testes `delegation_budget_is_enforced_per_root_cause`, `spawn_cascade_requires_human_gate`, `gate_d`/`reviewer != target`. Carregue a skill `lina-cold-review` (rubrica anti-slop) e `provar-enforcement-por-mutacao` (padrão ouro: desligar a guarda → ver o teste falhar → religar).
  4. Família dogfooding #22/#23 (handoff confirmado ≠ trabalho feito; 5 ocorrências): `tasks/despachos/achados-dogfooding-sessao.md` (achados #22/#22b/#22c/#23/#23b/#23c). Pistas no código: `mailbox.rs`, `router.rs:758` (freio), `resolve_check_node`.

## FUNÇÃO
Você é o **revisor adversarial** da onda: prova que a inteligência nova (interpretação, montagem de time, escalonamento) **nunca vira autoridade**, e investiga (sem tocar produção) por que despachos confirmados às vezes não viram trabalho — o gargalo nº1 da orquestração.

## DIRECIONAMENTO (regras do jogo)
- **Escreva SÓ testes** (`crates/**/tests/`, `#[cfg(test)]`) e o **relatório** (`tasks/epico-f3/repro-22-23.md`). **NÃO altere lógica de produção** — se achar um bug, reporte ao Maestro com evidência arquivo:linha; não conserte (o dono da fatia conserta).
- **Prove por mutação** onde fizer sentido: desligue a guarda → veja o teste ficar vermelho → religue. Existência de check ≠ propriedade enforçada (calibração da rubrica).
- **Re-prove achados de cético empiricamente** antes de afirmar (memória: um "crítico" pode ser falso positivo). Limpe scratch.
- **Sobre os testes flaky** (memória + achados #18/#57/#68): família install/discovery/history flakeia SÓ em paralelo — rode isolado/serial para desambiguar antes de gritar "regressão".

## OBJETIVO (o porquê de negócio)
A regra que atravessa a Fase 3 é "crença/interpretação/avaliação/effort é DADO, jamais autoridade". Se isso vazar, a inteligência do Maestro vira um buraco de segurança. E se handoff confirmado não vira trabalho, o loop do Maestro nativo (F3-2) não fecha na vida real — o fundador já viu falhar 5×.

## ESCOPO — 3 fatias
- **Segurança do loop (F3-2 gate g):** suíte adversarial sobre o caminho NOVO (montagem de time + escalonamento): `proposed_team`/`interpretation` injetados no payload **não** alteram `by`/`reviewer`/`origin` carimbados server-side; `reviewer != target` no caminho novo; `ReviewVerdict{Pass}` **não** libera `GatedHard`; `GoalConfirmed` forjado com `by` de outro nó **ignorado**. Coordene com B (CORE) para casar os testes ao código que ele entrega — via `route_message` (caminho real).
- **Adoção (risco #3 da fase):** um teste/script que conta uso real de `lina goal`/`origin:"@Tradutor"`/montagem de time no event log — para o gate medir adoção, não assumir.
- **Repro #22/#23 (READ-ONLY):** investigue a família "delivery→end_of_response sem DomainEvent novo". Produza `tasks/epico-f3/repro-22-23.md` com: hipótese de causa-raiz (com arquivo:linha), um **teste de reprodução** (mesmo que `#[ignore]` por ora) que capture o sintoma, e a recomendação de fix. **NÃO conserte** (toca `router.rs`/`mailbox.rs` — colidiria com o CORE desta rodada; o fix é rodada dedicada).

## RESULTADO ESPERADO (formato exato)
- Testes de segurança verdes (provados por mutação onde aplicável) + script/teste de adoção + `repro-22-23.md` (hipótese + teste de repro + recomendação).
- `cargo test -p lina-core` verde (rode isolado se piscar flaky; cole a contagem e diga se houve flaky).
- **NÃO commite.** Reporte o 1º progresso (`lina ask "@Terminal A" "comecei QA/segurança F3-2" --intent status`).
- Termine com **`PRONTO: <PASS/FAIL por invariante + estado do repro #22/#23>`** ou **`BLOCKED: <motivo>`**. Se achar 1 ALTA de segurança, sinalize explícito (0 ALTA é requisito do gate).
