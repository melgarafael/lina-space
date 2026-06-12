# Despacho D0 — Régua/Eval de UX da Fase 2 (PRIMEIRO da onda)

## CONTEXTO
A Fase 1 do Lina Space fechou; a Fase 2 é **UI/UX** (decisão do fundador 2026-06-12): visual único e "viciante", design system em gpui, navegabilidade, organizar/redimensionar terminais no canvas, menus/comandos, áreas de visibilidade (skills/agents/MCPs). Plano completo e restrições internas R1-R11: leia `tasks/pesquisa-f2/_plano-pesquisa-f2.md` (mesmo repo). O usuário-alvo é **leigo não-técnico** (inv#6). Antes de qualquer arquitetura de design, precisamos da RÉGUA: sem ela, "ficou bonito" vira opinião.

## FUNÇÃO
QA / engenheiro de avaliação. Você define COMO a F2 será medida — a régua que o épico vai usar como critério de aceite observável de cada story.

## DIRECIONAMENTO
1. **Interno primeiro (obrigatório):** leia `docs/adr/0019` (definições operacionais — já inclui estética), a rubrica anti-slop (`assets/lina-doctrine/` e skills lina-cold-review/lina-design-doctrine), e o padrão de critério de aceite do épico F1 (vault `34 - Epico Fase 1`, qualquer story: sempre "algo que roda e se mede"). A régua da F2 deve COMPOR com isso, não duplicar.
2. **Externo (delta):** como produtos de referência medem qualidade visual/UX de forma operacional — ex.: testes de 5 segundos/desirability (Microsoft Reaction Cards), task-success/time-on-task para usuário leigo (SUS, SEQ, NN/g), métricas de "delight"/retenção SEM dark pattern (o fundador quer "viciante" = fluxo/prazer, não casino), benchmarks de percepção de performance (frametime p95, input latency), conformidade a11y mensurável (WCAG 2.2 nível AA aplicado a desktop nativo/AccessKit).
3. Quality gates do plano valem integralmente (datar tudo, refutar, fetch real, piso 5/teto 15 buscas).
4. Proposta final: **régua em camadas** — (a) rubrica anti-slop PASS (já existe), (b) métricas de tarefa do leigo (quais? com alvo numérico), (c) métricas de percepção/distintividade (quais? como rodar barato com 1 fundador + poucos testers), (d) budget de perf (números do F1-5 como baseline), (e) a11y. Cada métrica: como medir, custo de medir, e o que é PASS.

## OBJETIVO
A síntese e o épico F2 ganham critérios de aceite observáveis por story de UI — "único e viciante" vira número/procedimento, não adjetivo.

## RESULTADO ESPERADO
Arquivo `tasks/pesquisa-f2/entrega-d0-eval.md` com: 5-8 achados no formato CLAIM | FONTE+URL | DATA | CONFIANÇA | REFUTAÇÃO TENTADA | SUSPECT | LOAD-BEARING; depois a **proposta de régua em camadas** (tabela métrica→como medir→PASS); CONFLITOS · LACUNAS · NOTA DE RECÊNCIA. Reporte começo/fim/travamento: `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status`. Última linha: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Não commite; não edite o vault.

## Tentativas anteriores
Nenhuma — 1º despacho desta pesquisa.
