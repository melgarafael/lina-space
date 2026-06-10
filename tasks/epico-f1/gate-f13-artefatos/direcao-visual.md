# Direção Visual — LP Curso de IA para Iniciantes

## 1. Direção estética (uma só): "Editorial Caloroso"

Cara de **editora premium brasileira + mesa de professor**, não de startup de tech.
Papel quente, serifa com personalidade no display, MUITO respiro, um único laranja-queimado
para toda ação. Zero gradiente, zero glassmorphism, zero dark-neon.

**Por que funciona para este público:** o leigo já desconfia de IA ("isso é coisa de
programador?"). O visual de revista/escola séria ativa confiança que ele já tem em
instituições editoriais — e o tom de papel quente tira a frieza "techie" da página.
Quem vende aqui é o professor, não a tecnologia.

## 2. Paleta (hex exatos)

| Token | Hex | Uso |
|---|---|---|
| `--papel` | `#FAF5EC` | fundo da página (nunca branco puro) |
| `--papel-alto` | `#FFFFFF` | superfície de card/FAQ |
| `--tinta` | `#221E18` | texto principal (preto quente, nunca #000) |
| `--tinta-suave` | `#6E6657` | texto secundário, legendas |
| `--brasa` | `#D4502B` | ÚNICA cor de ação: CTAs, links, marcações. Nada mais usa este hex |
| `--brasa-escuro` | `#B23E1F` | hover/active do CTA |
| `--verde-mata` | `#2E4B3C` | seção de contraste (faixa "Pra quem é" e CTA final em fundo escuro) |
| `--areia` | `#EFE6D5` | divisores, fundos de destaque sutis, tags |

Regra: máx. 1 seção em `--verde-mata` antes do CTA final. O resto da página é papel.

## 3. Tipografia (Google Fonts)

- **Display: `Fraunces`** — pesos 600 e SemiBold Italic; usar `opsz` alto nos títulos.
  Serif com personalidade, calorosa, humana. É o oposto do default de IA.
- **Corpo/UI: `Work Sans`** — 400 corpo, 500 UI, 600 botões. Humanista, legível, não é Inter.
- Escala 1.25 (major third), base 18px (corpo grande = público 35+ lê sem esforço):
  18 / 22.5 / 28 / 35 / 44 / 55px. Hero pode chegar a `clamp(2.5rem, 6vw, 4.5rem)`.
- Line-height: 1.65 no corpo, 1.1 no display. Parágrafos máx. 65ch.
- Recurso de marca: nos títulos, a palavra-chave em **Fraunces Italic + `--brasa`**
  (ex.: "Use IA no seu negócio *sem programar*"). É o único "efeito" tipográfico permitido.

## 4. Hero

- Assimétrico 60/40: texto à esquerda (60%), à direita UMA imagem real do instrutor
  (foto, não ilustração 3D genérica) com moldura `2px solid var(--tinta)` e leve
  rotação de -1.5deg — toque de "polaroid na mesa", humano.
- Acima do H1: tag pequena em `--areia`, Work Sans 500 uppercase tracking 0.08em
  ("CURSO ONLINE · EM PORTUGUÊS · SEM CÓDIGO").
- H1 em Fraunces, 1 frase concreta; subtítulo 22.5px `--tinta-suave`, máx. 2 linhas.
- 1 CTA primário + 1 link secundário sublinhado ("ver o que você vai aprender ↓").
  Nunca dois botões cheios lado a lado.
- Fundo: `--papel` liso. Sem gradiente, sem blob, sem partículas.

## 5. Botões

- Primário: bg `--brasa`, texto `#FAF5EC`, Work Sans 600 18px, padding `16px 32px`,
  `border-radius: 10px` (não pill, não quadrado), `border: none`, sem sombra.
  Hover: `--brasa-escuro` + translateY(-1px), transição 180ms ease-out.
- Secundário: texto `--tinta` sublinhado com `text-decoration-color: var(--brasa)`,
  espessura 2px, offset 4px. Sem borda, sem fundo.
- Texto do botão sempre verbo concreto ("Quero me inscrever"), nunca "Saiba mais".

## 6. Cards (módulos, FAQ, "pra quem é")

- bg `--papel-alto`, `border: 1px solid rgba(34,30,24,0.14)`, `border-radius: 12px`,
  padding 32px. **Sem box-shadow** — hierarquia vem do contraste papel/branco e do espaço.
- Numeração dos módulos em Fraunces Italic 35px `--brasa` ("01.") no lugar de ícones —
  zero grid de ícones genéricos.
- "Pra quem NÃO é": mesmo card, mas bg `--areia` e marcador "✕" em `--tinta-suave`.
- Hover (desktop): borda passa a `rgba(34,30,24,0.3)`, 180ms. Nada de levantar com sombra.

## 7. Espaçamento e grid

- Base 8px. Seções: `padding-block: 112px` desktop / `64px` mobile.
- Container: `max-width: 1080px` (página de leitura, não de dashboard); texto corrido
  em coluna de até 720px.
- Gaps: 24px entre cards, 16px interno entre elementos relacionados.
- Divisores entre seções: `1px solid var(--areia)` ou simplesmente espaço — nunca
  ondas SVG ou diagonais.
- Motion global: apenas fade-in + 8px de subida ao entrar no viewport, 400ms,
  `cubic-bezier(0.25, 0, 0, 1)`, com `prefers-reduced-motion` respeitado. Sem parallax.

## 8. Proibido nesta página (além do briefing)

Roxo em qualquer tom; emoji como ícone de feature; foto de banco de imagem de robô/cérebro;
contador regressivo piscando; mais de uma fonte display; sombras coloridas.
