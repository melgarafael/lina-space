# Despacho · W1 ENTREGA — a causa-raiz do handoff que não vira trabalho (#22/#23)
**Para:** Terminal B (BACKEND · Ultra Code) · **model·effort:** opus · High (Ultra) · **Dono de:** `crates/lina-core/src/a2a.rs` + `crates/lina-core/src/router.rs` (caminho de entrega) + `app/lina-gpui/src/bridge.rs` (`deliver_fn`/meter)

## CONTEXTO (puxe antes de tocar código)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (absoluto).
- **LEIA primeiro:** `tasks/epico-f3/rodada-confiabilidade-orquestracao.md` (a peça — diagnóstico-raiz + fronteiras + gate). É a tradução do bug que o fundador viu falhar 5× ao vivo.
- **Método OBRIGATÓRIO:** carregue a skill `systematic-debugging`. **Reproduza ANTES de consertar** — o Terminal R está perseguindo um teste de repro determinístico EM PARALELO; sincronize com ele (`lina ask "@Terminal R"`). NÃO conserte hipótese no escuro.
- **O caminho de entrega (file:line — já mapeado):**
  - `lina ask/handoff` → enfileira `outbox/<nó>/<id>.json` (`bin/lina.rs:278`→`mailbox.rs:479`).
  - pump (app): `bridge.rs:987` (`tick`)→`router.rs:562` (`Router::pump`)→`route_message` (`router.rs:810`).
  - `route_message`: persiste `MessageRouted` em `:1168` (**ANTES** da injeção física!); loop de entrega `:1242` chama `deliver(...)` → app `bridge.rs:903` (`deliver_fn`) → `a2a.rs:311` (`deliver_a2a`).
  - `deliver_a2a`: `wait_ready` (`a2a.rs:196`, usa `ready_now` `:192`) → timeout devolve `Injected{ready:false}` **sem escrever** (`:343-348`); senão `lock_pty`→`AgentText`→`sleep(submit_delay)` (`:359`)→`Submit` (`:360`).
  - retorno a Idle: `lifecycle.rs:309` (`on_end_of_response`), disparado pelo medidor `bridge.rs:5619-5644` (`meter.poll_finished_turns`).
- **Hipótese mais forte (confirme):** `ready_now` julga PRONTO um TUI que está FECHANDO o turno → injeta `AgentText`+`Submit` engolidos → o nó vai `Busy→Idle` por `on_end_of_response` do turno ANTERIOR → zero trabalho, zero DomainEvent. 2ª tentativa casa instante limpo. Mais: `MessageRouted` (pré-injeção) é tratado como "entregue" (o report do remetente é de H/W2 — você cuida da EMISSÃO de `MessageDelivered`).

## FUNÇÃO
Você é o **dono do caminho de entrega**. Faz a injeção no PTY só acontecer quando o terminal REALMENTE aceita input, e garante que `MessageDelivered` signifique "injeção real ocorreu" — nunca `MessageRouted` pré-injeção.

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `a2a.rs`, `router.rs` (caminho de entrega/retenção/emissão de MessageDelivered) e `bridge.rs` (`deliver_fn`/meter/transições). Perfil `profiles/claude-code.toml` (`prompt_ready_regex`/`busy_markers`/`submit_delay`) você pode ajustar — avise o Maestro. Precisa de `events.rs`/`lina.rs`/`attention.rs`? **Peça ao Maestro** (são de peers).
- **Segurança soberana:** `deliver_a2a`/`Router` é o caminho mais sensível. Nenhum campo de payload (`from`/contrato) decide identidade/ordem/autorização — a auth de duas camadas é a única autoridade. A suíte de segurança do router (`delegation_budget_is_enforced_per_root_cause`, `spawn_cascade_requires_human_gate`) **tem que seguir verde** (R prova por mutação).
- **Não regredir:** `delivery_ready_delivers_once` (`router.rs:6648`), `delivery_not_ready_is_retained_not_failed` (`:6591`), `retencao_alvo_busy_nao_injeta_e_entrega_no_idle` (`:4782`) seguem verdes. Endurecer `ready_now` **não pode** virar retenção de entrega legítima (mede: 1ª tentativa entrega onde antes engolia, sem latência nova).
- `cargo fmt -p` só seus arquivos; `clippy -D` 0; sem `unwrap()` em produção; testes via `route_message`/`deliver_a2a` (caminho real).

## OBJETIVO (o porquê de negócio)
O coração do produto é "o time coopera sem fios". Se um handoff confirmado não vira trabalho (e o fundador viu isso quebrar 5× ao vivo, inclusive numa entrega real a cliente), a orquestração automática não existe. Esta fatia restaura a confiança de que delegar = trabalho feito.

## ESCOPO
1. **Com R, reproduza** o "delivered-sem-trabalho" determinístico. Confirme a causa (provavelmente `ready_now` em turno fechando).
2. **Endurecer a prontidão:** `ready_now`/`wait_ready`/`submit_delay` não injetam quando o CLI está fechando turno (ex.: exigir janela de quietude pós-prompt, ou marcador de turno-aberto, conforme o repro mostrar). Ajuste o perfil se for o caso.
3. **`MessageDelivered` = injeção real:** garanta que só é emitido após `ready:true`+submit; `Injected{ready:false}` nunca conta como entregue.
4. **Janela meter×submit_delay:** feche a corrida em que `meter.poll_finished_turns` declara fim-de-turno logo após a injeção (turno injetado que nunca começou).

## RESULTADO ESPERADO (formato exato)
- Diffs em `a2a.rs`/`router.rs`/`bridge.rs` (+ perfil se tocado); o teste de repro de R passa de RED→GREEN com seu fix.
- `cargo test -p lina-core` e `cargo test --manifest-path app/lina-gpui/Cargo.toml` verdes (rode isolado se piscar flaky; cole a contagem); suíte de segurança do router verde; `clippy -D` 0; `fmt` limpo.
- **NÃO commite.** Reporte o 1º progresso ao Maestro (`lina ask "@Terminal A" "comecei W1, sincronizando repro com R" --intent status`).
- Termine com **`PRONTO: <causa-raiz confirmada + o que mudou + RED→GREEN do repro + contagens>`** ou **`BLOCKED: <motivo + o que precisa>`**.
