# Despacho r1 — Terminal A (ARQUITETO): F2-0-4 selar ADR 0028 + F2-0-5 nota-ADR câmera

## CONTEXTO
Execução da F2 iniciada (épico: vault `38 - Epico Fase 2`; decisões F2-0-D seladas no §VIII). Dois ADRs-gate bloqueiam ondas futuras: o 0028 (DRAFT) bloqueia F2-2-2 (toast/badge sobre live-region) e a nota da câmera bloqueia F2-3-7 (persistência de layout). Pesquisa-fonte: `tasks/pesquisa-f2/entrega-d2-gpui-ds.md` (A6) e `entrega-d3-canvas-ux.md` (A9 + resposta d), com vereditos em `entrega-v-verificacao.md` (§II.5).

## FUNÇÃO
Arquiteto — dono da série de ADRs.

## DIRECIONAMENTO
1. **F2-0-4 — selar `docs/adr/0028-live-region-caminho.md`:** DRAFT→Aceito. Antes de selar, re-cheque o pressuposto contra o vendor REAL do pin (a V-D2 confirmou 15 campos `aria_*`/zero live no checkout `09165c15` — referencie essa evidência). Acrescente a consequência nova da F2: todo componente do catálogo F2-2 que comunica estado (toast/badge/progresso) nasce compondo com o custom Element — retrofit proibido. Mantenha a cláusula "checar `set_live` upstream a cada bump".
2. **F2-0-5 — nova nota-ADR (próximo número livre):** "Câmera/viewport fora do event log". Decisão: posição/tamanho/organização de TERMINAL = eventos transacionais no log (R5/inv#4); câmera/zoom do viewport = estado de SESSÃO local (snapshot fora da stream do Espaço), honrando "mesmo zoom pós-crash" da nota 22. Fundamento: unânime em 5 organizações independentes (tldraw/Excalidraw/Figma/Liveblocks/JSON Canvas — D3-A9, verificado). Inclua: contrato dos eventos `TerminalMovido`/`TerminalRedimensionado` (estado FINAL, 1 gesto = 1 evento, cancelou = zero evento) + z-order por fractional index + limite explícito (se foco/câmera um dia precisar ser fato auditável, é evento aditivo novo — espelha o precedente do `last_focus`, addendum ADR 0010).
3. Fronteira: SÓ `docs/adr/`. Não toque em código nem em `tasks/` de outros.
4. Não commite. Valido de fora e commito.

## OBJETIVO
F2-2-2 e F2-3-7 nascem com contrato selado — zero re-decisão no meio da onda.

## RESULTADO ESPERADO
2 arquivos em `docs/adr/`. Reporte começo/fim: `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status`. Última linha: `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
