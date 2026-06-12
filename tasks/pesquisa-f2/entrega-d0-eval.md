# Entrega D0 — Régua/Eval de UX da Fase 2

> **Dono:** Terminal D (QA) · **Data:** 2026-06-12 · **Despacho:** `tasks/pesquisa-f2/despacho-d0-eval.md` · **Plano:** `_plano-pesquisa-f2.md`
> **Método:** interno-primeiro (ADR 0019 · rubrica anti-slop v1 · `prof-baseline.md` · `ondas-5-6.md` F1-5 · ADR 0028) → delta externo: **14 buscas** em 5 frentes (2-3 por frente, teto 15/agente respeitado) + **verificação adversarial independente de 10 claims load-bearing** (1 busca cada, fonte sempre distinta da original): 6 confirmados, 4 parciais, 0 refutados — correções dos "parciais" já incorporadas abaixo. Fetch real em toda URL citada; fetch falho (OUP/ACM/INFORMS 403) ⇒ claim marcado SUSPECT ou rebaixado. Delta confirmado: o vault não tem pesquisa interna de metodologia de eval de UX (varredura do PageIndex; as 13.x de R11 cobrem canvas/render/permissão/benchmark, não eval).

---

## I. Achados (formato da skill)

### A1 — Reaction cards transformam "visual bonito" em palavras-alvo contáveis, e funcionam com amostra pequena — mas o score agregado não
- **CLAIM:** O Microsoft Desirability Toolkit (118 palavras, 60% positivas/40% negativas; participante escolhe top-5; análise = % de participantes por palavra) é operacional com 4-23 participantes, e a adaptação NN/g para apelo visual (screenshot + ~25 palavras curadas com ≥40% negativas, randomizadas, mapeadas contra os atributos de marca pretendidos) é o mecanismo que torna "visual único" verificável. O que NÃO sobrevive: usar positivity-ratio como score quantitativo com n<14 (instável; perde para o SUS).
- **FONTE+URL:** Benedek & Miner 2002 via NN/g (https://www.nngroup.com/articles/microsoft-desirability-toolkit/) + MeasuringU (https://measuringu.com/microsoft-desirability/)
- **DATA:** origem 2002 / NN/g 2016-02 / análise MeasuringU 2020-02 / uso corrente 2025 (praticantes)
- **CONFIANÇA:** alta
- **REFUTAÇÃO TENTADA:** a própria MeasuringU ataca o método ("não há evidência de que mede desirability"; sem benchmark publicado; ratio instável) — derruba o uso como score, preserva o uso como contagem de palavras-alvo. NN/g admite ambiguidade interpretativa e "cliques preguiçosos" em survey — mitigado rodando moderado/síncrono. Verificação adversarial independente: protocolo/deck/análise **confirmados** por 3 fontes; "~5 min de administração" e o censo exato de amostras pequenas só existem na fonte original (parcial).
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada c)

### A2 — "5 usuários" vale só na forma estreita: rodadas iteradas de teste ESTRUTURADO, nunca rodada única
- **CLAIM:** Teste direto de Faulkner (60 usuários, 100 amostragens): conjuntos de 5 acham em média 85,55% dos problemas, mas o **mínimo é 55%**; conjuntos de 10 → mínimo 82%. A regra de Nielsen (2000) sobrevive apenas como "rodadas de 5, com tarefas definidas e correção entre rodadas"; problema que afeta 10% dos usuários exige ~18 testers para aparecer com confiança.
- **FONTE+URL:** Faulkner 2003, Behavior Research Methods (PDF íntegro: https://link.springer.com/content/pdf/10.3758/BF03195514.pdf) + Nielsen/NN-g 2000 + Sauro/MeasuringU 2010
- **DATA:** 2000/2003 · uso corrente 2018+ (NN/g mantém 5 p/ qualitativo, 20+ p/ quantitativo)
- **CONFIANÇA:** alta
- **REFUTAÇÃO TENTADA:** críticas clássicas verificadas no próprio PDF — Spool & Schroeder 2001 (5 acham só 35% em teste NÃO-estruturado), Woolrych & Cockton 2001 (fórmula infla p; IC real ±32%). Resultado: a régua exige teste estruturado e ≥2 rodadas nos fluxos centrais — o formato que resiste às críticas E cabe no orçamento do fundador.
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada b)

### A3 — SUS e SEQ têm benchmark numérico estável; com n=5 são termômetro direcional, nunca estatística
- **CLAIM:** SUS: média da indústria 68; ≥80,3 = nota A/top 10% ("excelente"); <51 = F (base ~500 estudos/5.000 respostas, Sauro & Lewis). SEQ (1-7, após cada tarefa): média de referência 5,3-5,6 (400+ tarefas/10k usuários); correlação com comportamento observado r≈0,5 → **nunca substitui sucesso observado**. Par recomendado: SEQ por tarefa + SUS por sessão.
- **FONTE+URL:** MeasuringU (https://measuringu.com/sus/ · https://measuringu.com/seq10/) corroborado por NN/g 2018 e UXtweak 2025
- **DATA:** origem 1986 (SUS)/2012 (SEQ) · benchmarks 2011/2019 · uso corrente 2023-2025
- **CONFIANÇA:** alta
- **REFUTAÇÃO TENTADA:** vendor-bias (banco proprietário MeasuringU) — mesmos números repetidos por NN/g e Lyssna (independentes; verificação adversarial: **confirmado** 2×). "Confiável com n=2" (Sauro) contestado por UXtweak/NN-g (20-30 p/ significância; "dados numéricos de 5 usuários não decidem design") → na régua, SUS/SEQ entram como gate qualitativo/tendência, não como número absoluto.
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada b)

### A4 — Distintividade se mede com estímulo SEM marca → atribuição; elemento isolado quase nunca distingue
- **CLAIM:** Distinctive Asset Grid (Romaniuk/Ehrenberg-Bass): Fame = % que liga o estímulo de-brandeado à marca; Uniqueness = % que nomeia SÓ a sua. Dado duro: apenas 4% das cores e 6% das taglines testadas foram unicamente atribuídas à marca certa — **distintividade vem da COMBINAÇÃO de elementos** (layout+paleta+tipografia+motion), não de uma cor/fonte "assinatura".
- **FONTE+URL:** Romaniuk 2018 (livro, OUP) via tianpan.co 2025-09 (https://tianpan.co/blog/2025-09-01-building-distinctive-brand-assets-by-jenni-romaniuk) + praticantes 2025-2026
- **DATA:** origem 2018 / uso corrente 2025-09 e 2026-05
- **CONFIANÇA:** média
- **REFUTAÇÃO TENTADA:** (a) método desenhado p/ brand tracking com amostra grande — com 5-10 testers o limiar publicado 50/50 ("Hero") é ruidoso demais p/ valer literalmente; (b) fonte primária OUP retornou 403; (c) página com o limiar é UNDATED. Sobrevive o MECANISMO (line-up com chance conhecida), caem os NÚMEROS como benchmark — a régua declara os alvos como convenção.
- **SUSPECT:** sim (números 50/50; ano do livro por citação secundária) · **LOAD-BEARING:** sim (camada c)

### A5 — "Viciante" ético é mensurável: engajamento bruto pode SUBIR com utilidade CAINDO — o par honesto é retorno voluntário + arrependimento como guarda, com auditoria objetiva de manipulação como pré-condição
- **CLAIM:** Otimizar tempo-no-app/sessões explora inconsistência entre escolha do momento e preferência real (Kleinberg et al., Management Science 70(9)); "regretful use" detecta o conteúdo que prende sem valer — mas regret não pode ser objetivo isolado (trivialmente zero sem uso). O HEART (Google) dá a FORMA das métricas: Engagement = uso por vontade própria por usuário; Retention = % que retorna no dia N; caveat dos próprios autores: para ferramenta de produtividade, medir SEMANAL, não diário. E a pré-condição tem instrumento objetivo: a ontologia peer-reviewed de **65 deceptive patterns** (Gray, Santos, Bielova & Mildner, CHI 2024, sobre a base de Brignull 2010) permite auditoria barata — checar a UI e exigir ZERO ocorrências (obstrução de saída, sneaking, pressão falsa, mecânicas de re-engajamento) ANTES de qualquer métrica de engajamento contar. Implicação: **tempo-no-app nunca é métrica de sucesso da F2**.
- **FONTE+URL:** Kleinberg, Mullainathan & Raghavan (https://arxiv.org/abs/2202.11776) + Rodden et al., CHI 2010 (https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/36299.pdf) + Gray et al., CHI '24 (https://arxiv.org/abs/2309.09640)
- **DATA:** 2022-02/2024 (Mgmt Science) · HEART 2010, uso corrente 2026 (IxDF) · ontologia 2024-05
- **CONFIANÇA:** alta
- **REFUTAÇÃO TENTADA:** Kleinberg é modelo teórico (a operacionalização como pergunta pós-sessão é inferência declarada); fetch do journal 403 (data via 2 fontes independentes). deceptive.design sozinho não distingue persuasão legítima de deception (confirmado no fetch) → ancorado na ontologia peer-reviewed; a auditoria é guarda NEGATIVA — ausência de manipulação não prova delight (na régua é pré-condição, nunca métrica de sucesso). Verificação adversarial: **confirmado** — paper independente (NeurIPS 2025, arXiv:2510.16368, autor distinto) replica a tese de engajamento-vs-utilidade; HEART confirmado por 4 fontes.
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada f)

### A6 — Latência: o limiar perceptível para input indireto é ~55ms (drag) / ~96ms (tap); a CAUDA é o que o usuário sente; o baseline atual do Lina está no nível dos piores terminais medidos
- **CLAIM:** JND de latência depende de tarefa+form-factor: input indireto (teclado/mouse→tela) = 55ms dragging / 96ms tapping (Deber et al., CHI 2015) — dragging é a interação mais sensível, exatamente o caso de um canvas com pan/drag. Medições reais de terminais (Dan Luu, 10k keypresses): medianas de ~5ms a ~45ms, cauda até 111ms sob carga; percentis (p90/p99.9) importam mais que mediana porque digitar amostra a distribuição milhares de vezes/dia; heurístico de Carmack: 20ms ok / 50ms "laggy" / 150ms insuportável. O frame p50 ~40ms do Lina (prof-baseline) está na ordem dos PIORES terminais de 2017 — antes mesmo de olhar a cauda.
- **FONTE+URL:** Deber et al. CHI 2015 (PDF íntegro: https://www.tactuallabs.com/papers/howMuchFasterIsFastEnoughCHI15.pdf) + Dan Luu (https://danluu.com/term-latency/)
- **DATA:** 2015 · 2017 (datado via API HN; página sem data) · JND citado em paper independente 2024-08
- **CONFIANÇA:** alta
- **REFUTAÇÃO TENTADA:** JND é percepção treinada em lab (limiar de incômodo real é MAIOR → usar 55ms como teto é conservador a favor); Pavel Fatin 2015 argumenta efeito motor abaixo de 20ms mesmo sem percepção consciente (registrado: o gate p50 fica em 25ms por isso). Números absolutos de 2017 envelheceram (hardware/compositores) — a metodologia (percentis, cauda sob carga) não. Verificação adversarial: **confirmados** ambos (Yamanaka 2024 cita os 4 valores de JND; LWN 2018 corrobora Dan Luu com medição própria).
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada d)

### A7 — O budget de frame da nossa própria stack é conhecido (Zed/gpui: 8,33ms p/ 120fps) e o gate honesto é estabilidade por percentil + % do tempo acima do orçamento, não média de FPS
- **CLAIM:** O Zed (mesma stack gpui) define ~8,33ms/frame para 120fps e tratou oscilação 8-16ms como BUG perceptível (usuário via stutter mesmo com render <4ms); critério de pronto = "120fps ESTÁVEL", medido com Metal HUD/Instruments. Complemento metodológico: média de FPS e até "1% low" mascaram stutter — a métrica fiel é **% do TEMPO em frames acima do orçamento** + percentil de frametime (SuperTuxKart 2024 define "Steady" como <1% do tempo em frames lentos; definições CapFrameX 2020).
- **FONTE+URL:** Zed (https://zed.dev/blog/videogame 2023-03 + post Metal 2024-02) + SuperTuxKart (https://blog.supertuxkart.net/2024/07/why-average-fps-and-1-low-fps-are.html)
- **DATA:** 2023-03 / 2024-02 / 2024-07
- **CONFIANÇA:** média (Zed) / alta (metodologia de percentil)
- **REFUTAÇÃO TENTADA:** vendor-bias do Zed testado — 8,33ms é aritmética de refresh, não marketing, e o post de 2024 documenta falha própria (cherry-picking improvável); limite real: pipeline Metal/macOS não transfere 1:1 a Windows/Linux, e o Zed NÃO publica SLO formal de p95 — meu gate de p95 é composição declarada (orçamento Zed + metodologia percentil), não padrão copiado. Cortes exatos do SuperTuxKart (1%/12%/50%) são escolhas do autor, não psicofísica — uso a estrutura, não os cortes.
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada d)

### A8 — WCAG 2.2 AA é aplicável a desktop nativo via WCAG2ICT, com números cobráveis — mas reduce-motion NÃO é AA, e WCAG sozinho não fecha a régua legal da UE
- **CLAIM:** O WCAG2ICT (W3C Group Note, 2024-10-08) transpõe WCAG 2.2 A/AA a software não-web. Números-chave por story: contraste ≥4,5:1 texto (≥3:1 texto grande e componentes de UI — 1.4.3/1.4.11); alvo de clique ≥24×24 px lógicos (2.5.8 AA, novo no 2.2; o Understanding atualizado 2026-05 recomenda mirar 44×44 AAA em controles importantes); foco visível (2.4.7) + foco não obscurecido (2.4.11); Name/Role/Value na árvore de acessibilidade (4.1.2). **Correções da verificação adversarial:** (i) respeitar reduce-motion do SO é 2.3.3 = **AAA**, não AA (no AA só 2.2.2: movimento >5s precisa de pausa); (ii) a lista de critérios excluídos varia por jurisdição — Section 508 isenta 2.4.1/2.4.5/3.2.3/3.2.4 (e segue em WCAG 2.0); a EN 301 549 ch.11 isenta também 2.4.2/3.1.2; (iii) "cumprir WCAG 2.2 AA já cobre a lei da UE" é EXAGERO — o EAA (vigente 28/06/2025, standard EN 301 549 v3.2.1) exige além do WCAG: declarações de acessibilidade, documentação e suporte acessíveis.
- **FONTE+URL:** W3C WCAG2ICT (https://www.w3.org/TR/2024/NOTE-wcag2ict-22-20241008/) + Understanding 2.5.8 (https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html) + EC digital-strategy (2025-06-30); correções: U.S. Access Board (https://www.access-board.gov/ict/) e Level Access
- **DATA:** 2024-10 · 2026-05 (Understanding) · 2025-06 (EAA/EC)
- **CONFIANÇA:** alta (números técnicos) / média (camada legal)
- **REFUTAÇÃO TENTADA:** WCAG2ICT é informativo, não normativo (quem normatiza é EN 301 549/508 — que o usam como base); a exceção de espaçamento do 2.5.8 permitiria alvo minúsculo passar (a régua cobra o tamanho real, não a exceção); verificação adversarial derrubou 2 partes do claim original (jurisdição das exclusões; suficiência legal) — **incorporadas acima**.
- **SUSPECT:** não · **LOAD-BEARING:** sim (camada e)

---

## II. Proposta: a régua em camadas da F2

> **Princípio herdado do épico F1:** todo PASS é *observável* — "algo que roda e se mede", **medido, não prometido** (gramática das peças F1, ex.: `ondas-5-6.md`). A régua COMPÕE com o que já existe: a camada (a) é a rubrica anti-slop v1 vigente; a (d) referencia a sonda `[PROF]` e o baseline medido; a (e) referencia ADR 0028/F1-2-6. Nada é duplicado.
>
> **Cadência (o que roda quando — é isso que mantém o custo real):**
> - **Por story de UI:** camadas (a) + (e) + (d-regressão). Custo ~1h/story, 1 pessoa, ferramentas grátis.
> - **Por rodada/marco (3-5 stories agrupadas):** camadas (b) + (c). Custo ~3h + 5 testers informais.
> - **Por release/fase:** camada (f) + smoke de screen reader + auditoria anti-manipulação. Custo ~3h.

### Camada (a) — Anti-slop PASS *(já existe; reuso integral)*

| Métrica | Como medir | Custo | PASS |
|---|---|---|---|
| Cold-review da story contra a rubrica v1 (dimensões DESIGN DES-1..5 + COPY — strings via fonte única R9) | Revisor cego (skill `lina-cold-review`): artefato + critérios + rubrica, sem contexto do autor | ~30 min/story; já operacional | **Score ≥80 E zero violação ALTA** (rubrica §1). DES-4 exige a direção estética DECLARADA — na F2, o statement vem da entrega D1 e cada story o cita (1 direção por projeto, conforme ADR 0019 §7) |

### Camada (b) — Tarefa do leigo *(por rodada; protocolo Faulkner-compatível)*

Perfil do tester: empreendedor não-técnico que **nunca viu o app**; roteiro estruturado de 1-3 tarefas críticas; sem ajuda do moderador; tela gravada.

| Métrica | Como medir | Custo | PASS |
|---|---|---|---|
| **Sucesso de tarefa (gate duro, binário)** | 5 testers executam o caminho crítico das stories da rodada | ~3h/rodada (fundador modera) | **5/5 completam sem ajuda.** Com n=5, sucesso é GATE, não taxa (5/5 → piso do IC95% ≈48%; Sauro 2011 — âncora de sanidade: média da indústria 78%, tarefa crítica mira 100%). Qualquer falha = problema observado em ≥1/5 → conserta e re-roda |
| **Fluxos centrais** (criar 1º agente; navegar/organizar o canvas) | Idem, em **≥2 rodadas de 5 com correção entre elas** | 2× o acima | 2ª rodada com 5/5 + zero problema crítico novo (1 rodada única pega no pior caso só 55% dos problemas — Faulkner 2003, A2) |
| **SEQ verbal (1-7) após cada tarefa** | Pergunta única falada; cada nota ≤3 dispara "por quê?" | embutido | **Mediana ≥5,5** (referência 5,3-5,6 — A3) **E nenhuma nota ≤3 sem causa identificada**. Mediana, não média — n=5 não aguenta média |
| **SUS pt-BR ao fim da sessão** | Formulário de 10 itens (escolher tradução pt-BR validada ANTES da 1ª rodada — lacuna L3) | 5 min/tester | Termômetro de FASE, nunca gate de story: alvo ≥68 (média), aspiração ≥80,3 (top 10% — coerente com "único e viciante"); por rodada exige-se só tendência não-decrescente |
| **Tempo-de-tarefa** | Da gravação | zero extra | Só comparação relativa entre rodadas (n=5 não dá tempo absoluto — Sauro 2010). O invariante "1º agente <2h" (inv#6) vira a tarefa golden-path cronometrada |

### Camada (c) — Percepção e distintividade *(por rodada; ~20 min/tester, ferramentas grátis)*

| Métrica | Como medir | Custo | PASS |
|---|---|---|---|
| **Palavras-alvo (reaction cards, protocolo NN/g — A1)** | Fundador declara as 5 palavras-atributo da direção D1 + monta lista de ~25 (≥40% negativas, randomizada, com kill-words: "genérico", "confuso", "complicado", "sem graça"); tester vê o app rodando e escolhe top-5 | Zoom/Lyssna free tier; 5 min/tester | **≥60% dos testers acertam ≥2 palavras-alvo no top-5 E nenhuma kill-word aparece em >1 tester.** NUNCA usar positivity-ratio como score (instável n<14 — A1). Alvo 60% = convenção declarada (não há benchmark publicado — L2); calibrar nas 2 primeiras rodadas |
| **Clareza em 5 segundos (gate do "zero jargão", inv#6)** | Screenshot do canvas por 5s exatos → "o que esse app faz?" | fivesecondtest.com/Lyssna grátis; 2 min/tester | **≥8/10 descrevem o propósito corretamente em linguagem leiga.** Escopo estrito: primeira impressão/clareza — nunca julgar comportamento por este teste |
| **Line-up de distintividade (mecanismo Romaniuk — A4)** | Screenshot de-brandeado do Lina entre 4 lookalikes da categoria (Warp, terminal genérico, chat-IA genérico, VS Code com splits); testers que usaram o Lina 1× identificam no dia seguinte | 10 min/tester | **≥8/10 corretos** (chance = 20%). Alvo = convenção declarada inspirada no limiar "Hero" 50/50 (suspect — A4). Princípio de design decorrente: distintividade por COMBINAÇÃO de elementos, nunca por 1 cor/fonte isolada (4%/6% — A4) |

### Camada (d) — Budget de perf *(por story: regressão; por fase: gate — baseline interno medido)*

Baseline (prof-baseline.md, 2026-06-10): frame p50 ~40ms/26fps constante N=4→28 (drawn 2-4), trabalho ativo ~16ms, `present_vsync` dominado por pacing (F1-5-1b ativada); sonda `[PROF]` válida (overhead ~2%). Alvo do produto: **8-12 terminais ativos fluidos** (decisão do fundador 2026-06-06 — "28 a 50fps" segue refutado). Célula drawn≥12 pendente da sessão de tela.

| Métrica | Como medir | Custo | PASS |
|---|---|---|---|
| **Regressão por story (R7 — já vigente)** | `[PROF]` antes/depois na matriz existente (`LINA_PROF=1 LINA_LOAD=…`) | sonda já existe; 1 run | Nenhuma story de UI regride frame p50/p95 vs baseline da rodada |
| **Frametime (gate da F2)** | Cena de estresse padronizada: 8-12 ativos (LINA_LOAD_ACTIVE=10) + pan/zoom contínuo 60s; estender `[PROF]` com p99 e acumulador "% do tempo acima do orçamento" (persistido como evento — inv#4, régua re-rodável). Validação externa no macOS: `MTL_HUD_ENABLED=1` (método Zed) | extensão S da sonda; HUD grátis | **p95 ≤16,6ms E <1% do TEMPO acima de 16,6ms** na cena de estresse (estrutura SuperTuxKart/CapFrameX — A7). Marco intermediário: p50 ≤16,6ms (hoje 40ms). **120Hz (p95 ≤8,33ms) é meta de PLATAFORMA, não gate da F2** — o Zed levou um ciclo inteiro de engenharia Metal para estabilizá-lo (A7); promover a gate quando 120Hz virar critério de release |
| **Input latency (keypress-to-photon no terminal focado)** — hoje NÃO medida (a sonda mede frametime, não input→photon — L7) | Typometer (grátis) apontado para a janela + validação física: câmera 240fps do celular (keydown→glifo, ±1 frame ≈4ms); medir idle E sob carga | grátis; ~30 min/medição | **p50 ≤25ms E p99 ≤50ms sob carga.** Fundamento: 50ms < JND de 55ms do drag indireto (interação mais sensível do canvas — A6) e na fronteira "laggy" de Carmack; p50 25ms cobre o argumento motor-control de Fatin (A6). A cauda é o gate porque é o que o usuário sente (Dan Luu — A6) |

### Camada (e) — A11y por story *(checklist mensurável; ~20-30 min/story, 1 pessoa, ferramentas grátis)*

| Métrica | Como medir | Custo | PASS |
|---|---|---|---|
| **Contraste (1.4.3/1.4.11)** | Pares de cor validados 1× na criação dos tokens do DS (D2); por story, só pares novos — Colour Contrast Analyser da TPGi (grátis, eyedropper sobre app nativo) | ~5 min/story | Zero par <4,5:1 (texto) / <3:1 (texto grande + componentes de UI) |
| **Alvo de clique (2.5.8)** | Assert no bounding box do nó na árvore AccessKit (o app JÁ expõe a árvore — `a11y.rs`/ADR 0028; gpui tem AccessKit 1ª classe, doc 33) ou screenshot ÷ scale factor | teste headless ou 5 min manual | Todo controle interativo **≥24×24 px lógicos** (FAIL-line), target de design ~32-44 (Understanding 2026-05 — A8); sem usar a exceção de espaçamento |
| **Name/Role/Value (4.1.2)** | Query por label/role na árvore AccessKit (padrão kittest; fallback: Accessibility Inspector no macOS / Accessibility Insights no Windows, grátis) | teste de CI quando o harness existir (L6); senão 10 min/story | 100% dos controles da story encontráveis por nome+papel; 0 erros no scanner |
| **Teclado + foco (2.1.1/2.4.7/2.4.11)** | Completar o fluxo da story SÓ com teclado, gravando — o canvas por teclado da F1-2-6 **não regride** (R6) | ~10 min/story | Fluxo completável; nenhum tab-stop sem indicador visível; nenhum foco escondido atrás de overlay |
| **Motion** | Toggle reduce-motion do SO → conferir animações | 5 min/story | Animação não-essencial desliga com reduce-motion **(exigência por DOUTRINA — ADR 0019 §7 —, não por compliance: 2.3.3 é AAA, correção A8)**; movimento contínuo >5s tem pausa (2.2.2, este sim AA) |
| **Por release:** smoke de screen reader | 30 min VoiceOver (macOS) / NVDA (Windows) no fluxo principal | 30 min/release | Fluxo narrável sem beco; honestidade do ADR 0028 mantida: nenhuma copy afirma "conforme ARIA live-region" até o spike passar NA TELA |

### Camada (f) — "Volta e flui" — o *viciante* operacionalizado, sem dark pattern *(por release)*

| Métrica | Como medir | Custo | PASS |
|---|---|---|---|
| **Pré-condição: auditoria anti-manipulação** | UI checada contra a ontologia de 65 deceptive patterns (CHI 2024 — A5) | 1-2h/release | **Zero ocorrências** (zero streak/badge, zero notificação de re-engajamento, zero obstrução de fechar/sair). Torna todo retorno medido abaixo VOLUNTÁRIO por construção |
| **Retorno voluntário W1** | Projeção do event log local (o app já registra abertura de sessão — zero instrumentação nova; inv#4): "reabriu por conta própria na semana seguinte ao 1º uso, sem ping" | grátis (projeção) | ≥60% dos testers retornam em W1, medido SEMANALMENTE (caveat HEART p/ produtividade — A5). Alvo 60% = convenção declarada (L2); a forma da métrica tem fonte, o número não |
| **Pergunta de decepção (Sean Ellis)** | "Como você se sentiria se não pudesse mais usar o Lina?" ao fim de cada rodada | 1 pergunta | ≥40% "muito decepcionado" como **tendência entre rodadas** (com n<40 é direcional, nunca veredito — fonte UNDATED, suspect). Robusta a manipulação por construção: dark pattern infla uso, não infla "sentiria falta" |
| **Fim lembrado + arrependimento** | Micro-survey de 2 perguntas pós-sessão (testers + fundador semanal): "o tempo valeu?" [valeu/mais ou menos/me arrependi] e "qual foi o melhor momento?" | 2 min/sessão | **Zero "me arrependi" em ≥10 sessões acumuladas** E o "melhor momento" citado é *resultado avançando* (agente entregou algo), não estímulo de interface (peak-end como heurística; regret como GUARDA, nunca objetivo — A5) |
| **Proibição métrica** | — | — | **Tempo-no-app e contagem de sessões NUNCA entram como métrica de sucesso da F2** (engajamento bruto pode subir com utilidade caindo — A5) |

---

## III. CONFLITOS

1. **Premissa do despacho × WCAG:** o despacho assumia reduce-motion dentro da "conformidade WCAG 2.2 AA" — falso: 2.3.3 é AAA. **Resolução:** a exigência permanece na régua **por doutrina interna** (ADR 0019 §7 já manda motion subordinado a reduce-motion) — a doutrina é mais forte que o AA aqui; só muda a justificativa.
2. **Suficiência legal:** "WCAG 2.2 AA via WCAG2ICT cobre e excede a lei da UE" foi derrubado parcialmente pela verificação (EAA exige declarações/documentação/suporte além do WCAG; harmonização no OJ é da WAD, não formalmente da EAA). **Resolução:** a régua TÉCNICA da F2 segue WCAG 2.2 AA via WCAG2ICT (mais exigente que o ch.11 vigente); compliance legal completa é tema de fase comercial, fora do escopo F2 (local-first, sem venda UE hoje) — registrado para não virar promessa.
3. **Exclusões por jurisdição:** Section 508 (ainda WCAG 2.0) isenta 2.4.1/2.4.5/3.2.3/3.2.4; EN 301 549 isenta também 2.4.2/3.1.2. A régua usa a INTERSEÇÃO cobrável (nenhuma das 6 entra no checklist por story).
4. **n pequeno × estatística:** Sauro ("SUS confiável com n=2") × NN-g/UXtweak ("20-30 p/ significância"; "dados de 5 usuários não decidem design"). **Resolução na régua:** com 5 testers, sucesso é gate binário, SEQ usa mediana+investigação, SUS é tendência de fase — nada vira estatística.
5. **Engajamento × utilidade:** HEART manda medir engagement; Kleinberg prova que otimizá-lo pode reduzir utilidade. **Resolução:** engagement só conta com a pré-condição anti-manipulação + guarda de arrependimento (ambos A5) + proibição de tempo-no-app.
6. **Meta 120Hz (R7/stack) × baseline 26fps:** o gate de F2 em 120Hz seria promessa sem evidência (o Zed gastou um ciclo de engenharia para estabilizá-lo; nosso pacing nem foi diagnosticado — F1-5-1b aberta). **Resolução:** gate da F2 = 60Hz por p95 + % do tempo; 120Hz = meta de plataforma, promovida a gate por decisão explícita futura. Coerente com "medir primeiro" do fundador.

## IV. LACUNAS

- **L1.** Benchmarks numéricos (SUS 68, SEQ 5,3-5,6, conclusão 78%) vêm do banco MeasuringU 2011-2019, web-app e anglófono; transferência p/ desktop+leigo brasileiro é assumida, não provada — por isso entram como âncoras/termômetros, nunca gates duros isolados.
- **L2.** Alvos 60% (palavras-alvo; retorno W1) e 8/10 (clareza; line-up) são **convenções declaradas** — não há benchmark publicado nesses formatos. Calibrar nas 2 primeiras rodadas e registrar a calibração como Decisão.
- **L3.** Tradução pt-BR validada do SUS: existe na literatura, não foi verificada nesta pesquisa — escolher antes da 1ª rodada (item de preparação do épico F2).
- **L4.** Kano model (priorizar features de encantamento) ficou FORA: sem fetch de fonte primária datada. Se o épico F2 quiser priorização de delight por feature, é 1 fetch adicional.
- **L5.** Célula drawn≥12 do baseline `[PROF]` pendente (sessão de tela do fundador) — pode mexer no marco intermediário da camada (d), não no gate.
- **L6.** Harness de teste automatizado da árvore AccessKit sobre o shell gpui é trabalho a fazer (kittest não tem integração gpui; a árvore existe no app — ADR 0028). Até lá, o fallback por story é o scanner de plataforma (gratuito).
- **L7.** Input latency keypress-to-photon nunca foi medida no Lina (a sonda mede frametime). A 1ª medição com Typometer/câmera deve entrar como story de abertura da onda de perf da F2 (análoga à F1-5-1: medir ANTES de otimizar).
- **L8.** EN 301 549 v4.1.1 (incorporando WCAG 2.2, "esperada 2026"): não confirmada em fonte fetchada — monitorar, sem decidir nada por ela.

## V. NOTA DE RECÊNCIA

Domínio **lento** (métodos clássicos de HCI): SUS (1986), reaction cards (2002), regra dos 5 (2000/2003), JND de latência (2015) — estáveis; uso corrente confirmado por fontes 2023-2026 onde possível (UXtweak 2025-09, praticantes 2025-2026, citação de JND em paper 2024-08). Domínio **rápido** (tudo 2024+): WCAG2ICT 2024-10, Understanding 2.5.8 atualizado 2026-05, EAA vigente 2025-06, kittest 2026-03 / accesskit_winit 2026-05, ontologia de deceptive patterns CHI 2024-05, corroboração de engajamento-vs-utilidade NeurIPS 2025-10, metodologia de frametime 2024-07. Páginas UNDATED (Lyssna n de 5s; First Round; limiar 50/50) estão marcadas SUSPECT nos achados que as usam. Baseline interno de perf: medido 2026-06-10.

---

PRONTO: régua em 6 camadas com PASS observável por métrica — (a) rubrica anti-slop reusada (score ≥80, zero ALTA), (b) tarefa do leigo (5/5 gate + SEQ mediana ≥5,5 + SUS tendência), (c) percepção/distintividade (palavras-alvo + 5s + line-up), (d) perf com baseline medido (p95 ≤16,6ms + <1% do tempo acima; input p99 ≤50ms; 120Hz = meta, não gate), (e) a11y WCAG 2.2-AA-via-WCAG2ICT por story (contraste/24px/árvore/teclado), (f) "viciante" ético (retorno voluntário + decepção + zero arrependimento, com auditoria anti-manipulação como pré-condição e tempo-no-app proibido como métrica) — 8 achados datados com refutação tentada, 10 verificações adversariais independentes (2 correções incorporadas), 6 conflitos resolvidos, 8 lacunas declaradas.
