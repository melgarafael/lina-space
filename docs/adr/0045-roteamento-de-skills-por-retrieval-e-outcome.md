# ADR 0045 — Roteamento de skills por retrieval + outcome (seleção certa em escala, não por recência)

- **Status:** **Aceito** (5 decisões de design fechadas em sessão direta com o fundador, 2026-06-24 — espelha o precedente de aceitação por instrução direta do ADR 0038; origem: dogfooding, ideia registrada em `Debriefing Lina.md → §Futuro`). As escolhas estão em **Decisões fechadas** abaixo. **ADR-gate** (toca provisionamento de skills + projeção nova sobre o event log — fundação): as stories só iniciam após o Maestro **sequenciar as fases F1/F3 no épico**.

## Contexto

Conforme o acervo de capacidades cresce (dezenas de plugins, centenas a milhares de skills, todas boas em sua especialidade), a IA **deixa de saber QUAL skill usar QUANDO** e cai no pior critério possível: a skill "mais próxima no histórico/memória". O sintoma é **recência vencendo relevância**, e a causa raiz **não é o prompt** — é o mecanismo de seleção.

Hoje o CLI (Claude Code incluso) seleciona skill assim: despeja as `description` de **todas** as skills no contexto e deixa o modelo casar por similaridade semântica contra a tarefa. Esse mecanismo tem teto. Funciona com ~15 skills; **colapsa com ~1000** por três motivos simultâneos:

1. **Não cabe.** 1000 descrições competem por espaço e atenção; ou trunca, ou o sinal dilui no ruído.
2. **Overlap semântico.** Skills boas sobre o mesmo domínio têm descrições parecidas → quanto melhor o acervo, **pior a discriminação** (4 ótimas skills de code-review viram 4 vetores quase colineares).
3. **Sem sinal de resultado, o desempate vira proximidade.** Quando dois candidatos parecem iguais, decide o que está mais perto no contexto — recência.

É um problema de **information retrieval em escala** ("tool selection at scale"), disfarçado de problema de prompt.

**O que o Lina já tem (e um CLI mono-agente não):**
- **Particionamento por papel** — o ADR 0038 (§Consequências) já registrou a porta: *"kit seletivo por papel — o ponto de extensão é a seleção da safra em `install_skills_into`, nunca a política de cwd"*. Hoje paramos no kit interno de 12 skills.
- **Event log como fonte da verdade** (inv #4) — permite **memória de outcome**, não só de recência: cada escolha+uso+resultado é um evento; uma projeção reconstrói "skill X, no contexto Y, deu certo/errado". É o fio condutor **"inteligência da Lina"** da F1.
- **Papel CURADOR + vault** — para podar redundância e ancorar preferência do usuário.

**Por que registrar agora (porta acima):** a solução exige (a) decidir como a fatia de skills por papel é provisionada (estende o ADR 0038), (b) um **índice de retrieval** novo no disco e (c) um **evento de domínio novo** com sua projeção. São decisões que fecham portas de arquitetura → ADR antes de código (regra de processo do `CLAUDE.md`).

## Decisão

Adotar um **roteador de skills em camadas** — o oposto de "todas as descrições no contexto competindo por proximidade". A seleção passa de *match semântico sobre todo o acervo no prompt* para *retrieval de poucos candidatos + ranking por resultado histórico*.

### C0 — Particionamento por papel (filtro grosso)
Cada terminal só considera a **fatia de skills do seu papel** + as universais (o kit Lina). DEVELOPER não vê copy/astrologia; WRITER não vê backend. Corta a maior parte do ruído antes de qualquer busca. **Fonte da fatia:** extensão do ponto já previsto no ADR 0038 (`install_skills_into` recebe o papel) — a política de cwd **não** muda; muda a *seleção da safra*.

### C1 — Retrieval (índice no disco, não no contexto)
Indexar as `description` das skills da fatia do papel num **índice local**. **MVP: busca léxica/BM25 pura** — zero dependência externa, 100% local-first; as descrições de skill já trazem gatilhos explícitos, sobre os quais o BM25 discrimina bem. **Embeddings/semântica entram como evolução aditiva** (o índice é reconstruível → liga-se depois sem migração). Quando a tarefa chega, recuperar os **top-k** candidatos (k ≈ 5–10) sob demanda, em vez de despejar o acervo no prompt. Determinístico, **local-first**, rápido. É o **mesmo padrão que o CLI já usa para _tools_** (deferred tools + `ToolSearch`), aplicado a _skills_. O índice é uma **projeção reconstruível** (inv #4): apagá-lo e reindexar a partir do disco é sempre válido.

### C2 — Ranking por outcome (o diferencial do Lina)
Entre os top-k de C1, escolher por um score que combina:
- **match de retrieval** (C1),
- **histórico de sucesso** daquela skill naquele *tipo de tarefa* — uma **projeção sobre o event log**,
- **preferência do usuário** (vault / decisões do plano).

O event log dá **memória de RESULTADO**, fechando o loop *"escolhi → usei → deu certo? → ajusto a próxima escolha"* — que nenhum agente mono-processo tem. **Replay do log reconstrói o score** retroativamente, sem retreino (padrão das projeções F1; cf. ADR 0005/0033).

O **sinal de outcome é inferido PASSIVAMENTE** (decisão do fundador; ver Decisões fechadas §4): a Lina **nunca pergunta** ao usuário. Sem troca da skill pelo humano e sem re-trabalho na janela seguinte → `Success`; `HumanOverride` (humano/Maestro trocou a skill) ou entrega devolvida (`Reworked`) → `Failure`. Reforço barato e já existente: gate/cold-review PASS. Respeita o TOM não-técnico (zero fricção para o leigo).

### C3 — Desempate
Empate entre candidatos **+ ação cara/irreversível** → **gate humano** (doutrina existente). Caso comum → o terminal **decide e narra** em pt-br (TOM), sem despejar o raciocínio técnico ao leigo.

### Curadoria (papel CURADOR) — metade do problema é acúmulo, não seleção
- Audita **redundância** ("4 skills de code-review; uso real mostra que você dispara 1") e **propõe consolidação** — **sugere, nunca aplica** (gate humano).
- Reescreve descrições de skills próximas com **gatilhos disjuntos** para maximizar discriminabilidade (ataca a causa #2 direto).
- Resultado: reduzir 1000→~200 boas é metade da cura; o roteador resolve a outra. Skills especializadas viram **ativo** (quanto mais, melhor o roteador), não passivo.

### Evento de domínio novo (contrato C2)
Par de eventos **aditivos** (`#[serde(default)]` em todo campo novo → replay de log antigo nunca quebra; inv "eventos aditivos"):
- `SkillSelected { task_kind, candidates: [skill_id…], chosen: skill_id, by_role, rank_reason }`
- `SkillOutcome { selection_ref, signal: Success | Failure | Reworked | HumanOverride, … }`

O `task_kind` e o sinal de `SkillOutcome` são o miolo das Questões em aberto.

## Segurança (o que este ADR NÃO muda — doutrina inegociável)

- **Skill escolhida é DADO transportado, JAMAIS autoridade.** O outcome-ranking **nunca** promove uma skill a ponto de pular gate humano, decidir identidade, ordem ou autorização. A autenticação em duas camadas e os gates permanecem a única autoridade (cf. `CLAUDE.md → Doutrina de segurança`).
- **Ação irreversível continua exigindo gate humano** — o C3 reforça, não relaxa.
- **Índice (C1) e histórico (C2) são locais** (inv #2 local-first): nenhuma descrição ou telemetria de uso sai da máquina.
- **`Router`/`deliver_a2a` não são tocados** → a suíte de segurança do router permanece intacta. Roteamento de *skill* (qual capacidade um terminal usa internamente) é ortogonal a roteamento de *mensagem A2A* (entre terminais).
- **Replay intacto:** eventos aditivos; log antigo sem os eventos novos reidrata trivialmente.

## Por quê assim (alternativas descartadas)

- **Manter tudo no contexto e só "melhorar as descrições":** paliativo; não ataca (a)/(c). Reescrever descrições é *parte* da Curadoria, não a solução. Rejeitado como solução isolada.
- **RAG puro (só C1, sem C2):** recupera bem, mas o desempate volta a ser similaridade — reintroduz o erro por overlap. Perde o diferencial do Lina (o event log). Rejeitado isolado.
- **Modelo de roteamento fine-tunado:** caro, não-local, não-event-sourced, acopla a um provedor → fere neutralidade multi-CLI (inv #3) e local-first (inv #2). Rejeitado.
- **LLM-as-router puro (perguntar ao modelo "qual das 1000?"):** é exatamente o mecanismo que já falha — sem retrieval o modelo não "vê" 1000 com discriminação. Rejeitado.
- **Regras determinísticas hardcoded (mapa tarefa→skill):** rígido, não aprende, vira dívida no primeiro acervo novo. Rejeitado.

## Consequências

- **Abre:** seleção que escala com o acervo; skills especializadas viram ativo; base para o Curador medir uso real; auto-aprimoramento do roteamento por replay (sem retreino).
- **Custo:** 1º índice por papel/fatia (amortizado, idempotente — só reindexar no delta); uma projeção nova no hot-path de seleção → **exige throttle/delta** (lição registrada: projeção event-sourced em hot-path não pode full-replay por tick).
- **Estende o ADR 0038** (`install_skills_into` por papel) sem mexer na política de cwd.
- **Porta aberta:** o mesmo roteador serve para escolher **terminal/modelo/effort** por tarefa (bullet irmão do Debriefing, linha "inteligência de terminal") — C2 é genérico o suficiente.
- **Porta que fecha se ignorado:** sem o evento de outcome desde já, o histórico não acumula e o C2 nasce cego — por isso o contrato do evento entra **antes** do código.

## Decisões fechadas (sessão direta com o fundador, 2026-06-24)

1. **Retrieval (C1):** **BM25/léxico puro no MVP**; embeddings (embedder local onnx/fastembed) como evolução aditiva. Justificativa: zero dependência externa, local-first puro (inv #2/#3), e descrições de skill já são orientadas a gatilho.
2. **Escopo do índice e do histórico:** **índice global** (derivado das descrições, reconstruível); **eventos de outcome no event log do workspace onde ocorrem** (fonte da verdade, inv #4); o **ranking é uma projeção que AGREGA os logs de todos os workspaces** (multi-workspace, ADR 0010) — o aprendizado **soma entre projetos**, não recomeça por projeto. MVP pode começar no workspace corrente e generalizar.
3. **`task_kind` (agrupamento de contexto):** **emerge do próprio retrieval** — tarefas que recuperam o mesmo conjunto de candidatos são "do mesmo tipo"; **nenhum classificador a priori**. MVP: papel + termos dominantes da query. A granularidade se auto-ajusta com dados (evita over-fitting e balde genérico de uma vez).
4. **Sinal de sucesso:** **inferência passiva — a Lina NUNCA pergunta** (escolha do fundador). Negativos fortes e baratos: `HumanOverride` (humano/Maestro trocou a skill) e `Reworked` (entrega devolvida). Positivo: skill escolhida + sem troca + sem re-trabalho na janela seguinte (reforçado por gate/cold-review PASS, que já existem). Sem 👍 explícito → respeita o TOM não-técnico e evita fricção. Definição operacional fina cabe num adendo (cf. ADR 0019).
5. **Faseamento:** **C0+C1 (papel + busca léxica) na F1**; **C2 (ranking por outcome) amadurece com a Mentality (F3)** — `SkillSelected`/`SkillOutcome` é primo do `CorrectionObserved` (spec 35), mesma máquina de aprendizado por evidência. O **evento já nasce logado na F1** (mesmo sem ranquear), senão o histórico não acumula e a C2 nasce cega. Sequenciamento fino no épico: **a cargo do Maestro**.

## Verificação (observável — algo que roda e se mede)

- **C1 (retrieval):** com N skills sintéticas de descrições controladas, uma tarefa-alvo recupera a skill relevante no **top-k**; reindexar após apagar o índice produz o mesmo resultado (projeção reconstruível).
- **C2 (outcome):** dado o mesmo top-k, injetar eventos `SkillOutcome{Success}` para a skill A e `{Failure}` para a B muda a escolha de B→A; **replay do log** reconstrói o score idêntico.
- **Sinal passivo (C2/§4):** uma seleção sem `HumanOverride` e sem `Reworked` na janela seguinte projeta `Success`; uma com troca humana projeta `Failure` — **sem nenhum prompt ao usuário** (varre-se o caminho: nenhuma pergunta de feedback é emitida ao leigo).
- **C0 (papel):** um terminal de papel X não recebe na fatia uma skill exclusiva do papel Y (estende o teste do ADR 0038).
- **Segurança:** uma skill com altíssimo score de outcome **não** dispensa o gate humano numa ação irreversível (prova por mutação: subir o score ao máximo → o gate ainda dispara).
- **Aditividade:** replay de um log **sem** `SkillSelected`/`SkillOutcome` reidrata sem erro (campos `#[serde(default)]`).
- **Regressão:** suíte de segurança do router verde; `cargo test` + `clippy -D warnings` limpos nos pacotes tocados.
