# Entrega F2-0-D — Protótipos dos 4 territórios estéticos

> **Sessão de decisão do fundador (F2-0-7)** · Dono: Terminal C (FRONTEND) · Data: 2026-06-12
> **Artefatos:** `tasks/pesquisa-f2/prototipos/` — `index.html` (picker "Direct") + `t1-instrumento.html` · `t2-oficina.html` · `t3-atelie.html` · `t4-sala-controle.html`
> **Como abrir:** duplo-clique no `index.html`. Sem build, sem framework, zero JS; única dependência externa = Google Fonts CDN (o app real embarca estáticos OFL — avisado no index).

## O que foi entregue

- **A MESMA tela nos 4** (comparação justa): canvas do Lina com 5 terminais em estados
  verdadeiros da F1 — Construtor de Páginas (Busy, saída estilo Claude Code), Redator de
  Vendas (Busy, narração "Construindo a página de vendas…"), Caixa e Pagamentos (pede
  aprovação, pergunta s/n + botões), Revisor de Qualidade (Idle, "Pronto — terminei a
  revisão"), Automações (recém-spawnado) + rail com Buscar/⌘K (D4), 3 Espaços com ação
  Descarregar, Criar Espaço, "Tudo salvo" sempre visível, pill de atenção (pull, D3-A8)
  e zoom-to-fit ("Ver tudo"). Conteúdo canônico fixado em `_spec-prototipos.md` (strings
  verbatim — auditoria transversal confirmou identidade entre os 4).
- **Mecanismo encarnado, não figurino** (cada arquivo abre com o comentário de 3 linhas):
  - **T1:** cor semântica fixa acoplada ao acionável (o vermelho do chip = da pill = do botão
    "Sim"); foco vivo + periferia congelada com selo "⏸ Pausado na tela — o trabalho continua"
    (correção da onda V honrada); flat honesto; IBM Plex Sans + Fraunces (momentos) + JetBrains Mono; carvão quente.
  - **T2:** monocromo; acento ÚNICO (#F08A3C) reservado ao "precisa de você"; Geist declarado
    (rota OFL, não inércia); densidade média-alta; sem display font.
  - **T3:** papel quente (nunca #fff) + ilhas dark; Atkinson Hyperlegible Next + Bricolage
    Grotesque; terracota como cor de identidade do Espaço; celebração contida (selo "Bem feito ✓"
    só no cartão 4, carimbando o canto sem cobrir texto).
  - **T4:** dark profundo, telemetria global + por cartão, sparklines CSS, glow contido ao
    significado; honesto e completo (sem capar — decisão é do fundador).
- **Index = picker "Direct":** 4 cards de mesmo peso (sem enviesar), statement de 1 linha,
  5 palavras-atributo por território (insumo do deck F2-0-3), avisos do que o protótipo NÃO
  decide (perf/motion), nota de fusão legítima (T1 com temperatura do T3) e nota de
  transparência (a D1 defende o T1). Design deliberadamente neutro (folha de especificação,
  Source Sans 3 + JetBrains Mono).

## Método e verificação (evidência observada)

1. Spec canônica de conteúdo (`_spec-prototipos.md`) escrita ANTES de qualquer pixel.
2. 5 builders paralelos → 5 revisores céticos isolados (AA **computado** via python3 nos pares
   principais — mínimos registrados nos comentários de cada arquivo; alvos ≥24px; strings
   verbatim; mecanismo; anti-slop) → correções aplicadas → auditoria transversal de identidade
   (6 divergências achadas e corrigidas, ex.: "aberto"→"atual" no T2, badge "◆ 1" harmonizado).
3. Inspeção visual final no navegador (screenshots dos 5 arquivos + zoom nos momentos-chave:
   selo de congelado do T1, celebração do T3, monocromia do T2) — 1 conserto a quente:
   selo do T3 reposicionado para não cobrir a frase canônica.
4. Gates mecânicos: zero marcador banido (gradiente roxo/glassmorphism/Inter-como-escolha),
   zero `<script>`, única dependência = Google Fonts, links do index conferidos.
5. Calibragem com a curadoria OpenDesign do Maestro (referências estreladas usadas como
   estudo de mecanismo; T1 manteve o carvão QUENTE da D1 onde as referências são frias).

## O que estes protótipos NÃO decidem

Performance real, motion/animação, comportamento de resize/zoom — são telas paradas.
A decisão que eles alimentam é UMA: o centro de gravidade estético do Lina (F2-0-7).

PRONTO: 5 arquivos em tasks/pesquisa-f2/prototipos/ — 4 territórios com a mesma tela canônica + index picker "Direct"; AA computado, identidade de conteúdo auditada, inspeção visual feita; abre com duplo-clique.
