# Design — "Instalar para mim" robusto (config-driven, silencioso, 3 SOs)

**Data:** 2026-06-04 · **Status:** implementado · **Escopo:** onboarding T1 (check-up de CLIs).

## Problema

O "Instalar para mim" usava uma tabela hardcoded `npm install -g <pkg>`. Falhava porque:
1. **`npm` invisível** quando o `.app` abre pelo Finder (PATH mínimo do launchd) — resolvido na onda
   anterior pela hidratação do PATH no boot.
2. **Método de instalação errado:** vários assistentes não se instalam por npm (opencode usa
   `curl | bash`; claude/codex têm instalador nativo). E o binário cai FORA do PATH atual
   (`~/.opencode/bin`, `~/.local/bin`) e/ou só no rc do shell → a verificação não achava →
   *"a instalação terminou, mas o programa não apareceu"*.
3. **Pacote errado do copilot** (`@githubnext/github-copilot-cli`, deprecado, binário
   `github-copilot-cli` ≠ `copilot`).

## Decisões (com o usuário)

- **Silencioso, não interativo:** receitas usam flags/env que evitam Y/N (nunca responder prompt no
  escuro). Sem caminho silencioso → fallback "rode no terminal".
- **3 SOs** (macOS verificado em runtime; Linux/Windows das docs + fallback).
- **Config-driven (CLI Profiles TOML):** receitas em config, não hardcoded (inv#3).

## Arquitetura

- **`lina-cli-profiles`** (aditivo, não mexe no `CliProfile`): `InstallRecipe { program, args, env,
  verify_paths }`, `InstallProfile { macos, linux, windows }`, `Installers` (tabela por id de
  descoberta), `recipe_for(id)` resolve pelo SO de compilação (`CURRENT_OS`).
  - *Por que tabela separada e não campo no `CliProfile`:* o `CliProfile` é orquestração A2A
    (id="claude-code"), exige campos que não temos p/ 4 CLIs; a tabela de install é chaveada pelo
    **id de descoberta** (claude/codex/…) e é puramente aditiva (seguro em multi-terminal).
- **Dados:** `profiles/installers/recipes.toml` (subdir p/ não colidir com o glob de `CliProfile`).
  Embutido no binário via `include_str!` (robusto, sem resolver caminho no `.app`); override externo
  opcional `LINA_INSTALLERS=<arquivo>` adiciona/sobrescreve sem recompilar.
- **Execução:** `run_install(recipe, …)` roda no PTY oculto, injetando `recipe.env`.
- **Verificação (o furo):** ao terminar, `refresh_path_after_install(verify_paths)` RE-resolve o PATH
  do login shell (o instalador acabou de escrever seu dir no rc) ∪ `verify_paths` fixos, grava no
  processo → a descoberta acha o binário E o canvas já o spawna sem reabrir.
- **Sem becos (inv#6):** sem receita p/ o SO → falha acionável "instale manualmente".

## Receitas (verificadas nas docs oficiais, 2026-06)

| CLI | macOS/Linux | Windows | verify_paths |
|---|---|---|---|
| claude | `curl -fsSL https://claude.ai/install.sh \| bash` | `irm …/install.ps1 \| iex` | `~/.local/bin` |
| codex | `CODEX_NON_INTERACTIVE=1 sh -c "$(curl …/codex/install.sh)"` | idem ps1 | `~/.local/bin` |
| gemini | `npm i -g @google/gemini-cli` | idem | (npm bin via PATH) |
| opencode | `curl -fsSL https://opencode.ai/install \| bash` | `npm i -g opencode-ai` | `~/.opencode/bin` |
| copilot | `npm i -g @github/copilot` | idem | (npm bin via PATH) |

## Verificação

- `lina-cli-profiles`: 11 testes (parse de receita, env, verify_paths, `recipes.toml` real válido +
  copilot ≠ deprecado).
- `lina-gpui`: 100 testes (resolução de receita + override; `expand_tilde`; `augmented_verify_path`).
- macOS (este host): hidratação do login shell resolve claude/codex/gemini/npm; instaladores testados
  em prefix isolado criam os binários esperados.
- Linux/Windows: não verificáveis aqui → cobertos pelo fallback.
