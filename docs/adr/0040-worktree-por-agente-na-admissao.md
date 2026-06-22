# ADR 0040 — `CwdPolicy::Worktree` na admissão: worktree + branch por agente (1º git de runtime)

- **Status:** 🟡 Proposto (2026-06-22, Maestro Terminal A) — gate da story F3-4-1
- **Onda/Story:** F3-4-1 (épico `39`, spec `36 - Coordenação de Código Multi-Agente` §1)
- **Data:** 2026-06-22
- **Fontes:** spec `36` §1 · doc-fonte linha 82 · ADR 0022 (admissão canônica `admit_node`) · ADR 0026 (identidade terminal = pasta + ENV no spawn) · ADR 0037 (cwd por nó editável) · invariantes #5 (pertencimento = conexão), #6 (não-técnico-first), #7 (core/shell split) · código: `app/lina-gpui/src/bridge.rs:3875` (`CwdPolicy`), `bridge.rs:5221` (`admit_node_inner`), `bridge.rs:5348` (match de cwd), `bridge.rs:2853` (`node_identity_env`), `crates/lina-core/src/workspace.rs:151` (`resolve_spawn_cwd`)

> **Gate:** F3-4-1 não inicia até este ADR ser aceito.

## Contexto

Incidente real 2026-06-10/11: dois times de IA co-existiram no **mesmo working tree** do repo. Mudar de branch num terminal mudava para todos (working tree única do git), e um despacho saiu contra `HEAD` defasado — trabalho se perdeu. A identidade de um agente Lina **já é a pasta dele** (ADR 0026 + cwd por-nó ADR 0037), mas hoje `admit_node` só sabe três políticas de pasta: `Managed` (dir gerenciado virgem), `UserHome`, `UserDir` (`bridge.rs:3875`). Nenhuma cria isolamento de git.

git tem a primitiva nativa para isso — **worktrees**: N árvores da mesma repo, cada uma em pasta própria com branch independente. O encaixe é direto: `pasta do agente = worktree da branch dele`. Falta a política na admissão e a execução do git (que **não existe em runtime hoje** — confirmado: zero `Command::new("git")` em produção; o app só *classifica* git no guard).

## Decisão

### 1. Nova variante `CwdPolicy::Worktree { base_repo, branch }`

Em `app/lina-gpui/src/bridge.rs:3875`, acrescentar ao enum:

```rust
Worktree { base_repo: PathBuf, branch: String },
```

Criar um agente "no projeto X" passa a significar: o app cria uma git worktree de `base_repo` numa pasta própria do nó, com uma branch nova nomeada pelo app (ex.: `lina/<nome-do-agente>`). O `match CwdPolicy` é exaustivo em ~5 sites (`shared_cwd_path` :3896, `effective_managed_policy` :4498, `admit_node_inner` :5348, `live_nodes_sharing_cwd` :5184, construtores `NodeAdmission::*`) — o compilador guia (enum não-`non_exhaustive`).

### 2. O 1º `git` de runtime vive no SHELL (core permanece git-free)

A execução (`git worktree add <pasta> -b <branch> <base_ref>`) entra no ramo novo do `match &plan_cwd` em `admit_node_inner` (`bridge.rs:5348`), write-before-spawn, antes de `cmd.cwd(...)`. **A decisão** de path/branch (nome derivado, validação) é função PURA em `crates/lina-core/src/workspace.rs` (ao lado de `resolve_spawn_cwd`) — o core decide, o app executa. Isso preserva o invariante #7 (core/shell split: o core não importa toolkit nem dispara subprocesso de SO).

### 3. Branch exposta ao filho via ENV (ADR 0026)

`node_identity_env` (`bridge.rs:2853`) ganha `LINA_BRANCH=<branch>` quando a política é `Worktree` (espelha `LINA_NODE_ID`/`LINA_EFFORT`). O agente sabe em qual branch está sem ler git.

### 4. Degradação honesta, nunca estado mentiroso

Se `git worktree add` falhar (não é repo git, branch já existe, disco), o app **não** sobe o terminal fingindo isolamento: segue o precedente dos ramos atuais (`kit_missing=true` + `eprintln!`, `bridge.rs:5391`) e/ou cai para `Managed`, narrando ao leigo. Sem `unwrap()` em produção.

### 5. A `develop` é a verdade apresentada; worktrees são bastidores

O leigo **nunca** vê branch/worktree (invariante #6). A UX humana mostra a linha de integração (`develop`); as worktrees por-agente são mecanismo interno. Ciclo de vida: poda da worktree só **após** `BranchIntegrated` (ADR 0042); **nunca** apaga branch não-mergeada.

## Consequências

- **Positivas:** resolve o incidente 06-10/11 **por construção** — checkout de um agente não afeta o outro. Habilita N IAs codando em paralelo no mesmo PC sem colisão. Reaproveita o funil único `admit_node` (ADR 0022) e a identidade-por-pasta (ADR 0026).
- **Porta que se ABRE (registrada):** introduz a **primeira invocação de `git` em runtime** no projeto. Fica confinada ao shell app, num helper único, com a decisão pura no core. Toda capacidade git futura (merge, hook, integração) ancora aqui — daí o ADR.
- **Custo / superfície:** uma variante no enum + ~5 braços de match + 1 helper git no app + 1 fn pura no core + 1 var de ENV. Local-first intacto (single-machine; cross-machine é F4/ADR 0034, não tocado).
- **A provar (red-team):** (a) 2 nós "no projeto X" → `git -C <wt1> branch --show-current` ≠ `<wt2>`, checkout de um não muda o outro; (b) falha de `git worktree add` degrada honesto, sem terminal mentiroso; (c) `base_repo`/`branch` no payload de um agente não concede nada (a política é montada pelo app/humano, não lida do envelope — família ADR 0007).

## Alternativas rejeitadas

- **`git` no core (`lina-core`):** violaria o split core/shell (inv #7); o core decide, não executa SO. Rejeitado.
- **Branches sem worktree (uma árvore, troca de branch):** é exatamente o que causou o incidente — troca de branch é global na árvore única. Rejeitado de saída.
- **Clones separados por agente:** desperdiça disco e desconecta o histórico; worktree é a primitiva git desenhada para isto. Rejeitado.
- **Cross-machine agora (cada agente numa VM/host):** F4 sob ADR 0034; viola local-first single-machine desta fase. Fora de escopo.
