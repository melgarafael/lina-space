# Despacho r6 — Terminal E: registry de ações + componente da toolbar (F2-2-5, metade 1)

## CONTEXTO
Rodada 6. Spec SELADA em tasks/epico-f2/spec-f2-2-5-toolbar.md (Terminal A — siga-a à risca; conflito = pare e me escale, não adapte). Notas da rodada: tasks/epico-f2/notas-r6.md. Strings: o @Redator entrega COPY_TB_* em paralelo (use placeholders dele; reconcilie no fim ou marque TODO-copy).

## FUNÇÃO
Developer — fronteira: palette.rs (seu de volta) + src/ui/node_toolbar.rs (novo). NÃO toque main.rs (C é dono único de novo — a montagem/anchor é dele).

## DIRECIONAMENTO
1. palette.rs: seletor node_commands(NodeCtx) -> Vec<Command> conforme a spec (3-4 ações por estado: Atender=GotoAtencao NAVEGA-nunca-aprova · Editar=EditAgent · Centralizar=FocusNode · Encerrar=CloseNode); FIE o Command::when de verdade (remove o allow(dead_code) anotado — o consumidor real chegou).
2. ui/node_toolbar.rs: componente Button-rotulado (NUNCA icon-only), tokens, alvo ≥24px, consumindo node_commands; modelo puro testável (dado NodeCtx → ações certas); ação executa via o MESMO caminho da paleta (PaletteCommandInvoked no log — zero evento novo).
3. Testes puros (node_commands por estado + when fiado) + os critérios de aceite da spec.
4. Validação: suíte + catraca (dívida zero) + clippy -D + fmt nos seus. Não commite. Reporte E continue.

## OBJETIVO / RESULTADO ESPERADO
Registry único alimentando 2ª superfície sem duplicação. Entrega + spec de montagem p/ o C (anchor já está na spec do A — só aponte o que seu componente expõe). PRONTO:/BLOCKED:.

## Tentativas anteriores
Nenhuma.
