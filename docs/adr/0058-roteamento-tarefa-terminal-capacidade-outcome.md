# ADR 0058 — Roteamento tarefa→terminal por capacidade + outcome (a porta irmã do 0045)

- **Status:** **Aceito** (ratificado em sessão direta com o fundador, 2026-06-29 — precedente ADR 0045/0038). Origem: análise do `ruvnet/ruflo` + realização explícita de uma porta deixada aberta pelo ADR 0045. **ADR-gate**: define uma decisão nova (qual terminal/papel assume uma tarefa) que toca a orquestração — projeção nova; **não** toca `Router`/`deliver_a2a` (roteamento de *mensagem*). Sequenciamento no épico **41 (Fase 4)** §III.
- **Depende de:** ADR 0057 (memória de trajetória — o sinal de outcome) + ADR 0052 (Área de Poderes — o dado de capacidade, `powers.rs`).

## Contexto

Hoje o casamento tarefa→terminal é **manual**: o Maestro lê o plano e decide quem assume. Funciona com um time pequeno; vira gargalo cognitivo quando o roster cresce e quando "quem é melhor para isto" depende de **histórico**, não só de papel.

O ADR 0045 já antecipou e deixou escrito (§Consequências): *"Porta aberta: o mesmo roteador serve para escolher **terminal/modelo/effort** por tarefa — C2 é genérico o suficiente."* Este ADR **realiza** essa porta. Dois insumos já existem no disco:

- **Capacidade** — a Área de Poderes (ADR 0052, `powers.rs`) escaneia o que cada terminal realmente pode (skills/plugins/poderes). É o "currículo" de cada colega, determinístico.
- **Outcome** — a memória de trajetória (ADR 0057) sabe "para uma tarefa como esta, o terminal/caminho X funcionou".

A análise do ruflo **confirma a forma certa** e nos poupa um caminho errado: o roteamento dele, vendido como "neural/SONA/MoE", é na verdade **heurístico** — chamadas a `hooks_route` (devolve `{recommended, confidence, reasoning}`) e a um seletor de modelo por **3 tiers de complexidade**, com os **outcomes gravados** realimentando a decisão (*"the router learns from these calls"*). Não há rede neural roteando; há **scoring heurístico + feedback de resultado**. É exatamente o que o event log do Lina faz melhor.

## Decisão

Um **roteador de tarefa** que **sugere** (autonomia ≤ assistido) ou **atribui** (autônomo, dentro do plano) o terminal/papel para uma tarefa, combinando capacidade + outcome, como projeção — nunca como rede neural, nunca pulando o gate humano.

### R1 — Score de fit = capacidade × outcome × disponibilidade
Para uma tarefa que chega ao plano:
- **Capacidade** (ADR 0052) — o terminal *pode* fazer? (tem o papel + as skills/poderes que o recall do 0057 indicou). Filtro grosso, determinístico.
- **Outcome** (ADR 0057) — para o `task_kind` desta tarefa, qual terminal/papel tem o melhor histórico de `Success`?
- **Disponibilidade** — `lina list` (livre / ocupado / travado). Não roteia para quem está afogado.

O resultado é uma **sugestão rankeada** com motivo legível (espelha o `{recommended, confidence, reasoning}` do ruflo, mas em pt-br e local): *"sugiro o @Especialista em Telas — é o papel certo e as últimas 3 telas dele passaram no cold-review de primeira"*.

### R2 — Tier de esforço/modelo por complexidade (o "3-tier" honesto)
O ruflo escala custo por complexidade: trivial → codemod determinístico ($0, sem LLM); baixa → modelo barato; alta → modelo forte. O Lina já tem o vocabulário (`LINA_EFFORT`). O roteador **sugere o tier** pela complexidade estimada da tarefa (sinais: tamanho da fronteira de arquivos, presença de ação irreversível, `task_kind`). **Sugere, não impõe** — o fundador/Maestro confirma.

### R3 — Guardrail anti-drift (o único pedaço de "swarm" que vale)
A análise mostrou que o "consenso Byzantine/Raft/Gossip" do ruflo é **rótulo de config, não implementado** — descartado. O que É real e concreto lá é o **anti-drift**: o coordenador, via *hooks* de `post-task`, pega quando um worker divergiu do objetivo. O Lina porta **só isso**: ao fechar um item, uma checagem barata "a entrega ainda casa com o objetivo do item do plano?" — divergiu → não fecha, devolve ao Maestro. Reusa o cold-review/verification que já existem; não inventa protocolo distribuído.

### R4 — Autonomia manda no verbo (o roteador respeita o guardrail existente)
- **manual** — o roteador **só informa** ("quem caberia"); nunca atribui.
- **assistido** (atual) — **propõe** a atribuição; o Maestro/fundador confirma antes do `handoff` (idêntico à regra de delegação já vigente).
- **autônomo** — atribui dentro do escopo do plano; escala ao humano fora dele.

O roteador é um **conselheiro**, não uma autoridade — não cria papel, não fura o teto de delegação, não decide ação irreversível.

## Segurança (o que este ADR NÃO muda)

- **Sugestão de roteamento é DADO, jamais autoridade.** Nenhum campo de capacidade ou score decide identidade, ordem ou autorização — a autenticação em duas camadas segue única autoridade. Um terminal **não** "se auto-elege" subindo o próprio score (capacidade vem do scan determinístico do 0052, não auto-declarável — cf. ADR 0034/`f4-0-canais-autoridade-nao-autodeclaravel`).
- **`Router`/`deliver_a2a` intocados.** Roteamento de *tarefa* (quem assume) ≠ roteamento de *mensagem* (entrega A2A). A suíte de segurança do router segue verde.
- **Gate humano de delegação preservado** — o teto de 8 delegações/turno e a profundidade 4 do `CLAUDE.md` continuam valendo; o roteador conta dentro deles.
- **Local-first** (inv #2): capacidade e outcome são locais; nada sai da máquina.

## Por quê assim (alternativas descartadas)

- **Roteador "neural" (rede treinada):** o próprio ruflo não tem isso — é branding. Caro, não-local, não-event-sourced. Rejeitado (heurística + outcome é o que funciona).
- **Consenso multi-agente (Raft/Byzantine):** não-implementado no ruflo; overkill para um time cooperativo pequeno com um Maestro. Rejeitado; ficamos com anti-drift (R3).
- **Mapa estático tarefa→papel hardcoded:** rígido, não aprende, vira dívida no 1º roster novo (mesma razão do 0045 §alternativas). Rejeitado.
- **Roteador que atribui sozinho em qualquer autonomia:** fere o guardrail de autonomia. Rejeitado — R4 subordina ao modo.

## Consequências

- **Abre:** distribuição do plano deixa de depender só do julgamento manual do Maestro; o custo (effort/modelo) passa a ser deliberado por tarefa; o time fica legível ("por que o @Fulano?").
- **Custo:** mais uma projeção no caminho de distribuição → throttle/delta (mesma lição do 0057/0045).
- **Porta aberta:** "inteligência de terminal" do Debriefing (escolher terminal/modelo/effort) fica realizada; base para o Maestro futuro automatizar a distribuição sob autonomia alta.
- **Porta que fecha se ignorado:** sem capturar qual terminal assumiu e como saiu, o histórico por-terminal não acumula e o R1 nasce cego (o `SkillOutcome` já carrega `by_role` — reusar).

## Verificação (observável)

- **R1 (fit):** dadas duas trajetórias sintéticas — @A com 3 `Success` no `task_kind` K, @B com 3 `Failure` — uma tarefa de tipo K sugere @A com motivo citando o histórico; inverter os outcomes inverte a sugestão (replay reconstrói).
- **R1 (capacidade):** um terminal sem a capacidade exigida (scan do 0052) **não** aparece na sugestão, por melhor que seja o histórico genérico.
- **R3 (anti-drift):** um item cuja entrega não casa com o objetivo declarado **não fecha** — devolve ao Maestro (prova por uma entrega propositalmente fora do objetivo).
- **R4 (autonomia):** em `manual`, o roteador **não** emite `handoff` (varre-se: nenhuma delegação parte do roteador); em `assistido`, emite proposta e espera confirmação.
- **Segurança (mutação):** um terminal forjando capacidade no payload **não** entra na sugestão (capacidade vem do scan, não do campo).
- **Regressão:** suíte do router verde; `cargo test` + `clippy -D warnings` limpos.

## Fonte (evidência da análise do ruflo — 2026-06-29)

- Roteamento heurístico: `ruflo-intelligence/skills/intelligence-route` → `hooks_route` devolve `{recommended, confidence, reasoning}`; `hooks_model-route` faz seleção Haiku/Sonnet/Opus; outcomes via `hooks_model-outcome` ("the router learns from these calls").
- 3-tier por complexidade: `ruflo-intelligence/README` → Tier 1 codemod determinístico ($0, sem LLM) / Tier 2 modelo barato (<30%) / Tier 3 reasoning complexo.
- "neural/SONA/MoE" = capacidades nomeadas mapeadas a tools, **não arquiteturas explicadas** (subagente sinalizou).
- Consenso = branding: `ruflo-swarm` lista "Byzantine/Raft/Gossip/CRDT/Quorum" como rótulos de config; **nenhuma das 12 MCP tools é de consenso**. Anti-drift (R3) é o mecanismo real: coordenador + `post-task` hooks + cap de agentes.
- Grafo: comunidade "Swarm & Anti-Drift" (C13) e hyperedge "anti-drift coordination stack".
