# Despacho r4 — Terminal D (QA·ultracode): F2-0-6 — DEC 2026 (synchronized output) no emulador

## CONTEXTO
Última story de código da F2-0 (épico vault `38` §F2-0). A pesquisa D3-A3 (verificada: thread HN 2026-01-20, dev da Anthropic verbatim) provou que os CLIs de IA que rodamos são o PIOR caso de flicker — e a cura no lado do terminal é o **DEC private mode 2026 (synchronized output)**: o app TUI envolve o redraw em `CSI ? 2026 h` … `CSI ? 2026 l` e o emulador segura a apresentação até o batch fechar (zero frame intermediário rasgado). Pré-requisito da story de resize da F2-3. Nosso emulador vive atrás do trait `VtBackend` (`crates/lina-vt`, alacritty_terminal por baixo).

## FUNÇÃO
QA/dev de core — dono de `crates/lina-vt` nesta rodada. TDD: o teste do comportamento vem antes.

## DIRECIONAMENTO
1. **Investigue ANTES de implementar:** o alacritty_terminal do nosso pin pode JÁ suportar 2026 (versões recentes suportam — confira no código vendorizado/Cargo.lock, não no README). Se suportar: a story vira (a) PROVAR com teste de integração (sequência 2026 h…l com writes no meio → snapshot do grid não muda até o `l`) + (b) garantir que NADA no nosso caminho de leitura fura o hold (o reader do wire_terminal apresenta por damage/poll — o hold precisa valer de ponta a ponta até o shell). Se NÃO suportar: implemente no seam do VtBackend (buffer de apresentação durante o modo; teto de segurança ~150ms anti-DoS — app que esquece o `l` não congela o grid; documente o teto).
2. **Teste guardião do fim-a-fim:** headless — alimentar o backend com `2026h + clear+redraw + 2026l` e provar que snapshots intermediários não vazam; + teste do teto (esquecer o `l` → apresenta após o teto com log).
3. **Fronteira:** `crates/lina-vt` (+ testes). Se o hold exigir mudança no caminho de apresentação do app (`bridge.rs`/reader), NÃO edite — especifique o diff exato no reporte (bridge é terreno do C/costura; coordeno eu).
4. Aditivo e atrás do trait: `libghostty-vt` futuro herda o contrato (porta âncora — não a solde no alacritty).
5. Validação: `cargo test -p lina-vt` + suíte workspace + clippy -D warnings + fmt nos seus. Não commite. "Reporte E continue no mesmo turno."

## OBJETIVO
Claude Code redesenhando dentro do Lina sem flicker — e a F2-0 fecha 7/7 no código (resta só a sessão de tela de latência).

## RESULTADO ESPERADO
Suporte 2026 provado por teste (ou implementado) + teto documentado + (se houver) spec de diff do caminho de apresentação. Reporte começo/fim com `--intent status`. Última linha `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
