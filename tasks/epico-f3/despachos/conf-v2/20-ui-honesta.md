# Despacho F3-CONF-2 · UI-HONESTA — Terminal G · model opus · effort Medium

> **Antes de codar:** rode `lina plan read CONF-UI` e LEIA `tasks/epico-f3/onda-f3-conf-2.md` + o achado **#21** em `tasks/despachos/achados-dogfooding-sessao.md` (seu próprio diagnóstico anterior do circuit_breaker invisível) + o achado F3-0-6 (badge). O contrato do `reason` já está COMMITADO pelo Maestro — parta dele.

## 1. CONTEXTO (onde o trabalho está)

`cwd`: raiz do repo. Você é **dono único** de `app/lina-gpui/src/{bridge.rs, canvas.rs, main.rs, dashboard.rs}` nesta rodada. Duas entregas, ambas só de PROJEÇÃO/RENDER (zero costura de core):

**(A) circuit_breaker LEGÍVEL (#21).** Você mesmo mapeou: hoje o estado mais crítico ("pausei por segurança") aparece como âmbar **"trabalhando"** — pior que jargão, é mentira. As 3 perdas do reason (já corrigidas no core pelo contrato do Maestro): o Bus agora carrega `reason`, e `lina_host::NodeStatus` agora tem `Blocked { reason: String }`. Os pontos seus:
- `app/lina-gpui/src/bridge.rs:2271-2282`: `map_status` **COLAPSA** `CoreStatus::Blocked → NodeStatus::Busy` (`:2279`) — pare de colapsar, mapeie para `NodeStatus::Blocked{reason}`. `bus_to_host` (`:2286-2306`, ramo `:2293-2296`) precisa **propagar o reason** (agora disponível no BusEvent — confira a assinatura com o Maestro).
- `app/lina-gpui/src/main.rs`: `status_dot` (`:4165-4174`, hoje Busy→âmbar) ganha braço `Blocked` (cor distinta de "trabalhando"); `node_status_label` (`:5889`) / `canvas.rs:aggregate_badge` (`:148`) ganham o badge "pausado". **A copy JÁ EXISTE e está testada**: `CIRCUIT_BREAKER_LABEL = "pausado por segurança — clique para liberar"` (`main.rs:6250`, `#[cfg(test)]`) — **promova para produção** e ligue ao render. O teste `new_state_copies_are_human_and_actionable` (`main.rs:6254`) exige conter "segurança" + "liberar" + zero jargão (`STATE_JARGON` em `main.rs:6162` proíbe "blocked"/"circuit"/"breaker").
- **"clique para liberar":** ligue o gesto humano (botão/clique no card) ao **verbo de reset do breaker** que o CORE (B) está criando — coordene a assinatura: `lina ask "@Terminal B - Effort: Ultra Code" "qual a assinatura do reset do breaker p/ eu ligar o botão liberar?" --intent ask`. O reset é gesto HUMANO (server-side `by`) — você só dispara o intent, igual ao `confirm_goal`/`human_intent` que você já fez na F3-1 (`5a999af`, fila view→pump no `NodeManager`).

**(B) Badge modelo·effort no header do card (F3-0-6).** A projeção effort-por-nó **já existe**: `dashboard::effort_badges(records) -> BTreeMap<String, EffortBadge>` (`dashboard.rs:248`); `EffortBadge::surface_text()` produz `"caprichoso · claude-opus-4-8"` (`dashboard.rs:199-242`); a tradução pt-br `rápido/equilibrado/caprichoso` em `effort_surface_label` (`dashboard.rs:174`). Hoje o badge só aparece no PAINEL de dashboard (`main.rs:2390-2517`, pílula em `:2506`), **NÃO no header do card do canvas**. O header do card é montado em `main.rs:4177` (`title = div()...` mostra dot + nome + "· kind · status" + chips). **Adicione a pílula modelo·effort ao header do card**, reusando o cache `effort_badges` (passe o mapa ao laço de cards, ~`main.rs:4140`).

## 2. FUNÇÃO

Você é o **dono da tela honesta**. Sua régua: o card nunca mente sobre o estado, e o leigo entende sem jargão. Você já tem o gosto e a copy — falta ligar o sinal que agora chega do core.

## 3. DIRECIONAMENTO

- **Mexa SÓ em** `app/lina-gpui/src/{bridge.rs,canvas.rs,main.rs,dashboard.rs}`. Precisa de algo no core/host? **PEÇA AO MAESTRO** — não toque `crates/`.
- **Doutrina visual (lina-design-doctrine):** o badge "pausado por segurança" precisa de cor própria (não o âmbar de "trabalhando", não o vermelho de "morto") — algo que leia "pausa/atenção". Sem jargão na superfície (o `STATE_JARGON` te pega). Reuse os tokens semânticos do `theme.rs`; zero literal de cor solto. A catraca de tokens (`token_ratchet`) conta `FontWeight::`/`px(n)` ATÉ em comentário (achado F2-2-1) — não escreva a substring literal em doc-comment.
- **token_ratchet intacto** (o gate de UI roda a suíte completa, não só o seu módulo — memória *validar fatia de UI inclui token_ratchet*).
- `reduce-motion` respeitado se houver qualquer transição no badge.

## 4. OBJETIVO

O fundador olhou um terminal pausado pelo breaker e concluiu "já concluiu" (#21) — o estado ilegível custou um worker perdido + leitura humana errada. E ele não enxerga qual IA/quanto-esforço cada terminal está usando (F3-0-6, doc-fonte 57). A tela tem que contar a verdade do time num relance: quem trabalha, quem pausou e por quê (com saída de 1 clique), e com que motor cada um pensa.

## 5. RESULTADO ESPERADO (formato + marcador)

Diffs em `app/lina-gpui/src/` + testes verdes:
1. Nó pausado pelo breaker → card mostra "pausado por segurança — clique para liberar" (cor própria, não âmbar); `honest_state_tests` verde; o clique dispara o reset do core. (gate **c**, metade UI)
2. Header do card do canvas mostra `opus · caprichoso` (modelo·effort pt-br). (gate **d**)

Valide de fora: `cargo test --manifest-path app/lina-gpui/Cargo.toml` (suíte completa — inclui token_ratchet) + `cargo clippy --manifest-path app/lina-gpui/Cargo.toml --all-targets -D warnings` + `cargo fmt --manifest-path app/lina-gpui/Cargo.toml --check`. **NÃO commite** — reporte exits ao Maestro.

Termine com `PRONTO: <o que entregou + exits + se o botão liberar já liga no verbo de B ou está stub aguardando a assinatura>` ou `BLOCKED: <o que falta>`. Reporte o **1º progresso** ao Maestro ao começar (`lina ask "@Terminal A" "comecei a UI-HONESTA" --intent status`).
