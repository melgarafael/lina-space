# ADR 0026 — Identidade de terminal pelo ENV de spawn (não pelo cwd)

- **Status:** Proposto (rodada r1-dogfood, 2026-06-10 — design acordado com o Maestro; rascunho do Bug Finder)
- **Contexto:** dogfooding de 2026-06-10 — o time inteiro do Lina foi spawnado com o MESMO cwd (este repo) e a identidade colapsou. O event log do workspace mostra 9 nós com `TerminalSpawned.cwd` IDÊNTICO. O app reescreve `<cwd>/.lina/bootstrap.json` a cada admissão (`bridge.rs`, kit write-before-spawn + `rewrite_bootstrap`) e o CLI `lina` lê `".lina/bootstrap.json"` **relativo ao cwd** (`bin/lina.rs`, `INPUT_PATH`) → **último escritor vence**: o terminal MAESTRO passou a se ver como "Automatizador" (whoami, handshake e o subdir do outbox por-nó — o canal A2A autenticado por origem — todos errados). Não é um acidente de dogfooding: a **F1-4-1 introduz `default_cwd` por Espaço**, tornando N terminais no mesmo cwd o comportamento PADRÃO do produto. Identidade keyed só por cwd não sobrevive à Fase 1.

## Decisão

1. **O app injeta a identidade no ENV do PTY no spawn.** No funil único de admissão (`admit_node`, ADR 0022), o comando do PTY nasce com:
   - `LINA_NODE_NAME=<nome do nó>` (ex.: `Bug Finder`) — o nome de display/roster, o mesmo que o supervisor registra;
   - `LINA_NODE_ID=<key do nó>` (`n-<uuidv7>`) — a **key durável de filesystem** do nó, cunhada ANTES do spawn. (O `NodeId` canônico nasce no `Supervisor::register`, DEPOIS de o PTY subir — por isso a key, que já é única e não-reciclável por construção, é o identificador que cabe no env.)
   - (`VIBE_ROLE` segue como antes; os três saem do MESMO ponto, `node_identity_env`.)

   O env de spawn é **autoridade do app**: por-processo (não por-diretório), herdado por todos os filhos do shell, e inforjável por conteúdo de mensagem. N nós legítimos no mesmo cwd deixam de colidir.

2. **O CLI `lina` resolve a identidade nesta ordem:** (a) `LINA_NODE_NAME` (se presente e não-vazio); (b) fallback: a ficha por-cwd (`.lina/bootstrap.json` — terminal puro/standalone/compat). **Só `terminal_name` prefere o env** — roster, vault, autonomia e plano continuam vindo da ficha (que segue sendo escrita, para compat e para o terminal puro).

3. **Todos os pontos de identidade usam o nome RESOLVIDO** (`load_identity()` em `bin/lina.rs`): `whoami`/hook SessionStart, `handshake`, `ask`/`handoff`/`plan claim|check` (o `from` e o **subdir do outbox por-nó**, `enqueue_as`), `spawn` e os custodiados `do`/`resume`. **Exceção doutrinária (red-team da própria rodada):** nos verbos CUSTODIADOS (`do`/`resume`, fila de broker), ficha ausente **não** cai no env — degrada para `agente-desconhecido`, como sempre. O env é exportável pelo processo do agente; num verbo de autoridade ele não pode ser a única fonte do subdir autenticado. (Com ficha presente, o env entra via `load_identity()` e a autoridade final segue sendo o dir-dono no drain — inalterado.)

4. **Compat preservado:** ficha ausente + env ausente → o erro/orientação atual permanece (nunca identidade inventada). Env vazio/whitespace = ausente (env vazio não apaga a ficha).

## Segurança (o que este ADR NÃO muda)

- **O env é escrito pelo APP no spawn** — nenhum campo escrito por agente ganhou autoridade. Um agente pode exportar `LINA_NODE_NAME=X` num subshell e **mascarar o PRÓPRIO whoami local** (cosmético, escopo dele mesmo); mas o `from` final de A2A é **re-carimbado pelo supervisor no drain (dir-dono do outbox)** e a autenticação em duas camadas não muda. Mascarar o nome também NÃO dá acesso ao outbox de outro nó além do que o agente já tinha (qualquer processo local já podia escrever em qualquer subdir; a autoridade nunca foi o nome declarado, e sim o drain do supervisor). O ganho aqui é **correção de identidade no caminho legítimo**, não autoridade nova.
- Doutrina intacta: contrato é dado transportado, jamais autoridade; gate humano para irreversíveis (ADR 0004); suíte de segurança do router inalterada.

## Por quê assim (alternativas descartadas)

- **Ficha por-nó no cwd (`.lina/bootstrap-<key>.json`)**: o CLI não teria como saber QUAL ficha é a sua sem... um env apontando para ela — ou seja, o env é inevitável; injetar só o nome/id é a forma mínima.
- **NodeId canônico no env**: nasceria DEPOIS do spawn (`register`) — exigiria reordenar o wire (register antes do PTY) e re-arquitetar a compensação do ADR 0022 §5. A key `n-<uuidv7>` já é a identidade durável de filesystem (dir-casa, PTY key) e existe pré-spawn.
- **`--resume`/re-injeção do nome via prompt**: conteúdo de PTY é forjável e frágil; env é estrutural.

## Consequências

- O dogfooding (e a F1-4-1) com N terminais no mesmo cwd passa a ter whoami/handshake/outbox corretos por terminal.
- A ficha compartilhada ainda carrega o `terminal_name` do último escritor — **superfícies que só leem a ficha** (sem env, ex.: um shell aberto à mão no mesmo dir) continuam vendo o último nome; aceito (compat), o caminho legítimo (PTY do app) está correto.
- **Risco conhecido (fora do escopo, registrado):** o kit por-cwd compartilhado também colide em `.claude/settings.json` (token de observabilidade keyed por nome — último escritor vence a atribuição dos hooks HTTP). Mesma família; pede story própria.
- Testes que rodam o binário `lina` DENTRO de um terminal do Lina herdam `LINA_NODE_NAME` — harnesses de teste que provam identidade-por-ficha precisam de `env_remove("LINA_NODE_NAME")` (feito em `handoff_cli.rs`/`broker_cli.rs`).
- Revisitar se: (a) o A2A passar a transportar o `LINA_NODE_ID` no envelope (hoje é só identidade local/auditoria); (b) surgir spawn remoto (ADR 0009/0010) — env de processo não atravessa máquinas.

## Verificação

- `identity_env_cli.rs` (binário real): env vence ficha divergente no `whoami` (controle: sem env → ficha) · `ask` deposita no subdir do outbox do NOME DO ENV (controle: sem env → subdir da ficha) · ficha ausente + env ausente → erro/orientação preservados · **custodiado** (`resume`) sem ficha IGNORA o env (degrada p/ `agente-desconhecido`) · whoami/handshake exibem papéis canônicos do `agents.json` (paridade BUG-3).
- Unit: `resolved_name` (puro — env vence; vazio/whitespace cai na ficha) · `outbox_per_node_uses_resolved_name` · `admitted_pty_env_carries_node_identity` (bridge: o funil carimba as 3 vars).
