# SPEC canônica — protótipos F2-0-D (conteúdo IDÊNTICO nos 4 territórios)

> Insumo de trabalho dos builders. Os 4 mockups (`t1..t4`) vestem ESTA MESMA TELA —
> só a pele (tokens/tipografia/densidade/mecanismo) muda. Qualquer divergência de
> conteúdo entre arquivos é bug. Fonte: despacho-prototipos-f2-0-d.md + entrega-d1 §II
> + correção da onda V (T1: viewer vivo SÓ no foco) + D4 (botão Buscar/⌘K) + roteiro r4/r5.

## 0. Regras de arquivo (todos)

- HTML estático autocontido: 1 arquivo, CSS inline em `<style>`, ZERO JS obrigatório
  (permitido: nenhum; mockup é estático). Abre com duplo-clique.
- `<html lang="pt-BR">` · `<meta charset="utf-8">` · `<meta name="viewport" ...>`.
- Título da aba: `Lina — T<N> · <Nome do Território>` (index: `Lina — Escolha o território estético`).
- Fontes: Google Fonts CDN com `display=swap` + stack de fallback honesto (fallback ≠ escolha).
- Desenhado para desktop ≥1280px (a sessão de decisão é em desktop). `min-width: 1180px` no frame.
- Comentário HTML de abertura com EXATAMENTE 3 linhas: statement do território ·
  mecanismo encarnado · o que NUNCA faz.
- Animação: só vida ambiente mínima (caret piscando, pulso suave no "Trabalhando") e
  TUDO dentro de `@media (prefers-reduced-motion: no-preference)` (ou guard equivalente).
- A11y na medida do estático: contraste AA nos pares principais (texto normal ≥4.5:1,
  texto grande/UI ≥3:1), alvos clicáveis ≥24px, estados sempre com TEXTO+ÍCONE+COR,
  HTML semântico razoável (`<button>`, `<nav>`, headings).
- Anti-slop (banido): gradiente roxo decorativo (`#7c3aed`/`#a855f7`/`#6366f1` e família),
  glassmorphism (`backdrop-filter: blur` + branco translúcido), Inter/Roboto/Arial/system-ui
  como ESCOLHA, sombra teatral sem função, copy de molde.
- Navegação do protótipo (não é UI do app): pill fixa no canto inferior-esquerdo,
  discreta, com texto `Protótipo T<N> — ← escolher outro território` linkando `index.html`.
  Mesma posição nos 4.

## 1. Estrutura da tela (mesmas regiões nos 4)

```
┌──────────┬─────────────────────────────────────────────┐
│ RAIL     │ CANVAS (fundo de canvas: dot-grid sutil)    │
│ esquerdo │  [pill de atenção, topo-centro]             │
│ ~270px   │  5 cartões-terminal dispostos espacialmente │
│          │  [controles de zoom, canto inferior-direito]│
└──────────┴─────────────────────────────────────────────┘
```

### RAIL (de cima para baixo)
1. Wordmark pequeno: `Lina` + setinha de recolher `◀` (alvo ≥24px).
2. Indicador do Espaço atual (destaque): `Loja Virtual — Lançamento`
   subtítulo: `5 terminais · 2 trabalhando agora`.
3. Botão **Buscar** (decisão D4 — sempre visível): label `Buscar`,
   sub-rótulo `terminais, Espaços e ações`, acelerador exibido: `⌘K`.
4. Seção `Espaços`:
   - `Loja Virtual — Lançamento` — marcado como atual.
   - `Marketing de Conteúdo` — meta `2 trabalhando ao fundo` + ação de linha
     VISÍVEL (como se em hover): botão `Descarregar`.
   - `Pesquisa de Fornecedores` — meta `descansando`.
5. Botão `+ Criar Espaço`.
6. Rodapé do rail:
   - `✓ Tudo salvo · agora há pouco`  (estado salvo sempre visível — inv#6)
   - Linha de aparência (ilustrativa): `Aparência: <valor do território> · segue o sistema`
     (T1: `Escuro (carvão)` · T2: `Escuro` · T3: `Claro (papel)` · T4: `Escuro profundo`).

### CANVAS
- Pill de atenção (padrão PULL — D3-A8), topo-centro:
  `◆ 1 pedido precisa de você — ir até lá` (clicável, cor do estado "precisa de você").
- Controles de zoom, canto inferior-direito: botões `Ver tudo` · `−` · `100%` · `+`
  (todos ≥24px; `Ver tudo` = zoom-to-fit com rótulo leigo).
- 5 cartões-terminal (disposição espacial idêntica nos 4 — grade solta com leves
  deslocamentos, cara de canvas, não tabela):
  - linha de cima: **Construtor de Páginas** (proeminente, à esquerda) ·
    **Redator de Vendas** (centro) · **Caixa e Pagamentos** (direita — é quem pede atenção).
  - linha de baixo: **Revisor de Qualidade** (esquerda) · **Automações** (direita, menor — recém-chegado).

### Anatomia do cartão-terminal (mesma nos 4 territórios)
1. Cabeçalho: nome do terminal + chip de estado (ícone+texto+cor) + meta curta.
2. Grid do terminal: SEMPRE dark em todos os territórios (âncora ADR 0019 §7 +
   precedente code-block-dark — A6h), fonte mono (JetBrains Mono).
3. Faixa de narração leiga (fora do grid, no chrome do cartão).

## 2. CONTEÚDO CANÔNICO (strings EXATAS — copiar verbatim)

### Vocabulário de estados (ícone + rótulo — mesmos glifos nos 4; cor vem do território)
- Trabalhando: `●` + `Trabalhando`
- Precisa de você: `◆` + `Precisa de você`
- Pronto: `✓` + `Pronto`
- Chegando: `○` + `Chegando`

### Cartão 1 — Construtor de Páginas  [estado: Trabalhando · viewer vivo]
- meta: `há 2 min nisso`
- narração: `Montando a seção de preços com os 3 planos.`
- grid (saída estilo Claude Code, é A saída de CLI real do despacho):
```
✻ Trabalhando na sua loja…

● Li a planilha  precos-planos.xlsx
  ⎿ 3 planos encontrados

● Editei  loja/secao-precos.html
  ⎿ 42 linhas ajustadas

Deixando os 3 planos lado a lado, com o
"Profissional" em destaque…
```

### Cartão 2 — Redator de Vendas  [estado: Trabalhando]
- meta: `há 5 min nisso`
- narração: `Construindo a página de vendas…`  ← string EXATA do despacho
- grid:
```
Título: "Sua loja no ar em 7 dias"

Testando 3 versões do subtítulo:
A — foco no prazo de entrega
B — foco na economia mensal
C — foco em "sem precisar de técnico"
```
- **SÓ NO T1:** este cartão é o periférico CONGELADO — grid esmaecido/sem vida +
  selo honesto sobreposto: `⏸ Pausado na tela — o trabalho continua`
  (a narração permanece viva/nítida). Nos demais territórios, cartão normal.

### Cartão 3 — Caixa e Pagamentos  [estado: Precisa de você · pede aprovação]
- meta: `esperando há 1 min`
- narração: `Esperando sua resposta para continuar.`
- grid (pergunta s/n visível, estilo prompt de terminal):
```
Configurei Pix e cartão no ambiente
de testes. Falta uma decisão sua:

Ativo o cupom LANCA10 — 10% de
desconto na primeira compra?

Permitir? (s/n) █
```
- botões no chrome do cartão (≥24px): `Sim, pode ativar` · `Agora não`
- este cartão é o destino da pill de atenção (badge/fila de atenção visível no cabeçalho).

### Cartão 4 — Revisor de Qualidade  [estado: Pronto]
- meta: `terminou há 3 min`
- narração: `Pronto — terminei a revisão`  ← string EXATA do despacho
- grid:
```
✓ Revisei as 5 páginas da loja
✓ Botões e links funcionando
✓ Nada quebrado no celular
✓ Carregamento rápido

Relatório completo guardado no Espaço.
```
- **SÓ NO T3:** este cartão carrega a celebração contida (1 momento de conclusão
  visível — ex.: selo/stamp estático `Bem feito ✓`, discreto, rotacionado de leve).

### Cartão 5 — Automações  [estado: Chegando · recém-spawnado]
- meta: `chegou agora`
- narração: `Acabou de chegar — conhecendo o trabalho do time.`
- grid (quase vazio, com cursor):
```
Olá! Sou o novo terminal de Automações.
Me apresentando ao time…  █
```

## 3. Territórios (pele + mecanismo — direção; o builder escolhe os hex EXATOS dentro de AA)

### T1 — Instrumento de Estúdio  → `t1-instrumento.html`
- Comentário de abertura (3 linhas):
  1. `T1 Instrumento de Estúdio — o Lina é um instrumento que o leigo aprende a tocar, não um painel que ele administra.`
  2. `Mecanismo: cor = UM significado fixo acoplado ao elemento acionável · viewer vivo SÓ no conjunto de foco (periferia congelada honesta) · flat honesto, função é a forma.`
  3. `Nunca: gradiente decorativo, sombra teatral, cor sem significado, animar input do usuário.`
- Chrome dark "carvão quente" (carvão com temperatura — base marrom-acinzentada quente,
  ex. família #221e19; NUNCA preto puro). Texto off-white quente. FLAT HONESTO: zero
  box-shadow; planos por degraus de tom + bordas 1px; radius pequeno (4px).
- Terminais sempre-dark, off-white sobre dark elevado (nunca #FFF/#000).
- Paleta semântica fixa (estilo RAL, ~4 cores, UM significado cada, em TODO o app):
  âmbar = trabalhando · verde = pronto · vermelho = precisa de você · azul = mensagem do time/chegando.
  ACOPLAMENTO OP-1: a cor do indicador É a cor do acionável — o chip vermelho do
  cartão 3, a pill de atenção e o botão `Sim, pode ativar` compartilham o MESMO vermelho;
  o cartão em foco tem anel/abas âmbar (trabalhando) etc.
- MECANISMO DE FOCO (correção onda V): cartão 1 = FOCO (nítido, vivo, levemente maior,
  anel âmbar); cartão 2 = periferia congelada com selo (ver §2). Os demais, presença normal.
- Tipografia: IBM Plex Sans (UI) · Fraunces (SÓ momentos — usar na narração de conclusão
  do cartão 4 e/ou no nome do Espaço) · JetBrains Mono (grids).
- Toggle ilustrativo de aparência no rodapé do rail (`Escuro (carvão) · segue o sistema`).

### T2 — Oficina de Precisão  → `t2-oficina.html`
- Comentário de abertura:
  1. `T2 Oficina de Precisão — a ferramenta que desaparece: chrome que recua, o trabalho é o protagonista.`
  2. `Mecanismo: monocromo + 1 único acento, reservado ao que precisa de você · densidade média-alta · zero ornamento.`
  3. `Nunca: display font, celebração, segunda cor, glow.`
- Monocromo dark-leaning (cinzas neutros; terminais um degrau mais escuros que o chrome).
- UM acento (escolha do builder — funcional, nem roxo nem azul-default-de-IA), usado
  EXCLUSIVAMENTE no estado "precisa de você" (chip do cartão 3 + pill de atenção +
  botões de aprovação). Trabalhando/Pronto/Chegando se distinguem por ícone+texto+pesos de cinza.
- Densidade média-alta: paddings menores, meta-linhas mais ricas (ex. `2 min · 14 ações`),
  rail mais compacto. Radius 2-3px, bordas hairline.
- Tipografia: Geist (UI — DECLARAR no comentário que é decisão, rota OFL, não inércia) ·
  sem display · JetBrains Mono (grids). Motion ~zero (só caret, com guard).

### T3 — Ateliê Caloroso  → `t3-atelie.html`
- Comentário de abertura:
  1. `T3 Ateliê Caloroso — a sala importa: um ateliê acolhedor onde os ajudantes trabalham à vista.`
  2. `Mecanismo: papel quente light-first · terminais como ilhas dark ancoradas (precedente code-block-dark) · celebração contida · cor de identidade do Espaço.`
  3. `Nunca: branco puro, festa permanente, esconder o terminal de verdade.`
- Light-first QUENTE: fundo papel/off-white quente (nunca #FFF), tinta escura quente (AA).
  Terminais = ilhas dark ancoradas (dark quente; âncora visual permitida: sombra suave
  com função de ancoragem — não é teatral aqui).
- Cor de identidade do Espaço (ex. terracota/argila) marcando o Espaço atual no rail
  e um detalhe do chrome. Estados em versão quente (sempre texto+ícone+cor).
- Densidade baixa, respiro generoso, radius 10-12px.
- Tipografia: Atkinson Hyperlegible Next (UI) · Bricolage Grotesque (display editorial —
  nome do Espaço, momentos) · JetBrains Mono (grids).
- Celebração contida no cartão 4 (ver §2). Demais cartões sem festa.

### T4 — Sala de Controle  → `t4-sala-controle.html`
- Comentário de abertura:
  1. `T4 Sala de Controle — o cockpit: telemetria total à vista, nada escondido.`
  2. `Mecanismo: dark profundo · densidade alta · números e fluxo de dados visíveis · glow de instrumentação contido ao que significa.`
  3. `Nunca: esconder números, glassmorphism, roxo decorativo, suavizar o sistema para parecer simples.`
- Dark profundo (azulado frio ok), painéis densos, linhas de grade discretas.
- Telemetria à vista: barra de status global no topo do canvas
  (`5 terminais · 2 trabalhando · 1 esperando você · 38 eventos/min`), faixa de
  telemetria POR cartão (ex. `2 min ativo · 14 ações · fila 0`), mini-sparklines em CSS puro.
- Acentos saturados de instrumentação (ciano/verde/âmbar/vermelho como cores de DADO);
  glow sutil SÓ em indicadores vivos (text-shadow/box-shadow leve — não é glassmorphism).
- Densidade ALTA por default — honesto e completo (sem capar; a decisão é do fundador).
- Tipografia: IBM Plex Sans (UI) · IBM Plex Mono (texturas pesadas/telemetria) ·
  Space Grotesk (acento de headings — DECLARAR no comentário que é decisão consciente,
  apesar do risco de clichê IA-branding).

## 4. INDEX (picker "Direct")  → `index.html`

- Design DELIBERADAMENTE NEUTRO (não pode enviesar a escolha): folha de especificação —
  papel neutro, tinta escura, JetBrains Mono para rótulos técnicos + UMA sans humanista
  discreta para leitura (declarar no comentário que a neutralidade é intencional).
- Conteúdo:
  1. Título: `Onde o Lina vai morar?` + subtítulo de 1 linha explicando: 4 territórios,
     a MESMA tela em cada um — só a pele muda; clique para entrar.
  2. 4 cards (clique abre o arquivo do território), cada um com:
     - nome + statement de 1 linha:
       - T1 `Instrumento de Estúdio` — `Um instrumento que você aprende a tocar: o trabalho do time à vista, cores que significam sempre a mesma coisa.`
       - T2 `Oficina de Precisão` — `A ferramenta que desaparece: quase tudo em cinza, uma única cor reservada para o que precisa de você.`
       - T3 `Ateliê Caloroso` — `Uma sala de trabalho clara e quente: os terminais são ilhas escuras ancoradas no papel.`
       - T4 `Sala de Controle` — `O cockpit completo: escuro profundo, números à vista, densidade máxima para quem quer ver tudo.`
     - as 5 palavras-atributo (insumo do deck de reaction cards F2-0-3):
       - T1: `vivo · honesto · preciso · legível · artesanal`
       - T2: `contido · sério · silencioso · focado · afiado`
       - T3: `caloroso · leve · acolhedor · humano · otimista`
       - T4: `denso · técnico · vigilante · potente · completo`
     - amostra mínima de pele (faixa de swatches/tipografia do território — pequena, igual nos 4 em tamanho).
  3. Avisos (1 linha cada):
     - `Estes protótipos NÃO decidem performance real nem animação — são telas paradas; velocidade e movimento se provam no app.`
     - `As fontes aqui vêm da internet (Google Fonts) só para o protótipo; no app de verdade elas são embarcadas (licença OFL).`
     - `Fusões são legítimas — ex.: T1 com a temperatura do T3. A escolha é o CENTRO DE GRAVIDADE, não uma prisão.`
     - Nota de transparência (pequena): `A pesquisa D1 defende o T1 — mas a tela não decide por você; os 4 estão completos e honestos.`
- Cards do picker: alvos grandes (card inteiro clicável), AA, sem ranking visual entre os 4
  (mesmo tamanho, mesma hierarquia).

## 5. Régua de verificação (o que a onda de verificação vai cobrar)

1. Strings canônicas do §2 presentes VERBATIM nos 4 territórios.
2. Comentário de abertura com as 3 linhas certas do território.
3. Zero marcador banido (grep: `7c3aed|a855f7|6366f1|backdrop-filter|Inter|Roboto|Arial`
  — Inter/Roboto/Arial só aceitos se claramente em stack de FALLBACK após a fonte escolhida; `system-ui`/`sans-serif` como último fallback é honesto, não escolha).
4. AA computado nos pares principais (chrome texto/fundo · narração/cartão · grid texto/fundo ·
   chips · botões): texto normal ≥4.5, grande/UI ≥3.0.
5. Alvos ≥24px (botões, chips clicáveis, setinha do rail, zoom).
6. Estados com texto+ícone+cor (nunca só cor).
7. T1: foco vivo + periferia congelada com selo EXATO. T3: celebração SÓ no cartão 4.
   T2: acento aparece SÓ no estado "precisa de você". T4: telemetria global + por cartão.
8. Autocontido: única dependência externa = fonts.googleapis.com.
9. Zero jargão técnico na superfície da UI (nada de spawn/handoff/PTY/busy/idle/A2A).
10. Pill de navegação do protótipo presente e discreta nos 4 + index linkando os 4.
