# Entrega Dev 01 — Ondas F1-2 e F1-4

> **Papel:** SPEC de UX/Produto do Épico Fase 1 (doc 34, em construção). Detalho as ondas **F1-2 (Experiência)** e **F1-4 (Workspaces e PRO)**.
> **Fontes lidas:** repo `CLAUDE.md` · vault [[32 - Epico Fase 0 (MVP)]] (cabeçalho + Onda 4 inteira) · [[13.3 - Pesquisa F1 - UX Canvas e Design Systems]] · [[13.15 - Pesquisa F1 R2 - A11y Live-Region em Canvas]] · [[13.6 - Pesquisa F1 - Licenciamento Local-First]] · [[13.12 - Pesquisa F1 R2 - Maestri v0.29 Internals]] · [[01 - Visao de Produto e Norte de Continuidade]] §3/§4 · 4 prints do modal "Novo Terminal" do Maestri (2026-06-03) · `ls app/lina-gpui/src/` (módulos existentes: a11y, bridge, canvas, cost, creators, dev_tools, gallery, inspector, main, obsidian, onboarding, palette, persistence_ui, wiring) · doc irmão `tasks/epico-f1/ux-flows.md` (telas M6/M6-E/M8/M9/T7§A/T7§P — lido após publicação, ver fronteira abaixo).
> **Fio condutor honrado:** governança, observabilidade, gates de qualidade, anti-slop por arquitetura, local-first, não-técnico-first — cada story abaixo aponta para o que perdura.
> **Fronteira com o `ux-flows.md` (declarada):** este doc é autoritativo para **stories, escopo, critérios de aceite e gates**; o `ux-flows.md` é autoritativo para **wireframe, interação, entry points e copy** das telas correspondentes (F1-2-1→T7§A; F1-2-2/3→M6; F1-2-4→M6-E; F1-4-4→M8/M9; F1-4-6→T7§P). Divergências conhecidas entre os dois estão listadas em "Propostas de ajuste ao esqueleto" — nenhuma é silenciosa.

---

## Onda F1-2 — Experiência

**Objetivo da onda:** transformar o shell validado na Fase 0 numa experiência com identidade própria e qualidade de produto: design system tokenizado (dark/light + personalização), o melhor modal de criar/editar terminal da categoria (superando o Maestri v0.29 nos prints), edição de terminal vivo de primeira classe, superfície sem resíduos de dev e a11y consolidada com decisão registrada sobre o live-region.

**Gate de saída da onda (observável):** **validação na tela pelo fundador** — num percurso único gravado: criar um terminal pelo modal novo com ≤5 interações visíveis, editar nome/papel de um terminal **vivo** sem matá-lo, alternar dark/light e personalizar 1 cor que sobrevive ao restart, navegar o canvas só por teclado, e percorrer T0→T8 sem encontrar nenhum resíduo de UI de dev/teste. No CI: gate WCAG passando nos **2 temas**. Em `docs/adr/`: ADR do caminho do live-region registrado.

### F1-2-1 · Design system com tokens nomeados: dark/light + personalização local-first

**Por quê** (fonte): [[13.3]] achado 5 (🟢 confirmado) — Zed mantém dois temas default com `ColorScaleSet` (stepping consistente light→dark) e **200+ tokens nomeados**; GPUI Component centraliza `Theme`/`ThemeColor` structs ("nunca cores hard-coded"); personalização via `theme_overrides` + export JSON local. [[13.3]] recomendação **#1 [P0]**: "menor risco, maior alavancagem — começar por aqui". [[13.15]] achado 5: o gate WCAG de hoje cobre só a paleta de `a11y.rs` — `main.rs` e `persistence_ui.rs` têm `rgb(0x…)` inline que **escapam do gate** (furo apontado pelo red-team, ação #3 P1 do 13.15).

**Escopo:**
- **Entra:** módulo de tema no shell com `Theme` struct e tokens **nomeados e modulares** (grupos: superfície, texto, acento, estado, foco, terminal — não um monólito de consts soltas); tema **dark** (atual, migrado) + tema **light** novo; stepping consistente entre os dois (modelo `ColorScaleSet`); **migração de todo `rgb(0x…)` inline do shell para tokens** (fecha o furo do 13.15); `theme_overrides` do usuário (ex.: trocar o acento) persistido pelo mesmo canal de Ajustes T7 (eventos) + **export/import JSON 100% local**; troca de tema ao vivo (sem restart); ampliação do gate WCAG do CI para cobrir **os 2 temas e todos os tokens de texto** (≥4.5:1 AA normal, ≥3:1 large/muted — mantém o contrato de W4-6).
- **NÃO entra:** marketplace/temas de terceiros; editor visual completo de temas (o *right-click inspector* do Zed — "UX gold" no 13.3 — fica como semente P2/Fase 2); tokens de syntax highlighting (não temos editor); tematização do conteúdo do terminal além de BG/FG/cursor padrão.

**Critérios de aceite observáveis:**
1. Lint/teste no CI **falha** se um `rgb(0x…)` literal aparecer no shell fora do módulo de tema (prova de "proibir cores hard-coded").
2. Alternar dark↔light na tela atualiza todo o shell sem restart; o gate WCAG passa **nos dois temas** no CI e **falha** se um token de texto baixar de 4.5:1.
3. Usuário altera 1 token (ex.: acento) por UI simples, fecha e reabre o app → o override sobrevive (replay); export gera JSON legível, import o aplica.
4. Monitor de rede prova **zero** conexão em todo o fluxo de tema (local-first).

**Âncoras** (doc 01 §3 + invariantes): porta **`UiHost`** — tema é assunto exclusivo do shell, **nenhum token atravessa para o core** (`cargo tree -p lina-core` segue sem toolkit); o **gate WCAG é contrato herdado de W4-6** (a paleta light não pode regredi-lo — amarração do requisito "paleta light com gate WCAG" do brief); invariantes **#2 local-first** (override JSON local), **#6 não-técnico-first** (personalizar cor sem editar arquivo), **#7 core/shell split**. Superfície de Ajustes conforme **T7§A do ux-flows.md**.

**Tamanho:** M · **Dono sugerido:** dev01 · **Deps:** — (primeira da onda; destrava F1-2-2/6)

### F1-2-2 · Modal completo de criar/editar terminal — superar o Maestri

**Por quê** (fonte): os 4 prints do Maestri v0.29 (2026-06-03) mostram o alvo: modal "Novo Terminal" com Início Rápido (Claude Code/Codex/Gemini/OpenCode/Shell) + abas Detalhes (nome, comando, monitorar atividade, flag maestro, diretório), Aparência e Agente (responsabilidade = nome + **instruções YAML cruas em inglês** + ícone + cor, persistida em pasta `.maestri/` do projeto com aviso de `.gitignore`). [[13.3]] achado 4 (🟢) + recomendação **#2 [P0]**: modal de config em **3-5 controles, não 20 campos**; template-driven > blank canvas; progressive disclosure; e o **anti-padrão explícito** (rec. #8): "forçar entender YAML". [[13.12]] achado 3 (🟢 confirmado): persistência do Maestri é snapshot proprietário **sem replay** — nossa chance sólida de superar por arquitetura; achado 2 (🟡 incerto — a verificação **não encontrou fonte primária** sobre o grau de acoplamento do sistema de skills do Maestri): a hipótese de acoplamento ao runtime existe, mas não pode ancorar a diferenciação — o que ancoramos é o nosso lado confirmado (CLI Profiles TOML declarativos + eventos).

**Onde superamos o Maestri (tese da story):**
| Eixo | Maestri v0.29 (prints) | Lina |
|---|---|---|
| Superfície | YAML cru em inglês na aba Agente | **zero YAML/TOML visível**; papel em linguagem de leigo |
| Papel | busca manual em biblioteca | **co-piloto sugere pelo nome** (F1-2-3) com "por quê" |
| Persistência | pasta `.maestri/` no projeto + aviso de `.gitignore` | **event-sourced** em `.lina/` — recuperável de crash, sem sujeira nova no repo do usuário |
| Edição | modal só cria | **mesmo modal edita terminal vivo** (F1-2-4) |
| Quick-start | 5 CLIs fixos | cards **dos CLIs detectados** (`CliDiscovery`/W4-1) + "Instalar para mim" para ausentes |
| Comando | digitado à mão | **auto do CLI Profile TOML** — neutralidade, sem hard-code |

**Escopo:**
- **Entra:** modal único criar/editar com **3-5 controles visíveis**: (1) **Nome** (dispara o co-piloto de papel — F1-2-3); (2) **Início Rápido** de CLI — cards apenas dos CLIs detectados via `DiscoveryIndexed` + "Shell", com "Instalar para mim" embutido para CLI ausente (reusa W4-1, não reimplementa); (3) **Pasta de trabalho** (picker nativo); (4) botão **Criar**. *Progressive disclosure* ("Mais opções"): comando custom (pré-preenchido do profile TOML), cor/ícone do nó, papel manual. Modo edição: o mesmo modal abre a partir do Inspetor P4 / menu de contexto / Cmd+R com os valores atuais (a semântica de aplicar em nó **vivo** é F1-2-4). Toda mutação = evento (`NodeRenamed`/`CliProfileSet`/`NodeRoleAssigned`), commit só no Criar/Salvar.
- **NÃO entra:** o motor do co-piloto (F1-2-3); criação/edição de CLI Profile TOML pela UI (Fase 2); aba "Aparência" rica estilo Maestri (só cor/ícone básicos no disclosure — anti-slop: ganhar por menos, não por paridade); gestão/descoberta de skills (Arsenal — outra onda do esqueleto).

**Critérios de aceite observáveis:**
1. Um leigo cria um terminal "Revisor" a partir do canvas com **1 decisão real (o papel) + ≤4 cliques no caminho-feliz** (excluindo a digitação do nome) **em <30s** — métrica alinhada ao objetivo do M6 do `ux-flows.md`, contada na gravação; nasce nó-terminal com CLI real spawned e papel `reviewer` (verificável em `lina list --json` + env `VIBE_ROLE` — mesmo contrato do gate W4-2).
2. **Nenhum YAML/TOML/jargão é visível em nenhum estado do modal** (auditoria de copy na tela; o termo "profile" não aparece para o usuário).
3. Quick-start lista somente CLIs detectados; com um CLI ausente, o card oferece "Instalar para mim" e o fluxo W4-1 completa dentro do modal.
4. **Estado zero CLIs detectados** (o leigo virgem — exatamente o público do "1º agente <2h"): o modal nunca mostra quick-start vazio — apresenta o caminho único "instalar o motor recomendado" (estado vazio já desenhado no `ux-flows.md` §Estados vazios) e a gravação prova que um usuário sem nada instalado chega a um agente vivo sem beco.
5. **Nome vazio/só-espaços é rejeitado inline** com mensagem leiga antes do commit — nenhum evento entra no log com nome vazio e o rótulo AccessKit nunca fica vazio (complementa o caso "nome duplicado" já coberto no `ux-flows.md`).
6. `kill -9` no meio do preenchimento → reabrir não deixa nó fantasma nem estado corrompido (nenhum evento parcial no log).
7. O comando exibido em "Mais opções" vem do CLI Profile TOML do CLI escolhido (trocar o TOML muda o comando sem recompilar — prova de neutralidade).

**Âncoras** (doc 01 §3 + invariantes): porta **CLI Profiles TOML** (comando/install/resume vêm do profile; zero conhecimento de CLI no modal); porta **Event Store** (toda mutação é evento; nada de estado paralelo); porta **Bootstrap turno-0** (papel entra pelo canal de W3-2); invariantes **#3 neutralidade multi-CLI**, **#6 não-técnico-first**, **#1 sem LLM próprio**.

**Tamanho:** L · **Dono sugerido:** dev01 · **Deps:** F1-2-1 (tokens), F1-2-3 (co-piloto — o ciclo F1-2-2↔F1-2-3 é quebrado por um **contrato nomeado `RoleSuggester`** (trait no shell: nome → sugestão+explicação), sub-entregável de fronteira a definir **antes** de qualquer das duas iniciar; o modal consome o trait, o co-piloto o implementa), W4-1 (`CliDiscovery`), W4-2 (M2/P4), W3-1 (papel-pelo-nome). Wireframe/entry points/copy: **M6 do ux-flows.md**.

### F1-2-3 · Co-piloto de papel: sugestão pelo nome + biblioteca de papéis em linguagem de leigo

**Por quê** (fonte): [[13.3]] achado 4 + rec. #2: "co-piloto: sugerir papel/comando a partir de linguagem natural", template-driven (duplicar + modificar) em vez de blank canvas — "base da personalidade anti-AI-slop"; rec. #8: transparência — "sempre mostrar por que o agente sugeriu". O print 3 do Maestri mostra o custo de não ter isso: criar uma responsabilidade exige **escrever YAML técnico em inglês**. O substrato já existe: W3-1 (papel-pelo-nome, heurística do doc 21, fechada na Onda 3) — esta story é a **apresentação** dessa inteligência no modal, não um motor novo.

**Escopo:**
- **Entra:** ao digitar o nome no modal ("Revisora", "Designer de Landing"), a heurística **local** de W3-1 sugere papel + doutrina, exibidos como chip "Papel sugerido: Revisor · *por quê?*" (tooltip com a explicação em pt-br leigo); aceite com 1 clique, troca pelo seletor, ou nenhum papel; **biblioteca de papéis** pré-construídos com nome/descrição em linguagem de leigo (modelo duplicar + modificar); papel custom do usuário persiste como evento e reaparece nos próximos modais; a doutrina técnica completa fica **atrás** de "ver detalhes" (progressive disclosure — nunca obrigatória).
- **NÃO entra:** gerar doutrina via LLM embutido (**invariante #1** — a sugestão é heurística local + templates; refinamento por IA real só via CLI do próprio usuário, opt-in, em fase posterior); marketplace de papéis; editor YAML cru.

**Critérios de aceite observáveis:**
1. Digitar 3 nomes distintos (ex.: "Revisor", "Pesquisadora", "Faz-tudo") produz 3 sugestões distintas e coerentes com a heurística W3-1, cada uma com o "por quê" legível na tela.
2. Duplicar um template da biblioteca, modificar 1 campo e salvar → papel custom persiste (evento no log, sobrevive a replay) e aparece em um novo modal.
3. Tudo funciona **offline** (monitor de rede: zero conexão) e **sem nenhum LLM embutido** (auditoria de deps: nenhum client de API de LLM no app).
4. Sugestão **nunca** é aplicada silenciosamente: sem aceite explícito, o nó nasce sem papel (governança — humano decide).

**Âncoras** (doc 01 §3 + invariantes): porta **Bootstrap turno-0 + doutrina** (W3-2) — papel/doutrina entram pelo mesmo canal injetado, sem hard-code por agente; invariantes **#1 sem LLM próprio**, **#6 não-técnico-first**, **#2 local-first**.

**Tamanho:** M · **Dono sugerido:** dev01 · **Deps:** W3-1, W3-2; implementa o trait **`RoleSuggester`** (contrato de fronteira definido com F1-2-2) — desenvolvível em paralelo ao modal sem bloqueio mútuo

### F1-2-4 · Editar terminal vivo de primeira classe (polir e expor o Cmd+R)

**Por quê** (fonte): o renomear por **Cmd+R já existe** (contexto do Maestro — produtização da Fase 0); falta torná-lo **descobrível** (atalho oculto não é UX de leigo — invariante #6) e ampliá-lo para **função/papel** sem matar o terminal. O Maestri não edita responsabilidade de terminal vivo no fluxo dos prints (modal é de criação; print 4 só tem "Remover responsabilidade"), e [[13.12]] achado 4 (🟢) lista exatamente "delegação instável" e "execução agente→terminal não confiável" entre as reclamações reais da v0.29 — re-atribuição de papel em nó vivo é onde esses failure modes moram; fazê-la bem é diferencial observável.

**Escopo:**
- **Entra:** entradas descobríveis para a edição **herdadas do M6-E do `ux-flows.md`** (duplo-clique no cabeçalho do nó, Inspetor P4 → "Editar", paleta, clique-direito) **+ Cmd+R mantido** como atalho existente (já implementado na Fase 0 — removê-lo seria regressão; registrar no mapa de atalhos do ux-flows); renomear re-dispara a heurística de papel **com confirmação explícita** ("O novo nome sugere outro papel: X. Atualizar também?" — copy do M6-E; nunca troca silenciosa em terminal com trabalho em curso); ao confirmar, a doutrina nova é re-injetada **pela fila serial do nó** (MailQueue de W0-4) respeitando o freio de W4-3; eventos `NodeRenamed`/`NodeRoleAssigned` no log; rótulo AccessKit do nó atualizado.
- **NÃO entra:** trocar o CLI Profile/motor de um terminal vivo (exige respawn — o design certo já está no M6-E do ux-flows: "Trocar quando ele terminar" como default + selo "aplica ao reiniciar"; fica como story separada se o Maestro priorizar); mover terminal entre Espaços (F1-4); editar `cwd` de processo vivo (mesmo caso do respawn).

**Critérios de aceite observáveis:**
1. Renomear um terminal vivo → nome novo na tela + `NodeRenamed` no log + label AccessKit atualizado + **PID inalterado** (verificável por `lina list --json` antes/depois).
2. **Renomear para vazio/só-espaços é rejeitado inline** (mesma regra de F1-2-2 critério 5): nenhum `NodeRenamed` vazio entra no log, o co-piloto não dispara, e o rótulo AccessKit permanece o anterior.
3. Aceitar a mudança de papel → `NodeRoleAssigned` no log + doutrina re-injetada visível no terminal como mensagem A2A faseada (sem interleave com digitação humana — fila serial); recusar → nada muda.
4. Com o freio de W4-3 ativo, a re-injeção fica enfileirada (não injetada) até "Retomar" — verificável no log de eventos JSONL canônico (`.lina/events/log.jsonl`; o nome `bus.jsonl` da Fase 0 é legado).
5. `kill -9` após a edição → replay reconstrói nome/papel novos (T8 mostra o estado pós-edição).

**Âncoras** (doc 01 §3 + invariantes): porta **Workspace Bus/Supervisor** (re-injeção via fila serial, nunca canal ad-hoc); porta **Envelope A2A versionado** (re-injeção usa o contrato, não formato novo); porta **Event Store**; invariantes **#4 event log = verdade**, **#5 pertencimento = conexão**, **#6**.

**Tamanho:** M · **Dono sugerido:** dev01 · **Deps:** F1-2-2 (modal em modo edição), W3-1/W3-2, W4-3 (freio)

### F1-2-5 · Faxina: remover resíduos de UI de dev/teste da superfície

**Por quê** (fonte): invariante **#6** ("zero jargão na superfície") + o gate da onda é validação na tela pelo fundador — resíduos de dev (módulo `dev_tools.rs` presente no shell, labels técnicos, métricas cruas, UUIDs visíveis) quebram a confiança do leigo e são anti-slop reverso: slop nosso. [[13.3]] achado 2 (Warp): a superfície de produto comunica **estado e atenção**, não internals.

**Escopo:**
- **Entra:** inventário de todo elemento visível de dev/teste no shell (varredura por `dev_tools.rs` e irmãos + percurso de telas); para cada item, uma de três decisões: **remover**, **esconder atrás de flag** (`LINA_DEV=1`), ou **promover** a feature legível para leigo; revisão de copy das superfícies tocadas; teste que garante que o painel dev não monta sem a flag.
- **NÃO entra:** remover instrumentação/log interno (a observabilidade continua — só sai da **superfície**); redesenhar telas inteiras; tocar em módulos de peers sem inventário próprio (lição multi-terminal: validar pegada pelo próprio registro).

**Critérios de aceite observáveis:**
1. Build release padrão: percurso T0→T8 gravado **sem nenhum** elemento de dev/teste visível (checklist do inventário, item a item).
2. Com `LINA_DEV=1`, as ferramentas de dev voltam (não se perde capacidade de diagnóstico).
3. Teste automatizado falha se o painel dev montar sem a flag.
4. O fundador percorre as telas no gate da onda sem apontar resíduo (critério humano explícito).

**Âncoras** (doc 01 §3 + invariantes): invariante **#6**; **#7 core/shell split** (faxina só no shell; o core não tem superfície).

**Tamanho:** S · **Dono sugerido:** dev01 · **Deps:** — (independente; rodar **cedo** na onda para limpar o palco do gate)

### F1-2-6 · Canvas navegável por teclado: roving tabindex + atalhos + zoom-to-focus

**Por quê** (fonte): [[13.15]] achado 6 (🟡, não-bloqueador — decisão de design): o padrão WAI-ARIA APG de **roving tabindex** (`tabindex=0` no pai, `-1` nos filhos, setas navegam) é o que o Figma usa em canvas; "as ids únicas por terminal já existem (NodeId UUID), o que torna o roving tabindex **plug-and-play com o AccessKit — falta apenas um focus manager no canvas**"; ação #6 do 13.15 propõe exatamente esta story (cobre WCAG 2.1 critérios 2.1.1/2.1.3). Complementa a navegação Tab/Enter que W4-6 já entregou.

**Escopo:**
- **Entra:** **focus manager do canvas**: Tab entra no canvas como widget composto; setas navegam entre nós em ordem espacial estável e determinística; Enter entra no terminal focado (digitação), Esc volta ao nível canvas; atalhos de produtividade (novo agente, alternar para próximo terminal ocupado); zoom-to-focus ao navegar, **respeitando `reduce_motion_effective()`** (fonte única de W4-6/13.15 achado 5 — com reduce-motion, salto sem animação); estado de foco **visível** com token de focus ring do design system (contraste ≥3:1).
- **NÃO entra:** live-region/auto-anúncio (F1-2-7 decide o caminho); navegação dentro do grid do terminal (já existe); customização de atalhos pelo usuário (Fase 2); seleção múltipla por teclado.

**Critérios de aceite observáveis:**
1. Percurso **só-teclado gravado**: do boot, criar um agente, navegar entre 3+ nós com setas, entrar/sair de um terminal, abrir o Inspetor — zero mouse.
2. A árvore AccessKit reflete o foco (exatamente 1 nó do canvas focável por vez — roving), verificável por teste headless da ordem de navegação **+ validação na tela** (lição da Fase 0: visibilidade ≠ lógica — o teste headless sozinho não fecha o critério).
3. Com reduce-motion do SO ligado, o zoom-to-focus não anima (salto direto) — mesma fonte única que os pulsos A→B.
4. O focus ring é visível nos 2 temas (token do design system, contraste ≥3:1 — entra no gate WCAG de F1-2-1).

**Âncoras** (doc 01 §3 + invariantes): porta **`UiHost`** (focus manager vive no shell; o core não conhece foco); invariante **#6**; reusa a fonte única de reduce-motion (não cria segunda autoridade — espírito do invariante #4).

**Tamanho:** M · **Dono sugerido:** dev01 · **Deps:** F1-2-1 (token de foco); soma com W4-6 sem reimplementar

### F1-2-7 · ADR: o caminho do live-region — patch no gpui pinado × custom Element × rastrear upstream

**Por quê** (fonte): [[13.15]] inteiro — achado 1 (🟢): o AccessKit 0.24 pinado **já expõe** `set_live(Live::Polite/Assertive)`, API estável desde a 0.5.0 (2022); achado 2 (🟢): o gpui do nosso SHA (`09165c1`) **nunca chama** `set_live` em `write_a11y_info` (struct `Interactivity` tem 18 campos `aria_*`, nenhum `aria_live`) — logo `Role::Status` sozinho não vira live-region e leitores de tela **não auto-anunciam** "resposta pronta" sem foco; achado 3 (🔴 parcial): afirmar "conforme ARIA live-region spec" hoje é falso — a mecânica de coalescing 1×/turno está pronta e testada (`a11y.rs:79-131`), o que falta é exclusivamente a politeness ativa. Ação **#1 P0 do 13.15**: "registrar um ADR declarando que o auto-anúncio depende de `set_live` no gpui; plano acionável é custom Element ou patch no pinado". Esta story é esse ADR + spike que tira a decisão do papel.

**Escopo:**
- **Entra:** ADR em `docs/adr/` avaliando os **3 caminhos**: **(a)** custom `Element` que sobrescreve `write_a11y_info` → `node.set_live(Polite)` (sem tocar o fork; custo: implementar `request_layout`/`prepaint`/`paint` e tipos associados — 13.15 achado 2); **(b)** patch no gpui pinado expondo `.live()` no builder (custo permanente: re-aplicar a cada bump de SHA do vendoring; afeta a governança do pin do doc 33); **(c)** rastrear upstream (issue/PR em zed-industries + critério objetivo de re-avaliação por release). **Spike time-boxed (≤3 dias)** provando o caminho preferido: **VoiceOver anunciando 1 frase sem foco** no Mac. O ADR inclui critério de reversão e o impacto na porta gpui↔Slint.
- **NÃO entra:** implementação completa do auto-anúncio em todos os nós (vira story de implementação na onda seguinte/Fase 2, conforme a decisão); auditoria com leitor de tela nos 3 SOs (pertence ao fechamento de W4-6 — bloco de infra); `aria-busy` durante output (refinamento P2 do 13.15, citado como futuro no ADR).

**Critérios de aceite observáveis:**
1. ADR registrado em `docs/adr/` com as 3 opções, custos, decisão e critério de reversão — referenciando 13.15 e o comentário "GAP CONHECIDO (red-team)" em `a11y.rs` (hoje ≈ linhas 211-216; citar pelo símbolo/comentário, que é estável, não pela linha).
2. O spike demonstra **em gravação** o VoiceOver anunciando um texto **sem o nó estar focado** via o caminho escolhido, **ou** documenta com evidência técnica por que o caminho falhou (e o ADR registra o fallback).
3. Honestidade de comunicação (ação #2 P0 do 13.15): nenhuma doc/copy do produto afirma "conforme ARIA live-region" enquanto o spike não passar — auditável por grep nas docs.

**Âncoras** (doc 01 §3 + invariantes): porta **`UiHost`** + decisão de framework (doc 33) — um patch no pinado encarece a porta gpui↔Slint e o ADR deve precificar isso explicitamente; invariante **#6** (a11y é P0 de produto, não polimento).

**Tamanho:** S · **Dono sugerido:** arquiteto (ADR) + dev01 (spike) · **Deps:** — (decisão upstream de qualquer story futura de anúncio)

---

## Onda F1-4 — Workspaces e PRO

**Objetivo da onda:** transformar o workspace único provado na Fase 0 em **multi-Espaço de verdade** — cada Espaço é um projeto com seus terminais, sessões e config, tudo event-sourced e isolado por fronteira de confiança — e ligar a primeira alavanca de receita do produto: **licença local ed25519** com gating por número de workspaces (free=1, PRO=N), ativação não-técnica sem cadastro e emissão em lote para alunos. Tudo **100% offline** (o licenciamento local-first é diferencial deliberado, não padrão de mercado — [[13.6]] ação #2).

**Gate de saída da onda (observável):** **2 Espaços isolados + gating free/PRO 100% offline** — na tela: com licença PRO, criar um 2º Espaço com time distinto, alternar entre os dois com terminais vivos e provar isolamento (mensagem broadcast de A não aparece em B; log de eventos JSONL canônico de B limpo); com licença free (ou sem licença), o 2º Espaço é bloqueado com upsell honesto e saída clara; um monitor de rede prova **zero conexão** durante todo o ciclo (criação, gating, ativação de chave). `kill -9` com qualquer Espaço aberto recupera via T8.

### F1-4-1 · Multi-workspace de verdade: Espaço = projeto, tudo event-sourced

**Por quê** (fonte): a Fase 0 entregou o T6 Switcher (W4-4) sobre a projeção SQLite e `WorkspaceCreated` v2 com upcasting (W4-5) — mas um Espaço ainda não é uma unidade completa de projeto. [[13.12]] achado 9 (🟢): os *floors* do Maestri (workspaces independentes, git-like, navegação Cmd+#) confirmam a demanda do mercado; achado 3 (🟢): Maestri persiste por **snapshot proprietário sem replay** — "oportunidade clara para o Lina", cujo event-sourcing dá recuperação e auditabilidade que eles não têm. Doc 01 §1: presets/times por Espaço são o coração da visão ("estúdio de IA").

**Escopo:**
- **Entra:** Espaço como unidade real: terminais, sessões, notas, layout, config local (**`default_cwd` = DIRETÓRIO DE TRABALHO do Espaço**, freio W4-3, trust) **pertencem ao Espaço** e são deriváveis do log; **o `default_cwd` é escolhido na criação do Espaço (M9 — ver UX abaixo) e vira o cwd PADRÃO de todo novo terminal/agente daquele Espaço** (referência direta do fundador: o "Diretório de Trabalho" dos workspaces do Maestri; **decisão do fundador 2026-06-09: manter o estilo de criação do Lina — Galeria de Focos — COM um seletor de diretório de trabalho embutido (pré-preenchido pelo Foco, editável via "Procurar…"), NÃO o modal simples do Maestri**). ⚠️ **Interação com ADR 0022 (admissão canônica / fix de contexto `feb6adc`):** quando o Espaço tem `default_cwd`, novos terminais/agentes nascem NELE (compartilhamento INTENCIONAL — colaboram no mesmo projeto); o dir gerenciado virgem `n-<uuid>` (que isola contra herança de slot reciclado) é o **fallback quando NÃO há `default_cwd`**. O arquiteto resolve essa precedência ao construir a story (cwd resolve em UM seam: `default_cwd` do Espaço ativo → senão fallback isolado; o terminal PURO hoje já cai no `$HOME` do workspace único, esse seam é o gancho). **escopo de eventos por workspace** (decisão de layout de storage — log único particionado por `workspace_id` × um store por Espaço — registrada em **mini-ADR** dentro da story, com atenção à lição da Fase 0: aberturas concorrentes de SQLite WAL exigem busy_timeout + retry na troca de modo); **bus escopado**: roster, presença e A2A **nunca cruzam Espaços** (fronteira é a F1-4-2); registry global mínimo (`~/.lina/`) apontando os Espaços conhecidos; ciclo de vida: criar (cai no fluxo preset T3/W4-5), renomear, arquivar.
- **NÃO entra:** visão simultânea de 2 Espaços / overview 3D estilo floors; mover nós entre Espaços; Espaço git-backed por branch (referência Maestri, fica Fase 2+); sync/multiplayer cloud (doc 01 §2 — Futuro).

**Critérios de aceite observáveis:**
1. Criar Espaços A e B com times distintos → `lina list --json` escopado mostra **só** os nós do Espaço ativo.
2. Broadcast em A não chega a B: o log de eventos JSONL canônico de B (`.lina/events/log.jsonl` — nome `bus.jsonl` é legado) sem o evento (isolamento provado por log, não por ausência visual).
3. `kill -9` com A aberto → T8 restaura A íntegro; abrir B em seguida → B íntegro (nenhum vazamento de estado: buffers, layout, freio por Espaço).
4. Replay determinístico por Espaço: apagar projeções e re-derivar reproduz o estado de cada Espaço byte-a-byte (mesmo contrato de W0-5).
5. Teste de abertura concorrente dos stores (2 processos/2 Espaços) sem "database is locked" (regressão da lição W5).
6. **Diretório de trabalho do Espaço → cwd padrão (fundador 2026-06-09):** definido um `default_cwd` na criação do Espaço (M9), criar um **novo terminal OU um novo agente** o faz nascer **nesse diretório por padrão** (sem o usuário especificar caminho) — o `TerminalSpawned.cwd` do nó reflete o `default_cwd` do Espaço ativo; trocar o `default_cwd` afeta só nós FUTUROS (nós vivos não migram, ver F1-2-4 "editar cwd de processo vivo = NÃO entra"). Editar o `default_cwd` do Espaço persiste no log e re-deriva no replay. (Resolução de cwd é UM seam único; ver a Interação com ADR 0022 no Escopo.)

**Âncoras** (doc 01 §3 + invariantes): porta **Event Store + event-sourcing** ("o log é a fonte da verdade; **não criar tabelas-autoridade paralelas**" — o registry global é ponteiro, não autoridade); porta **Workspace Bus/Supervisor** (escopo por Espaço pendura no broker — não inventar canais ad-hoc); invariantes **#4**, **#5 pertencimento = conexão** (por Espaço), **#2 local-first**.

**Tamanho:** L · **Dono sugerido:** dev02 (core), com revisão do arquiteto · **Deps:** — (base da onda)

### F1-4-2 · ADR da porta PF-1: allow-list e fronteira de confiança por Espaço

**Por quê** (fonte): a porta **PF-1 do esqueleto do Épico F1** (allow-list por Espaço) exige decisão registrada **antes** de o multi-workspace abrir uma superfície nova de segurança. Base da Fase 0: o furo de InjectPolicy foi fechado com `WorkspaceTrust`/**ADR 0006** e o fan-out com **ADR 0007** ([[32]] status geral, HEAD `32d1d50`); W4-3 já entregou "confirmação humana para A2A sensível (allow-list por nó)". Multi-Espaço multiplica essas fronteiras: cada Espaço precisa do **seu** trust e da **sua** allow-list, e a fronteira **entre** Espaços é hostil por default. Regra do CLAUDE.md: decisão arquitetural → ADR curto, não impulso de story.

**Escopo:**
- **Entra:** ADR decidindo: (a) escopo do trust por Espaço (herdado ao reabrir; **nunca derivado de campo escrito por agente** — lição de segurança da Fase 0: campos forjáveis não decidem autorização); (b) allow-list de ações sensíveis **por Espaço** (o que exige humano), evoluindo a de W4-3 sem reabrir o buraco do ADR 0006; (c) regra cross-Espaço: **negar tudo por default**, qualquer exceção futura é opt-in explícito e auditável; (d) migração do `WorkspaceTrust` single-workspace para o modelo multi-Espaço; (e) **red-team próprio** do modelo (impersonação de Espaço, eventos forjados no log de B referenciando A).
- **NÃO entra:** UI de permissões granulares por agente (Fase 2); políticas além do que W4-3+ADR 0006 já cobrem dentro de um Espaço; qualquer canal cross-Espaço (não existe nesta fase).

**Critérios de aceite observáveis:**
1. ADR registrado em `docs/adr/` referenciando ADR 0006/0007 e a porta PF-1 do esqueleto, com decisão + red-team documentado.
2. Teste: Espaço novo nasce **untrusted** e só decisão humana explícita o promove (evento auditável no log).
3. Teste adversarial: nenhum campo controlável pelo agente (nome, env, conteúdo de mensagem, filename) altera trust/allow-list de Espaço algum.
4. Teste: tentativa de A2A cross-Espaço é negada por default e a **negação fica auditável** no log de segurança (sem múltiplos escritores no mesmo log sem prova de append concorrente — lição W5).

**Âncoras** (doc 01 §3 + invariantes): porta **PF-1** (esqueleto F1 — numeração a confirmar com o Maestro, ver Dúvidas); porta **Envelope A2A versionado** (autorização nunca deriva de campos forjáveis; `from` autenticado em camadas); invariantes **#2**, **#5**.

**Tamanho:** M · **Dono sugerido:** arquiteto (ADR) + dev02 (testes) · **Deps:** F1-4-1 (modelo de Espaço — ver Propostas de ajuste #2)

### F1-4-3 · Restore de terminais vivos no boot (follow-up conhecido da produtização)

**Por quê** (fonte): follow-up declarado da produtização da Fase 0 (contexto do Maestro): W4-4/W0-6 cobrem o boot **pós-crash** (T8, re-spawn em modo-retomada), mas o boot **normal** (quit limpo → reabrir) precisa devolver o Espaço como o usuário deixou — senão o produto "esquece" o time toda manhã, quebrando o invariante #6 ("estado sempre salvo e visível") e a promessa do doc 01 §1 ("nada se perdendo"). [[13.12]] achado 3: Maestri recarrega snapshot simples; nosso replay + scrollback em log append-only (W5-2: disco guarda **toda** linha; RAM é cache) permitem restore honesto e superior.

**Escopo:**
- **Entra:** ao abrir o app, o último Espaço focado volta com seus terminais **re-spawnados**: mesmas posições/nomes/papéis, scrollback re-hidratado do disco (cabo W5-2), doutrina re-injetada pelo bootstrap turno-0; **retomada de sessão do CLI quando o profile declarar suporte** (campo de resume no CLI Profile TOML — ex.: `claude --resume`; neutralidade: o verbo vem do TOML, nunca do código); **badge honesto por terminal**: "sessão retomada" vs "novo começo — o agente não lembra da conversa anterior" quando o CLI não suporta resume (**nunca mentir** — doc 20 §4.5 via W4-4: o processo vivo não é recuperável, e a UI nunca é silenciosa sobre isso); opt-out por Espaço ("abrir sem religar os terminais").
- **NÃO entra:** restaurar processo vivo entre boots (impossível por definição); emular resume para CLIs sem suporte; restore cross-máquina; reimplementar T8 (o caminho pós-crash continua sendo W0-6/W4-4 — esta story trata o quit limpo e **reusa** o mecanismo).

**Critérios de aceite observáveis:**
1. Quit limpo com 3 terminais (claude com sessão ativa, shell puro, CLI sem resume) → reabrir: 3 nós nas mesmas posições, scrollback visível idêntico ao pré-quit (≥ as N linhas da janela viva), e o claude **responde com contexto da sessão anterior** (prova de resume real na tela).
2. O terminal do CLI sem resume exibe o badge honesto de "novo começo" (copy auditada, sem jargão).
3. `kill -9` em vez de quit → T8 aparece e o resultado final é equivalente (mesmo mecanismo por baixo, apresentação distinta).
4. Trocar o verbo de resume no TOML muda o comportamento sem recompilar (prova de neutralidade).
5. Tudo derivável do log: limpar projeções → replay → mesmo restore.

**Âncoras** (doc 01 §3 + invariantes): porta **CLI Profiles TOML** (resume declarado por config — não fecha a porta da neutralidade); porta **Event Store**; porta **Bootstrap turno-0** (re-injeção pós-restore usa o mesmo canal); invariantes **#3**, **#4**, **#6** (honestidade do badge).

**Tamanho:** M · **Dono sugerido:** dev02 · **Deps:** F1-4-1; coordenar com a onda que tocar o bootstrap turno-0 (ver Dúvidas #4)

### F1-4-4 · Switcher de Espaços (T6) de produto

**Por quê** (fonte): o T6 da Fase 0 (W4-4) é uma lista mínima sobre a projeção SQLite; com multi-Espaço real ele vira superfície central de navegação. Referência de mercado: Maestri navega floors por Cmd+# **como feature Pro** ([[13.12]] achado 9) — o switcher é também vitrine do gating (F1-4-5). Invariante #6: nunca tela em branco — o switcher é o "lar dos lares".

**Escopo:**
- **Entra:** switcher redesenhado **conforme o M8 do `ux-flows.md`** — cada linha é um mini-status de governança: nome, **nº de agentes vivos** + estado dominante, custo do dia e pendência de atenção (nada fica esquecido num Espaço de fundo — visibilidade é requisito do fio condutor de observabilidade, não polish); criação inline (cai no fluxo de presets T3/W4-5 = M9), renomear/arquivar, atalho de troca rápida entre Espaços recentes; semântica de troca **proposta**: terminais do Espaço de fundo **continuam vivos** (processos preservados), o render/atenção segue o focado — mesma questão em aberto da Dúvida #1 do ux-flows (manter vs adormecer tem custo de RAM/CPU/dinheiro; decisão única do Maestro para os dois docs, ver Dúvidas #8); o freio (W4-3) é por Espaço; cards AccessKit focáveis + navegação por teclado (consistente com F1-2-6); slot de upsell quando o limite free bloquear criação (copy vem de F1-4-6, **antes do esforço** — anti-padrão "paywall surpresa" do ux-flows).
- **NÃO entra:** preview/thumbnail ao vivo dos Espaços (P2); visão simultânea; drag de nós entre Espaços; overview 3D.

**Critérios de aceite observáveis:**
1. Trocar A↔B com terminais vivos nos dois lados: troca em <1s, PIDs preservados (verificável por `lina list --json` antes/depois), `WorkspaceFocusSet` no log a cada troca.
2. Criar Espaço do switcher → cai na galeria de presets T3 e o time nasce (mesmo contrato de W4-5).
3. Arquivar → some da lista padrão; replay/“mostrar arquivados” o recupera (nada se perde — invariante #4).
4. Percurso só-teclado do switcher (abrir, navegar cards, trocar, criar) gravado.

**Âncoras** (doc 01 §3 + invariantes): porta **Event Store** (switcher lê projeção; muda estado só por evento); invariantes **#5** (roster segue o Espaço), **#6**.

**Tamanho:** M · **Dono sugerido:** dev01 · **Deps:** F1-4-1; integra F1-4-5/6 (gating no fluxo de criação)

### F1-4-5 · Licença local ed25519 + gating por número de workspaces (free=1, PRO=N)

**Por quê** (fonte): [[13.6]] é a fonte inteira desta story — ação **#1 P0**: "o `license.json` deve ser **assinado**, não um JSON editável (...) **sem assinatura, o feature-gating é teatro**"; ação #3: estrutura `{tier, workspace_limit, entitlements, expiry, last_validated, signature}` em `~/.lina/license.json`, chave pública Ed25519 **embarcada no binário**, "sem servidor de contas, sem cadastro"; ação #5: gating **por nº de workspaces** (`free=1`, `PRO=N`) — "mais simples e claro para o usuário não-técnico"; achado 1 (🔴): JetBrains/Sublime/Raycast fazem phone-home — offline puro é **diferencial deliberado** do Lina (ação #2: documentar como decisão); achado 5 (🟢): anti-pirataria pragmática — sem DRM, sem blocklist; quem cracka perde updates/comunidade.

**Escopo:**
- **Entra:** crate **`lina-license`** no core (sem UI): verificação de assinatura ed25519 com chave pública embarcada; parse dos entitlements data-driven (`workspace_limit` como o gate primário); ausência de arquivo ⇒ free (1 workspace); **falha de assinatura ⇒ trata como free** (degradação graciosa — nunca trava o app, [[13.6]] ação #4 adaptada: Espaços existentes além do limite continuam **abrindo**; o que bloqueia é **criar** novos); integração com F1-4-1 (checagem no fluxo de criação de Espaço); **zero chamada de rede em runtime** (a plataforma de pagamento é só canal de emissão — ação #8); formato com campo opcional `machine_id` **não validado** nesta fase (arquitetar para node-locking futuro sem implementá-lo — ação #9); suporte a `expiry` no formato (usado pelas chaves de aluno de F1-4-7).
- **NÃO entra:** node-locking ativo; telemetria de ativação; servidor de contas; revogação/blocklist; UI de ativação (F1-4-6); **decisão de pricing** (perpetual × subscription — decisão de negócio do fundador, ver Dúvidas #3; a story entrega o **mecanismo**, agnóstico ao modelo).

**Critérios de aceite observáveis:**
1. Com licença PRO válida: criar 2º+ Espaço funciona; sem licença: bloqueio gracioso com mensagem honesta (e nunca trava o app inteiro).
2. **Gating data-driven com N intermediário:** vetor de teste com licença `workspace_limit=3` permite criar até 3 Espaços e bloqueia o 4º com o mesmo upsell gracioso (prova o "PRO=N" do título, não só o binário free/PRO).
3. **Teste adversarial:** editar `tier`/`workspace_limit` à mão no `license.json` (sem re-assinar) → assinatura inválida → app trata como free ("teatro" detectado — prova da ação #1 do 13.6).
4. Chave assinada por keypair de terceiros não valida (vetores de teste com chaves conhecidas).
5. Monitor de rede prova **zero conexão** em todo o ciclo de licença (gate da onda).
6. Relógio do sistema retrocedido/avançado não invalida licença perpetual (sem expiry); licença com `expiry` vencida degrada conforme a regra graciosa.
7. **Expiry cruzado com o app aberto** (Espaço PRO em uso): o trabalho aberto **nunca é interrompido nem rebaixado no meio da sessão** — a licença é re-avaliada apenas nos **pontos de decisão de gating** (boot e criação de Espaço; nenhuma autoridade de relógio paralela durante a sessão); o usuário recebe aviso honesto **não-bloqueante** na primeira re-avaliação pós-expiry (família de copy do upsell de F1-4-6).
8. Espaço pré-existente além do limite **continua abrindo** após downgrade para free (nunca sequestrar trabalho do usuário — mesmo anti-padrão "não degradar o Free retroativamente" do ux-flows).

**Âncoras** (doc 01 §3 + invariantes): invariante **#2 local-first** (validação 100% local — diferencial documentado, não acidente); **#7 core/shell split** (verificação no core, apresentação no shell); decisão consciente: a licença **não** entra no event log do projeto (o log é por projeto e replicável; a licença é estado da máquina/usuário em `~/.lina/` — não cria tabela-autoridade paralela ao log porque não é estado de projeto).

**Tamanho:** M · **Dono sugerido:** dev02 (core; crypto sensível — revisão do arquiteto) · **Deps:** F1-4-1 (conceito de limite); F1-4-4/6 consomem

### F1-4-6 · UX de ativação não-técnica: colar a chave, sem cadastro

**Por quê** (fonte): [[13.6]] ação #7: "modal pós-download com campo *Cole sua chave de licença* → valida assinatura localmente → sucesso/erro; exibir `[tier, expira em, workspaces usados/limite]`"; achado 6 (🟢): fluxo compra → email com chave → colar → pronto, com a plataforma (LemonSqueezy/Paddle) **só como canal de emissão** — nunca chamada em runtime. Invariante #6: zero jargão; "sem cadastro" é o local-first virando UX visível (ação #2).

**Escopo:**
- **Entra:** duas superfícies de ativação: em **Ajustes T7** (painel de licença: plano, validade, Espaços usados/limite, trocar/remover chave) e **no momento do bloqueio** (ao tentar o 2º Espaço no free: tela de upsell honesta explicando o limite, com "Já tenho uma chave" → campo de colar, e "Quero o PRO" → abre a página de compra **no navegador padrão** — único toque externo, opt-in explícito por clique, o app em si não faz requisição); validação imediata ao colar com erros **acionáveis e leigos** ("essa chave está incompleta — confira o e-mail de compra"); sucesso desbloqueia **sem restart**; a11y: campo, erros e status legíveis por leitor de tela ao focar (consistente com o estado real de W4-6).
- **NÃO entra:** compra in-app; cadastro/login/conta; trial com countdown (decisão de negócio futura); deep-link de ativação automática (P2).

**Critérios de aceite observáveis:**
1. Do bloqueio ao desbloqueio em **≤3 interações** (colar → confirmar → criar o Espaço), sem restart — gravado na tela.
2. Chave inválida/incompleta exibe erro legível sem jargão (auditoria de copy) e **sem beco**: sempre há saída (voltar/tentar de novo — navegação sem becos de W4-4).
3. T7 mostra o estado real da licença (plano, limite, uso) consistente com `lina-license`.
4. Monitor de rede: zero requisição do app em todo o fluxo; o único evento externo é o navegador padrão abrindo por clique explícito.
5. Leitor de tela lê campo, erro e confirmação ao focar (VoiceOver no Mac nesta fase, coerente com o status de W4-6).

**Âncoras** (doc 01 §3 + invariantes): invariantes **#2** (exposição externa = opt-in sinalizado — aqui, um link), **#6**; padrão "navegação sem becos" herdado de W4-4.

**Tamanho:** S · **Dono sugerido:** dev01 · **Deps:** F1-4-5. Wireframe/copy: **T7§P do ux-flows.md** — **com a ressalva crítica da Proposta de ajuste #6**: os estados do T7§P que pressupõem rede/servidor ("confirmação online quando houver rede", "chave já ativa em outro computador", "chave revogada", "associada a este computador") **contradizem o gate 100% offline desta onda e o [[13.6]]** (sem phone-home, sem node-locking na F1, sem revogação) e precisam ser reconciliados antes da execução.

### F1-4-7 · Batch de chaves para alunos (emissão em lote, offline, sem master key)

**Por quê** (fonte): [[13.6]] ação #8: "para a distribuição em curso/comunidade: **batch de chaves únicas via CLI (CSV), sem master key**". Canal real do fundador (alunos/comunidade) que não passa pela plataforma de pagamento; achado 6 confirma o padrão de emissão custom. Pré-requisito de governança: a chave **privada** nunca chega perto do produto distribuído.

**Escopo:**
- **Entra:** ferramenta CLI **separada do app** (`lina-keygen`, nunca incluída no bundle): gera N chaves assinadas (`gen --count 50 --tier pro --label turma-7 [--expiry 12m]`) exportando CSV (chave, tier, validade, rótulo); cada chave é única (sem master key compartilhada — vazamento de uma não compromete o lote); doc de operação para o fundador: onde a privada vive (recomendação: keyring do SO via o mecanismo de `lina-secrets`/W0-7 na máquina do fundador, ou mídia offline), como rotacionar, como gerar; chaves de aluno usam o `expiry` do formato de F1-4-5.
- **NÃO entra:** portal web de resgate; revogação remota (local-first: não existe — dissuasão é a do 13.6 achado 5); integração com LMS/planilha de turma; envio de e-mail automatizado.

**Critérios de aceite observáveis:**
1. `lina-keygen gen --count 50 --tier pro --label turma-7` produz CSV com 50 chaves distintas; duas execuções não repetem chaves.
2. Teste E2E: 3 chaves amostradas do CSV ativam o app **offline** pelo fluxo de F1-4-6.
3. Chave do CSV adulterada (1 byte) falha na validação.
4. Auditoria: a chave privada não existe no repo nem no bundle (grep no artefato de build + regra de exclusão), e a doc de operação do fundador existe e foi revisada.

**Âncoras** (doc 01 §3 + invariantes): invariante **#2** (emissão e ativação 100% locais); reuso do padrão de segredos de W0-7 (keyring — "zero segredo em disco") para a guarda da privada na máquina do fundador.

**Tamanho:** S · **Dono sugerido:** dev02 · **Deps:** F1-4-5

---

## Riscos (transversais às duas ondas)

1. **Patch no gpui pinado (F1-2-7)** cria custo permanente: re-aplicar a cada bump do SHA vendorado e encarece a porta gpui↔Slint (doc 33). Mitigação: o ADR precifica os 3 caminhos e o spike prova antes de comprometer; preferir o custom Element se o custo for comparável.
2. **Multi-workspace × SQLite (F1-4-1):** N stores/aberturas concorrentes reativam a classe de bugs "database is locked" na troca de WAL vista na Fase 0. Mitigação: busy_timeout + retry bounded desde o 1º commit e teste de abertura concorrente no critério de aceite; perguntar sempre "quem mais escreve aqui?" antes de unificar logs.
3. **Gating client-side é dissuasão, não DRM** ([[13.6]] achados 2/5): aceitar pirataria casual como custo; não escalar proteção (ROI negativo). O risco real é o crack na 1ª semana de lançamento — a assinatura ed25519 mitiga o trivial; o resto é valor (updates/comunidade).
4. **Tentação de paridade com o Maestri no modal (F1-2-2):** replicar aba Aparência rica/YAML visível seria anti-slop reverso. Nosso ganho é **menos** superfície com mais inteligência (3-5 controles + co-piloto), não mais campos.
5. **Restore que mente (F1-4-3):** "sessão retomada" exibido para CLI sem resume destrói a confiança do leigo. O badge honesto é **requisito do critério de aceite**, não polish.
6. **Teste headless ≠ experiência (F1-2-6 e gate das ondas):** lição da Fase 0 — visibilidade ≠ lógica. Todo critério de UX tem componente "na tela/gravado", e o gate humano do fundador é parte do contrato.
7. **Co-piloto sugerindo papel errado para nomes ambíguos (F1-2-3):** sempre confirmação humana, nunca aplicação silenciosa; instrumentar a taxa de aceite da sugestão ([[13.3]] nota de calibragem: tratar como hipótese de design e **medir internamente**, não assumir).
8. **Numeração/definição da porta PF-1 (F1-4-2):** o doc 34 ainda não está no vault; o ADR não deve citar uma porta com texto divergente do esqueleto canônico (ver Dúvidas #1).

## Propostas de ajuste ao esqueleto

1. **F1-2-5 (faxina de dev/teste) deveria abrir a onda F1-2** — é S, independente, e limpa o palco para qualquer validação na tela; rodá-la por último faria o fundador revisar telas que ainda vão mudar.
2. **F1-4-2 (ADR PF-1) em duas passadas:** o ADR só fecha com o modelo de Espaço definido (F1-4-1), mas decidir a fronteira *depois* do código pronto inverte a regra "ADR antes da decisão". Proposta: **draft do ADR na entrada da onda** (assinado pelo arquiteto, com as invariantes de segurança fixadas) e **selo final** junto do fechamento de F1-4-1 — ou promover o draft a spike pré-onda.
3. **Registrar a decisão de pricing como decisão de negócio fora das stories** ([[13.6]] ação #6: escolher **um** modelo — recomendação perpetual; ambas as lentes adversariais concordam apenas em *não* fazer perpetual+subscription dual). Deadline sugerido: antes do copy final de F1-4-6 (a mecânica não muda; o texto do upsell sim).
4. **Explicitar no esqueleto a dependência cruzada** entre F1-4-3 (restore) e qualquer onda que toque o bootstrap turno-0/orquestração: a re-injeção pós-restore deve usar o mesmo canal de W3-2 — se outra onda evoluir esse canal, F1-4-3 consome a versão nova, não bifurca.
5. **Semente para Fase 2 a registrar no esqueleto** (não criar stories agora): right-click inspector de tokens estilo Zed (13.3 achado 5, "UX gold"); implementação completa do auto-anúncio conforme o ADR de F1-2-7; `aria-busy` durante output (13.15 ação #7); notificações Complete/Request/Error estilo Warp (13.3 rec. #3 — se já não estiver em outra onda da F1, ver Dúvidas #2).
6. **Reconciliar o T7§P do `ux-flows.md` com o gate offline desta onda (CONFLITO REAL entre specs irmãos):** o T7§P desenha "confirmação online quando houver rede" (E2), erros "chave já ativa em outro computador", "chave revogada", "sem internet (e validação exige rede)" (E4) e o selo "associada a este computador" (E3) — tudo isso pressupõe servidor de licenças/node-locking, contradizendo o gate da onda ("gating free/PRO **100% offline**"), o [[13.6]] (ações #1-#3: validação local pura como **diferencial deliberado**; #9: sem node-locking na F1) e o invariante #2. Proposta: cortar/reformular esses estados no ux-flows (manter só: chave incompleta, chave expirada, e os acertos excelentes de trim-ao-colar e duas-colunas honestas) — ou, se o fundador **quiser** ativação com confirmação online, isso é mudança de arquitetura de licenciamento que invalida parte do 13.6 e precisa de decisão explícita registrada.
7. **Divergência free=1 × "até 2 Espaços":** meu brief fixa **free=1 workspace** (e é o que o gate da onda testa); o T7§P/M9 do ux-flows desenha "✓ até 2 Espaços" no Free. O mecanismo de F1-4-5 é data-driven (qualquer N), então a engenharia não muda — mas o número precisa ser **um só** nos dois docs antes do copy congelar. Decisão do Maestro/fundador.

## Dúvidas para o Maestro

1. **Porta PF-1:** o doc 34 ainda não existe no vault — qual é o texto canônico da porta PF-1 (allow-list por Espaço) para o ADR de F1-4-2 citar com fidelidade? Existem outras portas PF-N que as minhas stories devam ancorar?
2. **Fronteiras com as outras ondas:** observabilidade de custo/tokens (módulo `cost.rs` já existe; 13.3 rec. #7 a marca como "diferencial PRO") e notificações estilo Warp pertencem a F1-1/F1-3? Mantive ambas **fora** do meu escopo para não colidir com dev02/llmeng — confirmar.
3. **Pricing (decisão do fundador):** perpetual ~$99 (recomendação 13.6) × subscription? Necessária antes do copy de F1-4-6; a mecânica de F1-4-5 é agnóstica.
4. **PRO = N workspaces:** N é ilimitado (modelo Maestri Pro, 13.12 achado 6) ou um teto alto? O formato de entitlements aceita os dois; o copy do upsell muda.
5. **Usuários beta com 2+ Espaços pré-gating:** ao ativar o gating free=1, assumo **grandfather de leitura** (Espaços existentes continuam abrindo; bloqueia-se só criar novos — critério 6 de F1-4-5). Confirmar essa política.
6. **Co-piloto e invariante #1:** confirmo a leitura de que a sugestão de papel é **heurística local W3-1 + templates** (sem LLM embutido), e que qualquer refinamento por IA real fica para fase posterior via CLI do próprio usuário (opt-in)?
7. **Aba "Aparência" do nó:** mantive só cor/ícone no progressive disclosure de F1-2-2. Se o fundador quiser identidade visual rica por agente (avatares/formas), vira story própria — decidir se entra na F1-2.
8. **Espaços de fundo: vivos ou adormecidos?** Mesma pergunta da Dúvida #1 do `ux-flows.md` — F1-4-4 propõe "continuam vivos" (PIDs preservados, troca <1s), mas isso tem custo de RAM/CPU/dinheiro com PRO=N. Pedimos **uma** decisão para os dois docs (e, se "vivos", o mini-status do M8 com custo por Espaço vira requisito, não polish).
9. **Glossário de superfície:** o `ux-flows.md` (Dúvida #9 dele) usa "Agente"/"motor de IA" onde meu brief diz "terminal"/"CLI". Minhas stories usam os termos do brief no texto técnico, mas os critérios de copy ("zero jargão") assumem que o rótulo final na tela virá do glossário do ux-flows. Confirmar que a lei da Fase 0 ("Agente", nunca "terminal") segue valendo na F1 — aí os rótulos das minhas telas herdam de lá automaticamente.

---

PRONTO: 14 stories detalhadas (7 na F1-2 — design system, modal que supera o Maestri, co-piloto de papel, edição viva, faxina, teclado no canvas, ADR live-region; 7 na F1-4 — multi-Espaço event-sourced, ADR PF-1, restore no boot, switcher, licença ed25519, ativação leiga, batch de alunos), revisadas adversarialmente em 4 lentes (citações/completude/produto/técnica — 12 achados triados e incorporados, incluindo a reconciliação com o `ux-flows.md` e 2 conflitos reais entre specs irmãos escalados ao Maestro: free=1×"até 2" e T7§P online×gate 100% offline), com gates observáveis por onda, âncoras do doc 01 §3, riscos, 7 propostas de ajuste e 9 dúvidas — em `tasks/epico-f1/ondas-2-4.md`.
