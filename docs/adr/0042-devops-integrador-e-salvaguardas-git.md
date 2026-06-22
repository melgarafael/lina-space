# ADR 0042 — DevOps integrador (gatilho determinístico) + salvaguardas git gated-hard

- **Status:** 🟡 Proposto (2026-06-22, Maestro Terminal A) — gate das stories F3-4-5 e F3-4-6
- **Onda/Story:** F3-4-5 (integrador) + F3-4-6 (salvaguardas), épico `39`, spec `36` §3
- **Data:** 2026-06-22
- **Fontes:** spec `36` §3 (DevOps integrador + salvaguardas inegociáveis) · invariantes #1 (zero LLM no core), #4 (event log fonte), #6 (não perder trabalho em silêncio) · família gated-hard #19 (guard) · ADR 0022/0026 (spawn governado) · ADR 0040 (worktree por agente — git de runtime) · código: `crates/lina-core/src/guard.rs:225` (`is_gated_hard`, `git push --force` já barrado :230), `lifecycle.rs:46-51` (constantes de stall), `router.rs:3066` (`handle_spawn`), `events.rs` (`CodeChanged`/`BranchIntegrated` — fundação F3-4)

> **Gate:** F3-4-5/F3-4-6 não iniciam até este ADR ser aceito.

## Contexto

Com worktrees por agente (ADR 0040) e sinal de mudança (`CodeChanged`), o time acumula branches com trabalho. Falta: (1) **juntar** tudo numa linha de integração (`develop`) sem perder nada, e (2) **proteger** contra os comandos git que destroem trabalho. O guard hoje barra `git push --force` (`guard.rs:230`) mas **não** classifica `git reset --hard`, `git branch -D` (apagar não-mergeada) nem `git checkout --force` — exatamente os comandos que apagam trabalho em silêncio.

## Decisão

### 1. Gatilho determinístico do integrador (projeção, ZERO LLM)

Em `crates/lina-core/src/code.rs` (módulo novo, Trilha B), projeção pura por replay (molde `CostLedger`/`Mentality`):

- `branches_nao_integradas` = `{ branch | ∃ CodeChanged{branch} ∧ ∄ BranchIntegrated{branch} }`.
- **Gatilho** = `branches_nao_integradas ≠ ∅` **E** `todos os nós Idle/Dead por ≥ T` (lê `ProjectedState.nodes`/lifecycle; T calibrado pelas constantes de `lifecycle.rs:46-51`).

Disparado o gatilho, o core **enfileira um `SpawnRequested` do papel DevOps** pelo caminho de spawn governado existente (`handle_spawn`, ADR 0022 — origem com narração). O core **decide e dispara**; não interpreta linguagem (inv #1).

### 2. A inteligência de integração mora no CLI (skill `lina-integration`), nunca no core

O agente DevOps carrega a skill `lina-integration` (markdown, Trilha A): analisar branches → `git merge` limpo na `develop` → conflito **trivial** resolvido com justificativa **NO LOG** → conflito **não-trivial** = **gate humano** narrado em pt-br ("juntei o trabalho; estes 2 trechos disputam — escolhi A porque…; quer rever?"). Multi-CLI por skill renderizada (inv #3). O core não resolve conflito de merge.

### 3. `BranchIntegrated` é a ÚNICA prova de "branch fechada"

Cada merge provado apenda `BranchIntegrated{branch, into:"develop", commit}` (fundação F3-4). A projeção de §1 só remove uma branch do conjunto pendente com esse evento no log — nunca por suposição. Poda da worktree (ADR 0040) só **após** `BranchIntegrated`.

### 4. Salvaguardas git gated-hard (em TODOS os níveis de autonomia)

Em `crates/lina-core/src/guard.rs:225` (`is_gated_hard`), classificar como `GatedHard` (→ `Decision::Ask` sempre, autônomo nunca afrouxa — `guard.rs:208`):

- `git reset --hard` (helper análogo a `is_git_push`/`has_force_flag`),
- `git branch -D` / `git branch --delete --force` (apagar branch potencialmente não-mergeada),
- `git checkout -f` / `git checkout --force` (descarta working tree),
- `git push --force` — **já existe** (`guard.rs:230`), mantido.

**Não** classificar demais: `git reset` (soft/mixed sem `--hard`), `git branch -d` (lowercase, só mergeada) seguem rotineiros.

### 5. Trabalho órfão NUNCA vira lixo (invariante #6)

Trabalho que não entra na integração (branch não-mergeável, conflito não resolvido) **não** é descartado: vira **item de atenção** ("este trabalho não entrou; quer revisar?"), nunca `branch -D`. Perder trabalho em silêncio é o pecado que esta onda existe para matar.

## Consequências

- **Positivas:** o time junta trabalho automaticamente quando ocioso, sem o humano orquestrar git; as salvaguardas tornam impossível apagar trabalho por acidente (gate humano duro); `BranchIntegrated` dá prova auditável de fechamento.
- **Porta que se ABRE (registrada):** o spawn automático de um papel de SISTEMA (DevOps) por gatilho de projeção. Confinado: passa pelo `handle_spawn` governado (gates de cascata/cap/custo intactos); nesta onda a prova ao vivo é a integração executada pelo integrador seguindo a skill (o disparo 100% autônomo amadurece sob os mesmos gates).
- **Custo / superfície:** 1 módulo de projeção + 1 disparo de spawn + ~3 ramos no guard + 1 skill markdown. Local-first intacto (merge single-machine; cross-machine é F4).
- **A provar (red-team):** (a) `CodeChanged` sem `BranchIntegrated` ⇒ branch pendente; com ⇒ some; (b) gatilho só com todos Idle/Dead ≥T; (c) `reset --hard`/`branch -D`/`checkout --force`/`push --force` → `Ask` em `Autonomous` (mutação: desliga ramo → teste falha → religa); (d) órfão vira atenção, nunca `branch -D`; (e) reason/by da integração nunca lidos do payload (server-side).

## Alternativas rejeitadas

- **Integrador resolve conflito não-trivial sozinho:** viola inv #6 (humano é o árbitro do não-trivial) — perderia a escolha do humano em silêncio. Gate humano obrigatório no não-trivial.
- **Gatilho por LLM ("o time parece ter terminado?"):** não-determinismo num disparo de ação (viola inv #1). O gatilho é projeção determinística do log.
- **Afrouxar salvaguardas em autonomia `Autonomous`:** a matriz do guard NUNCA afrouxa `GatedHard` (`guard.rs:208`); ação destrutiva pede humano em todos os níveis. Rejeitado de saída.
- **Apagar branch órfã para "limpar":** é exatamente o histórico de perda de trabalho do fundador. Órfão vira atenção, nunca lixo. Rejeitado.
