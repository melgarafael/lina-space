# ADR 0037 — Pasta de trabalho POR-NÓ editável (persistida; "aplicar agora" = re-erguer)

- **Status:** Aceito (instrução direta do fundador, 2026-06-19 — melhoria de dogfooding).
- **Contexto:** o app já persiste o cwd de cada terminal (`TerminalSpawned.cwd`, ADR 0022 §3) e o restore re-entra fielmente na mesma pasta (`plan_restore` → `restore_terminals`). Mas isso só vale para a pasta escolhida **na criação** (`CwdPolicy::UserDir`). O modal de **edição** (M6-E) exibe a linha "Pasta de trabalho" com o botão "Trocar…" SEM gate `is_edit` (`agent_modal.rs`) — porém o valor é **descartado**: `SavePlan` não tem campo `cwd`, `new_edit` não pré-carrega o cwd real do nó, e o commit do `Plan::Save` só faz `rename_node` + `assign_role`. Resultado observado pelo fundador: trocar a pasta de um terminal (a) não move o terminal e (b) não sobrevive ao fechar/reabrir — "volta para a pasta original". O `WorkspaceDefaultCwdSet` existe mas é o cwd PADRÃO do Espaço inteiro e, por design (F1-2-4, `workspace.rs`), afeta só nós FUTUROS — não cobre "trocar a pasta DESTE terminal".

## Decisão

1. **A pasta de trabalho vira editável por-nó, persistida por um evento aditivo `NodeCwdSet { node, cwd }`.** O reducer atualiza `ProjectedNode.cwd` (último vence — mesma semântica do `TerminalSpawned.cwd`); o restore já consome `info.cwd`, então a persistência re-aplica de graça na próxima abertura. Evento ISOLADO (e não um campo novo em `TerminalSpawned`): o campo exigiria editar as 18 construções literais espalhadas por gates de várias ondas — custo de costura desproporcional para um dado que é metadado de nó, como o `CliProfileSet`.

2. **"Aplicar agora" = re-erguer o nó na nova pasta, reusando o caminho de restore.** O cwd de um processo Unix não muda por fora (injetar `cd` num PTY com um CLI no controle não move o processo). Então "ir para a pasta agora" encerra o PTY atual e re-admite o nó na nova pasta via o MESMO mecanismo do `restore_terminals`: aposenta o antigo (`NodeRemoved`, supersede), admite um sucessor preservando nome/papel/posição/scrollback, e grava `TerminalSpawned{cwd novo}` (que também persiste). Continuidade para o usuário é de **identidade-nome** (mesma do restore), não de PID.

3. **Isto NÃO viola F1-2-4 ("nós vivos não migram").** Não migramos o processo vivo — encerramos e recriamos. O cwd PADRÃO do Espaço (`WorkspaceDefaultCwdSet`) continua afetando só nós futuros; o que esta decisão adiciona é o cwd POR-NÓ editável, um eixo distinto.

4. **A escolha é do usuário, perguntada na edição (decisão de produto do fundador 2026-06-19).** Quando a pasta é trocada no modo edição, a opção "Reiniciar agora na nova pasta" aparece. **Default conservador: só persistir** (vale na próxima abertura) — encerrar um processo em curso (um `claude` trabalhando) exige ação explícita, nunca acontece por inércia.

## Segurança (o que este ADR NÃO muda)

- `NodeCwdSet` é gravado pelo APP a partir de um gesto humano na UI — mesmo padrão de `rename_node`/`assign_role` (gesto → evento). **A cwd é dado transportado, jamais autoridade**: a identidade do terminal continua vindo do ENV de spawn (ADR 0026), não do cwd; N nós no mesmo cwd seguem distintos por construção.
- Re-erguer é a sequência canônica de admissão (ADR 0022) — nenhum campo escrito por agente decide identidade/ordem/autorização. A suíte de segurança do router permanece intacta.
- A nova pasta passa pela mesma validação de escrita do M9 (`workspace_boot::validate_workdir` — canonicaliza + prova escrita real) ANTES de qualquer mutação. Crítico para o "reiniciar agora": validar antes de remover garante que uma pasta inválida nunca derruba o nó vivo (o terminal não some); o erro vira mensagem leiga inline, nunca crash (inv #6).

## Por quê assim (alternativas descartadas)

- **Injetar `cd <pasta>` no PTY vivo:** não muda o cwd do processo (o `claude`/CLI é quem controla o terminal, não um shell ocioso); conteúdo de PTY é forjável e frágil. Rejeitado.
- **Campo `cwd` em `TerminalSpawned`:** 18 construções literais em gates de ondas distintas — costura cara; o evento aditivo isolado separa a preocupação e custa zero churn nessas construções.
- **Respawn-in-place (mesmo `NodeId`, mata e re-sobe o PTY):** o código assume "um spawn por nó, PID preservado, sem respawn" (`bridge.rs`). Reusar o caminho de restore (novo `NodeId`, supersede via `NodeRemoved`) é o mecanismo já provado em produção — não abre um segundo regime de ciclo de vida.

## Consequências

- Editar a pasta de um terminal passa a persistir e o restore respeita — fim do "volta para a pasta original".
- "Reiniciar agora" encerra o processo em curso (perde o estado vivo do CLI) — por isso é opt-in explícito com microcopy; o scrollback é re-hidratado (continuidade visual, mesma garantia do restore).
- Nós que herdam o cwd PADRÃO do Espaço seguem inalterados; esta decisão não toca `WorkspaceDefaultCwdSet`.
- Porta aberta: se no futuro o cwd precisar de histórico/auditoria por-nó, `NodeCwdSet` já é o ponto de extensão (event-sourced).

## Verificação

- **Core (unit):** replay de `NodeCwdSet` atualiza `ProjectedNode.cwd`; replay de log ANTIGO (sem o evento) desserializa intacto (aditivo, ADR 0001 §2).
- **Modal (unit):** `new_edit` pré-carrega o cwd real do nó (campo mostra o caminho, não o default genérico); `save_plan` só inclui `cwd` quando muda; `restart_now` carregado no `SavePlan`.
- **bridge:** `restart_node_in_dir` re-ergue preservando nome/papel/posição com a nova cwd e grava `TerminalSpawned{cwd novo}`; o antigo é aposentado (`NodeRemoved`).
