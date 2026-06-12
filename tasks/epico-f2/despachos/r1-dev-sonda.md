# Despacho r1 — Terminal E: F2-0-1 estender a sonda [PROF] (p99 + % acima do orçamento + LoDPI)

## CONTEXTO
Execução F2 (épico: vault `38` §F2-0). A sonda [PROF] (F1-5-1, `app/lina-gpui/src/prof.rs`) mede frametime por fase/painel; o gate da F2 exige métricas que ela ainda não emite: p99 e **% do TEMPO acima do orçamento** (16,6ms) — estrutura SuperTuxKart/CapFrameX da régua D0 camada (d), com a 2ª condição "Steady" (<0,1% excess time) avaliável. Gap novo da onda V (§II.8): nitidez de texto em LoDPI não é medida por nada.

## FUNÇÃO
Developer — dono de `prof.rs` nesta rodada.

## DIRECIONAMENTO
1. **p99 + % do tempo acima do orçamento:** acumule frametimes na janela da sonda e emita p50/p95/p99 + `excess_time_pct` (soma do tempo dos frames acima de 16,6ms ÷ tempo total). O orçamento é parâmetro (16,6 default; 8,33 opcional via env) — a meta de plataforma muda, a sonda não.
2. **Persistência como evento (inv#4):** o resumo da janela vira evento aditivo no log (coordene o nome com o contrato §VI do épico: família de evento de métrica; `serde(default)`) — a régua re-deriva por replay. Se o seam de escrita de evento não estiver ao alcance de `prof.rs` sem tocar costura, implemente o cálculo + a serialização e me PEÇA a fiação via ask (events.rs é costura).
3. **LoDPI check:** o que for automatizável barato — no mínimo, a sonda loga o scale factor da janela e um aviso quando rodando em 1x (para a sessão de tela testar em monitor 1080p de verdade); se houver caminho barato para screenshot-de-glifo comparável, descreva-o no relatório (não implemente sem me consultar — pode ser story própria).
4. Cena de estresse reprodutível: documente a linha de comando canônica (`LINA_PROF=1 LINA_LOAD_ACTIVE=10` + pan/zoom 60s) no cabeçalho do módulo.
5. Fronteira: `app/lina-gpui/src/prof.rs` (+ testes). `events.rs`/`wiring.rs` = costura via Maestro. Não commite. Valide: `cargo test -p lina-gpui` + clippy -D warnings + fmt só nos seus arquivos.

## OBJETIVO
O gate de perf da F2 (p95 ≤16,6ms E <1% acima) torna-se MEDÍVEL por qualquer um, reproduzível, e auditável por replay.

## RESULTADO ESPERADO
`prof.rs` estendido + testes verdes + pedido de costura via ask (se precisar). Reporte começo/fim com `--intent status`. Última linha: `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
