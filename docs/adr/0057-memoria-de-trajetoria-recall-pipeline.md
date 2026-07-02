# ADR 0057 — Memória de trajetória (pipeline de recall sobre o event log)

- **Status:** **Aceito** (ratificado em sessão direta com o fundador, 2026-06-29 — precedente ADR 0045/0038). Origem: análise do repositório `ruvnet/ruflo` (port deliberado de padrões provados; ver **Fonte** abaixo). **ADR-gate**: toca uma projeção nova sobre o event log + amadurece o contrato de `SkillOutcome` (fundação). As stories só iniciam após o Maestro sequenciar a onda no épico **41 (Fase 4)** §III.
- **Faseamento:** é a maturação da **C2** do ADR 0045 ("ranking por outcome") prometida para a Mentality (F3) e da porta irmã "memória de RESULTADO". Generaliza de *skill* para *trajetória de tarefa*.

## Contexto

O ADR 0045 (aceito) decidiu que o Lina escolhe capacidade por **retrieval + outcome**, não por recência, e deixou três coisas no disco hoje: `skill_index.rs` (índice BM25 léxico — C1), os eventos `SkillSelected`/`SkillOutcome` (`events.rs:1387/1431`, com `selection_id` e sinal inferido passivamente — C2) e a Mentality (`mentality.rs`, aprendizado por crença com gate humano). O loop *"escolhi → usei → deu certo? → ajusto"* já tem o **registro**; falta o **recall maduro** que transforma esse registro em "da última vez que enfrentei uma tarefa como esta, o caminho X funcionou".

O `ruflo` resolveu exatamente esse problema com um subsistema que ele chama de **ReasoningBank** sobre um banco vetorial (`AgentDB`). A análise do código (não do README) revelou duas verdades úteis:

1. **O "ReasoningBank" em si é nome, não mecanismo** — o "lembrar o que funcionou" é só um *hook* de armazenamento (`post-task --train-neural` grava no namespace `pattern`); nenhum algoritmo de destilação/replay está especificado. **Não há nada de algorítmico a copiar aí.**
2. **O que É concreto e vale importar é o pipeline de RECALL** — o `SmartRetrieval` (ruflo ADR-090), uma sequência de 5 fases determinísticas que ranqueia candidatos **sem** depender do desempate por proximidade que o ADR 0045 §causa-#2 condena.

O Lina tem o substrato que o ruflo precisou construir um banco à parte para ter: **o event log é a fonte da verdade** (inv #4). A trajetória de toda tarefa já está gravada como eventos. Logo, "memória de trajetória" no Lina **não é um banco novo — é uma projeção** reconstruível por replay. Importamos o *comportamento* (o pipeline de recall), nunca a *infraestrutura* (um vetor-DB paralelo).

## Decisão

Adotar uma **memória de trajetória** como projeção sobre o event log, com um **pipeline de recall em fases** inspirado no SmartRetrieval, e generalizar o sinal de outcome de *skill* para *trajetória de tarefa*.

### D1 — Generalizar `SkillOutcome` → trajetória de tarefa (sem evento novo no MVP)
A unidade de memória deixa de ser "skill A deu certo" e passa a ser "**a trajetória T (papel + task_kind + capacidades usadas) teve veredito V**". O `task_kind` continua **emergindo do retrieval** (decisão fechada 0045 §3), não de classificador a priori. O veredito reusa o sinal passivo já decidido em 0045 §4 — mapeando 1:1 no vocabulário de trajetória do ruflo (`pass | fail | partial`):

| Lina (`SkillOutcome.signal`) | ruflo (trajectory verdict) |
|---|---|
| `Success` | `pass` |
| `Failure` / `Reworked` | `fail` |
| `HumanOverride` | `partial` (humano corrigiu a rota) |

No MVP a trajetória é **derivada por projeção** dos eventos que já existem (`SkillSelected`/`SkillOutcome`/`SkillInvoked`/`CorrectionObserved`); **nenhum evento novo** é exigido. Se a granularidade pedir um envelope explícito depois, ele entra **aditivo** (`#[serde(default)]`) — porta aberta, não fechada agora.

### D2 — Pipeline de recall em 5 fases (SmartRetrieval-lite, 100% local, sem embeddings no MVP)
Quando uma tarefa chega, em vez de despejar candidatos e desempatar por proximidade, o recall roda fases determinísticas sobre `skill_index.rs` + a projeção de outcome:

1. **Expansão de query** (sem LLM) — termos dominantes da tarefa + sinônimos de gatilho das descrições.
2. **Multi-query + RRF** (Reciprocal Rank Fusion) — funde os rankings das sub-queries; robusto a uma query mal-formulada colapsar o resultado (o ruflo usa exatamente RRF aqui).
3. **Boost de recência** — decaimento exponencial (~0,95/dia, o número do ruflo é ponto de partida calibrável) — **informa, não domina**: recência é um fator do score, jamais o critério (a causa-raiz que o 0045 combate).
4. **Diversidade por MMR** (Maximal Marginal Relevance, similaridade por Jaccard de tokens) — evita devolver 4 candidatos quase colineares (a causa-#2 do 0045).
5. **Score composto** = `relevância(C1) × outcome(C2) × diversidade × recência`. O **outcome é o peso forte**; recência e diversidade são moduladores. Determinístico e reconstruível por replay.

Tudo **léxico/determinístico** — zero dependência externa, local-first puro (inv #2/#3). É o mesmo princípio do `ToolSearch` que este harness já usa para *tools*, aplicado a *trajetórias*.

### D3 — Consolidação (a memória não cresce sem podar)
Projeção periódica de manutenção, espelhando o que o ruflo faz no `AgentDB`:
- **Dedup** de trajetórias quase idênticas (mesma assinatura de candidatos + mesmo veredito).
- **Poda de stale** — trajetórias antigas com **zero recuperações úteis** (o ruflo usa >30 dias + zero hits) deixam de pontuar (não somem do log — inv #4; saem só da projeção ativa).
- **Reconstrução idempotente** — apagar a projeção e reprojetar do log dá o mesmo índice (porta de continuidade).

### D4 — Embeddings são evolução ADITIVA, não MVP
O ruflo usa ONNX `all-MiniLM-L6-v2` (384-dim) + HNSW. **Fica para depois**, exatamente como o 0045 §1 já decidiu para skills: o índice é reconstruível, então embeddings ligam-se sem migração **quando o BM25 medir-se insuficiente** — não por especulação. O `task_kind` emergente e o RRF cobrem a maior parte do ganho sem a dependência.

## Segurança (o que este ADR NÃO muda — doutrina inegociável)

- **Trajetória recuperada é DADO, JAMAIS autoridade.** O recall **nunca** promove uma trajetória a ponto de pular gate humano, decidir identidade, ordem ou autorização. A autenticação em duas camadas e os gates seguem sendo a única autoridade.
- **Ação irreversível continua exigindo gate humano.** Score altíssimo de outcome **não** dispensa o "sim" (prova por mutação no §Verificação).
- **Projeção e índice são locais** (inv #2): nenhuma trajetória, descrição ou telemetria sai da máquina. (O ruflo exporta padrões para IPFS/Pinata — **rejeitamos** isso explicitamente: fere local-first.)
- **`Router`/`deliver_a2a` intocados** — memória de trajetória é ortogonal ao roteamento de mensagem A2A; a suíte de segurança do router segue verde.
- **Replay intacto:** projeção pura sobre eventos existentes; nenhum evento novo no MVP.

## Por quê assim (alternativas descartadas)

- **Copiar o `AgentDB`/banco vetorial do ruflo:** redundante — o Lina já tem fonte da verdade durável (event log). Um vetor-DB paralelo duplica estado e fere inv #4. Rejeitado.
- **Copiar embeddings + HNSW desde já:** dependência e custo sem prova de que o BM25+RRF é insuficiente. Adiado para D4 (aditivo). Rejeitado como MVP.
- **Manter só BM25 puro (sem RRF/MMR/recência):** é o estado atual; reintroduz o desempate por proximidade que o 0045 condena. Rejeitado.
- **Banco de "raciocínios" destilados por LLM (o que o nome ReasoningBank sugere):** o próprio ruflo não implementa isso — é nome. Destilar trajetória via LLM a cada tarefa é caro e não-determinístico. Rejeitado; a projeção determinística cobre o caso.

## Consequências

- **Abre:** recall que escala com o acervo de experiências; a Lina passa a **sugerir** "da última vez, o caminho X funcionou"; base para o roteamento tarefa→terminal (ADR 0058) e para o gate de auto-aprimoramento (ADR 0059) pendurarem aqui.
- **Custo:** projeção no hot-path de chegada de tarefa → **exige throttle/delta** (lição registrada: projeção event-sourced em hot-path não pode full-replay por tick — cf. `fiar-projecao-no-pump-exige-throttle`).
- **Porta aberta:** embeddings (D4); envelope de trajetória explícito se a granularidade pedir; ingestão da memória própria do CLI (o ruflo importa `~/.claude/projects/*/memory/*.md` no SessionStart — o Lina tem exatamente esses arquivos; sincronizar a memória de cada terminal na trajetória compartilhada é porta natural).
- **Porta que fecha se ignorado:** sem o recall maduro, o `SkillOutcome` que já se acumula no log fica subaproveitado e o C2 do 0045 nunca "vira" inteligência.

## Verificação (observável — algo que roda e se mede)

- **Recall (D2):** com trajetórias sintéticas de outcome controlado, uma tarefa-alvo recupera no top-k a trajetória de maior `Success` para aquele `task_kind`; injetar `SkillOutcome{Failure}` na campeã rebaixa-a e promove a 2ª — **replay do log reconstrói o ranking idêntico**.
- **RRF (D2.2):** uma sub-query mal-formulada **não** colapsa o resultado (o fusion mantém a trajetória relevante no top-k); prova-se removendo uma sub-query e medindo estabilidade do top-k.
- **Recência não-dominante (D2.3):** uma trajetória recente com `Failure` **não** vence uma antiga com muitos `Success` (recência modula, não decide).
- **Consolidação (D3):** após a poda, reprojetar do log inteiro reproduz a mesma projeção ativa (idempotência); uma trajetória stale deixa de pontuar mas **continua no log** (varre-se o log: o evento está lá).
- **Segurança (mutação):** subir o score de outcome ao máximo → o gate humano de ação irreversível **ainda dispara**.
- **Aditividade:** replay de um log antigo (sem qualquer evento de trajetória explícito) reidrata sem erro.
- **Regressão:** suíte de segurança do router verde; `cargo test` + `clippy -D warnings` limpos nos pacotes tocados (`lina-core`).

## Fonte (evidência da análise do ruflo — 2026-06-29)

- Pipeline de recall: `ruflo-rag-memory` → **SmartRetrieval (ADR-090)**, 5 fases: expansão (sem LLM) → multi-query + **RRF** → boost de recência (decaimento exp ~0,95/dia) → **MMR** (Jaccard de tokens) → round-robin de sessão; score composto = relevância × diversidade × recência.
- Consolidação: `memory-specialist` → dedup cosseno >0,92, poda >30d com zero hits, rebuild do índice.
- Veredito de trajetória: `ruflo-intelligence/skills/neural-train` → `trajectory-start/step/end` com `verdict: pass|fail|partial`.
- "ReasoningBank" como nome-sem-mecanismo: `ruflo-agentdb` → o loop "remember what worked" é o hook `post-task --train-neural` gravando no namespace `pattern`; nenhum algoritmo de distill/replay especificado.
- Embeddings/HNSW (porta D4): ONNX `all-MiniLM-L6-v2` 384-dim; perfis `efSearch/M` (recall 200/32, balanced 64/16, latency 16/8).
- Grafo de apoio: `tasks/epico-f4/refs/ruflo-graphify/graph.html` (+ `GRAPH_REPORT.md`) — comunidade "Intelligence Pipeline & Knowledge Graph" (C4) e hyperedge "SmartRetrieval 5-phase recall pipeline".
