# Onda F3-4 — Coordenação de Código Multi-Agente · "N IAs codam em paralelo sem se pisar; o time junta no fim" · plano de execução

> **A onda que resolve o incidente real 2026-06-10/11** (dois times no mesmo working tree; despacho contra HEAD defasado; trabalho perdido em branch órfã). Cada agente passa a ter **sua própria git worktree + branch** no MESMO PC — mudar de branch num terminal NÃO muda nos outros. Um `CodeChanged` por commit avisa o time **por pertencimento**; quem reservou (`claim`) um arquivo que o colega tocou **PARA e pede direção**; e um **DevOps integrador** junta tudo na `develop` sem nunca descartar trabalho em silêncio.
> **Design FECHADO — esta onda CONSTRÓI, não redesenha.** A spec 36 já decidiu os 3 mecanismos (worktree por agente · sinal de mudança · DevOps integrador), todos compostos de tijolos já construídos (lifecycle, spawn, claims, fila de atenção, guard, admissão canônica).
> **Fonte da verdade (LEIA antes de codar):** vault `36 - Spec F3 - Coordenacao de Codigo Multi-Agente` (os 3 mecanismos + salvaguardas inegociáveis) + épico `39 - Epico Fase 3 — O Maestro Nativo` §"Onda F3-4" (as 6 stories + gate de saída) · ADR 0032 (parents/claims) · ADR 0022 (admissão canônica `admit_node`) · ADR 0026 (identidade terminal = pasta + ENV no spawn) · ADR 0037 (cwd por nó editável) · família ADR 0007 (campo escrito por agente nunca decide autoridade) · invariantes #4 (event log = fonte) e #6 (não-técnico-first; nunca perder trabalho em silêncio).
> **Maestro:** Terminal A (ARQUITETO). **Esta rodada DOGFOODA a própria feature:** os 2 codificadores trabalham em git worktrees isoladas (manuais, já que a feature ainda não existe) e **commitam nas suas branches** — exatamente o fluxo que o F3-4 mecaniza. O integrador junta na `develop`.

## ⛔ PRÉ-REQUISITOS DE DISPARO (a rodada NÃO inicia sem isto)

1. **Pendências de rodadas anteriores incluídas** no git: skills do método (`.claude/skills/lina-*`), achados de dogfooding #34/#35 (sessão G), e a pesquisa-estado-f3-0-conf (já referenciada no épico). Working tree limpo antes de branchear.
2. **Branch `develop` criada** a partir de `main` (branch de integração da onda).
3. **FUNDAÇÃO entregue pelo Maestro (A)** e commitada na `develop`: o contrato de eventos (`CodeChanged`/`BranchIntegrated` em `events.rs`) — **a largada** (precedente F3-3 `bb52cae`). Sem o contrato, nenhuma trilha compila.
4. **2 worktrees criadas** a partir da `develop`-com-fundação — uma por codificador.
5. **ADRs propostos aceitos pelo fundador:** 0040 (worktree na admissão), 0041 (paths no claim), 0042 (DevOps integrador + salvaguardas git). São decisões que **fecham portas** (1º git de runtime; extensão do modelo de claim) → registradas antes de implementar (regra de processo do CLAUDE.md).

## Invariante que ATRAVESSA a rodada (regra-mãe — não regredir)

**`paths`/`branch`/`author_node`/`commit` é DADO transportado, JAMAIS autoridade.** `CodeChanged.author_node` é carimbado **server-side** (binding do outbox autenticado, ADR 0007/0026), nunca lido do payload escrito por um agente. A comparação `paths`×claims só pode **abrir um item de atenção** (fail-safe restritivo: PARA e pergunta), nunca conceder posse nem autorizar ação. O DevOps integrador **NUNCA** roda `push --force`/`reset --hard`/apaga branch não-mergeada (família gated-hard #19, gate humano em TODOS os níveis de autonomia). Branch só é "fechada" com `BranchIntegrated` PROVADO no log. **Trabalho órfão nunca é descartado** → vira item de atenção, não lixo. Toda story que toca `deliver_a2a`/`Router`/`guard`/`admit_node` tem como critério implícito a **suíte de segurança do router verde por mutação**. **ZERO LLM no core** (inv #1): o gatilho do DevOps é projeção determinística do log; a única inteligência de linguagem (resolver conflito não-trivial) é do CLI do agente DevOps, fora do core.

## DAG executável e frentes (fronteiras disjuntas — dono único por costura)

```
[FUNDAÇÃO (A, Maestro): events.rs ← CodeChanged + BranchIntegrated (aditivos) + kind() + apply() no-op
                        + ADRs 0040/0041/0042 + develop + 2 worktrees ⇒ LARGADA]
        │
        ├─► TRILHA A (B · Ultra Code) — worktree ../lina-f3-4-a, branch lina/f3-4-a ──────┐
        │     "Worktree, Sinal & Salvaguardas" (eixo git/shell/guard)                     │
        │     F3-4-1 CwdPolicy::Worktree + git worktree add (bridge.rs)                    │
        │     F3-4-2 verbo `lina code-changed` + git post-commit hook na worktree          │
        │     F3-4-6(guard) reset --hard / branch -D / checkout --force → gated-hard       │
        │     F3-4-5(skill) skill lina-integration + lógica git de merge                   │
        │                                                                                  │
        ├─► TRILHA B (I · Dev Core projeções) — worktree ../lina-f3-4-b, branch lina/f3-4-b┤
        │     "Pertencimento, Conflito & Gatilho" (eixo core/coordenação)                  │
        │     F3-4-3 paths no claim + broadcast por pertencimento + paths×claims (router)  │
        │     F3-4-4 AttentionKind::CodeConflict (attention.rs)                            │
        │     F3-4-5(core) projeção branches-não-integradas + gatilho determinístico       │
        │                                                                                  │
        └─► QA+INTEGRADOR (R) — provas eval-first (tests/) + integração na develop ────────┘
              F3-4-6(provas) push--force/reset--hard/branch-D bloqueados por MUTAÇÃO
              segurança do router verde (author_node server-side; paths×claims sem autoridade)
              DOGFOODING: junta lina/f3-4-a + lina/f3-4-b na develop, órfão→atenção, BranchIntegrated no log
        │
        ▼
   Maestro (A): valida de fora (exit codes diretos) por worktree + cold-review isolado
              + gate de onda (4 lentes) + integração ao vivo + atualizar épico 39 no vault
```

| Frente | Terminal | Toca (DONO ÚNICO) | model·effort | Deps |
|---|---|---|---|---|
| **FUNDAÇÃO** | **A (Maestro)** | `crates/lina-core/src/events.rs` (2 variantes + kind() + apply() no-op) · ADRs 0040/0041/0042 | opus·high | — |
| **TRILHA A** | **B (Ultra Code)** | `app/lina-gpui/src/bridge.rs` · `crates/lina-bootstrap/src/bin/lina.rs` · `crates/lina-core/src/guard.rs` · `crates/lina-core/src/workspace.rs` · `.claude/skills/lina-integration/` | opus·high | Fundação |
| **TRILHA B** | **I (Dev Core)** | `crates/lina-core/src/router.rs` · `crates/lina-core/src/attention.rs` · `crates/lina-core/src/plan.rs` · `crates/lina-core/src/code.rs` (NOVO) · `crates/lina-core/src/lib.rs` (1 linha mod) | opus·high | Fundação |
| **QA+INTEG** | **R (QA)** | `crates/**/tests/` (NOVOS) + integração na `develop` | opus·high | Trilha A, Trilha B |

**Reserva (escalada por breaker sticky 2×):** K, M.
**Por que disjunto de verdade:** `app/lina-gpui` está EXCLUÍDO do workspace cargo (build/gates próprios) → Trilha A roda gate do app (token_ratchet, clippy `--all-targets` do app) + `guard.rs`/`workspace.rs` (arquivos isolados de lina-core); Trilha B roda gate de lina-core nos arquivos de coordenação. Os 2 únicos arquivos tocáveis por ambas — `events.rs` (contrato) e `lib.rs` (re-export) — são dono único do Maestro e da Trilha B. **No merge na develop, arquivos diferentes ⇒ sem conflito de texto** (o ponto exato que a feature existe para garantir).

## Setup do Maestro (A) — ANTES do fan-out

1. **Incluir pendências** (skills do método + achados #34/#35 + pesquisa-estado) num commit na `main`; criar `develop` a partir daí.
2. **Fixar o contrato de eventos** (aditivo, `events.rs`), seguindo a doutrina §1 (events.rs:1119-1129): variantes TOTALMENTE novas → campos obrigatórios **sem** `#[serde(default)]` (replay antigo nunca as encontra; precedente `SpawnRequested`):
   - `CodeChanged { branch, paths, author_node, commit }` — sinal de mudança por commit; `author_node` carimbado server-side.
   - `BranchIntegrated { branch, into, commit }` — merge PROVADO no log; única prova de "branch fechada".
   - Braços em `DomainEvent::kind()` (events.rs:1292) e na lista no-op do reducer `apply()` (events.rs:1734) — são META (projeção por replay), não tocam `ProjectedState`.
3. **NÃO** tocar `plan.rs`/`attention.rs`/`router.rs` (são da Trilha B). `paths` no `PlanItem` é decisão de schema (ADR 0041) que a Trilha B implementa em `plan.rs` — o Maestro especifica o contrato, I escreve o arquivo.
4. **Commit do contrato = sinal de largada.** Replay F0/F1/F2/F3 carrega sem erro.
5. **Criar as 2 worktrees** a partir da `develop`-com-fundação:
   - `git worktree add ../lina-f3-4-a -b lina/f3-4-a develop`
   - `git worktree add ../lina-f3-4-b -b lina/f3-4-b develop`

## GATE DE SAÍDA F3-4 — RODA e se MEDE (épico §F3-4 + spec 36)

- **(a) Worktree por agente:** 2 agentes "no projeto X" codam em worktrees/branches isoladas no MESMO PC — `git -C <wt1> branch --show-current` ≠ `git -C <wt2>` ao mesmo tempo, e o checkout de um **não** muda o do outro. (Resolve o incidente 06-10/11 por construção.)
- **(b) Sinal de mudança + pertencimento:** um commit numa worktree apenda **exatamente um** `CodeChanged{paths}` com os caminhos reais; um nó com claim que **intersecta** os `paths` PARA e abre item de atenção; um nó **sem** interseção ignora em silêncio (não muda de estado).
- **(c) DevOps integrador:** com branches não-integradas E todos Idle/Dead ≥T, o gatilho determinístico dispara o DevOps; merge limpo na `develop` apenda `BranchIntegrated` no log; conflito não-trivial vira gate humano narrado ("juntei o trabalho; estes 2 trechos disputam — escolhi A porque…; quer rever?").
- **(d) Salvaguardas:** `push --force`/`reset --hard`/apagar branch não-mergeada bloqueados (gate humano em todos os níveis); trabalho órfão vira item de atenção, **nunca** lixo.
- **(e) Segurança:** suíte do router verde por mutação; `CodeChanged.author_node` carimbado server-side (forja no payload ignorada); `paths`×claims nunca concede autoridade. **0 ALTA.**
- **(f) Replay idêntico:** projeções (`CodeChanged` por branch; branches não-integradas) reconstroem byte-a-byte por replay; zero evento novo fora dos 2 declarados.
- **(g) [BLOQUEANTE — fundador na tela]** o dogfooding executado ao vivo: 2 agentes em worktrees isoladas → commits geram CodeChanged → conflito paths×claims abre atenção legível → integração na develop narrada em pt-br, zero git visível ao leigo. gpui não roda headless.

## Conselho de gate de onda (4 lentes, read-only — agentes que NÃO construíram a onda)

(1) **Visão/fios:** coordenação de código sem cabos (pertencimento), integração que junta sem perder, zero git visível ao leigo, humano árbitro do não-trivial. (2) **Arquitetura:** worktree no shell app (core git-free; decisão de path pura em workspace.rs); event log é a fonte do sinal (nunca regex); projeções por replay (molde CostLedger/Mentality); porta cross-machine (ADR 0034) intocada — tudo single-machine. (3) **Segurança (red-team, 0 ALTA):** author_node/paths nunca autoridade; salvaguardas gated-hard em todos os níveis; órfão nunca descartado; multi-CLI por skill renderizada (inv #3). (4) **Specs:** spot-check spec 36 §1-§3 vs implementado; nada do "Por que é F3 (e não agora)" violado (composição de tijolos existentes, não reescrita).

## Pendências / deferidas

- **Cross-machine (ADR 0034) — NÃO implementar:** tudo single-machine. A porta `BusTransport` fica nominal; qualquer travessia cross-machine é F4.
- **DevOps spawn 100% autônomo:** a MECÂNICA (gatilho determinístico + skill lina-integration + BranchIntegrated) é implementada e testada por unidade; a PROVA ao vivo do gate (c) é a integração executada pelo integrador seguindo a skill (junta A+B na develop). O disparo automático por supervisor sem humano amadurece sob os mesmos gates.
- **Ciclo de vida das worktrees:** poda só após `BranchIntegrated`; **nunca** apaga branch não-mergeada (salvaguarda dura). A `develop` é a verdade apresentada ao humano; worktrees são bastidores.

---

## STATUS — F3-4 PREPARADA (2026-06-22 · Maestro: Terminal A)

Plano + 3 despachos especialistas escritos. Aguarda OK do fundador para: incluir pendências → criar develop → entregar fundação → criar worktrees → disparar B/I/R.
