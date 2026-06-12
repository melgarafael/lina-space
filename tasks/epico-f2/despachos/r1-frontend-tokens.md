# Despacho r1 — Terminal C (FRONTEND): F2-1-1/2/3 — vocabulário de tokens (typography/spacing/radius/motion)

## CONTEXTO
Execução F2, onda F2-1 (épico: vault `38` §F2-1; você tem prontidão declarada). Decisão selada F2-0-D: fusão T1+temperatura-T3 — fontes IBM Plex Sans (UI) + Fraunces (momentos) + JetBrains Mono (grid); superfícies quentes. A fundação é o SEU alvo: `app/lina-gpui/src/theme.rs` (695 linhas, gpui-free, gates WCAG em CI + lint anti-cor-literal). Arquitetura decidida (D2 rec a, auditada): vocabulário em código toolkit-free no módulo existente; valores no JSON aditivo. Restrição dura verificada: SEM variable fonts no pin — **instâncias estáticas completas obrigatórias**.

## FUNÇÃO
Frontend — dono único de `theme.rs` nesta rodada.

## DIRECIONAMENTO
1. **F2-1-1 `TypographyTokens`:** famílias (ui/display/mono) + escala de tamanhos + pesos nomeados. JetBrains Mono no grid substituindo o Menlo hardcoded de `main.rs:90-91` — ATENÇÃO: `main.rs` é COSTURA (dono = Maestro); você prepara o token e o diff mínimo de consumo, e me PEDE a aplicação na costura via ask (não edite main.rs direto). Fontes embarcadas: confirme que Plex/Fraunces/JetBrains têm estáticos OFL completos e registre os caminhos/pesos escolhidos em comentário de módulo (o empacotamento físico no .app é story posterior — tokens primeiro). Mesmas garantias da cor: testes de integridade no módulo.
2. **F2-1-2 `SpacingTokens` (escala 4/8/12/16/24/32) + `RadiusTokens`:** definição + helpers de consumo. NÃO varra o shell substituindo magic numbers ainda (isso é a onda 3 com catraca) — esta story só cria o vocabulário.
3. **F2-1-3 `MotionTokens`:** durações nomeadas + flag reduce-motion; doutrina embutida em doc de módulo: *nunca animar ação iniciada por input frequente* (D1-A8).
4. Valores das famílias novas entram no export/import JSON existente com `serde(default)` (import antigo nunca quebra — teste disso).
5. WCAG/gates: estenda o teste de integridade para as famílias novas no que for verificável (ex.: escala monotônica, durações ≤ teto, contraste não regride).
6. Fronteira: `app/lina-gpui/src/theme.rs` (+ teste novo no mesmo módulo). `main.rs`/`wiring.rs` = costura via Maestro. Não commite.
7. Valide localmente: `cargo test -p lina-gpui` + `cargo clippy -p lina-gpui -- -D warnings` + `cargo fmt` SÓ nos seus arquivos (lição: fmt em árvore compartilhada toca hunk de peer).

## OBJETIVO
O vocabulário completo existe com as garantias da cor — a onda de catálogo (F2-2) nasce consumindo tokens, nunca números mágicos.

## RESULTADO ESPERADO
`theme.rs` com as 3 famílias + testes verdes + pedido de costura (se houver) via ask. Reporte começo/fim com `--intent status`. Última linha: `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
