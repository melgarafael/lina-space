# Despacho D3 — UX de canvas: navegação, organização, dimensionamento

## CONTEXTO
Fase 2 do Lina Space = UI/UX (fundador, 2026-06-12). Pedido explícito do debriefing: "poder organizar, redimensionar e mover os terminais no canvas". Hoje o canvas tem nós de terminal, navegação por teclado (F1-2-6), zoom básico e o rail de ações; falta a camada de ORGANIZAÇÃO espacial digna de Figma. Restrição estrutural R5: posição/tamanho/arranjo = evento no log + projeção (reconstruível após crash); R3: a cena precisa manter o slot do browser-portal futuro; R7: 120Hz. Plano: `tasks/pesquisa-f2/_plano-pesquisa-f2.md`.

## FUNÇÃO
Pesquisador de UX de canvas/spatial interfaces. Você traz os padrões de interação decision-grade para a F2 — o que copiar, o que evitar, e o que inventar.

## DIRECIONAMENTO
1. **Interno primeiro:** vault `13.3` §1-2 (padrões tldraw/Figma/Flowith já mapeados — DELTA apenas), `22 - Fluxo de Telas`, e o roteiro de telas r5 (`tasks/` — grep "roteiro"). O que o fundador já viu/validou na tela conta como decisão.
2. **Externo (2025-2026, fonte = produto shipped + praticantes):** padrões de canvas espacial em ferramentas reais — Figma/FigJam (multiplayer canvas, smart selection, auto-layout espacial), tldraw (geometria/snap), Miro (frames/áreas), Obsidian Canvas (grupos), Warp/terminal grids (blocos), e gerenciadores de janela tiling (PaperWM/Niri/Aerospace — scrollable/tiling workspaces) como contraponto ao free-form. Para CADA padrão: o gesto/atalho, custo de implementação num canvas próprio, e a armadilha conhecida (reclamação real de usuário).
3. Responda em específico: (a) redimensionar terminal vivo (PTY reflow!) — como produtos tratam resize de conteúdo vivo sem jank; (b) organização: snap-to-grid vs alinhamento inteligente vs frames/grupos nomeados — o que serve a um LEIGO com 3-9 terminais (não 1000 shapes); (c) navegação: zoom semântico (overview→foco), minimap, "ir para o terminal que pede atenção" — o que reduz o custo de atenção do leigo; (d) persistência: que eventos de layout outros canvas guardam (R5 pede evento+projeção).
4. Quality gates do plano (datar, refutar, fetch real, piso 5/teto 15). Armadilha: feature-parity com Figma é trap — nosso usuário tem N≤9 nós grandes e vivos, não milhares de shapes mortos; filtre por esse job.

## OBJETIVO
A onda de canvas do épico F2 nasce com padrões escolhidos por evidência (gesto, atalho, evento de layout) — não por "Figma faz assim".

## RESULTADO ESPERADO
`tasks/pesquisa-f2/entrega-d3-canvas-ux.md`: 5-8 achados (CLAIM | FONTE+URL | DATA | CONFIANÇA | REFUTAÇÃO | SUSPECT | LOAD-BEARING) + respostas posicionadas (a-d) + CONFLITOS · LACUNAS · RECÊNCIA. Reporte via `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status`. Última linha `PRONTO:`/`BLOCKED:`. Não commite; não edite o vault.

## Tentativas anteriores
Nenhuma — 1º despacho desta pesquisa.
