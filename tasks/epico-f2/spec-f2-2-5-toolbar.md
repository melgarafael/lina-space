# SPEC F2-2-5 — Toolbar contextual do nó

> **Entregável de ARQUITETURA (Terminal A).** Decisão de estrutura ANTES do código.
> A r6 constrói a partir desta spec **sem uma pergunta de esclarecimento**. Onde digo
> "r6 implementa", já dei o contrato; onde digo "porta", é continuidade a preservar.

- **Story:** F2-2-5 (épico vault `38 - Epico Fase 2` §F2-2) — toolbar de 3–5 ações
  rotuladas no terminal/card selecionado.
- **Rodada:** r5 (spec) → r6 (build).
- **Fontes (pesquisa embutida):**
  - **D4-A3** (`tasks/pesquisa-f2/entrega-d4-comandos-menus.md:50-57`): hierarquia
    `visível-rotulado > contextual redundante > busca com gatilho > atalho`. Toolbar
    contextual de 3–5 ações no objeto selecionado, **modelo Canva — nunca ribbon, nunca
    icon-only**. Experts localizam por POSIÇÃO; labels sobrevivem ao expert.
  - **D4-A7** (`:70`): estado acoplado a **ação nomeada de 1 clique**; "estado sem ação é
    banido"; decisões binárias (Manter/Remover); **WCAG 1.4.1 — cor nunca sozinha**
    (texto+ícone).
  - **D4 §regra dura anti-Zed** (`:150`, candidata a invariante de UI): *"nenhuma função
    existe SÓ atrás de atalho ou SÓ dentro da paleta — todo atalho é acelerador de um
    caminho de mouse visível."* **Esta toolbar é o caminho de mouse visível; a paleta é o
    caminho de teclado. O registry único garante paridade.**
  - **D4-A3 tie-breaker** (`:57,149`): ações **hover-only PROIBIDAS** — pista permanente
    (kebab visível). Hover é o long-press do desktop.
  - **ADR 0029** (`docs/adr/0029-*.md`): câmera/viewport FORA do event log; "câmera nunca
    se move sozinha".

---

## 0. Princípio que rege todas as decisões

**1 registry de ações, N superfícies.** A toolbar **não inventa** ações nem define um
catálogo paralelo: ela é a **segunda superfície** sobre o MESMO registry da paleta
(`PaletteAction` / `Command` / `action_key` em `app/lina-gpui/src/palette.rs`). Toda ação
que a toolbar mostra é um `Command`; toda invocação emite o MESMO evento da paleta. Isso é
o que torna a regra anti-Zed verdadeira por construção: mouse (toolbar) e teclado (paleta)
são duas portas para a mesma ação.

---

## 1. Onde a toolbar vive na CENA

**Decisão: overlay em ESPAÇO DE TELA, ancorado ao card focado, camada de composição própria
ligada ao nó selecionado — uma só por vez.**

### 1.1. Geometria
- O card hoje é `div().absolute().left(px(sx)).top(px(sy)).w(px(CARD_W*z)).h(px(CARD_H*z))`
  (`main.rs` ~3844; `CARD_W=680, CARD_H=500` em `bridge.rs:2370`), com `(sx,sy) =
  camera.world_to_screen((nv.x, nv.y))`.
- A toolbar é renderizada como **elemento `absolute` irmão no root do canvas**, posicionada
  na borda SUPERIOR do card focado (modelo Canva: barra flutuante acima do objeto):
  `left ≈ sx`, `top ≈ sy − toolbar_h − gap`.
- **Regra de flip (obrigatória):** se `sy − toolbar_h − gap < SAFE_TOP` (abaixo da topbar,
  44px), a toolbar vira para DENTRO da borda superior do card (logo abaixo do header). Nunca
  renderiza sob a topbar nem fora da viewport.

### 1.2. Tamanho em espaço de TELA (não ×zoom) — decisão deliberada
A toolbar é **chrome/controle**, não conteúdo de canvas. Suas dimensões vêm de **tokens**
(`spacing`, `typography`), **sem multiplicar por `z`** — igual à paleta e aos modais, que
são screen-space. Razão (D4-A3): icon-only e texto ilegível falham; um controle precisa ser
**sempre legível**, em qualquer zoom. Ela só LÊ a câmera (`world_to_screen`) para se
posicionar atrás do card; **nunca a escreve** (ADR 0029).

### 1.3. Só no nó FOCADO (R3 / LOD honrados)
- A toolbar aparece **exclusivamente** para o nó em `Zone::Focus`
  (`canvas.rs:classify_zone` — focado **e** dentro da viewport). Periferia e suspenso: sem
  toolbar.
- **Por que isto respeita R3 ("LOD degrada fidelidade, nunca remove camada de composição"):**
  a toolbar é uma camada de composição **vinculada ao nó selecionado** — há no máximo um
  selecionado. Não mostrá-la nos nós de periferia **não é remover a camada**; é que aqueles
  nós não são o alvo do contexto. O *slot* de composição permanece na arquitetura da cena
  (e fica disponível para o `PortalEngine` futuro renderizar controles sobre uma textura
  externa). No zoom-out extremo, como a toolbar é screen-space, ela **não encolhe até
  ilegível** — a fidelidade do controle é preservada, que é exatamente o que o LOD manda.
- **Câmera nunca se move sozinha:** aparecer/posicionar a toolbar não move a câmera. A única
  ação da toolbar que mexe na câmera é **Centralizar**, e ela só dispara por **clique humano**
  (gesto explícito) — dentro da invariante.

---

## 2. As 3–5 ações — por estado do nó, do registry único

### 2.1. Mapa estado→significado (fonte: `canvas.rs:aggregate_badge`)
Os "estados leigos" do despacho mapeiam para o modelo de badge JÁ existente (não inventar
um segundo vocabulário de estado):

| Estado (despacho) | Sinal real | Badge |
|---|---|---|
| **pede-aprovação** | `needs_human` (gate de custódia/permissão pendente p/ o nó) | `NeedsYou` |
| **trabalhando** | `NodeStatus::{Busy,Running,Starting}` | `Running` |
| **pronto** | `NodeStatus::Idle` (resposta terminada, vivo) | `Waiting` |
| **novo** | `NodeStatus::Starting` (recém-spawnado) | `Running` |

`needs_human` é a MESMA fonte da fila de atenção (`crates/lina-core/src/attention.rs` →
projeção; consumida hoje por `attention_ui.rs`). A toolbar **lê** esse sinal; não o computa.

### 2.2. Conjunto de ações (máx. 4 visíveis; todas rotuladas)

A toolbar mostra, para um nó-agente/terminal focado:

| Rótulo (leigo) | Ação no registry | Variante `Button` | `when(...)` (filtro de contexto) |
|---|---|---|---|
| **⚑ Atender** | `GotoAtencao(id)` **(novo, aditivo)** | `Confirm` (primária) | `when(needs_human)` |
| **✎ Editar** | `EditAgent(id)` *(existe)* | `Secondary` | `when(kind==Agent && status.is_alive())` |
| **⤢ Centralizar** | `FocusNode(id)` *(existe)* | `Secondary` | sempre |
| **✕ Encerrar** | `CloseNode(id)` **(novo, aditivo)** | `Destructive` | `when(roster.len() > 1)` |

- **pede-aprovação** → 4 ações (Atender + Editar + Centralizar + Encerrar).
- **trabalhando / pronto / novo** → 3 ações (Editar + Centralizar + Encerrar). Sem ação
  primária colorida: o nó já está no que tem de estar; cor é reservada a significado.
- Dentro da régua D4-A3 (3–5). Ordem fixa: primária → reversíveis → destrutiva (a
  destrutiva por último, padrão de rodapé).

### 2.3. As duas ações ADITIVAS (decisão + justificativa)

**`FocusNode` e `EditAgent` já existem** (`palette.rs:36-38`). Faltam duas, que a r6 ADICIONA
ao enum `PaletteAction` (aditivo — `action_key` ganha braços novos; logs antigos replayam):

- **`GotoAtencao(NodeId)`** — foca/revela o nó **e abre a superfície de resposta da fila de
  atenção** (toast/painel 🔔 que já tem Aprovar/Recusar — `attention_ui.rs:855-865`).
  **NÃO aprova nem recusa.** Decisão de segurança (doutrina: *autorização tem autoridade
  única; nenhum campo decide autorização*): a aprovação/recusa permanece **só** na fila de
  atenção, com seu gate. A toolbar dá **1 clique até a decisão**, não uma decisão paralela.
  Resolve a tensão entre D4-A7 ("ação de 1 clique para estado ruim") e a doutrina de
  segurança: o 1 clique leva ao lugar único onde se decide. (Coerente com o `GuardAsk`
  goto-only já existente — `attention_ui.rs:84`.)
- **`CloseNode(NodeId)`** — encerra o nó (mata o processo do CLI). É o que o ✕ do header faz
  hoje; vira ação de registry para ganhar paridade na paleta. Variante `Destructive`. Por
  matar processo, segue o caminho de encerramento **já existente** (mesmo gate do ✕ atual);
  a r6 **reusa** esse caminho, não cria um novo.

> **Cruzamento com a tecla "o" e a fila F1:** "o" (`canvas_focus.rs:135` → `NextBusy`) leva
> você ATÉ o próximo nó ocupado; a toolbar age SOBRE o nó onde você está. São complementares
> — navegação (o) vs. ação no contexto (toolbar). Nenhuma sobreposição. **Atender** fecha o
> ciclo: "o" te leva ao nó que precisa de você → a toolbar te leva à decisão.

### 2.4. Nós Nota/Pasta (escopo)
F2-2-5 mira **nós-agente/terminal** (o caso da pesquisa). Para Nota/Pasta: `Editar` (via
`EditAgent`) é filtrado por `when(kind==Agent)`; `Centralizar` e `Encerrar` permanecem. Uma
toolbar rica de Nota/Pasta (abrir/renomear) é **follow-up** (F2-2-6 ou despacho próprio) —
**porta aberta**, não fechada: o seletor de ações por contexto (§4.1) já aceita `kind`.

---

## 3. Contrato com o catálogo `ui/`

- **Botão:** `ui::Button` (`app/lina-gpui/src/ui/button.rs`), variantes da §2.2,
  `.size(ButtonSize::Sm)` (toolbar densa), `.role(Role::Button)` default, `.aria(...)`
  quando o glifo lidera. **`.on_click(cx.listener(...))`** no padrão dos call-sites
  (`agent_modal.rs:2288`). Token-puro de fábrica (zero cor/medida literal — catraca F2-1-5
  nasce verde).
- **NUNCA icon-only** (D4-A3): todo botão tem **label leigo visível**; o glifo (⚑✎⤢✕) é
  ornamento à esquerda, não o único sinal (WCAG 1.4.1).
- **Strings — fonte única por módulo** (padrão F1-4, igual `agent_modal.rs:42-207`): a r6
  cria `const COPY_TB_*: &str` no módulo da toolbar (`COPY_TB_ATENDER="Atender"`,
  `COPY_TB_EDITAR="Editar"`, `COPY_TB_CENTRALIZAR="Centralizar"`,
  `COPY_TB_ENCERRAR="Encerrar"`). Passam pela copy congelada (R9 — zero jargão; sem
  YAML/profile/PTY).
- **Superfície da barra:** container `surface.chrome` (ou `raised`) + `border` 1px +
  `radius.md` (degrau dominante) + **sem sombra** (flat honesto da casa). `gap = spacing.sm`,
  `pad = spacing.sm`. Tudo via `theme::active()`.
- **hover-only PROIBIDO** (D4-A3): a toolbar é **pista PERMANENTE** sobre o card focado —
  visível enquanto o nó está focado, não ao passar o mouse. Se um dia exceder 5 ações: kebab
  **⋯ visível** (nunca menu hover). Hoje (máx. 4) não há overflow.

---

## 4. Eventos — reusa `PaletteCommandInvoked`

**Decisão: clicar numa ação da toolbar apenda o MESMO `DomainEvent::PaletteCommandInvoked {
key }` (`crates/lina-core/src/events.rs:819-826`), com a chave estável `Command::key()` da
ação** (ex.: `"edit_agent:<uuid>"`, `"focus_node:<uuid>"`, `"close_node:<uuid>"`,
`"goto_atencao:<uuid>"`).

**Justificativa (por que reusar, não criar evento novo):**
1. **1 registry, 2 superfícies → 1 identidade de ação → 1 evento.** Uma ação usada pela
   toolbar TAMBÉM alimenta o ranking/hit-count da paleta (tier 4) — o leigo que clica
   "Editar" na toolbar vê "Editar" subir na paleta. Sem isso, as duas superfícies
   divergiriam e o ranking mentiria.
2. **Aditivo:** as duas keys novas (`close_node:`, `goto_atencao:`) entram em `action_key`;
   logs antigos replayam intactos (ranking degrada sem elas — `serde(default)` da série).
3. **Meta, não canvas** (o doc do evento já diz "META: não entra na projeção do canvas"): o
   evento é telemetria/ranking. O EFEITO real de cada ação continua pelos caminhos já
   modelados — `EditAgent`→modal; `CloseNode`→evento de encerramento de nó já existente;
   `FocusNode`→câmera (sessão local, ADR 0029); `GotoAtencao`→foco + abrir fila. A toolbar
   apenas **acrescenta o append de telemetria ao lado de executar**, exatamente como a paleta
   faz hoje (`main.rs open_command_palette`). **Nenhum tipo de evento novo.**

---

## 5. Fronteiras da r6 (arquivos + dono natural)

| Arquivo | Mudança | Dono natural |
|---|---|---|
| `app/lina-gpui/src/palette.rs` | +`PaletteAction::{GotoAtencao(NodeId), CloseNode(NodeId)}`; +braços em `action_key`; **+`pub fn node_commands(ctx: &NodeCtx) -> Vec<Command>`** (o seletor único de ações-de-nó, com os `when(...)` da §2.2) — reusado pela paleta dinâmica E pela toolbar. **Remove o `#[allow(dead_code)]` do `when`** (esta story é o consumidor prometido em `palette.rs:136-137`). | **Registry (Terminal B / dev)** |
| `app/lina-gpui/src/ui/node_toolbar.rs` **(novo)** | Componente da toolbar: render screen-space + `const COPY_TB_*`. Consome `node_commands` e renderiza um `Button` por `Command`. Função PURA de posicionamento `toolbar_anchor(card_rect, toolbar_h, safe_top) -> Option<(f32,f32)>` (None fora de `Zone::Focus`; aplica o flip). | **FRONTEND (Terminal C)** |
| `app/lina-gpui/src/ui.rs` | +`mod node_toolbar; pub use`. | FRONTEND |
| `app/lina-gpui/src/main.rs` **(costura — dono único)** | Render do overlay para o nó focado; `on_click` → executa ação + apenda `PaletteCommandInvoked`; passa `NodeCtx{kind,status,needs_human,roster_len}` ao seletor; aplica posição/flip. | **COSTURA (Maestro/designado) — peers não tocam** |
| Testes (colocados) | Puros, sem gpui: `node_commands` por estado; mapeamento ação→`key`; `toolbar_anchor` (None em periferia/suspenso; flip). | Quem implementa cada peça |

**Sem tocar:** `events.rs` (evento reusado), `canvas_focus.rs` (§6), `attention.rs`
(`GotoAtencao` só navega). Mantém `clippy -D warnings`, `fmt`, **catraca F2-1-5 verde**,
suíte verde.

---

## 6. a11y / teclado — F1-2-6 INTACTO

**Decisão: a toolbar é a superfície de MOUSE; o teclado usa a PALETA (registry compartilhado).
ZERO keybinding novo no canvas.**

- **Por quê:** a regra anti-Zed (D4 `:150`) exige que toda ação tenha caminho de mouse
  visível **e** de teclado, sem nenhum ser o único. A toolbar dá o mouse; a paleta (⌘K ou o
  botão rotulado "Buscar comandos") dá o teclado — e, como `node_commands` é o MESMO seletor,
  as ações do nó focado aparecem na paleta filtradas por contexto. Paridade por construção.
- **F1-2-6 não muda:** `canvas_focus.rs` (`FocusLevel`, `KeyIntent`, `on_key`) fica **intocado**.
  Não adicionamos intenção/tecla para "entrar na toolbar" — isso arriscaria o canvas por
  teclado que o fundador já validou. (Roving-focus DENTRO da toolbar é **porta aberta** para
  uma story futura via novo `KeyIntent`, **não** requisito da F2-2-5.)
- **Sem roubo de foco:** a toolbar renderiza passiva — não chama `.focus()`, não captura
  teclado, não rouba o foco do PTY (invariante de foco + ADR 0029). Mouse ativa; teclado é a
  paleta.
- **Árvore AccessKit:** cada `Button` carrega `role=Button` + `aria_label` (o componente já
  faz — `button.rs:244-245`). Como filhos associados ao card focado, um leitor de tela que
  navega o nó focado ouve as ações disponíveis. Recomendação: envolver a barra num container
  com `role=Toolbar` + `aria_label("Ações de <nome do nó>")` (rótulo via `a11y::node_label`).

---

## 7. Critério de aceite OBSERVÁVEL (roda e se mede)

1. **`node_commands` por estado** (teste puro): `needs_human=true` → contém `GotoAtencao`;
   `kind!=Agent` → **não** contém `EditAgent`; `roster_len==1` → **não** contém `CloseNode`;
   todo `Command` retornado tem label não-vazio (**anti icon-only**) e variante coerente
   (Encerrar=Destructive).
2. **Ação→evento** (teste puro): cada ação produz a `key()` esperada; clicar apenda
   `PaletteCommandInvoked{key}` com a chave estável (não o rótulo).
3. **Posição/zona** (teste puro): `toolbar_anchor` devolve `None` para periferia/suspenso e
   aplica o flip quando `sy−h<safe_top`.
4. **Gates de fora:** suíte verde · catraca F2-1-5 verde (zero magic number novo) ·
   `clippy -D warnings` · `fmt` — exits diretos.
5. **Gate na TELA (fundador):** na barra do card focado, 3–4 ações **rotuladas e
   permanentes** (não hover); "Atender" só aparece quando o nó pede você e leva à decisão (não
   decide); teclado faz o mesmo pela paleta. Nenhum estado "morto sem ação" (D4-A7).

---

## 8. Portas que esta spec NÃO fecha

- **`PortalEngine`/`ExternalTextureLayer`:** o slot de composição da toolbar é screen-space e
  vinculado ao nó — um controle sobre textura externa futura encaixa no mesmo padrão de
  ancoragem (`card_rect → toolbar_anchor`).
- **Toolbar de Nota/Pasta** e **roving-focus por teclado dentro da barra:** ambos cabem nas
  abstrações desta spec (`NodeCtx.kind`; novo `KeyIntent`) sem retrofit.
- **Mais ações por estado:** entram como `Command` no `node_commands` com `when(...)` — a
  régua 3–5 e o kebab-visível-no-overflow já estão definidos.

---

**PRONTO:** spec completa. Decisões: (1) overlay screen-space ancorado ao card focado, só em
`Zone::Focus`, R3/câmera honrados; (2) 3–4 ações do registry único filtradas por `node_commands`
+ `when`, com 2 ações aditivas (`GotoAtencao` navega-não-decide por segurança; `CloseNode` reusa
o encerramento atual); (3) `ui::Button` rotulado, strings `COPY_TB_*`, hover-only proibido;
(4) reusa `PaletteCommandInvoked` (1 registry, 2 superfícies, ranking íntegro); (5) fronteiras
e donos mapeados, costura em `main.rs` de dono único; (6) F1-2-6 intocado — paleta é o caminho
de teclado. A r6 constrói sem pergunta de esclarecimento.
