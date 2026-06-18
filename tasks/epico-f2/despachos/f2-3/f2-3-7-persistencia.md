# DESPACHO — F2-3-7 · Persistência/restauração de layout + câmera (sessão local) · fatia `f2-3-7`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
Você é o dono do resize-core (F2-3-2) — esta fatia fecha a durabilidade do canvas. Estado:
- **Posição** já restaura por replay (`NodeMoved` aplicado em `events.rs`). **Tamanho** restaura pelo
  SEU evento `TerminalRedimensionado` (cols/rows/w/h em `ProjectedNode`).
- **Câmera (pan/zoom) — ADR 0029: FORA do event log**, é estado de SESSÃO LOCAL. Falta implementar o
  snapshot de câmera (persistir pan/zoom por Espaço, restaurar ao reabrir → "mesmo zoom pós-crash").
- Spawn hoje usa `fit_dims()` fixo (main.rs:113); B (você) sugeriu na entrega do resize usar a grade
  persistida: `let (cols, rows) = pnode.cols.zip(pnode.rows).unwrap_or_else(fit_dims);`.

**FUNÇÃO** · Você é o dono da câmera persistida em sessão local (ADR 0029) e da restauração do tamanho.

**DIRECIONAMENTO** · Fronteira (SÓ estes): `crates/lina-core/` (um pequeno store de sessão local p/ a
câmera — SQLite local OU arquivo JSON sob o dir do Espaço, FORA da stream de eventos; ADR 0029 §3) +
a struct/serde da câmera. NÃO toque `main.rs`/`bridge.rs` (os hooks de salvar-ao-mudar/carregar-ao-abrir
são costura do Maestro — entregue diff PRECISO: ONDE salvar (debounced, ao fim de pan/zoom) e ONDE
carregar (no boot do Espaço, antes do 1º render)). Critérios:
- Câmera **NÃO** entra no event log (é ruído por-usuário; ADR 0029 rejeita explicitamente). Snapshot de
  sessão = `{pan:(f64,f64), zoom:f64}` por Espaço; perder o snapshot perde no máximo o enquadramento,
  nunca um fato de Espaço.
- Aditivo/retrocompatível: Espaço sem snapshot de câmera → câmera home (pan 0, zoom 1).
- Testes não-vacuosos: round-trip (salva pan/zoom → carrega = igual); ausência → default home; o evento
  de câmera NÃO aparece na stream do Espaço (guardião — espelha o teste do ADR 0029).

**OBJETIVO** · O fundador organiza e dá zoom no canvas, fecha o app, reabre — e está EXATAMENTE como
deixou (posições, tamanhos e enquadramento). Nada se perde num crash.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-7.md`: arquivo:linha, evidência
(`cargo test -p lina-core --test-threads=1` exit DIRETO + nº testes), **diff de costura main.rs/bridge.rs**
(salvar câmera debounced ao fim de pan/zoom; carregar no boot; usar grade persistida no spawn), achados.
Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
