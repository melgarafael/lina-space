# DESPACHO — fatia `bf-toasts` (Família Toasts / Notificações)

**CONTEXTO** · Leia `tasks/despachos/bugs-f2/_contexto.md` + `tasks/despachos/_regras-comuns.md` primeiro.
Há dois painéis flutuantes: "Fila de Atenção" (pedidos/alertas dos terminais; tem botão "→ Ir até o
terminal") e "Atividade e custo do time" (dashboard). Bugs: (T1) o botão "ir até" não faz nada; (T2) os
dois painéis se sobrepõem — sem camadas/organização que deixe os dois abertos de forma agradável.

**FUNÇÃO** · Você é o dono do layout/geometria e da clareza dos painéis flutuantes de notificação.

**DIRECIONAMENTO** · Fronteira de arquivos (SÓ estes):
- `app/lina-gpui/src/attention_ui.rs`  (render do toast/painel "Fila de Atenção" + botão)
- `app/lina-gpui/src/dashboard.rs`      (geometria do painel "Atividade e custo": `dashboard_panel_rect`)
- (NÃO toque `main.rs`; o handler `attention_goto_node` (~1898) e a COMPOSIÇÃO/ordem dos painéis (~4477)
  moram lá → registre pedido de costura na entrega com diff textual preciso.)

Tarefas:
1. **T2 — sobreposição (dashboard.rs + attention_ui.rs).** Os dois painéis colidem em viewports estreitas
   (toast: right16/bottom56/largura 480; dashboard: ancorado à direita, ~340 de largura). Defina geometrias
   que NÃO se sobreponham: ou empilhe verticalmente na mesma coluna direita com gap, ou dê colunas
   distintas, com regra clara quando ambos abertos. Entregue funções de geometria PURAS e testáveis
   (recebem viewport → retornam rects que não se interceptam). Prove com teste de não-interseção.
2. **T1 — botão "ir até o terminal" (diagnóstico).** O handler real (`attention_goto_node`) está em main.rs
   (do Maestro): ele faz `find(name)`→`focus`+`reveal`+`snooze`. Sua parte: garantir que o botão em
   `attention_ui.rs` dispara a ação certa e que o `name`/identificador passado casa com o roster (causa
   provável do no-op: nome não bate, ou o item é `goto_only` e o branch de render/click está errado).
   Documente na entrega a causa-raiz e, se o fix exige mudar o handler em main.rs, entregue o diff de costura.

**OBJETIVO** · Com os dois painéis abertos, nada se sobrepõe (camadas claras, gap, sem colisão);
clicar "→ Ir até o terminal" sempre leva ao terminal certo (ou a causa-raiz documentada + diff de costura).

**RESULTADO ESPERADO** · `tasks/despachos/bugs-f2/.entrega-bf-toasts.md`: arquivo:linha, evidência
(`cd app/lina-gpui && cargo test`/`clippy`/`fmt` exit DIRETO + nº testes, incl. teste de não-interseção),
pedidos de costura em main.rs (diff preciso), achados. Última linha `PRONTO`/`BLOCKED`.
