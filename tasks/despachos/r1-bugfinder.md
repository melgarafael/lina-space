# DESPACHO r1-dogfood — Bug Finder
**id:** `dogfood-ident` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Três bugs de DOGFOODING descobertos hoje usando o Lina para desenvolver o Lina (o time inteiro foi spawnado com o MESMO cwd = este repo, e a identidade colapsou). São bugs do PRODUTO. Prioridade máxima da rodada.

## Fronteira de arquivos (sua, dono único nesta rodada)
- `crates/lina-bootstrap/src/lib.rs` · `crates/lina-bootstrap/src/bin/lina.rs`
- `app/lina-gpui/src/bridge.rs` (você é o ÚNICO que toca bridge.rs nesta rodada)
- `docs/adr/0026-identidade-terminal-env-spawn.md` (novo)
- Testes novos nos crates acima.
- **NÃO toque:** `crates/lina-core/src/events.rs` (dono: Especialista em Dados), `mailbox.rs`, `router.rs`, `app/lina-gpui/src/main.rs` (dono: Especialista em IA).

## BUG-1 (o grave): identidade por cwd colide — `bootstrap.json` sobrescrito
**Evidência observada:** o event log do workspace mostra 9 nós com `TerminalSpawned.cwd` IDÊNTICO (este repo). O app reescreve `<cwd>/.lina/bootstrap.json` a cada admissão (`bridge.rs:3141-3148`) e `lina` lê `\".lina/bootstrap.json\"` relativo ao cwd (`bin/lina.rs:26`) → último escritor vence; o terminal MAESTRO passou a se ver como "Automatizador" (whoami/handshake/enqueue do outbox errados).
**Por que é arquitetural:** a F1-4-1 introduz `default_cwd` por Espaço — N terminais no mesmo cwd vira o COMPORTAMENTO PADRÃO do produto. Identidade keyed só por cwd não sobrevive à Fase 1.

**Fix (design acordado com o Maestro — rascunhe o ADR 0026 com isto):**
1. No spawn do PTY (bridge.rs, caminho do `admit_node`/wire do terminal), o app passa a injetar no env do PTY: `LINA_NODE_ID=<uuid do nó>` e `LINA_NODE_NAME=<nome>`. O env do spawn é autoridade do app (inforjável por conteúdo de mensagem; herdado pelos filhos do shell).
2. O CLI `lina` resolve a identidade nesta ordem: (a) `LINA_NODE_NAME`/`LINA_NODE_ID` se presentes; (b) fallback: ficha por-cwd (terminal puro/compat). Os DEMAIS campos da ficha (roster/vault/autonomy/plan) continuam vindo da ficha — só `terminal_name` prefere o env.
3. A escrita da ficha continua (compat + terminal puro), mas o whoami/hook/`enqueue_as` (outbox por-nó) usam o nome RESOLVIDO.
4. ⚠️ Segurança: documente no ADR que o env é escrito pelo APP no spawn — um agente pode exportar `LINA_NODE_NAME=X` num subshell e mascarar o PRÓPRIO whoami local, mas o `from` final de A2A é re-carimbado pelo supervisor no drain (dir-dono) e a autenticação de 2 camadas não muda. O ganho aqui é correção de identidade no caminho legítimo, não nova autoridade. Registre também o caso "2 nós legítimos no mesmo cwd" como motivador.

**Testes exigidos (não-vacuosos):**
- Com env setado e ficha divergente → identidade = env (controle: sem env → ficha).
- Outbox: `enqueue` com env setado deposita no subdir do nome do env.
- Compat: ficha ausente + env ausente → comportamento atual de erro/orientação preservado.

## BUG-2: bootstrap do papel MAESTRO manda carregar skill estrangeira
O SessionStart hook do Maestro de hoje instruiu: `Skills a carregar AGORA: maestri-orchestrator, lina-agent-bus` — e o guard do PRÓPRIO Lina bloqueia `maestri-orchestrator` (skill de orquestrador estrangeiro; o bloqueio está correto). Instrução contraditória no turno 0.
**Fix:** localize a tabela papel→skills (grep `maestri-orchestrator` em crates/) e troque a skill do papel MAESTRO por **`lina-orchestration`** (a skill da F1-3-4, que é o método Maestro internalizado) mantendo `lina-agent-bus`. Varra os outros papéis por skills inexistentes/estrangeiras (não invente skills novas; mantenha as que existem em `crates/lina-bootstrap/assets/lina-skills/` ou genéricas conhecidas). Teste guardião: nenhuma skill listada em role_bootstrap aponta para skill que o guard bloqueia (família do teste doctrine_verbs — promessa ⊆ instalado).

## BUG-3: papéis inconsistentes entre whoami/handshake/list
Mesmo nó com papéis diferentes por comando: "Automatizador" = `DEVELOPER` no whoami vs `AUTOMATOR` no list/agents.json; "Especialista em Telas" = `FRONTEND` no handshake vs `DEVELOPER` na descrição de colegas do whoami; "Revisor" = `reviewer` (minúsculo) no agents.json.
**Fix:** ache a(s) tabela(s) de papéis (descrição por papel, normalização). Papel desconhecido não pode silenciosamente virar "DEVELOPER — desenvolvimento geral" com OUTRO rótulo em outra superfície: canonize (normalização de caixa; AUTOMATOR e reviewer ganham descrição própria; desconhecido = mostra o papel cru + descrição genérica, IGUAL nas 3 superfícies). Teste de paridade: para cada papel do roster de teste, whoami(colegas) × handshake × list exibem o MESMO papel canônico.

## Entrega
`tasks/epico-f1/.entrega-dogfood-ident.md` (modelo nas regras comuns). Inclua o draft do ADR 0026 na sua fronteira. Marcador: `.iniciado-dogfood-ident`.
