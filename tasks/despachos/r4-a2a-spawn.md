# DESPACHO r4-a2a-spawn — Core A2A (terminal spawnado)
**id:** `a2a-spawn` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Três achados ALTA de dogfooding que bloqueiam a orquestração por spawn — o coração do produto ("o time coopera sem fios"). Rodada r4 (saída F1).

## CONTEXTO
- `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"` (HEAD `f1d4810`).
- Fonte primária (LEIA INTEIRA a tabela de 2026-06-11): `tasks/despachos/achados-dogfooding-sessao.md` — achados **#12, #13, #16** (ALTA) + **#4/#14** (cosmético, se sobrar):
  - **#12** `lina spawn --prompt` NÃO entrega o 1º prompt: o nó nasce, roda só o bootstrap e fica Idle; a msg `msg_spawn1st-*` cai em `dead-letter/`. Evidência: event log seq 20812-20865 do workspace walking-skeleton.
  - **#13** `lina handoff` para recém-spawnado fica preso em `outbox/.inflight/` sem `MessageDelivered`; agravante: resolução nome→nó casa com lifecycle MORTO de sessão antiga quando o nome foi reusado.
  - **#16** pump de entrega A2A não drena o `.inflight` após reabrir o app (zero `MessageRouted/Delivered/Retained` pós-boot).
- **⚠️ ANTES DE CODAR, RE-DERIVE:** esses achados foram observados ANTES dos fixes M8 (dreno por-runtime `e08ad5a`, troca viva `3eb915a`, integração `dad735d`). O HEAD pode ter consertado #16 por efeito colateral (o boot agora ergue `WsRuntime` com dreno próprio). Para cada achado: escreva primeiro um teste que REPRODUZ no HEAD (RED). Se já estiver verde, registre "fechado por efeito colateral de X" com a linha que prova — e siga ao próximo.
- Mapa do código: `crates/lina-core/src/mailbox.rs` (outbox/.inflight/drenagem/carimbo from) · `crates/lina-core/src/router.rs` (entrega, retenção, DLQ, guardrails) · caminho do spawn e do 1º prompt: grep `spawn1st\|SpawnAdmitted\|first_prompt` em `crates/lina-core/src/` e `app/lina-gpui/src/bridge.rs` · resolução nome→nó: grep `resolve` em lina-core + `crates/lina-bootstrap/src/` (verbo `check`). O boot por-runtime novo: `app/lina-gpui/src/runtime.rs` (SÓ LEITURA para entender o dreno — fronteira do app não é sua).
- Workspaces reais para inspecionar artefatos (read-only): `~/Library/Application Support/Lina/walking-skeleton/.lina/` (dead-letter e .inflight com as msgs presas citadas nos achados).

## FUNÇÃO
Você é o dono do core A2A nesta rodada (mailbox/router/spawn-delivery) — dono único das costuras `mailbox.rs`/`router.rs` na r4.

## DIRECIONAMENTO
- Fronteira: `crates/lina-core/src/**` + testes do crate; `crates/lina-bootstrap/src/**` SÓ se a resolução nome→vivo exigir (verbo check). **NÃO toque** `app/lina-gpui/**` — se a causa-raiz exigir 1 linha no app (ex.: fiação do pump no boot), REGISTRE o pedido de costura na entrega e o Maestro fia.
- **Doutrina de segurança é inegociável** (regra 7): você está mexendo no caminho de entrega — a suíte de segurança do router precisa seguir verde; nenhum campo escrito por agente decide identidade/ordem/autorização; a resolução nome→nó vivo NÃO pode virar vetor de redirecionamento (quem decide o alvo é a projeção do log do app, nunca payload).
- TDD estrito: teste RED que reproduz → fix de causa-raiz → GREEN. Sem fix temporário, sem engolir erro. Eventos novos aditivos (`serde(default)`).
- Causa-raiz > sintoma: se #12 e #13 tiverem a MESMA raiz (ex.: entrega a nó cujo PTY ainda não está pronto + retenção que não re-tenta), conserte a raiz uma vez e prove os dois.

## OBJETIVO
Um orquestrador que spawna um colega e o colega nunca recebe a tarefa quebra a promessa nº1 do Lina. Esses 3 achados são o que separa "o spawn funciona na demo" de "o spawn funciona de verdade". A F1 não deveria sair com eles abertos sem ao menos causa-raiz nomeada.

## RESULTADO ESPERADO
`tasks/epico-f1/.entrega-a2a-spawn.md`: por achado — causa-raiz nomeada (arquivo:linha) OU "já fechado por X" com prova; testes RED→GREEN listados; validação por-pacote (`cargo test -p lina-core -- --test-threads=1`, clippy `-D warnings`, fmt) com exit DIRETO; pedidos de costura se houver. Marcador `.iniciado-a2a-spawn` no primeiro ato. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
