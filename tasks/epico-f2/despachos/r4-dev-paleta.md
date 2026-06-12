# Despacho r4 — Terminal E: F2-2-4 — porta visível da paleta + ranking previsível

## CONTEXTO
Onda F2-2 (épico vault `38` §F2-2). A pesquisa D4-A1 (auditada) provou: paleta escondida = paleta morta (caso GitHub) — e a nossa ⌘K (`palette.rs`, W4-2: fuzzy por subsequência, sem ranking) está exatamente no estado pré-morte: NENHUMA porta de entrada visível. D4-A2 deu o blueprint mecânico verificado no código do Zed.

## FUNÇÃO
Developer — dono de `palette.rs` + `sidebar.rs` nesta rodada (fronteira sua; o C está em `src/ui/` e NÃO toca seus 2 arquivos).

## DIRECIONAMENTO
1. **Porta visível (search-first, modelo Notion):** botão permanente e ROTULADO no rail/sidebar — rótulo leigo ("Buscar" — confirme contra a fonte única de strings R9; se a string não existir lá, registre o pedido no reporte para a costura) com o atalho exibido no botão (⌘K). Clique abre a paleta existente.
2. **Ranking previsível (D4-A2, nesta ordem):** (a) alias estrito por prefixo (sinônimos técnicos como keywords invisíveis — "Gatilho" acha por "webhook"); (b) MRU curto e ESTÁVEL (persistido localmente); (c) fuzzy smart-case existente; (d) hit-count como desempate — **como projeção do event log** (`SkillInvoked`-like; veja se a família de evento de uso existente cobre; se precisar de evento novo, especifique no reporte — events.rs é costura minha, NÃO edite).
3. **Filtro por contexto** (padrão Zed `CommandPaletteFilter`): ações inaplicáveis ao estado atual ficam FORA da lista (não acinzentadas).
4. Modelo puro testável (como o palette.rs já faz): ranking/alias/MRU testados sem gpui.
5. Regra dura da fase (candidata a invariante): *nada existe só atrás de atalho* — esta story é a materialização dela; documente no doc do módulo.
6. Validação: suíte + catraca + clippy -D warnings + fmt nos seus arquivos. Não commite. "Reporte E continue no mesmo turno."

## OBJETIVO
O leigo descobre o que o Lina faz CLICANDO num botão rotulado — e quem digita ganha ranking que nunca surpreende.

## RESULTADO ESPERADO
palette.rs+sidebar.rs com porta/ranking/filtro + testes + (se houver) spec de evento/string para minha costura. Reporte começo/fim com `--intent status`. Última linha `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
