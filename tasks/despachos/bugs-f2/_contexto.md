# RODADA DE BUGS — Fila F2 (fonte: vault "Fila de Bugs Identificados no Lina")

> Leia este doc + as REGRAS COMUNS (`tasks/despachos/_regras-comuns.md`) ANTES de tocar código.
> O Maestro (Terminal A / Arquiteto) é o **dono único de `app/lina-gpui/src/main.rs`** nesta rodada.
> Você **não toca `main.rs`** — se precisar de 1 linha lá, registre o pedido na sua entrega; o Maestro costura.

## Contexto do produto
Lina Space: app desktop Rust-nativo, GPU-first (gpui/wgpu), canvas multi-terminal de IA para
não-técnicos. Cada nó é um CLI de IA (Claude Code) num terminal vivo. F2 é a onda de UI/UX.
Usuário é leigo — a barra de qualidade é "um sênior assinaria isto?". Banir slop visual.

## Os 14 bugs reportados (com prints no vault)
Prints (leia com a ferramenta de imagem — caminhos absolutos):
`/Users/rafaelmelgaco/Documents/Obsidian Vault/Captura de Tela 2026-06-*.png`

### Família TOASTS / NOTIFICAÇÕES  → frente `bf-toasts`
- T1. Toast "Fila de Atenção": botão "→ Ir até o terminal" não faz nada ao clicar.
- T2. Toast "Atividade e custo do time" e "Fila de Atenção" ficam SOBREPOSTOS — sem camadas/organização.

### Família MODAIS / OVERFLOW  → frente `bf-modais`
- M1. Modais não contêm overflow: crescem pra fora da viewport em vez de limitar altura (~85vh) e
  rolar o corpo internamente. Caso concreto: "Espaços & Ajustes". Aplicar a DOUTRINA abaixo.
- M2. Modal/popover de ações sem véu de oclusão: clique "vaza" para o terminal atrás (ver T-encerrar).

### Família TERMINAL (render + input)  → frente `bf-terminal`
- C1. Cores/atributos do texto do terminal diferentes do Claude real (mapeamento ANSI/SGR).
- C2. `ctrl+o` (expandir/colapsar detalhes de execução) — comportamento ausente/errado no terminal embutido.
- C3. Acionar skill com `/` e navegar com as setas não funciona dentro do terminal.
- C4. Cursor "|" piscando NÃO aparece nos campos de digitação da UI — sem indicação de foco de teclado.

### Família SIDEBAR + BOTÕES DO CANVAS  → frente `bf-sidebar` (+ costuras no Maestro)
- S1. Sidebar de Espaços: clipping/truncation — "Arquivar" não aparece, falta affordance. Mesmo
  problema na barra superior dos terminais.
- S2. Botão "Centralizar" não faz nada (costura main.rs — Maestro).
- S3. "Encerrar" com um terminal atrás: o clique foca o terminal em vez do botão (costura main.rs — Maestro).

## Mapa de código (de exploração — confirme antes de mudar)
- Toasts: `attention_ui.rs` (render toast/painel + botão "ir até"), `dashboard.rs` (geometria do
  painel "Atividade e custo": `dashboard_panel_rect`). Handler `attention_goto_node` e a COMPOSIÇÃO/
  z-order dos painéis vivem em `main.rs` (~1898, ~4477) → COSTURA do Maestro.
- Modais: wrapper canônico CORRETO já existe em `ui/modal.rs` (`Modal`, `clamp_frame`, body com
  `min_h(0)+overflow_y_scroll()`). Usa-o: `agent_modal.rs` (M6). NÃO usa: `persistence_ui.rs`
  ("Espaços & Ajustes", janela 560×680 sem scroll interno) e `render_create_space` (em main.rs → Maestro).
  Occlude/véu: `ui/modal.rs` tem `.occlude()` mas M6 não liga (`agent_modal.rs` ~2308).
- Terminal: cores ANSI em `crates/lina-vt/src/lib.rs` (`xterm256` ~874, `resolve_color` ~917, defaults
  ~630). Render run→cor em `main.rs` (`render_line`/`rgba` ~332-436) → COSTURA do Maestro. Caret dos
  inputs da UI em `ui/input.rs` (`CARET` const ~19, `display_text` ~119, sem blink). Keystroke→PTY e
  `keystroke_to_bytes` em `main.rs` (~127, ~2848) → COSTURA do Maestro.
- Sidebar: `sidebar.rs` (linha do item ~1117-1254; `.overflow_hidden()` ~1141 sem `w_full/min_w(0)`).
  Botão "Centralizar" handler em `main.rs` ~4228 (chama `camera.reset()` sem `cx.notify()`) → Maestro.
  Botão "Encerrar" toolbar em `main.rs` ~2022 → Maestro.

## DOUTRINA — Contenção de Overflow em Modais e Painéis (regra permanente do projeto)
Todo modal/dialog/drawer/popover/painel flutuante segue, SEM exceção:
- `max-height` relativa à viewport (ex.: 85vh) e `max-width` (ex.: `min(90vw, …)`).
- 3 regiões: **header fixo · corpo rolável · footer fixo**. Header/footer NÃO rolam (sempre visíveis).
- Corpo: `overflow-y: auto` **E** `min-height: 0` (a linha que quase sempre falta no flex-col).
- `box-sizing: border-box`; overlay com padding (modal nunca cola na borda).
- Indicação visual de mais conteúdo (scrollbar serve).
- **Teste obrigatório:** encher o modal com conteúdo de 3× a altura da tela e validar: (a) modal dentro
  da viewport, (b) corpo rola, (c) header e footer permanecem visíveis e fixos. Falhou um → NÃO está pronto.
- Equivalente gpui: no frame `.flex().flex_col().max_h(...)`; no corpo `.flex_1().min_h(px(0.)).overflow_y_scroll()`;
  header/footer com `.flex_shrink_0()`. O `ui/modal.rs::Modal` JÁ faz isso — prefira reusá-lo a reinventar.

## Protocolo desta rodada (além das REGRAS COMUNS)
1. 1º ato: `touch .iniciado-<sua-fatia>` (ex.: `.iniciado-bf-modais`).
2. Fronteira de arquivos = LEI. `main.rs` é do Maestro — pedidos de costura vão na sua entrega.
3. Reporte status ao Maestro: `lina ask "@Terminal A" "<status>" --intent status` ao começar/terminar/travar.
4. Valide POR PACOTE (app: `cd app/lina-gpui`), exit codes DIRETOS (sem pipe), e LEIA a saída.
5. Entrega: `tasks/despachos/bugs-f2/.entrega-<sua-fatia>.md` — o que mudou (arquivo:linha), evidência
   (comandos+exit+nº testes), pedidos de costura em main.rs (diff textual PRECISO p/ o Maestro aplicar),
   riscos/achados. Última linha: `PRONTO` ou `BLOCKED: <motivo>`.
6. NÃO commite. O Maestro valida de fora e commita por fatia.
