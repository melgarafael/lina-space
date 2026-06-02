# Design executável — Onda 3 (Orquestração / Camada de IA)

> Extração densa para o Master Maestro distribuir a construção do **cérebro** sem reler docs gigantes. Fontes: `21 - Camada de IA e Orquestracao` (§1, §2.5, §3, §4) + `30 - SPEC Mestre` (§6, §8 Trilho B). Sem código.
> **Princípio duro (invariante):** o app **NUNCA chama LLM**. É roteador + **supervisor de sessões com estado** (roster, presença, mailbox, locks, wait-for-graph, event log). Toda inteligência vive no agente que recebe o texto.
> **Onda 3 é Trilho B → consome o core W0 (não o render W2); valida-se headless via `.lina/events/log.jsonl`.**

## 🎯 CRITÉRIO DE ACEITE DO FUNDADOR (01/06) — o que TUDO isto precisa entregar
Um agente num terminal do Lina deve **saber que está no Lina** e **usar A2A sozinho** — o leigo digita *"manda oi pro Terminal B"* e o agente dispara a mensagem — **sem olhar a tela nem pesquisar a máquina**.
- *"saber que está no Lina" + conhecer colegas* = **W3-2 Bootstrap** (a ficha injetada no turno 0).
- *"manda oi pro Terminal B" → vira `lina ask @TerminalB "oi"`* = **W3-3 Skill A2A** (traduz linguagem natural do leigo em verbo `lina`, reconhece colega vs humano).
- Os dois juntos = o cérebro funcional. É o teste E2E que fecha a parte alta da Onda 3.

---

## 1) W3-2 · BOOTSTRAP TURNO-0 + system message canônico de 8 blocos

### Os 8 blocos fixos (ordem canônica — `<workspace>/CLAUDE.md`)
Blocos 1–2 = identidade · 3–4 = as duas mágicas · 5–7 = guardrails · 8 = tom. `{{...}}` preenchidos pelo app de `workspace.json`/`agents.json`/`vault.json`.

| # | Bloco | Conteúdo | Fonte `{{...}}` |
|---|---|---|---|
| 1 | **Quem você é** | identidade + **"estar no workspace = estar no time"** + **como reconhecer mensagem de colega** (sentinela `[LINA::MSG]`/`[LINA::HANDSHAKE]`) | `workspace.json` |
| 2 | **Seu papel** | papel canônico deste terminal + skills a carregar (`{{role_skills_list}}`, **+ `lina-agent-bus` sempre**) | nome do terminal → `agents.json` |
| 3 | **Vault como contexto** | "consulte o vault ANTES de assumir/perguntar"; verbos `lina vault search/read/path`; cite com `[[wikilink]]`; **nunca peça o caminho** (vem do bootstrap) | `vault.json` |
| 4 | **Auto-orquestração** | regra de 3 passos (está no meu papel→faço; precisa outro→**delego**; falta papel→marco no plano) + tabela papel→verbo + handshake + plano como fonte da verdade + anti-deadlock | `agents.json` (colegas) |
| 5 | **Autonomia** | qual dos 3 níveis vale agora e o que cada ação pode fazer | `workspace.json → autonomy` |
| 6 | **Orçamento + anti-loop** | teto de delegação/turno, profundidade de cadeia, anti-eco | defaults canônicos |
| 7 | **Gate humano** | ações irreversíveis / fan-out grande pedem confirmação sim/não | defaults canônicos |
| 8 | **Tom para o leigo** | narrar ações em pt-br simples, sem jargão (PTY/handoff/sentinela), na hora em que age | fixo |
> Lembrete final do template: **precedência de verdade** = instrução-corrente do usuário > `## Decisões` do `plan.md` (estado vivo) > vault (histórico, pode estar velho).

### Como a "ficha" é injetada — DOIS canais (arquivo é à prova de falhas; hook é enriquecimento)
- **Canal 1 — arquivo de doutrina** (`CLAUDE.md` / `AGENTS.md` / `GEMINI.md`): a **doutrina PERMANENTE** (o que não muda turno a turno). **O app reescreve este arquivo a cada mudança de roster** (entrar/sair colega, mudar papel/autonomia/plano). É o canal **à prova de falhas**.
- **Canal 2 — hook `SessionStart`** (só Claude Code) → roda `lina whoami --bootstrap` → injeta o **ESTADO VIVO** (colegas online, plano atual, vault path) via `hookSpecificOutput.additionalContext` (JSON estável, **nunca stdout cru**).
- **Por que o arquivo é a âncora e NÃO o hook:** o `SessionStart` **falha silenciosamente em conversas novas** (= exatamente o turno 0) e seu formato variou entre versões do Claude Code. Se o hook falhar, o arquivo ainda carrega papel + colegas + regras.

### Bloco que `lina whoami --bootstrap` imprime (estado vivo)
```
=== Lina Space BOOTSTRAP ===
Voce e o terminal "@Dev Backend" -> papel BACKEND.
Skills a carregar AGORA: senior-backend, tomik-db-doctrine, lina-agent-bus.
VAULT OBSIDIAN: <path>  (use `lina vault search "termo"`; voce JA tem acesso; nao peca o caminho)
COLEGAS NESTE WORKSPACE: @Arquiteto (ARQUITETO) — decide contratos…; @QA (QA) — valida…
PLANO COMPARTILHADO: .lina/plan.md  (use `lina plan read`)
AUTONOMIA: assistido (proponha delegacoes; o Maestro confirma)
PRIMEIRO PASSO OBRIGATORIO: 1) carregue skills  2) `lina handshake`  3) `lina plan read` (item @owner:voce?)
=== FIM BOOTSTRAP ===
```

### Adaptador de bootstrap por CLI (CONTEÚDO idêntico; só o GATILHO muda)
| CLI | Gatilho | Doutrina |
|---|---|---|
| **Claude Code** (tier 1, referência) | hook `SessionStart` → `lina whoami --bootstrap` (nativo, à prova de falhas) | `CLAUDE.md` + `.claude/skills/role-<PAPEL>/` |
| **Codex** | sem hook → app injeta bootstrap como 1º input no PTY | `AGENTS.md` |
| **Gemini CLI** | idem Codex (injeção no PTY) | `GEMINI.md` |
| **shell puro / OpenCode / Copilot** (tier 2) | injeção no PTY; **doutrina 100% no arquivo/bloco** + re-injeção curta após N turnos | bloco impresso |
> ⚠️ Em CLIs **sem hook**, o bootstrap vai **100% no arquivo de doutrina**, nunca numa injeção de input que o REPL pode não submeter. Tier 1 = Claude Code (hook+arquivo); tier 2 = demais (só arquivo + re-injeção). Promete-se paridade de **conteúdo**, não de mecanismo.
> Hook **`UserPromptSubmit`** (1 único hook encadeado, opcional): (1) `lina guard --check-autonomy` (recusa delegação em `manual`) → (2) tag de origem (anota se input tem sentinela `[LINA::…]`) → (3) `lina plan --reconcile`. Tudo string/arquivo, zero LLM.

### ✅ Critério de aceite W3-2
Add `@Dev Backend` ao workspace → `<workspace>/CLAUDE.md` existe, contém **os 8 blocos na ordem** e `{{role_canonical}}=BACKEND` resolvido · `lina whoami --bootstrap` imprime os **colegas reais** do roster · **renomear** um colega **reescreve** o `CLAUDE.md` (diff observável) **sem reiniciar o app**.

---

## 2) W3-3 · SKILL A2A (`lina-agent-bus`)

### Sentinelas — como o agente distingue COLEGA de HUMANO
- Input que **começa com `[LINA::MSG]`** = mensagem de colega (ask/handoff/broadcast). Input que **começa com `[LINA::HANDSHAKE]`** = apresentação automática de colega no turno 0.
- **Qualquer input SEM sentinela = usuário** (texto livre).
- **Enforcement real é do CANAL, não da string:** o supervisor injeta a mensagem por uma **fila serial no PTY**, separada do input que o humano digita — o app **sabe por construção** que é A2A. A sentinela é **UX/legibilidade** (a IA e o humano leem de onde veio); não se confia que o LLM "não será enganado por uma sentinela colada".
- A skill `lina-agent-bus` (e o `CLAUDE.md`, bloco 1) **ensina** isto: *"input começando com `[LINA::MSG]`/`[LINA::HANDSHAKE]` vem de um colega via supervisor; processe `from`/`intent`/`payload` e responda no formato `[EXPECTED]`."* Sem isso, CLIs sem hook não distinguem colega de humano.

### Formato de resposta
Ler `from` / `intent` / `payload` e **responder no formato pedido em `[EXPECTED]` / `expected:`** — para a resposta voltar acionável e o remetente não precisar de 2ª rodada (herda do template real da `maestri-orchestrator`).

### Antídoto de eco (load-bearing p/ o leigo)
- **NUNCA** ecoar/explicar/citar o bloco técnico (`[LINA::MSG]`, `from:`, `hops:`…) ao usuário — é comunicação interna. Para o usuário, **só narra o resultado em pt-br simples** (bloco 8).
- **Handshake:** o bloco `[LINA::HANDSHAKE]` traz, na última linha entre parênteses, *"NÃO responda — handshake é informativo, não interrogativo"*. Como o handshake é **lido de arquivo** (`agents.json`), não roteado, a tempestade já é estruturalmente eliminada; o antídoto cobre o caso de o bloco ser citado dentro de uma mensagem.
- **Anti-eco/anti-loop semântico:** não devolver ao colega a MESMA pergunta que ele fez (entrega X ou diz por que não consegue); nunca `broadcast` em resposta a `broadcast`.

### Mapa do time (injetado, custo O(1))
A skill lê `agents.json` (papel, skills, **`delega-me quando`**) e popula o mapa mental do agente — sem rotear N mensagens. Embute a tabela *"preciso de X → delego ao papel Y → verbo Z"*.

### ✅ Critério de aceite W3-3
Injetar no PTY de um agente `[LINA::MSG] from:@Arquiteto intent:handoff expected:"contrato em docs/"` → ele **produz o artefato no formato `expected`** e **NÃO repassa** o bloco `[LINA::MSG]` ao usuário · injetar texto **sem** sentinela → trata como pergunta do humano. Verificável por **golden-transcript** de um terminal real.

---

## 3) ENVELOPE A2A CANÔNICO (`lina/msg@1`, §3.4)

Texto puro chave→valor (o supervisor concatena strings antes de injetar). Toda mensagem (`ask`/`handoff`/`broadcast`) viaja nele.

| Campo | Obrig. | Semântica |
|---|---|---|
| `[LINA::MSG]` | sim | **Sentinela de prefixo** (UX/legibilidade). Enforcement de origem é do **canal**, não desta string. |
| `id` | sim | **ULID único da mensagem** → dedupe / anti-loop (dedupe por `id` em janela de **60s**). |
| `root_cause_id` | sim | **ULID do TURNO do usuário** que originou a cascata. **Carimbado pelo supervisor, NÃO pelo LLM.** Âncora do orçamento (`delegation_budget`/turno) e do anti-loop (grafo de eventos). |
| `from` | sim | terminal remetente. |
| `to` | sim | alvo (ou `*` / `--role X` em broadcast). |
| `intent` | sim | `ask` \| `handoff` \| `broadcast` \| `review` \| `status` \| `handshake`. |
| `ref` | opc. | item do plano (`plan:T4`) ou recurso (`file:docs/api.md`) — liga ao `plan.md`. |
| `context` | opc. | caminhos de arquivo/nota anexados (**só em `handoff`**). |
| `hops` | sim | contador de saltos A→B→C; corta cadeia em `max_depth=4`. **Mesmo conceito que profundidade de `--await`.** |
| `reply_to` | opc. | `id` da mensagem que originou esta (rastreia cadeia). |
| `await` | sim | `true` (bloqueia até resposta) \| `false` (fire-and-forget). **Default `false`.** |
| `payload` | sim | o texto da tarefa/pergunta. |
| `[EXPECTED] …` | (rodapé) | formato de retorno esperado (fecha o envelope; herda da `maestri-orchestrator`). |

> ⚠️ **Pegadinha de rename:** o exemplo no doc 21 (§3.4) ainda fecha com `[/ESTUDIO::MSG]` — **resíduo do nome antigo** ("Estúdio" → "Lina Space", decisão §10.1 do doc 30 "propagar o rename"). O terminador canônico deve seguir o namespace `LINA`, **não** copiar `ESTUDIO`. Consolidar no contrato versionado.
> ✅ **Contrato NÃO fragmentado:** doc 30 feature #5 (P0) lista o envelope como `id/root_cause_id/from/to/intent/hops/await` + sentinela — ou seja, **`root_cause_id` e `hops` nascem já no W0-4** (não são bolt-on de W3-3/W3-7). O aceite de W3-7 deve assertar o envelope final completo num teste de serialização.

### Entrega (§3.5) — recapitulação (já encanada na Onda 2)
Injeção faseada no PTY vivo (primário): **bracketed-paste → `submit_delay`(~0.3s) → Enter `0x0D` separado**; **sanitiza `ESC[201~`** (CVE-2021-31701); `texto+\r` monolítico NÃO submete. Fallback headless: sessão (`claude --resume` / `codex exec resume` / `-p`). Shell puro: mailbox `.lina/inbox/`. Pipeline `route_message` aplica guardrails 0–4 (alvo existe → autonomia → anti-deadlock → orçamento → presença/fila) antes da entrega.

---

## 4) ORDEM DE CONSTRUÇÃO + PARALELISMO

### Sequência canônica do Trilho B (doc 30 §8)
`Fundação (.lina/ + verbos)` → **W3-1 Role-discovery [FEITO]** → **W3-2 Bootstrap + 8 blocos** → `Entrega A2A` → **Envelope** → `anti-deadlock + anti-loop` → **W3-5 plan.md** → **W3-6 Autonomia + gate duro + W3-7 guardrails** → **W3-4 spawn preguiçoso + handshake por arquivo + W3-3 skill A2A`.

### O que arranca JÁ vs sequencial vs arquivos paralelos
- **JÁ FEITO:** **W3-1** (`lina-role-discovery`, crate isolada, puro Rust, zero LLM).
- **ARQUIVOS — paralelizáveis** (distintos do código do supervisor, integram no fim; só exigem o **contrato do envelope/sentinela congelado**):
  - **W3-2** = templates `CLAUDE.md`/`AGENTS.md`/`GEMINI.md` (8 blocos) + renderer de `{{placeholders}}` + `lina whoami --bootstrap` + hook `SessionStart`.
  - **W3-3** = skill `lina-agent-bus` (markdown/doutrina: reconhecer sentinela, formato `[EXPECTED]`, antídoto de eco, mapa do time).
  - → **W3-2 e W3-3 podem correr em paralelo** ao código do supervisor (um dev na doutrina/skill, outro no supervisor).
- **SUPERVISOR — SEQUENCIAL (mesma cadeia `lina-core`, escritor único de `.lina/`):** **W3-4 → W3-5 → W3-7**, com **W3-6** dependente de W3-5 (+W3-3). Todos mutam o mesmo estado (roster, mailbox, locks, `log.jsonl`, `plan.md`) → **não paralelizar entre si** (o supervisor é escritor único por design anti-corrupção cross-platform — sem `flock`).
  - **W3-4** spawn preguiçoso (sobe `@Maestro`+owner do item-1; demais on-demand) + handshake por arquivo (`agents.json`, **0 broadcast** no boot).
  - **W3-5** `plan.md` (parser rígido + escritor único + event-sourced `plan.claim`/`check`).
  - **W3-6** autonomia (manual/assistido/autônomo) + **gate DURO** no ponto de execução (`PreToolUse`/wrapper sobre `git push`/deploy/`rm -rf`/pagamento).
  - **W3-7** guardrails P0 (tabela abaixo) — **sorvedouro / gate da onda**.

### Guardrails P0 determinísticos (W3-7 — contador/grafo/timer, NUNCA LLM)
| Guardrail | Mecanismo | Default |
|---|---|---|
| Anti-deadlock | **wait-for graph**: se B já espera A, recusa `--await` A→B | imediato |
| Profundidade (=`hops`) | campo `hops` + cadeia de `--await` | `max_depth=4` |
| Dedupe | guarda `id` entregues | janela **60s** |
| Anti-loop (ciclo A→B→A) | **grafo de eventos `handoff from→to`** no `log.jsonl`, por item de plano + `root_cause_id`, **independe do texto** | imediato |
| Orçamento de delegação | conta eventos com mesmo `root_cause_id` por agente | `delegation_budget=8`/turno |
| Gate de fan-out | `broadcast` > N alvos exige confirmação | `fanout_gate=3` |
| Gate ação irreversível | `PreToolUse`/wrapper sobre comando real (não grep de intenção) | sempre |
| Teto de custo | tokens por workspace/dia → pausa com gate | `token_budget_day` |

### Pré-requisitos cross-onda (do core W0; Onda 3 NÃO depende do render W2)
1. **W0-4** Workspace Bus/Supervisor (NodeRegistry, MailQueues serial, presença, locks, wait-for-graph) — base de W3-4/5/7. *(já exercitado pelo walking skeleton)*
2. **Envelope canônico versionado** no W0-4 com `root_cause_id`+`hops` (ver §3 acima) — e **consolidar o terminador `[/LINA::MSG]`** (não `ESTUDIO`).
3. **W0-5** Event Store (SQLite WAL + JSONL) — `plan.md` event-sourced + `log.jsonl`.
4. **W0-8/9/10** CLI Profiles + entrega A2A faseada + fim-de-resposta — base da skill A2A e do roteamento.
5. **W0-7** Secret Vault (token-por-nó). · **Gate W0 headless** confirmado.
> **Spike de maior incerteza (doc 30 §8, risco #3):** validar a **entrega A2A via API de sessão** (`--resume`/`exec resume`/`-p`) + fallback mailbox nos 5 CLIs **antes** de robustecer o supervisor. O **primário (injeção faseada no PTY) já está encanado na Onda 2**; o spike cobre o fallback de sessão.

### ✅ Critério de aceite W3 (gate da onda, headless via `.lina/events/log.jsonl`)
2 agentes num Espaço com preset: supervisor sobe `@Maestro`+owner item-1 (W3-4); ambos resolvem papel pelo nome (W3-1) + bootstrap 8 blocos (W3-2); `lina handshake` por arquivo e se reconhecem por sentinela (W3-3); um roda `lina plan claim` num item seu (W3-5) — **tudo SEM instrução de orquestração do usuário** — e os guardrails (W3-7) + gate de autonomia (W3-6) provam bloqueio de loop/deadlock/budget. Sequência esperada no log: `agent.joined → handshake → plan.claim`, sem evento originado em input humano além do turno-0.

PRONTO
