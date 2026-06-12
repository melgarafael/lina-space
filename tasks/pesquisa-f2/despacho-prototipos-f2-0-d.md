# Despacho F2-0-D — Protótipos dos 4 territórios estéticos (sessão de decisão do fundador)

## CONTEXTO
Épico F2 escrito (vault `38`), kickoff bloqueado pela decisão de território (F2-0-7). Sua entrega D1 definiu os 4 territórios (`tasks/pesquisa-f2/entrega-d1-identidade.md` §II) — agora o fundador precisa VÊ-LOS para escolher. Decisão de pesquisa relevante: o picker "Direct" (3-5 direções nomeadas antes de gerar) é o formato; os protótipos são o conteúdo dele. Correção da onda V que você deve honrar: no T1, viewer vivo SÓ no conjunto de foco — periferia congelada (F1-5).

## FUNÇÃO
Designer de protótipos de alta fidelidade. Você NÃO está construindo o app — está construindo o instrumento de DECISÃO do fundador.

## DIRECIONAMENTO
1. **Artefato:** 4 mockups HTML estáticos autocontidos + 1 `index.html` (o picker "Direct": 4 cards nomeados com statement de 1 linha cada, clique abre o território). Pasta: `tasks/pesquisa-f2/prototipos/` (`t1-instrumento.html`, `t2-oficina.html`, `t3-atelie.html`, `t4-sala-controle.html`, `index.html`). Sem build, sem framework — abre com duplo-clique. Fontes via Google Fonts CDN é aceitável NESTE protótipo (declare no index que o app real embarca estáticos OFL).
2. **A MESMA TELA nos 4** (comparação justa — só a pele muda, o conteúdo é idêntico): o canvas real do Lina com **5 terminais** em estados verdadeiros da F1 — 2 Busy (um com saída de Claude Code visível no grid, outro com narração leiga "Construindo a página de vendas…"), 1 que PEDE APROVAÇÃO (badge/fila de atenção, pergunta y/n visível), 1 Idle ("Pronto — terminei a revisão"), 1 recém-spawnado. + sidebar/rail com ações rotuladas, botão "Buscar" visível (decisão D4), indicador de Espaço, zoom-to-fit. Conteúdo em pt-br leigo (R4; zero jargão na superfície).
3. **Cada território encarna o MECANISMO da D1, não o figurino:**
   - **T1 Instrumento de Estúdio:** cor semântica fixa (âmbar=trabalhando · verde=pronto · vermelho=precisa de você · azul=mensagem do time) acoplada ao elemento acionável; terminais como viewers vivos NO FOCO (na tela: foco nítido + 1 periférico visivelmente congelado com selo honesto "pausado na tela"); flat honesto; IBM Plex Sans + Fraunces (momentos) + JetBrains Mono no grid; terminais sempre-dark, chrome "carvão quente" (mostre o chrome dark; um toggle ilustrativo basta).
   - **T2 Oficina de Precisão:** monocromo + 1 acento; chrome que recua; densidade média-alta; sem display font; o mais contido dos 4.
   - **T3 Ateliê Caloroso:** light-first quente (papel, nunca branco puro) com terminais como ilhas dark ancoradas (o precedente code-block-dark); Atkinson Hyperlegible Next + display editorial; celebração contida (1 momento de conclusão visível); cor de identidade do Espaço.
   - **T4 Sala de Controle:** dark profundo, densidade alta, telemetria à vista — honesto e completo, mesmo você não o recomendando (a decisão é dele; capar o T4 viesaria a escolha).
4. **Anti-slop:** nada do vocabulário banido (gradiente roxo decorativo, glassmorphism, Inter-por-inércia). Cada arquivo abre com comentário HTML de 3 linhas: statement do território + o mecanismo encarnado + o que NUNCA faz.
5. **No index, além do picker:** aviso de 1 linha sobre o que o protótipo NÃO decide (perf real, motion — são mockups estáticos) e as 5 palavras-atributo de cada território (insumo do deck de reaction cards da F2-0-3).
6. Honre a régua na medida do estático: contraste AA nos pares de texto (cheque os principais), alvos ≥24px, texto+ícone+cor nos estados.

## OBJETIVO
O fundador abre o index, navega os 4, e escolhe o centro de gravidade do Lina com convicção — inclusive fusões (T1 com temperatura do T3 é legítima — declare isso no index).

## RESULTADO ESPERADO
Os 6 arquivos na pasta + reporte via `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status` ao começar e ao terminar. Última linha do seu reporte final: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Não commite.

## Tentativas anteriores
Nenhuma.
