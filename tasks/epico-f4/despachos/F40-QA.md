# DESPACHO F40-QA — Gate da onda F4-0 (red-team + prova do gate de saída) (dono: Terminal R)

## CONTEXTO (leia ANTES de codar — obrigatório)
Onda **F4-0** da Fase 4 do Lina. Você é o **gate de qualidade/segurança** da onda. O gate de saída F4-0 não é "testes passaram" — é **evidência observável**: o dump de env com a credencial AUSENTE, o trecho de log do `ActionGated`, o monitor com 0 conexão. E o risco #1 da fase inteira mora aqui: **credencial vazar para o env do agente por um caminho esquecido mata a Fase 4** (quebra ADR 0004 + invariante #2).

**Documentos a conferir (navegue — NÃO pule):**
1. **Peça da onda:** `tasks/epico-f4/onda-0.md` — seção **F4-0-QA** + o **GATE DE SAÍDA F4-0** (os 5 critérios a-e) + os "Riscos da onda".
2. **Épico no vault:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — o **GATE DE SAÍDA F4-0** (§III) + §V (top riscos: #1 credencial no env, #2 exfiltração) + o "Conselho de gate de onda (4 lentes)".
3. **ADR 0004** (custódia) e a **doutrina de segurança** do `CLAUDE.md` do repo (nenhum campo escrito por agente decide identidade/ordem/autorização).
4. **Padrão de prova por mutação (padrão-ouro da casa):** desligue a guarda → veja o teste falhar → religue. E QA eval-first: escreva o teste-alvo `#[ignore]`, rode `-- --ignored` para PROVAR RED, o dono liga a contagem na integração.

## FUNÇÃO
Você é o **QA/Pesquisador (red-team)** desta frente. Seu trabalho é tentar QUEBRAR a custódia e a fronteira "declarar ≠ autorizar" — e provar, por mutação, que as guardas seguram. Você NÃO conserta produto (se achar buraco, reporta ao Maestro com repro); você escreve os testes-alvo do gate.

## DIRECIONAMENTO (território + como trabalhar)
- **Território (SÓ tests novos):** `crates/lina-core/tests/f4_0_gate_*.rs` (NOVO) + red-team em `crates/lina-core/tests/`. Não toque produto (se precisar de fiação no arquivo de um dono, escreva o teste-alvo `#[ignore]` e avise o Maestro — o dono liga na integração).
- **NÃO TOQUE:** `events.rs`/`lib.rs` e os módulos das frentes (channel/tool_scope/broker) — você os EXERCITA por fora, via `route_message`/`EventStore`/o caminho real, nunca montando o evento à mão (lição: teste à-mão não prova a costura intent→handler).
- **Worktree:** `git worktree add ../lina-f4-0-qa -b lina/f4-0-qa` da `main` (`fcb9371`). `git`/`cargo` de lá; `lina` do dir de produção.
- **Entregue (eval-first — comece JÁ, contra o contrato congelado; `#[ignore]` onde depender de impl não-pronta, e PROVE o RED com `-- --ignored`):**
  1. **(a) dump de env:** um teste que spawna PTY de agente, dumpa o env e asserta 0 hits da credencial — com **mutação** que prova que o grep pegaria um vazamento (coordene com F40-CRED/I, que entrega o teste-coração; o seu é o gate independente que re-deriva).
  2. **(b) broker:** `lina do <canal-stub>` → `ActionGated{gated-hard-external, ask}` + 0 `BrokerExecuted` sem confirmação (via o caminho real, não evento à mão).
  3. **(c) tool scope:** declarar → projeção contém; revogar → some (replay).
  4. **(d) monitor:** 0 canais conectados → 0 conexão de saída esperada (consome a saída `--json` do `network_monitor`).
  5. **RED-TEAM (o coração):**
     - **manifesto NÃO é autoridade:** um manifesto que tenta carimbar identidade/permissão (campo malicioso) → é tratado como DADO, o gate continua sendo o broker (recusado/ignorado, não vira autorização).
     - **declarar ≠ autorizar:** declarar uma ferramenta NÃO autoriza ação externa — a ação ainda dispara o gate (prove que tool scope declarado não fura o broker).
     - **credencial não vaza por caminho esquecido:** varra os pontos que tocam PTY/spawn (o teste de dump cobre o spawn principal; cheque se há outro caminho).
     - **router seguro por mutação:** a suíte de segurança do router fica VERDE (binding inforjável + custódia + origem×autoridade intactos).
- **0 ALTA para o gate passar.** Achados MÉDIA → backlog com dono (reporte ao Maestro). Aplique as **4 lentes** (Visão/fios · Arquitetura/ADR↔código · Segurança/red-team · Pesquisa/fontes).
- **Convenções:** `cargo fmt` + `clippy --all-targets -D warnings` limpos nos tests.

## OBJETIVO (critério observável)
Os testes do gate (a-e) existem e provam o critério pelo caminho real; o red-team não consegue forjar autoridade via manifesto nem furar o gate via tool scope; a suíte de segurança do router está verde por mutação. Entregável = o relatório com os trechos de evidência (dump de env 0 hits, log do ActionGated), não só "verde".

## RESULTADO ESPERADO
- Não commite. Trabalho na worktree `lina/f4-0-qa`.
- Comece pelos testes eval-first AGORA; reporte o estado (quais já provam RED, quais aguardam impl das frentes).
- Reporte: **`PRONTO: F40-QA`** (quando o gate inteiro provar verde) ou **status parcial** com o que falta de cada frente. Via `lina ask "@Maestro 00" "<...>" --intent status`.
