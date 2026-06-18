# Rodada de Confiabilidade da Orquestração — hardening pré-F3-2 · plano de execução

> **Decisão do fundador (2026-06-18):** *confiabilidade primeiro.* Antes de empilhar o Loop do Maestro Nativo (F3-2), consertar o **gargalo nº1**: handoff confirmado que **não vira trabalho**. F3-2 (cujo gate F3-2-7 é um cenário real na tela) só fecha sobre orquestração confiável — daí esta rodada precede a `onda-f3-2.md` (já desenhada, em espera).
> **Método:** `systematic-debugging` — **reproduzir antes de consertar**. A causa-raiz do bug intermitente é hipótese até haver repro determinístico.
> **Maestro:** Terminal A. **Workers não commitam; o Maestro fixa o contrato, valida de fora (exit codes diretos) e commita por fatia.**

## Objetivo da rodada

Fechar a família de achados de dogfooding **#22/#23** (5 ocorrências, ALTA) — *"`lina handoff` é confirmado como entregue, o terminal vai a Idle (end_of_response), mas zero trabalho e zero `DomainEvent` novo; a 2ª tentativa idêntica funciona"* — e dar ao Maestro a **observabilidade** para enxergar quando um despacho foi engolido. É o substrato de TODA a inteligência de orquestração da Fase 3.

## Diagnóstico-raiz (mapeado no fonte — confirmar com repro antes do fix)

O elo fraco é a **definição de "entregue"**, em três pontos que se somam:

1. **Confirmação fraca no remetente.** `scan_log_outcome` (`crates/lina-bootstrap/src/bin/lina.rs:625`) trata **`MessageRouted` OU `MessageDelivered`** como sucesso, e "sucesso vence bloqueio" (`:636-643`). Mas `MessageRouted` é gravado **antes** da injeção física no PTY (`router.rs:1168`). → o agente vê "entregue" mesmo quando a injeção foi um no-op.
2. **`ready_now` pode julgar PRONTO um TUI fechando o turno** (`crates/lina-core/src/a2a.rs:192`): o prompt-regex casa e nenhum `busy_marker` aparece, então injeta `AgentText`+`Submit`; se o CLI estava no fim de um turno, o paste é engolido / o Enter chega cedo. O nó então vai `Busy→Idle` por `on_end_of_response` do turno **anterior** (`lifecycle.rs:309`) — **nada de novo**. A 2ª tentativa casa um instante limpo. **Hipótese mais forte.** Fronteira: `a2a.rs` (`ready_now`/`wait_ready`/`submit_delay`) + perfil `profiles/claude-code.toml` (`prompt_ready_regex`/`busy_markers`).
3. **Ponto-cego de observabilidade.** O stall detector só corre em **`Busy`** (`lifecycle.rs:277`; ADR 0019 §4 explícito). No #22/#23 o nó **volta a `Idle`** → o relógio de stall **nunca corre**, e como não há `DomainEvent` novo, **nada detecta**. O ADR 0019 cobre "Busy travado", não "delivered → Idle sem trabalho".

Correlatos da mesma família: **#4/#14/#23c** (`resolve_check_node` `lina.rs:454` pode resolver nome→nó MORTO de sessão antiga; UUID cru na superfície); **#17 residual** (override de env aplicado só a `terminal_name`; demais campos ainda do `bootstrap.json` do cwd compartilhado).

## ⛔ Rα-0 (Maestro/A) — setup (largada IMEDIATA)

1. **Árvore limpa:** fix de freeze do render commitado (`24ad732`, `try_with_store`). ✅ feito.
2. **Sem contrato a fixar:** decisão de arquiteto — o alarme de W3 é **projeção PURA derivada** dos eventos existentes (`MessageDelivered` + `NodeStatusChanged(Idle)` + ausência de `DomainEvent`), **sem evento novo** (segue o padrão das filas de atenção; menos acoplamento). Nenhuma frente espera contrato → **largada imediata**. Se W3 precisar de evento durável, pede ao Maestro (que então fixa `events.rs`).

## DAG executável e frentes (fronteiras DISJUNTAS — o Explore mapeou)

```
[Rα-0: fix-freeze ✅ + sem contrato a fixar → largada imediata (A)]
        │
        ├─► W4 REPRO+QA (R)   tests/ ───────────────┐  (começa JUNTO com B — repro antes do fix)
        ├─► W1 ENTREGA (B·Ultra) a2a.rs+router.rs+bridge.rs ─┤  o coração: causa-raiz #22/#23
        ├─► W2 BIN (H)        lina.rs ───────────────┤  "entregue de fato" + resolve nó-morto + #17
        ├─► W3 OBSERV. (I)    attention.rs ──────────┤  alarme "despacho engolido" (fecha o ponto-cego)
        └─► UI (G)            main.rs/goal_card.rs ──┘  estado honesto na tela (consome W3)
                │
                ▼
   A: consolida o diagnóstico → valida o fix contra o repro de R → gate (4 lentes) → commit por fatia
```

| Frente | Terminal | Toca (dono único) | Entrega |
|---|---|---|---|
| **W1 · ENTREGA** | B · Ultra | `crates/lina-core/src/a2a.rs` + `router.rs` (route_message/retention/deliver + emissão de `MessageDelivered`) + `app/lina-gpui/src/bridge.rs` (`deliver_fn`/meter/transições) | endurecer `ready_now`/`submit_delay` p/ não injetar em turno fechando; `MessageDelivered` só após injeção REAL (ready:true+submit), nunca pré-injeção; fechar a janela meter×submit_delay |
| **W2 · BIN** | H | `crates/lina-bootstrap/src/bin/lina.rs` | report do remetente exige **`MessageDelivered`** (não `MessageRouted`) p/ dizer "entregue" (mata o falso-entregue); `resolve_check_node` recusa nó MORTO (#4/#14/#23c); auditar/fechar #17 residual (campos além do nome reféns do cwd) |
| **W3 · OBSERV.** | I | `crates/lina-core/src/attention.rs` (projeção pura derivada) | cruza `MessageDelivered{to}` + retorno a `Idle` + **zero `DomainEvent` novo atribuível** numa janela → item de atenção com novo `AttentionKind` ("recebeu, não começou"). Fecha o ponto-cego do ADR 0019. Sem evento novo |
| **UI · TELA** | G | `app/lina-gpui/src/main.rs` + `goal_card.rs` (NÃO `bridge.rs` — é de B) | badge honesto "recebeu a tarefa, ainda não começou" (consome `AttentionKind` de W3); `circuit_breaker` legível (#21: "pausado por segurança — clique p/ liberar"); UUID cru → `@Nome` na superfície (#4) |
| **W4 · REPRO+QA** | R · High | `crates/lina-core/tests/` + `#[cfg(test)]` | **reproduzir** o "delivered-sem-trabalho" num teste determinístico (o santo graal — B valida o fix contra ele); regressão do caminho de A2A; **suíte de segurança do router verde** (0 ALTA) |

> **Regra de costura:** `events.rs` fixo no Rα-0 (A). Donos únicos: `a2a.rs`+`router.rs`+`bridge.rs`=B · `lina.rs`=H · `attention.rs`=I · `main.rs`+`goal_card.rs`=G · `tests/`=R. Precisa de algo na costura de peer? **Peça ao Maestro.** `cargo fmt -p` só nos próprios arquivos.

## Sequência (systematic-debugging)

1. **R + B começam JUNTOS:** R persegue o repro determinístico do sintoma; B instrumenta/lê o caminho. **Sem repro confirmado, B não "conserta no escuro".** Confirmada a causa (hipótese 2 acima é a aposta), B endurece `a2a.rs`/`router.rs` e valida contra o teste de R.
2. **H, I, G em paralelo** desde o contrato (causas conhecidas / disjuntos): H ataca o falso-entregue (impacto imediato), I o alarme, G a tela.
3. **A consolida e valida de fora** (exit codes diretos), commita por fatia.

## GATE DE SAÍDA — roda e se mede

(a) **Repro→fix provado:** existe um teste que reproduzia "delivered-sem-trabalho" (RED) e fica **GREEN** após o fix de B; o caminho real (`route_message`→`deliver_a2a`) é exercido (não monta-à-mão).
(b) **"Entregue" = injeção real:** `MessageDelivered` só aparece no log após injeção `ready:true`+submit; um `Injected{ready:false}` **não** vira "entregue"; o report do `lina ask`/`handoff` (H) só diz "entregue" com `MessageDelivered`.
(c) **Alarme do engolido:** delivered + retorno a Idle + zero `DomainEvent` novo na janela → `DeliveryStalled` 1× + item na fila de atenção; o Maestro **vê** "recebeu, não começou" sem perguntar ao humano (fecha #15/#22).
(d) **Identidade no A2A:** `resolve_check_node` não reporta nó MORTO de sessão antiga (#4/#14/#23c); #17 residual auditado e fechado ou registrado com porta.
(e) **Tela honesta:** badge "recebeu, não começou"; `circuit_breaker` legível; zero UUID cru na superfície.
(f) **Segurança intacta:** suíte do router **verde**, **0 ALTA**; nenhum campo de payload virou autoridade; aditividade preservada (replay F0/F1/F2).
**(g) Validação na tela do fundador** (badge + estado legível) — preparada; fechada no rebuild com o fundador.

## Riscos

- **🔴 Meta-risco (dogfooding):** despachar via `lina handoff` para 5 terminais PODE ser mordido pelo próprio bug que conserta. *Vira alavanca:* cada engolimento observado AO VIVO é evidência de repro para B/R. *Mitigação do Maestro:* confirmar o **1º evento de progresso** de cada worker (não o "ok" do verbo — lição da race `ask-ok-cego`); re-despacho informado 1×; se engolir 2×, **eu (Maestro) assumo a fatia** em vez de insistir; tudo vira nota para o diagnóstico.
- **Fix no escuro** (consertar hipótese sem repro) → systematic-debugging: R reproduz primeiro; B só mexe na causa confirmada.
- **Tocar `deliver_a2a`/`Router` afrouxar segurança** → suíte do router verde é critério implícito de W1/W2 (R prova por mutação).
- **Flaky em paralelo** (install/discovery/history — memória) → rodar isolado/serial para desambiguar antes de gritar regressão.
- **Endurecer `ready_now` demais** (passar a reter entrega legítima → latência) → medir: a 1ª tentativa passa a entregar onde antes engolia, sem regredir `delivery_ready_delivers_once`.

## Pendências relacionadas

- **F3-2 (Loop do Maestro Nativo)** — desenhada em `onda-f3-2.md` + despachos `despachos/f3-2/`; **próxima rodada**, sobre esta fundação.
- **Fora do escopo:** #25 ENOSPC → F3-5-8; F3-0-6 badge effort / F3-0-7 Teste C → encaixe oportunista.

---

## STATUS DA EXECUÇÃO — 2026-06-18

### Decisões registradas
- **#21 (circuit_breaker legível) DEFERIDO com mapa** — o G descobriu que fechá-lo exige o core surfaçar `Blocked+reason` até a UI: hoje `bridge.rs:2279` colapsa `Blocked→Busy` e `lina_host::NodeStatus` **não tem `Blocked` nem `reason`** (a info morre antes da tela; canvas mostra "rodando" para nó pausado). Mexer em `lina_host::NodeStatus` toca a **fronteira `UiHost`** (âncora de continuidade) → NÃO no impulso desta rodada. A parte de UI do G (consumidor + copy "pausado por segurança — clique para liberar") está PRONTA e testada, esperando. **Porta:** fatia dedicada (provável mini-ADR) = `bridge.rs:2279` (não colapsar Blocked) + `lina_host::NodeStatus{Blocked, reason}` + verbo de liberação no core.
- **Costuras de teste aprovadas:** H tocou `handoff_cli.rs`/`params_cli.rs` (`.env_remove("LINA_AUTONOMY")`, espelha ADR 0026); G tocou `attention_ui.rs` (consome o `AttentionKind` novo de I — UI do W3, disjunto do core). Ambas sinalizadas e legítimas.

### Progresso das fatias
- **W1 (B):** PRONTO. Causa-raiz confirmada (flicker de 1 frame no fechamento de turno); fix `READY_CONFIRMATIONS=2` em `a2a.rs` (19+/1-); item 4 fechado pela raiz (sem fix no escuro); repro de R RED→GREEN. Validação de fora: em curso.
- **W2 (H):** PRONTO. `MessageDelivered` exigido no report; `resolve_check_node` anti-nó-morto; #17 fechado (autonomia env-first). Flag de costura aprovada.
- **UI (G):** PRONTO. Jargão pt-br; badge "recebeu, não começou" fiado ao contrato de I; #4 UUID limpo. #21 deferido (acima).
- **W4 (R):** repro RED→GREEN entregue a B; finalizando mutação de segurança em worktree isolado.
- **W3 (I):** variant `AttentionKind::DeliveredNoProgress` publicado (G já integrou); ficou **Blocked em permission_prompt** aguardando aval humano (gate ADR 0021/0025) — o fundador aprovou; I destravou. Achado meta: o Maestro deveria poder ver/aprovar o pedido de um worker (extensão do #15 + do alarme do W3).

### GATE DE CÓDIGO — FECHADO ✅ (2026-06-18)
- **Validação de fora (exit codes diretos):** lina-core 399/0 · lina-bootstrap ~156/0 · app 551/0 (token_ratchet intacto) · clippy `-D` 0 nos 3 · fmt limpo. Repro de R **RED→GREEN** confirmado de fora (`repro_w4_delivered_sem_trabalho` 2/2).
- **Revisão CEGA (revisor isolado, sem contexto do autor):** **LIBERAR PARA COMMIT** — 0 ALTA, 0 MÉDIA, 5 fatias PASS; rodou os testes (não só leu); regra-mãe de segurança intacta (identidade server-side; payload de agente nunca autoridade); fix resolve sem regredir nem criar lock permanente; report não estagna; alarme discrimina progresso real.
- **Commitado por fatia:** `c45b90e` W1 (a2a flicker) · `86346a7` W2 (report honesto + check no vivo + #17) · `69aad62` W3+UI (alarme + tela honesta) · `3e880f2` W4 (repro). Setup: `24ad732` (fix freeze).
- **PENDENTE (gate g, BLOQUEANTE, diferido ao fundador):** validação na TELA (badge âmbar "recebeu, não começou" + cards pt-br) — precisa do Lina.app rebuildado + olho do fundador. gpui não roda headless.
- **Deferido com porta:** #21 (Blocked→Busy colapsado em `bridge.rs:2279` + `lina_host::NodeStatus` sem Blocked/reason — toca fronteira `UiHost`) → fatia dedicada.
