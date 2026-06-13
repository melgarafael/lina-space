# DESPACHO — fatia `bf-terminal` (Família Terminal: render + input)

**CONTEXTO** · Leia `tasks/despachos/bugs-f2/_contexto.md` + `tasks/despachos/_regras-comuns.md` primeiro.
O app embute o Claude Code CLI num terminal vivo (alacritty_terminal atrás do trait `VtBackend`,
render gpui/wgpu). O usuário comparou com o terminal "real" do Claude e apontou diferenças. Esta é a
fatia mais core — por isso vai para o Backend/Ultra Code.

**FUNÇÃO** · Você é o dono da fidelidade visual do terminal embutido (cores) e da indicação de foco
(caret) dos campos de input da UI.

**DIRECIONAMENTO** · Fronteira de arquivos (SÓ estes):
- `crates/lina-vt/src/lib.rs`       (mapeamento ANSI/SGR → cor: `xterm256` ~874, `resolve_color` ~917, defaults ~630)
- `app/lina-gpui/src/ui/input.rs`   (caret dos campos de input da UI: `CARET` ~19, `display_text` ~119)
- (NÃO toque `main.rs`; `render_line`/`rgba` (~332-436), `keystroke_to_bytes` e `handle_key` (~127, ~2848)
  moram lá → registre pedido de costura com diff textual preciso.)

Tarefas (priorize C1 e C4 — são os de causa-raiz clara nos SEUS arquivos):
1. **C1 — cores ANSI fiéis (lina-vt/lib.rs).** Verifique a paleta das 16 cores ANSI base (`xterm256`
   ~874-892) e os defaults fg/bg (~630). O usuário quer "igual ao Claude real". Alinhe a paleta base a
   uma referência reconhecida (ex.: paleta padrão de terminal escuro consistente) e garanta que
   bold/inverse/SGR resolvem a cor certa. Prove com testes de `resolve_color`/`xterm256` para índices-chave.
2. **C4 — caret de foco (ui/input.rs).** Hoje o caret é um caractere anexado quando focado, sem pisca e
   possivelmente apagado. O usuário não enxerga indicação de que o teclado está ativo. Garanta um caret
   VISÍVEL e de alto contraste quando o campo tem foco (a indicação de foco é o essencial; blink é
   secundário). Se um blink real exigir timer no render loop (main.rs), NÃO faça aqui — entregue o caret
   estável visível e registre na entrega o pedido de costura de animação (respeitando reduce-motion).
3. **C2/C3 — diagnóstico (ctrl+o, `/` + setas).** Estes dependem de keystroke→PTY em main.rs (do Maestro).
   Investigue se o byte enviado ao PTY está correto: `ctrl+o` deve ir como `0x0F`; setas como CSI no modo
   certo (app-cursor vs normal — o picker de skills do CLI usa setas e depende do modo de cursor correto).
   NÃO altere main.rs: entregue o diagnóstico + o diff de costura preciso (qual byte/modo está errado e o
   conserto) para o Maestro aplicar.

**OBJETIVO** · Cores do terminal embutido visualmente fiéis ao Claude real; campos de input mostram caret
de foco nítido; diagnóstico claro (com diff de costura) para `/`+setas e `ctrl+o`.

**RESULTADO ESPERADO** · `tasks/despachos/bugs-f2/.entrega-bf-terminal.md`: arquivo:linha, evidência
(`cargo test -p lina-vt -- --test-threads=1` e `cd app/lina-gpui && cargo test` p/ input — exit DIRETO,
nº testes), pedidos de costura em main.rs (diff preciso p/ C2/C3 e blink), achados. Última linha `PRONTO`/`BLOCKED`.
