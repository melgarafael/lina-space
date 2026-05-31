# Onda 2 — Render de terminal de VERDADE + todo nó é um SHELL REAL · RESULTADO

> **TL;DR:** os terminais são **usáveis** e **todos idênticos em capacidade**. Cada card pinta
> o **snapshot do GRID VISÍVEL** (`VtBackend::screen()` — cols×rows com **cor por célula +
> cursor**) lido do `alacritty_terminal` **a cada frame** (não mais `row_text` empilhado), e
> **TODO terminal (A, B, …) roda um SHELL INTERATIVO REAL** (`$SHELL` com fallback `/bin/sh`)
> — sem mock/`cat`. O fundador clica no B, digita `claude`/`ls` e funciona **igual ao A**. O
> A2A continua (botão injeta um comando em B via `deliver_a2a`, com pulso "sem fios"), mas o B
> também aceita o humano digitar — o A2A é só **mais uma fonte de input pela fila serial do
> Supervisor**. Há scroll, teclas especiais (CSI/SS3 sem lixo), resize (**84×26** ≥ 80×24),
> cores ANSI/256, canvas/pan e persistência. **Build passa; clippy `-D warnings` + fmt limpos;
> render e A2A provados em teste; core 57 testes verdes (core intocado nesta correção); sem panic.**

---

## 0. Conserto de design: nada de terminal mock — todo nó é um shell real

Antes, o Terminal B rodava `cat` (só ecoava) — um receptor mock que não aceitava comandos.
**Agora** `main.rs` cria **todo** terminal com o mesmo `shell_cmd(name)`: um banner neutro + `exec
"${SHELL:-/bin/sh}" -i` — o shell do usuário (zsh/bash/fish) com fallback `/bin/sh`, interativo,
**idêntico** para A, B, C…. Consequências:
- **Input do humano** já roteia para o nó **focado** (`handle_key → self.focused →
  CoreInput::write_human`): clicar no B e digitar escreve no PTY do B; no A, no A. Vale para os dois.
- **O A2A coexiste:** o botão injeta um **comando válido** (`echo '…'`) de A→B via `deliver_a2a`
  (bracketed-paste faseado → Enter), que entra pela **MailQueue serial** do Supervisor — a mesma
  fila do input humano. Logo o B é um shell normal que recebe tanto o humano quanto o A2A, sem
  conflito (a fila serializa). O texto chega e **roda** no shell do B; o pulso A→B aparece.
- **Textos neutros:** banner de cada terminal = "Terminal X — shell interativo (digite comandos;
  rode claude/vim/…)". Nada de "recebe mensagens A2A".

---

## 1. Causa raiz (o que estava errado) e o conserto

**Antes:** o `GridDelta { dirty_rows }` disparava `row_text(r)` por linha suja → o shell
guardava um `Vec<String>` por-linha (texto puro, **sem cor, sem cursor**) e desenhava isso.
Uma TUI que **redesenha a tela inteira** (alternate screen, `ESC[2J` + cursor absoluto) ficava
embaralhada: só as linhas que o alacritty marcou sujas eram puxadas, misturadas com linhas
velhas; sem cursor, menus/spinners perdiam a âncora; e setas viravam `^[[C` porque o texto
echoado aparecia cru.

**Agora:** o shell lê o **snapshot completo do grid** a cada frame e pinta tudo:
```
PTY → pty-host (lina-core) → alacritty_terminal (lina-vt)
   → VtBackend::screen()  ──►  VtScreen { cols, rows, cells: Vec<VtCell{c,fg,bg,bold}>, cursor, display_offset }
   → o card pinta cols×rows: runs de mesmo estilo viram spans coloridos; a célula do cursor é um bloco.
```
O `GridDelta` vira só um sinal "mudou" (drenado); o `SharedModel` carrega só metadados do nó
(nome/status/posição) + pulso + persistência. O conteúdo do terminal vem **do grid**, sempre fresco.

---

## 2. Mudança de core (lina-vt) — o acessor que faltava

`crates/lina-vt/src/lib.rs` (promovido à trait `VtBackend`, backward-compatible):
- **`fn screen(&self) -> VtScreen`** — snapshot do viewport via `Term::renderable_content()`:
  itera o `display_iter` (terminal-absoluto → viewport com `point.line + display_offset`), resolve
  cada `Color` (`Named`/`Indexed`/`Spec`) numa `VtRgb` com uma **palette xterm-256 auto-contida**
  (o alacritty não embarca palette), aplica `INVERSE`/`BOLD`, e marca o **cursor** (oculto se
  `CursorShape::Hidden`).
- **`fn scroll(&mut self, delta: i32)`** — `Term::scroll_display(Scroll::Delta)` (scrollback).
- **`fn dims(&self) -> (usize,usize)`** + tipos públicos `VtScreen`/`VtCell`/`VtRgb`/`VtCursor`.
- **`TermMode.app_cursor`** (DECCKM) — para as setas irem como `ESC O A/B/C/D` quando a TUI pede.

`lina-core` re-exporta `VtScreen/VtCell/VtRgb/VtCursor` (facade). **57 testes do workspace verdes.**

---

## 3. O que ficou usável (gate)

1. **Grid visível completo, colorido, com cursor** — `render_grid(screen)` pinta `rows` linhas;
   cada linha agrupa células de mesmo `(fg,bg,bold)` em **spans** (`div().bg().text_color()` +
   `font_weight(BOLD)`), monoespaçado → alinhado; a célula do cursor é um **bloco invertido**.
2. **Cores/atributos por célula** — fg/bg/bold/inverse resolvidos no `screen()`; 16 ANSI + cubo
   256 + grayscale + RGB direto.
3. **Cursor** — posição + bloco; oculto quando o app esconde (`CursorShape::Hidden`).
4. **Scroll/scrollback** — `on_scroll_wheel` (trackpad `Pixels`/roda `Lines`) → `grid.scroll(±n)`
   → `display_offset`; o próximo `screen()` mostra o histórico.
5. **Resize** — `fit_dims()` calcula **84×26** (≥80×24) do tamanho do card; o PTY (`PtyManager::resize`)
   e o grid (`VtBackend::resize`) acompanham (provado no teste: dims mudam para 100×30).
6. **Teclas especiais** — setas (CSI/SS3 por `app_cursor`), Home/End, PageUp/Down, Insert/Delete,
   F1-F12, Ctrl+letra → bytes corretos ao master; **sem eco de lixo** (o alacritty interpreta as CSI).

---

## 4. Compila? Roda? (evidência)

- **Compila:** `cargo build` → `Finished`, **0 erros**.
- **Clippy:** `cargo clippy --all-targets -- -D warnings` → **limpo**.
- **Fmt:** limpo.
- **Gate de render (teste determinístico, sem display) — `screen_renders_colors_cursor_and_scroll`:**
  alimenta ANSI controlado (`ESC[2J` clear + `ESC[H` home + `ESC[31m`VERMELHO + `ESC[1m`NEGRITO) e
  asserta o `screen()`: caractere, **fg vermelho** (`0xcd0000`), **negrito**, **cursor em (0,8)**, e
  o **scroll** (`display_offset` 0→5→0). **Passa.**
- **Integração — `a2a_roundtrip_pulse_persist_and_screen`:** 2 terminais, A2A A→B via `deliver_a2a`,
  `BusEvent::Message` → pulso + aura, o texto A2A aparece no **snapshot `screen()` de B**, **resize**
  100×30, evento persistido. **Passa.** (+ `recovery_pair_toggles_banner`.) **3/3.**
- **57 testes do workspace verdes** (a mudança de core é compatível).
- **Roda (smoke):** a janela abre com 2 cards, grid **84×26**, A2A dispara (pulso + B recebe), persiste;
  relaunch → contador 23 → 29 (**estado sobrevive**); **sem panic**.

---

## 5. Como rodar (teste visual do fundador)
```bash
cd app/lina-gpui
cargo run                      # abre o canvas (ou rode o binário pronto: ./target/debug/lina-gpui)
# Clique no Terminal A (ou no B — IDÊNTICOS) p/ focar; ambos são shells reais:
#   digite `claude` (ou `ls`, `vim`, `htop`) → caixas/cores/cursor corretos; setas navegam; scroll rola.
# Digitar no B escreve no PTY do B; no A, no A (roteamento pelo nó focado).
# Clique "⚡ Enviar A2A (A→B)" → pulso A→B + o comando injetado RODA no shell do B (que segue digitável).
cargo test                     # 3/3 (gate de render + A2A + recovery)
```

## 6. Próxima story
StyledText/`TextRun` (1 elemento por linha) se o nº de spans pesar; resize dinâmico ao redimensionar
o card/janela; seleção + copy/paste; mouse reporting (SGR) para TUIs que usam mouse; `NodeMoved`
persistido ao arrastar cards.
