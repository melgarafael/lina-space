# Repro #22/#23 — "handoff confirmado ≠ trabalho feito" (investigação read-only · F3-2 QA)

> **Escopo (despacho F2QA):** investigar a família #22/#23 (o gargalo nº1 de confiabilidade da
> orquestração) **sem tocar produção** — `router.rs`/`mailbox.rs` são do CORE (B) nesta rodada; o FIX é
> rodada dedicada. Entregáveis: taxonomia da família, hipótese de causa-raiz com `arquivo:linha`, testes
> de reprodução, recomendação. **Autor:** Terminal R (QA/Red-team). **Não commitar.**

## 1. O sintoma e o impacto

`lina handoff` é **confirmado como entregue** ao terminal-alvo, mas **zero trabalho** e **zero
`DomainEvent` novo** acontecem — a 2ª tentativa idêntica funciona. **5 ocorrências** em terminais
distintos (achados #22/#22b/#23/#23b/#22c, severidade ALTA), incluindo **uma sessão de uso real com
cliente** (#22c: 3 handoffs confirmados → 0 artefatos). É o gargalo nº1 porque quebra a premissa do
loop do Maestro nativo (F3-2): se o despacho não vira trabalho, o outer loop não fecha na vida real.

## 2. Taxonomia — a família tem 3 modos (mesmo sintoma, causas distintas)

| Modo | O que se observa no log | Causa-raiz | Status |
|---|---|---|---|
| **A — entrega FALSA** | `MessageDelivered` logado, mas o texto foi engolido / o Enter chegou cedo | `ready_now`/`wait_ready` aprova um TUI no instante de um **turno fechando** (flicker de prontidão) — `a2a.rs:~192` | **FIX W1 commitado** (`c45b90e`): entrega só injeta com **prontidão CONFIRMADA**; senão RETÉM. Repro vivo: `repro_w4_delivered_sem_trabalho.rs` |
| **B — entrega REAL sem trabalho** | `MessageDelivered` legítimo, nó volta a `Idle` (`end_of_response`) **sem** `TokenUsageReported`/`MessageRouted`/`pty_output` | o **INNER loop do CLI de terceiro** não agiu (recebeu o texto, não trabalhou). **Não há fix de core** — zero LLM no core (inv #1) | **DETECÇÃO W3** (`69aad62`): alarme `DeliveredNoProgress`. CAUSA não-corrigível no core; mitigação = protocolo (§5). Repro novo: `repro_22_23_residual.rs` |
| **C — roteado sem entregar** | `MessageRouted` logado, **sem** `MessageDelivered` **nem** `MessageRetained` subsequente | a msg ficou retida/presa e a re-tentativa não correu (pump não tickou para o nó, ou `.inflight` preso) | parcialmente fechado na r4 (#12/#13/#16); risco residual pós-rebuild (§4.2) |

> A observação ao vivo da F3-2 (ponto de retomada `1f0f4af`) — *"2 `MessageDelivered`-sem-Busy + 3
> `MessageRouted`-sem-Delivered; roster 100% Idle"* — é **modo B (×2) + modo C (×3)**, com o app rodando
> o **binário ANTIGO** (pré-fix W1). O rebuild põe o fix de W1 no ar; este relatório mapeia o que
> permanece DEPOIS do rebuild.

## 3. O que já está coberto (não re-fazer)

- **Modo A** — `a2a.rs` (fix W1, prontidão confirmada). Guarda viva: `repro_w4_delivered_sem_trabalho.rs`
  (`turno_fechando_nao_pode_virar_entregue_via_route_message` + `deliver_a2a_nao_injeta_em_turno_fechando`).
- **Detecção do modo B** — alarme `AttentionKind::DeliveredNoProgress` (`attention.rs:128,744`): arma na
  entrega (`MessageDelivered` → `arm_dispatch`, `attention.rs:407`), abre a janela no retorno a `Idle`
  (`mark_dispatch_idle`, `attention.rs:429`), dispara após `DELIVERED_NO_PROGRESS_WINDOW_MS = 90_000`
  (`attention.rs:99,746`) **sem progresso atribuível**. "Progresso" = `TokenUsageReported`
  (`attention.rs:411`) ∨ `MessageRouted{from}` (`attention.rs:413`) ∨ `NodeStatusChanged{Busy,pty_output}`
  (`attention.rs:420`). Report honesto na tela: `MessageDelivered` exigido p/ dizer "entregue" (`86346a7`).
- **Modo C (parcial, r4)** — #12/#13/#16 fechados: spawn furava o freio (`router.rs:758`), `msg_spawn1st-*`
  quebrava o FIFO do drain (`mailbox.rs:612`), `resolve_check_node` apontava nó morto. 5 testes RED→GREEN.

## 4. Gap residual — hipótese de causa-raiz (`arquivo:linha`)

### 4.1 Modo B (entrega real sem trabalho) — o mais resistente
A entrega é perfeita; o CLI de terceiro não inicia o turno. **Por construção, o core não pode forçar o
CLI a trabalhar** (invariante #1: zero LLM/harness no core — orquestramos CLIs de terceiros). A defesa
correta é **detecção + protocolo**, não um fix de entrega:
- **Detecção** já existe (W3). Janela de 90s (`attention.rs:99`) — calibrável; turnos reais de 24 min já
  foram observados (#19), mas o alarme mede desde o **`Idle`**, não desde a entrega, então um turno longo
  legítimo (Busy) não dispara — `idle_ts == None` fica fora (`attention.rs:742`). Calibração defensável.
- **O que falta** é o **protocolo de orquestração** que CONSOME o alarme: confirmar o **1º `DomainEvent`
  de progresso** de cada worker (não o "ok" do verbo), re-despacho informado 1×, breaker sticky após 2
  falhas → escala ao humano. Isso é trabalho do papel orquestrador (mitigação já documentada na onda
  §Riscos), não de `router.rs`.

### 4.2 Modo C (roteado sem entregar) — re-tentativa da retenção
Quando o alvo está ocupado/não-pronto, o router **RETÉM** em vez de entregar: `RouteOutcome::Retained`
**não dá ack** (re-avaliada no próximo tick — `router.rs:786`), emite `MessageRetained` **1×**
(anti-amplificação A4, `router.rs:392,715`) e cronometra o teto `retention_timeout_ms` (600s,
`router.rs:131,187`); estourou → DLQ (F1-0-7). **A re-tentativa depende do PUMP TICKAR** (`drain_to_inflight`,
`mailbox.rs:540`). Hipótese do "MessageRouted-sem-Delivered" ao vivo:
1. **(mais provável, app antigo)** o fix W1 não estava no ar → a entrega divergia entre `MessageRouted` e
   a confirmação de prontidão; com o rebuild isso vira `MessageRetained` + re-tentativa limpa.
2. **(residual)** se o pump deixa de tickar para um nó retido (eco do #16, *"pump morto após reabrir o
   app"*, fechado na r4 mas sensível a reabertura), a `Retained` nunca re-tenta e a msg fica em
   `MessageRouted` até o teto → DLQ silenciosa. **Vigiar pós-rebuild**: contar `MessageRetained` sem
   `MessageDelivered`/DLQ subsequente dentro de `retention_timeout_ms`.

## 5. Testes de reprodução entregues (read-only, não tocam produção)

- **`crates/lina-core/tests/repro_22_23_residual.rs`** (NOVO, 3 testes verdes):
  - `handoff_real_loga_message_delivered_que_arma_o_vigia` — caminho REAL (`route_message` → entrega
    legítima → `MessageDelivered{to}`), provando que a entrega real é a que arma o vigia W3.
  - `handoff_entregue_que_volta_a_idle_sem_trabalho_e_detectado` — o **sintoma do modo B**: entrega real
    + retorno a `Idle` + zero progresso ⇒ alarme `DeliveredNoProgress` após a janela (silêncio antes).
  - `entrega_que_vira_trabalho_nao_alarma` — o **discriminador**: progresso (`TokenUsageReported`) desarma
    o vigia ⇒ a 2ª tentativa que funciona não gera falso-positivo.
- **`repro_w4_delivered_sem_trabalho.rs`** (já existente, W4) — cobre o modo A.

Juntos, os repros cobrem A (entrega falsa) e B (entrega real sem trabalho) pelo caminho real. O modo C
não ganhou repro determinístico nesta rodada (depende de simular "pump não tickou" — toca o agendamento
do pump, fronteira do CORE); a recomendação §6 é a vigia por contagem no log.

## 6. Recomendação de fix (rodada dedicada — NÃO nesta onda)

1. **Protocolo de orquestração anti-#22/#23 (papel Maestro, não core):** confirmar o **1º `DomainEvent`
   de progresso** de cada worker antes de considerar o despacho aceito (não o ack do verbo); re-despacho
   informado 1×; breaker sticky após 2 falhas → escala ao humano. Carimbar `PRONTO` com **evidência**
   (hash/stat do artefato — o disco é a verdade; lição direta do #22c).
2. **Modo C — vigia de retenção presa:** métrica/alerta `MessageRetained` sem `MessageDelivered`/DLQ
   dentro de `retention_timeout_ms`; garantir que o pump re-tickeia para nós com `retained_since` aberto
   após reabertura do app (re-derivar o #16 no caminho de boot).
3. **Calibração da janela W3** (`DELIVERED_NO_PROGRESS_WINDOW_MS`, hoje 90s): revisitar contra os tempos
   reais por agente quando o dashboard de custo (F1) der p95 — turnos de 24 min (#19) são legítimos e o
   alarme só conta desde o `Idle`, mas vale confirmar com dados.
4. **`loop_detected` cego (#22c):** distinguir cascata-tempestade de **correção legítima de um worker
   travado** — hoje o anti-loop barra o Maestro de redirecionar um worker engolido (4 msgs em ~5min). Fora
   do escopo desta família, mas mesmo gargalo operacional.

## 7. Veredito da frente repro

- **Modo A:** fechado (fix W1 no ar pós-rebuild; guarda viva `repro_w4`).
- **Modo B:** causa **não-corrigível no core** (inv #1); **detectada** (W3) e **reproduzida** end-to-end
  (`repro_22_23_residual.rs`). Fix da causa = protocolo de orquestração (§6.1), rodada dedicada.
- **Modo C:** parcialmente fechado (r4); risco residual de re-tentativa presa pós-reabertura — vigia por
  log recomendada (§6.2), repro determinístico deferido (toca o pump = CORE).

**Nenhuma alteração de produção foi feita.** Só testes (`crates/lina-core/tests/`) e este relatório.
