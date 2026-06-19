# Despacho F3-CONF-2 · CORE-LOOP — Terminal B (Ultra Code) · model opus · effort High

> **Antes de codar:** rode `lina plan read CONF-CORE` e LEIA `tasks/epico-f3/repro-22-23.md` (taxonomia completa — você vai fechar o Modo B) + `tasks/epico-f3/onda-f3-conf-2.md` (o plano da rodada). O contrato do `reason` já está COMMITADO pelo Maestro (HEAD da largada) — parta dele.

## 1. CONTEXTO (onde o trabalho está)

`cwd`: raiz do repo. Você é **dono único** de `crates/lina-core/src/router.rs` + `crates/lina-core/src/attention.rs` nesta rodada. O contrato (reason do breaker no Bus/host) já foi fixado e commitado pelo Maestro — `router.rs:1533-1541` já passa `reason:"circuit_breaker"`; NÃO refaça isso.

**O gargalo (Modo B do #22/#23):** um handoff é ENTREGUE de verdade (`MessageDelivered`), o nó volta a `Idle` (`end_of_response`) e **zero trabalho acontece** — nenhum `TokenUsageReported`/`MessageRouted{from}`/`pty_output`. Hoje há DETECÇÃO mas **ninguém CONSOME o alarme**. Mapa preciso do terreno (já levantado):

- **A projeção que detecta** (`attention.rs`): `AttentionKind::DeliveredNoProgress` (`attention.rs:128`), janela `DELIVERED_NO_PROGRESS_WINDOW_MS = 90_000` (`attention.rs:99`). Arma em `arm_dispatch` (`attention.rs:551`) no `MessageDelivered` (`attention.rs:407`); abre a janela no `Idle` via `mark_dispatch_idle` (`attention.rs:571`, fold em `attention.rs:424`); desarma por progresso (`remove_dispatch`, `attention.rs:564`) — progresso = `TokenUsageReported` (`attention.rs:410`) ∨ `MessageRouted{from}` (`attention.rs:412`) ∨ `NodeStatusChanged{Busy,pty_output}` (`attention.rs:420`). A síntese pública é `items(now_ms)` → closure `swallowed` (`attention.rs:744`). **Hoje só a UI lê `items()`; o `router.rs` NÃO tem caminho para perguntar "este nó está engolido?".**
- **O OUTER loop** (`router.rs`): `review_and_advance` (`router.rs:2132`) é disparado por `plan.check` (`handle_plan`, `router.rs:1917`); no FAIL chama `escalate_on_fail` (`router.rs:2215`), onde vivem o `effort_ladder` (`router.rs:2225`), o **breaker sticky** (`count_respawns_with_effort >= 1`, `router.rs:2234`; helper `router.rs:3109`) e a emissão do (re)despacho via `SpawnRequested` (`router.rs:2275`). **O re-despacho HOJE só nasce de `ReviewVerdict::Fail`** — o despacho engolido nunca chega ao juiz, então nunca gera re-despacho. Esse é o vão.
- **O anti-loop** (`router.rs`): `handoff_would_tight_loop` (`router.rs:3324`), chamado em `route_message` (`router.rs:1099`). Conta repetições topológicas do salto `from→to` sob o mesmo `root_cause_id` (`router.rs:3356`), cego ao progresso → barra o Maestro de re-despachar um worker engolido (achado #22c).
- **Eventos** (`events.rs`, NÃO editar — contrato fixo): `MessageDelivered{to}` (376), `TokenUsageReported{node}` (483), `MessageRouted{from,root_cause_id,to_node}` (359), `GoalEscalated{goal_id,reason}` (644, vocabulário inclui `"stalled"`), `SpawnRequested{...effort,goal_id,prompt}` (963), `NodeStatusChanged{node,status,reason}` (317).

## 2. FUNÇÃO

Você é o **dono do mecanismo de orquestração confiável**. Mecaniza no OUTER loop o protocolo que hoje só vive em skill (`lina-orchestration` passos 6-7): confirmar o 1º sinal de progresso, re-despachar informado 1×, breaker sticky → escalar. Você também cria o **verbo de reset do circuit_breaker** (não existe hoje — confirmado).

## 3. DIRECIONAMENTO (as regras do jogo)

- **Mexa SÓ em** `router.rs` + `attention.rs`. Precisa de campo novo em `events.rs`/`lib.rs`? **PEÇA AO MAESTRO** (`lina ask "@Terminal A" "preciso de <campo> em events.rs para <razão>" --intent ask`) — `events.rs`/`lib.rs` são contrato fixo. Provavelmente você NÃO precisa: reuse `SpawnRequested` (re-despacho) + `GoalEscalated{reason:"stalled"}` (escala). Se precisar de um marcador durável do "1º progresso confirmado" ou do contador sticky-por-engolimento, **proponha a variante aditiva ao Maestro antes**.
- **ZERO LLM no core (inv #1):** o "juiz de engolimento" é ESTRUTURAL — ausência dos 3 sinais de progresso na janela. Nunca um modelo, nunca heurística de linguagem.
- **Regra-mãe (ADR 0007):** o re-despacho/escala é DADO, não autoridade. `reason`/`by` carimbados server-side. O **reset do breaker exige gesto humano** (`HUMAN_GESTURE`, espelhe `human_intent`/`GoalConfirmed.by`) — um agente NUNCA auto-resseta seu próprio breaker.
- **loop_detected:** ao isentar a correção legítima, consulte o **sinal de progresso** (a mesma noção do `attention.rs`: houve `TokenUsageReported`/`MessageRouted{from}`/`pty_output` desde a última entrega ao alvo?). "Engolido" = re-despacho ao mesmo nó SEM progresso desde a entrega anterior → ISENTA. "Tempestade" = saltos repetidos COM atividade no meio → CONTINUA barrado.
- `cargo fmt -p lina-core` só nos SEUS arquivos (memória *fmt em árvore compartilhada*). `clippy -D warnings` limpo. Sem `unwrap()` em produção. Eventos aditivos.
- A fronteira REAL de "+campo em evento" inclui TODO construtor literal (lição #61: `grep -rn "SpawnRequested {"` antes) — mas você provavelmente não adiciona campo; se adicionar, é via Maestro.

## 4. OBJETIVO (o porquê de negócio)

Hoje o fundador despacha 3 tarefas, o app diz "entregue", e **nada acontece** — ele descobre só olhando o disco vazio (aconteceu numa reunião real com cliente, #22c). O loop do Maestro nativo (o coração do produto: "o time coopera sem fios") precisa **fechar na vida real**: se o despacho não vira trabalho, o Maestro percebe sozinho, tenta de novo informado, e se não fecha, avisa o humano — sem o humano ter que vigiar a tela.

## 5. RESULTADO ESPERADO (formato + marcador)

Diffs em `router.rs`/`attention.rs` + testes pelo **caminho real** (`route_message`/`pump`, nunca montando o evento à mão — lição "teste à-mão não prova a costura"), provando RED→GREEN:
1. **Protocolo anti-engolimento:** entrega real → Idle sem progresso na janela → re-despacho 1× informado → persiste → `GoalEscalated{reason:"stalled"}`; breaker sticky impede 3ª tentativa automática. (gate **a**)
2. **loop_detected calibrado:** teste por contraste — correção de engolido passa, tempestade barra. (gate **b**)
3. **Verbo de reset do breaker** sob gesto humano (`by` server-side), + o reason já flui (contrato). Exponha o ponto que a UI (G) e/ou o bin chamam — coordene a assinatura com o Maestro p/ G ligar o botão "liberar". (gate **c**, metade core)
4. **(encaixe se folga)** vigia de retenção Modo C (`attention.rs` consome `MessageRetained` sem `Delivered`/DLQ na janela) — secundário, não bloqueante.

Valide de fora: `cargo test -p lina-core` (sem `--lib` se tocar tests/) + `cargo clippy -p lina-core --all-targets -D warnings` + `cargo fmt -p lina-core --check`. **NÃO commite** — reporte ao Maestro com os exit codes.

Termine com `PRONTO: <o que entregou + exits dos testes/clippy/fmt + a assinatura do verbo de reset p/ G ligar>` ou `BLOCKED: <o que falta + o que tentou>`. Reporte o **1º progresso** ao Maestro assim que começar (`lina ask "@Terminal A" "comecei o CORE-LOOP" --intent status`).
