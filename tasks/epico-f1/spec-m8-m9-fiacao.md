# SPEC de fiação — M8 (Switcher de Espaços) + M9 (Criar Espaço) · onda F1-4, rodada 3

> Papel UIUX_DESIGNER · despacho r2-specs-f14 · 2026-06-10 · v1.1 (revisada por banca adversarial de 5 lentes: símbolos×código, citações×copy, cobertura×despacho, territórios, consistência×stories — 26 achados triados e incorporados)
> Fontes: `ondas-2-4.md` F1-4-4 (linhas 220-237) e F1-4-1 critério 6 · `ux-flows.md` fluxo (e) M8/M9 · `copy-f1-4.md` (strings congeladas) · código em HEAD de 2026-06-10 (pós-F1-4-1; símbolos verificados por grep + banca nesta data).
> **Regra deste doc:** toda string de superfície é **citação literal** de `copy-f1-4.md` (referida por §) ou do `ux-flows.md` (linha) — string nova só com racional explícito, marcada **[NOVA]**. Símbolos de código citados com `arquivo:linha`.
> **Territórios (coordenacao-maestri-externa.md):** `crates/lina-core/src/attention.rs`/`broker.rs` = time EXTERNO; `events.rs` = costura quente. Nada aqui propõe editar arquivo alheio — necessidades viram **pedidos de costura** (§6), o Maestro coordena. `attention_ui.rs`/`persistence_ui.rs`/`dashboard.rs` são INTERNOS (rodada 3, dono único a definir no despacho).

## §0 · Mapa de módulos (verificado em 2026-06-10)

| Símbolo | Onde | Papel na fiação |
|---|---|---|
| `WorkspaceEntry { name, events_dir, focused, nodes }` · `list_workspaces()` · `project_entry()` | `app/lina-gpui/src/persistence_ui.rs:151` · `:177` · `:164` | lista do M8 hoje (T6 por varredura). ⚠ `project_entry` devolve `Option` e **descarta** Espaço que não projeta — a fiação INVERTE isso (§1, linha de erro) |
| `WorkspaceRegistry` (core): `active_count()` · `can_create(tier)` · `focused()`/`last_focus` · `rederive_entry()` | `crates/lina-core/src/workspace.rs:466` · `:471` · `:457` · `:478` | registry global `~/.lina/` (F1-4-1) — autoridade de arquivado/recência; **fonte canônica da lista do M8** (a varredura vira fallback; convergência = §6-C3) |
| `LicenseTier::{Free, Pro}` · `workspace_limit()` · `can_create_workspace(tier, active_count)` | `crates/lina-core/src/workspace.rs:82` · `:92` · `:101` | **seam PURO de gating — JÁ EXISTE** (Free=1; arquivado não conta; `Err(LimitReached{limit})`) |
| `PersistenceModel::switch_to(name)` → `WorkspaceFocusSet` | `persistence_ui.rs:439` · `events.rs:447` | troca de Espaço (já emite evento) |
| `UiState` (6 estados) · `from_projection(status, stalled)` · `label()` · `color_token()` | `app/lina-gpui/src/dashboard.rs:61` · `:81` · `:95` · `:108` | estado por Agente — «Trabalhando / Esperando você / Ocioso / Sem resposta / Dormindo» + «Encerrado» (6ª palavra, extensão registrada ao fluxo (c) em `dashboard.rs:72` — ver §6-A3) |
| `CostLine` (`text/usd/estimated/has_data`) · `from_today` · fallbacks `without_data()/no_session_today()/untrackable()` | `dashboard.rs:163` · `:206` · `:175/:186/:197` | custo de hoje — formato real `~US$ {valor} (estimado)`; **zero nunca é exibido** ("zero exibido seria mentira" — subcontagem JSONL, `dashboard.rs:208`) |
| `workspace_cost_today(...)` | `dashboard.rs:375` | agregado de custo do Espaço (hoje: só do focado) |
| `BadgeView { count, pulsing }` · `badge_view(&[AttentionItem])` | `app/lina-gpui/src/attention_ui.rs:59` · `:66` (INTERNO) | contagem 🔔 (hoje: global) |
| `AttentionItem` (sem `workspace_id`) | `crates/lina-core/src/attention.rs` — **TERRITÓRIO EXTERNO** | gap p/ 🔔 por Espaço (§6-C1) |
| `FocusPreset` · `preset_team()` · `apply_preset()` → `WorkspaceCreated{focus_preset}` + `NodeAdded`/`NodeRoleAssigned` | `app/lina-gpui/src/gallery.rs:41` · `:127` · `:147` | Galeria de Focos (T3/W4-5) que o M9 reusa (3 presets hoje × 5 cards desenhados — §6-B4) |
| `WorkspaceDefaultCwdSet` · `WorkspaceRenamed` · `WorkspaceArchived` (projeções em `events.rs:1087/:1096/:1102`) | `crates/lina-core/src/events.rs:750/:755/:761` | ciclo de vida — **já existem** (F1-4-1); falta só "desarquivar" (§6-C2) |
| `ProjectedState.workspace_id` · `.default_cwd` · `.archived` | `events.rs:945/:951/:955` | projeção por Espaço |
| `resolve_spawn_cwd(default_cwd, ws_root, node_key)` | `crates/lina-core/src/workspace.rs:151` | seam ÚNICO de cwd (ADR 0022): `default_cwd` → senão `n-<key>` isolado |
| `Settings.default_cwd` · `set_default_cwd()` | `persistence_ui.rs:221` · `:530` | pasta-base sugerida (T7) |
| a11y | `app/lina-gpui/src/a11y.rs` + `a11y_live.rs` | AccessKit (spike já aterrissado) |

---

## §1 · M8 — Switcher: fiação linha-a-linha

Gramática congelada (citação literal de `copy-f1-4.md §5`):

```
▣ {nome do Espaço}     {N} Agentes ●    ~R$ {valor}  🔔  ⌘{n}
```

> ⚠ A célula de moeda está **PENDENTE-MAESTRO**: a copy congela «~R$», o produto inteiro emite `~US$` (`CostLine`, `dashboard.rs:212`) — e conversão para R$ exigiria taxa de câmbio (= rede ou taxa fixa mentirosa, ambas contra o invariante offline/honestidade). Variantes na célula abaixo; emenda registrada em §6-A1. Esta spec NÃO decide a moeda.

| Campo | String (citação) | Fonte de dados (fiação) | Notas/gaps |
|---|---|---|---|
| Nome do Espaço | `{nome}` | `WorkspaceRegistry` (canônico) / `WorkspaceEntry.name` (`persistence_ui.rs:151`); fallback `"(sem nome)"` já existe | convergência registry×varredura: §6-C3 |
| Agentes vivos | «{N} Agentes» / «1 Agente» (§5) | projeção do Espaço: `ProjectedState.nodes` com `kind == "Terminal"` e `UiState != Encerrado`, via `UiState::from_projection` (`dashboard.rs:81`) | Espaços de FUNDO exigem projetar cada store (read-only) — risco de perf/locks registrado em §6-B5; estender `WorkspaceEntry` com os campos derivados (§6-C3) |
| Estado dominante (● + tooltip) | tooltip «{N} Agentes — {Estado}» (§5); {Estado} = `UiState::label()` (`dashboard.rs:95`) | **regra de dominância (decisão desta spec):** pior-primeiro — `Sem resposta > Esperando você > Trabalhando > Ocioso > Dormindo > Encerrado`; cor = `color_token()` (`dashboard.rs:108`) | racional: governança worst-first — o dot do Espaço de fundo grita o pior estado de dentro ("nada fica esquecido", F1-4-4); vermelho continua reservado ao hang real. «Encerrado» é a 6ª palavra (extensão do código) — emenda de vocabulário em §6-A3 |
| Todos dormindo | a dupla vira «dormindo» (§5, caixa baixa intencional) | todos os vivos em `UiState::Dormindo` | `Dormindo` SEM produtor v1 (`dashboard.rs:70`) — ramo fiado e dormente até o B1/teto (§6-B2) |
| Sem Agentes | «sem Agentes ainda» (§5) | 0 nós Terminal na projeção | todos-Encerrado (≥1 nó, 0 vivos): «{N} Agentes» + ●cinza-escuro, tooltip «{N} Agentes — Encerrado» — composição com a 6ª palavra do código (`dashboard.rs:102`); racional: "sem Agentes ainda" mentiria (houve time). Depende da emenda §6-A3 |
| Custo do dia | tooltip «gasto estimado de hoje neste Espaço» (§5) | `workspace_cost_today()` (`dashboard.rs:375`) sobre a projeção daquele Espaço; forma curta derivada de `CostLine.usd/estimated` (formatação de `from_today`, `dashboard.rs:206`, sem o sufixo — o «estimado» viaja no tooltip) | **PENDENTE-MAESTRO (§6-A1/A2), duas variantes prontas:** (a) mecanismo atual — «~US$ {valor}»; (b) decisão por R$ exibido — «~R$ {valor}» + fonte de conversão a definir. Em AMBAS: sem dado → «—» na linha + tooltip com o fallback honesto de `CostLine.text` («sem estimativa de custo ainda» `dashboard.rs:177` / «sem sessão hoje» `dashboard.rs:188`) — **nunca «0,00»** |
| 🔔 pendência | tooltip «{N} Pedidos esperando você» / «1 Pedido esperando você» (§5) · item cross-Espaço: prefixo «{Espaço} · {Agente} pede permissão» (§5) | Espaço FOCADO: `badge_view().count` (`attention_ui.rs:66`); por-Espaço: **bloqueado pelo gap §6-C1** | a costura C1 é **pré-requisito INCONDICIONAL do DoD de F1-4-4** ("pendência de atenção" é parte do mini-status, `ondas-2-4.md:225`; um Pedido criado ANTES da troca persiste mesmo se o fundo adormecer, e ux-flows:672 já exige aprovar cross-Espaço sem trocar). Enquanto a costura não aterrissa (estado TRANSITÓRIO, nunca aceite final): linha de fundo não mostra sino nem afirma "em dia" — ausência de dado ≠ zero pendência |
| `⌘{n}` | atalho visível na linha (ux-flows:661-663) | **índice ESTÁVEL** por ordem de criação no `WorkspaceRegistry` — o rótulo ⌘n da linha mostra o índice real do Espaço, NÃO a posição visual | racional: ⌘1…9 funciona com o M8 FECHADO (ux-flows:652) — indexar pela ordem visível com focado-primeiro reembaralharia a cada troca e deixaria ⌘1 sempre no-op. A lista visual pode seguir focado-primeiro sem tocar nos atalhos. "Espaços recentes" (escopo F1-4-4): `last_focus` já existe no registry (`workspace.rs:457`) — adiado, registrado em §6-B3; Dúvida #2 (colisões) segue PENDENTE com o Maestro |
| Busca | «( buscar… )» (ux-flows:659) · vazio: «Nenhum Espaço com esse nome.» + `[ + Criar “{texto}” ]` com selo «PRO» p/ Free no limite (§5) | filtro substring (case/acento-insensível) sobre os nomes, client-side | — |
| Troca | — | `switch_to(name)` (`persistence_ui.rs:439`) → `WorkspaceFocusSet`; critério F1-4-4: <1 s, PIDs preservados | já fiado no modelo; falta o render gpui |
| `[ Renomear ]` | (§5) | `WorkspaceRenamed` (`events.rs:755`) no store do Espaço-alvo | evento JÁ existe ✓ |
| `[ Arquivar ]` + toast | «“{nome}” arquivado · [ Desfazer ]» (§5) | `WorkspaceArchived` (`events.rs:761`); lista padrão filtra `archived` | `[ Desfazer ]`/«Trazer de volta» exigem evento de desarquivar — **não existe** (§6-C2); desarquivar passa pelo gating (§3, brecha fechada) |
| «Espaços arquivados ▸» · «{nome} — arquivado em {data}» · `[ Trazer de volta ]` · vazio: «Nenhum Espaço arquivado…» (§5) | (§5, congeladas) | stores com `archived == true`; {data} = timestamp do `WorkspaceArchived` no log | §6-C2 |
| **Vazio: só um Espaço** | CTA «Crie um segundo Espaço para separar este projeto do resto — cada um com seu time.» + selo «PRO» quando Free (§1a do copy; ux-flows:765) | `active_count() == 1` (`workspace.rs:466`) + seam §3 | 3ª vitrine do gating |
| **Erro: pasta do Espaço sumiu** | linha com ⚠ «pasta não encontrada»; ao trocar: «A pasta deste Espaço não está acessível.» + `[ Localizar pasta… ]` `[ Voltar ]` (ux-flows:772 — reuso literal) | caminho do `WorkspaceEntry.events_dir`/raiz inacessível na abertura do M8 ou na troca | **REGRA desta spec:** `project_entry` (`persistence_ui.rs:164`) falhando **NÃO remove a linha** — vira linha-⚠ (hoje o código descarta a entrada, materializando o anti-padrão 6 "esconder é mentir", ux-flows:792). `rederive_entry` (`workspace.rs:478`) é o caminho de recuperação |
| **Erro: Espaço corrompido** | M8 não esconde; fluxo de recuperação W0-6 assume (ux-flows:773) | idem | anti-padrão 6 |

---

## §2 · M9 — Criar Espaço (Galeria de Focos + Diretório de Trabalho embutido)

**Decisão do fundador (2026-06-09, F1-4-1 escopo):** manter a Galeria de Focos COM seletor de diretório embutido — NÃO o modal simples do Maestri.

### Composição e fiação

1. **Cards de Foco** — reusa `FocusPreset`/`preset_team()` (`gallery.rs:41/:127`); presets no código hoje: `DevApp`, `ResearchContent`, `Blank` (o ux-flows:681 desenha 5 cards — divergência registrada em §6-B4). Nome pré-preenchido pelo Foco; **nome duplicado** segue o padrão do M6: sufixo automático «(2)» + aviso leve inline, não bloqueia (ux-flows:208 — reuso).
2. **Campo «Diretório de Trabalho»** (rótulo do fundador — F1-4-1 critério 6):
   - **Pré-preenchimento:** `Settings.default_cwd` (T7, `persistence_ui.rs:221`) como base se definido; senão `~/EspaçosDeTrabalho/{nome}` (ux-flows:681). Atualiza ao editar o nome **enquanto o usuário não tocou no campo** (depois de editado manualmente, nunca sobrescrever).
   - **`[ Procurar… ]`** → seletor de pasta NATIVO do SO (mesmo padrão do M6, ux-flows mapa de cliques #8).
   - **Validação na seleção e na criação** (cedo, nunca só no fim — navegação sem becos): (a) caminho canonicaliza e é diretório (se existir); (b) teste real de escrita (criar+remover arquivo temporário). Pasta sugerida inexistente NÃO é erro — o app a cria no caminho feliz.
3. **Criação** (`[[ Criar Espaço ]]`, ux-flows mapa #6) — sequência de eventos no store do NOVO Espaço:
   `WorkspaceCreated { focus_preset }` + `NodeAdded`/`NodeRoleAssigned` (via `apply_preset()`, `gallery.rs:147`) → `WorkspaceDefaultCwdSet { cwd }` (`events.rs:750`) → `WorkspaceFocusSet` (troca + aterrissagem povoada, nunca tela em branco — T4).
   A partir daí **todo novo terminal/agente nasce no `default_cwd`** via o seam único `resolve_spawn_cwd` (`workspace.rs:151`, ADR 0022) — a fiação do M9 **não** resolve cwd por conta própria, só grava o evento.

### Estados de erro do diretório (copy leiga)

| Quando | Mensagem | Saída | Origem |
|---|---|---|---|
| Caminho não acessível (é arquivo, drive ausente, caminho morto) | «Essa pasta não está acessível — escolha outra ou crie uma nova.» | `[ Procurar… ]` | **[NOVA]** — racional: herda a fraseologia congelada de pasta inacessível (ux-flows:772, «A pasta deste Espaço não está acessível.»), mas o contexto difere — lá é um Espaço EXISTENTE (saídas Localizar/Voltar); na CRIAÇÃO as saídas honestas são escolher outra ou criar uma nova |
| Sem permissão de escrita | «Não consigo trabalhar nessa pasta.» | `[ Escolher outra ]` | reuso literal do M6 (ux-flows:207) — mesma situação, mesma frase |
| Falha ao criar a pasta (disco cheio, permissão na hora) | «Não consegui criar a pasta do Espaço.» | `[ Escolher outro lugar ]` | reuso literal do M9 (ux-flows:774) |

---

## §3 · Upsell free=1 no fluxo de criação (reuso integral de `copy-f1-4.md §1`)

**O seam de gating JÁ EXISTE no core** (corrigido pela banca — não é decisão desta spec): `workspace::can_create_workspace(tier, active_count)` (`workspace.rs:101`; Free=1 via `workspace_limit`, `:92`; arquivado não conta) e o atalho `WorkspaceRegistry::can_create(tier)` (`:471`, sobre `active_count()` `:466` — o "grandfather" já está lá: arquivados liberam a vaga e Espaços existentes nunca são fechados, ux-flows:791).

**O que a rodada 3 fia (o que de fato falta):**
1. **Resolução do tier:** `lina-license` (F1-4-5) → `LicenseTier` (ausência de arquivo ⇒ `Free`; assinatura inválida ⇒ `Free` — fail-to-free). **Até o crate aterrissar:** o app resolve o tier explicitamente como `Pro` (não-gating deliberado, necessário para F1-4-1 crits 1-5 criarem A e B). ⚠ **O gate de saída da onda F1-4 ("2º Espaço bloqueado no free") só é avaliável com o stub SUBSTITUÍDO pelo fail-to-free real — nenhum build com o stub é candidato a release** (§6-B1).
2. **Consulta nos 4+1 pontos** (tabela abaixo) + UI das vitrines.
3. **Desarquivar passa pelo seam (brecha fechada pela banca):** sem isso, Free no limite faria arquivar → criar → «Trazer de volta» = 2 ativos sem chave. Regra: «Trazer de volta»/`WorkspaceUnarchived` consulta `can_create(tier)`; `Blocked` → mesma vitrine do §1a e o Espaço **segue arquivado e ABRÍVEL pela vista de arquivados** (coerente com o grandfather: arquivado nunca conta contra ABRIR, só contra voltar a ATIVO). Registrado em §6-C2.

| Ponto | Estado `Blocked` (strings — citação literal) |
|---|---|
| Vitrine 1 — item `+ Novo Espaço…` do M8 | selo «PRO» (§1a) — o item continua clicável (vitrine, não beco) |
| Vitrine 2 — `[ + Criar “{texto}” ]` da busca | selo «PRO» (§1a) |
| Vitrine 3 — M8 com um só Espaço | CTA congelado + selo «PRO» (§1a; ux-flows:765) |
| Vitrine 4 — card de criação no M9 | «Você já usa o Espaço do plano Free (1 de 1)» · `[ Conhecer o PRO ]` (§1a) |
| Destino do clique bloqueado (não é vitrine) | tela do bloqueio (§1b): «Seu segundo Espaço vem com o Lina PRO» + `[[ Já tenho uma chave ]]` (expande o campo §2 do copy ali mesmo) / `[ Quero o Lina PRO ↗ ]` + nota «A compra acontece no site…» / `[ Agora não ]` |
| Pós-ativação na tela do bloqueio | `[[ Continuar ]]` do sucesso (§2 do copy) volta ao M9 com a criação liberada — ≤3 interações (F1-4-6 critério 1) |

Regra herdada: o limite aparece **antes** do esforço nas 4 vitrines; nenhum caminho monta o Espaço para negar no fim.

---

## §4 · Checklist de a11y por tela (AccessKit — `a11y.rs`/`a11y_live.rs`; padrão W4-6/VoiceOver)

### M8 (dropdown/switcher)
- [x] ~~Abrir (`Cmd+O`/clique) põe o **foco no campo de busca** (ux-flows:670).~~ **EMENDADO na r5
  (aprovado pelo Arquiteto, 2026-06-12):** este critério ERA a causa-raiz do bug 4 da tela do
  fundador — o rail aberto com a busca focada SEQUESTRAVA toda digitação dos terminais. Novo
  contrato (`.entrega-sidebar-ux.md`): abrir o rail **não move o foco**; a busca só captura
  quando explicitamente focada (⌘F com o rail aberto, ou clique); `Esc` fecha o rail SÓ com o
  foco nele; `⌘O` ALTERNA (abre/recolhe) + chevron de mouse. O ux-flows:670 fica supersedido
  neste ponto — uso real do fundador vence o wireframe.
- [ ] `↑↓` navega as linhas (roving), `Enter` troca, `Esc` fecha **sem trocar**; `Cmd+1…9` funciona com o M8 fechado (índice estável, §1).
- [ ] Cada linha é UM elemento focável; aria-label = **concatenação dos fragmentos congelados do §5** (separadores tipográficos apenas): «{nome} · {N} Agentes — {Estado} · gasto estimado de hoje neste Espaço: {custo} · {N} Pedidos esperando você» — sem 🔔/custo, o fragmento é OMITIDO (não lido como "zero").
- [ ] Ordem de leitura = ordem visual; o estado NUNCA é só cor — a palavra viaja no tooltip/aria (regra do fluxo c, "consistência absoluta").
- [ ] Selo «PRO»: aria-label do item = «Novo Espaço — PRO» (concatenação dos dois rótulos visíveis; nada novo).
- [ ] Linha-⚠ (pasta sumida): o aria-label inclui «pasta não encontrada» (fragmento congelado de ux-flows:772).
- [ ] Toast de arquivamento anunciado via live-region (`a11y_live.rs`); `[ Desfazer ]` alcançável por teclado antes do timeout.

### M9 (modal de criação)
- [ ] Foco inicial no **1º card de Foco**; ordem de tab: cards → Nome → Diretório de Trabalho → `[ Procurar… ]` → `[[ Criar Espaço ]]` → `[ Cancelar ]` (ordem de leitura = ordem visual).
- [ ] Campos com label programático: «Nome» e «Diretório de Trabalho» (rótulo literal do fundador).
- [ ] Erros do diretório (§2) anunciados ao focar o campo (mesmo padrão do critério 5 de F1-4-6).
- [ ] Estado bloqueado (free=1): o card de upsell é focável e lê a string congelada inteira de §1a; `Esc` fecha o modal sem criar (nunca beco).
- [ ] `Enter` no formulário **submete sempre** (`[[ Criar Espaço ]]`). A precedência da Fila de Atenção vale SÓ para `Cmd+Enter` (gesto global de aprovação — ux-flows:140 e anti-padrão 5 do fluxo b); transplantá-la para o Enter simples criaria beco silencioso que a fonte não pede (correção da banca).

---

## §5 · Critérios de aceite da fiação (amarração com as stories)

1. F1-4-4 crit 1: trocar A↔B no M8 com terminais vivos → <1 s, PIDs preservados (`lina list --json` antes/depois), `WorkspaceFocusSet` no log.
2. F1-4-4 crit 2: criar do M8 → M9 (Galeria T3) → time nasce (`apply_preset`, mesmo contrato W4-5) — nunca tela em branco.
3. F1-4-4 crit 3: arquivar → some da lista padrão; «Espaços arquivados ▸» recupera; replay reproduz (o "Trazer de volta" depende de §6-C2).
4. F1-4-4 crit 4: percurso só-teclado (abrir, navegar, trocar, criar) gravado — checklist §4 completo.
5. F1-4-1 crit 6: Espaço criado no M9 com Diretório de Trabalho X → novo terminal E novo agente nascem com `TerminalSpawned.cwd == X` (via `resolve_spawn_cwd`); trocar o `default_cwd` afeta só nós futuros.
6. Vitrine free=1: com tier `Free` e 1 ativo, as 4 vitrines do §3 mostram o limite ANTES do clique; com `Pro`, nenhum selo aparece.
7. Honestidade do mini-status: linha sem dado NUNCA afirma dado — custo sem dado exibe «—» + tooltip com o fallback de `CostLine.text` (nunca «0,00» falso); Espaço de fundo sem contagem de 🔔 não exibe sino nem "em dia" (estado transitório até §6-C1 — o DoD final exige o 🔔 por Espaço). *(A moeda da célula de custo é PENDENTE-MAESTRO — §6-A1; este critério vale nas duas variantes.)*
8. Brecha de gating fechada: com tier `Free` e 1 ativo, a sequência arquivar → criar 2º → «Trazer de volta» no 1º resulta em `Blocked` com a vitrine do §1a — o 1º segue arquivado e abrível; nunca 2 ativos sem chave.

## §6 · Achados, emendas pendentes e pedidos de costura (o Maestro coordena)

**A — Conflitos copy×mecanismo (regra 8: registrados, NÃO decididos por esta spec — emendas de `copy-f1-4.md §5` pendentes, fora da minha fronteira nesta rodada):**
1. **Moeda:** copy congela «~R$ {valor}»; o produto emite `~US$` (`CostLine::from_today`, `dashboard.rs:212` — moeda real do provedor). Dado para a decisão: converter para R$ exigiria taxa de câmbio (rede = contra o local-first; taxa fixa = contra a honestidade do medidor). Duas variantes prontas na célula do §1.
2. **Zero de custo:** copy diz que «0,00» aparece; `CostLine` nunca exibe zero (subcontagem JSONL, `dashboard.rs:208`). Variante honesta fiada («—» + tooltip); emenda da célula pendente.
3. **Vocabulário 5×6 palavras:** copy/fluxo c fixam 5 palavras de estado; o código tem a 6ª «Encerrado» como "extensão registrada ao fluxo (c)" (`dashboard.rs:72`) e o caso todos-Encerrado do M8 precisa dela. Emenda: copy §5 decidir se o vocabulário do M8 é 5+1.

**B — Dependências, reduções de escopo e riscos:**
1. **`lina-license` não existe** → tier resolvido como `Pro` até F1-4-5 (não-gating deliberado). **Bloqueante de release:** o gate de saída da onda exige o fail-to-free real no lugar do stub.
2. `Dormindo` segue sem produtor (v1) — ramo fiado e dormente até o teto/B1.
3. **"Espaços recentes" (escopo F1-4-4) adiado:** ⌘n fiado por índice estável; ordenação/atalho por recência fica para depois (`last_focus` já existe no registry, `workspace.rs:457` — a porta está aberta). Dúvida #2 do ux-flows (colisões de atalhos) segue PENDENTE.
4. **Presets 3×5:** código tem `DevApp/ResearchContent/Blank`; ux-flows:681 desenha 5 cards com rótulos diferentes («Construir um App», «Criar Landing Page», «Automatizar Rotinas»…). Decisão/backlog do Maestro — rótulos de superfície são copy congelada.
5. **RISCO PERF/locks (lição W5):** abrir o M8 = projetar N stores (replay completo por `project_entry`) + varredura de sessões p/ custo — O(N×log) a cada abertura. Estratégia mínima na fiação: projetar 1× por abertura com cache e staleness declarada, `busy_timeout`+retry nas aberturas read-only (F1-4-1 crit 5), e MEDIR contra o crit 1 (<1 s).

**C — Costuras (arquivos alheios ou de dono único; NÃO editar sem o dono):**
1. **`crates/lina-core/src/attention.rs` (EXTERNO):** `workspace_id` aditivo (`serde(default)`) em `AttentionItem` — **pré-requisito INCONDICIONAL do DoD de F1-4-4** (mini-status inclui pendência; ux-flows:672 exige aprovar cross-Espaço sem trocar). Encaminhar via Maestro ao time externo. O **filtro por Espaço em `badge_view`** é trabalho INTERNO (em `attention_ui.rs` — atenção: arquivo com WIP não-commitado na árvore).
2. **`events.rs` (costura quente):** evento aditivo de desarquivar (proposta: `WorkspaceUnarchived`, espelho de `events.rs:761`) — sem ele, `[ Desfazer ]` e `[ Trazer de volta ]` não têm fiação; **com a regra de gating do §3.3** (desarquivar consulta o seam).
3. **`persistence_ui.rs` (interno, rodada 3):** estender `WorkspaceEntry` com os campos derivados do §1; `project_entry` deixa de DESCARTAR Espaço que não projeta (vira linha-⚠ — anti-padrão 6); convergir a lista do M8 para o `WorkspaceRegistry` do core como autoridade (varredura como fallback/recuperação via `rederive_entry`).

— fim da spec —
