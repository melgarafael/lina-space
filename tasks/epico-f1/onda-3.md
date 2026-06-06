# Entrega LLM Engineer — Onda F1-3 · Inteligência da Lina

> **Autor:** LLM Engineer · **Data:** 2026-06-06 · **Tarefa:** stories da onda F1-3 (documento, zero código de produção)
> **Fontes primárias:** [[13.4 - Pesquisa F1 - Personalidade Anti-Slop]] · [[13.8 - Pesquisa F1 R2 - Hermes Desktop Deep-Dive]] · [[13.14 - Pesquisa F1 R2 - Seguranca e Sandboxing Inter-Terminal]] · [[_HANDOFF - Continuar Lina Space]] §4/§8 (protocolo Maestro real) · `docs/adr/0007` · `tasks/epico-f1/insforge-ref.md` (nota do Maestro) · engenharia reversa autorizada de `~/hermes-agent` (read-only; fatos re-derivados no código com arquivo:linha)
> **Doutrina/skill que esta onda EVOLUI:** `assets/lina-doctrine/CLAUDE.md` (8 blocos, W3-2) e `assets/lina-skills/lina-agent-bus/SKILL.md` — *caminho canônico verificado no repo (spot-check 2026-06-06): é `assets/` na RAIZ; `crates/lina-bootstrap/assets/` NÃO existe; o crate consome a doutrina via `include_str!("../../../assets/lina-doctrine/CLAUDE.md")` (`crates/lina-bootstrap/src/lib.rs:24-26`), que se declara "FONTE DA VERDADE" (`lib.rs:9`).*
> **Revisão:** 2026-06-06 pós-auditoria do Maestro (batch 2) — escopo F1-3-7 fechado por decisão do fundador; refs a ADR 0019 (arquiteto); validação na tela bloqueante no gate.

---

## 1. Objetivo da onda

Fazer a inteligência que o Lina **provê** (não a que ele *tem* — inv#1: a inteligência é do CLI) deixar de ser um acidente do modelo e virar **arquitetura**: personalidade em camadas estruturais, skills anti-slop agnósticas, orquestração multi-terminal que internaliza o método que construiu a Fase 0, despacho rico, criação de terminal pelo agente com guardrails, e o primeiro loop de auto-aprimoramento (`lina retro`).

A tese da onda foi confirmada em duas lentes pela pesquisa ([[13.4]] achado 5, 🟢 load-bearing): **personalidade emerge de decisões estruturais — harness, loop, memória, camadas de verificação — não de prompt bonito.** Toda story abaixo é uma decisão estrutural; nenhuma é "ajustar adjetivos no system prompt".

A engenharia reversa do Hermes confirma a viabilidade sob nossos invariantes: o motor de auto-manutenção de skills do Curator é **função pura sobre timestamps** (`curator.py:268-323` — zero LLM), e o julgamento qualitativo é uma *tarefa* delegável a um agente existente sob gate humano (`CURATOR_DRY_RUN_BANNER`, `curator.py:330-354`). Ou seja: auto-aprimoramento **sem LLM próprio** é um padrão provado em produção (~39k commits, 17k testes — [[13.8]] achado 5).

## 2. Gate da onda (observável)

> **Cenário real, sem ensaio:** o fundador pede, em português comum, uma tarefa multi-terminal — ex.: *"constrói uma landing page do meu curso usando 3 terminais"* — num Espaço com terminais de papéis adequados. **Sem o fundador ensinar como orquestrar:**
> 1. um terminal assume a liderança (ou o @Maestro do preset), **decompõe** a tarefa em itens com dependências explícitas no `plan.md`, **define funções** e **despacha** com o template rico (F1-3-5);
> 2. os workers executam; o orquestrador **monitora** (`lina check`/event log), **detecta** ≥1 worker travado ou desviado (cenário induzido no teste) e **corrige o trajeto** (re-despacho ou re-direcionamento) sem intervenção humana;
> 3. a entrega final passa por **cold-review** (revisor isolado, F1-3-2) e é avaliada pela **rubrica anti-slop**: score ≥ limiar definido na rubrica v1, com zero marcador de slop "duro" (fonte Inter/gradiente roxo default/copy genérica — [[13.4]] achado 3);
> 4. o fundador vê tudo **narrado em pt-br simples** (bloco 8 da doutrina) — zero jargão de orquestração na superfície (inv#6).
>
> **Medição:** evento-a-evento no log (decomposição → claims → despachos → correção → review), réplica do método de validação da Fase 0 (instrumentação que o Maestro LÊ + olho do fundador — handoff §4.1).
>
> **⛔ BLOQUEANTE (decisão do Maestro 2026-06-06):** a **validação do fundador na tela** é condição de fechamento da onda — padrão do projeto (gpui não roda headless; a Fase 0 provou que o uso real expõe o que teste + red-team não pegam, handoff §8). A onda NÃO fecha por evidência de log sozinha.

---

## 3. Stories

### F1-3-1 · Personalidade da Lina em camadas estruturais (doutrina do gênio criativo)

- **Por quê:** [[13.4]] achado 5 (🟢 duas lentes): personalidade é engenharia de sistemas — system prompt (job description) → doutrina (valores + heurísticas) → tom → estrutura operacional (loop, memória, boundaries). O arquétipo Creator ([[13.4]]: Adobe L1→L3; "marca com opinião forte sobre qualidade, rejeição do genérico") é a identidade que o fundador escolheu: **o gênio criativo** — busca informação por **internalização** (consulta vault/plan/event log porque faz parte da natureza dele, não porque mandaram), **resolve problemas porque resolver faz parte da sua natureza** (zero hand-holding), e **anti-AI-slop total** em código, design e arquitetura. [[13.4]] achado 9: transparência da "constituição" é expectativa de 2026, não diferencial.
- **Escopo — entra:**
  - Evoluir `assets/lina-doctrine/CLAUDE.md` **dentro do contrato dos 8 blocos** (W3-2): bloco 1 (identidade) ganha o arquétipo; bloco 3 (vault) reframa a consulta como internalização ("você busca antes de assumir porque é assim que você trabalha", não como ordem); bloco 8 (TOM) ganha a opinião estética explícita e o anti-slop de comunicação (sem filler "Certainly!", recomendação decisiva > menu de opções — padrão do system prompt publicado da Anthropic, [[13.4]] achado 9).
  - **Opinião estética explícita** (paradigma duplo do cookbook Anthropic, [[13.4]] achado 3): prescritivo sobre o que **banir** (Inter/Roboto/Arial como default, gradiente roxo+branco, glassmorphism genérico, copy de template) + **encorajador de risco criativo** (escolher UMA direção estética clara por projeto, lida do vault do usuário quando existir). A doutrina define o *processo* de ter opinião; a direção específica por projeto vem do contexto.
  - Espelhamento em `AGENTS.md`/`GEMINI.md` (mesmo conteúdo, gatilho diferente — contrato já previsto no header do template).
  - Orçamento de tokens: o delta de personalidade cabe em ~2k tokens ([[13.4]] achado 2: economia de tokens é o que torna doutrina sustentável); o detalhe profundo vai para as skills da F1-3-3 (ativação on-demand), não para o turno-0.
- **Escopo — NÃO entra:** novo canal de injeção (usa o bootstrap turno-0 que JÁ existe — âncora W3-2); novos `{{placeholders}}` (o contrato do renderer diz "NENHUM outro existe" — se precisar, é mudança de contrato coordenada com o dono do renderer); traços de personalidade como engajamento (a pesquisa mostra que engajamento vem de progresso-na-tarefa, não de adjetivos — [[13.4]] achado 5); comando `/constitution` (Fase 1 tardia ou F2 — ver Dúvidas).
- **Critérios de aceite OBSERVÁVEIS:**
  1. Teste A/B mensurável: a MESMA tarefa (ex.: "faça a hero section da LP") despachada a 2 terminais idênticos, um com doutrina v-atual e outro com doutrina F1-3-1; ambas saídas avaliadas por cold-review cego (F1-3-2) contra a rubrica; a versão nova pontua ≥ a antiga e **zero marcador duro de slop** (a antiga hoje não tem opinião estética — espera-se diferença observável). **Responsável: red-team do Pesquisador** (despacho manual dos 2 terminais pelo harness do cenário — Analista monta, Pesquisador avalia às cegas); **NÃO depende de F1-3-4** (não há orquestração no cenário, só 2 despachos diretos).
  2. Render do bundle continua íntegro: golden test do bootstrap (família `pretooluse_golden`/render W3-2) passa sem mudança de contrato; os 8 blocos seguem na ordem; nenhum placeholder novo.
  3. `wc -w`/token-count do delta ≤ ~2k tokens equivalentes.
  4. Zero vazamento de jargão: cenário "agente narra ao leigo" continua passando o antídoto de eco (SKILL §3).
- **Âncoras (§3 doc 01 + invariantes):** Bootstrap turno-0 + doutrina (W3-2) — "tudo entra pelo mesmo canal; não hard-codar contexto por agente". Inv#1 (sem LLM próprio: personalidade vive em texto injetado, não em modelo), inv#3 (neutralidade: mesmo conteúdo nos 3 espelhos), inv#6 (não-técnico-first: tom).
- **Tamanho:** M · **Dono sugerido:** LLM Engineer (dono histórico da doutrina W3-2/W3-3) · **Deps:** F1-3-2 (rubrica, para o critério 1) — pode desenvolver em paralelo e validar no fim.

---

### F1-3-2 · Rubrica anti-slop executável + skill `lina-cold-review`

- **Por quê:** [[13.4]] achado 1: slop tem definição operacional (competência superficial, assimetria de esforço, reprodutibilidade em massa) e marcadores **concretos e detectáveis** (nomes genéricos, comentários óbvios, abstrações desnecessárias, cast para `any`; em design: branco+gradiente roxo, Inter). [[13.4]] achado 2: o mecanismo anti-slop comprovado do Superpowers é o **revisor isolado** — sessão nova, zero contexto do autor, que impede o auto-endosso. `insforge-ref.md`: pipeline de quality gates nativo é oportunidade aberta e diferenciada ("não encontrado no InsForge"). Memórias da Fase 0 avisam os dois modos de falha: revisor **otimista demais** (assina "ok" sem desafiar — [[verificador-adversarial-otimista-demais]]) e revisor **paranoico** (rateia a feature pedida como bug — [[revisao-adversarial-confunde-feature-com-bug]]); a rubrica existe para calibrar os dois.
- **Escopo — entra:**
  - **Rubrica anti-slop v1** como artefato versionado (`assets/lina-skills/lina-cold-review/references/rubrica.md`): dimensões com itens objetivos e pontuáveis — (a) código: marcadores do [[13.4]] achado 1 + duplicação (GitClear) + `unwrap()`/erro engolido; (b) design: lista de banimentos + exigência de direção estética declarada; (c) copy: filler, genericidade, voz; (d) arquitetura: abstração especulativa, complexidade não pedida. Cada item: descrição + como verificar + peso. Veredito = score + lista de violações + PASS/FAIL contra limiar.
  - **Skill `lina-cold-review`** (SKILL.md agnóstico): ensina o revisor a (1) operar SEM contexto do autor — recebe só o artefato + critérios de aceite + rubrica; (2) **triar contra o intent do spec** antes de ratear (lição [[revisao-adversarial-confunde-feature-com-bug]]); (3) devolver no formato `[EXPECTED]` (PASS/FAIL + violações com evidência apontável); (4) nunca "aprovar confirmando que o mecanismo existe" — desafiar se o critério é *enforced* (lição [[verificador-adversarial-otimista-demais]]).
  - Integração com a orquestração: o fluxo "autor termina → orquestrador despacha cold-review a um terminal SEM o contexto do autor" vira receita na skill de orquestração (F1-3-4).
- **Escopo — NÃO entra:** review thread visível no canvas (UX — onda de UX da F1); automação de score por parsing (quem pontua é o agente revisor lendo a rubrica — inv#1); rubrica como hook bloqueante de runtime (isso é guard/F1-6, outra camada).
- **Critérios de aceite OBSERVÁVEIS (cenário-teste mensurável):**
  1. **Teste de slop plantado:** um artefato-fixture com N violações conhecidas e deliberadas (ex.: 10 — Inter no CSS, gradiente roxo, função `handleData`, comentário óbvio, `any`-cast…) é submetido ao cold-review por um terminal fresco; o revisor reporta ≥80% das violações plantadas e o FAIL.
  2. **Teste de não-paranoia:** um artefato-fixture bom (zero violações plantadas, com direção estética declarada) recebe PASS — sem inventar violações "por desencargo" (máx. tolerado: itens BAIXA informativos).
  3. **Isolamento provado:** o despacho do review nunca inclui o histórico do autor — verificável no event log (o handoff de review referencia só artefato+rubrica).
  4. **Reprodutibilidade:** 3 execuções do mesmo review divergem no máximo em itens de severidade BAIXA (o PASS/FAIL não flipa).
- **Âncoras (§3 doc 01 + invariantes):** inv#1 (o juiz é o agente do terminal, não um LLM nosso), inv#4 (vereditos viram eventos — auditáveis e reconstruíveis), Workspace Bus (o review viaja pelos verbos `lina`, não por canal ad-hoc), Envelope A2A (intent `review` JÁ existe no envelope — `lina/msg@1`).
- **Tamanho:** M · **Dono sugerido:** LLM Engineer (rubrica+skill) com red-team do Pesquisador (testar os dois modos de falha) · **Deps:** nenhuma dentro da onda (é a fundação das demais).

---

### F1-3-3 · Catálogo de skills anti-slop agnósticas (1ª safra)

- **Por quê:** [[13.4]] achado 2 (Superpowers: ~14 SKILL.md compostos, núcleo ~2k tokens, ativação semântica via `description`) + achado 4 (SKILL.md é padrão aberto consolidado, 32+ CLIs — **com a ressalva verificada**: portabilidade não é 100% automática; testar em 3+ CLIs e aceitar extensões tool-specific marcadas). A doutrina (F1-3-1) carrega o *núcleo* da personalidade; as skills carregam a *profundidade* on-demand — é o que mantém o turno-0 barato.
- **Escopo — entra:** a 1ª safra implementada (itens 1–8 do catálogo abaixo) + o catálogo completo registrado como backlog. Cada skill: SKILL.md agnóstico (frontmatter `name`+`description` caprichado — o `description` governa a ativação semântica, [[13.4]] achado 4), ≤2k tokens no corpo, seção anti-slop própria ([[13.4]] item 3 da lista priorizada), zero verbo específico de um CLI no caminho principal (especificidades em seção "Notas por CLI" marcada).
- **Escopo — NÃO entra:** marketplace/distribuição de skills de terceiros (Fase 2 — e quando vier, com o pipeline allowlist+review+hooks do [[13.14]] achado 3, lição ClawHavoc); assinatura ed25519 (F2, [[13.14]] P2); skills de domínio do usuário (LP, e-mail…) — isso é preset/curador, outra onda.

**Catálogo proposto (nome + propósito de 1 linha):**

| # | Skill | Propósito (1 linha) | Safra |
|---|---|---|---|
| 1 | `lina-cold-review` | Revisor isolado sem contexto do autor; aplica a rubrica anti-slop e devolve PASS/FAIL com evidência (F1-3-2). | F1-3 |
| 2 | `lina-design-doctrine` | Opinião estética explícita: banimentos (Inter, gradiente roxo, glass genérico), exigência de direção declarada, tokens semânticos, motion com intenção ([[13.4]] achado 3). | F1-3 |
| 3 | `lina-architecture-doctrine` | Simplicidade primeiro: a menor mudança que resolve; sem abstração especulativa; pergunta "isto fecha uma porta?" antes de decidir (espelha o norte §3). | F1-3 |
| 4 | `lina-code-doctrine` | Anti-slop de código: nomes que dizem o quê, zero comentário óbvio, zero erro engolido, causa raiz > fix temporário, duplicação é dívida ([[13.4]] achado 1). | F1-3 |
| 5 | `lina-verification` | Nunca declarar "pronto" sem evidência observada: rodou, leu o output, viu o comportamento; "um staff engineer aprovaria?" (lição central do handoff §4.1). | F1-3 |
| 6 | `lina-orchestration` | O método Maestro internalizado: liderar, decompor, delegar, monitorar, corrigir trajeto, garantir objetivo (F1-3-4). | F1-3 |
| 7 | `lina-spawn-terminal` | Quando perceber que falta um papel no time, criar o terminal com nome+função+1º prompt — dentro dos gates (F1-3-6). | F1-3 |
| 8 | `lina-retro` | Ler o relatório determinístico do `lina retro` e propor melhorias com evidência do event log (F1-3-7). | F1-3 |
| 9 | `lina-copy-doctrine` | Anti-slop de texto: sem filler, sem genericidade de template, voz do usuário (lida do vault), uma recomendação decisiva > menu. | F1-3 (se couber) |
| 10 | `lina-dispatch` | O template rico de despacho e como preenchê-lo (F1-3-5 — pode nascer como seção da #6 e ganhar vida própria). | F1-3 |
| 11 | `lina-debugging` | Caça sistemática de causa raiz: reproduzir contra o HEAD antes de fabricar fix (lição [[diagnostico-de-task-pode-estar-defasado]]). | F2 |
| 12 | `lina-brainstorm` | Brainstorm multi-terminal: divergir em paralelo (N perspectivas), convergir por síntese com julgamento. | F2 |
| 13 | `lina-research` | Pesquisa multi-terminal: varredura → verificação adversarial em 2 lentes → síntese (o método que gerou as pesquisas 13.x). | F2 |
| 14 | `lina-bug-hunt` | Descoberta de bugs multi-terminal: finders com lentes diversas + céticos verificadores + critério de parada "zero ALTA" (handoff §4.8). | F2 |
| 15 | `lina-red-team` | Red-team contínuo como produto: re-derivar o mecanismo no código antes de assinar severidade (lição [[redteam-reverificar-mecanismo-do-finder]]). | F2 |
| 16 | `lina-skill-author` | Criar skill nova bem-formada: description que ativa, ≤2k tokens, agnóstica — com o gatilho do Hermes ("tarefa difícil superada → ofereça salvar como skill", `skill_manager_tool.py:917-924`). | F2 |
| 17 | `lina-plan-writing` | Quebrar épico em stories com critério de aceite observável e dependências explícitas (o formato do doc 32 internalizado). | F2 |

- **Critérios de aceite OBSERVÁVEIS (cenário-teste mensurável por skill):**
  1. **Ativação semântica:** para cada skill da 1ª safra, 10 frases de usuário plausíveis (sem citar o nome da skill) → o agente ativa a skill certa em ≥8 (testado em Claude Code; espelhado no mínimo em mais 2 CLIs — Codex e um terceiro — conforme ressalva [[13.4]] achado 4).
  2. **Portabilidade:** cada SKILL.md carrega e é seguível em 3+ CLIs sem editar o corpo (diferenças vivem na seção "Notas por CLI"); evidência = transcript de cada CLI executando o cenário-teste da skill.
  3. **Orçamento:** corpo ≤2k tokens (medido); description ≤ limite do frontmatter dos 3 CLIs.
  4. **Eficácia anti-slop:** cenário-teste da F1-3-2 critério 1 executado COM as skills de doutrina ativas melhora o score da saída vs. sem skills (mesma tarefa, mesmo CLI, A/B). **Pré-condição obrigatória:** o critério 3 de F1-3-2 (**isolamento do cold-review provado**) deve estar VERDE antes — sem isolamento provado, o A/B não conta como evidência (revisor contaminado invalida a comparação).
- **Âncoras (§3 doc 01 + invariantes):** inv#3 (neutralidade multi-CLI — skills agnósticas são a *prova* do invariante), Bootstrap turno-0 (skills do papel já chegam pelo `{{role_skills_list}}` — canal existente), CLI Profiles (nenhum conhecimento de CLI no core; especificidade em config/skill).
- **Tamanho:** L (1ª safra de 8–10 skills + testes em 3 CLIs) · **Dono sugerido:** LLM Engineer (autor) + Analista (harness de teste de ativação) · **Deps:** F1-3-2 (rubrica, para o critério 4).

---

### F1-3-4 · Skill de orquestração multi-terminal (`lina-orchestration`) — o método Maestro internalizado

- **Por quê:** é o coração da direção do fundador: **internalizar no produto o método que o construiu**. O protocolo Maestro REAL está documentado com lições pagas em sangue no [[_HANDOFF - Continuar Lina Space]] §4 e §8: nunca "pronto" por teste verde em coisa visual (§4.1), validar de fora em canal limpo (§4.2), fronteiras de arquivo disjuntas (§4.3), marcador INICIADO + watcher (§4.4), workers saturam → rotação (§4.5), red-team contínuo com critério de parada "zero ALTA" (§4.7-8), docs vivos (§4.11). E a engenharia reversa do kanban do Hermes fornece os algoritmos que o broadcast puro não tem: dependências explícitas (`task_links`, `kanban_db.py:1043`), re-verificação de pais no instante do claim (anti-race, `kanban_db.py:2996-3012` — ecoa [[lina-a2a-routeblocked-race-e-ask-ok-cego]]), detecção graduada de worker travado em 4 camadas onde **PID vivo ≠ progresso** (`kanban_db.py:3207-3284`), circuit breaker sticky após N falhas (`kanban_db.py:4749`), e **violação de protocolo** = terminar sem emitir DONE/BLOCKED → falha na 1ª ocorrência (`kanban_db.py:5437-5443` — exatamente o "lina ask deu ok cego" da memória).
- **Escopo — entra:**
  - SKILL.md `lina-orchestration` ensinando o PAPEL de orquestrador (qualquer terminal pode assumi-lo; o preset Maestro o carrega por padrão): **liderar** (assumir o objetivo e mantê-lo), **decompor** (itens no `plan.md` com `parents:` explícitos — nunca prosa "espere o T1"; doutrina do orchestrator do Hermes, `kanban-orchestrator/SKILL.md` linha 54), **definir funções** (mapear papel→item; se falta papel → F1-3-6), **despachar** (template F1-3-5, fire-and-forget + `lina check`), **intermediar** (conflito entre colegas → decisão registrada, nunca silenciosa), **monitorar** — usando DUAS projeções nomeadas do event log: (i) **evento-de-progresso** (último evento emitido pelo worker por item/`root_cause_id` — status/claim/plan/A2A contam como progresso; "respirar" no bus NÃO conta) e (ii) **timeout-de-travamento** (staleness = agora − último evento-de-progresso > limiar → travado, mesmo com processo vivo). **As definições operacionais de "progresso" e "travamento" moram no ADR 0019 do Arquiteto — pré-requisito desta story**; a skill consome as definições, não as inventa. **Corrigir trajeto** (re-despachar com contexto das tentativas anteriores; após 2 falhas consecutivas do mesmo item → parar e escalar ao humano — breaker sticky), **garantir objetivo** (gate de saída: cold-review + critérios do plan antes de narrar "pronto" ao usuário).
  - Receitas operacionais destiladas do protocolo real: despacho com marcador de início + verificação de vida (handoff §4.4 → no produto: o evento de entrega A2A JÁ confirma a injeção; a skill ensina a checar o *primeiro evento de progresso*, não a confiar no "ok" do verbo — lição [[lina-a2a-routeblocked-race-e-ask-ok-cego]]); validação de entrega "de fora" (orquestrador confere o artefato, não aceita o auto-relato — handoff §4.2); fronteiras de escopo por worker (claims de arquivo via `lina claim` — mecanismo já existe).
  - Interação com guardrails EXISTENTES documentada na skill (não reinventa): autonomia (bloco 5), orçamento/cascata (ADR 0007 — orquestrar é gastar o orçamento de CASCATA com sabedoria), anti-eco (SKILL lina-agent-bus §3), gate humano (bloco 7).
- **Escopo — NÃO entra:** dispatcher mecânico/daemon de polling (contradiz o design event-sourced — ressalva explícita da engenharia reversa; o "supervisor mecânico" do Lina é o router/Supervisor que JÁ existe); mudanças no router para esta story (se a story F1-1 de cognição entregar projeções de progresso, esta skill as consome — dep cross-onda); UI de visualização da orquestração (onda de UX).
- **Critérios de aceite OBSERVÁVEIS:**
  1. **É o gate da onda** (§2 deste doc): cenário LP-com-3-terminais executa de ponta a ponta sem o fundador ensinar; evidência evento-a-evento.
  2. **Correção de trajeto provada:** no cenário-teste, um worker é induzido a travar (prompt que o faz parar) e outro a desviar (entregar fora do spec); o orquestrador detecta ambos pelo event log/check (não por sorte), re-despacha com contexto da tentativa anterior, e o log mostra a sequência detecção→correção.
  3. **Breaker respeitado:** item que falha 2× consecutivas NÃO é re-despachado uma 3ª vez automaticamente — vira escalada narrada ao usuário (pt-br simples).
  4. **Anti-race:** despacho de item com `parents:` pendentes é recusado/adiado (re-verificação no instante do despacho, não só na promoção) — teste induz a corrida.
  5. Saída final do cenário passa cold-review com a rubrica (PASS).
- **Âncoras (§3 doc 01 + invariantes):** Workspace Bus/Supervisor ("toda orquestração futura pendura aqui" — a skill USA os verbos, não cria canal paralelo), Event Store (monitoramento = projeções do log; inv#4), Envelope A2A versionado (intents existentes; nada de formato novo por feature), inv#5 (pertencimento = conexão: o time é o roster vivo), inv#6 (narração ao leigo).
- **Tamanho:** L · **Dono sugerido:** LLM Engineer (skill) + Arquiteto (revisão da interação com router/guardrails) + Analista (cenário-teste do gate) · **Deps:** **ADR 0019 (Arquiteto — definições de progresso/travamento; pré-requisito)**, F1-3-5 (template de despacho), F1-3-2 (cold-review no gate); cross-onda: F1-1 (cognição do trabalho — `lina check` rico/histórico) eleva a qualidade do monitoramento, mas a v1 funciona com os verbos existentes.

---

### F1-3-5 · Engenharia de prompt de DESPACHO: o template rico

- **Por quê:** o despacho pobre é o modo de falha nº1 de orquestração multi-agente ([[13.4]] achado 5: harness > prompt, e o despacho É harness). O protocolo da Fase 0 já cristalizou o formato vencedor (handoff §3: "cd repo + ler design/arquivo:linha + a story + REGRAS + ENTREGA + PRONTO/BLOCKED") — este documento que você está lendo foi despachado EXATAMENTE assim, e funcionou. O Hermes confirma o padrão e adiciona estrutura (`build_worker_context`, `kanban_db.py:6850-7065`): corpo com critérios de aceite, **tentativas anteriores** (retry sabe o que falhou), **handoffs dos pais** (fan-in de dependências), histórico do papel, tudo com **caps por campo** (4-8KB) — que endereça a lição [[workflow-sintese-em-chunks]] (contexto grande trunca).
- **Escopo — entra:**
  - Template canônico de despacho com os 5 campos do pedido do fundador, mapeados ao que a Fase 0 + Hermes provaram:
    1. **CONTEXTO** — onde está o trabalho (cwd, arquivos:linha a ler, handoffs dos itens-pais, tentativas anteriores se re-despacho);
    2. **FUNÇÃO** — o papel deste terminal nesta tarefa ("você é o revisor isolado", "você é o dono do CSS");
    3. **DIRECIONAMENTO** — as regras do jogo (escopo: mexa só em X; proibições; doutrina aplicável; orçamento);
    4. **OBJETIVO** — o resultado de negócio em 1-2 frases (o "por quê" que permite decisão autônoma boa);
    5. **RESULTADO ESPERADO** — formato exato da entrega + marcador de conclusão (`PRONTO: resumo` / `BLOCKED: motivo`) — sem o marcador, o orquestrador trata como falha de protocolo (F1-3-4 critério 3; `kanban_db.py:5437` como precedente).
  - Padrão **pull-then-context** ensinado junto: a mensagem A2A carrega o ponteiro (`ref: plan:T4`) e o essencial; o contexto pesado o worker PUXA (`lina plan read`, `lina vault read`, arquivo) — não empurrar tudo pelo bus (precedente: argv do Hermes carrega só o id, `kanban_db.py:6639`).
  - Entrega como seção rica da skill `lina-orchestration` + exemplos preenchidos (bom/ruim) por tipo de tarefa (build, review, pesquisa, bug).
- **Escopo — NÃO entra:** renderização automática do template pelo app (o app não é IA — inv#1; quem preenche é o agente orquestrador); validação sintática por hook (possível F2).
- **Critérios de aceite OBSERVÁVEIS:**
  1. **Checklist mecânica:** despacho gerado pelo orquestrador no cenário do gate contém os 5 campos + marcador de conclusão — verificável por inspeção do payload no event log.
  2. **Teste de autossuficiência:** um terminal FRESCO (sessão nova) recebe um despacho preenchido para uma tarefa-fixture e a executa sem nenhum `ask` de esclarecimento sobre o que/onde/como (medido: zero asks de volta com intent "esclarecimento" no log do cenário).
  3. **Re-despacho informado:** no cenário de correção de trajeto (F1-3-4 critério 2), o 2º despacho contém a seção "tentativas anteriores" com o que falhou — verificável no payload.
- **Âncoras (§3 doc 01 + invariantes):** Envelope A2A versionado (`ref`/`context` já existem no envelope — o template os usa, não cria campos), inv#1 (template é doutrina para o agente, não feature de app), inv#4 (despachos são eventos auditáveis).
- **Tamanho:** S · **Dono sugerido:** LLM Engineer · **Deps:** nenhuma (alimenta a F1-3-4).

---

### F1-3-6 · Skill + verbo `lina spawn`: agente-cria-terminal com guardrails

- **Por quê:** fecha o ciclo da auto-orquestração: hoje a regra de 3 passos da doutrina termina em "ninguém tem o papel → faça o possível e marque no plano" (bloco 4). Com `spawn`, o time se completa sozinho — o diferencial "automático SEM pedir" ([[01]] §1, o moat) levado à topologia. A nota do Maestro (`insforge-ref.md` item 1) crava a doutrina: **capacidades sensíveis são VERBOS estruturados do `lina`, nunca shell direto** — spawn é a capacidade mais sensível da onda (cria processo, consome recursos, gasta dinheiro do usuário). O ADR 0007 fornece o mecanismo de gate pronto: a distinção ORIGEM/CASCATA via `hops` **não-forjável** (binding interno `delivered_root`, nunca campo da mensagem). O [[13.14]] fornece o vocabulário de política: deny-by-default, permissões escopadas, e o campo `intent` para observabilidade intent-vs-action (achado 5: "nós vemos e bloqueamos em runtime", nunca "impedimos").
- **Escopo — entra:**
  - **Verbo `lina spawn "@Nome" --role PAPEL --prompt "..."`** (core): cria o nó-terminal via Supervisor (que aloca o NodeId — autoridade única, lição [[lina-app-nodeid-autoridade-unica]]), monta o bundle de bootstrap turno-0 (doutrina+papel+skills — canal existente), entra na `WorkspaceTrust` por pertencimento (ADR 0006/inv#5) e entrega o 1º prompt pela fila serial normal.
  - **O gate (✅ CONFIRMADO pelo Maestro 2026-06-06: `max_spawns_per_turn=2` + gating origem vs cascata como proposto; formalização via ADR 0019 do Arquiteto — o ADR é pré-requisito do merge do verbo):**
    1. **Autonomia primeiro** (matriz existente, bloco 5): `manual` → bloqueado; `assistido` → propõe e espera o sim; `autonomo` → segue para os gates abaixo.
    2. **Origem vs cascata (ADR 0007):** spawn pedido na ORIGEM (`hops==0` efetivo — agente agindo a mando direto do humano) → permitido com narração obrigatória ao usuário (mesma regra da 1ª delegação em autonomo); spawn na CASCATA (`hops>=1` — um agente que recebeu A2A querendo criar terminal) → **gate humano SEMPRE** (é o análogo do fan-out: a tempestade de spawn mora na cascata; um worker spawnar workers é fork-bomb em potencial).
    3. **Teto por turno:** `max_spawns_per_turn=2` (confirmado; guardrail novo no workspace.json ao lado de `max_delegations_per_turn`) contado por `root_cause_id` — estourou → gate humano.
    4. **Intent no envelope:** o pedido de spawn viaja com `intent: spawn` ([[13.14]] P1 intent-vs-action: o log registra o pedido, os args reais e a decisão do gate — visível, não escondido).
    5. **Recursos:** spawn respeita o teto de custo existente (W3-7c `CostCeiling`) — terminal novo conta no ledger do workspace.
  - **Skill `lina-spawn-terminal`**: QUANDO perceber a falta (a regra de 3 passos ganha o 4º passo: "ninguém tem o papel E a tarefa o exige → spawn"), COMO nomear (papel pelo nome — role-discovery W3-1 já resolve), como escrever o 1º prompt (= template F1-3-5), e o que narrar ao leigo ("trouxe um especialista de X pro time porque...").
- **Escopo — NÃO entra:** spawn de terminal com CLI diferente do default do workspace (escolha de CLI é config/preset — neutralidade inv#3 preservada via CLI Profiles, mas a UX de escolher fica pra onda de presets); auto-kill/despawn pelo agente (só o humano remove — simetria com "ação irreversível"); sandbox por-PTY (fio F1-6/[[13.14]] — o spawn herda o isolamento que existir).
- **Critérios de aceite OBSERVÁVEIS:**
  1. Cenário feliz: em `autonomo`, agente na ORIGEM percebe papel faltante (tarefa-fixture exige QA, workspace não tem), narra, spawna; o novo terminal aparece no roster (`lina list`), recebe bootstrap turno-0 íntegro (handshake no `agents.json`) e executa o 1º prompt — tudo verificável por eventos.
  2. **Gate de cascata:** worker que recebeu handoff (binding presente) pede spawn → evento `SpawnGated` + banner de confirmação humana; sem confirmação, nada é criado.
  3. **Não-forjabilidade:** teste irmão do `f005324` (B2): pedido de spawn forjando `hops=0`/root fresco no outbox → AINDA gated (o gate lê o binding, nunca o campo) — não-vacuoso, quebra se o gate reler `msg.hops`.
  4. Teto: 3º spawn no mesmo turno → gated. `manual` → recusado localmente pelo verbo (mesmo mecanismo do handoff bloqueado, doutrina bloco 5).
  5. Red-team do verbo: 0 ALTA (critério de parada do handoff §4.8), com red-team próprio ANTES do externo (lição [[gate-humano-campos-controlaveis-e-redteam]]: nenhum campo escrito pelo agente decide identidade/autorização do spawn).
- **Âncoras (§3 doc 01 + invariantes):** Workspace Bus/Supervisor (spawn pendura no broker; NodeId pelo Supervisor), Bootstrap turno-0 (o novo terminal nasce pelo MESMO canal — não hard-codar), Envelope A2A versionado (intent `spawn` é extensão do contrato existente, registrada — não formato novo), inv#5 (nascer no Espaço = estar no time), inv#2 (tudo local), inv#6 (narração ao leigo).
- **Tamanho:** M (core: verbo+gate) + S (skill) · **Dono sugerido:** Dev 01 ou Arquiteto (core — mexe no router/supervisor; quem pegou ADR 0006/0007 tem o contexto) + LLM Engineer (skill+doutrina) + Pesquisador (red-team) · **Deps:** **ADR 0019 (Arquiteto — formaliza caps e gating do spawn; pré-requisito)**, F1-3-5 (1º prompt usa o template); coordenar com F1-6 (campo `intent`/política de segurança — o gate de spawn deve nascer alinhado, não retrofitado).

---

### F1-3-7 · Auto-aprimoramento v0: verbo `lina retro` (Curator adaptado a event-sourcing, sem LLM próprio)

- **Por quê:** [[13.8]] matriz de decisões #1 marca auto-manutenção de skills como **decisão crítica da Fase 1** ("se omitir, a biblioteca vira bloat"). A engenharia reversa mostrou COMO portar sem violar inv#1: o Curator do Hermes tem dois subsistemas separáveis — (a) um **motor determinístico de lifecycle**, função pura sobre timestamps de uso (`apply_automatic_transitions`, `curator.py:268-323`; thresholds 30d/90d em `curator.py:58-59`; reativação ao reusar em `curator.py:318-321`; pinned imune em `curator.py:292`), e (b) uma **passada de julgamento por LLM auxiliar** (single-pass, `max_iterations=9999` em `curator.py:1764` — corrigido pelo Maestro vs a doc que diz 8). O (a) é portável como **projeção sobre o event log** (estritamente melhor que o sidecar `.usage.json` imperativo do Hermes); o (b) vira uma **tarefa que o agente do terminal executa** lendo um relatório preparado — exatamente o padrão dry-run + gate humano que o próprio Hermes usa (`CURATOR_DRY_RUN_BANNER`, `curator.py:330-354`).
- **Escopo — FECHADO por decisão do fundador (2026-06-06):** *auto-aprimoramento v0 = Curator-like com gate humano: **observa** eventos do workspace (o que o usuário mais pede, onde agentes travam) e **SUGERE** melhorias — **skills, papéis e presets** — sempre com aprovação humana; **nunca aplica sozinho**. Aprendizado automático fica para a Fase 2.* Tudo abaixo implementa exatamente esse contorno.
- **Escopo — entra (v0 enxuto):**
  - **Família de eventos de uso** no log (coordenar upcasting com o dono do core): `SkillInvoked`, `SkillCreated` — pré-requisito dos dados de skill; delegações/custos/bloqueios JÁ são eventos (router/CostLedger W3-7c).
  - **Verbo `lina retro`** (bin `lina`, determinístico, zero LLM — quem pensa é o agente): emite um RELATÓRIO estruturado do workspace a partir de projeções do log:
    - **Skills:** uso por skill (invocações, última atividade), candidatas a stale (>30d) e archive (>90d) — mesma máquina de estados do Hermes como *sugestão*, não ação; clusters de nomes estreitos candidatos a consolidação (equivalente determinístico do `_render_candidate_list`, `curator.py:1395-1412`);
    - **Coordenação:** gargalos do turno — contagem de `RouteBlocked`/`FanoutGated`/re-delegações da mesma tarefa/itens que tripam breaker — com `root_cause_id` para auditoria;
    - **Custos:** gasto por terminal/papel/turno (CostLedger), outliers;
    - **Padrões de pedido do usuário:** o que o usuário mais pede aos terminais (frequência por tema/papel a partir dos turnos de origem `hops==0` no log) — insumo direto para sugerir skills e presets que encurtem o caminho do pedido recorrente;
    - **Lacunas:** papéis pedidos e ausentes (delegações que caíram no passo 3 da regra "ninguém tem o papel").
  - **Skill `lina-retro`**: o agente lê o relatório e propõe melhorias **com evidência apontável**, nos TRÊS tipos da decisão do fundador — **skills** (criar skill X porque a sequência Y se repetiu N vezes; arquivar Z não usada há 90d), **papéis** (quebrar tarefas do papel W que tripam breaker; papel ausente recorrente) e **presets** (o conjunto de papéis+skills que o padrão de pedidos do usuário sugere) — e TODA mutação proposta passa por **gate humano** (em pt-br simples). O agente **nunca aplica sozinho** ([[13.8]] item 1: portar o padrão, descrever como revisão single-pass, sem "quorum").
  - **Padrões do Curator a copiar já na v0:** `pinned` como opt-out durável (evento `SkillPinned`); arquivar ≠ deletar (evento reversível — compensating event, de graça no event-sourcing); `absorbed_into` declarado no ato da consolidação (campo do evento — elimina a reconciliação heurística pós-hoc que o Hermes precisa fazer em `curator.py:795-923` por não ter log); first-run defer (sem dados de uso suficientes → relatório diz isso e não sugere prune — evita o mass-prune do primeiro tick, `curator.py:238-254`).
- **Escopo — NÃO entra (= "aprendizado automático", marcado para Fase 2 pela decisão do fundador):** transições automáticas stale/archive sem humano (F2, e SÓ com telemetria de uso madura); agendamento em background/inactivity-trigger (F2 — v0 é on-demand: usuário ou orquestrador roda `lina retro`); consolidação umbrella executada (v0 só *sugere* clusters; executar merge de skills é F2 com o pipeline de review); qualquer análise por daemon (nunca — inv#1); user-modeling dialético tipo honcho (refutado para nós: é serviço cloud/daemon-LLM — engenharia reversa, campo local_vs_cloud).
- **Critérios de aceite OBSERVÁVEIS:**
  1. **Determinismo:** contra um event log fixture semeado (N delegações conhecidas, M `RouteBlocked`, custos por terminal, 2 skills sem uso >90d, 1 cluster de nomes), `lina retro` produz exatamente esses números no relatório — 2 execuções, saída byte-idêntica (módulo timestamp do cabeçalho).
  2. **Replay:** o relatório é uma projeção — apagar o estado derivado e reconstruir do log dá o mesmo resultado (inv#4 provado no teste).
  3. **Cenário agêntico:** um terminal com a skill `lina-retro` lê o relatório fixture e propõe ≥3 melhorias, cada uma citando a evidência (número/evento do relatório) — avaliado por cold-review (sem proposta genérica "melhorar a comunicação").
  4. **Gate inviolável:** nenhum caminho do verbo `lina retro` escreve fora do relatório; mutação de skill (archive/pin) só via verbo próprio com confirmação — teste tenta induzir o agente a arquivar direto e o verbo recusa sem o gate.
  5. **First-run honesto:** workspace recém-criado → relatório declara "dados insuficientes" e zero sugestões de prune.
- **Âncoras (§3 doc 01 + invariantes):** Event Store + event-sourcing ("a Fase 1 inteira — Curador, webhooks, discovery — reusa o log"; esta story é o primeiro consumidor sério), inv#1 (análise pelo agente do terminal), inv#2 (tudo local — diferencial direto vs honcho-cloud do Hermes), inv#4 (relatório = projeção reconstruível).
- **Tamanho:** M (verbo+eventos) + S (skill) · **Dono sugerido:** Dev 02 (eventos/projeção — dono histórico de fundação core) + LLM Engineer (skill + formato do relatório) · **Deps:** F1-3-2 (cold-review no critério 3); cross-onda: F1-1 (cognição do trabalho — se F1-1 entregar projeções de histórico, `lina retro` as reusa; sequenciar F1-1 antes, como a nota do Maestro já aponta).

---

## 4. Riscos da onda

1. **Doutrina inflada = imposto em todo turno-0.** Cada token da doutrina é pago por TODOS os terminais em TODA sessão. Mitigação: núcleo ≤2k tokens na doutrina; profundidade nas skills com ativação on-demand ([[13.4]] achado 2). Critério de aceite 3 da F1-3-1 trava isso.
2. **Portabilidade de skill superestimada.** A verificação do [[13.4]] (achado 4) avisa: contar com extensões tool-specific inevitáveis. Mitigação: seção "Notas por CLI" + teste em 3+ CLIs como critério duro (F1-3-3 critério 2). Se um CLI não suportar ativação semântica, o bootstrap ordena o carregamento explícito (canal turno-0 cobre).
3. **Cold-review vira teatro.** Os dois modos de falha são conhecidos das memórias da Fase 0 (otimista que assina sem desafiar; paranoico que rateia a feature). Mitigação: rubrica com itens objetivos + testes de slop-plantado E de não-paranoia (F1-3-2 critérios 1-2) — o revisor é testado nos dois sentidos.
4. **Spawn = fork-bomb/gasto descontrolado se o gate falhar.** A família de ataque é a mesma do fan-out (ADR 0007): sinal de gate DEVE vir de fonte não-forjável (binding), nunca de campo da mensagem. Mitigação: critério 3 da F1-3-6 (teste de forja não-vacuoso) + teto por turno + cascata sempre gated + red-team 0 ALTA antes de fechar. Risco residual: spawn na origem em `autonomo` é O(teto) por turno — aceitável e narrado.
5. **`lina retro` sem dados.** A v0 depende de eventos (`SkillInvoked` etc.) que ainda não existem; relatório nasce magro. Mitigação: first-run honesto (critério 5) + priorizar as seções que JÁ têm eventos (delegações, custos, bloqueios — tudo no log desde a Onda 3). O Hermes confirma o anti-pattern a evitar: o prompt do Curator manda *ignorar* contadores de uso porque são novos e não-confiáveis (`curator.py:378-386`) — não repetir: nossos eventos nascem no log com replay, confiáveis desde o dia 1.
6. **Auto-aprimoramento mutando a biblioteca sozinho.** Tese refutável por uma má leitura do Hermes ("o Curator faz sozinho, façamos também"). Nossa resposta: dry-run + gate humano é o padrão que o PRÓPRIO Hermes usa para runs supervisionadas, e o nosso invariante #1 + público não-técnico exigem o gate sempre (F1-3-7 critério 4 trava).
7. **Skill de orquestração conflitando com guardrails do router.** Uma receita mal escrita pode ensinar o agente a lutar contra o ADR 0007 (ex.: contornar gate de cascata) ou re-abrir o anti-pattern "broadcast em resposta a broadcast". Mitigação: revisão cruzada da skill pelo Arquiteto (dono do router) como parte do aceite da F1-3-4; a skill documenta os guardrails como FÍSICA do mundo, não como obstáculo.
8. **Personalidade regredindo a cosmética.** A tentação de "deixar a Lina charmosa" no prompt é o anti-pattern que a pesquisa nomeia: engajamento vem de progresso-na-tarefa visível, não de traços ([[13.4]] achado 5). Mitigação: todo item de personalidade nesta onda tem mecanismo associado (rubrica, review, template, gate) — se uma proposta de "personalidade" não muda comportamento mensurável, ela não entra.

## 5. Propostas de ajuste ao esqueleto

1. **Sequenciar F1-1 (cognição do trabalho) antes do miolo da F1-3.** A nota do Maestro (`insforge-ref.md` item 2) já aponta: agente bem-informado erra menos. `lina retro` (F1-3-7) e o monitoramento da orquestração (F1-3-4) ficam medíocres sem as projeções de histórico/check rico da F1-1. Proposta: F1-3-2 e F1-3-5 (fundações, sem dep) podem correr em paralelo à F1-1; F1-3-4/6/7 entram depois.
2. **F1-3-6 (spawn) tem um pé no core.** O verbo+gate mexe em Supervisor/router — se o esqueleto tiver uma onda de "núcleo/arquitetura" (F1-2?), o mecanismo pode morar lá com a skill/doutrina ficando na F1-3. Alternativa: manter junto aqui (como propus) para o gate nascer colado na doutrina — mas então reservar o dono certo (Arquiteto/Dev 01) e a coordenação explícita com F1-6 (intent/política).
3. **A rubrica anti-slop é infraestrutura do ÉPICO inteiro, não só desta onda.** O gate de várias ondas (presets ricos, curador, webhooks de conteúdo) vai querer "saída avaliada por rubrica". Proposta: promover a rubrica (F1-3-2) a artefato transversal do épico, citado no cabeçalho como o doc 32 cita o contrato A2A consolidado.
4. **Registrar a doutrina InsForge no épico:** *"capacidades sensíveis são VERBOS estruturados do `lina`, nunca shell direto"* (insforge-ref item 1) merece virar âncora nomeada do épico F1 (vale para spawn F1-3-6, proxy de credenciais F1-6, e qualquer verbo futuro).
5. **Manter "A2A-ready" como fronteira de design, não entrega.** A pesquisa refutou A2A formal em F1 ([[13.4]] achado 6, 🔴 duas lentes). Nenhuma story desta onda depende de protocolo externo; sugiro o esqueleto declarar isso globalmente para as outras ondas não re-importarem o risco.
6. **Caminho dos assets corrigido:** o esqueleto deve referenciar `assets/lina-doctrine/` e `assets/lina-skills/` (raiz do repo), não `crates/lina-bootstrap/assets/...` (caminho citado no despacho não existe; o bundle do `.app` os embute de `assets/`).

## 6. Dúvidas para o Maestro

1. **Direção estética:** a doutrina define o *processo* de ter opinião (como propus na F1-3-1) ou o fundador quer cravar UMA identidade visual da Lina como default (ex.: o "Solarpunk tech" sugerido em [[13.4]] item 3)? Muda o conteúdo da `lina-design-doctrine`.
2. ~~**Tetos do spawn:** propus `max_spawns_per_turn=2` + cascata sempre gated. Validar os números com o fio de segurança (F1-6) e com a sua leitura do ADR 0007 — quer um ADR próprio para o gate de spawn?~~ → **✅ RESPONDIDA (Maestro 2026-06-06):** caps confirmados (`max_spawns_per_turn=2`, gating origem vs cascata como proposto); formalização via **ADR 0019 do Arquiteto**.
3. **Eventos novos (`SkillInvoked`/`SkillCreated`/`SkillPinned`):** quem é o dono do upcasting do Event Store nesta fase? F1-3-7 precisa dessa coordenação cedo para não nascer com sidecar imperativo (o anti-pattern do Hermes).
4. **`/constitution` ([[13.4]] achado 9):** entra nesta onda (transparência da doutrina como feature) ou vai para a onda de UX/confiança? Deixei fora do escopo da F1-3-1 por ser superfície, não estrutura.
5. **Cota da 1ª safra de skills:** propus 8-10 implementadas + catálogo de 17. Se o orçamento da onda apertar, o mínimo viável que sustenta o gate é: cold-review + design-doctrine + code-doctrine + verification + orchestration + dispatch (6).
6. ~~**Gate da onda:** o cenário LP-com-3-terminais precisa do fundador na tela — agendar como o "ponto de teste humano" da onda?~~ → **✅ RESPONDIDA (Maestro 2026-06-06):** validação do fundador na tela é **BLOQUEANTE** (padrão do projeto) — incorporada ao §2.

---

PRONTO: stories F1-3-1..7 da onda Inteligência da Lina entregues em `tasks/epico-f1/onda-3.md` — personalidade estrutural, rubrica+cold-review, catálogo de 17 skills agnósticas, orquestração com o método Maestro internalizado, template de despacho, `lina spawn` gateado por binding não-forjável e `lina retro` event-sourced sem LLM próprio (Curator do Hermes re-derivado no código com arquivo:linha).
