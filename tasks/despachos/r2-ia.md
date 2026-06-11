# DESPACHO r2-f1-6-3 — Especialista em IA
**id:** `f1-6-3` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md`

## F1-6-3 · Warning de perda sob CSI Ps S/REP no harvest (fonte: `ondas-5-6.md` linhas 176-182; MEDIA-3 do §7-D)
No `advance_capturing` (lina-vt): detectar evicção-antes-da-colheita (`after==ceiling`) → warning estruturado + contador observável por painel (perda passa de silenciosa a SINALIZADA). Avaliar sub-chunk por contagem-de-linha quando SU detectado — implementar SÓ se barato e medido sem regressão. Critérios: (a) SU/REP torrencial num sub-chunk → warning + contador; (b) NÃO-VACUOSO: output normal (LF/wrap, inclusive torrencial) zero falso-positivo; (c) gate `scrollback_cable_w52` verde e custo por-advance não regride (meça e registre).

## Fronteira (sua)
`crates/lina-vt/**` (crate inteiro) · testes.
**⛔ NÃO toque:** lina-core (scrollback.rs é do Dados nesta rodada; events/attention/broker do EXTERNO) · app (main.rs do externo) · bootstrap.
Entrega: `tasks/epico-f1/.entrega-f1-6-3.md` · Marcador: `.iniciado-f1-6-3`.
