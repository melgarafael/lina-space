# DESPACHO r3-f1-5-8 — Arquiteto
**id:** `f1-5-8` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md` (fronteiras v2!)

## F1-5-8 · API de consulta paginada do histórico — FATIA CORE (fonte: `ondas-5-6.md` linhas 120-128)
Sobre o `ScrollbackStore` (que já tem range/tail + `expired_before` da F1-5-9): `search(panel, regex, limit, cursor)`, `tail(panel, n, offset)`, `export(panel, json|txt, janela)` — **toda chamada com limite duro** (default + máximo; NENHUM caminho não-paginado exposto a agente); saída estável sem ANSI (linearização da W0-2 já existe). **Policy de acesso:** leitura cross-terminal passa pela MESMA fronteira de pertencimento (`WorkspaceTrust`/ADR 0006) — membros do mesmo Espaço leem, com **evento auditável** no log por leitura cross (variante aditiva em events.rs — você é interno autorizado; registre). O VERBO `lina history` no bin = costura à janela do time externo (bin/lina.rs é deles agora) — entregue a API core + os testes + o CONTRATO do verbo documentado (flags/saída) em `tasks/epico-f1/contrato-lina-history.md` para a fiação ser 10 linhas.
**Critérios headless:** (a) ler as últimas N linhas de painel com histórico > cap — paginado, byte-idêntico, nunca excede o limite; (b) pedir 10^9 linhas → página máxima + cursor (adversarial); (c) search encontra linha JÁ no disco; (d) export json round-trip íntegro; (e) leitura cross-terminal emite o evento auditável; (f) janela expirada → "expirado", não erro (consome `expired_before`).

## Fronteira (sua)
`crates/lina-core/src/scrollback.rs` (hunks da API — o Dados liberou) · NOVO `crates/lina-core/src/history.rs` se preferir · `events.rs` hunk aditivo · testes · `tasks/epico-f1/contrato-lina-history.md`.
**⛔ NÃO toque:** bin/lina.rs + lib.rs bootstrap (EXTERNO — costura) · attention/broker (externo) · webhooks (estável) · app/.
Entrega: `tasks/epico-f1/.entrega-f1-5-8.md` · Marcador: `.iniciado-f1-5-8`.
