# ADR 0030 — Paleta ANSI oficial do terminal embutido: ancorada no design system (Tokyo Night)

- **Status:** **Aceito (2026-06-13, rodada de bugs F2).** Fecha a porta de design da paleta de 16
  cores ANSI do terminal embutido, que até aqui era um default herdado (VGA/xterm legacy), não uma
  decisão *de jure*.
- **Onda/Story:** rodada de correção de bugs da Fila F2 · fatia `bf-terminal` (bug C1).
- **Data:** 2026-06-13
- **Fontes:** investigação empírica do Terminal B (`tasks/despachos/bugs-f2/.entrega-bf-terminal.md` §C1):
  (a) `defaults read com.apple.Terminal` → perfil ativo do usuário = "Basic" (paleta Apple sobre fundo
  preto); (b) ambiente do terminal embutido = `TERM=xterm-256color`, `TERM_PROGRAM` vazio; (c) paleta
  Tokyo Night "Night" cruzada em duas fontes oficiais (`folke/tokyonight.nvim` extras alacritty + kitty,
  idênticas); (d) tokens de cor verificados em `app/lina-gpui/src/theme.rs`.

## Contexto

O bug C1 da Fila F2 dizia: *"cores/atributos do texto do terminal diferentes do Claude real"*. As 16
cores ANSI base do `lina-vt` eram a paleta **VGA/xterm legacy** (`#cd0000`, `#00cd00`, `#0000ee`…) —
cruas e berrantes sobre o fundo navy do terminal embutido (`terminal.bg #0d1228`). Resultado: terminal
"esquizofrênico" — chrome elegante (o tema do app já é Tokyo Night de facto), sintaxe do CLI berrante.

**Reenquadramento técnico (o que desambigua o critério do usuário):** as 16 cores ANSI são do
**EMULADOR de terminal**, não do Claude. O CLI só emite o código de atributo (`SGR 31` = "primeiro
plano vermelho"); **quem decide qual vermelho pintar é o terminal**. Logo "igual ao Claude real"
significa, ao pé da letra, *a paleta do Terminal.app do usuário* — perfil "Basic", paleta Apple sobre
fundo **preto**. Essa literalidade **não transplanta** para o Lina:

1. O fundo do Lina é navy `#0d1228`, não preto. O azul ANSI da Apple (`#0000b2`) é quase invisível
   sobre navy — baixo contraste, ilegível.
2. Terminais embutidos sérios (VS Code, Zed) **não** replicam a paleta do terminal externo do SO — usam
   a paleta do **próprio tema**. Replicar o terminal externo de cada usuário quebraria a identidade
   visual do app e seria impossível de garantir (cada um tem um perfil diferente).

## Decisão

**A paleta ANSI de 16 cores do terminal embutido é ancorada no design system do app (Tokyo Night),
não no terminal externo do SO.** A âncora é verificável em arquivo, não estética:

- **As 6 cores cromáticas (ANSI 1–6) são, hex-a-hex, os tokens semânticos que o app já usa**
  (`theme.rs`): vermelho = `STATE_DANGER` (#f7768e), verde = `STATE_SUCCESS` (#9ece6a), amarelo =
  `STATE_WARNING` (#e0af68), azul = acento "azul"/`TERMINAL_CURSOR` (#7aa2f7), magenta = acento
  "violeta" (#bb9af7), ciano = acento "ciano" (#7dcfff). **6 de 6** casam o sistema de cores existente.
- **As 4 não-semânticas (black/white + os brights correspondentes)** vêm da paleta publicada **Tokyo
  Night "Night"** (`folke/tokyonight.nvim`, alacritty+kitty cruzados). A tabela completa
  índice→hex→token→fonte está na entrega da fatia `bf-terminal`.
- **O cubo 6×6×6 (índices 16–231) e a rampa de cinza (232–255) permanecem padrão xterm universal** —
  só as 16 cores base são por-tema.
- **`default_fg`/`default_bg` NÃO mudam:** seguem os tokens curados do app (`terminal.bg #0d1228`,
  `text.primary #c8d3f5`). Aplicar o `#1a1b26`/`#c0caf5` canônico do Tokyo Night dessincronizaria do
  shell — o casamento correto é com os tokens do app, não com a paleta de origem ao pé da letra.

## Alternativas rejeitadas

- **Manter a paleta VGA/xterm legacy** — era o default herdado, causa do bug: cruas e dissonantes sobre
  o navy; sem nenhuma relação com o design system.
- **Replicar a paleta do Terminal.app do usuário (Apple Basic)** — é a leitura literal de "igual ao
  Claude real", mas: (a) baixo contraste sobre o navy do app (o azul Apple some); (b) quebra a
  identidade visual; (c) impossível de garantir entre usuários com perfis diferentes. Se algum dia a
  literalidade com o terminal externo virar requisito de produto, é decisão de produto em story própria
  — não o default.

## Consequências

- Bug C1 fechado: a sintaxe do CLI (diff verde/vermelho, prompts cyan, avisos amber) cai na mesma
  família dos acentos do shell — coerência interna e contraste garantido.
- A paleta vira **decisão *de jure*** com fonte verificável; um futuro tema alternativo do app
  redefine as 6 cromáticas re-ancorando nos seus próprios tokens semânticos (mecanismo, não cor fixa).
- Critério auditável: teste `ansi_base_palette_is_tokyo_night` (lina-vt) trava os 16 valores; os 6
  cromáticos podem ser cross-checados contra os tokens de `theme.rs`.
- **Validação na tela do fundador pendente:** a fidelidade percebida ("ficou como esperado") só se
  confirma rodando o app; se o fundador preferir outra direção ao ver, é trocar as constantes ancoradas.
