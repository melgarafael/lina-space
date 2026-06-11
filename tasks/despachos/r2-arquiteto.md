# DESPACHO r2-f1-6-4 — Arquiteto
**id:** `f1-6-4` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md` (time externo no repo)

## A — F1-6-4 · Teto-IP sob túnel (fonte: `ondas-5-6.md` linhas 184-190)
Anti-starvation p/ o cenário túnel SEM campo forjável decidir identidade/orçamento (X-Forwarded-For só com garantia VERIFICADA de ingresso único). Candidatos: janela deslizante · per-hook ANTES do 404 sem virar oráculo de enumeração (custo de lookup constante) · teto global generoso como backstop. Critérios: flood de corpo-lixo não derruba hook legítimo p/ 429; sem oráculo de hook_id (timing/comportamento); rate-limit per-hook pós-HMAC de 7c04172 intacto; mini red-team em linguagem de invariantes anexado (0 ALTA).
## B — Selar o ADR 0027 (a F1-4-1 fechou em `08bbb92` — leia o mini-ADR de storage na mensagem do commit) + escrever o ADDENDUM do ADR 0010 §3 que o Dados pediu: store-por-Espaço supersede o campo workspace_id por-evento; §1/§2 (namespace no Supervisor, trust por Espaço, deny-all cross) PERMANECEM pendentes de story; autoridade do "Espaço ativo" = log de cada Espaço (registry é ponteiro de boot).

## Fronteira (sua)
`crates/lina-webhooks/**` · `docs/adr/0027-*.md` · `docs/adr/0010-*.md` (addendum no fim, não reescreva).
**⛔ NÃO toque:** events.rs/attention.rs/broker.rs/bin/lina.rs/lib.rs-do-bootstrap/app (externo) · lina-core.
Entrega: `tasks/epico-f1/.entrega-f1-6-4.md` · Marcador: `.iniciado-f1-6-4`.
