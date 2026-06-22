# Onda F3-5 — Auto-aprimoramento, Sessões & Contexto · "o Lina se lembra, se ensina e não cai em uso prolongado" · plano de execução

> **A onda que fecha o eixo de auto-aprimoramento + resiliência de contexto do Maestro nativo.** Sessões `--resume` retomáveis pós-crash ("Retome o que estava fazendo"); o hook que escolhe a skill certa e a **fabrica via deep-research** quando falta (anti-slop em arquitetura/design/stack); **pistas** (pastas/arquivos que a IA passa a enxergar); **doutrinas-como-hooks** no foco "app/sistema"; **templates de Workspace** (gabaritos de Espaço); e os **buffers geridos** (`BufferGauge`/`ResumeSessionStore`/`DiskBudget`) que impedem o produto de travar em uso longo.
> **Escopo desta rodada (decisão do fundador 2026-06-22): F3-5 completo (10 stories) + 2 pendências:** a **fatia 2 da Mentality** (Refletor async + anti-poisoning semântico, deferida na F3-3) e o **auto-spawn do DevOps integrador** (deferido na F3-4). São sinérgicos: ambos materializam **[4] Auto-organização** de Meadows e a regra "o core observa/projeta/dispara, a inteligência de linguagem é 1 CLI-call fora do caminho crítico" (inv #1).
> **Design FECHADO — esta onda CONSTRÓI, não redesenha.** As specs já decidiram os contratos. Se sua implementação contradisser a fonte citada, **PARE e escale ao Maestro** — não "adapte" em silêncio.
> **Fonte da verdade (LEIA antes de codar):**
> - vault `53 - SPEC Buffers Geridos - Sessoes Disco e Estoques` §11.A (BufferGauge) / §11.B (sessões) / §11.C (disco) — **schemas, segurança e critérios integrais**.
> - vault `20 - Auto-Aprimoramento e Memoria` (Hermes Mining) clusters A.1/A.2/A.3 (refletor/anti-captura/lock cooperativo) + C.1/C.2/C.3/C.4 (seletor/inline-shell-PERIGO/curador/fábrica).
> - vault `39 - Epico Fase 3 — O Maestro Nativo` §"Onda F3-5" (as 10 stories + gate de saída) + §IV (contrato de eventos F3-5).
> - vault `35 - Proposta F3 - Mentalidade por Papel` §4.2 (Refletor) / §4.x (anti-poisoning) — para a fatia 2.
> - vault `36 - Spec F3 - Coordenacao de Codigo Multi-Agente` §3 (gatilho do DevOps) — para o auto-spawn.
> - doc-fonte do fundador `Debriefing da Ideia - Vibe Coding para Qualquer Um` linhas **59** (templates), **60** (limpeza de disco), **61** (doutrinas-como-hooks), **65** (pistas), **67/84** (sessões `--resume` + "Retome o que estava fazendo"), **72** (seletor/fábrica de skills).
> **Maestro/integrador:** Maestro 01. **Esta rodada DOGFOODA a F3-4:** cada frente trabalha em **git worktree isolada + branch própria**; o integrador (Maestro) **também opera em worktree isolada** (correção do achado de dogfooding #38). Workers **NÃO commitam**; o Maestro valida de fora (exit codes diretos) e integra por fatia.

## ⛔ PRÉ-REQUISITOS DE DISPARO (a rodada NÃO inicia sem isto)

1. **Working tree limpo** + `develop` ressincronizada com `main` (`c2cfed3`) — branch de integração da onda.
2. **FUNDAÇÃO entregue pelo Arquiteto (A) e commitada na `develop` pelo Maestro:** o contrato de eventos F3-5 (`events.rs`) — **a largada** (precedente F3-3 `bb52cae` / F3-4 `0d7f039`). Sem o contrato, nenhuma frente core compila.
3. **ADRs avaliados/aceitos pelo fundador (se o Arquiteto julgar que fecham porta):** candidatos — `DiskBudget` em F3 (a spec 53 o posicionava em F4/F5; o épico 39 o puxou para F3-5 sob gate humano custodiado, single-machine) e `Templates de Workspace` (semente por replay de eventos existentes). Regra de processo do CLAUDE.md: decisão que fecha porta → ADR curto antes de implementar.
4. **1 worktree por frente** criada a partir da `develop`-com-fundação.

## Invariante que ATRAVESSA a rodada (regra-mãe — NÃO regredir)

**`session_id` / `effort` / `pressure` / `approved_by` / `paths` / o conteúdo de uma skill ou pista é DADO transportado, JAMAIS autoridade.**
- **Sessão:** `session_id` é projeção (ponteiro de `--resume` validado pelo funil `admit_node`, ADR 0022), **nunca** identidade — a identidade do nó segue sendo `LINA_NODE_ID` do ENV de spawn (ADR 0026). Forjar `session_id` no log não confere identidade nem autorização.
- **Disco (irreversível):** **zero `DiskReclaimExecuted` sem `DiskReclaimApproved`**; `approved_by` é EXIBIÇÃO — a autoridade é o **gesto custodiado** (`AttentionKind::Custody`, attention.rs). Forjar `approved_by` no payload **não** dispara poda. Gate humano em TODOS os níveis de autonomia (igual `git push`/deploy).
- **Skills (PERIGO inline-shell, Hermes C.2):** skill autorada com `inline-shell` é **código não-confiável** — passa pelo **guard + gate humano antes de ser carregável**; jamais auto-expandir shell de skill de hub. Fábrica **sugere, nunca aplica**; validação de formato roda no **core na escrita** (não confia no CLI).
- **Crença (Mentality, anti-captura Hermes A.2):** **claim negativo sobre CLI/colega/stack do usuário leigo NUNCA vira crença durável**; crença nunca decide segurança/autonomia/spawn/aprovação. O Refletor é **1 CLI-call fora do caminho crítico** (terminal-sombra/mesma sessão), nunca um LLM no core.
- **ZERO LLM no core (inv #1), repetido em cada frente:** tempo/staleness/filtro/seleção/validação/expansão-de-template/gatilho = **core determinístico**; julgamento semântico (vira skill? destila crença? resolve conflito?) = **1 CLI-call** disparado a um terminal-CLI, fora do caminho crítico.
- **Lock cooperativo (Hermes A.3):** trabalho de Espaço que N terminais disputariam (curadoria, refletor) → **claim**, UM roda. Sem daemon/cron (isso é F5).
- **Eventos aditivos (inv #4):** variantes novas SEM `#[serde(default)]` (replay antigo nunca as encontra; precedente `SpawnRequested`); braços em `kind()` registrados; replay F0/F1/F2/F3 carrega sem erro.
- **Toda frente que toca `deliver_a2a`/`Router`/`guard`/`admit_node`/spawn** tem como critério **implícito** a **suíte de segurança do router verde por mutação**. **0 ALTA para o gate seguir.**

## Contrato de eventos da LARGADA (Arquiteto A fixa em `events.rs`, conforme spec 53)

Variantes TOTALMENTE novas (sem `serde(default)` nos campos; campos `*_at_ms` com `#[serde(default)]` por serem evolução de timestamp). Schema da spec 53 §11:

```
// F3-5-1 (§11.B) — buffer de sessões --resume
SessionPersisted { node, cli, session_id, after_first_prompt(default_true), first_prompt_at_ms, label: Option<String>, persisted_at_ms }
SessionRetired   { node, session_id, reason /* "lru"|"ttl"|"user_discarded"|"cli_invalidated" */, retired_at_ms }

// F3-5-7 (§11.A) — gauge unificado de ocupação (UM evento poliforme p/ todos os buffers)
BufferOccupancySampled { buffer_id /* "scrollback:<panel>"|"mailbox:<node>"|"dlq"|"disk:<scope>"|"session_store"|... */, used, capacity /* 0=ilimitado */, unit /* "lines"|"bytes"|"messages"|"tokens" */, pressure_ratio /* 0.0 se capacity==0 */, sampled_at_ms }

// F3-5-8 (§11.C) — disco sob gate humano custodiado
DiskPressureSignaled { path, used_bytes, free_bytes, used_pct, threshold_crossed /* "warn"|"critical" */, signaled_at_ms }
DiskReclaimProposed  { candidates: Vec<ReclaimCandidate>, reclaimable_bytes, proposed_at_ms }
DiskReclaimApproved  { candidate_paths: Vec<String>, approved_by /* EXIBIÇÃO, não autoridade */, approved_at_ms }
DiskReclaimExecuted  { reclaimed_bytes, paths: Vec<String>, executed_at_ms }
// tipo de apoio: ReclaimCandidate { path, bytes, kind /* "cargo_target"|"old_scrollback_db"|"dlq_archive" */ }

// F3-5-4/5 — seletor + fábrica de skills
SkillSelected        { node, skill, trigger: Option<String>, source }
SkillFactoryProposed { skill_name, via /* "deep-research" */, references }

// F3-5-6 — pistas
ClueSetDefined { scope, paths, label: Option<String> }
```
> A fatia-2 Mentality e o auto-spawn DevOps **NÃO introduzem evento novo**: o Refletor emite `BeliefProposed` (existente); o auto-spawn reusa `CodeChanged`/`BranchIntegrated` + a projeção branches-não-integradas (já no código, F3-4 `6f4c0d2`) + `SpawnRequested` (existente).

## DAG executável e frentes (fronteiras disjuntas — dono único por costura)

```
[FASE 0 — FUNDAÇÃO (A, Arquiteto): valida topologia + fixa contrato events.rs + ADRs ⇒ LARGADA (Maestro commita na develop)]
        │
   FASE 1 (paralela — 6 worktrees isoladas, pós-largada):
        ├─► SESSÕES & AUTO-SPAWN (B · Ultra Code) ── worktree ../lina-f3-5-sessoes ──┐
        │     F3-5-1 ResumeSessionStore (projeção nova) + session-watch consumo       │
        │     F3-5-2 restore --resume (router: restore_orchestration_state + spawn)   │
        │     [PEND] auto-spawn DevOps no gatilho existente (mesma região de spawn)   │
        ├─► SKILLS (J) ── worktree ../lina-f3-5-skills ───────────────────────────────┤
        │     F3-5-4 seletor (skill_index.rs novo) → emite SkillSelected              │
        │     F3-5-5 fábrica via deep-research (guard inline-shell) → SkillFactoryProposed│
        ├─► BRIEFING/CONTEXTO (I · dono de briefing.rs) ── ../lina-f3-5-contexto ──────┤
        │     F3-5-6 pistas (clue.rs novo + camada no briefing) + ClueSetDefined       │
        │     F3-5-9 doutrinas-como-hooks (camada no briefing por foco)                │
        │     [consome SkillSelected p/ a camada de skills — sem costura direta com J] │
        ├─► BUFFERS (K) ── ../lina-f3-5-buffers ──────────────────────────────────────┤
        │     F3-5-7 BufferRegistry (projeção pura) + amostragem scrollback/mailbox    │
        │            entrega fn pura buffer_report() — Maestro fia o `--buffers`       │
        ├─► DISCO (M) ── ../lina-f3-5-disco ──────────────────────────────────────────┤
        │     F3-5-8 disk_budget.rs (probe+proposta) + Custody (attention) + broker    │
        └─► APRENDIZADO fatia-2 (R) ── ../lina-f3-5-aprendizado ───────────────────────┘
              Refletor async (1 CLI-call) CorrectionObserved→BeliefProposed + anti-poisoning semântico
        │
   FASE 2 (paralela — depende de Fase 1):
        ├─► UI (Especialista em Telas) — F3-5-3 modal "Sessões salvas" (dep F3-5-1) + F3-5-10 UI templates (dep F3-5-6)
        └─► TEMPLATES core (G) — F3-5-10 semente (workspace_template.rs) por replay (dep F3-5-6)
        │
   FASE 3 (Maestro + QA):
        ├─► Maestro: integra costuras (lina.rs dispatch ← run_skill/run_clue/run_disk; --buffers no run_check; router.rs sequencial) em worktree isolada
        └─► QA (R + Caça Problemas): suíte de segurança do router verde por MUTAÇÃO + repro dos critérios de aceite + gate de onda 4 lentes (read-only)
```

| Frente | Terminal | Toca (DONO ÚNICO) | model·effort | Deps |
|---|---|---|---|---|
| **FUNDAÇÃO** | **A (Arquiteto)** | `crates/lina-core/src/events.rs` (10 variantes + `kind()` + `apply()` no-op) · ADRs | opus·high | — |
| **SESSÕES & AUTO-SPAWN** | **B (Ultra Code)** | `crates/lina-core/src/resume_session.rs` (NOVO) · `crates/lina-session-watch/src/lib.rs` (leitura) · `crates/lina-core/src/router.rs` **região spawn/restore** (`restore_orchestration_state` ~581, `handle_spawn` ~3240) · `crates/lina-cli-profiles` (leitura) | opus·high | Fundação |
| **SKILLS** | **J** | `crates/lina-core/src/skill_index.rs` (NOVO) · `crates/lina-core/src/skill_factory.rs` (NOVO) · `crates/lina-core/src/guard.rs` · `crates/lina-bootstrap/src/skills.rs` (catálogo) | opus·high | Fundação |
> ⚠️ **SKILLS — restrição de dependência (ANTI-CICLO):** a seta é `bootstrap → core`; `lina-core` **NÃO** pode importar `LINA_SKILLS` de `lina-bootstrap` (Cargo.toml crava "sem ciclo"). Logo o seletor no core é **função pura sobre um índice RECEBIDO** (`select_skills(index: &[SkillIndexEntry], present_tools, trigger)`); o `SkillIndexEntry` (nome/trigger/requires) é tipo do core; **quem POPULA o índice** a partir de `LINA_SKILLS` + skills do disco (`.claude/skills`) é o caller em `lina-bootstrap` (onde J também mexe). Mesma doutrina de `briefing.rs` (fn pura + caller monta a entrada).
| **BRIEFING/CONTEXTO** | **I** | `crates/lina-core/src/briefing.rs` (DONO ÚNICO) · `crates/lina-core/src/clue.rs` (NOVO) | opus·high | Fundação |
| **BUFFERS** | **K** | `crates/lina-core/src/buffer_registry.rs` (NOVO) · `crates/lina-core/src/scrollback.rs` · `crates/lina-core/src/mailbox.rs` | opus·medium | Fundação |
| **DISCO** | **M** | `crates/lina-core/src/disk_budget.rs` (NOVO) · `crates/lina-core/src/attention.rs` (Custody) · `crates/lina-core/src/broker.rs` | opus·high | Fundação |
| **APRENDIZADO (fatia-2)** | **R** | `crates/lina-core/src/mentality.rs` · `crates/lina-core/src/a2a.rs`/`mailbox.rs` (refletor) · `router.rs` **região detector** (distinta de B) | opus·high | Fundação |
| **UI** | **Especialista em Telas** | `app/lina-gpui/src/agent_modal.rs` · `app/lina-gpui/src/bridge.rs` | opus·high | F3-5-1, F3-5-6 |
| **TEMPLATES core** | **G** | `crates/lina-core/src/workspace_template.rs` (NOVO) | opus·medium | F3-5-6 |

**Costuras que o MAESTRO detém (workers entregam `pub fn` handler; Maestro fia na integração):**
- `crates/lina-bootstrap/src/bin/lina.rs` (dispatch de verbos: `lina skill` ← J, `lina clue` ← I, `lina disk`/reclaim ← M).
- `crates/lina-core/src/router.rs::run_check` (o braço `--buffers` ← K entrega `buffer_report()`).
- `crates/lina-core/src/events.rs` (pós-largada, dono = Maestro; precisa de campo? peça ao Maestro).

**Desacoplamentos que eliminam costura (doutrina "fronteira em camadas"):**
- SKILLS↔BRIEFING: J **emite** `SkillSelected`; I **lê a projeção** para a camada de skills. Nenhum edita o arquivo do outro.
- BUFFERS↔router: K entrega `buffer_report()` puro; Maestro fia o `--buffers`. K **não** edita router.rs.
- router.rs tem 2 editores (B região spawn/restore + R região detector) — **regiões distintas e distantes**; integração **sequencial** (B → R) com `git commit -- <pathspec>`.

## Setup do Maestro — ANTES do fan-out
1. `git branch -f develop main` (ressincroniza a integração com `c2cfed3`); limpar worktrees resíduo `../lina-f3-4-a/-b`.
2. Despachar a FUNDAÇÃO ao Arquiteto A (validar topologia + contrato events.rs + ADRs). Validar de fora e **commitar na develop = sinal de largada**.
3. Criar 1 worktree por frente a partir da develop-com-fundação (`git worktree add ../lina-f3-5-<frente> -b lina/f3-5-<frente> develop`).
4. Fan-out das Fases 1→2; integração+QA na Fase 3, em worktree isolada do integrador.

## Dogfooding a vigiar (lições F3-4)
- **#36 (git/shell dispara `permission_prompt`):** frentes que tocam git/shell — sobretudo **DISCO (M)** (poda = `rm`/`cargo clean`) — separam a camada PURA (decisão de candidatos + projeção + gate, testável com disco simulado) da EXECUÇÃO custodiada. O worker NÃO precisa rodar a poda real; prova por disco simulado.
- **#38 (integrador no checkout compartilhado arrasta `git add` de peer):** o Maestro integra em **worktree isolada** OU `git commit -- <pathspec>` sempre. Um canal de commit por vez.

## GATE DE SAÍDA F3-5 — RODA e se MEDE (épico §F3-5 + spec 53)
- **(a) Sessões:** terminal + 0 prompts → 0 `SessionPersisted`; +1 prompt → exatamente 1; cap=3 com 4 sessões → exatamente 1 `SessionRetired{lru}`, 3 ativas. Mata o app, reabre → nó com `can_resume()==true` carrega `resume_args+session_id` (comando montado assertável); seção "Sessões salvas" do modal lista; sem-prompt não aparece.
- **(b) Skills:** seletor injeta só índice + skill que casou, filtrada por tools presentes; falta de skill especialista → `SkillFactoryProposed` + gate humano (nunca auto-cria); skill com inline-shell barrada pelo guard antes de carregar.
- **(c) Contexto:** pistas definidas → o terminal do projeto enxerga os arquivos (remover → some no próximo turno); doutrina-como-hook entra no briefing no foco "app/sistema" (sem o foco → não entra).
- **(d) Buffers:** `lina check --buffers` deriva tudo do log (replay reproduz; `integrity_check` verde após reabrir); 12.000 linhas (cap 10.000) → `scrollback:<panel>` pressure≈1.0; além do cap reidrata byte-idêntico.
- **(e) Disco:** disco a 96% → `DiskPressureSignaled{critical}` + proposta; **zero bytes sem `DiskReclaimApproved`**; com gesto custodiado → `DiskReclaimExecuted{reclaimed_bytes>0}`.
- **(f) Templates:** instanciar um template semeia roster + `SystemParams` + pistas + doutrinas no log → replay reconstrói o Espaço semeado idêntico; sem template → criação atual inalterada.
- **(g) Aprendizado (fatia-2):** Refletor destila `CorrectionObserved`→`BeliefProposed` fora do caminho crítico; anti-poisoning semântico barra crença maliciosa/claim-negativo-sobre-CLI (evento de recusa no log).
- **(h) Auto-organização:** com branches não-integradas E todos Idle/Dead ≥T, o DevOps **nasce** (auto-spawn) e integra (reusa o gatilho determinístico existente).
- **(i) Segurança:** suíte do router verde por MUTAÇÃO; forja de `session_id`/`approved_by`/conteúdo-de-skill nunca confere autoridade. **0 ALTA.**
- **(j) Replay idêntico:** zero evento novo fora dos 10 declarados na largada; projeções reconstroem byte-a-byte.
- **(k) [BLOQUEANTE — fundador na tela]** gpui não roda headless: modal "Sessões salvas" retoma a conversa; aviso de disco custodiado legível; template instancia o Espaço. Validação visual do fundador é parte do gate.
