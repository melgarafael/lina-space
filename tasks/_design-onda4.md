# Design executável — Onda 4 (UX do MVP / Lina Space)

> Extração densa para o Master Maestro distribuir o fan-out da construção da **Onda 4** sem reler o épico inteiro. Fontes: `32 - Epico Fase 0 (MVP)` §"Onda 4 — UX do MVP" (W4-1…W4-6 + GATE W4) + §"Pré-requisitos cross-onda"; `12 - Red-team Escala e Benchmark` (P0 de degradação); `tasks/_resumo-ondas-2-3.md` (estado real Onda 2/3); estado de código (`crates/`, `app/lina-gpui/`).
> **Invariante norteador (CLAUDE.md #6 NÃO-TÉCNICO-FIRST):** zero jargão na superfície · 1º agente respondendo rápido (gate épico: **<15min**) · **nunca tela em branco** · estado **sempre salvo e visível**. + **#2 LOCAL-FIRST:** nada sai da máquina por padrão; exposição é opt-in sinalizado. + **#1:** o app **nunca chama LLM** (a inteligência é o CLI de terceiro).
> **Princípio de produto da Onda (red-team doc 12 §6.3):** desenhar para **"POUCOS EM FOCO, MUITOS EM FILA"** — NÃO "muitas janelas vivas". Foco = 1-4 terminais legíveis; periferia = nós-cartão com badge de status agregado. Isto é **P0**, não polimento — ver §0.
> **Trilho:** C-UX. Superfície dominante: o **shell** `app/lina-gpui` (fora do workspace; hospeda gpui + render W2). Stories tocam o shell + pequenas adições no core (`lina-core`).

---

## 0) ⭐ P0 TRANSVERSAL — Degradação de nós ociosos/fora-de-foco (red-team escala, doc 12)

> **Origem:** o red-team de escala (doc 12) **refuta a premissa forte** ("dezenas operadas ao vivo") e prova que o teto real não é render (folgado, centenas) e sim **RAM/CPU dos CLIs (~4-7 ativos)** + **atenção humana (~2-6)**. A peça que **casa produto e stack vira P0**: *fazer nós ociosos/fora-de-tela ficarem baratos* (como Chrome Memory Saver ~40% RAM / Edge Sleeping Tabs 83-85% RAM, 99% CPU). É o **Gate C do benchmark** (doc 12 §5.3) e a "implicação para a Onda 4" (§6.3).

**O mecanismo (4 peças — desenho de requisito, doc 12 §6.2):**
1. **Culling de viewport** — nó fora-de-tela não desenha. → custo de **render** de ociosos. **JÁ ENTREGUE** como estado `SUSPENSO` em **W2-3** (0 draw-calls, grid vivo no core). A Onda 4 **reusa**, não reimplementa.
2. **Throttle de output de nó SEM FOCO** — terminal em background atualiza o grid em **baixa frequência (2-4 Hz)** ou só "badge de atividade"; só o nó em foco roda a 120fps. → custo de **CPU de render**. **NOVO — não há story nomeada no épico.** Ver ⚠️ abaixo.
3. **Suspensão de scrollback em RAM → paginação em disco** para nó ocioso há > T. → **RAM do Lina**. É **W5-2** (Onda 5); a Onda 4 só **expõe o estado**, não implementa a paginação.
4. **Estado "💤 dormindo" visível e honesto** (invariante #6 "estado sempre salvo e visível"): o nó mostra *"💤 em pausa, X linhas novas"* em vez de fingir vivo; reativa ao clicar **sem pop-in** (< 1 frame perceptível, critério do Gate C). → é **UX pura da Onda 4**.

**Onde isto THREADA nas stories desta onda:**
- **W4-2** (canvas/5 zonas): o modelo **"foco vs periferia"** — nós-cartão compactos com **badge agregado** ("precisa de você" / "rodando" / "💤") na periferia; 1-4 terminais grandes no foco. **Este é o lar do P0.**
- **W4-3** (pulsos/freio): respeita reduce-motion; o "freio no rodapé" (pausar orquestração) é coerente com o regime de fila.
- **W4-6** (a11y): o estado "💤" e o badge precisam ser **anunciáveis** pelo leitor de tela; reduce-motion corta animação de reativação.

> ⚠️ **[A CONFIRMAR com fundador] — lacuna épico × red-team:** a **peça #2 (throttle 2-4 Hz de nós sem foco)** e o **modelo foco/periferia com badge** são mandato do red-team (§6.3) mas **NÃO são uma story nomeada na Onda 4 do épico** (W4-1…W4-6) nem aparecem no GATE W4. Decisão necessária: **(a)** dobrar como **sub-tarefa P0 de W4-2** (recomendado — é onde o canvas/zonas mora) + critério de aceite adicional; **OU (b)** criar uma story discreta **`W4-7 · Degradação de ociosos (Plano B / Gate C)`**. NÃO inventei a story nº; sinalizo a decisão. O **Gate C** (medir ocioso < 20% do custo de ativo, sem pop-in) é critério objetivo e deveria entrar no GATE W4 se (a) for escolhido.

---

## 1) DAG da Onda 4 + ordem + paralelização

### Mapa de dependências internas (intra-onda)
```
                          ┌─→ W4-3 (sem-fios/pulsos/freio) ─┐
W4-1 (onboarding) ─→ W4-2 ┼─→ W4-4 (salvo✓/T8/T6/T7) ──────┼─→ W4-6 (a11y P0, integrativa)
   (T0→T3 + install)  (T4  └─→ W4-5 (galeria presets) ──────┘
                       hub) 
```
- **W4-2 é o tronco**: tudo depende dele (canvas + scene-graph + nó-terminal na cena).
- **W4-3 · W4-4 · W4-5 são um leque paralelo** — todos dependem **só** de W4-2 e tocam **superfícies disjuntas** do shell (chrome de conexão · chrome de persistência/navegação · galeria de criação). Paralelizáveis se o shell estiver modularizado (ver §5).
- **W4-6 (a11y) é o sorvedouro integrativo**: depende de W4-2+W4-3+W4-4 (precisa das telas e do canvas existirem para instrumentar AccessKit/contraste/reduce-motion). É **L** e atravessa TODAS as superfícies → ver §5 (constrói-se em paralelo mas **integra por último**).
- **Caminho crítico:** `W4-1 → W4-2 → W4-6`. Encurta-se rodando W4-3/W4-4/W4-5 em paralelo logo que W4-2 trava o scene-graph + o contrato de nó na cena.

### Pré-requisitos CROSS-ONDA (deps backward — ler antes de paralelizar; fonte: épico §"Pré-requisitos cross-onda")
| Story W4 | Exige (outra onda) | Estado | Por quê |
|---|---|---|---|
| **W4-1** | **W0-8** CLI Profiles (`lina-cli-profiles`) + **W0-1** PTY (`lina-pty`) | ✅ pronto | "Instalar para mim" = comando de install por SO declarado no Profile, rodado em PTY oculto. |
| **W4-2** | **W3-1** role-discovery (`lina-role-discovery`) + **W2-3** composição de N nós-terminal | ✅ W3-1 / ✅ W2-3 ("todo terminal é shell real") | M2 "Novo Agente" deriva papel do nome (W3-1) e compõe o nó na surface (W2-3). |
| **W4-3** | **W0-4** pub/sub do Workspace Bus (`broker.rs`) + **W3-3** skill A2A | ✅ W0-4 / ✅ W3-3 | O pulso A→B tem de ser **evento real** do pub/sub, não mock; senão a metáfora "sem fios" é falsa. |
| **W4-4** | **W0-6** recuperação pós-crash (core) | ✅ W0-6 (core) | T8 é a **camada de apresentação** de W0-6 — **NÃO reimplementar** a recuperação na UI; só renderizar o `recovering→recovered`. |
| **W4-5** | **W3-1** + **W3-2** bootstrap turno-0 (`lina-bootstrap`) | ✅ | Preset spawna N agentes com papéis ⇒ cada um precisa de role (W3-1) + doutrina de boot (W3-2). |
| **W4-6** | **W2-4** texto/IME via winit + **W2-2** scene-graph | ⚠️ W2-4 parcial (IME/grapheme **[FALTA]**, ver resumo Onda 2) | A11y reusa o caminho IME do winit (W2-4) e a árvore de nós do scene-graph (W2-2). **Risco:** se W2-4 não fechou IME, W4-6 herda a dívida. |

> **Gate W2/W3 ainda abrem furos?** Pelo `_resumo-ondas-2-3.md`: **W2-2 (pan/zoom+culling+hit-test+z-order)** e **W2-4 (IME/grapheme/emoji)** estavam **[EM CURSO]/[FALTA]**. **W4-2 depende de pan/zoom (W2-2)** e **W4-6 depende de IME (W2-4)**. **Confirmar W2-2 e W2-4 fechados ANTES de arrancar W4-2/W4-6** — senão a Onda 4 reabre dívida da Onda 2.

---

## 2) Stories da Onda 4 (densas — viram contrato de fan-out)

### W4-1 · Onboarding T0→T3 + T1 Check-up "Instalar para mim"
- **Objetivo (1 linha):** levar um leigo de boas-vindas (T0) à criação do 1º Espaço (T3) sem jargão, com um check-up (T1) que detecta CLIs no PATH e oferece **"Instalar para mim"** sem abrir terminal.
- **CRITÉRIO DE ACEITE OBSERVÁVEL:** num SO **limpo sem nenhum CLI**, o usuário conclui **T0→T3 só com cliques**; T1 mostra "nenhum CLI encontrado"; "Instalar para mim" instala o CLI escolhido e a **re-detecção passa a exibir o binário com versão** — verificável por `which <cli>` no host pós-fluxo **e** por evento `DiscoveryIndexed` no log. Progresso de onboarding é **retomável** se o app fechar no meio (reabrir cai no passo onde parou).
- **Dependências:** intra: — (arranca já). cross: **W0-8** CLI Profiles · **W0-1** PTY · **W0-5** Event Store (persistir progresso).
- **Reusa do core:** `lina-cli-profiles` (comando de install por SO declarado em TOML) · `lina-pty::PtyManager.spawn` (PTY **oculto** p/ instalar, barra de progresso lida do grid) · `lina-vt` `last_nonempty_line()`/`damage()` (ler progresso sem expor terminal) · `EventStore` (persistir onboarding). **Core add:** evento **`DiscoveryIndexed{ clis: [{id,version,path}] }`** + módulo **`CliDiscovery`** (varre PATH `which`/`where`; epic manda "no core") — pequena adição em `lina-core`.
- **Superfície:** shell `app/lina-gpui` (views T0/T1/T2/T3 = widgets vetoriais Vello + glyph runs `cosmic-text`/`glyphon`, **sem grid de terminal nesta fase**) + `lina-core::CliDiscovery` (novo módulo). **Disjunto** do canvas (W4-2) exceto pela transição T3→T4.

### W4-2 · Canvas T4 hub (5 zonas) + M1 paleta + M2 Novo Agente + P4 Inspetor + M3/M4 criadores  ← TRONCO
- **Objetivo (1 linha):** o canvas (T4) como **"lar"** em 5 zonas, com paleta de comandos (M1), criador de agente papel-pelo-nome (M2), Inspetor de nó (P4) e criadores de nota/pasta (M3/M4) — tudo na GPU sobre o scene-graph retido.
- **CRITÉRIO DE ACEITE OBSERVÁVEL:** abrir T4; via **M2** criar um agente "Revisor" → **nasce um nó-terminal com CLI real spawned**, papel `reviewer` gravado (verificável em `lina list --json` + env `VIBE_ROLE`), aparece no **Inspetor P4**; criar **nota (M3)** e **pasta (M4)** que persistem em `<workspace>/.lina/`; **pan/zoom a 60fps** com os nós na tela. **(+P0 §0 se opção (a)):** nó sem foco vira **cartão compacto com badge** ("rodando"/"💤"/"precisa de você"); fora-de-viewport → `SUSPENSO` (reusa W2-3).
- **Dependências:** intra: **W4-1** (chega-se ao T4 vindo do T3). cross: **W3-1** (papel-pelo-nome) · **W2-2** (scene-graph + pan/zoom) · **W2-3** (composição de N nós-terminal + `SUSPENSO`).
- **Reusa do core:** `spawn_terminal(nodeId, profile, cwd)` (W0-1/W0-8) · eventos `NodeAdded{kind}`/`NodeRoleAssigned`/`NodeRenamed`/`NodeMoved`/`NodeStatusChanged` (**já existem** em `events.rs`) · projeção `ProjectedWorkspace`/`ProjectedNode` (já existe) · `lina-role-discovery::resolve_role` (W3-1) · `lina-vt::damage()` p/ `activity: idle/busy` no P4 · scene-graph + índice espacial + `hit_test` (W2-2). **Core add:** **`NoteCreated`** + **`FolderCreated`** (hoje só há `NoteUpdated`); `CliProfileSet` (P4) se ainda não existir.
- **Superfície:** shell `app/lina-gpui` — **a maior superfície da onda** (layout T4, M1, M2, P4, M3, M4). **Recomenda-se modularizar** o shell em sub-módulos disjuntos (`canvas/`, `palette/`, `inspector/`, `creators/`) já aqui, para liberar o leque paralelo W4-3/4/5 (ver §5).

### W4-3 · Metáfora "sem fios" (aura + selo "Time conectado" + pulsos efêmeros A→B) + freio no rodapé
- **Objetivo (1 linha):** tornar **visível** a invariante "estar no workspace = estar conectado": aura de pertencimento por nó, selo "Time conectado", pulsos efêmeros A→B (ghost wires) ao trafegar A2A — sem ligar cabos; auto-orquestração com **freio** sempre acessível no rodapé.
- **CRITÉRIO DE ACEITE OBSERVÁVEL:** com 2 agentes, um `lina ask A→B` dispara um **pulso visível A→B que some após a entrega**; o **selo "Time conectado"** aparece **sem nenhum arraste de cabo**; ativar o **freio no rodapé** faz uma nova `ask` **ficar enfileirada (não injetada)** até "Retomar" — verificável em `bus.jsonl` (mensagem registrada, entrega adiada) + estado do broker. Pulso respeita **reduce-motion** (W4-6).
- **Dependências:** intra: **W4-2**. cross: **W0-4** pub/sub (`broker.rs`) · **W3-3** skill A2A.
- **Reusa do core:** pub/sub in-process do **broker** (`broker.rs`, eventos `message`/`node.status`) — a UI thread **consome** e anima; **NÃO** desenhar aresta lógica persistente (decisão arq §2.2: só pulsos efêmeros) · `NodeRegistry` para o selo (full-mesh **lógico** por `membership × role`, derivado — sem tabela de edges). **Core add:** ganho de **estado "orquestração pausada"** no broker (enfileira sem injetar) + evento de pausa/retomada — pequeno hook no supervisor (**escritor único**; cuidado de concorrência, ver §5).
- **Superfície:** shell `app/lina-gpui` (chrome de conexão: aura/selo/ghost-wire Vello decorativo; rodapé com freio) + **micro-hook no broker** do `lina-core`. Disjunto de W4-4/W4-5 no shell; **toca o supervisor** → coordenar com quem mexe no core.

### W4-4 · "Tudo salvo ✓" + T8 recuperação visível pós-crash + navegação sem becos + T6 Switcher + T7 Ajustes
- **Objetivo (1 linha):** confiança permanente: indicador **"Tudo salvo ✓"** sempre visível, **T8 recuperação visível** (nunca silenciosa), navegação sem becos sem saída, Switcher de Espaços (T6) e Ajustes (T7).
- **CRITÉRIO DE ACEITE OBSERVÁVEL:** matar com **`kill -9`** durante atividade e reabrir → a **UI de recuperação T8 aparece sempre**, lista nós/terminais restaurados, re-spawna os PTYs e re-hidrata com contexto mínimo; **"Tudo salvo ✓"** visível em idle e alterna para "salvando…" sob escrita (**nunca mente** — ligado ao fsync do event store); **T6** troca de Espaço e **T7** altera um Ajuste que **persiste após reabrir**.
- **Dependências:** intra: **W4-2**. cross: **W0-6** recuperação pós-crash (core) — **camada de apresentação apenas**.
- **Reusa do core:** sinais **`recovering`/`recovered`** do W0-6 (consumidos via `UiHost`/`HostEvent`) · commit/fsync do `EventStore` (W0-5) p/ o indicador "salvo✓" · projeção SQLite p/ listar workspaces (T6) · eventos de Ajustes (T7) event-sourced. **Core add:** **`WorkspaceFocusSet`** (T6 troca de foco) se ainda não existir; expor estado de flush ("salvando/✓") via `HostEvent`. **NÃO reimplementar recuperação** (já é W0-6).
- **Superfície:** shell `app/lina-gpui` (chrome de persistência/navegação: rodapé salvo✓, tela T8, T6, T7). Disjunto de W4-3/W4-5.

### W4-5 · Galeria de Focos (T3) — presets básicos com time pronto + defaults 100% locais
- **Objetivo (1 linha):** galeria de Focos (T3) com presets que ao escolher **já montam o Espaço com o time de agentes e papéis atribuídos**, com **defaults 100% locais** (nada sai da máquina).
- **CRITÉRIO DE ACEITE OBSERVÁVEL:** escolher o preset "App" cria um Espaço com o **time já spawned e papéis atribuídos** (verificável em `lina list --json`: N nós com papéis esperados), `focus_preset='dev_app'` na projeção SQLite; **nenhuma conexão de rede de saída** é aberta na criação (verificável por captura de rede / ausência de túnel ativo) — local-first (inv #2). "Em Branco" cria Espaço vazio (caminho de fuga).
- **Dependências:** intra: **W4-2**. cross: **W3-1** (papéis) · **W3-2** (bootstrap por papel).
- **Reusa do core:** `WorkspaceCreated` (+ campo **`focus_preset`**) · `spawn_terminal` + `NodeRoleAssigned` em lote · `lina-bootstrap` (doutrina de boot por papel, W3-2) · `lina-role-discovery` (W3-1). **Core add:** campo **`focus_preset`** em `WorkspaceCreated`; definição dos presets como dado.
- **Superfície:** shell `app/lina-gpui` (galeria T3 = grid de cards Vello + glyph runs) + dado de preset no `lina-core`. Tamanho **S**. Disjunto de W4-3/W4-4.
- ⚠️ **[A CONFIRMAR com fundador] — taxonomia de presets:** o épico **W4-5** lista **App / Pesquisa & Conteúdo / Em Branco** (`dev_app|research_content|blank`), mas a Arquitetura §9.2 lista `dev_app|landing|automation|design`. **Os nomes divergem.** Confirmar o conjunto canônico de presets do MVP antes de codificar.

### W4-6 · Acessibilidade P0 (AccessKit + temas WCAG + reduce-motion + settings de a11y do SO)  ← SORVEDOURO
- **Objetivo (1 linha):** a11y **prática como P0** (não polimento): `Role::Terminal` + Accessible View linearizado sem ANSI, live-region throttled, temas com contraste **WCAG ≥4.5:1**, reduce-motion e respeito aos settings de a11y do SO — auditado com **leitor de tela real nos 3 SOs**.
- **CRITÉRIO DE ACEITE OBSERVÁVEL:** **auditoria com leitor de tela real nos 3 SOs** (NVDA/Win, VoiceOver/macOS, Orca/Linux) navega **foco/Tab por T0–T8** e **lê o Accessible View** de um terminal anunciando uma **"resposta pronta"**; o **gate de contraste WCAG passa no CI (≥4.5:1)** e **falha** se um tema baixar disso; com **reduce-motion** ligado no SO, os **pulsos A→B param** (W4-3); zoom de texto do SO **escala a UI** — tudo verificável e **gravado** nos 3 SOs.
- **Dependências:** intra: **W4-2 + W4-3 + W4-4** (precisa das telas e do canvas para instrumentar). cross: **W2-4** (IME via winit) · **W2-2** (scene-graph p/ a árvore semântica).
- **Reusa do core:** árvore semântica paralela do **scene-graph** via `accesskit`/`accesskit_winit` · contrato de **fim-de-resposta** (W0-10) p/ disparar "resposta pronta" na live-region (**nunca a cada byte**) · `lina-vt` linearização sem ANSI (tabs→espaços, helper já previsto em W0-2) p/ o `AccessibleBuffer = Vec<String>` · caminho IME do winit (W2-4). **Core add:** mínimo (o grosso é shell/render).
- **Superfície:** **TRANSVERSAL ao shell inteiro** `app/lina-gpui` (todas as telas T0–T8 + zonas do canvas) + gate WCAG no CI. **Por atravessar tudo, integra por último** (ver §5). Tamanho **L**.

---

## 3) Mapa de superfícies — PARALELIZÁVEL vs SEQUENCIAL (para o Maestro distribuir)

| Story | Tam | Superfície primária | Toca o core? | Paraleliza com |
|---|---|---|---|---|
| **W4-1** | M | shell: views T0–T3 (Vello) + `CliDiscovery` (novo módulo core) | sim (módulo NOVO, isolado) | **arranca já** — disjunto do canvas; em paralelo ao fechamento de W2-2/W2-4 |
| **W4-2** | L | shell: canvas T4/M1/M2/P4/M3/M4 (**tronco**) | sim (eventos `Note/Folder`, já há `Node*`) | depois de W4-1; **modulariza o shell aqui** |
| **W4-3** | M | shell: chrome de conexão (aura/selo/pulsos/freio) + **micro-hook no broker** | **sim — supervisor** | W4-4, W4-5 (superfícies de shell disjuntas) ⚠️ coordenar o hook no broker |
| **W4-4** | M | shell: chrome persistência/navegação (salvo✓/T8/T6/T7) | leve (consome `recovering/recovered`, `HostEvent`) | W4-3, W4-5 |
| **W4-5** | S | shell: galeria T3 (cards) + dado de preset | leve (`focus_preset`) | W4-3, W4-4 |
| **W4-6** | L | shell: **transversal** (T0–T8 + canvas) + gate WCAG CI | leve | construída em paralelo, **integra por ÚLTIMO** |

**Regras de paralelização (analogia ao W3: o supervisor é escritor único):**
- **Pré-condição de paralelismo do leque (W4-3/4/5):** o shell `app/lina-gpui` hoje é **`main.rs` + `bridge.rs` num único `src/`** (não modularizado). Rodar 3 workers no mesmo arquivo = conflito de merge garantido. **AÇÃO P0 em W4-2:** modularizar o shell em sub-módulos disjuntos (`onboarding/`, `canvas/`, `wiring/`, `chrome/`, `gallery/`, `a11y/`). Só então W4-3/4/5 paralelizam de verdade (superfícies disjuntas).
- **W4-3 toca o broker (core, escritor único):** a parte do "freio" (enfileira-sem-injetar) muta o supervisor. **Não paralelizar essa mutação** com outra story que toque `broker.rs`/`router.rs`. O resto de W4-3 (aura/selo/pulso = consumo read-only do pub/sub) paraleliza livre.
- **W4-6 atravessa tudo:** pode ser desenvolvida em paralelo (instrumentação AccessKit por superfície), mas a **integração + auditoria de leitor de tela** ocorre **depois** de W4-2/3/4 estabilizarem — é o sorvedouro do gate.
- **Caminho crítico real:** `W4-1 → W4-2(+modularização) → leque(3/4/5) → W4-6`. W4-1 e (instrumentação de) W4-6 podem começar cedo; o gargalo é W4-2.

---

## 4) GATE DE SAÍDA W4 (épico, observável)

> Um leigo cria um Espaço por preset (**T3, W4-5**), vê o **1º agente responder em <15min** com conta no provedor (**onboarding T0→T3 + Instalar-para-mim, W4-1** + **canvas/M2, W4-2**), adiciona o 2º e vê o **pulso efêmero A→B sem ligar cabos** (**W4-3**); **"Tudo salvo ✓"** está sempre visível e a **recuperação T8** aparece após `kill -9` (**W4-4**); **a11y prática** (foco/Tab/contraste ≥4.5:1/zoom/reduce-motion) é **auditada com leitor de tela real nos 3 SOs** (**W4-6**) — tudo observável por `lina list --json`, `bus.jsonl`, gate WCAG no CI e gravações da auditoria.

**Recomendação de adição ao gate (se P0 §0 entrar como (a)):** incluir o **Gate C** do red-team — *com 30 nós abertos mas só 4 ativos, os ociosos custam < 20% do custo de um ativo, sem pop-in (<1 frame) ao reativar*. Hoje o GATE W4 do épico **não** menciona degradação; é a lacuna a fechar com o fundador (§0).

---

## 5) LACUNAS e [A CONFIRMAR com fundador] (consolidado)

1. **[P0 — escopo]** O **throttle 2-4 Hz de nós sem foco** + **modelo foco/periferia com badge** (red-team §6.3) não é story nomeada na Onda 4 do épico. Decidir: sub-tarefa P0 de **W4-2** (recomendado) **ou** nova story **W4-7**; e se o **Gate C** entra no GATE W4. (§0)
2. **[Presets]** Taxonomia de `focus_preset` diverge: épico W4-5 = `dev_app|research_content|blank` vs Arquitetura §9.2 = `dev_app|landing|automation|design`. Definir o conjunto canônico do MVP. (W4-5)
3. **[Dívida da Onda 2]** **W4-2 depende de pan/zoom (W2-2)** e **W4-6 depende de IME/grapheme (W2-4)** — ambos estavam **[EM CURSO]/[FALTA]** no `_resumo-ondas-2-3.md`. Confirmar W2-2 e W2-4 **fechados** antes de arrancar, senão a Onda 4 herda dívida.
4. **[Tempo do 1º agente]** GATE W4 e tabela do épico dizem **"<15min"**; o invariante #6 do CLAUDE.md fala **"1º agente <2h"**. Não conflitam (15min ⊂ 2h), mas confirmar o alvo de gate = **<15min**.
5. **[Core adds pequenos]** Esta onda pede no `lina-core`: `DiscoveryIndexed` + módulo `CliDiscovery` (W4-1); `NoteCreated`/`FolderCreated`/`CliProfileSet` (W4-2); estado "orquestração pausada" no broker (W4-3); `WorkspaceFocusSet` + estado de flush exposto (W4-4); campo `focus_preset` em `WorkspaceCreated` (W4-5). **Todos aditivos** — confirmar que entram como extensão do contrato de eventos (sem fragmentar; upcasting versionado, como `NodeAdded` v1→v2).

---

*Design produzido pelo DEV 02 (decompositor da Onda 4). Mapeado 1:1 ao épico `32 - Epico Fase 0 (MVP)` §Onda 4 (W4-1…W4-6 + GATE W4); o P0 de degradação vem do red-team `12 - Red-team Escala e Benchmark` §6.3 e está sinalizado como lacuna a confirmar. Nenhum escopo inventado além do épico.*
