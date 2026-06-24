# ADR 0049 — Seleção AUTOMÁTICA de skills na chegada da tarefa (escopo b do R45-EMIT)

- **Status:** **Aceito** (decisão de produto/arquitetura aprovada pelo fundador; Arquiteto, 2026-06-24). Desenha o gancho de ciclo de vida do escopo (b) e resolve o teto MVP `selection_id=''` do escopo (a) (commit `12b5a05`). Implementação: **Terminal J**.
- **Escopo:** o PONTO DE SELEÇÃO automático — quando uma tarefa chega ao terminal, o sistema roda o retrieval+outcome, emite `SkillSelected` enriquecido com o `root_cause_id` REAL do turno, e a fatia certa flui ao contexto. DISJUNTO da Fase 4.
- **Relacionados:** ADR 0045 (retrieval+outcome), ADR 0047/0048 (padrões de detecção-no-pump e snapshot-que-viaja — reusados aqui), ADR 0019 (`root_cause_id`/turno), spec `0045-contrato-eventos-skill-routing` (`selection_id`).

## Contexto

O escopo (a) (commit `12b5a05`) ligou retrieval (`skill_index::rank_with_outcome`, `skill_index.rs:357`) + outcome ao **verbo manual** `lina skill select`: o bin computa (tem o índice), o `router::handle_skill` (`router.rs:2551`) carimba SERVER-SIDE (`by_role` = papel do remetente autenticado; `selection_id` derivado) e emite `SkillSelected`. Ficou um **teto explícito**: o verbo não conhece o `root_cause_id` do turno → `selection_id = ''` → a seleção fica **fora do aprendizado** (sem desambiguação por-turno; o outcome não consegue casar com a seleção).

O fundador aprovou o escopo (b): a Lina selecionar skills **automaticamente, na chegada da tarefa** — não só por verbo. Isso exige um **gancho de ciclo de vida novo** e resolve o teto: rodando NO turno, o `root_cause_id` real está disponível.

## Decisão

A seleção automática **compõe três padrões já provados** (não inventa mecanismo): detecção-no-pump (ELO1/refletor), snapshot-que-viaja (ADR 0048) e carimbo server-side do binding inforjável (escopo a + `derive_root_hops`).

### (a) QUANDO — na CHEGADA da tarefa, por turno (não no boot)

O gatilho dispara quando uma **tarefa de trabalho é entregue** ao terminal — `RouteOutcome::Delivered { targets }` (`router.rs:294`), o mesmo sinal que o app já trata no `match` do MailboxPump (`bridge.rs:1415`). **Boot é cedo demais** (não conhece a tarefa; a fatia certa depende da query). **Cada turno re-seleciona** (a query muda o top-k). Input direto do usuário no PTY origina o mesmo gancho quando vier a ser observável como início de turno — porta aberta, mesmo caminho.

### (b) QUEM/COMO — detecção no pump, execução FORA do hot-path

Espelha o ELO1 (refletor), **separando decisão pura de efeito** (lição registrada: projeção event-sourced no hot-path exige throttle/delta):

1. **No pump (hot-path, O(1), sem I/O):** no braço `RouteOutcome::Delivered` do MailboxPump (irmão de `CorrectionObserved`, `bridge.rs:1447`), captura-se `(query da tarefa, node, root_cause_id)` num `SkillSelectionRequest` e enfileira-se (fila serial, dedup por turno+node — molde `ReflectorQueue`, `a2a.rs:587`). **NÃO** roda o retrieval aqui (não acoplar a latência do rank ao pump — o bug dos ~298s).
2. **Fora do hot-path (drain, com throttle/delta):** roda `rank_with_outcome(índice, tools, query, role, k, scores)` — o **índice vive no bootstrap/bin** (anti-ciclo: o router não constrói índice). Top-k → enfileira um `skill.select` **de SISTEMA** carregando `{query, task_kind, candidates}` (DADO) + o `root_cause_id` capturado.
3. **No router (carimbo, server-side):** `handle_skill` deriva `by_role` e `selection_id` como no escopo (a) — agora COM `root_cause_id` real → emite `SkillSelected` enriquecido, `selection_id` ≠ `''`.

### (c) `root_cause_id` → `selection_id` — snapshot inforjável que viaja

O `root_cause_id` é lido do binding **`delivered_root[node]`** (`router.rs:470`; atualizado na entrega, `router.rs:2057`; a MESMA fonte inforjável de `derive_root_hops` — JAMAIS do payload). É **capturado na detecção (passo 1) e VIAJA com o `SkillSelectionRequest`** — não é re-lido no drain (entre detecção e execução o binding pode mudar com uma nova entrega → casaria a seleção no turno errado; é o race do ADR 0048, mesma defesa). O `skill.select` automático carrega esse `root_cause_id` num **envelope de SISTEMA** (origem inforjável, à la `WEBHOOK_SYSTEM`/`REFLECTOR_TRIGGER`); o router só aceita o `root_cause_id` por essa via de sistema — o verbo manual de um agente **não** o carrega (segue `selection_id=''`, escopo a).

### (d) EFEITO — a fatia flui pelo briefing (já existe)

O `SkillSelected` é consumido pela projeção `SelectedSkills` (`briefing.rs:93`), que o briefing LÊ na camada de skills (`briefing.rs:76`) e injeta como **dica de contexto** ao terminal — **"Declarar ≠ autorizar"** (`briefing.rs:69`). A camada volátil re-renderiza por turno. O gatilho (decisão) e o briefing (efeito) ficam **desacoplados** pela projeção, nunca por chamada direta.

## Segurança (portas que NÃO fecham — doutrina inegociável)

- **Anti-ciclo (índice no bootstrap):** o retrieval roda no bin/bootstrap (tem o índice); o **router só lê log+roster e carimba** (preservado do escopo a). A detecção no pump só captura `query+root_cause_id` e enfileira — não faz o router construir índice.
- **Server-side:** `by_role`, `selection_id` e o `root_cause_id` vêm de projeção/binding inforjável, **nunca do payload de um agente**. `query/task_kind/candidates` são DADO descritivo (já provado por mutação no escopo a: `skill_select_stamps_authority_server_side_ignoring_payload_forgery`).
- **Gatilho de SISTEMA não-forjável:** um agente não consegue enfileirar um `skill.select` com `root_cause_id` forjado para envenenar o aprendizado — o `root_cause_id` só é aceito pela via de sistema (origem inforjável); o caminho de agente não a carrega.
- **Skill = DADO, jamais autoridade:** `SkillSelected` é observabilidade/aprendizado; a skill injetada é DICA de contexto, **nunca** decide identidade/ordem/autorização nem pula gate humano (ADR 0045 §Segurança).
- **Throttle/delta no drain:** o retrieval é por-turno, fora do hot-path; sem full-replay por tick (lição registrada).

## Por quê assim (alternativas descartadas)

- **No boot do terminal:** não conhece a tarefa → seleciona pela fatia do papel (já é o C0/instalação), não pela query. Não é o escopo (b). Rejeitado como ponto único.
- **Router chama o índice direto:** fecharia a porta anti-ciclo (router passaria a depender da construção do índice). Rejeitado.
- **Síncrono no hot-path do pump:** acopla a latência do `rank` (+ leitura do índice) à responsividade do pump — exatamente o bug dos ~298s que a separação detecção/execução existe para evitar. Rejeitado.
- **`root_cause_id` do payload:** forjável → veneno do aprendizado. Rejeitado; vem do binding inforjável.

## Consequências

- **Resolve** o teto `selection_id=''`: a seleção automática casa com o outcome (C2 fecha o loop escolha→uso→resultado).
- **Porta aberta (ADR 0045 §Consequências):** o mesmo gancho de "chegada de tarefa + ranqueamento por outcome" serve para escolher **terminal/modelo/effort** por tarefa — C2 é genérico; este ponto de seleção é o lugar.
- **Custo:** uma fila + drain a mais (amortizado, molde do refletor); o briefing volátil re-renderiza por turno.
- **Porta que fecha se ignorado:** sem o `root_cause_id` real, o histórico de outcome nasce cego (seleções não casam com resultados).

## Verificação (observável)

- **Dispara na chegada:** uma tarefa entregue (`RouteOutcome::Delivered`) produz um `SkillSelected` com `selection_id` ≠ `''` e `root_cause_id` do turno — não há `SkillSelected` no boot sem tarefa.
- **Desambiguação por-turno:** duas tarefas distintas (root_cause_id diferentes) ao mesmo nó geram `selection_id` distintos; o `SkillOutcome` de cada uma casa com a seleção certa (replay reconstrói).
- **Inforjável (mutação):** um agente emitindo `skill.select` com `root_cause_id`/`by_role`/`selection_id` forjados no payload não muda o carimbo (server-side vence — herda `skill_select_stamps_authority_server_side_ignoring_payload_forgery`).
- **Snapshot anti-race:** uma nova entrega ao mesmo nó entre a detecção e o drain NÃO troca o `root_cause_id` da seleção em voo (o snapshot do request vence o binding corrente — espelha o teste do ADR 0048).
- **Fora do hot-path:** o retrieval não roda sob o lock do pump (não congela a UI) — herda o teste de "dispatch off the critical path" do refletor.
- **Anti-ciclo:** o router não importa o índice; reindexar do disco reproduz a seleção (projeção reconstruível, ADR 0045 C1).
