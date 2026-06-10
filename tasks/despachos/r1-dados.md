# DESPACHO r1-f1-4-1 — Especialista em Dados
**id:** `f1-4-1` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

## Story: F1-4-1 · Multi-workspace de verdade (Espaço = projeto, tudo event-sourced) — tamanho L, base da onda F1-4
**Fonte integral:** `tasks/epico-f1/ondas-2-4.md` linhas 163-181 (LEIA INTEIRA — os critérios 1-6 são o contrato). Consolidado: vault `34 - Epico Fase 1` §III F1-4.

## Fronteira de arquivos (sua, dono único nesta rodada)
- `crates/lina-core/src/workspace.rs` (NOVO — módulo do multi-Espaço)
- `crates/lina-core/src/events.rs` (você é o DONO ÚNICO de events.rs nesta rodada — eventos ADITIVOS)
- `crates/lina-core/src/lib.rs` (tocar MINIMAMENTE: mod + re-exports)
- `crates/lina-core/tests/gate_f1_4_1.rs` (NOVO)
- **NÃO toque:** `mailbox.rs`, `router.rs`, `guard.rs`, `app/` (a fiação do app/UI — switcher M8, modal M9 — é rodada FUTURA; esta fatia é o CORE headless), `crates/lina-bootstrap/`, `crates/lina-webhooks/`.

## Decisões do Maestro (valem como contrato desta fatia)
1. **Free = 1 workspace** (o gate da onda testa free=1; ignore o "até 2 Espaços" do ux-flows — o Designer está reconciliando).
2. **Layout de storage:** escreva o mini-ADR DENTRO da entrega comparando (a) log único particionado por `workspace_id` vs (b) um store por Espaço, e RECOMENDE com base nas lições reais do repo (busy_timeout/retry WAL; lição `unificar-log-ativa-concorrencia-multi-escritor`). Minha inclinação: **(b) um store por Espaço** (isolamento físico = isolamento de falha; o registry global `~/.lina/` é ponteiro, NUNCA autoridade) — mas se a evidência do código apontar (a), argumente.
3. **Registry global mínimo** em `~/.lina/workspaces.json` (ou caminho que o código já use para estado de máquina): lista de Espaços conhecidos (id, nome, path, last_focus). Ponteiro re-derivável, não autoridade.
4. **Interação com ADR 0022 (LEIA `docs/adr/0022-*.md`):** `default_cwd` do Espaço → cwd PADRÃO de novo terminal/agente; fallback quando ausente = dir gerenciado virgem `n-<uuid>` (comportamento atual). A resolução de cwd deve ficar num SEAM ÚNICO (função pura testável no core que o app consome depois). Eventos: `WorkspaceDefaultCwdSet` aditivo (ou campo aditivo no WorkspaceCreated v3 com serde(default) — decida e registre).
5. ⚠️ **Pré-aviso de segurança/identidade:** hoje a identidade de terminal colapsa quando N nós compartilham cwd (bug em fix pelo Bug Finder nesta rodada — env `LINA_NODE_ID` no spawn). Seu seam de cwd NÃO deve assumir cwd-único-por-nó. Anote no mini-ADR.

## Critérios de aceite (da peça — TODOS observáveis por teste headless nesta fatia)
1. Criar Espaços A e B com times distintos → projeção escopada lista SÓ os nós do Espaço ativo.
2. Broadcast em A não chega a B: o log de B não contém o evento (isolamento provado por LOG).
3. `kill -9` (simulado: drop sem flush/reabertura) com A aberto → replay restaura A íntegro; B íntegro; zero vazamento de estado entre Espaços.
4. Replay determinístico por Espaço: apagar projeções → re-derivar → estado byte-a-byte.
5. Abertura concorrente dos stores (2 conexões/2 Espaços) sem "database is locked" (busy_timeout+retry; teste de regressão da lição W5).
6. Seam de cwd: com `default_cwd` definido → novo nó nasce nele (TerminalSpawned.cwd reflete); sem → fallback `n-<uuid>`; trocar `default_cwd` afeta só nós futuros; persiste no log e re-deriva no replay.

## Âncoras (não feche estas portas)
Event Store/upcasting (nada de tabela-autoridade paralela — registry é ponteiro) · Workspace Bus/Supervisor (escopo por Espaço pendura no broker; NÃO invente canal ad-hoc) · invariantes #2 local-first, #4 log é a verdade, #5 pertencimento=conexão POR ESPAÇO.

## Entrega
`tasks/epico-f1/.entrega-f1-4-1.md` + mini-ADR de storage embutido. Marcador: `.iniciado-f1-4-1`.
