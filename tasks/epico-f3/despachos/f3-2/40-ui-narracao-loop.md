# Despacho F3-2 · UI — a tela narra o loop do Maestro (F3-2-7, parte visual)
**Para:** Terminal G · **model·effort:** opus · Medium · **Dono de:** `app/lina-gpui/src/` (goal_card / canvas — sem costura de core)

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (caminho absoluto).
- **LEIA primeiro:**
  1. `tasks/epico-f3/onda-f3-2.md` §gate (h) + spec 52 §"Superfície para o leigo" (linha ~339): a meta como o usuário disse; "o que entendi"; critérios em pt-br claro; barra por iteração; ao escalar, aviso narrado *"Tentei 3 vezes e não fechou sozinho; quer um time mais reforçado ou prefere olhar comigo?"*; confirmar é 1 toque. **Nunca jargão** (`ReviewVerdict`/`root_cause_id`/effort) na tela.
  2. O card que JÁ existe (estenda, não refaça): `app/lina-gpui/src/goal_card.rs` (render em pt-br, teste anti-vazamento de jargão `:546-575`, identidade Fraunces/IBM Plex, terracota — roxo-IA banido) + os botões já wired (`main.rs`): confirm "Sim, é isso" / "Quero ajustar" (editor inline) / recolher / arrastar / fechar.
  3. Doutrina de design da casa: invoque a skill `lina-design-doctrine` (bane slop visual; exige direção estética declarada, tokens semânticos). Compare com o protótipo aprovado a olho (memória: gate verde ≠ design aplicado).
- **Como o estado chega à UI:** `refresh_goals_cache` (`main.rs`) projeta `project_goals` do event log via `try_with_store` (não-bloqueante — fix de freeze recém-commitado pelo Maestro). Você consome a projeção; **não toca core**.

## FUNÇÃO
Você é o dono da **narração visual do loop** no canvas: o leigo vê a meta sendo interpretada, confirma/corrige com 1 toque, acompanha o progresso por iteração e — quando o sistema escala — recebe um aviso humano (não um log técnico).

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `app/lina-gpui/src/` (goal_card/canvas). Zero costura de core (`events.rs`/`router.rs`/`goal.rs` são de outros donos). Precisa de um campo novo na projeção? **Peça ao Maestro.**
- **Zero jargão na tela** (é critério de aceite, com teste anti-vazamento — estenda o teste de `goal_card.rs` para os estados novos). pt-br do leigo.
- **Identidade da casa:** fonte de destaque ≠ corpo; tokens semânticos; terracota como acento; **roxo-IA / gradiente genérico / Inter-por-inércia BANIDOS**. Se for fazer algo novo, declare a direção e banque-a.
- **`reduce-motion` respeitado** em qualquer animação (barra/transição de fase).
- Convenções: `cargo fmt`, `clippy -D` limpo, **token_ratchet intacto** (a catraca conta `FontWeight::`/`px(n)` até em comentário — memória: não escreva a substring literal em comentário). O gate de UI roda a suíte completa do app (memória: filtro de módulo não casa o token_ratchet).

## OBJETIVO (o porquê de negócio)
A inteligência do Lina só "existe" para o fundador quando ele a VÊ acontecer na tela. Esta fatia é o que torna o gate (h) — validação ao vivo — possível: o circuito interpretar→confirmar→trabalhar→escalar→entregar precisa estar legível e bonito, sem uma palavra técnica.

## ESCOPO — F3-2-7 (visual)
- **Interpretação proposta:** o card mostra claramente "o que entendi" + estratégia + critérios em pt-br, com os 2 toques (Sim, é isso / Quero ajustar) — refine o que já existe para o ciclo de correção (re-interpret) ficar óbvio.
- **Estado do loop por iteração:** barra/indicador de progresso por iteração (1ª, 2ª, 3ª tentativa) sem expor `iteration`/`effort` como número técnico — traduza ("ainda trabalhando", "revisando", "tentando de novo").
- **Escalada narrada:** quando `GoalEscalated` (turn_budget_exhausted), o card exibe o aviso humano da spec (2 opções: time reforçado / olhar junto) — sem `GoalEscalated`/`reason` crus.
- **Degradação graciosa:** sem critérios/itens, o card não quebra (já faz — preserve).

## RESULTADO ESPERADO (formato exato)
- Diffs só em `app/lina-gpui/src/`; teste anti-vazamento de jargão estendido aos estados novos.
- `cargo test --manifest-path app/lina-gpui/Cargo.toml` verde (suíte completa — rode e cole a contagem); `clippy -D` 0; `fmt` limpo; token_ratchet intacto.
- **NÃO commite.** Como gpui não roda headless, descreva o que deve aparecer na tela (para o roteiro de validação do fundador) — a confirmação a olho é do fundador no rebuild.
- Reporte o 1º progresso (`lina ask "@Terminal A" "comecei a UI do loop" --intent status`).
- Termine com **`PRONTO: <resumo + o que o fundador deve ver na tela + testes>`** ou **`BLOCKED: <motivo>`**.
