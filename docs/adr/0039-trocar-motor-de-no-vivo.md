# ADR 0039 — Trocar o MOTOR (CLI) de um Agente vivo = re-erguer

- **Status:** Aceito (instrução direta do fundador, 2026-06-20 — melhoria de dogfooding).
- **Contexto:** o modal de edição (M6-E) exibe o seletor de motores (chips Claude/Codex/…) também em modo EDITAR e os chips são clicáveis (`modal_select_engine` → `select_engine`), MAS a troca era descartada: o `SavePlan` não tinha campo de motor e a UI mostrava o aviso congelado "⚠ trocar o motor reinicia o Agente — **em breve**" (`COPY_EDIT_ENGINE_LOCKED`). O fundador, na tela, selecionava outro motor e nada acontecia ao salvar. (Mesma família do cwd da edição, fechada pelo ADR 0037; o motor ficara para "F1-2-4".) Pré-requisito separado: a descoberta só listava os motores que o PATH do app via Dock cobria — claude/opencode/antigravity ficavam de fora; corrigido na hidratação de PATH (`ensure_known_cli_dirs_in_path` sempre garante `~/.local/bin`/`~/.opencode/bin`), fora do escopo deste ADR (é bug de robustez, não decisão arquitetural).

## Decisão

1. **Trocar o motor de um nó VIVO re-ergue o nó com o novo CLI** — reusa o mesmo mecanismo do ADR 0037 (`restart_node_in_dir`, agora generalizado): encerra o nó vivo e admite um sucessor preservando nome/papel/posição/autonomia, com o novo `AgentEngine`. O cwd: mantém o atual (trocar só o motor) ou usa a nova pasta se ela também mudou. O "cérebro" (CLI) de um processo não troca por fora — como o cwd, exige reiniciar o processo.

2. **A troca só vale com gesto consciente.** O `SavePlan` ganha `engine: Option<AgentEngine>` (`None` = não mudou). `save_plan` só o preenche quando (a) o usuário CLICOU num chip de motor (`engine_touched`) E (b) o `profile_id` selecionado difere do motor ATUAL do nó. `set_engines`, no Editar, PRÉ-SELECIONA o motor atual (achando o chip pelo `profile_id` do nó) — senão o reset para o índice 0 (claude, primeiro da lista) marcaria uma troca falsa para um nó que já é codex/gemini.

3. **Aviso honesto, não congelado.** O "em breve" sai; entra `COPY_EDIT_ENGINE_RESTART_WARN` ("Trocar o motor reinicia este Agente com o novo CLI. O que estiver rodando agora é encerrado."), exibido SÓ quando o motor foi de fato trocado (`engine_changed()`) — não polui quem só abriu o Editar.

## Segurança (o que este ADR NÃO muda)

- Re-erguer é a sequência canônica de admissão (ADR 0022) — o motor é DADO escolhido pelo humano no modal, jamais autoridade. A identidade do terminal segue vindo do ENV de spawn (ADR 0026); o `CliProfileSet` do sucessor registra o novo CLI no log (auditável).
- Encerrar o processo vivo (perde o estado do CLI antigo) exige gesto explícito (clicar outro motor + Salvar) e é avisado — nunca silencioso. A suíte de segurança do router permanece intacta.

## Por quê assim (alternativas descartadas)

- **Trocar o CLI in-place (mesmo processo):** impossível — o CLI É o processo da PTY (ADR 0026 / `admit_node`); trocar o binário exige re-spawn.
- **Aplicar sempre que o chip está selecionado (sem `engine_touched`):** a descoberta chega assíncrona e reseta a seleção para 0 — sem o flag de clique consciente, todo Salvar de um nó codex viraria uma troca-fantasma para claude.
- **Evento `NodeEngineSet` sem re-spawn (só "na próxima abertura"):** trocar o cérebro sem reiniciar deixa o processo vivo com o CLI antigo — a tela mentiria sobre o motor ativo. Para o motor, "aplicar agora" é a única semântica honesta (ao contrário do cwd, que tem o caminho "só na próxima vez").

## Consequências

- O seletor de motor no Editar passa a funcionar; trocar o motor reinicia o Agente com o novo CLI, preservando identidade-nome.
- `restart_node_in_dir` agora é o ponto único de "re-erguer um nó vivo" para AMBOS os eixos (pasta — ADR 0037; motor — este ADR), com `new_cwd: Option` e `engine_override: Option`.
- Porta aberta: se a troca de motor precisar de histórico/auditoria além do `CliProfileSet` do sucessor, o ponto de extensão já é event-sourced.

## Verificação

- **Modal (unit):** `set_engines` pré-seleciona o motor atual (codex, não o 1º da lista) → sem troca falsa; clicar o mesmo motor não conta; clicar outro carrega o `engine` no `SavePlan` com o `profile_id`/`program` certos (`edit_mode_save_plan_detects_engine_change`).
- **bridge:** `restart_node_in_dir(node, new_cwd, engine_override, registry)` re-ergue preservando identidade; `new_cwd: None` mantém a pasta atual, `engine_override: Some` aplica o novo motor.
