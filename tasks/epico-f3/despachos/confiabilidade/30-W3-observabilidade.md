# Despacho · W3 OBSERVABILIDADE — o alarme do "despacho engolido" (#22/#23, #15)
**Para:** Terminal I · **model·effort:** opus · Medium · **Dono de:** `crates/lina-core/src/attention.rs`

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (absoluto).
- **LEIA primeiro:** `tasks/epico-f3/rodada-confiabilidade-orquestracao.md` (diagnóstico §3 — o ponto-cego) + `docs/adr/0019-definicoes-operacionais-progresso-travamento.md`.
- **O ponto-cego estrutural:** o stall detector só corre quando o nó está **`Busy`** (`lifecycle.rs:277`; ADR 0019 §4 explícito). No #22/#23 o nó **volta a `Idle`** (`on_end_of_response`, `lifecycle.rs:309`) → o relógio de stall **nunca corre** e, sem `DomainEvent` novo, **nada detecta** o despacho engolido. PROGRESSO (ADR 0019 §2) = `tail_hash` mudou OU ≥1 `DomainEvent` novo atribuível.
- **A fila de atenção:** `attention.rs` cobre permissões (`fold_asked:394`), custódia (`:480`), spawn gated (`:220`), guard-ask (`:460`). `AttentionKind` em `:90`. **Nenhum kind** corresponde a "despacho sem resposta" — é o que você cria.
- **Decisão de arquiteto (Maestro):** o alarme é uma **projeção PURA derivada** dos eventos que já existem (`MessageDelivered{to}` + `NodeStatusChanged(Idle)` + ausência de `DomainEvent` novo atribuível), com `now_ms` passado pelo tick para a janela — **NÃO cria evento novo** (segue o padrão das outras filas de atenção, que derivam do log; menos acoplamento, sem dependência de contrato). Se durante a implementação você concluir que precisa de um evento durável para deduplicar/auditar (como `NodeStalled` `events.rs:773`), **peça ao Maestro** que ele fixa em `events.rs` — não toque `events.rs` por conta própria.

## FUNÇÃO
Você é o **dono da observabilidade do engolido**. Constrói a projeção que cruza "entregou" × "não produziu trabalho" e levanta um alarme — para o Maestro deixar de ser cego (achado #15: hoje ele PERGUNTA ao humano "o terminal recebeu?").

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `attention.rs` (projeção + `AttentionKind` novo). NÃO toque `router.rs`/`lifecycle.rs`/`events.rs` (são costura/peer) — você CONSOME `DeliveryStalled` (Maestro fixa) e `MessageDelivered` (B emite). Precisa de algo lá? **Peça ao Maestro.**
- **Projeção pura, ZERO LLM** (inv #1) e DERIVADA do log (inv #4): cruza `MessageDelivered{to}` + ausência de `DomainEvent` novo atribuível ao alvo + retorno a `Idle` dentro de uma janela → detecta. Emite o alarme **1×** (anti-amplificação, padrão `NodeStalled`/`RouteBlocked`).
- **Não confunda com stall legítimo:** o nó pode ter ido a Idle por um turno legítimo curto. A janela + a ausência de QUALQUER progresso (tail_hash + DomainEvent) é o discriminador. Calibre para não gritar falso-positivo num turno rápido real.
- Convenções: `cargo fmt -p lina-core` (só seu arquivo), `clippy -D` 0, teste de projeção (replay reconstrói idêntico; alarme emitido 1×; turno legítimo NÃO dispara).

## OBJETIVO (o porquê de negócio)
O passo 6 do loop do orquestrador é "monitorar trajeto". Hoje o Maestro não vê quando um despacho foi engolido — ele depende do humano apontar a tela (#15, MEDIA-ALTA). Este alarme é o que torna a orquestração autônoma observável: "este terminal recebeu a tarefa e não começou".

## ESCOPO
- Projeção pura em `attention.rs`: detecta delivered→Idle→zero-progresso-na-janela → item de atenção com novo `AttentionKind` (ex.: `DispatchSwallowed`/`DeliveredNoProgress`) e `stable_id` do nó (nunca texto/posição). Idempotente no replay (re-computa o mesmo).
- Teste: cenário engolido dispara 1 alarme; turno legítimo curto NÃO dispara; replay reconstrói a fila idêntica.

## RESULTADO ESPERADO (formato exato)
- Diff em `attention.rs` (consumindo o contrato do Maestro); testes de projeção verdes.
- `cargo test -p lina-core` verde (rode isolado se piscar flaky); `clippy -D` 0; `fmt` limpo.
- **NÃO commite.** Reporte o 1º progresso (`lina ask "@Terminal A" "comecei W3 (alarme do engolido)" --intent status`).
- Termine com **`PRONTO: <o AttentionKind criado + o discriminador da janela + testes>`** ou **`BLOCKED: <motivo + o que precisa do contrato>`**.
