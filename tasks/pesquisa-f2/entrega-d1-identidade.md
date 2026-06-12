# Entrega D1 — Identidade visual e referências (anti-genérico)

> **Pesquisa F2 · dimensão D1** · Dono: Terminal C (FRONTEND) · Data: 2026-06-12
> **Método:** interna-primeiro (13.3 · 22 · 10 · ADR 0019 §7 · 13.4 §3-5 · doutrina `assets/lina-doctrine/`), depois fan-out externo de 9 frentes paralelas (zed · linear · warp-ghostty · arc-dia-figma · raycast · instrumentos · opendesign · tipografia · dark-first), 92 buscas totais (6-14 por frente — dentro do piso 5/teto 15), refutação tentada por achado, toda URL citada foi aberta de verdade (fetch real). Award-sites (Dribbble/Behance/Awwwards) excluídos por doutrina do plano.
> **O que NÃO repeti (já coberto na 13.3):** mecânica de temas do Zed (200+ tokens, ColorScaleSet, theme_overrides, right-click inspector — assunto da D2), notificações do Warp, padrões de interação de canvas tldraw/Figma/Flowith (assunto da D3).
> **Âncora interna inegociável:** ADR 0019 §7 já sela JetBrains Mono para terminal/código e exige tipografia display com personalidade **via tokens** — esta entrega não re-litiga o mono; a decisão em aberto é a voz tipográfica da UI/display e a identidade como um todo.

---

## I. Achados (8, decision-grade)

### A1 — O "Linear look" é o novo genérico — e o próprio Linear o abandonou

- **CLAIM:** A fórmula dark + gradiente desfocado + glow que todo SaaS copia ("The Linear effect", catalogada já em 2023-01 com squint test mostrando 4 sites indistinguíveis) virou o template da era — acelerada por shadcn + ferramentas de IA (Lovable/Bolt/V0, debate Ask HN 2025-12). O próprio Linear saiu dela: o redesign 2025 do site cortou gradientes e quase toda a cor (monocromo). Adotar o "Linear look" em 2026 é adotar um default morto duas vezes: pela doutrina anti-slop do Lina e pelo abandono do originador.
- **FONTE:** Rectangle/Daryl Ginn (https://rectangle.substack.com/p/the-linear-effect) · Ask HN (https://news.ycombinator.com/item?id=46179202) · LogRocket (https://blog.logrocket.com/ux-design/linear-design/)
- **DATA:** 2023-01 · 2025-12 · 2025-06
- **CONFIANÇA:** alta (o detalhe do redesign 2025 do Linear: média — veio de artigo atualizado + buscas, não de fonte primária do Linear)
- **REFUTAÇÃO:** Tentada e registrada — a defesa da conformidade existe ("procurement folks get jittery"; padrão sinaliza profissionalismo), mas vale para B2B enterprise com comitê de compra, não para um app desktop escolhido por um indivíduo leigo. O contraexemplo é o próprio Linear: nasceu distinto e venceu por isso. O risco real documentado não é "ser diferente" — é ser diferente com execução fraca.
- **SUSPECT:** não · **LOAD-BEARING:** sim — é a prova externa de que a F2 não pode trocar um default genérico por outro.

### A2 — "Premium" é latência, não estilo: a velocidade é a estética — e é perecível

- **CLAIM:** O que usuários percebem como "premium" no Zed e no Linear não é o visual: é resposta instantânea como propriedade arquitetural (Zed: frames <3ms moldam cada decisão — "we are forced to do things a certain way because of performance"; Linear: sync engine local-first, zero spinners, ⌘K consultando dados locais). E a marca cobra juros: quando a velocidade regrediu, a revolta foi imediata e pública nos dois (HN 2025-02 "Zed Has Suddenly Become Terrible"; HN 2026-06 "I've lost enthusiasm... Linear has become the thing it sought to kill"). O Lina tem essa vantagem por construção (Rust nativo, event log local) — mas ela precisa de gate de qualidade permanente, não de sprint.
- **FONTE:** Zed blog (https://zed.dev/blog/we-have-to-start-over 路 https://zed.dev/blog/between-editors-and-ides) · Bytemash deep-dive do sync engine (https://bytemash.net/posts/i-went-down-the-linear-rabbit-hole/) · HN (https://news.ycombinator.com/item?id=43041923 · https://hn.algolia.com/api/v1/items/48437609)
- **DATA:** 2024-02/03 · 2025-08 · 2025-02 · 2026-06
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** "Tracker novo só parece rápido porque você largou o entulho do antigo" (HN 2020) — parte da percepção é frescor. E updates otimistas compram velocidade com risco de "silent data loss" e estado divergente entre quem vê (críticas reais ao Linear, 2026-06) — limite que o event log único do Lina não tem, mas que proíbe "consistência otimista sem reconciliação visível" num canvas multi-agente.
- **SUSPECT:** não · **LOAD-BEARING:** sim — define a relação identidade↔R7: o 120Hz é orçamento de latência e nitidez, não de espetáculo. Corolário verificado: texto nítido em monitor barato é parte do contrato (Zed levou ~2 anos para subpixel em LoDPI — issue #7992, P1 frequency:common; fix só em v0.219.4, 2026-01); o Lina renderiza texto próprio (cosmic-text/swash) e deve testar em 1080p/1440p desde a F2.

### A3 — O que intimida o leigo no terminal é a PASSIVIDADE, não o visual dark/monospace — que virou signo mainstream de craft

- **CLAIM:** A barreira documentada do terminal para não-técnicos é psicológica: o prompt passivo ("só fica ali esperando você digitar"), a ausência de affordances e o medo de quebrar algo — não a estética dark/mono (Teresa Torres, público product/não-técnico, 2025-10: "parece coisa de engenheiro", mas atravessa com 3-4 comandos guiados). Em paralelo, o console-look virou capital estético mainstream 2025-26 ("Technical Mono": monospace como signo de autenticidade/controle/craft, "não é nostalgia, é rebelião"). E o caso Warp fecha o argumento por contraste: o ódio histórico ao Warp foi sobre AGÊNCIA (login obrigatório, telemetria, closed-source), nunca sobre estética — e quem monetizou foram devs querendo agentes, não leigos querendo terminal amigável.
- **FONTE:** producttalk.org (https://www.producttalk.org/claude-code-what-it-is-and-how-its-different/) · Medium/Seris Miralis (https://medium.com/@phazeline/the-terminal-aesthetic-and-the-return-of-texture-to-the-web-ed37ee8183bd) · HN Warp (https://hn.algolia.com/api/v1/items/42247583) · Warp blog (https://www.warp.dev/blog/warp-is-now-open-source)
- **DATA:** 2025-10 · 2025-10 · 2024-11 · 2026-04
- **CONFIANÇA:** média (direcionalmente consistente entre 3+ fontes independentes; sem estudo quantitativo — ver LACUNAS)
- **REFUTAÇÃO:** Tentada — "leigo não quer terminal" (céticos no HN 2024-11) pode ter expirado: em 2025-26 nasceu uma razão nova para leigos entrarem (agentes que agem; Torres documenta não-técnicos adotando Claude Code). O "Technical Mono" é ensaio de tendência sem dados e mede designers usando o código visual do terminal, não leigos achando-o bonito. Saldo: manter a cara de terminal é defensável; assumir que ela ENCANTA o leigo é hipótese a medir (D0).
- **SUSPECT:** não · **LOAD-BEARING:** sim — decide a pergunta central da frente: **mantém-se a cara de terminal; mata-se a passividade.** Regra de render derivada: nenhum terminal parado sem estado legível — o nó sempre narra o que o agente faz/fez/o que dá pra fazer (alinha com inv#6 "nunca tela em branco"). E o anti-Warp: "nada sai da sua máquina" sinalizado na superfície é capital de confiança grátis que o Lina já tem.

### A4 — Calor na SUPERFÍCIE, nunca na ESTRUTURA: o "novelty tax" matou o Arc; presença nomeada fez o Figma vencer

- **CLAIM:** A carta oficial de Josh Miller (2025-05) atribui o fim do Arc ao "novelty tax" — "too many things to learn, for too little reward": as features que o time amava eram usadas por 0,4%-5,5% dos usuários ativos. O Dia nasceu simples — e a simplicidade pura TAMBÉM falhou ("Chrome com ChatGPT acoplado"), forçando o pêndulo de 2025-11: re-importar os greatest hits sob a fórmula "simples por default, poder progressivo". Já o Figma venceu levando leigos para dentro da ferramenta técnica com zero atrito de entrada + calor em microcamadas (cursores nomeados — "não é funcional, mas te faz sorrir"; stamps; skeuomorfismo lúdico do FigJam) sobre interações que todo mundo já conhece. Contrapeso documentado: presença ao vivo é amada quando intencional/simétrica e vira ansiedade quando involuntária/assimétrica (fórum Figma 2021-2023, "alguém olhando por sobre seu ombro"; hide-cursors que não persiste).
- **FONTE:** Carta oficial (https://browsercompany.substack.com/p/letter-to-arc-members-2025) · TechCrunch (https://techcrunch.com/2025/11/03/dias-ai-browser-starts-adding-arcs-greatest-hits-to-its-feature-set/) · Inverse/time de design do Arc (https://www.inverse.com/input/design/the-browser-company-arc-design-interview) · Kwokchain (https://kwokchain.com/2020/06/19/why-figma-wins/) · Fórum Figma (https://forum.figma.com/ask-the-community-7/multiplayer-mode-and-emotional-triggers-22233)
- **DATA:** 2025-05 · 2025-11 · 2022-06 · 2020-06 · 2021→2023
- **CONFIANÇA:** alta (métricas de uso do Arc são dados internos da empresa, não contestados)
- **REFUTAÇÃO:** Tentada — o colapso do Arc teve componente de modelo de negócio que a carta minimiza (sem search deal, RAM, conta obrigatória); o luto dos power users não mediu adoção (o que eles amavam era exatamente o que <6% usava). A tese sobrevive refinada: personalidade na moldura emocional e na presença dos agentes; mecânica de navegação familiar.
- **SUSPECT:** não · **LOAD-BEARING:** sim — para o Lina: (a) terminais com nome, papel e atividade narrada SÃO os "cursores nomeados" — e aqui a vigilância vira deleite sem custo humano (quem trabalha são agentes); (b) cada conceito novo do canvas precisa se pagar no primeiro uso; (c) instrumentar uso de feature desde o dia 1 (o event log já permite — inv#4) para nunca descobrir "os 5,5%" tarde demais; (d) o canvas não pode cobrar um segundo imposto de novidade por cima do terminal — toda interação espacial precisa de análogo conhecido.

### A5 — A estética de instrumento é MECANISMO, não figurino: cor semântica acoplada, todo nó é um viewer vivo, flat honesto

- **CLAIM:** As três encarnações da estética de instrumento compartilham um princípio-raiz — honestidade brutal sobre o que a coisa É, com cuidado obsessivo com como aparece: **Ableton** = flat radical como expressão da função ("a slider is just a line"; design do synth Drift rejeitado por "não representar como ele realmente funciona"; "o maior overhaul é o que ninguém nota"), layout adjacente-no-espaço em vez de menus; **Teenage Engineering** = paleta restrita (RAL) onde cor é código semântico acoplado ao controle ("laranja/vermelho significa gravação"; "elemento verde na tela = knob verde muda o valor"), monospace que "sinaliza engenharia sem mensagem explícita"; **TouchDesigner** = densidade operável porque todo nó é um viewer vivo — o dado flui visível, nada é caixa-preta. A fronteira dura: restrição que corta o básico destrói confiança (OP-1 Field a US$1.999 sem undo: "completely lost all respect"; a TE recuou via firmware).
- **FONTE:** Eric Carl, Principal Designer Ableton (https://ericcarl.link/blog/ableton-live-and-designing-for-authenticity/) · CDM/Live 10 (https://cdm.link/ableton-live-10-depth-hands-impressions-whats-new/) · SFMOMA/Kouthoofd (https://www.sfmoma.org/read/stay-curious-stay-naive-an-interview-with-teenage-engineering-jesper-kouthoofd/) · Engadget+Synthtopia (https://www.engadget.com/teenage-engineering-op1-tx6-expensive-fun-181029889.html) · Medium/Huot sobre TouchDesigner (https://medium.com/@antoinehuot1995/touchdesigner-but-for-graphics-programmer-ee8bf2f005da)
- **DATA:** 2026-03 · 2017-11 · 2024-02 · 2022-09 · 2024-04
- **CONFIANÇA:** alta
- **REFUTAÇÃO:** Tentada por frente — backlash anti-flat do Ableton existe (minoria: "too washed out"; survey 2017: ~40% acham a Session View "cluttered" e pedem controle de densidade, não glamour); críticos da TE chamam de "Fabergé eggs", mas atacam preço/features, não o sistema de cor; TouchDesigner é "esmagador" na primeira tela — o viewer vivo transplanta, a densidade default não. Risco nomeado sem fonte (inferência minha): pastiche TE como skin (laranja + parafusos sem o mecanismo) é exatamente o slop que a doutrina bane.
- **SUSPECT:** não · **LOAD-BEARING:** sim — é o mecanismo que faz um leigo ler o estado de um sistema complexo num relance: paleta restrita com UM significado fixo por cor em todo o app + a cor do indicador = a cor do elemento acionável + thumbnail vivo do trabalho em cada terminal mesmo em zoom-out (é PARA isso que o budget 120Hz existe — e conversa com o achado interno 13.4: engajamento vem de progresso visível, não de adjetivos). E o limite sagrado: nenhuma pureza estética corta desfazer/estado-salvo (inv#6).

### A6 — Dark OLED imposto não se sustenta: a evidência pede "terminais sempre-dark como ilha de identidade + chrome que segue o sistema"

- **CLAIM:** (a) Para visão normal, polaridade positiva (light) vence em legibilidade na maioria das condições e a vantagem cresce conforme a fonte diminui (NN/g); (b) a preferência real é ~⅓ dark / ⅓ light / ⅓ misto, decidida no nível do SO — e "fãs de dark mal notam quando um design não tem dark mode" (NN/g 2023, n=115); (c) o "82% preferem dark" viral rastreia para polls tech auto-selecionadas de 2019-2020 (Android Authority: leitores de site Android), sem telemetria 2024-26 de população geral; (d) halation atinge parte do ~⅓ de adultos com astigmatismo (nunca #FFF sobre #000); (e) o padrão consumer de produtividade é light default ou seguir-o-sistema (Notion, ChatGPT desktop, Canva; dark-first deliberado é nicho dev — Linear declara mirar "engineers"); (f) PORÉM a demanda emocional por dark é real e intensa em canvas tools (feature mais votada da história do Miro: 2.392 votos, churn declarado) — como OPÇÃO, nunca como imposição; (g) <5% dos leigos mudam settings (UIE) — o default É o produto; "seguir o sistema" herda a única escolha de tema que o leigo já fez; (h) o precedente visual consagrado de dark-sobre-claro é o code block dark em página clara (Stripe docs, Docusaurus) — e o terminal do Lina é exatamente essa âncora.
- **FONTE:** NN/g (https://www.nngroup.com/articles/dark-mode/ · https://www.nngroup.com/articles/dark-mode-users-issues/) · Android Authority (https://www.androidauthority.com/dark-mode-poll-results-1090716/) · Stéphanie Walter (https://stephaniewalter.design/blog/dark-mode-accessibility-myth-debunked/) · Miro Community (https://community.miro.com/ideas/dark-mode-night-mode-874/index11.html) · UIE (https://archive.uie.com/brainsparks/2011/09/14/do-users-change-their-settings/) · VS Code docs (https://code.visualstudio.com/docs/terminal/appearance)
- **DATA:** 2020-02 · 2023-08 · 2020-03 · 2024-05 · 2025-03 · 2011-09 · UNDATED (doc viva)
- **CONFIANÇA:** alta no agregado (peças individuais: média — ver SUSPECT)
- **REFUTAÇÃO:** Tentada nos dois sentidos — astigmáticos que PREFEREM dark existem (Ask HN ~2025-11: variância individual grande); quem vota dark no Miro é early adopter, não a base leiga; o achado (e) mistura docs oficiais e guias de terceiros. A refutação reposiciona, não derruba: refuta "leigo não liga para dark" E refuta "dark deve ser imposto".
- **SUSPECT:** parcial (defaults de Notion/Canva/ChatGPT vieram de guias de terceiros, não doc primária; doc do VS Code é viva/sem data; UIE é 2011 — refinado pelo NN/g 2023) · **LOAD-BEARING:** sim — **contesta explicitamente o default "Dark OLED já aplicado" do fluxo 22 (T0, 2026-05-29)**, conforme a regra A5 do plano. Proposta decision-grade: terminais SEMPRE dark (ilha de identidade, JetBrains Mono selada); chrome do canvas segue o sistema com dark E light de 1ª classe; toggle de 1 clique visível no 1º uso; no modo light, canvas off-white/quente com elevação mediando as ilhas dark (sem precedente shipped — validar em protótipo, item para D0/"testar opções na tela").

### A7 — Tipografia: o stack decide metade (sem variable fonts no gpui/cosmic-text → instâncias estáticas); humanista vence geométrica fria para leitura de relance; rota OFL = risco zero

- **CLAIM:** (a) gpui NÃO aplica font-variation-settings (zed#4850 aberta desde 2023-04; verificado state=open via API em 2026-06-12) e cosmic-text ignora eixos variáveis (#406 aberta desde 2025-07; o PR #371 de 2025-03, contribuído pela Canva, só trouxe kerning/ligaturas) → a fonte display/UI do Lina precisa ter **pesos estáticos completos**; (b) para leitura de relance — exatamente o modo do canvas (narrações, cards, rail) — formas humanistas abertas venceram square grotesque em ~8,8-10% no tempo de exposição (MIT AgeLab/Monotype; replicado em Dobres et al. 2014); (c) licenças: OFL com caráter = custo e risco zero (Atkinson Hyperlegible Next, 2025-02, 7 pesos + estáticos + mono, Braille Institute; IBM Plex; Fraunces; Geist v1.7.2 ativa em 2026-06 — mas é o "novo Inter" com identidade da Vercel: só como decisão declarada); foundries comerciais cobrem app desktop via App licence (Klim/Söhne/Untitled Sans: perpétua, por MAU/downloads, referência US$630/5k MAU em 2021; Dinamo/Diatype: por headcount) mas preços 2025-26 estão atrás de configuradores JS — cotar por e-mail; Fontshare/FFL (General Sans, Satoshi, Clash Display) é zona cinzenta de redistribuição para fontes embutidas em binário — NÃO resolvida (texto primário da FFL inacessível ao fetch).
- **FONTE:** GitHub (https://github.com/zed-industries/zed/issues/4850 · https://github.com/pop-os/cosmic-text/issues/406) · MIT News (https://news.mit.edu/2012/agelab-automobile-dashboard-fonts-1005) · Klim (https://klim.co.nz/blog/klim-type-foundry-end-user-licence-agreements/) · Dinamo (https://abcdinamo.com/news/about-our-pricing) · Google Fonts (https://fonts.google.com/specimen/Atkinson+Hyperlegible+Next) · Vercel (https://github.com/vercel/geist-font) · Fontshare via secundárias (https://madegooddesigns.com/fontshare/)
- **DATA:** 2026-06 (issues verificadas) · 2012-10 · 2019/2021 (preços) · 2025-02 · 2026-03
- **CONFIANÇA:** alta nas restrições técnicas (verificação direta via API hoje); média nos preços (2019/2021, podem estar defasados) e no estudo MIT (patrocinado pela Monotype; contexto automotivo — transfere como direção, não como número)
- **REFUTAÇÃO:** Tentada — o fechamento de cosmic-text#229 como "completed" parecia indicar eixos variáveis: derrubado lendo o PR (#371 só passou features ao shaper). Atkinson "não é objetivamente superior a Arial" para visão normal (HN) — o argumento é caráter + acessibilidade de borda, não superpoder. Ressalva ao ADR 0019 §7: o cookbook da Anthropic cita Clash Display como display-com-personalidade — Clash é Fontshare/FFL, a zona cinzenta acima; se algum dia virar fonte EMBUTIDA, a licença precisa ser resolvida primeiro.
- **SUSPECT:** parcial (apenas o sub-claim FFL) · **LOAD-BEARING:** sim — restringe TODAS as candidatas da D2 (estáticos completos obrigatórios) e dá a direção com evidência: humanista com calor para a superfície leiga.

### A8 — Consistência de terceiros é ARQUITETURAL (catálogo fechado), e motion tem uma regra de ouro: nunca animar ação iniciada por input frequente

- **CLAIM:** O Raycast mantém milhares de extensões de terceiros com UMA cara por arquitetura, não por guideline: a extensão declara conteúdo num catálogo fechado de componentes (List/Grid/Detail/Form/ActionPanel) e o app renderiza tudo com a identidade dele — terceiro nunca desenha pixel livre — somado a review com regras mecânicas verificáveis ("ícone default = rejeição"; Title Case; EmptyView obrigatório anti-flicker). E a doutrina de motion observada em produção: a janela abre/fecha SEM animação; "Never animate keyboard initiated actions... repeated hundreds of times a day, an animation would make them feel slow"; animações restantes <300ms ease-out; delight confinado a momentos raros/opt-in (confetti, easter eggs) — nunca no caminho do trabalho. Prioridade declarada: "fast, simple, delightful" NESSA ordem. Limite documentado: o pacote keyboard-first atravessou até "anybody in tech" (CEO, 2024-09), não até leigos — zero evidência de adoção fora de tech.
- **FONTE:** Raycast docs/guidelines (https://developers.raycast.com/basics/prepare-an-extension-for-store, atualizado 2026-04) · Raycast blog (https://www.raycast.com/blog/a-fresh-look-and-feel · https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast) · Emil Kowalski (https://emilkowal.ski/ui/great-animations) · TechCrunch (https://techcrunch.com/2024/09/25/raycast-raises-30m-to-bring-its-mac-productivity-app-to-windows-and-ios/)
- **DATA:** 2026-04 · 2022-07 · 2026-05 · 2024-12 · 2024-09
- **CONFIANÇA:** alta (a "regra de motion" é observação de praticante de alta reputação + comportamento verificável do app — não doc oficial do Raycast; lacuna registrada)
- **REFUTAÇÃO:** Tentada — a reescrita 2.0 (2026-05, AppKit→WebView) poderia ter matado o "fast": sobreviveu como prioridade (hacks anti-throttling do WebKit), com custo admitido de RAM. O custo do catálogo fechado: expressividade limitada do terceiro + gargalo humano de review (~1 semana). Crítica estética ao Raycast é rara; o ressentimento mira assinatura/VC/IA-push — lição paralela: aparência nunca deve ser paywall.
- **SUSPECT:** não · **LOAD-BEARING:** sim — é o único mecanismo comprovado em escala para o problema que a F2 herda nas áreas de skills/agents/MCPs (D4): conteúdo de terceiro renderizado pelos tokens do tema, nunca pixel livre. E a regra de motion é a operacionalização exata do R7: 120Hz gasto em latência e suavidade espacial (pan/zoom, spawn), jamais em cerimônia de input. Keyboard-first fica como camada 2 — a superfície primária do leigo é espacial e mouse-first.

---

## II. Territórios estéticos para o Lina (4, nomeados)

> Critério comum: todos respeitam R1 (gpui, estáticos), R4 (não-técnico-first), R6 (a11y/halation), R7 (motion = latência, não espetáculo), R8 (anti-slop) e a âncora ADR 0019 §7 (JetBrains Mono no terminal; tudo via tokens — D2). Nenhum usa o vocabulário banido (Inter por inércia, gradiente roxo, glassmorphism, Linear-look). O fundador escolhe; a D2 nasce apontada para o escolhido.

### T1 — "Instrumento de Estúdio" ⭐ (a que eu defendo)

**Statement:** o Lina é um instrumento que o leigo aprende a tocar — não um painel que ele administra. Honestidade Ableton (flat radical, função é a forma), gramática TE (cor = significado fixo, acoplada ao controle), vida TouchDesigner (todo terminal é um viewer vivo). O canvas é a mesa do estúdio; os terminais são as máquinas trabalhando à vista.

- **Tipografia:** UI = **IBM Plex Sans** (OFL, estáticos completos, humanista-técnica nascida como anti-Helvetica da IBM — "corporativa com alma", suporta pt-br impecável). Display de marca (telas de momento: onboarding, conclusões) = **Fraunces** (OFL, soft-serif com caráter, instâncias estáticas pré-geradas). Alternativa de UI na mesma tese: **Atkinson Hyperlegible Next** (OFL, 2025-02, desambiguação radical 0/O-I/l — variante "acessibilidade como identidade", conversa com R6). Mono: JetBrains Mono (selada). Razão anti-inércia: Plex e Fraunces carregam herança declarável (engenharia + editorial humano) — nenhuma é o default estatístico de IA.
- **Cor/contraste:** paleta funcional restrita (~5 cores semânticas estilo RAL: ex. âmbar = trabalhando, verde = pronto, vermelho = precisa de você, azul = mensagem do time) com UM significado fixo em todo o app; a cor do indicador É a cor do elemento acionável (acoplamento OP-1). **Terminais sempre dark** (off-white sobre dark elevado, nunca #FFF/#000 — A6); **chrome segue o sistema** (dark "carvão quente" e light "papel quente" de 1ª classe). Zero gradiente decorativo.
- **Densidade:** média e GOVERNÁVEL — "adjacente no espaço" (mostrar lado a lado > esconder em menu), com graduação de detalhe por zoom (o Detail Level que 40% dos usuários do Ableton pediram). Nunca a densidade default do TouchDesigner (esmaga o leigo).
- **Motion:** anima só o que significa — estado mudou (cor do nó transiciona), trabalho fluindo (thumbnail vivo do terminal mesmo em zoom-out — é nisso que o 120Hz é gasto), pan/zoom espacial. NUNCA anima: input do usuário (abrir/focar terminal = 0ms), chrome, hover decorativo. Reduce-motion respeitado (R6). 1-2 momentos raros de celebração (missão concluída) — confinados, fora do hot path.
- **Por que cabe no leigo-empreendedor:** ele lê o estado do time num relance SEM ler texto (cor semântica + viewer vivo) — e "ver o trabalho acontecendo" é o que gera engajamento (13.4: progresso > adjetivos). Instrumento = desejo + sensação de maestria sem frieza dev-tool nem cara de brinquedo.
- **Por que eu a defendo:** (1) é o único território que NENHUM concorrente ocupa — Maestri é SwiftUI-macOS padrão, BridgeMind é grid dev, Warp é IDE-moderno (nota 10); (2) é o único onde o budget 120Hz compra VALOR para o usuário (viewers vivos) em vez de decoração; (3) operacionaliza a evidência mais forte da pesquisa (A5, confiança alta) e a doutrina interna já vigente (flat honesto, anti-slop); (4) é coerente com o nome — o violino de Einstein: instrumento, orquestra, estúdio; (5) sobrevive a dark E light (as ilhas-terminal são a identidade constante).
- **Risco nomeado:** pastiche (laranja + monospace + "parafusos" como skin sem o mecanismo). Antídoto: transplantar mecanismo (cor=significado, mostrar o trabalho), nunca figurino; e o limite sagrado — nenhuma restrição estética corta desfazer/estado-salvo (lição OP-1, A5).

### T2 — "Oficina de Precisão"

**Statement:** a ferramenta que desaparece — chrome monocromático que recua, trabalho como protagonista, 1 acento só. Linhagem Zed/Ghostty/Linear-método (o método, jamais o look).

- **Tipografia:** UI = **Untitled Sans** (Klim, "deliberadamente plain" com craft — App licence por MAU, cotar) ou **ABC Diatype** (Dinamo, por headcount); rota OFL: Geist — APENAS como decisão declarada (é o quase-default dev-tool, A7). Sem display: a tipografia não performa.
- **Cor/contraste:** monocromo + 1 acento funcional; dark-leaning com light de 1ª classe (A6 vale aqui também). Nada de glow/gradiente.
- **Densidade:** média-alta, estável (sem graduação elaborada).
- **Motion:** quase zero — só transição de estado, <200ms. O mais barato em frametime dos quatro (R7 folgado).
- **Por que cabe (e onde rasga):** sinaliza "ferramenta séria" e é o mais barato de executar com perfeição — mas é a resposta de OUTRA pergunta: o minimalismo do Zed pressupõe alfabetização dev (palette+atalhos+JSON — o próprio time admitiu em 2025-12 que isso mata discoverability; A2/A8), e zero evidência de não-técnicos nesse pacote. Para o leigo, restraint sem narração vira porta trancada. Só viável com a camada de narração leiga por cima — o que já o empurra na direção do T1.

### T3 — "Ateliê Caloroso"

**Statement:** a sala importa ("the room matters, too" — Arc): o canvas como ateliê acolhedor onde um time de ajudantes trabalha à vista. Calor na moldura e na presença; mecânica 100% familiar.

- **Tipografia:** UI = **Atkinson Hyperlegible Next** (calor humanista + missão de acessibilidade); display = **Bricolage Grotesque** ou **Fraunces** (ambas OFL, personalidade editorial quente — licenças a confirmar na D2 por dever de fetch). Mono: JetBrains Mono (selada).
- **Cor/contraste:** **light-first quente** (papel/off-white, jamais branco puro) com terminais como ilhas dark ancoradas — o precedente "code block dark em página clara" (A6h); cor de identidade por Espaço (a "sala" de cada time — a tese superficial do Arc que sobreviveu, A4); dark mode de 1ª classe.
- **Densidade:** baixa, progressiva — "uma decisão por tela" estendido ao canvas; poder de entusiasta escondido atrás do caminho simples (fórmula Dia, A4).
- **Motion:** micro-momentos de celebração (stamps/confete na conclusão — FigJam) + spawn suave; nunca em input nem permanente (festa constante vira ansiedade — A4/A8). Reduce-motion.
- **Por que cabe no leigo-empreendedor:** é o de menor intimidação — o Figma provou que calor em microcamadas leva leigos para dentro de ferramenta técnica (A4). **Risco:** virar brinquedo aos olhos de quem quer "instrumento de negócio"; e a mistura dark-sobre-claro não tem precedente shipped em ferramenta de trabalho (A6 — exige protótipo). É o melhor segundo lugar; pares bem com T1 (um T1 com a temperatura do T3 é uma fusão legítima).

### T4 — "Sala de Controle" (mapeada por honestidade; eu NÃO a recomendo como default da F2)

**Statement:** cockpit de telemetria — densidade TouchDesigner, glow de instrumentação, observabilidade total à vista.

- **Tipografia:** IBM Plex Sans + Plex Mono em texturas pesadas; acento Space Grotesk (OFL — porém em vias de virar clichê de branding IA: usar só com decisão).
- **Cor/contraste:** dark profundo, acentos saturados de telemetria.
- **Densidade:** alta por default. **Motion:** fluxos de dado contínuos (caro em frametime — tensão direta com R7).
- **Veredito:** é a primeira tela "esmagadora" do TouchDesigner (crítica nº1 mesmo entre quem ama — A5) aplicada a quem NÃO aceitou aprender uma gramática. Contradiz R4 frontalmente. Valor real: como inspiração para superfícies PRO de observabilidade (F3+, projeções do event log), nunca como identidade default.

---

## III. Nota OpenDesign (pedido do fundador: recomendá-lo no onboarding)

**O que é (verificado 2026-06-12):** open-design.ai / repo `nexu-io/open-design` — clone open-source (Apache-2.0) do Claude Design da Anthropic (lançado 2026-04-17), nascido 11 dias depois (2026-04-28). Real e MUITO vivo: 63,8k stars, 369 contribuidores, release 0.10.0 em 2026-06-11, site com pt-br. App desktop (Win/Mac/Linux) que detecta CLIs no PATH (Claude Code, Codex, Gemini CLI…) e os usa como "motor de design" num loop Detect → Discover → **Direct** (escolha entre ~5 direções visuais nomeadas antes de gerar) → Deliver (preview sandboxed; export HTML/PDF/PPTX/MP4), com 155-259 skills SKILL.md + ~150 design systems DESIGN.md. Grátis/BYOK; monetiza via gateway hosted (AMR, desde 2026-06-02).

**Recepção independente (hype-filter aplicado):** HN ~2026-05 (232 pts/92 comentários) — críticas dominam: custo de tokens, output com fama de genérico ("o novo template Microsoft keynote"), tom de vendedor no README, suspeita de stars (fork-ratio NÃO confirma farm — inconclusivo); ZERO relatos de produção. XDA (2026-05-25): não serve a leigos na via BYOK; a via fácil (AMR) tem dias de vida e é hosted pago. Autoria sem entidade jurídica identificável.

**Recomendação (contesta a forma, preserva a intenção do fundador):**
1. **Importar o FORMATO, não o app:** DESIGN.md/SKILL.md são Markdown Apache-2.0 que o Claude Code dos terminais do Lina consome direto — embarcar uma curadoria local vendorizada/versionada no onboarding da F2 (zero dependência externa, 100% local-first).
2. **Transplantar o passo "Direct":** oferecer ao leigo 3-5 direções visuais NOMEADAS e determinísticas antes de gerar qualquer artefato — é a doutrina "escolha UMA direção e banque-a" operacionalizada num picker (custa zero GPU). Os territórios do §II podem alimentar exatamente esse picker.
3. **Se houver link:** SOMENTE https://open-design.ai (o `opendesigner.io` é fan site não-oficial — risco real de o leigo cair no site errado), rotulado como recurso externo opt-in, nunca bloqueando o fluxo.
4. **NÃO fazer:** embedar/depender do app (é outro workspace de agentes — confundiria o leigo sobre "qual app eu uso"; 6 semanas de vida; governança incerta) nem sugerir os design systems de marcas alheias (Apple/Stripe/Linear) como direção — é mimetismo de marca com risco de trademark que o leigo não entende, e o disclaimer de responsabilidade só foi verificado no fan site (suspect).

---

## IV. CONFLITOS · LACUNAS · RECÊNCIA

### Conflitos (declarados, não resolvidos por mim)

1. **A6 × fluxo 22 (T0):** a evidência contesta o "tema Dark OLED já aplicado" como default do onboarding (decisão interna de 2026-05-29). Proposta: terminais sempre-dark + chrome segue o sistema + light de 1ª classe. Decisão é do fundador no épico F2; se mantiver Dark OLED imposto, que seja como escolha consciente contra a evidência registrada aqui.
2. **A3 (manter cara de terminal) × A3-refutação (encanta ou só não-assusta?):** sem estudo quantitativo de reação de leigos à estética de terminal — as duas pontas são anedóticas. Converge em "manter o look, matar a passividade", mas a magnitude do fascínio é hipótese → instrumentar na D0.
3. **ADR 0019 §7 (cookbook cita Clash Display) × A7 (FFL zona cinzenta):** se uma fonte Fontshare algum dia virar fonte EMBUTIDA no app, a licença precisa ser resolvida ANTES (texto primário da FFL segue não-lido — fetch falhou).
4. **OpenDesign: intenção do fundador (recomendar no onboarding) × recepção independente (genérico, hosted pago para leigo, governança incerta):** resolvido na forma proposta no §III (formato + picker "Direct" + link opt-in), que preserva a intenção sem criar dependência.
5. **T1×T3:** não são excludentes — um "Instrumento de Estúdio" com a temperatura do "Ateliê" (light quente como opção de chrome, celebração contida) é fusão legítima; a escolha real do fundador é sobre o CENTRO de gravidade.

### Lacunas (o que esta pesquisa NÃO conseguiu responder)

- Nenhum estudo quantitativo 2024-26 sobre reação de não-técnicos à estética de terminal (só praticantes/ensaios; paper "Training Wheels for the Command Line" apareceu em busca e não foi aberto).
- Preços 2025-26 exatos das App licences Klim/Dinamo (configuradores JS) — cotar via sales@klim.co.nz / licensing@abcdinamo.com antes de qualquer decisão por fonte comercial.
- Texto primário da ITF Free Font License (Fontshare) — zona cinzenta de embedding não resolvida.
- Telemetria de população geral não-técnica sobre dark vs light (2024-26) — não existe pública; tudo recicla polls tech 2019-20.
- "Terminais dark sobre canvas claro" em ferramenta de trabalho shipped — sem precedente fetchável; precisa de protótipo próprio (etapa "testar opções na tela" da F2).
- OpenDesign: zero relato de produção por leigos; qualidade do pt-br interno não verificada; entidade jurídica não identificada.
- Filosofia de motion do Linear em fonte primária (inferida do produto/terceiros — Emil Kowalski ex-Linear é a melhor pista).
- Vozes cruas de Reddit/fóruns Ableton/Gearspace (403/429 recorrentes) — entraram de segunda mão via HN/agregadores, com peso reduzido.

### Recência

- **Núcleo 2024-2026** (domínio rápido ok): Zed settings 2025-12 + subpixel 2026-01; Linear redesign/críticas 2025-2026; Warp open-source 2026-04; carta Arc 2025-05 + pêndulo Dia 2025-11; Raycast 2.0 2026-05; Eric Carl/Ableton 2026-03; OpenDesign 2026-04→06 (verificado HOJE via API); issues gpui/cosmic-text verificadas 2026-06-12; Atkinson Next 2025-02.
- **Âncoras antigas mantidas com justificativa:** MIT AgeLab 2012 (replicado peer-reviewed 2014; única evidência empírica de glance legibility); Kwokchain 2020 (explicação canônica citada até hoje); NN/g 2020/2023 (metodologia controlada > polls); UIE 2011 (refinado pelo NN/g 2023: leigo decide tema no SO); Klim/Dinamo 2019-2021 (estrutura de licença viva; números defasados — flagueado); Inverse/Arc 2022 (formulação primária da tese, anterior à janela — flagueada no achado).
- **UNDATED → suspect (por regra):** doc viva do VS Code; review FigJam (Adam Fard); ensaio Blake Crosley sobre TE; Linear Method (página viva).

---

## V. Apêndice — frentes do fan-out (para a verificação V do Red Team)

| Frente | Buscas | Achados (LB/suspect) | Fontes-chave |
|---|---|---|---|
| zed | 10 | 6 (5 LB / 0 s) | zed.dev/blog (3 posts) · GH #7992 · HN 43041923 · dev.to |
| linear | 8 | 7 (7 LB / 1 s) | linear.app/now + /method · rectangle.substack · HN 46179202 + 48437609 · bytemash · LogRocket |
| warp-ghostty | 11 | 8 (6 LB / 0 s) | warp.dev/blog (2) · HN 42247583 + 42517447 + 46141082 · producttalk · Medium "Technical Mono" |
| arc-dia-figma | 10 | 8 (5 LB / 1 s) | browsercompany.substack · TechCrunch 2025-11 · Inverse · kwokchain · fórum Figma (2 threads) · dev.to |
| raycast | 10 | 6 (6 LB / 0 s) | raycast.com/blog (3) · developers.raycast.com · emilkowal.ski · TechCrunch 2024-09 · HN 46024754 |
| instrumentos | 14 | 8 (5 LB / 1 s) | ericcarl.link · cdm.link · SFMOMA · Engadget/Synthtopia · Medium/Huot · stevezafeiriou · nenadmilosevic · blakecrosley |
| opendesign | 6 | 7 (7 LB / 1 s) | open-design.ai (+pt-br) · GitHub API nexu-io · HN 47985750 · XDA · TechTimes · opendesigner.io (fan site) |
| tipografia | 11 | 8 (5 LB / 1 s) | GH zed#4850 + cosmic-text#406/#371 (API 2026-06-12) · klim.co.nz · abcdinamo.com · MIT News · Google Fonts · vercel/geist |
| dark-first | 12 | 8 (8 LB / 2 s) | NN/g (2) · androidauthority · stephaniewalter.design · Miro Community · UIE archive · code.visualstudio.com · Figma blog |

Total: 92 buscas · 66 achados brutos → 8 consolidados acima. Dado bruto integral (JSON por frente) disponível comigo (Terminal C) sob demanda para a onda V.

---

PRONTO: 8 achados consolidados (todos datados, refutados, fetch real) + 4 territórios nomeados com defesa do T1 "Instrumento de Estúdio" + dossiê OpenDesign (importar formato/picker "Direct", não o app; link só open-design.ai) + conflito declarado com o default Dark OLED do fluxo 22.
