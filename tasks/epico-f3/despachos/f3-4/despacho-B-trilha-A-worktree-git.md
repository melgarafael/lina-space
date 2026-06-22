# Despacho — Terminal B (Ultra Code) · TRILHA A: Worktree, Sinal & Salvaguardas (eixo git/shell/guard)

> Entregue via `lina handoff "@Terminal B - Effort: Ultra Code"`. Marcador obrigatório no fim: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (onde o trabalho está — PUXE antes de codar)

- **Você trabalha numa git worktree ISOLADA, só sua:** `cd ../lina-f3-4-a` (irmã do repo principal; branch `lina/f3-4-a`). Mudar de branch ou commitar aqui **NÃO afeta** os colegas — é o ponto da feature que você está construindo. **Você COMMITA na sua branch** (cada commit é um teste vivo do `CodeChanged`). Não faça merge nem push.
- **LEIA primeiro (fonte da verdade):** vault `36 - Spec F3 - Coordenacao de Codigo Multi-Agente` (`lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/36 - Spec F3 - Coordenacao de Codigo Multi-Agente.md"`) §1 (worktree por agente) e §3 (DevOps integrador + salvaguardas). Depois o plano `tasks/epico-f3/onda-f3-4.md` e os ADRs propostos `docs/adr/0040-*`, `docs/adr/0042-*`.
- **A FUNDAÇÃO já está na sua base** (branch `develop`, da qual sua worktree saiu): os eventos `CodeChanged{branch, paths, author_node, commit}` e `BranchIntegrated{branch, into, commit}` já existem em `crates/lina-core/src/events.rs` (declaração + `kind()` + no-op em `apply()`). **NÃO toque em `events.rs`** — é dono do Maestro; se faltar campo, peça ao Maestro (A).
- **Mapa do código (já levantado — confirme com Read):**
  - Enum `CwdPolicy` JÁ EXISTE em `app/lina-gpui/src/bridge.rs:3875` (variantes `Managed`/`UserHome`/`UserDir`). **Você adiciona `Worktree { base_repo: PathBuf, branch: String }`.**
  - `admit_node_inner` em `bridge.rs:5221`; o `match &plan_cwd` que cria dir/kit e devolve `cwd_real` está em `bridge.rs:5348` (ramos `Managed`→5349, `UserHome`→5380, `UserDir`→5390). **Seu ramo `Worktree` entra aqui** — é onde o 1º `Command::new("git") … worktree add …` do projeto nasce (write-before-spawn, antes de `cmd.cwd(...)`).
  - O `match CwdPolicy` é EXAUSTIVO em vários sites — todos precisam do braço novo: `shared_cwd_path` (bridge.rs:3896), `effective_managed_policy` (bridge.rs:4498), `admit_node_inner` (bridge.rs:5348), `live_nodes_sharing_cwd` (bridge.rs:5184), e os construtores `NodeAdmission::*` (bridge.rs:3945/3962/3988). O compilador te guia (enum não-`non_exhaustive`).
  - Injeção de ENV no filho: `node_identity_env` em `bridge.rs:2853` (já carimba `LINA_NODE_ID`/`LINA_EFFORT`). Adicione `LINA_BRANCH` quando a política for `Worktree` (espelha o padrão ADR 0026).
  - **NÃO há git de runtime hoje** (nenhum `Command::new("git")` em produção; confirme com grep). **NÃO há git hook hoje** — a infra de hook existente é só CLI-observability (`crates/lina-hooks`, `crates/lina-bootstrap/src/lib.rs:525` monta `settings.json`). O post-commit hook que emite `CodeChanged` é NOVO.
  - Verbos `lina` despachados em `crates/lina-bootstrap/src/bin/lina.rs:43-66` (padrão: bin enfileira envelope na mailbox; supervisor é escritor único — veja `run_plan_intent` lina.rs:1060 como molde). `usage` em lina.rs:69.
  - Guard: `is_gated_hard` em `crates/lina-core/src/guard.rs:225`; `git push --force` JÁ barrado (guard.rs:230, helpers `is_git_push` :359 / `has_force_flag` :365). **`git reset --hard`, `git branch -D`, `git checkout --force` NÃO estão classificados** — confirme por grep (zero ocorrências de `reset`/`--hard`/`-D` em guard.rs).
  - Decisão PURA de path/branch (sem filesystem/git) pode virar fn em `crates/lina-core/src/workspace.rs` (ao lado de `resolve_spawn_cwd` :151, que devolve `ResolvedCwd` :113) — o core decide o NOME da branch/path; o app EXECUTA o git.

## 2. FUNÇÃO

Você é o **dono do eixo git/shell/guard** desta onda: a worktree por agente, o sinal de mudança (hook→evento), as salvaguardas do guard e a skill de integração. Tudo que toca filesystem/git de verdade é seu. O core permanece git-free; você executa o git no shell app.

## 3. DIRECIONAMENTO (as regras do jogo)

- **Mexa SÓ nos seus arquivos (DONO ÚNICO):** `app/lina-gpui/src/bridge.rs`, `crates/lina-bootstrap/src/bin/lina.rs`, `crates/lina-core/src/guard.rs`, `crates/lina-core/src/workspace.rs`, `.claude/skills/lina-integration/SKILL.md` (novo). **NÃO toque** `events.rs` (Maestro), `router.rs`/`attention.rs`/`plan.rs`/`code.rs`/`lib.rs` (Trilha B — Terminal I).
- **Core git-free (inv #7, core/shell split):** nenhum `Command::new("git")` em `crates/lina-core`. A decisão de path/branch em `workspace.rs` é função PURA (nome derivado, ex.: `lina/<nome-do-agente>`); a EXECUÇÃO (`git worktree add`, `git merge`, instalar hook) fica em `app/lina-gpui/src/bridge.rs`.
- **Degradação honesta:** se `git worktree add` falhar (não é repo git, branch existe, disco), NÃO faça o terminal subir num estado mentiroso — siga o precedente dos ramos atuais (`kit_missing=true` + `eprintln!`, bridge.rs:5391) e/ou caia para `Managed`, narrando. Sem `unwrap()` em produção; sem `catch` que engole.
- **Salvaguardas SÃO duras (família gated-hard #19):** `reset --hard`, `branch -D`/`branch --delete --force`, `checkout -f`/`--force`, `push --force` (já existe) → `ActionClass::GatedHard` → `Decision::Ask` em TODOS os níveis (autônomo NUNCA afrouxa — veja guard.rs:208). Escreva helpers análogos a `is_git_push`. **Não classifique demais:** `git reset` (soft/mixed sem `--hard`), `git branch -d` (lowercase, só mergeada) NÃO são gated-hard.
- **Verbo `lina code-changed`:** o bin SÓ enfileira o envelope (intent `code.changed`, `from`/`author_node` carimbado pelo supervisor — NUNCA do payload); ele não escreve o log. Quem emite `CodeChanged` é o supervisor. O hook git post-commit (que você instala na worktree) chama `lina code-changed --paths … --branch … --commit …` passando os fatos do commit (`git diff-tree`), mas o `author_node` vem do binding do nó, não do hook.
- **A `develop` é a verdade apresentada; worktrees são bastidores.** O leigo nunca vê branch/worktree.
- Convenções: edition 2021; `cargo fmt` + `clippy -D warnings` limpos POR PACOTE; formate só os SEUS arquivos (`cargo fmt -p` toca hunks de peer — use `rustfmt <arquivo>`); eventos aditivos; sem `unwrap()` em prod.
- **A skill `lina-integration`** (markdown em `.claude/skills/lina-integration/SKILL.md`): doutrina do DevOps integrador — analisar branches não-integradas → `git merge` limpo na `develop` → conflito trivial resolvido com justificativa NO LOG → não-trivial = gate humano narrado em pt-br → emitir `BranchIntegrated` por merge provado → NUNCA `push --force`/`reset --hard`/apagar não-mergeada → trabalho órfão vira item de atenção. Escreva-a no padrão das skills `lina-*` existentes (frontmatter `name`/`description` + corpo). Ela é consumida pelo agente DevOps (humano-proxy), não pelo core.

## 4. OBJETIVO (o porquê de negócio)

O fundador já perdeu trabalho com dois times no mesmo working tree (incidente 06-10/11) e com branches órfãs no vibe coding. Esta trilha faz cada IA codar na sua própria árvore sem pisar nas outras, e dá ao time um jeito seguro de juntar tudo no fim — sem nunca apagar trabalho em silêncio. É a infraestrutura que torna "N IAs codando em paralelo" real e seguro.

## 5. RESULTADO ESPERADO (formato exato da entrega)

Na sua worktree `../lina-f3-4-a`, commits atômicos por story na branch `lina/f3-4-a`, com:

- **F3-4-1:** `CwdPolicy::Worktree` + `git worktree add` no `admit_node` + `LINA_BRANCH` no env + decisão pura de path em `workspace.rs`. Teste: criar 2 nós "no projeto X" → `git -C <wt1> branch --show-current` ≠ `<wt2>`, e checkout de um não muda o outro.
- **F3-4-2:** verbo `lina code-changed` (enfileira envelope; supervisor carimba author_node) + instalação do git post-commit hook na worktree. Teste: um commit numa worktree → o hook chama o verbo (em teste, mock do envelope) com os paths reais do `diff-tree`.
- **F3-4-6 (guard):** `reset --hard`/`branch -D`/`checkout --force` → `GatedHard`. Teste unitário no `guard.rs` provando `decide(class, Autonomy::Autonomous) == Ask` para cada um, e que as variantes não-destrutivas (`reset` mixed, `branch -d`) NÃO viram GatedHard.
- **F3-4-5 (skill):** `.claude/skills/lina-integration/SKILL.md` completo + a lógica git de merge que a integração usa (verbo/helper que executa `git merge` na develop e enfileira `BranchIntegrated`).
- **Gate verde por pacote** (rode e LEIA o exit code, sem pipe que mascara): `cargo test -p lina-core` (guard/workspace), `cargo build -p lina-bootstrap`, e o gate do app `app/lina-gpui` (clippy `--all-targets -D warnings` + token_ratchet). Reporte os exit codes.

Termine com **`PRONTO: <o que entregou + exit codes observados>`** ou **`BLOCKED: <motivo + o que precisa do Maestro>`**.
