# DESPACHO — Especialista em Telas · 3 bugs da Área de Poderes/Visual (fundador) · id: f2-4-ui-r3

> Carregue `senior-frontend` + `lina-design-doctrine`. Worker NÃO commita — Maestro aplica o diff de main.rs e valida.

## CONTEXTO (3 bugs do fundador na tela, textual)
A Área de Poderes "ficou bom" (r2 aprovada!). Mas testando, 3 bugs:
1. **"Não tem como fechar, nem com Esc."** — a Área de Poderes (overlay `inset_0`) cobre a topbar
   (o botão "Poderes" fica atrás e não fecha); não há X, nem Esc, nem clique-fora.
2. **"O modal de visual, faça a mesma coisa."** — a galeria de Direções Visuais ainda é um painel
   LATERAL (não a área central que a de Poderes virou). Aplique o MESMO padrão.
3. **"O modal de visual não está funcional, os botões 'usar esta' não funcionam."** — clicar "Usar esta"
   só chama `select(idx)`; o botão continua escrito "Usar esta" (`CHOOSE_LABEL` fixo), sem feedback claro
   → parece morto.

## DIAGNÓSTICO (causa raiz — confirmada no código, não chute)
Os 3 são o mesmo buraco: **um overlay que parece modal mas não tem os contratos de modal** (backdrop,
saída óbvia, occlusão, estado de resultado). Pontos:
- `render_powers_panel` (main.rs): overlay `absolute().inset_0()` transparente → cobre a topbar, sem saída.
- `render_design_directions` (main.rs): ainda `top_16().right_0()` lateral (não overlay central).
- `design_directions.rs`: `CHOOSE_LABEL="Usar esta"` (`:298`) é fixo; `DirectionCard.selected` (`:374`)
  existe mas o botão não reflete o estado escolhido.
- **`theme.rs` TEM troca de tema runtime** (`static ACTIVE: RwLock<Theme>` `:768` + setters) — mas
  aplicar uma direção exige mapear o `DESIGN.md`→tokens de cada uma (feature à parte; ver §Escopo do #3).

## FUNÇÃO
Refatora `app/lina-gpui/src/powers_panel.rs` (X de fechar) + `app/lina-gpui/src/design_directions.rs`
(área central como Poderes + X + estado "Em uso"). A costura `main.rs` (Esc, backdrop clicável, render
central da galeria, handlers) é do Maestro — entregue como **diff**; eu aplico.

## DIRECIONAMENTO — os 3 consertos

### Bug 1 — FECHAR (ambos os painéis), 4 contratos de modal:
- **Backdrop escuro com FOCO**: o overlay ganha um fundo escurecido (separa do canvas). Precisa de um
  **token de scrim** novo em `theme.rs` (ex.: `SurfaceTokens.scrim` — `theme.rs` é isento do ratchet, então
  pode ter o valor; defina-o para os 2 modos, alpha que escureça sem apagar). Sem hex solto na UI.
- **X visível** no canto sup. direito do cabeçalho de CADA painel (Poderes + Galeria) — `ui::Button`
  ghost/ícone "✕", dispara um `on_close` (handler do call-site).
- **Clique-fora fecha**: clicar no backdrop (fora do painel central) fecha; clicar DENTRO do painel não
  (occlusão — `.occlude()` ou parar a propagação no painel central; reuse o idioma do `ui::Modal` se ajudar).
- **Esc fecha**: é costura minha em `main.rs:handle_key` (~:4877, junto do `attention_panel_open`). Você só
  garante que o estado (`powers_panel_open`/`design_gallery_open`) é o que eu seto. Me diga se precisa de algo.

### Bug 2 — GALERIA = ÁREA CENTRAL (igual Poderes):
Refatore a galeria para o MESMO padrão que você fez na r2: container `size_full flex_col`, cabeçalho FIXO
(título Fraunces + intro + seções "A cara do Lina" / curadas), **corpo ROLÁVEL** (`overflow_y_scroll`), e o
call-site (main.rs, eu) põe num overlay central `~0.72×0.82`. Reuse o que você já construiu — mesma casa.

### Bug 3 — "USAR ESTA" COM FEEDBACK CLARO:
- Quando a direção é a escolhida (`selected`), o botão vira **"✓ Em uso"** (variante `Confirm`/destaque) e o
  card ganha realce forte (borda de acento + selo "Sua direção atual"). A escolha fica **inequívoca** — some
  a sensação de "morto". `CHOOSE_LABEL` ("Usar esta") só nos não-escolhidos.
- **Escopo honesto (sinalizar, não esconder):** APLICAR a direção de verdade (trocar TODA a aparência do app
  para aquele `DESIGN.md`) é uma **feature à parte** — exige mapear cada direção → params de `Theme`
  (`theme::set_*`/`Theme::build`). O mecanismo existe (`theme.rs:768`), mas o mapeamento de 12 direções é a
  próxima fatia. Nesta r3: o botão REGISTRA a escolha (feedback "Em uso"); deixe o gancho pronto
  (`selected_direction()` já existe) para a fatia de aplicação. **Não finja aplicar o que não aplica.**

## RESULTADO ESPERADO
- Poderes e Visual: ambos área central, com **backdrop escuro + X + clique-fora**; a galeria rola.
- "Usar esta" → "✓ Em uso" com realce — escolha inequívoca; aplicação real do tema sinalizada como fatia seguinte.
- `token_ratchet` verde (scrim via token, não literal); testes do módulo cobrindo o estado "em uso" e a
  estrutura; clippy `--all-targets` verde; suíte serial verde.
- Diff de costura `main.rs`: Esc fecha ambos (bloco ~4877), overlay central da galeria, backdrop clicável
  (handler de fechar), `on_close` de cada painel. Eu aplico.

> gpui não roda headless → o juiz é a tela do fundador. Render provado por teste + costura que compila.

Reporte ao @Maestro 01. Entrega `tasks/epico-f2/despachos/f2-4/.entrega-f2-4-ui-r3.md`. Última linha:
`PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
