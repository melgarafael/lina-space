# DESPACHO — fatia `bf-sidebar` (Família Sidebar + affordance)

**CONTEXTO** · Leia `tasks/despachos/bugs-f2/_contexto.md` + `tasks/despachos/_regras-comuns.md` primeiro.
O sidebar de Espaços lista workspaces ("N Agentes · ~US$ X · ⌘k · Renomear") com ações (Renomear,
Arquivar, Descarregar) e rodapé ("Espaços arquivados", "+ Novo Espaço"). Bug: clipping/truncation — a
ação "Arquivar" não aparece, e o nome é cortado silenciosamente sem affordance (o usuário não percebe que
há mais texto / mais ações). Mesmo tipo de clipping na barra superior dos terminais.

**FUNÇÃO** · Você é o dono da legibilidade e affordance do sidebar de Espaços.

**DIRECIONAMENTO** · Fronteira de arquivos (SÓ estes):
- `app/lina-gpui/src/sidebar.rs`  (linha do item ~1117-1254; `.overflow_hidden()` ~1141)
- (NÃO toque `main.rs`; a barra superior dos terminais e os botões "Centralizar"/"Encerrar" moram lá →
  são costura do Maestro. Se o mesmo padrão de clipping da top-bar precisar de fix em main.rs, registre o
  diagnóstico + diff de costura na entrega.)

Tarefas:
1. **S1 — clipping/truncation do item (sidebar.rs).** A linha do item usa `.overflow_hidden()` no nome
   sem `w_full()`/`min_w(px(0.))` no container flex, então quando aparecem agentes+custo+atalho+3 botões,
   o nome é cortado e/ou os botões somem. Conserte o layout: nome com truncamento ELEGANTE e affordance
   (ellipsis visível + tooltip/título completo no hover), e as ações (Renomear/**Arquivar**/Descarregar)
   SEMPRE acessíveis (não somem por overflow). Use `min_w(px(0.))` no filho flexível, `flex_shrink`
   correto nos botões, e `text_ellipsis`/truncate gpui em vez de corte mudo.
2. **Affordance.** A "Arquivar" precisa ser DESCOBRÍVEL — decida o padrão (ações sempre visíveis vs menu
   "⋯" explícito) e banque um. Sem slop: nada de hover-only invisível que o leigo nunca acha.

**OBJETIVO** · Em qualquer largura do sidebar e com qualquer combinação de ações, o nome do Espaço é
legível (ellipsis + título no hover) e TODAS as ações — incluindo "Arquivar" — estão sempre acessíveis e
descobríveis. Nada é cortado silenciosamente.

**RESULTADO ESPERADO** · `tasks/despachos/bugs-f2/.entrega-bf-sidebar.md`: arquivo:linha, evidência
(`cd app/lina-gpui && cargo test`/`clippy`/`fmt` exit DIRETO + nº testes), diagnóstico+diff de costura para
o clipping da top-bar (se aplicável, em main.rs), achados. Última linha `PRONTO`/`BLOCKED`.
