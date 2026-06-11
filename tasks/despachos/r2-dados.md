# DESPACHO r2-f1-5-6-9 — Especialista em Dados
**id:** `f1-5-6-9` · Leia ANTES: `tasks/despachos/_regras-comuns.md` + `tasks/despachos/coordenacao-maestri-externa.md` (há um time EXTERNO no repo — fronteiras lá!)

## Duas stories IRMÃS no scrollback (sequência no MESMO arquivo)
**A — F1-5-6 · Durabilidade de flush** (fonte: `tasks/epico-f1/ondas-5-6.md` linhas 100-108): idle-drain 1-2s como job ÚNICO no PtyHost (thread única — não piorar a BAIXA-iii do Mutex global); signal handler SIGTERM/SIGINT/SIGHUP → flush_all; métrica de linhas pendentes. Critérios: SIGTERM com ~500B pendentes → zero perda byte-idêntica; idle ≤2s → disco; gates `scrollback_cable_w52`/`scrollback_cap_caps_ram` verdes; output torrencial NÃO dispara o idle-drain.
**B — F1-5-9 · Retenção 30d** (linhas 130-138): coluna `ts` com migração idempotente; `retention_days` default 30 por workspace; job diário NO MESMO idle-drain; leitura pós-expiração = "expirado", não erro. Critérios: relógio injetável; `idx` monotônico sobrevive ao DELETE; tamanho do .db ESTABILIZA sob workload contínuo; fixture da versão antiga abre sem perda.

## Fronteira (sua)
`crates/lina-core/src/scrollback.rs` · hunks mínimos em `crates/lina-core/src/lib.rs` (PtyHost idle-drain — o lib.rs do CORE está livre; o do BOOTSTRAP é do time externo, NÃO toque) · testes novos.
**⛔ NÃO toque:** `events.rs`/`attention.rs`/`broker.rs` (time EXTERNO trabalhando agora), `bin/lina.rs`, `app/`, `workspace.rs`. Precisa de evento novo? REGISTRE como costura — não edite events.rs.
Entrega: `tasks/epico-f1/.entrega-f1-5-6-9.md` · Marcador: `.iniciado-f1-5-6-9`.
