# Despacho · UI — a tela conta a verdade do estado (#22, #21, #4)
**Para:** Terminal G · **model·effort:** opus · Medium · **Dono de:** `app/lina-gpui/src/main.rs` + `goal_card.rs` (UI — NÃO `bridge.rs`, que é de B)

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (absoluto).
- **LEIA primeiro:** `tasks/epico-f3/rodada-confiabilidade-orquestracao.md` (gate (e)). Carregue a skill `lina-design-doctrine`.
- **O que mostrar (3 estados ilegíveis hoje):**
  1. **"Recebeu, não começou"** — consome o `AttentionKind` novo que o Terminal I (W3) cria (`attention.rs`): um terminal que recebeu o despacho e não produziu trabalho deve ter um badge honesto, não aparecer como Idle comum (achado #22; o fundador olhou a tela e concluiu errado).
  2. **`circuit_breaker` legível** (achado #21): um terminal preso em `Blocked(circuit_breaker)` hoje mostra jargão; deve dizer "pausado por segurança — clique para liberar" (estado humano + caminho de recuperação).
  3. **UUID cru → `@Nome`** (achado #4): qualquer superfície que ainda exiba `019eb26f-…` deve resolver para o nome do colega (zero jargão, inv #6).
- **Como o estado chega à UI:** projeções do event log via `try_with_store` (não-bloqueante, fix recém-commitado `24ad732`). Você CONSOME a projeção/fila de atenção; não toca core.

## FUNÇÃO
Você é o **dono da legibilidade do estado** no canvas. O fundador precisa LER a verdade na tela: quem recebeu e não começou, quem está pausado e como soltar, quem é quem (nome, não UUID).

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `app/lina-gpui/src/main.rs` e `goal_card.rs` (e módulos de UI próprios). **NÃO toque `bridge.rs`** (é de B/W1) nem core. Precisa de um campo na projeção/fila? **Peça ao Maestro** (coordene o `AttentionKind` com o Terminal I).
- **Zero jargão na tela** (`circuit_breaker`/`DeliveryStalled`/UUID/`AttentionKind`): pt-br do leigo, com teste anti-vazamento (estenda o de `goal_card.rs:546-575`).
- **Identidade da casa:** tokens semânticos; terracota como acento; **roxo-IA / gradiente genérico / Inter-por-inércia BANIDOS**; `reduce-motion` respeitado.
- Convenções: `cargo fmt`, `clippy -D` 0, **token_ratchet intacto** (conta `FontWeight::`/`px(n)` até em comentário — não escreva a substring literal em comentário). O gate de UI roda a suíte completa do app.

## OBJETIVO (o porquê de negócio)
O fundador diagnostica o time pela tela. Quando ele vê "Idle" num terminal que na verdade engoliu um despacho, ou jargão "circuit_breaker" sem saída, a confiança quebra. A tela honesta é o que fecha o laço de observabilidade do lado humano.

## ESCOPO
- Badge/estado "recebeu a tarefa, ainda não começou" alimentado pelo `AttentionKind` de W3 (comece pela parte independente; integre o kind quando I publicar o contrato).
- `circuit_breaker` → texto humano + affordance de liberar (1 clique, gesto humano — reuse o canal `human_intent`/ADR 0036 se aplicável; confirme com o Maestro).
- Varredura: nenhum UUID cru em superfície visível → `@Nome`.

## RESULTADO ESPERADO (formato exato)
- Diffs em `main.rs`/`goal_card.rs`; teste anti-vazamento de jargão estendido aos estados novos.
- `cargo test --manifest-path app/lina-gpui/Cargo.toml` verde (suíte completa; cole a contagem); `clippy -D` 0; `fmt` limpo; token_ratchet intacto.
- **NÃO commite.** gpui não roda headless: descreva o que o fundador deve ver na tela (para o roteiro de validação).
- Reporte o 1º progresso (`lina ask "@Terminal A" "comecei a UI de estado honesto" --intent status`).
- Termine com **`PRONTO: <o que muda na tela + testes>`** ou **`BLOCKED: <motivo>`**.
