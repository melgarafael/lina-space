# Entrega D4 — Comandos, menus, paleta e áreas de visibilidade (skills/agents/hooks/MCPs)

> **Pesquisa F2 · dimensão D4** · Dono: Terminal C (FRONTEND, realocada do Core A2A) · Data: 2026-06-12
> **Método:** interna-primeiro (roteiro r5 `tasks/epico-f1/roteiro-tela-r4.md` + ADENDO r5 · ADR 0008 · vault 13.13 · código real `app/lina-gpui/src/palette.rs`/`canvas_focus.rs` · inventário do disco DESTA máquina), depois fan-out externo de 5 frentes paralelas (palette · menus-leigo · ext-managers · atalhos · fs-watch), 56 buscas (8-15 por frente, dentro do piso 5/teto 15), refutação tentada por achado, toda URL citada com fetch real.
> **O que NÃO repeti:** toast/mailbox/badge de permissão (13.13 já cobre — detectar→sinalizar→contextualizar→agir); achados estéticos da D1 (aqui é mecânica, não estética).
> **Fato interno que muda a pergunta:** o Lina JÁ TEM paleta ⌘K (`palette.rs`, W4-2: fuzzy por subsequência, ↑↓/Enter/Esc, comandos com rótulos leigos, `FocusNode` = ir-para-terminal) e JÁ TEM canvas por teclado (`canvas_focus.rs`, F1-2-6: roving tabindex, setas em ordem espacial, Enter desce ao terminal). **Mas a paleta não tem nenhuma porta de entrada visível** (grep por entry point gráfico: vazio) — exatamente o anti-padrão que esta pesquisa documenta.

---

## I. Inventário real do disco — onde skills/agents/hooks/MCPs vivem NESTA máquina (2026-06-12)

| O quê | Caminho | Conteúdo real medido | Implicação para a "área de Poderes" |
|---|---|---|---|
| Skills (Claude) | `~/.claude/skills/` | **75 pastas, 66 `SKILL.md`, 12MB** — scan listou em ~0ms | Scan-ao-abrir é grátis; frontmatter inválido é detectável no scan (estado "precisa de conserto") |
| Plugins (Claude) | `~/.claude/plugins/` | **1,9GB** (repos git + cache); manifesto `installed_plugins.json` = **33 plugins, 13KB**; + `marketplaces/`, `known_marketplaces.json` | **Manifest-first obrigatório**: ler o JSON de 13KB; scan recursivo do 1,9GB proibido por arquitetura |
| Agents (Claude) | `~/.claude/agents/` | 6 arquivos `.md` | Scan raso trivial |
| Commands (Claude) | `~/.claude/commands/` | 9 arquivos `.md` | Scan raso trivial |
| Hooks (Claude) | `~/.claude/settings.json → hooks` (+ pasta `~/.claude/hooks/`) | 1 tipo configurado (`UserPromptSubmit`) | Hooks vivem em config JSON, não em pasta — a área lê o settings |
| MCPs (Claude) | `~/.claude.json` | `mcpServers` global **vazio**; **160 projetos** com config própria dentro do mesmo arquivo; `.mcp.json` por projeto (ausente neste repo) | MCP é por-projeto na prática: a área precisa mostrar "globais + do projeto aberto", não um pool único |
| Codex | `~/.codex/` | `AGENTS.md`, `config.toml` com `[mcp_servers.playwright]` | MCP do Codex vive em TOML próprio — formato por-CLI, como o ADR 0008 previu para perfis |
| Gemini | `~/.gemini/` | `GEMINI.md`, **`skills/` com 13 skills** (lina-* espelhadas + find-skills) | O instalador do Lina JÁ espelha a doutrina por CLI — prova viva do padrão SKILL.md multi-CLI |
| OpenCode | `~/.config/opencode/` | `opencode.jsonc`, `skills/` (2) | idem |
| Copilot | `~/.copilot/` | `skills/` (1) | idem |
| Projeto | `<repo>/.claude/` (settings.json, skills) + `.lina/` (bootstrap.json, events, outbox) | escopo por-projeto existe e é menor | Área mostra global + projeto, com origem rotulada |

**Dois fatos estruturantes:** (1) pastas `skills/` existem em **4 CLIs** — a mesma skill pode estar na pasta de um CLI e faltar na de outro → o estado "instalada-mas-não-funciona-NESTE-motor" não é caso raro, é o caso NORMAL do multi-CLI (inv#3); (2) a fonte da verdade de plugins é um **manifesto pequeno** (13KB), nunca a árvore (1,9GB) — igual ao `extensions.json` do VS Code.

---

## II. Achados (8, decision-grade)

### A1 — Paleta escondida = paleta morta: o "low usage" do GitHub mediu invisibilidade, não valor

- **CLAIM:** O GitHub anunciou a remoção da sua command palette em 2025-07-16 citando "low usage" e recuou em 5 dias após backlash — mas a paleta passou 3,5 anos em beta DESLIGADA por padrão atrás de opt-in; o changelog admite "the specific examples you shared helped us better understand its value beyond what our usage metrics captured". O épico do Grafana (#60852, 2022-12) corrigiu o mesmo problema movendo a paleta do atalho-only para o header visível. E o único padrão ⌘K com tração consumer comprovada é o do Notion — que NÃO é paleta de comandos: é BUSCA com botão "Search" permanente na sidebar, com ações como camada secundária.
- **FONTE:** The Register (https://www.theregister.com/2025/07/22/github_command_palette_backtrack/) · GitHub Changelog · Notion Help (https://www.notion.com/help/search) · HN "Command K Bars" (https://news.ycombinator.com/item?id=46602383)
- **DATA:** 2025-07 · 2026-02 · Notion UNDATED (suspect)
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** Refutada nas duas direções — "paletas fracassam" não: o backlash prova valor intenso para minoria (devs, a11y); "paletas servem a todos" não: até no GitHub (população ~100% técnica) o uso espontâneo foi baixo. "A LOT of users are shockingly bad at typing" (HN 2026-02); paleta pressupõe digitação fluente + vocabulário do produto — leigo não tem nenhum dos dois.
- **SUSPECT:** parcial (doc Notion sem data) · **LOAD-BEARING:** sim — **a paleta ⌘K que o Lina já tem está exatamente no estado pré-morte do GitHub** (atalho-only, sem porta visível). A correção não é removê-la: é dar porta visível (botão rotulado no rail) e virá-la search-first.

### A2 — Mecânica de ranking convergente: previsível vence esperto (alias estrito > MRU estável > fuzzy > frecency por último)

- **CLAIM:** Raycast documenta hierarquia explícita: alias com prefix-match ESTRITO no topo ("Aliases don't use fuzzy matching… always predictable"), depois fuzzy no título, frecency POR ÚLTIMO — e precisa de "Reset Ranking" porque o aprendizado erra. VS Code rejeita deliberadamente ordenar por frequência desde 2017 (v1.14, vivo até hoje): seção MRU persistida no topo sobre lista alfabética ESTÁVEL ("stable and memorable"). O Zed (código-fonte aberto, gpui — o mesmo stack do Lina) faz: hit-count por comando persistido em DB local (contando SÓ invocações via paleta), sort por (hit-count, alfabético), fuzzy `nucleo` smart-case, aliases em settings, e `CommandPaletteFilter` que ESCONDE ações inaplicáveis ao contexto — UMA paleta global filtrada, nunca paletas separadas. Argumentos: só formulário inline de 1 passo (Raycast: máx 3, dropdown estático; VS Code adverte: "Avoid using quick picks for long flows… they aren't well suited to function as a wizard").
- **FONTE:** Raycast Manual (https://manual.raycast.com/search-bar) · VS Code v1.14 (https://code.visualstudio.com/updates/v1_14) · Zed source (https://github.com/zed-industries/zed/blob/main/crates/command_palette/src/command_palette.rs) · Raycast Arguments (https://developers.raycast.com/information/lifecycle/arguments) · VS Code UX Guidelines
- **DATA:** 2026-06 (manual + código) · 2017-06 (mecânica viva) · Arguments UNDATED (suspect)
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** "Frecency é o padrão da indústria" — derrubada: VS Code faz o oposto de propósito; o próprio Raycast embute a válvula de escape. "O matcher resolve tudo" — derrubada: usuários do Zed seguem pedindo tuning (#32638). Procurado contraexemplo de paletas separadas por contexto em produto maduro: NÃO existe — todos convergem em superfície única + filtro.
- **SUSPECT:** parcial · **LOAD-BEARING:** sim — responde a pergunta (a) e dá o blueprint mecânico: o `palette.rs` atual (fuzzy subsequência, sem ranking) ganha alias do glossário no topo, MRU curto, hit-count via event log (projeção — inv#4) e filtro por contexto.

### A3 — Hierarquia de superfícies para leigo: visível-rotulado > contextual redundante > busca com gatilho visível > atalho

- **CLAIM:** (i) 81% de ~69.000 usuários do Firefox Test Pilot nunca usaram Ctrl+F (Mozilla 2011; amostra MAIS técnica que a média); em 2023, ~40% de 519 universitários japoneses desconheciam Ctrl+C e ~70% não sabiam desfazer pelo teclado — a geração mobile-first sabe MENOS atalhos. (ii) Ícone sem rótulo falha: "not a single test participant clicked" (NN/g); praticante testou icon-only "11 vezes em 19 anos, em 5 empresas" com fracasso consistente — e experts localizam por POSIÇÃO, não significado. (iii) Menu contextual nunca pode ser o único caminho (NN/g 2019/2025: "secondary actions only"). (iv) O caso Ribbon: "people weren't finding the very features they asked us to add" — a resposta vencedora foi comandos visíveis, rotulados, agrupados (telemetria de >3bi de sessões); IntelliMenus (esconder adaptativo) fracassou.
- **FONTE:** Mozilla Metrics (https://blog.mozilla.org/metrics/2011/08/25/do-90-of-people-not-use-ctrlf/) · Tom's Hardware/Menter (https://www.tomshardware.com/news/survey-finds-40-percent-of-university-students-in-japan-dont-know-shortcut-keys) · NN/g Icon Usability (https://www.nngroup.com/articles/icon-usability/) + Contextual Menus 2025-11 (https://www.nngroup.com/articles/contextual-menus-guidelines/) · Calabro (https://trevorcalabro.substack.com/p/every-icon-needs-a-label) · Ribbon (https://en.wikipedia.org/wiki/Ribbon_(computing))
- **DATA:** 2011-08 · 2023-12 · 2014-07 · 2025-11 · 2025-01 · 2006-2008
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** Ribbon teve custo real para experts (~20% queda autorrelatada, ExcelUser; espaço vertical) — visibilidade não é grátis; a resolução para canvas denso é toolbar CONTEXTUAL do objeto selecionado (3-5 ações, à la Canva), não ribbon global. "Icon-only é mais rápido para expert" — só depois de aprendida a língua, e POR POSIÇÃO; sobrevive COM labels.
- **SUSPECT:** não · **LOAD-BEARING:** sim — é a régua para decidir onde CADA ação nova do épico F2 entra; e contesta um detalhe do r5 (ações de linha do rail são hover-only — hover é o long-press do desktop; precisa de pista permanente, ex.: kebab visível).

### A4 — Progressive disclosure: funciona com NO MÁXIMO 2 níveis e porta visível rotulada

- **CLAIM:** Doutrina NN/g: mostrar só o importante e oferecer o resto sob demanda melhora learnability e taxa de erro PARA NOVATOS — mas "designs that go beyond 2 disclosure levels typically have low usability" e a falha clássica é a porta para o nível 2 não ser óbvia. O que fica invisível "não existe": leigos pedem como feature o que já está instalado (efeito Ribbon).
- **FONTE:** NN/g Progressive Disclosure (https://www.nngroup.com/articles/progressive-disclosure/)
- **DATA:** 2006-12 (doutrina estável; corroborada pelas guidelines 2025-11)
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** Críticas encontradas confirmam o failure mode (esconder agressivo → usuário assume que não existe) em vez de derrubar. Fronteira validada: esconder só o raramente usado.
- **SUSPECT:** não · **LOAD-BEARING:** sim — formata as áreas de visibilidade: nível 1 = resumo contado e traduzido ("75 Poderes · 33 Plugins · 1 Gatilho"), nível 2 = lista com nome+descrição simples; NADA essencial num 3º nível; e a área nunca enterrada em settings.

### A5 — Estados de extensão: linguagem de consequência + UMA ação de 1 clique; "instalada-mas-inerte-AQUI" tem precedente maduro

- **CLAIM:** O mercado convergiu: estado ruim acoplado a ação nomeada de 1 clique — Chrome "This extension may have been corrupted" + botão **Repair**; Safety check reduz decisão a binário Remove/Keep; VS Code Remote mostra extensão local que não roda no contexto como **"dimmed and disabled"** com botão **"Install Local Extensions in SSH: {host}"** — visível, esmaecida, com a ação que resolve. Anti-padrões documentados: estado sem ação (Chrome "grayed out… won't be able to turn them back on" → beco sem saída); sinal que mente (VS Code "Reload Required" em tudo, bug confirmado #144900); erro sem causa (Obsidian "Failed to load community plugins" → leigos reinstalam o app e mexem em DNS às cegas, threads 2022→2025). Obsidian separa instalar≠ativar e updates NUNCA são automáticos "for security purposes". WCAG 1.4.1: cor nunca sozinha — texto+ícone sempre. E renomear vocabulário sem âncora quebra tutoriais externos (caso "Safe mode"→"Restricted mode").
- **FONTE:** VS Code Extension Marketplace (https://code.visualstudio.com/docs/configure/extensions/extension-marketplace, 2026-02) · VS Code Remote SSH (https://code.visualstudio.com/docs/remote/ssh, 2026-06) · Chrome Safety Hub (https://developer.chrome.com/blog/extension-safety-hub) · UiPath/Chrome Repair (https://docs.uipath.com/studio/standalone/2024.10/user-guide/chrome-extension-corrupted) · Obsidian Forum (https://forum.obsidian.md/t/failed-to-load-community-plugins-not-resolved/43381 · /turn-off-safe-mode/43570) · W3C (https://www.w3.org/TR/UNDERSTANDING-WCAG20/visual-audio-contrast-without-color.html)
- **DATA:** 2026-02/06 · 2023-08 · 2024-10 · 2022-09→2025 · 2023
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** O dimming do VS Code Remote SOZINHO ainda confunde (#1443) — o que funciona é o conjunto: esmaecido + frase do porquê + botão nomeado. Repair às vezes entra em loop (threads 2024-25, não fetchadas — taxa de sucesso real sem fonte). Rename do Obsidian custou pouco (resolvido em horas) — o argumento é manter âncora, não evitar tradução.
- **SUSPECT:** parcial (Raycast debug doc UNDATED, excluída dos 8) · **LOAD-BEARING:** sim — responde a pergunta (b) e dá o blueprint do estado mais importante do Lina (skill na pasta de um CLI, terminal rodando outro — o caso NORMAL do inventário §I).

### A6 — Atalhos: hint passivo converte ≤35% mesmo em lab; conhecer ≠ usar; o pacote mínimo é hint inline + "?" + 1 just-in-time

- **CLAIM:** ExposeHK (CHI 2013): com tooltips clássicos, só 35% das seleções viraram hotkey em LAB com tarefa repetitiva (94% com rehearsal forçado — mecanismo agressivo demais para leigo); hotkey é 2,3x mais rápida QUANDO adotada. Satisficing (IwC 2013): "even when put under time pressure users often fail to select those methods they themselves believe to be fastest" — e hint mostrado APÓS a ação com mouse REFORÇA o mouse. Gmail (bilhões de leigos): atalhos DESLIGADOS por default + overlay "?" sob demanda. Nem a Superhuman confiou em hints: coaching humano 1:1 de 30min (que não escalou — migrou a self-serve em 2025). NN/g 2024-10: accelerators "readily available, yet easy to ignore"; nunca pré-requisito. Anti-padrão documentado (Zed, dev.to 2025-11): função que "only works through a specific keyboard shortcut" é função inexistente.
- **FONTE:** ExposeHK CHI 2013 (https://www.csse.canterbury.ac.nz/andrew.cockburn/papers/ehk.pdf) · Tak et al. (https://academic.oup.com/iwc/article-pdf/25/5/404/2743338/iwt016.pdf) · Gmail (https://support.google.com/mail/answer/6594) · Superhuman (https://blog.superhuman.com/the-fastest-way-to-inbox-zero-a-single-coaching-session/) · NN/g Accelerators (https://www.nngroup.com/articles/ui-accelerators/) · dev.to Zed (https://dev.to/1ce/zed-the-editor-i-wish-i-liked-2jn2)
- **DATA:** 2013-04 · 2013 · Gmail UNDATED (suspect) · 2021-07 · 2024-10 · 2025-11
- **CONFIANÇA:** alta (papers fetchados + convergência independente; follow-up ExposeHK 2025 não fetchável — ACM 403)
- **REFUTAÇÃO:** "Lab infla adoção" — sim, a favor do achado: no mundo real os 35% seriam menores. "Satisficing é imutável" — não: o próprio paper diz que ferramenta ATIVA aumenta uso; mas a ferramenta eficaz é rehearsal/custo, não hint passivo. Dado Ctrl+F é de 2011 (ordem de grandeza, direção hoje é pior — Menter 2023).
- **SUSPECT:** parcial · **LOAD-BEARING:** sim — dimensiona o investimento da F2 em ensino de teclado: atalho inline discreto em toda ação (higiene, ~custo zero), overlay "?" no vocabulário leigo agrupado por tarefa, UM hint just-in-time por hábito repetido (teto 1/sessão, dismiss permanente), campanha de ensino SÓ para ⌘número e ⌘Z. Resto: visível mas sem campanha.

### A7 — Disco: manifest-first + scan-ao-abrir (~0ms) + watcher raso debounced nos manifestos; watch recursivo proibido

- **CLAIM:** Os 3 gerenciadores reais convergem: VS Code é manifest-first (`extensions.json`; issues #28331/#15442 criaram o cache exatamente para não re-ler N package.json) e só ganhou detecção ao vivo de install via CLI em v1.55 (2021-03) após 3,5 anos de fricção reclamada (#38607); Obsidian NÃO tem watcher de plugins (restart para ver — shipou assim para milhões; o plugin `hot-reload` existe por isso, com debounce ~0,75s); Raycast só roda watcher em DEV, fora do app ("There's no server running in Raycast to detect changes"). Crate `notify` (8.2.0) documenta as pegadinhas: atomic saves → rename storms (observar o diretório-PAI do manifesto, não o inode); debounce oficial recomendado; e a assimetria de plataforma — FSEvents (macOS) escala "500GB sem degradação", inotify (Linux) = 1 watch/diretório com default 8192 e estouro ENOSPC que quebra TODAS as ferramentas da máquina.
- **FONTE:** VS Code v1.55 (https://code.visualstudio.com/updates/v1_55) + issue #38607 · Obsidian forum 75609 + pjeby/hot-reload (https://github.com/pjeby/hot-reload) · Raycast (https://www.raycast.com/blog/how-raycast-api-extensions-work) · notify (https://docs.rs/notify/latest/notify/) · fswatch monitors (https://emcrisostomo.github.io/fswatch/doc/1.14.0/fswatch.html/Monitors.html)
- **DATA:** 2021-03 · 2024-01 · 2025-08 · 2023-05 · 2025 · fswatch UNDATED (suspect; fatos de kernel estáveis)
- **CONFIANÇA:** alta (+ medição local: listar 75 pastas de skills = ~0ms)
- **REFUTAÇÃO:** "Só scan-ao-abrir basta?" — Obsidian prova que dá para shipar; mas o fórum mostra leigos confusos ("instalei e não apareceu" = bug, para leigo) e o VS Code investiu engenharia para eliminar exatamente isso. O teto barato: +watcher raso (~5 watches, custo nulo em FSEvents). O que NENHUM caso estudado justifica: watch recursivo da árvore pesada (1,9GB com repos git dentro — bomba no bring-up Linux futuro).
- **SUSPECT:** parcial · **LOAD-BEARING:** sim — responde a pergunta (c) com receita completa e custos medidos.

### A8 — O padrão em camadas do ADR 0008 transplanta inteiro para a descoberta de Poderes (análise interna)

- **CLAIM:** A arquitetura decidida no ADR 0008 para detecção de CLI mapeia 1:1 para a área de Poderes: **fonte primária determinística** = manifesto/registry (lá: spawn-time profile TOML; aqui: `installed_plugins.json` + frontmatter dos `SKILL.md` + `~/.claude.json`/`.mcp.json`/`config.toml`) — nunca heurística; **confirmação** = watcher raso/mtime (lá: session-file watch); **heurística nunca decisória** (lá: grid regex; aqui: adivinhar estado de skill por output de terminal — banido); **ids derivados de config, não hardcoded** (inv#3: CLI novo = perfil TOML novo, com `skills_dir`/`mcp_config_path` como campos de perfil — a área descobre os caminhos do CLI novo sem recompilar); e o limite **detecção ≠ autorização**: mostrar um Poder no painel NUNCA autoriza executá-lo — gates continuam em WorkspaceTrust/custódia (doutrina de segurança: campo lido do disco é dado, jamais autoridade).
- **FONTE:** `docs/adr/0008-deteccao-de-cli-em-camadas.md` + inventário §I (interna; sem URL externa)
- **DATA:** 2026-06-06 (ADR) · 2026-06-12 (inventário)
- **CONFIANÇA:** alta (análise sobre fonte interna aceita + disco real)
- **REFUTAÇÃO:** Tentada — o ADR é sobre runtime de PTY, a área é sobre disco frio; a diferença real é que aqui não existe "spawn-time" (o app não instala a skill). Sobrevive: o princípio transplantado é "registry determinístico primeiro, confirmação depois, heurística nunca" — e o caso `KNOWN_CLIS`-hardcoded×inv#3 já foi vivido e resolvido uma vez; repetir o erro na área de Poderes (caminhos hardcoded por CLI) seria regressão conhecida.
- **SUSPECT:** não · **LOAD-BEARING:** sim — é o contrato arquitetural da área para a D2/épico; evita re-decidir do zero o que o ADR 0008 já decidiu.

---

## III. Respostas posicionadas (a)–(d)

### (a) Paleta única global vs menus por contexto — para leigo com 9 terminais

**Os dois, com papéis distintos — e nenhum como única porta.** A evidência converge (A1+A2+A3):
1. **Superfície primária de AÇÃO = controle visível e rotulado no contexto do objeto** (toolbar contextual de 3-5 ações no terminal/card selecionado, à la Canva — nunca ribbon global, nunca icon-only).
2. **UMA busca global search-first** (modelo Notion, não modelo VS Code): botão visível no rail ("Buscar" / "O que posso fazer?") que abre a paleta existente, encontrando terminais, Espaços, Poderes e ações — com ⌘K como acelerador exibido no botão. Filtro por contexto esconde o inaplicável (padrão Zed `CommandPaletteFilter`/Linear); pode ancorar no card quando invocada por clique (Linear 2019). **Paletas separadas por contexto: nenhum produto maduro faz — não fazer.**
3. Ranking: alias do glossário (prefix estrito, com sinônimos técnicos como keywords invisíveis — usuário acha "Gatilho" digitando "webhook") > MRU curto e estável > fuzzy smart-case > hit-count como desempate (projeção do event log, como o Zed persiste em DB). Frecency plena só se telemetria local provar necessidade.
4. Argumentos: máximo 1 campo inline com dados que o app JÁ tem (lista de terminais). Fluxo com 2+ decisões não é paleta: é conversa com o terminal (inv#1) ou modal próprio.

### (b) Como sinalizar skill instalada-mas-inativa SEM jargão

**Vocabulário proposto (5 estados, ancorados em A5; sempre TEXTO+ícone+cor — WCAG 1.4.1, nunca cor sozinha):**

| Estado | Rótulo leigo | Mecânica | Ação acoplada (obrigatória) |
|---|---|---|---|
| ok | **"Pronto pra usar"** | SKILL.md válido na pasta do CLI do terminal focado | — |
| update | **"Atualização disponível"** | manifesto/marketplace | botão Atualizar (manual — nunca automático; padrão Obsidian, casa com o gate humano) |
| quebrada | **"Precisa de um conserto"** | frontmatter inválido/arquivo faltando, detectado no scan | botão **Consertar** (re-scan/diagnóstico; tradução neutra do Repair do Chrome) |
| inerte-aqui | **"Não funciona neste motor"** | skill na pasta do CLI X, terminal roda CLI Y (o caso NORMAL — §I) | card **esmaecido + frase do porquê** ("está na pasta do Gemini; este terminal usa Claude Code") + ação nomeada — padrão VS Code Remote completo: dimming sozinho confunde (#1443) |
| desligada | **"Desligada"** | SÓ se o Lina realmente puder religar | toggle — **senão o estado não existe: a tela nunca mente** (toggle que o app não enforça é banido) |

Regras transversais: estado sem ação é banido (o "grayed out" terminal do Chrome); severidade com intervenção proporcional (grave = agir+avisar; leve = badge); decisões binárias (Manter/Remover); **tradução com âncora** — "Poder" como rótulo, "(skill)" visível no detalhe, porque o usuário cruza tutoriais externos que dizem "skill" (lição do rename do Obsidian). Strings novas passam pela copy congelada (R9 — funções `copy_*` por módulo, padrão F1-4).

### (c) Padrão de atualização da área (painel gpui lendo o disco)

**Scan-ao-abrir + manifest-first + watcher raso debounced + refresh oportunista** (A7, custos medidos nesta máquina):
1. **Abrir o painel = scan raso** de `~/.claude/skills` (75 pastas ≈ 0ms) e leitura dos manifestos (13KB) — nunca dado velho ao abrir, sem cache para invalidar.
2. **Plugins: SÓ o manifesto** (`installed_plugins.json`) — scan recursivo do 1,9GB proibido por arquitetura (e fatal no Linux futuro: inotify ENOSPC degrada a máquina inteira do usuário).
3. **Watcher raso e debounced (≥750ms, `notify` + debouncer oficial) sobre o diretório-PAI dos ~5 manifestos** (installed_plugins.json, ~/.claude.json, .mcp.json do projeto aberto, config.toml do Codex, skills/ raso) — atomic saves substituem o arquivo, então o watch é no pai, não no inode. Custo no macOS (FSEvents): nulo. Ganho: o efeito mágico "instalei pelo terminal e o Poder apareceu sozinho" — a lição da v1.55 do VS Code (3,5 anos de fricção até entregarem isso).
4. **Refresh oportunista no foco da janela/reabertura do painel** como rede de segurança (eventos coalescidos, NFS, casos que o watcher perde).
5. Caminhos por CLI vêm do **CliProfile TOML** (campos novos `skills_dir`/`mcp_config_path`) — CLI novo entra sem recompilar (A8/inv#3).

### (d) Navegabilidade: o que falta vs roteiro r5

**Já existe (não refazer):** ⌘O alterna rail · ⌘F busca no rail · Esc contextual (terminal vs rail) · ⌘número troca Espaço · ações por linha com undo 8s · paleta ⌘K com `FocusNode` (ir-para-terminal), rótulos leigos e termos de busca · canvas por teclado completo (setas em ordem espacial, Enter desce, zoom_to_focus, reduce-motion — F1-2-6).

**Gaps, em ordem de retorno:**
1. **Porta visível para a paleta** (A1 — o gap nº1): botão rotulado no rail/topbar que abre o ⌘K, com o atalho escrito nele. Hoje ela é o anti-padrão GitHub/Zed.
2. **Busca global search-first** (A1): o ⌘F atual só filtra o rail; a evolução natural é a paleta encontrar terminais+Espaços+Poderes+ações numa caixa só (modelo Notion), tornando-a também a porta das áreas de visibilidade.
3. **Atalho inline visível em toda ação** (A6, higiene NN/g): rail, menus, tooltips — "available yet easy to ignore".
4. **Overlay "?"** com o mapa completo no vocabulário leigo, agrupado por tarefa ("trocar de Espaço", "buscar um Poder") — padrão Gmail, sob demanda, custo cognitivo zero para quem nunca abre.
5. **UM hint just-in-time com teto** (A6): ex. 3ª troca de Espaço via mouse no dia → linha não-modal "dica: ⌘2 vai direto", dismiss permanente, máx 1/sessão. Campanha de ensino SÓ para ⌘número e ⌘Z (teto realista do leigo).
6. **Pista permanente nas ações hover-only do rail** (A3): kebab/ícone discreto sempre visível — hover é o long-press do desktop.
7. **Regra dura (anti-Zed), candidata a invariante de UI no épico:** nenhuma função existe SÓ atrás de atalho ou SÓ dentro da paleta — todo atalho é acelerador de um caminho de mouse visível.

---

## IV. CONFLITOS · LACUNAS · RECÊNCIA

### Conflitos (declarados)

1. **Paleta ⌘K existente (código real, W4-2) × evidência A1:** está atalho-only — o estado pré-morte do GitHub. Não é conflito de remoção, é de evolução (porta visível + search-first). Decisão do épico F2.
2. **Frecency (Raycast) × estabilidade (VS Code):** evidência genuinamente dividida; recomendação calibrada = MRU curto + hit-count desempate; frecency plena só com telemetria local. Red Team pode atacar essa calibragem.
3. **Hover-only no rail (r5, validado na tela) × NN/g (A3):** o r5 shipou ações por hover; a evidência pede pista permanente. Contesta um detalhe de algo já validado pelo fundador — decisão dele.
4. **Watcher "apareceu sozinho" × R5/inv#4 (event log = fonte da verdade):** a área de Poderes lê DISCO (estado externo ao app), não event log. Resolução proposta: o painel é projeção de scan (como `lina list` é projeção de runtime), e mudanças detectadas PODEM virar eventos (`PowerScanned`/aditivo) para auditabilidade — decisão de arquitetura para a D2/épico, não tomada aqui.
5. **Glossário 22 ("Poder") × lição do rename (A5):** não contradiz, refina — tradução SEMPRE com âncora do termo técnico no detalhe. Strings via copy congelada (R9).

### Lacunas

- Não existe pesquisa 2024-26 de command palette com usuários NÃO-técnicos (extrapolação via atalhos/invocação); posts da Command AI (empresa que vendia palette para end-users e pivotou) inacessíveis (HTTP 500 + archive bloqueado) — a tese deles reforçaria o A1, fica NÃO-verificada.
- Follow-up ExposeHK (IHM 2025) atrás de paywall ACM (abstract indica replicação: 26% mais rápido) — reforçaria A6 com data 2025.
- First Round Review 2025-04 (Superhuman human-led→self-serve) em paywall — detalharia os mecanismos in-product que substituíram o coach (ouro para o hint just-in-time).
- Sem telemetria pública: % de usuários que ligam atalhos no Gmail; uso real de paleta por leigos; taxa de sucesso do Repair do Chrome.
- Sem teste de usabilidade publicado para rótulos de estado leigos (a proposta (b) é ancorada em padrões + WCAG, não em teste com o público-alvo → candidata a teste da régua D0).
- Implementação exata da detecção v1.55 do VS Code não inspecionada no código; comportamento do `notify` sob atomic save do `~/.claude.json` não reproduzido localmente (mitigação padrão da doc: watch no pai).
- Windows: ReadDirectoryChangesW só em doc (buffer overflow sob rajadas) — teste empírico quando o bring-up Windows abrir essa frente.

### Recência

- **Núcleo 2023-2026:** GitHub palette 2025-07 · VS Code docs 2026-02/06 + Zed source 2026-06 (fetch direto do main) · NN/g 2024-10 e 2025-11 · Menter 2023-12 · notify 2025 · Raycast manual 2026-06 · hot-reload release 2025-08 · Calabro 2025-01.
- **Âncoras antigas mantidas com justificativa:** ExposeHK CHI 2013 + Satisficing 2013 (fundacionais de HCI; nenhum estudo pós-2020 os contradiz — KeyMap CHI 2020 parte do mesmo diagnóstico); Ctrl+F 2011 (ordem de grandeza; direção hoje é pior — Menter 2023); Ribbon 2006-08 (caso canônico, telemetria >3bi sessões); VS Code v1.14/2017 e v1.55/2021 (mecânicas vivas, verificadas na doc 2026); Linear changelog 2019-10 (padrão ainda em produção); NN/g progressive disclosure 2006 (re-validada pelas guidelines 2025).
- **UNDATED → suspect (por regra):** Raycast Arguments · Notion Search help · Gmail shortcuts help · fswatch monitors · Obsidian help (corroborada por fóruns datados).

---

## V. Apêndice — frentes do fan-out (para a verificação V do Red Team)

| Frente | Buscas | Achados (LB/suspect) | Fontes-chave |
|---|---|---|---|
| palette | 9 | 8 (8 LB / 2 s) | theregister 2025-07 · raycast manual · vscode v1.14 · zed source (main) · HN 46602383 · notion help · linear changelog 2019 |
| menus-leigo | 12 | 8 (6 LB / 0 s) | NN/g (4 artigos: progressive disclosure, icons, contextual 2019+2025) · mozilla 2011 · tomshardware 2023 · ribbon/wikipedia · calabro 2025 · HN 29373536 |
| ext-managers | 8 | 8 (7 LB / 1 s) | vscode marketplace 2026-02 + remote ssh 2026-06 · chrome safety hub 2023 · uipath 2024-10 · obsidian forum (2 threads) · W3C WCAG 1.4.1 |
| atalhos | 12 | 7 (7 LB / 1 s) | ExposeHK CHI 2013 (PDF) · Tak IwC 2013 (PDF) · gmail help · superhuman blog 2021 · NN/g 2024-10 · dev.to zed 2025-11 |
| fs-watch | 15 | 7 (7 LB / 1 s) | vscode v1.55 + #38607 · obsidian forum 75609 + pjeby/hot-reload · raycast blog 2023-05 · docs.rs/notify · fswatch · medições locais |

Total: 56 buscas · 38 achados brutos → 8 consolidados. Dado bruto integral (JSON por frente) disponível comigo (Terminal C) sob demanda para a onda V.

---

PRONTO: 8 achados (datados/refutados/fetch real) + inventário real do disco (skills em 4 CLIs; manifesto 13KB vs árvore 1,9GB) + respostas (a) busca global search-first com porta visível + toolbar contextual, (b) 5 estados leigos com ação obrigatória e âncora do termo técnico, (c) scan-ao-abrir + manifest-first + watcher raso debounced, (d) 7 gaps de navegabilidade priorizados sobre o que o r5 já tem — com a regra dura anti-Zed como candidata a invariante de UI.
