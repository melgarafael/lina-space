# DESPACHO — fatia `bf-modais` (Família Modais / Overflow)

**CONTEXTO** · Leia `tasks/despachos/bugs-f2/_contexto.md` + `tasks/despachos/_regras-comuns.md` primeiro.
A F2 é a onda de UI/UX. Existe um wrapper canônico de modal CORRETO (`app/lina-gpui/src/ui/modal.rs`)
com contenção de overflow (`min_h(0)+overflow_y_scroll()` no corpo, `clamp_frame`, véu `.occlude()`).
O problema: nem todo modal o usa. O usuário documentou modais que estouram a viewport e um popover/modal
cujo clique vaza para o terminal atrás.

**FUNÇÃO** · Você é o dono da contenção de overflow e oclusão dos modais. Frente de UI gpui.

**DIRECIONAMENTO** · Fronteira de arquivos (SÓ estes):
- `app/lina-gpui/src/ui/modal.rs`
- `app/lina-gpui/src/persistence_ui.rs`  ("Espaços & Ajustes")
- `app/lina-gpui/src/agent_modal.rs`     (M6 — occlude/véu)
- (NÃO toque `main.rs`; `render_create_space` (M9) e o popover de toolbar do nó moram lá — registre
  pedido de costura na entrega com diff textual preciso p/ o Maestro.)

Tarefas:
1. **M1 — "Espaços & Ajustes" estoura (persistence_ui.rs).** Aplicar a DOUTRINA de overflow (ver _contexto):
   header fixo (título "Espaços & Ajustes" + ✕) · corpo rolável (`flex_1 min_h(px(0.)) overflow_y_scroll`)
   · max-altura relativa à viewport. Prefira REUSAR o padrão do `ui/modal.rs`; se for janela própria
   (não overlay), aplique as mesmas regras no conteúdo raiz. Não invente um 2º padrão.
2. **M2 — occlude/véu (agent_modal.rs + ui/modal.rs).** Garanta que TODO modal do `Modal` wrapper
   bloqueie cliques no que está atrás (véu com `.occlude()`), para o clique não vazar ao terminal/canvas.
   Decida: ligar `occlude` por padrão no wrapper (mais seguro) é preferível a ligar caso-a-caso — registre
   a decisão na entrega. Confirme que ligar o véu não regride o visual (o usuário não quer escurecer demais;
   um véu transparente-mas-capturador resolve a captura sem mudar a aparência).
3. **Doutrina como gate (opcional, se sobrar):** o `ui/modal.rs` deve ter um teste que prove a contenção
   (frame nunca > viewport; corpo tem min_h(0)+scroll). Se já existe, estenda; senão, registre como achado.

**OBJETIVO** · Nenhum modal/painel do app cresce além da viewport; o corpo rola com header/footer fixos;
clique em modal nunca atinge o que está atrás. "Espaços & Ajustes" passa o teste de 3× a altura da tela.

**RESULTADO ESPERADO** · `tasks/despachos/bugs-f2/.entrega-bf-modais.md`: arquivo:linha das mudanças,
evidência de validação (`cd app/lina-gpui && cargo test`/`clippy`/`fmt` — exit DIRETO, nº de testes),
pedidos de costura em main.rs (diff preciso p/ M9 `render_create_space` e popover de toolbar do nó),
achados/riscos. Última linha `PRONTO` ou `BLOCKED: <motivo>`.
