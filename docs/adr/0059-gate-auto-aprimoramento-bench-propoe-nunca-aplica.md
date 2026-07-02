# ADR 0059 — Gate de auto-aprimoramento: medir e PROPOR, nunca auto-aplicar

- **Status:** **Aceito** (ratificado em sessão direta com o fundador, 2026-06-29 — precedente ADR 0045/0038). Origem: análise do `ruvnet/ruflo` (subsistema "metaharness/Darwin"), que **valida de forma independente** a invariante do fundador "auto-aprimoramento: sugere, nunca aplica". **ADR-gate**: governança — define como uma proposta de melhoria de harness/skill é medida e oferecida ao humano. Sequenciamento no épico **41 (Fase 4)** §III; amadurece com a Mentality (F3).
- **Depende de:** ADR 0057 (memória de trajetória — o sinal de "o que funcionou") + `mentality.rs` + eventos `BeliefProposed→…→BeliefEstablished` (o gate humano já existe).

## Contexto

Um dos fios condutores do fundador é **auto-aprimoramento que sugere, nunca aplica** — gate humano obrigatório. O Lina já tem a metade do mecanismo: a Mentality (`mentality.rs`) observa `CorrectionObserved`, propõe `BeliefProposed`, reforça, e só **estabelece** (`BeliefEstablished`) sob aval. Falta a parte que **mede** se uma mudança candidata (uma skill reescrita, uma política de retry, um briefing melhor) é de fato melhor — hoje a proposta é qualitativa.

O ruflo construiu exatamente essa camada de medição — e, surpreendentemente para o "harness maximalista", **chegou à mesma conclusão de governança que o fundador**. O subsistema `metaharness` ("Darwin Mode") evolui 7 superfícies de política (planner, contextBuilder, reviewer, retryPolicy, toolPolicy, memoryPolicy, scorePolicy), mas o código é **explícito**:

> *"Sem `--confirm`: imprime o plano e sai (exit 0)."* · *"Darwin Mode is human-initiated… not for autonomous self-modification."* · *"ADR-153 §5 explicitly rejects auto-evolving ruflo."*

Ou seja: **medir é automático; aplicar é humano.** É a nossa doutrina, comprovada por um terceiro que tentou o caminho agressivo e recuou. Isso nos dá tanto a validação quanto o **mecanismo de medição** que faltava.

## Decisão

Uma camada de **bench + score** que avalia mudanças candidatas contra um corpus fixo e **emite uma PROPOSTA** ao humano — jamais aplica. Reusa o gate `BeliefProposed→BeliefEstablished` que já existe.

### G1 — Score: cartão de prontidão multi-dimensão (read-only)
Uma projeção que pontua uma capacidade/harness em dimensões objetivas, espelhando o `harness-score` do ruflo (`harnessFit / compileConfidence / taskCoverage / toolSafety / memoryUsefulness`), adaptadas ao Lina: **fit ao papel · confiança de gate (cold-review PASS rate) · cobertura de task_kind · segurança de ferramenta · utilidade de memória (0057)**. Puro-leitura, determinístico, sai de uma projeção sobre o log. Serve de **linha de base** para comparar candidatos.

### G2 — Bench: corpus fixo de avaliação (desacoplado dos testes do repo)
Um conjunto pequeno e versionado de tarefas-alvo `{entrada, resultado_esperado, peso}` — o `harness-bench` do ruflo, *"decoupling evolution from the repo's natural tests"*. É contra **este** corpus que uma mudança candidata é medida, não contra o humor do dia. Local, reconstruível.

### G3 — Proposta de mudança = **uma** mutação, medida, oferecida
Uma melhoria candidata altera **uma** superfície por vez (o ruflo: *"one mutation per variant… causal attribution stays clean"*). O fluxo:
1. Gera a variante (ex.: descrição de skill com gatilhos disjuntos — já é trabalho do Curador no 0045; política de retry; briefing).
2. **Mede** contra o bench (G2) e o score (G1), em sandbox.
3. **Promove só ganho medido** — mas "promover" aqui significa **emitir `BeliefProposed{ proposta, evidência: delta de score }`**, que vira `BeliefEstablished` **só com aval humano**. O modelo/skill em produção **não muda** sem o "sim".

### G4 — Nunca contínuo, nunca em background autônomo
Espelha o `--confirm`/exit-0 do ruflo e a regra de gate do `CLAUDE.md`: a avaliação é **iniciada por humano ou por marco** (fim de onda, retro), **não** um loop de auto-modificação rodando sozinho. O resultado de uma rodada sem `--confirm` é **um relatório + propostas**, não uma aplicação.

## Segurança (o que este ADR NÃO muda — e reforça)

- **A proposta é DADO, jamais autoridade.** Nenhum delta de score promove uma mudança a produção sem `BeliefEstablished` humano. A barreira é o gate que já existe; este ADR **alimenta** o gate, não o contorna.
- **Tripwire de segurança herdado:** o ruflo tem um exit-99 "safety-disqualified" (secrets/shell-out/network/dynamic-eval) que **desqualifica** uma variante antes de medir. O Lina reusa sua própria guarda (`guard.rs`/`command_touches_secret`, `bypass-nao-desliga-hooks`): uma variante que toque segredo/egress é **desqualificada**, não pontuada.
- **Auto-aplicação é proibida por construção** — não existe caminho de código da medição para a produção sem o evento de aval humano. Provado por mutação (§Verificação).
- **Local-first** (inv #2): bench, score e sandbox são locais; nenhuma telemetria de avaliação sai da máquina.

## Por quê assim (alternativas descartadas)

- **Auto-evoluir (aplicar a variante vencedora sozinho):** o próprio ruflo **rejeita** (ADR-153 §5) e o fundador proíbe. Fere a invariante. Rejeitado — é a linha vermelha.
- **Loop contínuo de otimização em background:** o ruflo marca como "NÃO use para isso". Custo + risco de drift silencioso sem humano no circuito. Rejeitado (G4).
- **Proposta sem medição (só "acho que melhora"):** é o estado atual da Mentality; sem bench, a proposta é fé. Este ADR adiciona a evidência. Rejeitado como suficiente.
- **Score/bench como gate de CI que barra merge:** o ruflo permite (`--alert-on-fit-below` exit 1), mas no Lina o gate de mérito já é o cold-review/verification humano; transformar score num portão automático arrisca falsos negativos. Score **informa** o humano, não barra sozinho. Rejeitado como gate duro.

## Consequências

- **Abre:** a Mentality ganha evidência quantitativa para suas propostas; o Curador (0045) pode medir "esta consolidação de skills melhora o roteamento?" antes de propor; auto-aprimoramento deixa de ser slogan e vira loop com prova — **sempre terminando no humano**.
- **Custo:** manter o corpus de bench (G2) atualizado; rodar sandbox custa tempo (por isso G4: sob marco, não contínuo).
- **Porta aberta:** as 7 superfícies do ruflo são um cardápio do que pode ser proposto no futuro (retryPolicy, contextBuilder…) sem reabrir a decisão de governança.
- **Porta que fecha se ignorado:** sem bench, toda melhoria de harness/skill é opinião; o auto-aprimoramento não acumula evidência e o fundador decide no escuro.

## Verificação (observável)

- **G1 (score):** duas variantes de uma skill com cold-review PASS rate diferente produzem scores ordenados na direção certa; replay reconstrói.
- **G2/G3 (bench + proposta):** uma variante que vence o corpus de bench emite `BeliefProposed` com o delta de score no payload; uma que empata ou perde **não** emite proposta.
- **Gate humano (mutação — o teste-chave):** force uma variante a vencer o bench com folga → **nada muda em produção**; só após um `BeliefEstablished` (aval humano simulado) a mudança passa a valer. Varre-se o caminho: **não existe** transição medição→produção sem o evento de aval.
- **Tripwire:** uma variante que toca segredo/egress é **desqualificada** antes de pontuar (reusa a guarda existente; prova religando a guarda).
- **G4 (não-contínuo):** sem gatilho humano/marco, nenhuma avaliação dispara (varre-se: nenhum loop autônomo de evolve no log).
- **Regressão:** `cargo test` + `clippy -D warnings` limpos; suíte de segurança do router verde.

## Fonte (evidência da análise do ruflo — 2026-06-29)

- Gate humano explícito: `ruflo-metaharness/skills/harness-evolve` → *"Without `--confirm`: print plan + exit 0"*; *"Darwin Mode is human-initiated… not for autonomous self-modification"*; *"ADR-153 §5 explicitly rejects auto-evolving ruflo"*.
- Score/bench: `harness-score` (5 dimensões: harnessFit/compileConfidence/taskCoverage/toolSafety/memoryUsefulness) + `harness-bench` (corpus `{input, expectedOutput, weight}`, desacoplado dos testes naturais).
- Uma mutação por variante: `harness-evolve` → 7 superfícies de política, *"one mutation per variant… causal attribution stays clean… promote only measured wins. The model is frozen; the harness evolves."*
- Tripwire: exit-code 99 "safety-disqualified" (secrets/shell-out/network/dynamic-eval), propagado.
- Grafo: comunidades "Metaharness Evolve" (C6) e "Darwin Evolve Engine" (C5); hyperedge "Darwin evolution write-loop".
