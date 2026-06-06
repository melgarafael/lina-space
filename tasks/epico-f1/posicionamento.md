# Entrega Analista — Posicionamento do Épico F1

> **Papel:** Analista de Posicionamento · **Data:** 2026-06-06
> **Fontes:** doc 10 (baseline Fase 0), doc 13 (índice + vereditos 🟢🟡🔴), capítulos 13.1–13.16 (pesquisa F1 R1+R2).
> **Critério do fundador:** integrar governança, observabilidade, inteligência de orquestração, auto-aprimoramento e quality gates ao que o mercado busca — construir o que PERDURA e é INEVITÁVEL, não hype.
> **Disciplina epistemológica:** nenhuma afirmação abaixo reafirma claim 🔴 refutado pela auditoria adversarial (tabela de vereditos do doc 13). Onde a evidência é 🟡, isso está dito.
> **Revisão batch 2 (2026-06-06, pós-triagem do Maestro):** nota de terminologia A2A (evita falsa contradição F1-0 × F1-3); donos atribuídos aos sinais de risco; janela competitiva reformulada com incerteza explícita; decisão do fundador incorporada — alvo de render: 8-12 terminais ativos, profiling primeiro.

## Como a direção do fundador mapeia nas ondas

| Direção do fundador | Onda(s) que a materializam | Evidência de mercado |
|---|---|---|
| Inteligência de orquestração | F1-0 | 13.2, 13.11, 13.12 |
| Observabilidade | F1-1 | 13.1 (🟢🟢), 13.5, 13.10 |
| Quality gates | F1-2, F1-3 | 13.3, 13.13, 13.4 |
| Auto-aprimoramento | F1-3 | 13.8 (Curator), 13.4 |
| Governança | F1-6, F1-1, F1-4 | 13.14, 13.1 (🟢🟢), 13.6 |

A tese central, confirmada pelas 2 lentes da auditoria (13.1): **o mercado 2024→2026 pivotou de "flexibilidade de agentes" para governança, auditoria e observabilidade**. O Lina já é event-sourced por decisão da Fase 0 — "auditável por design" é posicionamento que a arquitetura entrega de graça. A direção do fundador não está em conflito com o mercado; está à frente do que os concorrentes locais entregam hoje — a pesquisa estima essa dianteira numa janela da ordem de ~6 meses (13.1 🟡; ver Risco 1 para a justificativa e a incerteza dessa estimativa).

---

## a) Tabela de posicionamento por onda

> **Nota de terminologia (A2A) — evita falsa contradição entre F1-0 e F1-3:** neste documento, "A2A automático" é a capacidade de comunicação agente-a-agente do Lina via **mailbox+router próprio, por pertencimento ao workspace** — decisão da rodada 1 da pesquisa (13.1: "A2A feito simples", não conformidade a padrão de comitê; 13.2: seguir a *semântica* A2A com mecanismo próprio). Coisa distinta é o **A2A Protocol** (padrão externo Google/Linux Foundation): a adoção DELE na Fase 1 foi 🔴 refutada (13.4) — refuta-se o protocolo externo, mantém-se o nosso mecanismo. **A onda F1-0 não é afetada por essa refutação.**

### F1-0 — Coordenação Confiável

- **Demanda:** coordenação é a maior fonte estrutural de falha multi-agente — a taxonomia MAST (NeurIPS 2025, 1.600+ traces) atribui 36,94% das falhas à categoria *coordination* (13.11 🟢). Os fixes P0-P4 têm padrões de solução mapeados (guard Busy/Idle, handoff como primitivo, state machine de lifecycle — 13.2), justificados pelo comportamento local observado (a equivalência direta com taxonomias da indústria foi 🔴 refutada em 13.2 — não a usamos como argumento).
- **Concorrente mais próximo:** Maestri — tem A2A via PTY, mas com **delegação instável, freeze de workspace em sessão longa e command-relay não confiável confirmados na v0.29** (13.12 🟢), e exige conectar cabos manualmente (doc 10). Hermes ainda **não tem A2A automático** (issues abertas — 13.1 🟡).
- **Moat:** A2A automático por **pertencimento ao workspace, sem cabos e sem humano no meio** (gap nº 1 do mercado, doc 10) — via mailbox+router próprio (ver nota de terminologia acima), sobre replay event-sourced com dedup/idempotência já forte no código (13.11 🟢 — o único status "confirmado" da matriz de failure modes). Maestri é snapshot sem replay (13.12 🟢): nossa recuperação é arquiteturalmente superior — desde que provada com injeção de falha.

### F1-1 — Observabilidade e Cognição

- **Demanda:** o pivô para governança+observabilidade é o achado mais forte da pesquisa (13.1 🟢🟢, 2 lentes): governança é prioridade nº 1 para ~75% dos líderes; 95% dos pilotos falham por gaps de integração/governança, não por capacidade de modelo. Tokens/custo/estado por terminal **100% local** é viável via JSONL (armazenamento primário, com asterisco de precisão) + OTel opt-in + hooks (13.5) — sem assumir paridade entre CLIs (13.5/13.10 🟡: Codex tem lacunas; Gemini → Antigravity em transição forçada).
- **Concorrente mais próximo:** Warp Oz — entrega auditoria e orquestração, mas é **puramente nuvem** (os dados saem da máquina — 13.1 🟢). O Ombro do Maestri monitora e resume, mas é Apple-only e não rastreia tokens/custo (13.12 🟢).
- **Moat:** ser o **"Oz local"** (13.1): trilha de auditoria e observabilidade de custo sem nuvem. Nenhum orquestrador local entrega tokens/custo/estado por terminal hoje — e a detecção de CLI por perfil de spawn (13.9, camada determinística ancorada no código) dá ao Lina a "cognição" de saber o que roda em cada nó, que ninguém no espaço tem.

### F1-2 — Experiência

- **Demanda:** modal fatigue é problema real e documentado — usuários aprovam 93% dos prompts; a resposta validada é boundaries + plano antes de executar (13.3 🟢 nos dados Anthropic, 🟡 na causalidade — instrumentar, não assumir). O terminal travado em yes/no é invisível para o não-técnico — a spec de 4 camadas detectar→sinalizar→contextualizar→agir resolve essa dor, com fila unificada e injeção remota tratada como problema de segurança real (CVEs documentados — 13.13).
- **Concorrente mais próximo:** Warp — referência nas notificações Complete/Request/Error (13.3 🟢), mas é dev/enterprise, sem canvas e com dados na nuvem (doc 10). Maestri tem o canvas, mas é dev-facing e a v0.29 só entregou QoL (13.12 🟢).
- **Moat:** **não-técnico genuíno** — modal template-driven de 3-5 controles com co-piloto que sugere papel/comando (13.3), notificação de permissão com contexto e aprovação a um clique (13.13). O cruzamento canvas + cross-platform + não-técnico continua sem dono (doc 10, gap nº 3). Ressalva honesta: "canvas supera chat para leigos" é 🟡 — hipótese de design forte a medir com usuários reais, não fato (13.3).

### F1-3 — Inteligência da Lina

- **Demanda:** "slop" virou problema nomeado e mensurável (palavra do ano 2025; GitClear: duplicação 4-8x em 211M de linhas — 13.4 🟢). **Personalidade de agente emerge de arquitetura (harness, loop, memória, verificação), não de prompt** — 🟢🟢 nas 2 lentes (13.4). SKILL.md consolidou como padrão aberto agnóstico (27-32+ CLIs, 🟢🟢 — 13.4), reforçando a neutralidade multi-CLI. Adotar o A2A Protocol formal (padrão externo) na Fase 1 foi 🔴 refutado (13.4) — a via é nosso mailbox+router próprio; essa refutação não afeta o A2A automático da F1-0 (ver nota de terminologia no topo da seção a).
- **Concorrente mais próximo:** Hermes — o mais avançado em auto-aprimoramento (learning loop + Curator mantendo o catálogo de skills em background — 13.8). Mas é chat-centric, sem canvas, com delegação que exige estrutura explícita (13.8/13.1).
- **Moat:** quality gates **estruturais**: doutrina via skills compostas + cold-review por terminal isolado sem contexto do autor (mecanismo anti-slop comprovado do Superpowers — 13.4 🟢) + opinião estética explícita. O padrão Curator do Hermes é o blueprint do auto-aprimoramento (13.8 — decisão crítica nº 1 da matriz); portá-lo ao formato SKILL.md agnóstico cria algo que o Hermes não tem: auto-manutenção **que funciona em qualquer CLI**.

### F1-4 — Workspaces e PRO

- **Demanda:** local-first provou ser o modelo de negócio defensável: Terragon fechou (jan/2026) e o Vibe Kanban descontinuou a cloud — orquestração cloud pura não se sustentou (doc 10). A mecânica validada: chave ed25519 assinada validada localmente, gating data-driven por nº de workspaces (free=1, PRO=N), grace period e honor system à la Obsidian (13.6 🟢 via plataformas de licensing). Importante: "líderes indie fazem offline puro" foi 🔴 refutado (JetBrains/Sublime/Raycast fazem phone-home) — **ser a exceção sem servidor de contas é diferencial deliberado, não padrão de mercado** (13.6).
- **Concorrente mais próximo:** Maestri — modelo mais parecido (free 1 workspace, Pro $18 one-time, zero cloud/telemetria — 13.12 🟢). O que não tem: cross-platform, e não compete no eixo governança (B2C indie dev vs nosso founder não-técnico — 13.12 🟢).
- **Moat:** privacidade radical **com prova**: sem cadastro, sem phone-home, licença legível e validada na máquina. Para o público não-técnico que está confiando trabalho real a agentes, "seus dados nunca saem do seu computador" + batch de chaves para alunos (13.6) é proposta que nem o Hermes (que exige stack técnica) nem o Warp (nuvem) entregam.

### F1-5 — Render-Scale e Scrollback

- **Demanda:** confiabilidade percebida é dor permanente: perda silenciosa de scrollback sob crash foi 🔴 confirmada NO CÓDIGO ("o que temos basta" refutado — falta flush por idle, signal handler e restore visível — 13.16); o mercado tem cicatrizes públicas (bug de 41 GB do Warp — 13.16). No render, as técnicas são reais e em produção (instancing/atlas do Zed, culling do Figma, suspensão do Chrome), mas as promessas de ganho não se sustentam: o "dirty tracking = 3x imediato" foi 🔴 refutado, o "LOD imperceptível" 🔴🟡, e o "28 @ 40-50fps" está **refutado como promessa de roadmap** (13.7: "roadmap otimista, não ceiling confirmado"; índice 13, achado 9: promessas de ganho do render 🔴 refutadas na magnitude) — a onda abre com profiling e gates medidos na tela, nunca com número prometido (13.7). **Decisão do fundador (triagem 2026-06-06): alvo honesto da Fase 1 é 8-12 terminais ativos, com profiling primeiro.**
- **Concorrente mais próximo:** Maestri — Metal nativo, mas com **freeze em sessão longa confirmado** (13.12 🟢); Zed/Ghostty são a referência técnica, não concorrentes diretos.
- **Moat:** durabilidade **visível** — `Recovered` + "(encerrada)" + "Ver histórico" paginado + retenção 30d (13.16) transforma histórico em ativo de auditoria (alinha com F1-1); e escala provada por benchmark próprio em vez de marketing — exatamente o que diferencia produto que perdura de demo que impressiona.

### F1-6 — Hardening

- **Demanda:** o mercado migrou de "sandbox técnico" para **zero-trust policy-first** — whitepaper da Anthropic (mai/2026) com 6 domínios e 3 níveis de maturidade; 88% das enterprises reportaram incidentes de segurança com agentes em 2025 (13.14 🟢). A crise do ClawHub (1.400+ skills maliciosos em 10.700+, ~12% de maliciosidade nas auditorias) tornou allowlist + code review + hooks de pré-execução obrigatórios (13.14 🟢). Prompt injection é classificado como permanente pelo NCSC — mitigável por política de runtime + observabilidade, nunca eliminável (13.14 🟢).
- **Concorrente mais próximo:** nenhum orquestrador local trata segurança inter-terminal como produto; o OpenClaw/ClawHub é o contra-exemplo do que acontece sem isso. Os CLIs têm sandbox próprio (Seatbelt/bubblewrap), mas isso não resolve autorização **entre** terminais (13.14).
- **Moat:** threat model **honesto** — isolamento same-uid documentado como processual, não vendido como fronteira de kernel (a tese de "suficiência" foi 🔴 refutada — 13.14); allowlist deny-by-default + log intent-vs-action. O discurso é "nós vemos os ataques, bloqueamos em runtime e mantemos trilha de auditoria" — nunca "impedimos ataques" (13.14). Honestidade técnica é diferencial de confiança num mercado onde 88% já se queimou.

---

## b) Teste PERDURA-E-INEVITÁVEL

| Onda | Classificação | Justificativa (evidência, não opinião) |
|---|---|---|
| **F1-0 Coordenação Confiável** | **DURADOURO** | Coordenação é a maior categoria estrutural de falha multi-agente (MAST/NeurIPS 2025: 36,94% de 1.600+ traces — 13.11 🟢), e as reclamações reais da v0.29 do Maestri (delegação instável, freeze — 13.12 🟢) provam que é dor de produto viva. Não é tendência: é condição de funcionamento de qualquer orquestrador. |
| **F1-1 Observabilidade e Cognição** | **INEVITÁVEL** | Pivô para governança/observabilidade confirmado pelas 2 lentes (13.1 🟢🟢), com lastro regulatório (EU AI Act em enforcement, NIST AI RMF 1.1) e econômico (governança prioridade nº 1 p/ ~75% dos líderes; mercado de AI governance ~+34%/ano). Tendência estrutural, não ciclo de hype. |
| **F1-2 Experiência** | **DURADOURO** (núcleo) + **APOSTA** (canvas p/ leigos) | Modal fatigue e o terminal travado em yes/no são dores permanentes com dados primários (93% de aprovação; issues públicas de Codex/Claude Code — 13.3 🟢, 13.13). Já "canvas supera chat para não-técnicos" segue 🟡 sem estudo formal (13.3) — é a nossa aposta de interface, a instrumentar e medir internamente. |
| **F1-3 Inteligência da Lina** | **APOSTA** (sobre fundações confirmadas) | As fundações são 🟢🟢: personalidade emerge de arquitetura, e SKILL.md é padrão agnóstico consolidado (13.4). Mas a "Lina gênio criativo anti-slop" como diferencial de produto e a auto-manutenção estilo Curator (13.8 🟡 — custo/benefício não medido) são apostas nossas: risco maior, potencial de diferenciação maior. |
| **F1-4 Workspaces e PRO** | **DURADOURO** (com componente de aposta deliberada) | Local-first é o modelo que sobreviveu onde cloud pura morreu (Terragon fechado, Vibe Kanban em sunset — doc 10), e a mecânica de licença local assinada é comprovada por plataformas de licensing (13.6 🟢). A aposta consciente: ser exceção sem phone-home (🔴 refutado que "todo mundo faz" — 13.6) — diferencial, com o risco de pirataria assumido via honor system. |
| **F1-5 Render-Scale e Scrollback** | **DURADOURO** | Perda silenciosa de histórico é gap confirmado no nosso próprio código (13.16 🔴 sobre a suficiência atual) e dor pública do mercado (Warp 41 GB). A antiga aposta de escala caiu: "28 @ 40-50fps" foi refutado como promessa (13.7; índice 13, achado 9 🔴) e a decisão do fundador fixa o alvo honesto em **8-12 terminais ativos, com profiling primeiro** — a onda deixa de carregar aposta não medida. |
| **F1-6 Hardening** | **INEVITÁVEL** | Zero-trust para agentes virou framework padronizado (whitepaper Anthropic, mai/2026) com 88% das enterprises reportando incidentes (13.14 🟢); prompt injection é "permanente como SQL injection" segundo o NCSC (13.14 🟢); a crise ClawHub provou o custo de não ter gates (13.14 🟢). Segurança policy-first deixou de ser opcional. |

**Leitura agregada para o épico:** 2 ondas INEVITÁVEIS (F1-1, F1-6), 4 DURADOURAS (F1-0, F1-2-núcleo, F1-4, F1-5), 2 com componente de APOSTA explícita (F1-2-canvas, F1-3). O portfólio é saudável: a base do épico está em tendência estrutural confirmada, e as apostas estão identificadas e instrumentáveis — nenhuma onda repousa sobre claim refutado.

---

## c) Riscos competitivos top-3

### Risco 1 — Hermes Desktop ganhar canvas (e fechar nossa janela de diferenciação)
O Hermes é o concorrente local-first mais maduro (open-source MIT, ~39k commits, ~5 releases/mês, 17k+ testes — 13.8 🟢) e o A2A automático deles está **em planejamento, não entregue** (issues #514/#7708/#4454 abertas — 13.1 🟡). A pesquisa estimou em ~6 meses a janela para o Lina entregar canvas + A2A automático + não-técnico (13.1 🟡 — avaliação de uma lente, não medição). A estimativa é coerente com a velocity observada do Hermes (~5 releases/mês, ~1 tag/semana — 13.8): cadência suficiente para fechar uma lacuna de produto em poucos meses, se eles priorizarem canvas. Tratar como **faixa com incerteza (um semestre, ± um trimestre), recalibrada a cada checagem** — não como deadline.
**Sinal de alerta a monitorar:** issues/RFCs de canvas e A2A no GitHub do hermes-agent mudando para "in progress" (em especial novas RFCs em `.plans/` — onde eles publicam roadmap, 13.8); release notes mencionando "canvas"/"spatial"/"multi-agent view". Cadência sugerida: checagem quinzenal (eles lançam ~1x/semana). **Dono: Maestro** — o clone local `~/hermes-agent` permite `git fetch` + diff de `.plans/` e das release notes sem fricção.

### Risco 2 — Maestri sair do macOS (cross-platform)
A maior fraqueza do Maestri é ser macOS-only/Apple Silicon (doc 10) — e é nosso maior trunfo. Atenuantes baseados em evidência: o stack Swift/SwiftUI/Metal torna um port custoso (13.12 🟢), o roadmap público não menciona cross-platform (13.12 🟢), e a v0.29 foi QoL, não arquitetura (13.12 🟢). Mas é founder solo com tração: se portarem antes do nosso lançamento, o gap nº 3 do mercado (canvas + cross-platform + não-técnico, doc 10) deixa de ser livre pela metade.
Não há base para estimar prazo de um port: o sinal público hoje é ausência (roadmap sem menção a cross-platform — 13.12 🟢), e ausência pode virar anúncio sem aviso — por isso o monitoramento é por release, não por calendário.
**Sinal de alerta a monitorar:** changelog e site do Maestri mencionando Windows/Linux; vagas/posts do founder sobre port ou mudança de stack; aparição do Maestri fora do ecossistema Apple (ex.: sair do Setapp para distribuição própria multi-OS). Cadência: a cada release deles (~quinzenal). **Dono: fundador** — usuário ativo do Maestri, vê releases/changelog em primeira mão; registra o sinal para o Maestro a cada release.

### Risco 3 — O CLI nativo absorver a orquestração visual (plataformas descem ao nosso espaço)
Três frentes do mesmo risco: (a) Claude Code Agent Teams — hoje experimental e desabilitado por padrão (13.1 🟢), mas se virar default com UI, parte do valor de orquestração migra para dentro do CLI; (b) Warp/Oz — 700K+ devs e capital (doc 10); se simplificarem para não-técnicos, dominam — improvável (foco enterprise), mas é o gigante do espaço; (c) Microsoft Conductor — orquestração determinística com gates humanos built-in (13.1), hoje YAML/dev-facing, a um passo de ganhar GUI. O mercado é fragmentado e sem padrão único (🔴 refutado "Claude Code é o padrão obrigatório" — 13.1), o que nos protege: a neutralidade multi-CLI é a defesa estrutural contra qualquer um desses três capturar tudo.
**Sinal de alerta a monitorar:** Agent Teams saindo de experimental (release notes do Claude Code); anúncios do Warp sobre canvas/visual orchestration ou tier não-técnico; Conductor ganhando frontend visual. Cadência: mensal, nas release notes das três plataformas. **Dono: Maestro** — varredura mensal no ciclo de planejamento.

**Menção de watchlist (fora do top-3):** BridgeMind — o mais completo em features e bem financiado (doc 10: cross-platform + roles + mailbox; comunidade 10k+ Discord); se adicionar canvas espacial, vira concorrente frontal. Monitorar releases do BridgeSpace (**dono: Maestro**, na mesma varredura mensal do Risco 3). E a transição **Gemini → Antigravity (EoL 18/jun/2026** — 13.10 🟢) não é risco competitivo, mas é data dura dentro da Fase 1 que afeta a promessa multi-CLI.

---

## d) Mensagem de produto — 3 frases candidatas

1. **"Seu time de inteligências artificiais, trabalhando à vista: o Lina mostra o que cada agente faz, quanto custa e quando precisa de você — tudo no seu computador, nada na nuvem."**
   *(nota do analista, não faz parte da frase — eixo: observabilidade + local-first; traduz "auditável por design" sem jargão)*

2. **"O Lina coordena vários assistentes de IA como um time de verdade: eles conversam entre si, ninguém trava sem avisar, e toda a história fica guardada para você conferir."**
   *(nota do analista — eixo: coordenação confiável + notificação + trilha de auditoria; ataca a dor nº 1 do mercado)*

3. **"Feito para quem não é programador: você descreve o que quer, o Lina monta o time, cobra qualidade do trabalho e te chama só quando a decisão é sua."**
   *(nota do analista — eixo: não-técnico + quality gates + aprovação sem fadiga; é a frase mais "aposta", alinhada à persona Creator)*

---

## Dúvidas para o Maestro

1. **Framing do comprador no épico:** o 13.12 posiciona o Lina como "B2B founder não-técnico (governança)" em contraste ao B2C indie dev do Maestri; outros trechos (13.6, pricing) falam de indie builders/freelancers e alunos. Para a seção POR QUE ISSO GANHA, qual é o framing oficial — founder não-técnico como comprador primário e indie/alunos como secundário?
2. **[RESPONDIDA na triagem 2026-06-06] Alvo de escala da F1-5:** o 13.7 deixava a decisão pendente (28 painéis vs 4-8 ativos). Decisão do fundador: **alvo honesto de 8-12 terminais ativos, com profiling primeiro** — já refletido na F1-5 (seções a e b).
3. **Auto-manutenção de skills (F1-3):** o 13.8 marca como "decisão crítica" — incluir o padrão Curator (revisão por LLM single-pass, sem o termo "quorum", que foi refutado) já na F1-3 ou registrar como decisão e adiar a implementação? A escolha muda o quanto a onda F1-3 é APOSTA.
4. **Pricing da F1-4:** ambas as lentes do 13.6 convergem em "escolher UM modelo na Fase 1" (recomendação: perpetual ~$99; assinatura fica para expansão). Essa decisão já está tomada pelo fundador ou o épico deve carregar as duas hipóteses?
5. **[RESPONDIDA na triagem 2026-06-06] Dono dos sinais de alerta:** donos atribuídos na seção c — Hermes, CLIs nativos e BridgeMind: **Maestro**; Maestri: **fundador** (com registro do sinal para o Maestro a cada release).

---

PRONTO: Posicionamento do Épico F1 entregue e revisado (batch 2) — 7 ondas cruzadas com a pesquisa 13.x (zero claims 🔴 reafirmados), teste perdura-e-inevitável (2 INEVITÁVEIS, 4 DURADOURAS, 2 APOSTAS instrumentáveis), top-3 riscos com sinais de alerta e donos atribuídos, e 3 frases de posicionamento para escolha do fundador.
