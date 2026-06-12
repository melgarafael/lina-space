# Despacho r2 — Terminal D (QA): F2-1-5 teste-catraca de tokens

## CONTEXTO
Onda F2-1 (épico vault `38`): vocabulário de tokens completo e commitado (439c5e7 + 47d52d8 — Typography/Spacing/Radius/Motion em `theme.rs`). A onda 3 da migração vai varrer magic numbers arquivo-a-arquivo; a CATRACA garante que a dívida só CAI no caminho — é o mesmo mecanismo que mantém cor 100% limpa desde a F1-2-1 (lint `no_hardcoded_colors_outside_theme_module`, theme.rs:574-602 — modele nele).

## FUNÇÃO
QA — dono do teste-catraca.

## DIRECIONAMENTO
1. **Teste novo** em arquivo PRÓPRIO (`app/lina-gpui/tests/token_ratchet.rs` — fronteira limpa; C está em theme.rs/persistence_ui.rs): conta por arquivo do shell as ocorrências de (a) `px(` literal fora de theme.rs, (b) `FontWeight::` inline fora de theme.rs, (c) `.text_size(px(` inline. Compara contra um **snapshot versionado** (`app/lina-gpui/tests/token_ratchet_snapshot.txt` ou similar): contagem MAIOR que o snapshot em qualquer arquivo = FALHA com mensagem ensinável ("use o token X / atualize o snapshot se a queda foi real"); contagem menor = teste pede atualização do snapshot (catraca aperta).
2. Snapshot inicial = estado REAL de hoje (gere e confira contra os números da D2-A1 auditada: ~126 px() mágicos + 31 FontWeight — divergência razoável é esperada pós-r1, documente a contagem que encontrar).
3. Exclusões honestas e documentadas no teste: theme.rs (a fonte dos tokens), testes, e o que for falso-positivo estrutural (ex.: `px(0.)` de reset?) — cada exclusão com 1 linha de porquê.
4. Roda no `cargo test` normal (o job de CI existente pega de graça — NÃO toque em ci.yml).
5. Valide: `cargo test` no app (suíte inteira) + clippy -D warnings + fmt só nos seus arquivos. Não commite.

## OBJETIVO
A dívida de magic numbers nunca mais sobe — a onda 3 da migração trabalha com rede de proteção, e qualquer story futura que regredir quebra a CI na hora.

## RESULTADO ESPERADO
Teste + snapshot + contagem inicial reportada. Reporte começo/fim com `--intent status`. Última linha `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
