# Despacho r5 — Terminal A (ARQUITETO): design/spec da F2-2-5 — toolbar contextual do nó

## CONTEXTO
Rodada 5. A F2-2-5 (toolbar de 3-5 ações rotuladas no terminal/card selecionado — pesquisa D4-A3: visível-rotulado > contextual redundante > busca > atalho; modelo Canva, nunca ribbon nem icon-only) será CONSTRUÍDA na r6 — esta story é a DECISÃO DE ESTRUTURA antes do código, seu papel canônico.

## FUNÇÃO
Arquiteto — entregável é SPEC, não código. Fronteira: `tasks/epico-f2/spec-f2-2-5-toolbar.md` (novo).

## DIRECIONAMENTO
1. **Onde a toolbar vive na CENA:** overlay do card focado? camada própria? Respeite R3 (slot do PortalEngine — LOD degrada fidelidade, nunca remove camada de composição) e a invariante "câmera nunca se move sozinha".
2. **Quais 3-5 ações** (por estado do nó: trabalhando/pede-aprovação/pronto/novo) — cruze com a fila de atenção da F1 e a tecla "o"; ações vêm da MESMA fonte dos comandos da paleta (1 registry, 2 superfícies — zero duplicação de definição de ação).
3. **Contrato com o catálogo:** Button do ui/ (variantes por significado); strings pela fonte única; hover-only PROIBIDO (pista permanente — tie-breaker da pesquisa: kebab visível).
4. **Eventos**: clique em ação da toolbar emite o quê no log? (provavelmente reusa PaletteCommandInvoked com key da ação — decida e justifique; aditivo).
5. **Fronteiras da r6:** que arquivos a construção tocará e quem é dono natural.
6. a11y: foco/navegação por teclado da toolbar (não pode quebrar o canvas por teclado da F1-2-6).

## OBJETIVO
A r6 constrói a toolbar sem UMA pergunta de esclarecimento — e sem re-decidir nada no meio.

## RESULTADO ESPERADO
Spec completa no arquivo. PRONTO:/BLOCKED:.

## Tentativas anteriores
Nenhuma.
