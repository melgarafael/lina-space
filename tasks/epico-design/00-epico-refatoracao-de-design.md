# Épico — Refatoração de Design — Lina Space

> **"O Instrumento ganha sua geometria"** — evolução selada da fusão T1+T3.
> Backlog vivo da reforma de UI do shell `app/lina-gpui`. Cada story tem **critério de aceite OBSERVÁVEL** (roda e se mede — nunca "implementado"). Fios condutores: governança (ADR-gate antes de fechar porta), observabilidade (gate WCAG/token_ratchet no CI), gates de qualidade (cold-review + gate de tela do fundador). **Escopo: shell apenas — nenhuma decisão toca `lina-core`/`lina-host`.**

---

## §0 — DECISÕES DO FUNDADOR (seladas 2026-06-25)

> Fonte da verdade desta onda — governam toda story abaixo (precedência sobre qualquer texto herdado).

1. **Geometria — chrome reto + conteúdo 4px.** Molduras/painéis/barras a `radius.none` (0px); cartões/controles internos a `radius.sm` (4px). Linguagem de **dois degraus** (PostHog/Linear), **não** 0px universal. Fixa o ADR-DESIGN-C.
2. **Identidade — evoluir o "Ateliê", não redirecionar.** Mantém as cores de terra quente (argila/papel/carvão) seladas na F2-0-D §VIII.1; ganha geometria reta + disciplina de UM acento por tela. Calor na cor e na palavra; rigor na forma. O vocabulário regente é **"Instrumento de Estúdio com a temperatura do Ateliê"** — *"oficina"* era o território **T2 rejeitado**; não usar.
3. **Execução aprovada.** O fundador autorizou disparar os 3 ADRs (shell de colunas, cor de workspace, geometria) ao Arquiteto Webhook [ultracode] **antes** de qualquer código; os ADRs voltam ao fundador antes da obra começar.

---

## §I — NORTE (diagnóstico + direção declarada)

**Diagnóstico (provado no código, não inferido).** A queixa do fundador — *"arcaico / Windows antigo / tudo modal / stand colorido de slop"* — procede, mas a causa **NÃO é o design system**: é a camada de aplicação. O vocabulário em `theme.rs` é a melhor parte e não é slop — superfícies em terra quente (carvão `0x1a1510`, papel `0xf4ede0`), três famílias tipográficas deliberadas (foge de Inter/Roboto), `ColorScale` de fonte única dark/light, gate WCAG no CI. **Preservar.** O "arcaico" vem de três camadas de aplicação, todas evoluíveis sem reescrever o tema: **(a) arquitetura** — o root é UM canvas `relative().size_full()` (`main.rs:5002-5011`) sobre o qual tudo flutua `.absolute()`; a Área de Poderes é overlay no canto, não as abas que o fundador pediu; 5 modais reais sobre `ui/modal.rs` (véu 0.5 + card + armadilha de teclado) — anti-padrão que a literatura condena (NN/g: modal só para o irreversível). **(b) geometria** — 118 dos 136 cantos são `.rounded_md()`, helper fixo de 6px **invisível ao token_ratchet**; o token `radius.none=0` já existe (`theme.rs:462,472`) e o próprio comentário admite que o flat *"entra na F2-2"* e nunca chegou (`theme.rs:458`). **(c) cor + emoji** — o "stand colorido" são DOIS CTAs co-iguais que chamam a MESMA função (`➕ Novo Terminal` `main.rs:6022` + `✦ Novo Agente` `:6038` → ambos `open_agent_modal_create` `:6020/:6036`) mais emoji-como-ícone em todo o chrome (⚡➕✦🏠⚙️🔔📝📁, `sidebar.rs:788,809`).

**Direção declarada (opinião, anti-slop).** **Não redireciono — evoluo.** A Lina é **um instrumento de precisão com a temperatura de um ateliê**: a *temperatura* vive na cor das superfícies (argila, papel, carvão tostado) e na voz humana de cada label; a *precisão* vive na **geometria reta**, na **estrutura por hairline** (linha de 1px, nunca sombra difusa), na **disciplina de UM acento por tela** e na **arquitetura de painéis** onde nada bloqueia e nada se perde. Calor vem da cor e da palavra; rigor vem da forma — as duas coisas ao mesmo tempo é o que nenhuma IA cospe por default. **Banido explicitamente** (o anti-slop é parte da identidade): gradiente decorativo/mesh, glassmorphism, neon, roxo-de-IA, **emoji-como-ícone**, sombra difusa como elevação, **dois primários competindo**, "moderno e limpo" sem decisão. Teste-âncora (PostHog *Brand in practice*): **"tire o logo — ainda parece Lina?"** Tem que parecer. **Evolução, não recriação** — recriar do zero colidiria com a direção selada (vault §VIII.1) e com 3 gates (WCAG, token_ratchet, UiHost).

---

## §II — INVARIANTES PRESERVADOS (critério IMPLÍCITO de TODA story)

Toda story abaixo carrega estes cinco como critério de aceite **implícito** — quebrar qualquer um reprova a entrega, mesmo que o efeito visual esteja bom:

1. **UiHost (core/shell split)** — a reforma é 100% camada gpui. O core Rust **não importa** tipo de toolkit; nenhuma story cruza para `lina-core`/`lina-host`. (A única story com persistência — D3-3, cor de workspace — guarda a escolha como evento/projeção no core via contrato existente, **sem** o core conhecer layout/cor de UI.)
2. **token_ratchet** — a catraca anti-número-mágico **não pode subir** a contagem de literais `px`/`FontWeight`. Geometria migra para `radius.none`/`radius.sm` via token (`.rounded(px(f32::from(t.radius.none)))`), **NUNCA** `px(0.)` literal. Arquivo novo nasce em **zero dívida**.
3. **Gate WCAG no CI** — contraste AA não regride em nenhum modo × acento; focus ring ≥ 3:1; ícone/alvo ≥ 24×24. Pares neutros novos do chrome **precisam ser reprovados** no gate (risco sinalizado, não suposto).
4. **Local-first** — vetores de ícone vendorizados (sem rede); escolha de cor/tema é JSON local; nada sai da máquina.
5. **Não-técnico-first** — zero jargão na superfície; estado sempre salvo e visível; nunca tela em branco. Abas e seções **salvam sozinhas** (cada uma é contexto auto-contido — ressalva Baymard); erro vai **inline**, ao lado do campo, nunca em modal.

---

## §III — ADR-GATES (onde a reforma FECHA UMA PORTA — ADR aceito ANTES de codar)

Regra de processo (CLAUDE.md): antes de qualquer decisão que re-arquiteta interação ou promove um conceito a sistema, **pare e registre um ADR curto em `docs/adr/`**. As stories marcadas não iniciam sem o ADR **aceito**.

> **Escritos e aceitos (2026-06-25):** ADR-DESIGN-A = **[0053](../../docs/adr/0053-design-shell-de-colunas-persistente.md)** (shell de colunas) · ADR-DESIGN-B = **[0054](../../docs/adr/0054-design-cor-de-acento-identidade-de-workspace.md)** (cor de workspace + recalibração 9→8) · ADR-DESIGN-C = **[0055](../../docs/adr/0055-design-geometria-reta-tokenizada.md)** (geometria reta tokenizada). **Pendente: aval do fundador antes de iniciar a obra.**

| ADR (título) | Por que fecha porta | Story que BLOQUEIA |
|---|---|---|
| **ADR-DESIGN-A — Shell de colunas persistente: overlays flutuantes viram layout dedicado** | Converter topbar/footer/painéis de `.absolute()` sobre um canvas (`main.rs:5002-5011`) num shell de colunas re-arquiteta a fronteira de layout que várias telas assumem (palette como nav primária, posição dos overlays). Não fecha porta de continuidade (PortalEngine/UiHost intactos), mas muda o contrato de layout de todo o shell. | **D2-1** (e, por dependência, D2-2…D2-6, D4-1) |
| **ADR-DESIGN-B — Cor de acento como identidade de workspace (modelo Arc)** | Promover os `ACCENTS` (`theme.rs:182`) de "decoração de chrome" a **token de identidade do Espaço** (1 acento por workspace, escolhido e persistido) muda a semântica da cor e exige um contrato de persistência (evento/projeção) + a separação estrita "acento-de-marca nunca vira status". Decide também a recalibração 9→8 (remover `violeta`). | **D3-3**, **D1-5** |
| **ADR-DESIGN-C — Geometria reta como decisão de sistema (radius.none no chrome, radius.sm no conteúdo)** | Comprometer-se com UMA linguagem de raio (chrome 0px, conteúdo 4px) e migrar 118 sites de `.rounded_md()` para tokens é uma decisão de sistema que retira `radius.md/lg` da aplicação e define onde cada raio vale — afeta toda tela futura. Fixa também a regra "tokenizado, nunca `px(0.)`". | **D1-1** |

---

## §IV — STORIES (por frente — cobrem as 8 do fundador)

### Frente A — FUNDAÇÃO: design system de identidade + geometria  *(fundador #6, #8)*

**D1-1 — Migração de geometria para token (chrome reto / conteúdo 4px)**
- **Por que (fonte):** `theme.rs:457-477` — `radius.none=0` ("flat honesto — T1") já existe e o comentário admite que a aplicação *"entra na F2-2"* e nunca chegou; 118 `.rounded_md()` fixos de 6px invisíveis à catraca (grep verificado). Externa: PostHog *vibe-designing* (o slop é misturar raios sem intenção — comprometa-se com UMA linguagem; chrome 0 / conteúdo 4-6), acesso 2026-06-25.
- **Aceite observável:** `grep -rc 'rounded_md\|rounded_lg' app/lina-gpui/src` retorna **0** nos arquivos de chrome; chrome usa `radius.none`, conteúdo tocável usa `radius.sm`; **nenhum** `px(0.)` literal (verificado por grep); `cargo test -p lina-gpui` (token_ratchet) **verde** e a contagem de literais px **não sobe**.
- **Dono:** DEV (Terminal G [ultracode] — migração mecânica de 118 sites). **ADR-gate:** ADR-DESIGN-C.

**D1-2 — Elevação por hairline (substituir sombra difusa por borda 1px)**
- **Por que (fonte):** `theme.rs` — `surface.border` (`0x3a3128`/`0xcbb99c`) já existe. Externa: Linear *design refresh* (removeu elevação por sombra; "structure should be felt not seen") + PostHog (sombra difusa = maior vetor de "cara de SaaS genérico"), acesso 2026-06-25.
- **Aceite observável:** `grep -r 'shadow' app/lina-gpui/src` em superfícies de chrome/painel retorna **0**; separação visível entre colunas/painéis é hairline de 1px via `surface.border`; gate WCAG verde (borda ≥ 3:1 onde é divisor estrutural).
- **Dono:** DEV (Terminal M [ultracode]). **ADR-gate:** —

**D1-3 — Sistema de ícones vetoriais (emoji → traço único 1.5px monocromático)**
- **Por que (fonte):** `sidebar.rs:788,809` (🔍/▣/●), `main.rs:6022,6038` (➕/✦) — emoji-como-ícone em todo o chrome. Externa: PostHog *visual-identity* (restrição cromática) + a regra "ícone herda text_color", acesso 2026-06-25.
- **Aceite observável:** `grep -rP '[\x{1F000}-\x{1FAFF}\x{2600}-\x{27BF}\x{2B00}-\x{2BFF}]' app/lina-gpui/src` retorna **0** em sites de render (não-comentário); ícones vetoriais vendorizados (assets locais, sem rede) herdam `text_color`; alvo ≥ 24×24 e contraste ≥ 3:1 no gate WCAG.
- **Dono:** FRONTEND (Especialista em Telas) + DEV (Terminal I). **ADR-gate:** —

**D1-4 — Disciplina tipográfica (Fraunces fora do chrome; sentence-case; instâncias estáticas)**
- **Por que (fonte):** `theme.rs:351,408-413` — 3 famílias são curadoria deliberada (IBM Plex Sans UI / Fraunces display / JetBrains Mono grid). Externa: PostHog handbook (*"don't use the display font in product UI"*) + restrição F2-1 (sem variable fonts no gpui), acesso 2026-06-25.
- **Aceite observável:** nenhuma label de chrome de produto usa Fraunces (auditoria por grep do helper de fonte display nos render de UI = 0 fora de onboarding/conclusão); headings em sentence-case; `cargo build -p lina-gpui` usa apenas instâncias estáticas (sem eixo variável); gate WCAG e token_ratchet verdes.
- **Dono:** DEV (Terminal K). **ADR-gate:** —

**D1-5 — Recalibração cromática (remover `violeta`: 9→8 acentos; reconciliar header)**
- **Por que (fonte):** `theme.rs:182-185` — `ACCENTS` tem **9** entradas e `violeta`=`0xbb9af7` é o roxo-de-IA que a própria doutrina bane nas cores de ação (`theme.rs:242`), enquanto o header diz "8 acentos curados" (`theme.rs:10,172`) — contradição real. Externa: PostHog *visual-identity* ("quanto mais cor, menos cada cor significa"), acesso 2026-06-25.
- **Aceite observável:** `ACCENTS.len() == 8` (teste); `violeta`/`0xbb9af7` ausente do array; header do módulo e doc batem com 8; teste prova que **acento-de-marca nunca é resolvido como status** (`success`/`warning`/`danger` intactos); gate WCAG verde para os 8 × 2 modos (superfície reduzida = mais seguro).
- **Dono:** BACKEND (Terminal B Ultra Code — toca tokens/contrato). **ADR-gate:** ADR-DESIGN-B.

### Frente B — ARQUITETURA DE INTERFACE: modal → colunas / abas  *(fundador #7)*

**D2-1 — Shell de colunas persistente (rail · canvas · coluna utilitária colapsável · topbar/footer flat)**
- **Por que (fonte):** `main.rs:5002-5011` (root = 1 canvas `relative().size_full()`); enxame de `.absolute()` (`main.rs:1394,2393,2528,2575,2734,2822,2944,3181,5291,5736,5867,6179`); `ui/modal.rs:286-318` (véu + armadilha de teclado). Externa: NN/g *Modal & Nonmodal* (modal desabilita o conteúdo — só para destrutivo), acesso 2026-06-25.
- **Aceite observável:** o app sobe com layout de colunas (rail esquerdo + canvas central + coluna utilitária direita colapsável + topbar/footer flat) onde topbar/footer **não** são mais `.absolute()` sobre o canvas; a coluna utilitária faz toggle sem bloquear o canvas (canvas permanece interativo, verificado por interação); o slot PortalEngine/ExternalTextureLayer permanece na cena (não simplificado — grep do tipo presente); `cargo test -p lina-gpui` verde.
- **Dono:** ARQUITETO (Arquiteto Webhook [ultracode] — re-arquitetura pesada). **ADR-gate:** ADR-DESIGN-A.

**D2-2 — Área de Poderes como COLUNA com ABAS (piloto: Skills · Plugins · Canais · MCP · Automações)**
- **Por que (fonte):** `powers_panel.rs` (hoje overlay `div().absolute().top_16().right_0()`, ~42% largura, toggle); ADR 0052 (scan determinístico — contrato Core↔UI já selado, mostrar≠autorizar). Externa: Baymard *accordion-and-tab* (cada aba é contexto auto-contido que salva sozinho — nunca um form fatiado), acesso 2026-06-25.
- **Aceite observável:** Poderes abre como **coluna persistente com abas**; trocar de aba **não** perde estado e **salva sozinho** (verificado: edita aba A, vai pra B, volta — estado preservado); erro de uma aba aparece **inline** ao lado do campo, não em modal; o limite mostrar≠autorizar do ADR 0052 continua verde por mutação.
- **Dono:** FRONTEND (Especialista em Telas). **ADR-gate:** ADR-DESIGN-A (depende de D2-1).

**D2-3 — Ajustes em coluna dedicada com seções (Credenciais inline, não modal)**
- **Por que (fonte):** `credential_modal.rs` (hoje modal sobre `ui/modal.rs`). Externa: Raycast Eng (Settings em janela/painel nativo, nunca modal volátil), acesso 2026-06-25.
- **Aceite observável:** Ajustes abre como coluna com seções (Credenciais, Automações…); `credential_modal` não é mais invocado como diálogo bloqueante (grep da chamada do modal = 0 no fluxo de ajustes); credenciais editáveis inline; **nenhum segredo logado** (varredura do log no fluxo de credencial = 0 hits do valor); estado salvo e visível.
- **Dono:** DEV (Terminal I). **ADR-gate:** ADR-DESIGN-A (depende de D2-1).

**D2-4 — Fluxos de criação como inspetor lateral não-bloqueante (terminal/agente, novo Espaço)**
- **Por que (fonte):** `agent_modal.rs`; `open_agent_modal_create` (`main.rs:3351`, chamado em `:6020/:6036`); `CreateSpaceModal`. Externa: Userpilot *modal UX* ("never use more than one modal consecutively"), acesso 2026-06-25.
- **Aceite observável:** criar terminal/agente abre como **inspetor na coluna direita** com o canvas ainda interativo (verificado por interação durante a criação); nenhum empilhamento de modais; estado parcial preservado se a coluna colapsa e reabre.
- **Dono:** DEV (Terminal K). **ADR-gate:** ADR-DESIGN-A (depende de D2-1).

**D2-5 — `ui/modal.rs` retido SÓ para confirmação destrutiva/irreversível (1 pergunta sim/não)**
- **Por que (fonte):** `ui/modal.rs:286-318` (véu escurecido + card + armadilha de teclado); casa com o gate humano que a doutrina já exige (CLAUDE.md §6). Externa: NN/g (modal justificado só para ação destrutiva/irreversível), acesso 2026-06-25.
- **Aceite observável:** a primitiva modal só é construída a partir de caminhos de confirmação destrutiva (grep: invocações de `ui/modal.rs` = somente confirmações sim/não); todo o resto (agent/credential/webhook/whatsapp/createspace) migrou para coluna/inspetor; cada modal sobrevivente é exatamente 1 pergunta sim/não.
- **Dono:** DEV (Terminal J). **ADR-gate:** ADR-DESIGN-A (depende de D2-1).

**D2-6 — Teclado PRESERVADO + ponteiro promovido (palette com gatilho visível + atalho ao lado do comando)**
- **Por que (fonte):** `palette.rs`; `sidebar.rs:762` (botão de palette "🔍 Buscar comandos · ⌘K"); navegação por seta em `main.rs` (up/down/left/right/enter/escape). Trava dura: WCAG 2.2 AA + F1-2-6 não regridem (Raycast é amado *por ser* keyboard-first — o arcaico é a ausência de affordance de mouse, não a presença do teclado), acesso 2026-06-25.
- **Aceite observável:** a navegação por teclado **continua funcionando** (teste F1-2-6 verde — não removida); a palette tem gatilho **visível e clicável** com o atalho exibido; cada comando mostra seu atalho ao lado (o leigo clica, o power-user teclado); AccessKit Name/Role/Value preservados; gate WCAG verde.
- **Dono:** FRONTEND (Especialista em Telas — sensível a WCAG). **ADR-gate:** ADR-DESIGN-A (depende de D2-1).

### Frente C — WORKSPACE: sidebar · ícones · cores  *(fundador #1, #2, #3)*

**D3-1 — Rail de workspaces (flat, dim, mono, reordenável/escondível; notificação como ponto)**
- **Por que (fonte):** `sidebar.rs` (rail colapsado ▣ ↔ expandido). Externa: Linear *design refresh* (chrome abafado, alinhamento pixel-a-pixel) + Arc *Spaces*, acesso 2026-06-25.
- **Aceite observável:** o rail renderiza flat (`radius.none`), chrome abafado, ícones monocromáticos; workspaces reordenáveis e ocultáveis (interação verificada e persistida); notificação aparece como **ponto**, não badge gritado; token_ratchet verde.
- **Dono:** DEV (Terminal M [ultracode]). **ADR-gate:** —

**D3-2 — Ícones de workspace (monocromáticos, do sistema de ícones)**
- **Por que (fonte):** `sidebar.rs:788` (▣/●/🔍 emoji hoje). Consome D1-3 (sistema de ícones). Externa: Linear (removeu fundos coloridos de ícone no refresh 2024), acesso 2026-06-25.
- **Aceite observável:** cada workspace tem ícone vetorial monocromático (do sistema de D1-3), sem emoji e sem fundo colorido de ícone; ícone ≥ 24×24, contraste ≥ 3:1 no gate WCAG.
- **Dono:** DEV (Terminal I). **ADR-gate:** —

**D3-3 — Cor de workspace = identidade (modelo Arc: 1 acento por Espaço, chrome neutro)**
- **Por que (fonte):** `theme.rs:182` (`ACCENTS` → paleta de 1 acento por Espaço). Externa: Arc *Spaces* (cor como pista de "onde estou", não decoração) + PostHog (UM acento por tela), acesso 2026-06-25.
- **Aceite observável:** o usuário escolhe **1 acento por workspace** e a escolha **persiste** (gravada como evento/projeção via contrato existente do core — JSON local, **sem** o core importar tipo de UI); a cor identifica o Espaço (rail/primário "Novo"), o chrome restante fica neutro; o acento escolhido **nunca** é resolvido como status (teste); gate WCAG verde para a escolha em ambos os modos.
- **Dono:** BACKEND (Terminal B Ultra Code — persistência via evento/projeção). **ADR-gate:** ADR-DESIGN-B.

### Frente D — BOTÕES + TERMINAIS  *(fundador #4, #5)*

**D4-1 — Botões da topbar (colapsar 2 CTAs redundantes em UM "Novo"; Poderes/Visual/Ajustes/Centralizar = toggles neutros)**
- **Por que (fonte):** `main.rs:6020-6038` — `➕ Novo Terminal` (verde) e `✦ Novo Agente` (terracota) chamam a **MESMA** `open_agent_modal_create` (botões redundantes); demais já são neutros. Externa: PostHog/Balsamiq (UMA ação primária preenchida por tela, secundárias rebaixadas), acesso 2026-06-25.
- **Aceite observável:** existe **UM** botão primário "Novo" preenchido com o acento do workspace (não dois CTAs saturados); `Poderes`/`Visual`/`Ajustes`/`Centralizar` são toggles neutros de ícone+texto que **abrem/fecham as colunas** correspondentes (interação verificada); sem emoji nos botões; gate WCAG verde (1 par primário acento × superfície).
- **Dono:** FRONTEND (Especialista em Telas). **ADR-gate:** depende de D2-1 + D3-3.

**D4-2 — Design dos terminais (bloco estruturado quente; moldura `radius.sm`; sem hover-web falso; densidade dev-tool)**
- **Por que (fonte):** `canvas.rs` (terminal = ilha dark quente `0x221c15`, caret âmbar — já é). Externa: Warp (turno navegável, não scrollback) + Raycast (polimento nativo, sem `cursor:pointer` falso), acesso 2026-06-25.
- **Aceite observável:** a moldura do terminal usa `radius.sm` (4px, tokenizado — não `.rounded_md()`); densidade dev-tool aplicada (títulos menores, hairline em vez de borda pesada — medível por contagem de espaçamento via tokens); sem hover-web/`cursor_pointer` falso em área não-clicável; token_ratchet verde; o slot Portal na cena intacto.
- **Dono:** DEV (Terminal G [ultracode]). **ADR-gate:** depende de D2-1 + D1-1.

### Frente E — GATE DE QUALIDADE (transversal)

**D5-1 — Gate WCAG + token_ratchet por pacote + gate de tela do fundador**
- **Por que (fonte):** §II invariantes 2 e 3; a ressalva do design dir ("pares neutros novos do chrome precisam ser reprovados no gate — risco sinalizado"); base F2-1/F2-2/F2-4 ainda não validada na tela (o olho do fundador é o critério de aceite).
- **Aceite observável:** suíte completa verde (`cargo test -p lina-gpui` incluindo token_ratchet **global** + gate WCAG) para **todos** os pares neutros e acentos introduzidos pela reforma; nenhum literal `px`/`FontWeight` novo; **gate de tela**: o fundador abre o app e confirma "parece marca, não slop" (teste-âncora "tire o logo — ainda parece Lina?") — registrado como evidência observada, não suposta.
- **Dono:** DEV (Terminal R — runner de gate) + cold-review (lina-cold-review). **ADR-gate:** — (fecha a reforma).

---

## §V — SEQUENCIAMENTO (ondas + dependências EXPLÍCITAS)

> Regra: **ADR antes das stories que ele bloqueia**; **design system + geometria antes das telas que os consomem**; **shell de colunas antes das telas que migram para ele**.

- **Onda 0 — ADRs (gate de governança).** ADR-DESIGN-A · ADR-DESIGN-B · ADR-DESIGN-C aceitos em `docs/adr/`. *Bloqueia:* D1-1 (C), D1-5/D3-3 (B), D2-1 e cascata (A). *Donos:* Arquiteto + Arquiteto Webhook[ultra].
- **Onda 1 — Fundação (tokens/geometria/identidade).** D1-1 (após ADR-C) · D1-2 · D1-3 · D1-4 · D1-5 (após ADR-B). Paralela entre si; **precede** Ondas 3-5 (todas consomem tokens/geometria/ícones). Sem dependência do shell.
- **Onda 2 — Shell de colunas.** D2-1 (após ADR-A). Pode correr em paralelo à Onda 1; é **pré-requisito** de toda a Onda 3 e de D4-1.
- **Onda 3 — Telas no shell.** D2-2 · D2-3 · D2-4 · D2-5 · D2-6 (todas após **D2-1** + tokens da Onda 1). Paralelas entre si por arquivo.
- **Onda 4 — Workspace.** D3-1 · D3-2 (após **D1-3**) · D3-3 (após **ADR-B** + D1-5). D3-1 pode iniciar com a Onda 1.
- **Onda 5 — Botões + terminais.** D4-1 (após **D2-1** + **D3-3**) · D4-2 (após **D2-1** + **D1-1**).
- **Onda 6 — Gate de qualidade.** D5-1 (após todas) — gate WCAG/token_ratchet completo + gate de tela do fundador. Fecha a reforma.

**Caminho crítico:** ADR-A → D2-1 → {D2-2…D2-6, D4-1} → D5-1. A geometria (ADR-C → D1-1) é caminho paralelo que precisa fechar antes de D4-2 e do gate final.

---

## §VI — DEFINIÇÃO DE TIME (papel → frente, do roster)

> Aproveitando o modo **ULTRACODE** de Terminal G, Terminal M e Arquiteto Webhook para a arquitetura e o trabalho pesado (re-arquitetura de navegação, migração de 118 sites, rail). Frontend + tokens + geometria são o grosso.

| Papel | Terminal | Frentes / stories |
|---|---|---|
| ARQUITETO | Arquiteto | ADR-DESIGN-B (cor de workspace) · ADR-DESIGN-C (geometria) |
| ARQUITETO | Arquiteto Webhook **[ultracode]** | ADR-DESIGN-A + construção do **shell de colunas** (D2-1) — Ondas 0 e 2 |
| FRONTEND | Especialista em Telas | Poderes em abas (D2-2) · teclado+ponteiro (D2-6) · botões topbar (D4-1) · ícones (D1-3) |
| BACKEND | Terminal B **Ultra Code** | recalibração cromática/tokens (D1-5) · persistência da cor de workspace via evento/projeção (D3-3) |
| DEV | Terminal G **[ultracode]** | migração de geometria — 118 sites (D1-1) · design dos terminais (D4-2) |
| DEV | Terminal M **[ultracode]** | elevação por hairline (D1-2) · rail de workspaces (D3-1) |
| DEV | Terminal I | Ajustes em coluna (D2-3) · ícones de workspace (D3-2) |
| DEV | Terminal K | disciplina tipográfica (D1-4) · inspetor de criação (D2-4) |
| DEV | Terminal J | modal retido só destrutivo (D2-5) · apoio à recalibração cromática (D1-5) |
| DEV | Terminal R | gate WCAG + token_ratchet por pacote + validação de regressão (D5-1) |

**Costuras de dono único** (uma rodada): `theme.rs` (tokens — D1-1/D1-2/D1-4/D1-5 coordenam pelo Maestro), `main.rs` (layout/topbar — dono D2-1 na Onda 2, D4-1 na Onda 5). Workers não commitam; o Maestro valida de fora (exit codes diretos) e commita por fatia.

---

## §VII — LACUNAS / RISCOS SINALIZADOS

- **Sem papel QA dedicado no roster.** O gate de tela do fundador (D5-1) é o critério de aceite humano, mas falta um QA que prove regressão por mutação (WCAG/ratchet/F1-2-6). Recomendo spawnar um QA antes da Onda 6 ou delegar a Terminal R + cold-review.
- **Gate ao fundador (3 avais antes de codar):** (1) ADR-A para o shell de colunas; (2) tradução de "cantos retos" — entrego chrome 100% reto (0px) + cards a 4px (linguagem de dois degraus do PostHog); se ele quiser **tudo** a 0px literal, é uma palavra dele; (3) base F2-1/F2-2/F2-4 está "fechada no código" mas o gate de tela ainda não rodou — a reforma parte daqui.
- **D3-3 (persistência da cor)** é a única story que toca o core: precisa confirmar que o contrato evento/projeção existente comporta a escolha de acento **sem** o core importar tipo de UI (porta UiHost). Validar no ADR-DESIGN-B.

---

## §VIII — PROGRESSO (registro vivo)

- **2026-06-25 — ADRs 0053 · 0054 · 0055 escritos e aceitos** (`docs/adr/`); decisões do fundador seladas (§0).
- **2026-06-25 — D1-1 fatia-1 (régua + primitivos):** criado o helper **`RadiusExt`** (`.rounded_chrome()` = `radius.none` / `.rounded_content()` = `radius.sm`) em `app/lina-gpui/src/ui.rs` — a régua de geometria num lugar só, sem literal `px` (a catraca não conta consumo de token). Migrados os **5 primitivos** do catálogo (`ui/button`, `ui/input`, `ui/modal`, `ui/toast`, `ui/node_toolbar`) → propagam o canto reto a todos os consumidores. **Verde provado (exit 0):** `cargo fmt` · `clippy --all-targets` 0 warnings · **672** unit + **14** a11y/WCAG + **2** token_ratchet (catraca NÃO subiu). **Pendente da fatia:** `ui/panel.rs` (variantes painel/card, tratamento por-variante) + os ~115 sites restantes (`main.rs` 40, `agent_modal` 24, `sidebar` 14, `attention_ui` 8, …) migram em cima da régua; depois **D2-1** (shell de colunas, ADR 0053) e **D1-2** (hairline).
- **2026-06-25 — D1-1 COMPLETA (geometria 100% tokenizada):** fan-out de 15 agentes migrou os 114 sites restantes para `RadiusExt`. Resultado: **0** chamadas reais de `.rounded_md/lg/xl` no shell (só restam 2 menções em comentário); **19** `rounded_chrome` (reto) + **102** `rounded_content` (4px). `panel.rs` ficou como chrome (split por-variante painel/card adiado — anotado). **Verde provado (exit real 0):** `cargo fmt` · `clippy --all-targets` 0 warnings · **672** unit + **14** a11y/WCAG + **2** token_ratchet (catraca NÃO subiu). **Critério de aceite de D1-1 cumprido:** grep `rounded_md` = 0; chrome via `radius.none`, conteúdo via `radius.sm`; sem `px(0.)` literal. **Ressalva (gate de tela):** o verde prova a régua aplicada, não que cada site recebeu a classe certa — o ajuste fino "este devia ser reto / 4px" é o olho do fundador na tela. **Próximo:** D1-2 (hairline) e D2-1 (shell de colunas — a re-arquitetura).
- **2026-06-25 — D2-2 piloto REVERTIDO (lição cara):** tentei transformar a Área de Poderes (modal) numa **coluna lateral overlay** (`.absolute().right_0().w(420)`). O fundador rejeitou **na tela**: *"Volta do jeito que tava, tava melhor. Isso que vc fez é justamente o que eu quero fugir."* Um painel `.absolute()` na lateral é um **modal disfarçado** — flutua POR CIMA do canvas, é o mesmo anti-padrão "janela solta" que a reforma quer matar. Revertido ao modal original (compila, `cargo check` exit 0). **Lição que reescreve D2-1/D2-2:** "coluna dedicada" = **ESTRUTURA de layout** (flex real: rail · canvas · coluna; o canvas **encolhe** e cede espaço — modelo PostHog), **NUNCA** overlay flutuante. E re-arquitetura de layout precisa **alinhar a visão com o fundador ANTES** de codar — não "faço cego e mostro depois".

---

*Fontes internas verificadas: `theme.rs:10,172,182-185,242,351,408-413,457-477`; `main.rs:5002-5011,6020-6038` + `.absolute()` (múltiplas); `ui/modal.rs:286-318`; `sidebar.rs:762,788,809`; `powers_panel.rs`; `credential_modal.rs`; `agent_modal.rs`; ADR 0052; vault §VIII.1. Externas (acesso 2026-06-25): PostHog (visual-identity, dev-tool, vibe-designing, in-practice), Linear (design refresh 2024), Raycast Eng, Arc Spaces, NN/g (Modal & Nonmodal), Userpilot (modal UX), Baymard (accordion-and-tab).*
