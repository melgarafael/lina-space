# Diagnóstico 360° — Admissão de nó (⌘N "Novo Agente" como cidadão de 2ª classe)

> **Origem:** diagnóstico orquestrado (7 auditores read-only em paralelo + síntese), sessão do Maestro de 2026-06-07 noite (workflow `wf_f7a2b27a`). A sessão do Maestro morreu por erro de AUP da API logo após esta síntese fechar — este doc é a versão durável. Findings brutos dos 6 auditores preservados no JSON de origem (`/private/tmp/claude-501/-Users-rafaelmelgaco-einstein-workspace/219b81b6-82e8-472a-8009-87189dd7220d/tasks/w8eubrqi2.output` — copie antes de reboot se precisar).
> **Gatilho:** direção estratégica do fundador (22:02): o "Novo Agente" (⌘N) não aparece em custo/atividade, vazou skill/env — hipótese de componente separado que pula o pipeline canônico.
> **Status:** AGUARDANDO rodada de correção. R2c-2 (guard de skill estrangeira, `7b2bff6`) já commitado e deve entrar no MESMO reempacote que os fixes daqui.

## ROOT CAUSE

A admissão de um terminal no Lina não existe como UMA função — são três implementações paralelas escritas em épocas diferentes (seed_one main.rs:3261, add_node bridge.rs:2595, create_agent_with bridge.rs:2704), cada uma persistindo um subconjunto diferente do contrato de admissão (papel, motor, doutrina, hooks, skill A2A), enquanto cada consumidor downstream correlaciona por uma CHAVE diferente (custo por cwd-da-sessão, atividade por nome+token, teto por UUID, bootstrap por key t{seq}). A chave mais crítica — o cwd REAL do spawn — nunca é persistida no event log (TerminalSpawned não tem campo cwd; o mapa keys aponta para <ws_root>/t{seq} mesmo quando o agente roda na pasta do usuário), então o ramo "pasta própria" do ⌘N desliga silenciosamente todos os consumidores chaveados por cwd: custo, doutrina, skills e observabilidade.

## ARQUITETURA EXPLICADA (para o fundador, sem jargão)

Imagine que o seu Espaço é um prédio com TRÊS portarias diferentes, construídas em momentos diferentes, cada uma com a sua própria ficha de cadastro incompleta — e não existe uma recepção central. A portaria antiga (⌘T / botão +) cadastra o funcionário só com nome, sem cargo nem crachá de ponto: por isso 31 dos 36 agentes do seu log histórico não têm papel nem motor registrados — incluindo o Maestro (759 mil tokens) e o Teste (201 mil), que são exatamente os que você queria ver no painel de custos. A portaria nova (⌘N) é a MAIS completa no cadastro — registra nome, cargo E motor — então a sua hipótese está certa no efeito, mas invertida num detalhe importante: o ⌘N não "pula o pipeline canônico", porque NÃO EXISTE pipeline canônico; ele é a terceira cópia paralela, e por acaso a mais rica. O que isola o agente do ⌘N é outra coisa: quando você escolhe uma PASTA PRÓPRIA no modal, o código pula de propósito a entrega do "kit de integração" (o CLAUDE.md com a doutrina, a skill de comunicação entre agentes e o arquivo de settings que liga a observabilidade) — há até um comentário no código dizendo "SEM CLAUDE.md aqui" — e ainda escreve esse kit numa pasta-fantasma t{N} que o agente nunca lê. E o dinheiro do painel tem um segundo problema, independente do ⌘N: o número "Hoje:" NÃO vem dos 17.608 eventos de tokens do seu log (esses alimentam só o freio de teto interno); vem dos arquivos de sessão do Claude Code, filtrados por "a sessão aconteceu DENTRO da pasta do Espaço?". Agente em pasta própria = sessão fora do Espaço = dinheiro invisível. Sua conclusão final está certa: enquanto existirem três portarias com fichas diferentes, cada nova forma de criar agente vai esquecer alguma obrigação e nascer um novo bug desta mesma família.

## DIMENSÕES AUDITADAS

### [OK] Identidade (NodeId)

Nenhum. Todos os três caminhos passam por wire_terminal → Supervisor::register, que é a autoridade única do UUID v7.

**Evidência:** crates/lina-core/src/lib.rs:1149 (Uuid::now_v7 no register); app/lina-gpui/src/bridge.rs:1453-1467 (wire_terminal usado por add_node:2627, create_agent_with:2766 e seed_one main.rs:3270)

### [QUEBRADO] Custo (painel 'Atividade e custos do time')

O dinheiro do painel NUNCA lê TokenUsageReported (que alimenta só o teto do core, router.rs:1842): vem de workspace_cost_today sobre arquivos de sessão JSONL do SessionWatch, filtrados por cwd.starts_with(ws_root). Agente ⌘N em pasta própria → sessão fora do prefixo → excluída do total. E o custo POR NÓ (build_cards + hints) é código morto: render_dashboard nunca monta hints — nenhum nó, de nenhum caminho, tem custo individual na tela.

**Evidência:** app/lina-gpui/src/main.rs:902 (workspace_cost_today com dash.ws_root); app/lina-gpui/src/dashboard.rs:296-305 (filtro starts_with(ws_root)); main.rs:895-1046 (render_dashboard sem hints/build_cards); crates: router.rs:1842-1881 (único consumidor de TokenUsageReported)

### [PARCIAL] Atividade (timeline de hooks)

O ponto de estado (GridDelta) funciona para todos. Mas a linha 'rodando: X' depende do settings.json com token de hook escrito no cwd do agente via write_one. Com pasta própria, esse settings nunca chega ao cwd real (pulado no spawn; o rewrite_bootstrap escreve no dir-fantasma t{seq}) → CLI roda sem hooks → atividade eternamente '—'.

**Evidência:** bridge.rs:2241-2257 (write_one + HookWiring token via dir_for); bridge.rs:2744-2749 (ramo cwd=Some pula write_one); bridge.rs:2224-2226 (dir_for = ws_root.join(key), nunca a pasta do usuário); main.rs:933-939 (activity por tl.activity(&view.name))

### [QUEBRADO] Skills (A2A turno-0)

install_agent_bus_skill só roda dentro de write_one. ⌘N com pasta própria: pulado no spawn e desviado para a pasta-fantasma no rewrite_bootstrap → o agente nasce sem a skill lina-agent-bus no cwd real → não participa da auto-orquestração no turno-0.

**Evidência:** bridge.rs:2266-2269 (install_agent_bus_skill dentro de write_one); bridge.rs:2744-2761 (skip quando cwd=Some); bridge.rs:2428-2432 + 2275-2279 (rewrite_bootstrap → rewrite_all → write_one no dir_for, não no cwd real)

### [QUEBRADO] Contexto (doutrina CLAUDE.md)

Intencional-mas-silencioso: comentário 'SEM CLAUDE.md aqui' (bridge.rs:2745) e doc 2699-2701 adiando para F1-2-4/W3-2. O agente em pasta própria nasce sem doutrina, sem roster, sem vault — e o rewrite_bootstrap ainda cria um <ws_root>/t{seq} órfão com a doutrina completa que ninguém lê (efeito colateral: tokens de hook re-registrados apontando para o nada).

**Evidência:** bridge.rs:2744-2749 (ramo Some sem write_one + comentário); bridge.rs:2699-2701 (adiamento documentado); bridge.rs:2840 (rewrite_bootstrap pós-criação escreve via dir_for no dir gerenciado)

### [PARCIAL] Estado (Ocioso vs Blocked)

Duas fontes vivas de estado sem reconciliação: agents.json é escrito do roster do Supervisor (sup.list), enquanto as linhas do painel leem NodeView.status do model alimentado pelo pump — podem divergir por janelas de timing. Agrava: sup.register acontece ANTES do append de NodeAdded (bridge.rs:2766 vs 2782) — o nó existe no roster vivo antes de existir no log.

**Evidência:** bridge.rs:53 + 540-555 (refresh_agents via sup.list); main.rs:914-932 (painel via view.status do NodeView); bridge.rs:2764-2777 (wire antes) vs 2780-2823 (persistência depois)

### [QUEBRADO] Log de admissão / Event stores

Cardinalidade de eventos diverge por porta de entrada: add_node/seed emitem só NodeAdded+TerminalSpawned (papel hardcoded 'terminal' fora do log); ⌘N emite 4-5 eventos. Resultado no log vivo: 36 NodeAdded mas só 5 NodeRoleAssigned/CliProfileSet — os 2 nós de maior gasto (759k e 201k tokens) não têm papel nem motor no log, irrecuperáveis por replay. Além disso há dois stores no disco: .lina/events (17.817 linhas) vs onboarding/events (19 linhas, parado desde 06-04).

**Evidência:** Verificado no log vivo (~/Library/Application Support/Lina/walking-skeleton): 36 NodeAdded, 5 NodeRoleAssigned, 5 CliProfileSet, 17.608 TokenUsageReported; nó 019e95e0-7666=759.004 tokens e 019e95e0-995a=201.506, ambos sem role-event; bridge.rs:2643-2653 (add_node só 2 eventos) vs 2782-2821 (⌘N 4-5); onboarding.rs:386 (store separado)

## QUICK WINS (baratos, podem entrar já)

- add_node (⌘T / botão +) passa a emitir também NodeRoleAssigned('terminal') + CliProfileSet(profile default): ~10 linhas reaproveitando o bloco do create_agent_with — fecha, para nós novos, a classe dos 31-sem-papel que deixou Maestro (759k tokens) e Teste (201k) órfãos no log.
- Campo aditivo `cwd` no payload do TerminalSpawned (serde(default)): barato, não quebra replay antigo, e é o pré-requisito de quase todo o resto — sem ele a correlação nó↔sessão é irreconstituível para sempre.
- ⌘N com pasta própria: instalar pelo menos a skill lina-agent-bus + settings de hooks no cwd real (são dirs .claude/ aditivos, menos invasivos que o CLAUDE.md) — devolve A2A turno-0 e timeline de atividade sem decidir ainda a política do CLAUDE.md.
- Parar de criar o dir-fantasma: rewrite_bootstrap pular keys cujo cwd real difere do dir_for(key) (guard de 3 linhas até o mapa node→cwd existir) — elimina os <ws_root>/t{seq} órfãos com doutrina e tokens de hook que apontam para o nada.
- Honestidade no painel: quando houver TokenUsageReported recente de um nó sem sessão correlacionada, trocar 'sem estimativa' por 'trabalhando fora da pasta do Espaço — custo não rastreável aqui' (o dado para distinguir já existe no log).
- Unificar o atalho na superfície: o empty-state ensina '⌘T' (caminho pobre em metadados) enquanto o atalho canônico documentado é ⌘N (modal completo) — apontar o aluno para UMA porta, a rica, até o admit_node igualar as duas.

## REFACTOR ESTRUTURAL (a correção de forma)

UM funil de admissão: criar NodeManager::admit_node(NodeAdmission) como o ÚNICO lugar do app que transforma a intenção 'quero um terminal' em nó vivo — encapsulando validação, alocação de identidade, política de cwd (gerenciado OU pasta do usuário, com kit de integração entregue no cwd REAL ou degradação explícita e visível), a sequência canônica e COMPLETA de eventos (NodeAdded + TerminalSpawned com cwd + NodeRoleAssigned sempre + CliProfileSet quando há profile), a ordem persiste-antes-de-projetar com compensação de falha, e a projeção atômica (model/grids/keys/rewrite). Os três entry points atuais (seed_one do demo, add_node do ⌘T/botão, create_agent_with do ⌘N) viram tradutores finos de intenção → NodeAdmission, proibidos de apendar eventos ou tocar o Supervisor diretamente; um teste de paridade garante que qualquer porta produz a mesma sequência de eventos módulo parâmetros. Isso fecha a classe inteira do bug: hoje cada nova forma de criar agente (a próxima será API, CLI ou a inteligência da Lina) precisa LEMBRAR de N obrigações implícitas espalhadas; depois, ela não tem como esquecer, porque a obrigação mora no único caminho que existe — exatamente o padrão que o próprio código já consagrou para a identidade (Supervisor.register como autoridade única do NodeId, a única dimensão hoje OK), estendido para todas as outras dimensões da admissão.

## FIX BLUEPRINT (8 passos, com dono e risco)

### Passo 1: ADR curto: 'Admissão canônica de nó' — define o contrato único de admissão (sequência completa de eventos, binding node↔cwd persistido, política de kit de integração por tipo de pasta). O CLAUDE.md do projeto exige ADR antes de mudança de forma — e esta é uma.

- **Arquivos:** docs/adr/ (novo)
- **Dono sugerido:** Maestro (decisão arquitetural, gate antes de qualquer worker tocar código)
- **Risco:** Baixo — só documento; destrava os passos seguintes com critério.

### Passo 2: Criar NodeAdmission (plano) + NodeManager::admit_node(plan) em bridge.rs: validação de nome, seq/key, política de cwd (Managed(key) | UserDir(path)), bootstrap, wire_terminal, sequência CANÔNICA de eventos (NodeAdded + TerminalSpawned + NodeRoleAssigned SEMPRE — 'terminal' também é papel — + CliProfileSet sempre que o profile é conhecido), projeção atômica e rewrite. Toda a disciplina persiste-antes-de-projetar/retire_pty em UM lugar.

- **Arquivos:** app/lina-gpui/src/bridge.rs
- **Dono sugerido:** Worker do app (bridge.rs é território do app; um dono só nesta rodada)
- **Risco:** Médio — é o coração da criação de nós; mitigar reaproveitando os testes existentes (add_node_grows_count_persists_and_reconstructs, bridge.rs:4415) e adicionando teste de paridade: os 3 fluxos produzem a MESMA sequência de eventos módulo parâmetros.

### Passo 3: Redirecionar os três entry points para admit_node: add_node() vira admit_node(NodeAdmission::default_terminal()); create_agent_with() vira tradutor do CreatePlan do modal para NodeAdmission; seed_one (demo) vira admit_node com flag de seed. Nenhum entry point apenda evento diretamente nunca mais.

- **Arquivos:** app/lina-gpui/src/bridge.rs (add_node, create_agent_with), app/lina-gpui/src/main.rs (seed_one ~3261, modal_commit ~1245)
- **Dono sugerido:** Mesmo worker do passo anterior (mesma fatia, mesma fronteira)
- **Risco:** Médio — regressão em três fluxos de UI; validar com o teatro do fundador no dist/Lina.app reempacotado (lição registrada: o fundador testa o .app, não o dev build).

### Passo 4: Persistir o cwd REAL do spawn: campo aditivo `cwd` no TerminalSpawned (serde(default) — replay de log antigo não quebra, doutrina de eventos aditivos do CLAUDE.md). Isso dá a TODA projeção futura o binding node↔cwd↔sessão que hoje não existe em lugar nenhum do log.

- **Arquivos:** crates/lina-core/src/events.rs (COSTURA — dono único por rodada, pedir ao Maestro)
- **Dono sugerido:** Dono da costura events.rs designado pelo Maestro (protocolo multi-terminal do projeto)
- **Risco:** Baixo — aditivo por construção; risco residual é call-sites de teste que constroem o evento (atualizar com autorização, lição 'fidelidade > contorno').

### Passo 5: Política de pasta do usuário DENTRO do admit_node: UserDir(path) escreve o kit de integração (CLAUDE.md + settings/hooks + skill lina-agent-bus) no cwd REAL com consentimento explícito no modal ('a Lina vai criar arquivos de orquestração nesta pasta') OU degrada VISIVELMENTE (badge 'agente sem doutrina/observabilidade' no card) — nunca silencioso. rewrite_bootstrap passa a iterar o mapa node→cwd-real, eliminando o dir-fantasma t{seq}.

- **Arquivos:** app/lina-gpui/src/bridge.rs (write_one/rewrite_all/dir_for, admit_node), app/lina-gpui/src/agent_modal.rs (consent no M6)
- **Dono sugerido:** Worker do app + revisão do fundador no copy do consentimento (escrever na pasta do usuário é decisão de produto, gate humano)
- **Risco:** Médio-alto — escrever em pasta do usuário é sensível (sobrescrita de CLAUDE.md pré-existente do usuário!); exigir merge-safe/append ou recusa com aviso, jamais overwrite cego.

### Passo 6: Fechar a race roster-antes-do-log: dentro do admit_node, persistir NodeAdded antes (ou compensar o register no Supervisor em falha de persistência), para o nó nunca existir no roster vivo (agents.json, AppPermissionWatch) sem existir no log.

- **Arquivos:** app/lina-gpui/src/bridge.rs (ordem wire_terminal vs appends, hoje 2766 vs 2782)
- **Dono sugerido:** Mesmo worker do admit_node
- **Risco:** Médio — inverter a ordem muda o tratamento de falha de spawn; o NodeId nasce no register, então a alternativa prática é a compensação (unregister em falha de append), já meio-existente via retire_pty.

### Passo 7: Religar o custo por nó no painel: projetar node↔cwd do log (agora que TerminalSpawned carrega cwd) e ligar o build_cards/hints que hoje é código morto; no total 'Hoje:', distinguir 'sem sessão hoje' de 'agente trabalhando fora da pasta do Espaço' (e somar opt-in as sessões dos cwds adotados pelos nós do Espaço, não só prefixo de ws_root).

- **Arquivos:** app/lina-gpui/src/main.rs (render_dashboard ~895-1046), app/lina-gpui/src/dashboard.rs (build_cards/workspace_cost_today)
- **Dono sugerido:** Worker de dashboard/observabilidade (fronteira separável do admit_node)
- **Risco:** Baixo-médio — build_cards já existe e é testado; o risco é UX de atribuição ambígua (2+ nós no mesmo cwd), que a doutrina 'nunca chutar' do próprio dashboard.rs já resolve.

### Passo 8: Demarcar os dois event stores: declarar .lina/events como única fonte da verdade do Espaço e o onboarding/events como scratch descartável do fluxo de onboarding (ou migrar WorkspaceCreated/DiscoveryIndexed para o store principal no fechamento do onboarding). Documentar no ADR do passo 1.

- **Arquivos:** app/lina-gpui/src/onboarding.rs (~386), docs/adr/
- **Dono sugerido:** Maestro decide no ADR; implementação trivial por qualquer worker
- **Risco:** Baixo — hoje já são desacoplados; o fix é tornar a fronteira intencional em vez de acidental.

## CONTRADIÇÕES ENTRE AUDITORES (arbitradas na síntese — não re-litigar)

- Auditor 1 (SPAWN) vs Auditores 2/4 sobre bootstrap no create_agent_with: o Auditor 1 afirma que o ⌘N 'chama ensure_cwd que cria o dir E chama bootstrap.write_one' e confirma 'ambos encaixam no bootstrap'. O código mostra que isso vale SÓ para o ramo cwd=None; com pasta escolhida no modal, write_one é pulado por inteiro (bridge.rs:2744-2761, comentário 'SEM CLAUDE.md aqui'). O Auditor 4 está certo; o Auditor 1 não viu o condicional.
- Auditor 2 (CUSTO) vs o código e vs Auditor 5: o Auditor 2 alega que 'lock(&self.keys).insert(node, key) NUNCA executa porque key não existe' com cwd customizado. FALSO: a key t{seq} é computada incondicionalmente (bridge.rs:2730) e inserida incondicionalmente (2839) — o próprio Auditor 5 confirma que o nó ⌘N 'está no model/grids/keys'. O gap real de custo é outro: a key aponta para o dir gerenciado <ws_root>/t{seq} enquanto o agente roda na pasta do usuário, e render_dashboard nem usa hints/build_cards (código morto no painel).
- Auditor 2 vs Auditor 3 sobre o 'sem estimativa': o Auditor 2 culpa hints/keys ausentes e diz que o SessionWatch 'nunca detecta' sessões de cwd customizado. O Auditor 3 (na open question) suspeita corretamente que o painel usa SessionWatch e não TokenUsageReported. O código confirma o Auditor 3: o glob é global (~/.claude/projects/*/*.jsonl — o watch VÊ todas as sessões, inclusive de pasta própria); a exclusão acontece no filtro cwd.starts_with(ws_root) de workspace_cost_today (dashboard.rs:301). Os 17.608 TokenUsageReported nunca foram lidos pelo painel — alimentam só o teto do core (router.rs:1842).
- Auditores discordam sobre QUAL é o caminho canônico: Auditor 1 trata seed_one como canônico; Auditores 4/6 tratam add_node; Auditores 3/5 chamam o próprio create_agent_with de 'canonical path'. A discordância é o próprio diagnóstico: não existe caminho canônico — são três cópias paralelas, e por isso cada auditor elegeu uma referência diferente.
- Auditor 1 vs Auditor 6 sobre a direção da deficiência: o Auditor 1 enquadra o NodeRoleAssigned do ⌘N como divergência que 'quebra a projeção'. Os dados do Auditor 6 (confirmados no log vivo: 5 nós com role, 31 sem) mostram o oposto — o ⌘N é o comportamento CORRETO e os caminhos legados (seed/add_node) são os deficientes. A hipótese do fundador está certa no efeito (isolamento), mas invertida nesta dimensão: o ⌘N é o que MAIS registra.
- Auditor 4 sobre o token de hook ('h.register(name) nunca roda se cwd=Some'): impreciso — o rewrite_bootstrap final (bridge.rs:2840) chama write_one para TODOS os terminais do snapshot, incluindo o novo, então o token É registrado; mas o settings.json que o carrega é escrito no dir-fantasma t{seq}, não no cwd real — o efeito prático (CLI sem hooks, timeline cega) está certo, o mecanismo descrito está errado.
- Auditor 5 afirma que 'dashboard lê ProjectedState (replay do log)': o código mostra que as linhas do painel leem NodeView.status do model vivo alimentado pelo pump (main.rs:921-932), não replay de ProjectedState. O ponto válido que sobrevive é o de duas fontes vivas (sup.list para agents.json vs model para o painel).
