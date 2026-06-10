# Achados de DOGFOODING — sessão 2026-06-10 (desenvolvendo o Lina DENTRO do Lina)

> Auto-percepção: bugs/atritos vividos pelo próprio time de IA usando o produto. Cada item vira
> fix ou story. Status atualizado conforme a sessão avança.

| # | Achado | Evidência | Severidade | Status |
|---|---|---|---|---|
| 1 | **Identidade por cwd colide**: 9 nós spawnados com o MESMO cwd (repo) → `<cwd>/.lina/bootstrap.json` reescrito a cada admissão; whoami/handshake/outbox do MAESTRO viraram "Automatizador". F1-4-1 (`default_cwd` por Espaço) tornaria isso o DEFAULT do produto. | event log seq 20133-20184 (cwd idêntico) + `bridge.rs:3141` + `bin/lina.rs:26`; vivido pelo Maestro no turno 0 | ALTA (correção de identidade; autorização não afetada — duas camadas intactas) | **em fix** (Bug Finder, r1: env `LINA_NODE_ID/NAME` no spawn + ADR 0026) |
| 2 | **Bootstrap do papel MAESTRO instrui carregar skill estrangeira** `maestri-orchestrator` — que o guard do próprio Lina bloqueia. Instrução contraditória no turno 0. | SessionStart hook da sessão + erro do guard ao invocar a skill | MEDIA (UX do agente; turno 0 confuso) | **em fix** (Bug Finder, r1: papel MAESTRO → `lina-orchestration`) |
| 3 | **Papéis inconsistentes por superfície**: AUTOMATOR vira "DEVELOPER" no whoami; FRONTEND idem na lista de colegas; "reviewer" minúsculo no agents.json. | saídas de whoami/handshake/list da sessão | BAIXA-MEDIA | **em fix** (Bug Finder, r1: canonização + teste de paridade) |
| 4 | **`lina check` exibe UUID cru na superfície**: "ultima atividade A2A: handoff de 019eb26f-9190-… para @X" — deveria resolver para `@Nome` (zero jargão, inv#6). | saída do `lina check` desta sessão (Maestro) | BAIXA | aberto → rodada 2 |
| 5 | **Positivo (registro honesto):** despacho de 7 handoffs via `lina handoff` → 7/7 entregues e iniciados sem re-despacho (marcadores 14:08). A injeção faseada da F1-0 + retenção not-ready eliminaram a intermitência que o protocolo Maestri sofria (lição §4.4 do handoff ficou OBSOLETA para o Lina). | `.iniciado-*` 7/7 + `lina check` Busy 7/7 | — | evidência p/ o gate F1-0/F1-3 (uso real) |
