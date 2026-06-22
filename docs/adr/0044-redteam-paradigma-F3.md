# ADR 0044 — Red-team de paradigma (fechamento da Fase 3): algum invariante virou dogma?

- **Status:** 🟡 Proposto (2026-06-22, Arquiteto Terminal A) — rito de fechamento da Fase 3 (épico `39` §III; spec-mestre `50`, Meadows ponto [2] Paradigma)
- **Onda/Story:** transversal — fecha a Fase 3 (Maestro Nativo, F3-0..F3-5)
- **Fontes:** `CLAUDE.md` raiz (7 invariantes + âncoras) · épico `39` §III (a regra: *"qual invariante virou dogma que custa mais do que protege?"*) + §VI (top riscos) · evidência empírica das 6 ondas (F3-0..F3-5, achados de dogfooding #25-#43, freeze attention_heartbeat, costura órfã branch.integrated, working-tree compartilhado #38)

> **O valor deste rito é FORÇAR A PERGUNTA, não mudar o invariante.** A revisão é adversarial e honesta: para cada invariante, *onde ele nos custou na Fase 3?* — e só então o veredito.

## A pergunta-mãe

Examinados um a um, **nenhum dos 7 invariantes virou dogma** — todos ainda protegem mais do que custam, com evidência. **Mas o red-team honesto achou 3 tensões reais (custo > zero, recorrente)** que merecem **mitigação estrutural — sem tocar o invariante** (todas as propostas o REFORÇAM, reduzindo seu imposto).

## Exame por invariante (evidência F3 → veredito)

### #1 — Sem LLM/harness/chat próprios (zero-LLM-no-core) · **PROTEGE ≫ custa**
- **Custo F3 (real):** forçou o Refletor da Mentality (F3-3) e a fábrica de skills (F3-5) a **1 CLI-call assíncrona fora do caminho crítico** — a inteligência semântica nunca é síncrona/inline no core. O **anti-poisoning SEMÂNTICO** da Mentality foi **DEFERIDO** (só o estrutural/determinístico entrou). Foi o risco #1 da fase (§VI-1), exigindo vigilância constante ("não recriar harness").
- **Protege:** core determinístico → replay byte-a-byte, testável **headless em 3 OS** (o CI Linux/macOS/Windows existe por causa disto), e neutro (não casado com um LLM, reforça #3). Sem #1, o core teria não-determinismo (impossível replay/CI) e violaria #3.
- **Veredito:** o invariante mais forte e o que mais exige disciplina — **paga o custo sem bloquear nada essencial** (o estrutural cobre o MVP; o semântico é refinamento async). Não é dogma.

### #2 — Local-first · **PROTEGE · custo nulo na F3**
- **Custo F3:** ZERO — nenhuma feature precisou cross-machine. `DiskBudget` (F3-5) e auto-spawn (F3-4) são single-machine por natureza; `BusTransport` (ADR 0034) ficou **nominal** (porta para F4, custo de manutenção ≈0).
- **Veredito:** não-exercido contra nada nesta fase ⇒ não pode ser dogma aqui. Protege soberania/privacidade (alinhado à direção de produto). Mantém-se.

### #3 — Neutralidade multi-CLI · **PROTEGE > custa · com DÍVIDA DE ADERÊNCIA honesta**
- **Custo F3 (real):** o `effort` (low/medium/high) e o `resume_args`/`can_resume()` (F3-5) são neutros via CLI Profile — **mas há acoplamento prático ao Claude Code**: os **hooks** (`.claude/settings.json`, PreToolUse/PostToolUse) são específicos do Claude Code, e o `--resume` é dele. **Trocar de CLI exigiria portar os hooks, não só o Profile TOML.** O slogan "trocar CLI = trocar um Profile" vale para roteamento/effort/sessão, **não** para a camada de hooks.
- **Veredito:** protege um diferencial central (não ser refém de um LLM) e não bloqueou feature alguma — **não é dogma**. Mas a **aderência é imperfeita** e vale nomeá-la (proposta abaixo). É a tensão mais honesta da fase: o invariante é parcialmente aspiracional.

### #4 — O event log é a fonte da verdade · **PROTEGE ≫ custa · IMPOSTO recorrente (o achado mais forte)**
- **Custo F3 (real e REPETIDO):** o replay é a fonte do **risco de freeze**: `attention_heartbeat` replayava o log **inteiro O(N) na main-thread** (causa-raiz de travamento); na F3-5, "fiar projeção no pump exige throttle/delta" repetiu a lição. A **disciplina anti-freeze** (nunca full-replay no hot-path; throttle/delta) virou um **imposto cognitivo recorrente** — cada frente que fia uma projeção precisa lembrar, ou reintroduz o freeze.
- **Protege:** auditabilidade total, reconstrução, nada se perde, replay byte-a-byte — **o ativo durável do produto**.
- **Veredito:** **não é dogma** — o freeze é culpa de *implementação* (full-replay no hot-path), não do invariante. Mas o imposto é o mais alto e mais recorrente da casa. **PROPOSTA 1 (reforça #4, não o revisa):** um padrão/trait CANÔNICO de **projeção incremental** (cursor de `seq`, fold por delta) que torne *full-replay-no-hot-path* um **erro de tipo**, não uma tentação repetida. O `dispatch_progress_seq` já é a semente; falta promovê-lo a contrato.

### #5 — Pertencimento = conexão · **PROTEGE · custo baixo**
- **Custo F3:** ~nulo — o broadcast por pertencimento (F3-4 `CodeChanged`) e o auto-spawn usam o **roster vivo** (`Supervisor::resolve` filtra `is_alive`), zero topologia manual.
- **Veredito:** diferencial sem atrito (sem cabos). Não é dogma.

### #6 — Não-técnico-first · **PROTEGE ≫ custa · DÍVIDA de processo (gate visual)**
- **Custo F3 (real):** o **gate visual BLOQUEANTE por onda** acumulou dívida — a Fase 3 está "completa em código" mas **3 ondas (F3-3, F3-4, F3-5) esperam a validação visual do fundador**; o desenvolvimento corre mais rápido que a validação, que depende de **1 pessoa na tela**. E cada feature paga uma camada de tradução leiga (o detail técnico do CodeConflict *vazou* em F3-4, exigiu conserto).
- **Protege:** é a **TESE do produto** (a razão de existir): zero jargão, estado sempre visível, o leigo no comando.
- **Veredito:** não é dogma — é o norte. Mas o gate **por onda** vira gargalo. **PROPOSTA 2 (reforça #6, não o afrouxa):** **validação em LOTE** — uma sessão de tela cobre as ondas pendentes de uma vez, em vez de bloquear onda a onda.

### #7 — Core/shell split · **PROTEGE > custa**
- **Custo F3:** duas camadas (core-decide / shell-executa) em cada ação de mundo — o git de runtime (F3-4) e a poda de disco (F3-5) ficaram no shell/broker, com a *decisão* pura no core (às vezes uma fn quase cerimonial, ex.: `worktree_branch_name` só para o nome).
- **Protege:** core testável headless (o CI 3-OS pegou o flaky de PTY justamente por rodar o core sem UI) e UI substituível (`UiHost`).
- **Veredito:** o custo (duas camadas) é o preço da testabilidade + substituibilidade, comprovado em campo. Não é dogma.

## Exame das âncoras de continuidade

- **CLI Profiles · Event Store · Bus/Supervisor · Envelope A2A** — muito exercidas na F3, protegem (Event Store ver #4). Saudáveis.
- **`UiHost` · `VtBackend` · `PortalEngine`/`ExternalTextureLayer`** — **NÃO exercidas** na F3 (apostas de futuro: troca de UI/VT, browser navegável-pela-IA). Custo de manutenção ≈0 hoje; proteção alta **se** F4/F5 as exercer. **Honesto:** são apostas não-validadas — se o futuro nunca chegar, viram peso morto. Mas o custo de mantê-las nominais é baixo. Sem ação por ora; reavaliar em F4.

## Conclusão

**Resposta à pergunta-mãe: NENHUM invariante virou dogma na Fase 3.** Todos os 7 protegem mais do que custam, com evidência das 6 ondas. O red-team, porém, **não foi um carimbo** — achou **3 impostos reais e recorrentes**, todos endereçáveis SEM tocar o invariante:

1. **#4 (imposto anti-freeze)** → padrão canônico de projeção incremental (full-replay-no-hot-path = erro de tipo). *[maior alavanca]*
2. **#6 (gate visual acumula dívida)** → validação em lote multi-onda.
3. **#3 (dívida de aderência ao Claude Code)** → nomear que o "CLI Profile completo" deve abstrair hooks/sessão, não só args.

Nenhuma proposta contradiz um invariante (não exige item de revisão de invariante; todas o **reforçam**). São backlog técnico/processo, com dono a definir pelo Maestro/fundador.

## Parecer: implementar o evento `ParadigmReviewed`?

**Recomendo IMPLEMENTAR a variante** (feito neste mesmo commit). Razão de princípio: **um rito que audita o invariante #4 ("o event log é a fonte") não pode viver só num `.md`** — seria incoerente com o próprio invariante sob exame. `ParadigmReviewed{phase, invariant_challenged, verdict, adr_ref}` torna o rito um **fato auditável por replay** (dogfooding do #4). É barato (1 variante aditiva, sem `serde(default)` nos obrigatórios; braço em `kind()`; no-op em `apply()` — META, padrão da largada F3-5).

**Ressalva (porta aberta, não costura órfã):** implemento o **contrato** (variante + teste de aditividade), **não o produtor**. A emissão fica para um verbo `lina paradigm` futuro **ou** o gesto do Maestro/fundador ao aceitar este ADR. Diferente da órfã `branch.integrated` (F3-4), aqui **não há projeção que fique inerte** sem o produtor — é um evento de auditoria, não um sinal de feature; sem emissor, simplesmente nenhum `ParadigmReviewed` existe (o estado atual de qualquer forma). Documentado para não esquecer.

## Consequências

- **Positivas:** a Fase 3 fecha com uma auto-crítica honesta e auditável; 3 impostos nomeados viram backlog explícito (não dívida silenciosa); o paradigma da casa sai do rito **reforçado**, não enfraquecido.
- **A fazer (backlog, dono = Maestro/fundador):** as 3 propostas acima; fiar o produtor de `ParadigmReviewed` (verbo/gesto) quando houver o próximo rito (fechamento de F4).
