# Direcionamento — Otimização da camada de coordenação A2A do Lina Space

> **Para a IA da próxima sessão.** Este doc nasceu de uma **observação ativa de ~14h** de uma tarefa
> real com 8 agentes Claude (terminais A–H) no Lina Space. Ele te diz **o que investigar, corrigir,
> testar e re-validar na prática** — em ordem de prioridade, com evidência de código (`file:line`) e do
> event log. **Não aplique fix cego:** cada item tem uma hipótese a CONFIRMAR no código antes de mexer.

---

## 0. Contexto em 60 segundos

- **O que é o Lina:** app desktop Rust/gpui (macOS) — canvas de N terminais de IA que cooperam "sem
  fios" via um barramento A2A (`lina ask`/`broadcast`/`plan`). Leia `CLAUDE.md` (raiz) antes de tocar
  em código: invariantes inegociáveis (#1 zero-LLM no core, #2 local-first, #3 neutralidade multi-CLI,
  #4 event log = fonte da verdade, #6 não-técnico-first, #7 core/shell split).
- **Como o A2A funciona (caminho real):** `lina ask` (bin `crates/lina-bootstrap/src/bin/lina.rs`)
  deposita a msg no outbox por-nó → o `MailboxPump` (`app/lina-gpui/src/bridge.rs`) drena, roteia pelo
  `Router` (`crates/lina-core/src/router.rs`) e ENTREGA injetando no PTY do alvo via
  `deliver_a2a` (`crates/lina-core/src/a2a.rs`): `wait_ready(prompt_regex)` → cola o texto
  (bracketed-paste) → `sleep(submit_delay)` → Enter (`0x0D`) separado.
- **Já corrigido nesta sessão (NÃO refazer):** veja `git log`. Em especial:
  `f353838` retry de bloqueio transiente (fechou a race de registro do roteamento — A2A entrega de
  forma confiável agora); `65b6f39` `lina ask` confirma o roteamento real (fim do "ok" cego);
  `cd049f6` `lina vault` implementado. **Os achados abaixo são o que SOBRA.**

## 1. Protocolo de trabalho (LEIA — evita retrabalho e colisão)

1. **Skills primeiro.** Use `superpowers:systematic-debugging` (investigar antes de fixar) e
   `superpowers:test-driven-development` (teste que prova o fix ANTES do fix). Para mudança de design
   (ex.: agregação de respostas), `superpowers:brainstorming` antes de implementar.
2. **Multi-terminal = árvore git compartilhada.** Outros terminais podem editar/trocar de branch a
   QUALQUER momento. Antes de editar: `git branch --show-current` (deve ser a sua branch de trabalho;
   se "sumir" arquivo ou `git log` mostrar commits alheios, é troca de branch de peer — `git reflog`,
   não entre em pânico, `git checkout` de volta). **Commite CEDO e com frequência** (trabalho
   não-commitado é volátil). Toque só nos arquivos do seu escopo.
3. **Gates com evidência (não cache-mask):** rode `cargo check`, `cargo test`, `cargo clippy
   --all-targets -- -D warnings` e cheque o **exit code direto** (`echo $?`), não confie em stdout
   glitchado. Force relint com `touch <arquivo>` antes do clippy. Redirecione saída longa pra arquivo
   e leia.
4. **2 workspaces de build:** o app (`app/lina-gpui`) é um crate **standalone** (target/ próprio,
   excluído do workspace raiz). O core (`crates/*`) builda no workspace raiz. Rode os gates no crate
   certo.
5. **Nunca declare "funciona" sem evidência observada.** Para mudança de A2A, a prova final é o
   **teste prático** (seção 4), não só os gates.
6. **`.app` de teste:** após buildar, remonte com `bash packaging/make-app.sh --skip-build` (reusa os
   binários release já buildados). O bundle é `dist/Lina.app`.

---

## 2. Os achados, em ordem de prioridade

### 🔴 P0 — Perfil de ENTREGA errado (causa-raiz UNIFICADA de 2 bugs reais do fundador)

**Sintomas observados:** (a) "`lina ask` às vezes vira texto colado no chat do Claude e não envia"
(intermitente); (b) "respostas se atropelam" (quando 2+ colegas terminam juntos).

**Hipótese (CONFIRME no código):** o app entrega A2A com `demo_profile()` (perfil da Onda 0, feito pra
rodar `sh`), em vez do perfil real `profiles/claude-code.toml`.
- `demo_profile()` em `app/lina-gpui/src/bridge.rs:327` → `prompt_ready_regex = '.'`,
  `submit_delay_ms = 150`.
- `profiles/claude-code.toml` → `prompt_ready_regex = '(?m)^\s*[>❯]\s'` (casa o prompt REAL do Claude
  Code), `submit_delay_ms = 300`.
- O pump usa `profile: demo_profile()` em `bridge.rs:395`; também `main.rs:2355` e `main.rs:2385`.
  **O `claude-code.toml` nunca é carregado em produção** (só num teste do inspector).
- Mecânica em `crates/lina-core/src/a2a.rs:222-260`: `wait_ready(grid, regex, READY_TIMEOUT)` →
  `build_paste` → `enqueue_write(AgentText)` → `sleep(submit_delay)` → `enqueue_write(Submit)`.

**Por que causa os 2 sintomas:**
- `prompt_ready_regex = '.'` casa QUALQUER caractere → `wait_ready` passa SEMPRE, mesmo o Claude
  estando no meio de uma resposta (não espera o prompt `❯` real). → injeta em cima do trabalho (atropelo).
- `submit_delay = 150ms` é curto → o Enter chega antes do bracketed-paste assentar no Claude Code →
  às vezes o texto fica colado sem submeter (intermitente, timing-dependente).

**Fix proposto (investigue a melhor forma):**
1. Carregar o `claude-code.toml` em produção. O loader já existe:
   `lina_cli_profiles::ProfileRegistry::load_dir` (`crates/lina-cli-profiles/src/lib.rs:301`) e
   `CliProfile::load_file`. Decida onde carregar (provável: no boot, em `main.rs`, e passar ao
   `MailboxPump` em vez do `demo_profile()`). **Mantenha um fallback** ao demo se o arquivo faltar
   (não derrube o boot). Cuide do empacotamento: o `claude-code.toml` precisa ir pro `.app` (ver
   `make-app.sh` — hoje copia `assets/` e os binários; pode ser preciso embutir via `include_str!`
   como o `recipes.toml`/`dev-tools.toml` já fazem, p/ não depender de caminho no bundle).
2. Validar o `prompt_ready_regex` contra o **grid real** do Claude Code (o prompt pode ser `❯ ` ou
   `> `; confirme capturando o scrollback de um terminal Claude vivo). Se o regex não casar, o
   `wait_ready` vai sempre dar timeout (2s) e o problema vira outro — TESTE de verdade.
3. Avaliar subir `submit_delay_ms` (300 → 400/500?) e `READY_TIMEOUT` (`a2a.rs:39`, hoje 2s — curto pra
   respostas longas; mas cuidado: ele é o teto de espera do prompt, não da resposta). Meça empiricamente.

**Como testar:** (a) teste headless do `deliver_a2a` com um grid mock que NÃO está em prompt → deve
ESPERAR/timeout, não injetar imediatamente (hoje injeta por causa do `.`). (b) Teste prático (seção 4):
mandar `lina ask` repetidas vezes pra um Claude ocupado e confirmar que o texto SUBMETE (não fica
colado) e que NÃO injeta no meio de uma resposta.

---

### 🟠 P1 — Atropelamento de respostas (sem checagem de Idle / sem agregação)

**Observado:** A faz fan-out pra B–H; quando 2+ terminam juntos, retornam a A com < 1s de intervalo
(evidência: seq 1853 G→A, +0.6s seq 1860 C→A) e atropelam o input de A. Confirmado intermitente: com
tarefas longas os retornos vêm espaçados (200-600s) e NÃO colidem — **o atropelamento é função da
sincronia das tarefas** (fan-out de trabalho curto = colisão).

**Hipótese (CONFIRME):** `deliver_fn` (`bridge.rs:406-419`) injeta DIRETO no PTY do alvo sem checar o
`status` do alvo (Busy/Idle). Não há fila de espera nem agregação: N respostas a A = N injeções.

**Opções de fix (BRAINSTORM antes — é design, não bug óbvio):**
- (a) **Adiar entrega quando o alvo está Busy:** se `sup.node_by_name(target).status == Busy`,
  NÃO injetar; reter no `.inflight` e re-tentar no próximo tick (mecanismo de retry já existe no
  `router.pump` — pode estender a condição de "reter"). Quando o alvo volta a Idle, entrega. Já existe
  `wait_ready`, mas ele olha o GRID, não o status do roster — avalie qual sinal é mais confiável.
- (b) **Agregar respostas:** se há N msgs na fila pro mesmo alvo, injetar como UM bloco. Mais complexo;
  talvez over-engineering pro MVP.
- (c) **Serializar no lock:** o `deliver_a2a` já pega `lock_pty(target)` — confirme se o lock está
  realmente serializando ou se o problema é injetar enquanto o Claude (não o PTY) está ocupado.
- **Cuidado:** não reintroduza deadlock nem perca mensagens. Teste o caminho "alvo Busy → vira Idle →
  entrega" headless.

---

### 🟡 P2 — `lina handoff` e `lina check` não existem (a doutrina os promete)

**Observado:** `CLAUDE.md:142-163` (doutrina gerada) promete `lina handoff "@X" "tarefa" --context` e
`lina check "@X"`; o bin `lina` (`crates/lina-bootstrap/src/bin/lina.rs:27-37`) só tem
`whoami/ask/handoff(NÃO)/broadcast/handshake/plan/guard/resume/do/list/vault`. Os agentes contornam
usando `lina ask --intent handoff` (visto ao vivo: seq 16242), mas o comando documentado falha.
- **`lina handoff`:** é açúcar fino sobre `run_ask` (como `run_broadcast` já é) — fixa `intent=handoff`,
  trata `--context <arquivo>` anexando o conteúdo ao payload. Fácil e alto valor (a doutrina já o ensina).
- **`lina check "@X"`:** espia o colega SEM mandar prompt. O `ScrollbackStore` foi DESENHADO pra isso
  (`crates/lina-core/src/scrollback.rs:30` cita "um leitor externo `lina check --tail N`"), mas
  **atenção:** o `scrollback.db` NÃO está sendo persistido no workspace (confirme: `find <ws>/.lina -name
  'scrollback.db'`). Investigue por que (o `ScrollbackStore` é aberto? o sink é ligado?). Se não houver
  scrollback persistido, a 1ª versão do `lina check` pode mostrar **status vivo + atividade A2A recente
  do colega** (lido do `agents.json` + `log.jsonl`) e deixar o tail-de-tela como incremento.
- **Bônus:** documentar/canonizar o intent `status` que os agentes inventaram (`#3` das notas) — ou
  removê-lo da prática ajustando a doutrina.

---

### 🟠 P3 — Adoção zero do sistema de PLANO + topologia em estrela (a causa PROFUNDA)

**Observado:** em ~14h, **ZERO** uso de `lina plan claim/running/review/check`. A doutrina tem um
sistema de coordenação estruturada (`.lina/plan.md`: claim de tarefa, status, anti-colisão), mas os
agentes o IGNORAM — coordenam 100% por `lina ask` ad-hoc pra A (topologia ESTRELA, A é gargalo, ninguém
fala B↔C direto). `--await` quase não usado (1/19). Isto é o que TORNA o atropelamento possível: sem
plano, A é o único ponto de sincronização.

**Isto NÃO é um bug — é design/adoção.** Investigue e proponha (brainstorm com o fundador):
- A doutrina (`assets/lina-doctrine/CLAUDE.md`, gerada por `crates/lina-bootstrap`) ensina o plano de
  forma que convença o agente a usá-lo? Talvez precise de instrução mais forte/exemplos.
- Vale o orquestrador (Terminal A) ser instruído a distribuir via plano (claim) em vez de só `ask`?
- Esse item provavelmente vira uma rodada PRÓPRIA depois que P0-P2 estiverem firmes. **Não comece por
  aqui** — sem o P0/P1 resolvidos, mexer na coordenação não adianta.

---

### 🟡 P4 — Lifecycle: reabrir o app deixa nós-fantasma no log

**Observado:** 3 gerações de terminais (3→6→8 NodeIds) = 3 reaberturas do app no mesmo workspace, com
**ZERO** evento de NodeRemoved/Dead pros antigos (17 NodeAdded / 0 NodeRemoved vs 8 vivos). Não causa
bug visível (o roster vivo vem do Supervisor, não do replay), mas é uma **tensão com o invariante #4**
(log = fonte da verdade): um replay do log "veria" 17 terminais. Investigar se ao reabrir o app deveria
emitir o ciclo de morte dos nós da geração anterior, ou se há um snapshot/compactação que resolve.
**Baixa prioridade** — anote, não bloqueie nas outras.

---

### ✅ O que JÁ funciona bem (não quebrar)

- **Estabilidade:** 8 Claudes reais rodando ~14h, ~338 token-reports/min, **zero crash**.
- **A2A entrega de forma confiável** (a race de registro foi resolvida pelo retry transiente `f353838`;
  20/20 msgs roteadas → entregues no período observado).
- **Persistência durável** (15k+ eventos, espelho jsonl, sem corrupção), **spawn/PTY/render/IME**,
  **infra de segurança** (custódia, guard, freio, autenticação anti-impersonação).
- Ao mexer no `deliver_a2a`/Router, **rode a suíte de segurança** do roteador
  (`unknown_sender_and_target_are_rejected`, `plan_claim_*`, anti-loop, fan-out) — não regrida.

---

## 3. Sequência sugerida de execução

1. **Confirme o estado** (`git log`, leia `CLAUDE.md` + este doc) e crie sua branch de trabalho.
2. **P0** (perfil de entrega) — é o de maior impacto e mais contido. TDD: teste headless do
   `deliver_a2a` provando o comportamento errado (injeta sem prompt pronto), depois o fix, depois o
   teste prático. Commite.
3. **P1** (atropelamento) — brainstorm da abordagem, implemente a escolhida, teste headless + prático.
   Commite.
4. **P2** (`lina handoff`/`check`) — fácil, alto valor de UX p/ o agente. Commite.
5. **Re-teste prático completo** (seção 4) e relate o comportamento observado.
6. **P3/P4** ficam para uma rodada seguinte, com o fundador, se P0–P2 melhorarem o quadro.

A cada item: gates verdes (check/test/clippy) + commit incremental + (se tocou A2A) rebuild release +
`make-app.sh`.

---

## 4. TESTE PRÁTICO (a prova final — replica a observação)

Os gates headless NÃO provam o A2A com Claude real. Faça assim:

1. **Build + bundle:** builde o bin `lina` (`cargo build --release -p lina-bootstrap --bin lina`) e o
   app (`cd app/lina-gpui && cargo build --release`), depois `bash packaging/make-app.sh --skip-build`.
2. **Abra o app:** `open "dist/Lina.app"` (ou rode o binário direto com `LINA_WS_ROOT` apontando p/ um
   workspace de teste, p/ não poluir o de produção). Crie 3-5 terminais Claude.
3. **Observe o event log em tempo real** (técnica usada na observação):
   ```
   WS="$HOME/Library/Application Support/Lina/walking-skeleton/.lina"   # ou seu LINA_WS_ROOT/.lina
   tail -f "$WS/events/log.jsonl" | python3 -u /tmp/lina_watch.py        # script de observação (ver abaixo)
   ```
   O `/tmp/lina_watch.py` filtra o ruído de tokens e imprime A2A/plano/lifecycle com nomes legíveis
   (foi deixado em `/tmp` nesta sessão; se sumiu, recrie: mapeia NodeId→nome via TerminalSpawned, pula
   `TokenUsageReported`/`MessageDelivered`, imprime os demais kinds com `from`/`to`/`intent`/`reason`).
4. **Reproduza os bugs ANTES do fix** (linha de base) e DEPOIS:
   - **Texto colado:** peça a um terminal pra mandar `lina ask` pra um colega que está OCUPADO
     processando algo longo. ANTES: às vezes o texto cola sem submeter. DEPOIS: submete (espera o
     prompt do colega).
   - **Atropelamento:** faça um terminal (A) mandar `lina ask` pra vários colegas com tarefa CURTA
     (terminam juntos). ANTES: os retornos colidem no input de A (veja no log retornos a A com < 2s de
     intervalo; veja na tela o input de A embaralhado). DEPOIS: serializado/sem colisão.
5. **Métricas de sucesso a coletar** (compare antes/depois):
   - `RouteBlocked` deve seguir ~0 (a race já está resolvida — não regrida).
   - Intervalo entre retornos ao mesmo alvo: não deve haver injeção concorrente.
   - Inspeção visual: nenhum `lina ask` "preso" como texto não-submetido.
6. **Relate** o comportamento observado (com trechos do log) — esse é o entregável final, não só "os
   testes passaram".

---

## 5. Referência rápida (arquivos-chave)

| Arquivo | Papel |
|---|---|
| `app/lina-gpui/src/bridge.rs` | `MailboxPump` (drena+roteia+entrega), `deliver_fn`, `demo_profile()`, `wire_terminal`, `BootstrapWriter` |
| `crates/lina-core/src/a2a.rs` | `deliver_a2a` (paste→delay→Enter), `wait_ready`, timeouts, fim-de-resposta |
| `crates/lina-core/src/router.rs` | `Router::pump`/`route_message`, guardrails, retry transiente, anti-loop |
| `crates/lina-core/src/lib.rs` | `Supervisor` (roster vivo), `node_by_name`, `register`, status |
| `crates/lina-bootstrap/src/bin/lina.rs` | bin `lina` (subcomandos do agente: ask/vault/plan/…) |
| `crates/lina-cli-profiles/src/lib.rs` | `CliProfile`, `ProfileRegistry::load_dir`, `Installers` |
| `profiles/claude-code.toml` | o perfil CERTO de entrega (regex do prompt + submit_delay) |
| `assets/lina-doctrine/{CLAUDE,AGENTS,GEMINI}.md` | doutrina gerada (promete handoff/check; ensina plano) |
| `crates/lina-core/src/scrollback.rs` | `ScrollbackStore` (desenhado p/ `lina check --tail`) |

**Notas de campo completas (origem deste doc):** `/tmp/lina-observacao.md` (10 achados + retrato + o que
funciona bem). Se sumiu do `/tmp`, este doc já carrega o essencial.
