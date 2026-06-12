# Pedido de costura — r3 F2-1-1b (Terminal C → Maestro)

> Meu lado da story atômica está completo (theme.rs · bridge.rs · attention_ui.rs ·
> agent_modal.rs · assets/fonts/): suíte 421/0 + catraca 2/2 + clippy -D warnings + fmt nos meus
> arquivos. Este pedido tem os diffs exatos do `main.rs` (seus) + 1 linha no theme.rs (meu,
> autorizo) + as notas de validação na tela. **Story atômica: commit só com tudo junto.**

## O que JÁ está no meu lado

- `assets/fonts/` (novo, ~1,25MB): JetBrains Mono Regular+Bold (+OFL.txt) · IBM Plex Sans
  Regular/Medium/SemiBold/Bold (+LICENSE.txt) · Fraunces72pt-SemiBold (+OFL.txt). Estáticos
  OFL verificados (magic sfnt + tabela name parseada em teste).
- `theme::embedded_fonts()` — os 7 TTFs via `include_bytes!`, prontos para `add_fonts`.
- **Token display corrigido: `"Fraunces"` → `"Fraunces 72pt"`** — é o nome INTERNO da instância
  estática (name ID 16); a OFL reserva "Fraunces" (renomear tabela = perder o nome). Com o nome
  antigo a fonte embarcada NUNCA resolveria (fallback silencioso). Teste de paridade
  token↔arquivo embarcado cobre os 3 tokens (`embedded_fonts_match_typography_tokens`).
- **Célula re-derivada**: `CELL_W 7.84→7.80` · `CELL_H 17.0→17.16` (JetBrains Mono 13px;
  advance 600/1000 em · ascent 1020 + descent 300). Teste `cell_fallbacks_match_embedded_grid_
  font_metrics` RE-DERIVA as consts parseando head/hhea/hmtx do TTF que viaja no binário —
  trocar fonte/tamanho sem re-derivar falha no CI.
- **Métricas vivas**: `bridge::cell_w()/cell_h()` (atomics; default = fallback) +
  `bridge::set_cell_metrics(w,h)` com janela de sanidade (medição insana = recusada com log —
  fonte errada não desalinha o grid). Hit-testing/seleção do bridge já leem os accessors.
- 2 dos 3 Menlo→token aplicados (attention_ui.rs:774, agent_modal.rs:2253).
- **Lint novo** (`no_hardcoded_font_families_outside_theme_module`): zero `.font_family("…")`
  literal fora do theme.rs — exceção TEMPORÁRIA: 1× `"Menlo"` em main.rs (a sua costura).
- Catraca: snapshot INALTERADO e verde — as 3 categorias dela não cobrem `font_family` (string,
  não px/FontWeight/text_size); quem cobre famílias é o lint novo acima. O `.text_size(px(
  FONT_PX*scale))` também nunca contou (FONT_PX é ident, não literal) — a expectativa de
  "baixar contagens" do despacho não se materializa nessas réguas; a garantia real é o lint.

## Diffs do main.rs (seus — na ordem)

```text
1) BOOT — logo no INÍCIO do closure do run, ANTES do fit_dims/log (linha ~4751) e do bloco de
   tema (~4790), porque fit_dims/PTY já dependem da célula medida:

   if let Err(e) = cx.text_system().add_fonts(theme::embedded_fonts()) {
       eprintln!("lina-gpui: fontes embarcadas não carregaram ({e}) — seguindo nas do sistema");
   }
   {
       let t = theme::active().typography;
       let font_id = cx.text_system().resolve_font(&gpui::font(t.family.mono));
       let sz = px(f32::from(t.size.grid));
       match cx.text_system().advance(font_id, sz, 'M') {
           Ok(adv) => {
               let h = cx.text_system().ascent(font_id, sz)
                   + cx.text_system().descent(font_id, sz);
               bridge::set_cell_metrics(f32::from(adv.width), f32::from(h));
           }
           Err(e) => eprintln!(
               "lina-gpui: não medi a célula do grid ({e}) — usando o fallback derivado do TTF"
           ),
       }
   }
   (API verificada no pin: add_fonts(Vec<Cow<'static,[u8]>>) text_system.rs:102 ·
    resolve_font:148 · advance:195 · ascent:269 · descent:275 · gpui::font() helper :1077.)

2) main.rs:87 (import do bridge): remover `CELL_H, CELL_W`; adicionar `cell_h, cell_w`.

3) main.rs:90-91: remover `const FONT_PX: f32 = 13.0;` (e o comentário "fonte do grid (Menlo)")
   e criar no lugar:
   /// Tamanho da fonte do grid, do token (contrato grid=13 — teste no theme.rs).
   fn grid_font_px() -> f32 { f32::from(theme::active().typography.size.grid) }

4) fit_dims (main.rs:107-108): `CELL_W` → `cell_w()` · `CELL_H` → `cell_h()`.

5) render_grid (main.rs:416-422):
   .font_family("Menlo")            → .font_family(theme::active().typography.family.mono)
   .text_size(px(FONT_PX * scale))  → .text_size(px(grid_font_px() * scale))
   .line_height(px(CELL_H * scale)) → .line_height(px(cell_h() * scale))
   (No comentário BUG A acima dessas linhas, trocar "Menlo 13px" por "da fonte do grid".)

6) Log de boot (main.rs:~4756): a format string usa {CELL_W}/{CELL_H}/{FONT_PX} — trocar por
   valores locais (let cw = cell_w(); let ch = cell_h(); font = grid_font_px()) mantendo o
   texto. ATENÇÃO: este log roda DEPOIS da medição do item 1 (ordem do boot) para imprimir a
   célula real.

7) theme.rs (MEU arquivo — autorizo esta única linha): no teste
   `no_hardcoded_font_families_outside_theme_module`, trocar
   `const SEAM_EXCEPTIONS: &[(&str, &str, usize)] = &[("main.rs", "\"Menlo\"", 1)];`
   por
   `const SEAM_EXCEPTIONS: &[(&str, &str, usize)] = &[];`
   — a exceção existe SÓ para a janela pré-costura; sem removê-la, "Menlo" poderia voltar ao
   main.rs sem ninguém ver.
```

## Validação na tela (seu repack — roteiro de 4 olhares)

1. Grid em JetBrains Mono (desambiguação visível: `l` vs `1` vs `I`, zero com ponto) + bold ANSI.
2. Seleção com mouse: arrastar sobre uma palavra seleciona EXATAMENTE as células sob o cursor
   (a prova do alinhamento célula/hit-test com a célula medida).
3. Log de boot imprime `cell 7.8x17.16` (≈; medido) — se imprimir o fallback com aviso de
   medição, investigar antes de aceitar.
4. Última linha da TUI (barra de input) visível, não clipada (CELL_H 17.16 reduz ~1 row por
   card vs Menlo — `fit_dims` recalcula; o critério é o de sempre: input aparece).

Nota: esta máquina pode TER JetBrains Mono instalada — para provar o embedding, rodar uma vez
com `fc-list`/Fontbook conferido ou em conta limpa é o ideal; o teste de paridade + add_fonts
no boot cobrem o caminho de qualquer forma.
