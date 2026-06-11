# DESPACHO r2-breaker — Revisor
**id:** `breaker-antirace` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md`

## Pendência de story F1-3-4 (item 8 do conselho): breaker 2-falhas-do-mesmo-item + anti-race `parents:` — EXERCITAR com testes não-vacuosos
O cenário do gate F1-3 não exercitou: (a) o breaker sticky após 2 falhas DO MESMO item do plano (re-despacho → 2ª falha → escala ao humano, NÃO 3ª tentativa automática); (b) o anti-race de `parents:` no plan.md (item só vira disponível quando TODOS os parents fecham; claim concorrente de 2 nós no mesmo item → 1 vence, evento auditável). Localize os mecanismos (router.rs/plan; `git log --grep=f1-3-4` e `d3fa024`; skill lina-orchestration descreve o contrato) e escreva TESTES que os provem — não-vacuosos: quebre o mecanismo (mutação local temporária) e veja o teste falhar; restaure. Se um dos dois NÃO existir no código (era só promessa da skill), NÃO implemente em silêncio: registre BLOCKED com evidência — vira story da rodada 3.

## Fronteira (sua)
`crates/lina-core/src/router.rs` (testes; hunks de produção SÓ se forem fix de 1 linha) · `crates/lina-core/src/plan.rs` (se existir) · testes novos.
**⛔ NÃO toque:** events.rs/attention.rs/broker.rs/gate_onda3.rs (EXTERNO!) · scrollback.rs (Dados) · bin/lina.rs + lib.rs bootstrap (externo).
Entrega: `tasks/epico-f1/.entrega-breaker-antirace.md` · Marcador: `.iniciado-breaker-antirace`.
