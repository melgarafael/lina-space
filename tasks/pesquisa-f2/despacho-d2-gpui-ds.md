# Despacho D2 — Design system em gpui (viabilidade e arquitetura)

## CONTEXTO
Fase 2 do Lina Space = UI/UX (fundador, 2026-06-12); o design system é entregável central. Stack travada (R1): gpui pinned (SHA `09165c15…`, vendorizado), wgpu/vello/cosmic-text/accesskit — sem JS/CSS. Âncora inegociável (R2): core/shell split via `UiHost` — a porta de troca para Slint fica viva; tokens não podem soldar o core ao gpui. Plano e restrições: `tasks/pesquisa-f2/_plano-pesquisa-f2.md`.

## FUNÇÃO
Arquiteto. Você responde "COMO se constrói um design system num app gpui sem fechar nossas portas" — decisão de estrutura antes de qualquer story de implementação.

## DIRECIONAMENTO
1. **Interno primeiro:** `13.3` §5 (Design Systems Rust/GPU — base já coletada, busque o delta), nosso código atual de UI (crates do shell gpui no repo — como hoje cores/espaçamentos estão espalhados? amostre 3-4 arquivos e DESCREVA a dívida), ADR 0019 (estética operacional), F1-5 (sonda [PROF], budget de frametime), `33 - Decisao de Framework de UI`.
2. **Externo (fonte primária = código real):** como o **Zed** estrutura theme/tokens/componentes em gpui (repo aberto: theme registry, JSON de temas, StyledExt, ui crate) — datado e com links para arquivos; o que a comunidade gpui oferece (gpui-component e similares, 2025-2026: maturidade real, issues abertas, dá para depender ou só modelar?); como apps Rust não-gpui resolvem tokens multi-backend (Iced/Slint/Makepad) — pela porta R2.
3. Responda com posição: (a) tokens como **dados** (TOML/JSON no core, shell consome) vs tokens como código no shell — qual respeita R2 com menos cerimônia? (b) theming dark/light + customização futura do usuário: o que o gpui pinned suporta hoje? (c) catálogo de componentes próprio (botão/painel/menu/card/badge): quanto custa em gpui, e o que o Zed nos dá de graça por modelagem? (d) qual o caminho de migração incremental do shell atual SEM big-bang (a F2 refatora por onda)?
4. Quality gates do plano (datar, refutar, fetch real, piso 5/teto 15). Armadilha: docs/README de lib imatura prometem mais do que o issue tracker confessa — leia issues antes de recomendar dependência.

## OBJETIVO
O épico F2 sabe ONDE o design system mora (core-data vs shell-code), o que reutilizamos do ecossistema, e o caminho incremental — sem violar R1/R2/R7.

## RESULTADO ESPERADO
`tasks/pesquisa-f2/entrega-d2-gpui-ds.md`: 5-8 achados (CLAIM | FONTE+URL | DATA | CONFIANÇA | REFUTAÇÃO | SUSPECT | LOAD-BEARING) + recomendação de arquitetura (a-d acima, com posição e trade-offs) + CONFLITOS · LACUNAS · RECÊNCIA. Reporte via `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status`. Última linha `PRONTO:`/`BLOCKED:`. Não commite; não edite o vault.

## Tentativas anteriores
Nenhuma — 1º despacho desta pesquisa.
