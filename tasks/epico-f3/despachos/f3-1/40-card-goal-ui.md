# Despacho F3-1 · UI (card da Goal) → Terminal G

## CONTEXTO
Rodada F3-1, épico 39. A Goal precisa ganhar **rosto na tela do leigo**. **Fonte:** spec 52 §"Superfície para o leigo" (l.339-341) + invariante #6 (não-técnico-first). LEIA. Consome a projeção `Goal` (`goal.rs`, fatia CORE-Goal) e o **design system da F2** (tokens, header de card, modais).

## FUNÇÃO
Dev Frontend. effort **Medium**.

## FRONTEIRA (só este)
`app/lina-gpui/src/` — novo módulo `goal_card.rs` + wiring no canvas. **NÃO toque** `crates/` (core).

## DIRECIONAMENTO [F3-1-7]
O card mostra, em **pt-br simples**:
- A **meta como o usuário disse** (`statement`).
- **"O que entendi"** (`interpretation`) — o Maestro devolvendo o entendimento.
- Os **critérios em linguagem clara** ("a página abre sem erro", "o teste passa") — `acceptance` humanizado.
- **Barra de progresso por iteração**.
- Ao escalar: **aviso narrado** ("Tentei 3 vezes e não fechou sozinho; quer um time mais reforçado ou prefere olhar comigo?").
- **Confirmar a interpretação = 1 toque** (sim / corrigir).

**NUNCA jargão na superfície** (`ReviewVerdict`/`root_cause_id`/`effort`/`goal_id`). **Identidade visual = design system F2** — tokens nomeados, nada de slop (gradiente roxo, glassmorphism genérico, Inter por inércia). **Compare a olho com o protótipo aprovado** antes de dizer "pronto" (memória: *gate verde não prova design aplicado*).

## OBJETIVO
A Goal ganha um card que um leigo entende e confirma com 1 toque, na identidade da casa.

## RESULTADO ESPERADO
- Card renderiza meta + interpretação + critérios + barra de iteração; confirmar é 1 toque; ao escalar, aviso narrado em pt-br.
- `cargo test --manifest-path app/lina-gpui/Cargo.toml` + `cargo clippy` do app + `fmt` verdes (memória: *validar app inclui cargo test do manifest do app*, não só clippy; e a catraca de token/a11y).
- **Pendência de TELA do fundador é parte do gate (h)** — prepare para a sessão de tela.
- **NÃO commite** — reporte `PRONTO`/`BLOCKED` ao Maestro @Terminal A.
- **DEP:** a projeção `Goal` (campos `statement`/`interpretation`/`phase`/`acceptance`/`items`/`iterations`) vem do contrato; enquanto o wiring real não fecha, renderize contra a struct `Goal` com dados mockados. O Maestro liga na integração.
