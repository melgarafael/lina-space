# Despacho · W4 REPRO+QA — reproduzir o engolido + blindar a segurança (#22/#23)
**Para:** Terminal R · **model·effort:** opus · High · **Dono de:** `crates/lina-core/tests/` + `#[cfg(test)]` (lê produção, não altera)

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (absoluto).
- **LEIA primeiro:** `tasks/epico-f3/rodada-confiabilidade-orquestracao.md` (diagnóstico-raiz + gate). Carregue as skills `systematic-debugging`, `provar-enforcement-por-mutacao` e `lina-cold-review`.
- **O sintoma a reproduzir:** delivery confirmada → nó vai `Busy→Idle` (`lifecycle.rs:309`, `on_end_of_response`) → **zero DomainEvent novo, zero trabalho**. 2ª tentativa idêntica funciona.
- **Teste-âncora existente (ponto de partida):** `app/lina-gpui/src/bridge.rs:7536` (`end_of_response_returns_node_to_idle_and_unblocks_second_delivery`) cobre a transição, mas **NÃO** o caso "turno injetado não produz trabalho" — esse é o gap.
- **Caminho real a exercer:** `route_message` (`router.rs:810`) → `deliver_a2a` (`a2a.rs:311`); `ready_now` (`a2a.rs:192`), `Injected{ready:false}` (`a2a.rs:343`), `MessageRouted` pré-injeção (`router.rs:1168`).
- **Suíte de segurança a manter verde:** `delegation_budget_is_enforced_per_root_cause` (`router.rs:3647`), `spawn_cascade_requires_human_gate` (`:6366`), `forged_root_does_not_bypass_loop_detection` (`:4462`), retenção `retencao_alvo_busy_nao_injeta_e_entrega_no_idle` (`:4782`).

## FUNÇÃO
Você é o **revisor adversarial e dono do repro**. O fix de B só vale se houver um teste que falhava (RED) e passa (GREEN) com a correção — você produz esse teste. E prova que endurecer a entrega **não** afrouxou a segurança.

## DIRECIONAMENTO (regras do jogo)
- **Escreva SÓ testes** (`tests/`, `#[cfg(test)]`). **NÃO altere lógica de produção** — achou bug? reporte ao Maestro com arquivo:linha; o dono conserta.
- **Sincronize com B (W1):** vocês arrancam JUNTOS. Seu teste de repro é o critério de aceite do fix dele. Compartilhe o teste assim que ele estiver RED (`lina ask "@Terminal B - Effort: Ultra Code"`).
- **Prove por mutação:** desligue a guarda de segurança → veja o teste ficar vermelho → religue. Existência de check ≠ propriedade enforçada.
- **Re-prove achados empiricamente** antes de afirmar (um "crítico" pode ser falso positivo — memória). **Flaky em paralelo:** install/discovery/history flakeiam SÓ em paralelo — rode isolado/serial antes de gritar regressão.

## OBJETIVO (o porquê de negócio)
Sem repro determinístico, "consertamos" um bug intermitente no escuro e ele volta. Seu teste é o que prova que o gargalo nº1 da orquestração foi REALMENTE fechado — e a guarda que impede que o fix reabra um buraco de segurança no caminho mais sensível do sistema.

## ESCOPO
1. **Repro (o santo graal):** um teste determinístico que reproduz "delivered → Idle → zero trabalho" via `route_message`→`deliver_a2a` (simule o CLI fechando turno no instante da injeção). Pode nascer `#[ignore]` se precisar de timing, mas idealmente determinístico. Entregue-o a B como critério RED→GREEN.
2. **Regressão de A2A:** após o fix, a suíte do caminho de entrega segue verde (`delivery_ready_delivers_once`, retenção, restart/dedupe).
3. **Segurança (0 ALTA):** suíte do router verde, provada por mutação onde aplicável; nenhum campo de payload virou autoridade no caminho endurecido.

## RESULTADO ESPERADO (formato exato)
- Teste de repro (RED no HEAD atual, GREEN com o fix de B) + provas de mutação da segurança.
- `cargo test -p lina-core` verde (diga se houve flaky e como desambiguou); contagem colada.
- **NÃO commite.** Reporte o 1º progresso (`lina ask "@Terminal A" "comecei o repro W4, sincronizando com B" --intent status`).
- Termine com **`PRONTO: <repro RED→GREEN sim/não + PASS/FAIL por invariante de segurança>`** ou **`BLOCKED: <motivo>`**. Achou 1 ALTA? sinalize explícito (0 ALTA é requisito do gate).
