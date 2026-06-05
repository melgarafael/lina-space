# Observação ativa do Lina Space — notas de campo (workspace walking-skeleton, 8 terminais A–H)

## ACHADOS (com evidência de código + event log)

### 🔴 #1 [BUG, alto impacto] Perfil de ENTREGA errado — causa-raiz UNIFICADA dos 2 sintomas do fundador
O app entrega A2A com `demo_profile()` (Onda 0, feito p/ `sh`): `prompt_ready_regex='.'`, `submit_delay_ms=150`.
O perfil CERTO `profiles/claude-code.toml` tem `prompt_ready_regex='(?m)^\s*[>❯]\s'` (casa o prompt real do
Claude) e `submit_delay_ms=300`. **O claude-code.toml NUNCA é carregado em produção** (só em teste do inspector).
Locais que usam demo_profile: `bridge.rs:395` (MailboxPump), `main.rs:2355`, `main.rs:2385`, `bridge.rs:3128`.
Mecânica (a2a.rs:242-256): `wait_ready(regex)` → paste → `sleep(submit_delay)` → Enter(0x0D).
- **Sintoma "lina ask vira texto colado, não envia" (intermitente):** `wait_ready` com `.` passa SEMPRE (não
  espera o Claude estar no prompt) + `submit_delay=150ms` curto → o Enter chega antes do paste assentar →
  às vezes não submete. Timing-dependente = intermitente, exatamente como o fundador descreveu.
- **Sintoma "respostas se atropelam":** `wait_ready` inerte injeta EM CIMA de uma resposta em andamento de A.

### 🟠 #2 [DESIGN, atropelamento] Sem coordenação de retorno / sem checagem de Idle
`deliver_fn` (bridge.rs:406-419) injeta DIRETO no PTY do alvo, sem checar `status` (Busy/Idle). `READY_TIMEOUT=2s`
(a2a.rs:39) é curto p/ respostas longas do Claude → expira e injeta no meio. NÃO há agregação: N colegas
respondendo a A = N injeções separadas. Evidência no log: A faz fan-out p/ B–H (seq 1808-1831, +0.3s entre
cada); retornos a A se atropelam (seq 1853 G→A, +0.6s depois 1860 C→A).

### 🟡 #3 [INCONSISTÊNCIA] intent `status` não-canônico
Agentes usam `lina ask --intent status` (seq 1860, 2899). Não é intent canônico (ask/handoff/broadcast/review).
Funciona (tratado como delegação genérica) mas não documentado — improvisação do agente.

### 🟡 #4 [FALTANTE] `lina check` / `lina handoff` não existem (doutrina promete)
CLAUDE.md:142-163 promete `lina handoff "@X" "tarefa" --context` e `lina check "@X"`; o bin não os tem. O
ScrollbackStore foi DESENHADO p/ `lina check` ler a tela (scrollback.rs:30) mas `scrollback.db` NEM EXISTE no
workspace (não persiste). [já diagnosticado; implementação fica p/ a fase de otimização]

### ✅ #5 [POSITIVO] A race de roteamento está RESOLVIDA
Zero RouteBlocked no histórico recente (5391+ eventos); o retry transiente (f353838) + estabilização seguram.
18 MessageRouted → 24 MessageDelivered, todas entregues.

### 🟠 #6 [DESIGN] Topologia em ESTRELA + falta de ferramenta de coordenação (agrava o atropelamento)
TODA comunicação envolve A (hub): nenhum B↔C/C↔D etc. A é gargalo — recebe todas as respostas. `--await`
quase não é usado (1 de 19 msgs): a doutrina DESENCORAJA await (anti-deadlock, CLAUDE.md:160) e manda usar
`lina check` — que NÃO EXISTE (#4). Os agentes ficam sem coordenação: fire-and-forget cego → todos atropelam
A. Custo alto (13.460 TokenUsageReported, mas 0 evento de teto — token_budget folgado/desligado).

### 🟠 #7 [DESIGN/ADOÇÃO] O sistema de PLANO compartilhado NÃO é usado
ZERO eventos de `lina plan claim/running/review/check` em 13,8h. A doutrina tem um sistema inteiro de
coordenação via `.lina/plan.md` (claim de tarefa, status, anti-colisão) — os agentes o IGNORAM e coordenam
100% por `lina ask` ad-hoc pra A. Sem claim, ninguém sabe quem está fazendo o quê → A vira o único ponto
de coordenação (gargalo + atropelamento). O mecanismo existe mas a adoção é zero (doutrina não convence?).

### 🟡 #8 [CUSTO] Consumo massivo não-monitorado
15.478 TokenUsageReported em 13,8h (~338/min nos últimos 5min — alta atividade). 0 evento de teto de custo
(token_budget_day folgado/desligado). 8 agentes Claude rodando horas = custo real alto, sem visibilidade.

### 🟡 #9/#10 [LIFECYCLE] Reabrir o app re-spawna tudo, deixa nós-fantasma no log
3 gerações de terminais (3→6→8) = 3 reaberturas do app no mesmo workspace. ZERO crash (estabilidade boa!),
mas o reabrir NÃO emite NodeRemoved/Dead pros antigos → log tem 17 NodeAdded / 0 NodeRemoved, não reflete os
8 vivos. O roster vivo vem do Supervisor, não do replay do log → tensão com inv#4 (log = fonte da verdade).

## ✅ O QUE FUNCIONA BEM (reconhecer)
- **Estabilidade:** 8 agentes Claude reais rodando ~14h, 338 token-reports/min, ZERO crash registrado.
- **A2A entrega** quando usada (race resolvida pelo retry transiente; 20/20 roteadas → entregues).
- **Persistência durável:** event log com 15k+ eventos, espelho jsonl, sem corrupção.
- **Spawn/PTY/render/IME** funcionam: os agentes Claude rodam de verdade, processam, respondem.
- **Infra de segurança presente:** custódia (lina do), guard, freio, autenticação anti-impersonação.

## RETRATO (como o Lina funciona na prática, 8 agentes / 14h)
Os agentes trabalham ~95% ISOLADOS (15k token-reports vs 20 msgs A2A), em ESTRELA centrada em A, SEM plano,
SEM await, comunicando-se esporadicamente por ask. A "cooperação sem fios" acontece, mas em baixa frequência
e SEM estrutura — toda coordenação é ad-hoc pra A. Quando 2+ colegas terminam juntos, atropelam A (a entrega
não checa se A está ocupado). O perfil de entrega errado (demo) faz o paste às vezes não submeter. O núcleo
(spawn/render/persistência/roteamento) é SÓLIDO; a CAMADA DE COORDENAÇÃO é o que falta amadurecer.

## FIX PROPOSTO (núcleo, resolve #1 e ataca #2)
Carregar o `claude-code.toml` (via `ProfileRegistry` que já existe) e usá-lo no `deliver_fn` em vez do
demo_profile. Avaliar: aumentar `submit_delay` (300→500?), aumentar `READY_TIMEOUT` (2s→?), e/ou adiar entrega
quando o alvo está Busy (status) com agregação. Validar o regex do prompt contra o grid real do Claude Code.
