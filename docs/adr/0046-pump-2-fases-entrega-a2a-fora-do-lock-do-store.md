# ADR 0046 — pump em 2 fases: a entrega A2A faseada NÃO segura o lock do `EventStore`

- **Status:** **ACEITO + IMPLEMENTADO (2026-06-24, variante PRAGMÁTICA).** Fase 0 (diferir `submit_delay`) + Fase 1 PRAGMÁTICA (orçamento de prontidão da entrega viva capado + backoff de não-pronto curto) implementadas e validadas — suíte de segurança do router VERDE, retenção/retry/DLQ/settle verdes, app compila. O **split estrutural** (pump 2-fases, descrito abaixo) foi **deferido em favor da variante pragmática** por decisão do fundador: ele elimina o freeze usando a máquina de retry que JÁ existe (ADR 0020), com risco baixo e ZERO cirurgia no `route_message` (dispatcher crítico de anti-forja, 259 call-sites). Fica registrado como refinamento FUTURO opcional, se o hold residual (~40ms/entrega) algum dia incomodar.
- **Onda/Story:** dívida de RAIZ herdada dos dois diagnósticos de travamento (freeze #1 `attention_heartbeat` O(N); freeze #2 `deliver_a2a` segura o store). O follow-up de raiz ficou registrado no §54 do vault (*Diagnóstico de Travamento (Runtime Perf)*) e nunca foi feito — só remendos no lado leitor.
- **Data:** 2026-06-24

## Contexto

O app congela "coincidentemente" quando um terminal roda `lina ask` em resposta a outro. Não é
memória nem deadlock clássico — é **contenção de um mutex global**. Diagnóstico fechado e provado por
código:

1. `MailboxPump::tick` (`app/lina-gpui/src/bridge.rs:1246`) adquire `lock(&self.store)` — lock
   **bloqueante** do `EventStore` — e passa `&mut store` para `Router::pump`, **mantendo o lock
   durante o `pump` inteiro**.
2. `pump` → `route_message` (`router.rs:1430`) chama `deliver(...)` **com o store travado**, e usa o
   store logo antes e logo depois (`mark_busy_after_delivery`, `store.append(MessageDelivered)`).
3. `deliver` → `deliver_a2a` (`a2a.rs`) bloqueia por **`wait_ready` (poll até `ready_timeout`, ~2s,
   quando o alvo está OCUPADO)** + (antes da Fase 0) `submit_delay` ~0,3s.

→ O mutex do `EventStore` fica preso por **até ~2,3s por entrega**. A thread de UI (render, painéis,
custo) e a thread do PTY (`spawn_pump`, que anota tokens/custo/lifecycle a cada saída) disputam o
MESMO lock. Quando dois terminais conversam, o lock fica preso quase continuamente → congela.

**Por que os remendos anteriores não bastaram.** O freeze #2 (2026-06-18) só blindou os **leitores
da UI** (`try_lock` no `AttentionHub::sync`; `NodeManager::try_with_store` em 3 painéis). Isso faz a
UI "pular o frame" quando o store está ocupado, mas **não remove o long-hold** — apenas o tolera. É
enxugar gelo: cada feature nova (F3, F4-WA) reintroduz um `lock(&store)` bloqueante na main-thread
(candidatos vivos: `main.rs:886/1804/6593`, `runtime.rs:130/140/249`), e qualquer um deles volta a
congelar. O próprio código admite o long-hold como *"por design"* (`bridge.rs:2871`). É essa premissa
que este ADR reverte.

**Por que a espera não pode simplesmente ser encurtada.** O teste `deliver_waits_until_ready_then_injects`
(`a2a.rs`) prova que a espera de `wait_ready` é **carga útil**: um alvo que fica pronto em ~60ms DEVE
ser entregue em UMA tentativa, dentro do budget. Cortar o budget globalmente quebra essa semântica
testada (e a defesa anti-texto-colado #22/#23 do `READY_CONFIRMATIONS`). A espera precisa CONTINUAR
existindo — só **não pode acontecer com o lock do store na mão**.

## Decisão

A regra passa a ser: **o lock do `EventStore` só envolve as operações rápidas de log — nunca a
espera/sleep/escrita no PTY da entrega faseada.**

### Fase 0 — diferir o `submit_delay` (JÁ FEITA, fora do seam)

O `submit_delay` (espaçamento AgentText→Enter para a TUI processar a colagem) era um
`std::thread::sleep` **inline** em `deliver_a2a`, na thread que segura o store. Ele foi **movido para
o `serial_writer`** (a thread de escrita por-terminal, que não segura lock compartilhado): o atraso
agora é carregado em `WriteOp::Submit { delay_ms }` e aplicado antes de escrever o `0x0D`. Os dois
ops (AgentText + Submit) são enfileirados sob o `_guard` do PTY antes de soltá-lo, então a **FIFO
serial** preserva o invariante de não-interleave (humano/outro agente nunca entram no meio). Isso tira
~0,3s/entrega de baixo do lock — o hold do caso comum (alvo pronto) cai de ~0,3s para ~10ms.

- Provado por `deliver_does_not_block_caller_on_submit_delay`: RED = a chamada bloqueava **219ms**;
  GREEN = retorna em <80ms, o Enter ainda chega depois do atraso pela thread de escrita.
- Não toca `router.rs`. Invariantes de faseamento/serialização verdes (29 testes a2a + atomicidade).

### Fase 1 — variante PRAGMÁTICA (IMPLEMENTADA, 2026-06-24)

Em vez de reestruturar o `route_message` (4 sites de `deliver`, 259 call-sites, anti-forja), a
entrega VIVA **deixa de bloquear longo sob o lock** usando a máquina de retenção/retry que JÁ existe
(ADR 0020):

- **`INTERACTIVE_READY_BUDGET_MS = 40` (`a2a.rs`):** o `deliver_fn` do app (`bridge.rs`) capa o
  `ready_timeout` da entrega viva em ~40ms (clona o perfil do alvo e aplica `min`). Alvo pronto
  injeta na hora; alvo ocupado retorna `Injected{ready:false}` em ~40ms (não em até 2s) → o
  `route_message` RETÉM (sem strike/DLQ). O lock do `EventStore` é segurado ~40ms/entrega, não ~2,3s.
  Headless/bench/teste seguem com o `ready_timeout` cheio (sem contenção de UI; `deliver_a2a` e o
  teste `deliver_waits_until_ready_then_injects` INTACTOS).
- **`NOT_READY_BACKOFF_MS: 2000 → 150` (`router.rs`):** o throttle longo existia porque "cada
  re-tentativa custava ~`ready_timeout` (2s de poll)". Com a espera viva capada (~40ms), o re-check é
  barato → o backoff pode ser curto: um alvo momentaneamente ocupado assenta em ~150ms (impercebível)
  em vez de 2s. Const exclusiva de não-pronto (separada do failed-retry `backoff_with_jitter`);
  backstop `retention_timeout` (600s → DLQ) inalterado.

**Resultado:** hold do lock ~2,3s → ~40ms; assentamento de alvo ocupado ~150ms. Freeze eliminado
sem tocar o coração anti-forja. Validado: suíte de segurança do router + retenção/retry/DLQ/settle
verdes, clippy `-D` limpo, app compila.

### Fase 1-alt — split estrutural do pump (DEFERIDO, refinamento futuro)

Se algum dia o hold residual (~40ms) ou a latência de ~150ms incomodar, o caminho ideal é
reestruturar a entrega para **decidir-sob-lock → entregar-SEM-lock → gravar-sob-lock**:

- **Sob lock (curto):** `pump` drena a mailbox, roda os sweeps/ledger, decide a rota de cada msg
  (orçamento, binding de root, render do bloco `[LINA::MSG]`). Produz uma lista de
  `PendingDelivery { target, from, block, msg_id, root, hops }`. **Solta o lock.**
- **Lock SOLTO:** roda `deliver_a2a` para cada entrega pendente (`wait_ready` + paste + Submit) — a
  parte que pode bloquear segundos. Coleta os `DeliveryOutcome`.
- **Sob lock (curto, breve):** aplica os desfechos — `mark_busy_after_delivery`,
  `append(MessageDelivered)`, retenção/retry/DLQ.

**Serialização e segurança não regridem:**
- A serialização "uma entrega por terminal de cada vez" vem do `sup.lock_pty(target)` (lock do PTY),
  **não** do lock do store — soltar o store não permite escrita concorrente no mesmo PTY.
- Os carimbos anti-forja (`delivered_root`, `dispatched_at`) e a marcação `Busy` continuam
  **síncronos no commit** (fase 3), na ordem do `now_ms` da decisão. A suíte de segurança do Router
  é critério de aceite — verde antes de fechar.
- A retenção por not-ready (ACHADO-2) e o motor de retry/DLQ (ADR 0020) seguem intactos: o desfecho
  `Injected{ready:false}` é processado na fase de commit como hoje.

## Consequências

- **Positiva:** o `EventStore` deixa de ser segurado durante I/O bloqueante. A UI e a thread do PTY
  conseguem o lock em microssegundos → fim do freeze e dos remendos `try_lock` defensivos (que podem
  ser removidos numa limpeza posterior, sem pressa).
- **Custo de latência:** se a fase de entrega for serial e um alvo OCUPADO consumir o budget de
  `wait_ready` (~2s), as entregas seguintes do mesmo tick esperam. Mitigação: a fase de entrega pode
  ser concorrente por-alvo (cada `deliver_a2a` já é serializado pelo seu próprio `lock_pty`), ou o
  `wait_ready` do caso vivo pode passar a confiar mais no retry de 120ms (decisão da implementação).
- **Risco:** mexe em `Router::pump`/`route_message`, costura de dono único na onda viva. Precisa de
  coordenação (não fazer no meio de um round que toque `router.rs`).

## Alternativas consideradas (e por que NÃO)

- **Cortar o `ready_timeout` globalmente** (wait_ready não-bloqueante puro): quebra
  `deliver_waits_until_ready_then_injects` (settle-within-budget é testado e desejado) e enfraquece o
  anti-flicker #22/#23. Rejeitada — muda semântica de entrega para "consertar" concorrência.
- **Mais remendos `try_lock` nos leitores**: enxugar gelo confirmado (o freeze voltou apesar deles).
  Não ataca a causa; cada feature nova reabre o buraco.
- **Tornar a entrega A2A assíncrona/deferida**: quebraria o entrelaçamento de segurança (o carimbo de
  `delivered_root` e o `Busy` só ocorrem numa entrega real e síncrona). Rejeitada no freeze #2 e
  segue rejeitada — o commit síncrono PÓS-entrega (fase 3) preserva a invariante sem deferir a
  decisão.

## Porta de continuidade

Mantém vivas as âncoras *Event Store* (fonte da verdade; aqui só muda a DISCIPLINA de lock, não o
log) e *Workspace Bus/Supervisor* (a serialização do PTY migra explicitamente para o `lock_pty`, onde
sempre pertenceu). Nenhuma porta fecha; uma premissa equivocada ("segurar o store durante a entrega é
por design") é corrigida.
