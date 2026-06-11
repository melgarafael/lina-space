# 🤝 COORDENAÇÃO ENTRE ORQUESTRADORES — sessão Lina (interno) × sessão Maestri (externo)
**2026-06-10 ~21h50 · autorizado pelo Rafael · documento de fronteira vivo**

## Estado que o time EXTERNO precisa saber ANTES de codar (urgente)

1. **⛔ BUG-1 (identidade por cwd) JÁ ESTÁ CONSERTADO E COMMITADO EM MAIN** — commit `162a15a`
   + **ADR 0026** (`docs/adr/0026-identidade-terminal-env-spawn.md`). A solução adotada:
   o app injeta `LINA_NODE_ID`/`LINA_NODE_NAME` no env do PTY **no funil `admit_node`**
   (`bridge.rs` `node_identity_env`); o CLI resolve identidade env-first com fallback à ficha;
   verbos custodiados (`do`/`resume`) NÃO caem no env (red-team da rodada); testes em
   `crates/lina-bootstrap/tests/identity_env_cli.rs`. **"Tratar na admissão" é exatamente onde
   o fix já vive** — antes de qualquer linha: `git pull`, ler o ADR 0026 e o commit. Se a
   abordagem de vocês divergir, escalem ao Rafael ANTES de editar — não sobrescrevam/revertam
   o env-first (doutrina: nunca reverter linha de peer).
2. **`main` avançou ~100 commits hoje** (`ff4f60d` → `04bb411`): F1-4-1 core multi-workspace,
   CI 3-SO como gate (CADA push roda core-windows EXECUTANDO testes!), sonda [PROF], spike
   a11y, webhooks F1-6-1/2, ADRs 0026/0027/0028, skills com description ≤1024 (teste guardião).
   Trabalhar sobre árvore velha = conflito garantido.
3. **A validação aqui é POR PACOTE com exit direto** e o repack do `.app` exige árvore
   compilável: às 21:51 o build release quebrou com `fold_guard_ask` inexistente
   (`attention.rs:361` — WIP de vocês no meio da escrita). Normal em WIP, mas: **commitem/
   estabilizem a fatia o quanto antes** — o repack do Lina.app e qualquer validação de
   workspace ficam REFÉNS de árvore que compila.

## Fronteiras acordadas (a partir de agora)

| Território | Dono | Nota |
|---|---|---|
| `crates/lina-core/src/attention.rs` + `broker.rs` + hunks de `events.rs` p/ ActionGated-na-fila | **EXTERNO** | item 2 deles (fila cega aos asks do guard) — ninguém do time interno está nesses arquivos |
| `app/lina-gpui/src/agent_modal.rs` + hunks de autonomia em `main.rs` | **EXTERNO** | item 3 deles (UX da autonomia) — atenção: `main.rs` levou a sonda [PROF] hoje (`8522032`); puxem antes |
| `bridge.rs` admissão/identidade (`node_identity_env`, `admit_node`) | **INTERNO (fechado)** | BUG-1 resolvido; mudanças aí exigem conversa prévia |
| `crates/lina-webhooks/**`, `crates/lina-core/src/workspace.rs`, `crates/lina-license*` (futuro), `assets/lina-skills/**`, `docs/adr/**`, `tasks/**` | **INTERNO** | rodada 2 interna retoma quando a conta Claude voltar |
| `events.rs` | **costura quente** | hoje teve dono interno (F1-4-1); hunks de vocês = SÓ aditivos, e avisem aqui |

## Regras de convivência (as mesmas que fecharam a Fase 0)
- Workers não commitam? Cada ORQUESTRADOR commita a própria fatia — commits PEQUENOS e
  frequentes (a janela de reversão com 2 times é o dobro do risco).
- Eventos novos SEMPRE aditivos (`serde(default)`); replay de log antigo nunca quebra.
- Suíte de segurança do router verde é critério implícito de qualquer toque em
  `deliver_a2a`/`Router`.
- Atualizem ESTE arquivo ao fechar a fatia de vocês (status + commits), como nós fazemos.

## Status interno (para referência rápida)
- F1-0 ✅ gate · F1-1 ✅ conselho zero-falha (declara com tela AC-0021.7) · F1-2 spike 2-7 ✓,
  faltam 2-5/2-6 · F1-3 ✅ gate + docs fechados (A/B e ativação-cega re-validação pendentes
  de conta) · F1-4: 4-1 core ✓ + copy ✓ + ADR 0027 draft (faltam 4-3..4-7 + fiação app) ·
  F1-5: 5-1 ✓ MEDIDO (tabela de ativação no `prof-baseline.md`; célula drawn≥12 = tela do
  fundador) · F1-6: 6-1/6-2/6-8 ✓ (CI real: 3 cores verdes no 1º run; app+license falharam
  por causas já tratadas — fix `d2c5cac` + rede transitória do runner).
- Workers internos PAUSADOS pelo limite mensal da conta Claude (~15h). Roteiro de tela do
  fundador: `tasks/epico-f1/roteiro-tela-consolidado.md`.
