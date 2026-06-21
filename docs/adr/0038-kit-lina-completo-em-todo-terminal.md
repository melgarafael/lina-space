# ADR 0038 — Kit Lina completo e uniforme em todo terminal que o app abre (provisionamento independe da política de cwd)

- **Status:** Aceito (instrução direta do fundador, 2026-06-19 — achado de dogfooding: um terminal apontado a uma **pasta nova/zerada** "não sabe que está no Lina"; perde orquestração, gates e inteligência).
- **Contexto:** o provisionamento por-nó é hoje **assimétrico pela política de cwd**. No funil de spawn (`bridge.rs`):
  - `CwdPolicy::Managed` → `write_one` → `write_terminal_files_with_hooks` → **kit COMPLETO**: 3 doutrinas (`CLAUDE.md`/`AGENTS.md`/`GEMINI.md`) + `settings.json` (hooks SessionStart + guard) + `.lina/bootstrap.json` + **as 12 skills** (`skills::install_skills_into`).
  - `CwdPolicy::UserDir { consent: true }` → `write_user_dir` → 3 doutrinas (merge-safe) + `settings.json` + `bootstrap.json` + **só `lina-agent-bus`** (`install_agent_bus_skill`) — **faltam 11 skills** (orquestração, dispatch, cold-review, spawn, retro, verification + 4 doutrinas + translator).
  - `CwdPolicy::UserDir { consent: false }` → **NADA escrito** (badge "sem doutrina/observabilidade").
  - `CwdPolicy::UserHome` → terminal puro, **ZERO arquivos por design** (pedido do fundador — não poluir o Home).
  - Existe ainda o install GLOBAL por-CLI (`ensure_lina_globally_available`, `main.rs`): doutrina auto-gated + safra completa em `~/.claude|.codex|.gemini` — mas **não instala hooks** (decisão consciente: não invadir o CLI fora do app) e é auto-gated.
  - **Resultado observado:** um terminal de projeto cai no ramo parcial (kit sem as 11 skills) ou no ramo vazio (pasta sem consentimento) → comporta-se como um `claude`/`codex` comum. **Viola o inv #6** (nunca tela em branco; não-técnico-first; 1º agente <2h). O ADR 0037 (cwd editável) **herda o buraco**: `restart_node_in_dir` re-ergue como `UserDir` → kit parcial.

## Decisão

1. **Todo terminal que o APP abre numa pasta de PROJETO recebe o kit COMPLETO e idêntico** — a mesma safra das 12 skills, doutrina 3-CLI, `settings.json` com hooks e `.lina/bootstrap.json` — **independentemente da política de cwd** (`Managed` ou `UserDir`). A política de cwd decide **onde** os arquivos moram e **se** preservamos um arquivo do usuário (merge-safe), **nunca o conteúdo do kit**. Concretamente: `write_user_dir` passa a instalar a **safra completa** (`skills::install_skills_into` no namespace `.claude/skills/`), não só a `lina-agent-bus` — espelhando `write_terminal_files_with_hooks`.

2. **Auto-provisionamento sem gate de consentimento para terminais que o app abre** (decisão de produto do fundador, 2026-06-19: "auto-instalar sempre, sem perguntar"). A pasta de projeto é provisionada na hora; a proteção do usuário continua sendo a **escrita MERGE-SAFE** (`write_guarded` + marcador de máquina + chave `_lina_managed` no settings): um `CLAUDE.md`/`settings.json` pré-existente **não-Lina** é **preservado + badge** — jamais sobrescrito. O namespace Lina (`.lina/`, `.claude/skills/`) é aditivo e sempre escrito. O ramo `UserDir { consent: false }`-sem-kit deixa de existir para nós que o app spawna: o eixo "consentimento" **deixa de ser pré-requisito do kit** (a proteção real sempre foi o merge-safe, não o gate — pasta zerada não tem o que proteger; pasta com arquivos é protegida pela escrita guardada).

3. **Exceção preservada: `UserHome` ("Terminal puro") continua um shell limpo**, ZERO arquivos do Lina (decisão de produto reafirmada 2026-06-19). Só o que é **projeto** recebe o kit; o shell-de-rascunho, não.

4. **Multi-CLI por construção (inv #3):** a doutrina já sai nos 3 arquivos (`CLAUDE.md`/`AGENTS.md`/`GEMINI.md`) e as skills no namespace de skills. Trocar de LLM = trocar `CLI Profile`, não muda o kit.

5. **Escopo: DENTRO do app.** O uso do CLI **fora** do app segue governado pelo install global auto-gated (sem enforcement, `global_install.rs`) — este ADR **não o altera** (a fronteira de não-overreach sobre o uso pessoal do CLI é mantida).

## Segurança (o que este ADR NÃO muda)

- **Identidade vem do ENV de spawn (ADR 0026), nunca do cwd nem de arquivo escrito por agente.** O kit é **dado entregue, jamais autoridade**. N nós no mesmo cwd seguem distintos por construção.
- **Merge-safe intacto:** `write_guarded` + `LINA_MANAGED_MARKER` (posse por `starts_with`) + `settings_is_ours` (posse por chave estruturada, parse+valor — nunca substring). Nenhum arquivo do usuário sobrescrito; recusa → `KitOutcome::Partial` → badge visível (degradação honesta, inv #6).
- **Enforcement (guard PreToolUse) só no `settings.json` por-nó dentro do app** — fora do app segue sem enforcement (não-overreach preservado).
- **Router/`deliver_a2a` não são tocados** → a suíte de segurança do router permanece intacta.
- **Sem evento novo:** a mudança é lógica de provisionamento no app; nenhum `DomainEvent` é adicionado → replay de log antigo é trivialmente intacto.

## Por quê assim (alternativas descartadas)

- **Manter a assimetria e só "documentar":** não resolve a dor — o usuário leigo não escolhe (nem entende) política de cwd. Rejeitado.
- **Um 4º caminho de provisionamento:** aumenta a superfície de buracos. A decisão **COLAPSA** os caminhos no provisionador completo **já existente** (`install_skills_into`/`write_terminal_files_with_hooks`) — a menor mudança que resolve (doutrina de simplicidade).
- **Manter o gate de consentimento como pré-requisito do kit:** atrita com "auto-configurar sozinho" e é redundante — a proteção real é o merge-safe, não o gate.
- **Instalar enforcement global (fora do app):** overreach sobre o uso pessoal do CLI — rejeitado (mantém a fronteira de `global_install.rs`).
- **Tornar o "Terminal puro" Lina-aware:** rejeitado pelo fundador — o shell limpo é intencional; não poluir o Home.

## Consequências

- Pasta nova/zerada aberta por um terminal do Lina nasce **100% funcional** (orquestração, gates, inteligência) — fim do "não sabe que está no Lina".
- O ADR 0037 (cwd editável) passa a entregar o **kit completo** na pasta nova **de graça** (mesmo provisionador).
- `write_user_dir` fica mais pesado no 1º spawn (instala 12 skills) — **idempotente** (arquivo idêntico não reescreve; sem churn de mtime), custo amortizado.
- O badge "sem kit" deixa de aparecer para nós de projeto (só em recusa merge-safe **real** de arquivo do usuário).
- **Porta aberta:** kit seletivo por papel (ex.: FRONTEND sem skills de orquestração) — o ponto de extensão é a **seleção da safra** em `install_skills_into`, nunca a política de cwd.

## Verificação (observável — algo que roda e se mede)

- **Unit (`bridge.rs`):** `write_user_dir` numa pasta **limpa** instala as **12 skills** (`.claude/skills/<nome>/SKILL.md` para cada) + as 3 doutrinas + `settings.json` com hooks + `bootstrap.json`; **2ª chamada byte-idêntica** (idempotente).
- **Unit (`bridge.rs`):** `write_user_dir` com `CLAUDE.md`/`settings.json` **pré-existentes não-Lina** → preservados, `KitOutcome::Partial` (badge), demais skills instaladas mesmo assim.
- **Spawn:** um nó `UserDir` nasce com o kit completo no cwd; o ramo sem-kit deixa de existir para app-spawn (`kit_missing` só em recusa real).
- **ADR 0037:** `restart_node_in_dir` → kit completo na pasta nova.
- **3-CLI:** as 12 skills chegam ao namespace; doutrina nos 3 arquivos.
- **Regressão:** suíte de segurança do router verde; `cargo test`/`clippy -D warnings` verdes em `app/lina-gpui` e `lina-bootstrap`; replay de log antigo intacto.
