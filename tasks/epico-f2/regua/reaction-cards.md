# Deck de reaction cards — F2 (kit da régua · camada c)

> **Dono:** Terminal D (QA) · **Data:** 2026-06-12 · **Fontes (fetchadas em 2026-06-12):** lista original das 118 palavras — NN/g (https://www.nngroup.com/articles/desirability-reaction-words/); protocolo de subconjunto ~25/apelo visual — Kate Moran, NN/g (https://www.nngroup.com/articles/microsoft-desirability-toolkit/); protocolo original e críticas — Lewis & Sauro, MeasuringU 2020-02-19 (https://measuringu.com/microsoft-desirability/); cross-check da lista — UX Firm + en-academic. Régua D0: achado A1, camada (c).
> **Atribuição obrigatória da licença (sempre que o deck for usado):** *"Developed by and © 2002 Microsoft Corporation. All rights reserved."* (Benedek & Miner, UPA 2002).
> **Palavras-alvo:** as 5 da decisão F2-0-D (território "Instrumento de Estúdio com a temperatura do Ateliê").

---

## 1. O deck — 25 palavras (14 positivas · 11 negativas = 44%, acima do piso de ≥40%)

| Grupo | Palavras | Origem |
|---|---|---|
| **Alvo (5)** — positivas | **vivo · honesto · acolhedor · preciso · artesanal** | Atributos da marca selados na F2-0-D. A curadoria NN/g manda incluir "palavras relevantes aos objetivos do estudo" — alvo não precisa vir das 118 (mapeamento aproximado ao original, p/ rastreio: vivo≈Energetic/Engaging · acolhedor≈Inviting/Approachable · preciso≈Consistent/Reliable · honesto≈Trustworthy · artesanal≈High quality/Creative) |
| **Distratoras (9)** — positivas | bonito (Attractive) · moderno (Cutting edge) · profissional (Professional) · rápido (Fast) · confiável (Reliable) · simples (Straight Forward) · divertido (Fun) · poderoso (Powerful) · organizado (Organized) | Traduzidas das 118. Escolhidas ADJACENTES aos alvos de propósito (moderno×artesanal, rápido×preciso, divertido×vivo, bonito×tudo): se o tester escolhe a vizinha genérica em vez do alvo, o território **não** comunicou o atributo — é exatamente o que queremos detectar |
| **Kill-words (5)** — negativas | **genérico (Ordinary) · confuso (Confusing) · complicado (Complex) · sem graça (Dull/Boring) · frio (Sterile)** | As 4 da régua D0 + "frio" (despacho r1) — "frio" é o anti-atributo direto da temperatura T3 (superfícies quentes) |
| **Negativas extras (6)** | intimidador (Intimidating) · estressante (Stressful) · lento (Slow) · imprevisível (Unpredictable) · ultrapassado (Dated) · frágil (Fragile) | "intimidador" instrumenta a hipótese do gate F2-2 ("cara de terminal encanta vs assusta"); "lento" cruza com a camada (d) — percepção de latência dita por leigo |

> **Tradução é nossa, e declarada:** não existe adaptação pt-BR publicada do deck (3 buscas dirigidas em literatura SBC/IHC e praticantes, 2026-06-12 — ausência de evidência, não prova). Cada palavra carrega o original em inglês entre parênteses para rastreabilidade; o balanço 60/40 do desenho original é preservado (44% negativas).
> "lento" contraria a dica NN/g de remover palavras de performance — de propósito: o nosso teste mostra o app **rodando** (D0 camada c), não um screenshot estático, então performance percebida é sinal legítimo.

## 2. Aplicação (no bloco 5 da sessão — ver `regua/roteiro-leigo.md` §3)

1. Com o app ainda visível (rodando — nunca screenshot), entregar a folha com as 25 palavras **na ordem do tester** (§3).
2. **Ler:** "Marque as **5 palavras** que melhor descrevem o que você acabou de usar. Não existe resposta certa."
3. Para CADA palavra marcada: **"por que essa?"** — registrar a resposta literal (é a entrevista pós-teste do protocolo original; o insight mora no porquê, não na contagem).
4. Sem limite de tempo; sem ajuda; não definir nenhuma palavra ("o que ela significa pra você?" se perguntarem — a dúvida é dado).

## 3. Randomização (documentada — regra determinística e auditável)

A ordem de apresentação muda por tester para diluir viés de posição (recomendação NN/g). Método: sobre a lista-base em ordem alfabética, a ordem do tester *k* é a permutação de ciclo completo `palavra[(k + i·salto) mod 25]`, com saltos coprimos de 25 — `{7, 11, 13, 17, 19}` para os testers 1-5. Sem sorteio em tempo de sessão: as 5 folhas abaixo já saem prontas (rodada de 5; na 2ª rodada, reusar as mesmas 5 ordens — testers novos).

- **Ordem 1 (salto 7):** artesanal · frio · moderno · simples · confiável · honesto · preciso · acolhedor · estressante · lento · sem graça · complicado · genérico · poderoso · vivo · divertido · intimidador · rápido · bonito · frágil · organizado · ultrapassado · confuso · imprevisível · profissional
- **Ordem 2 (salto 11):** bonito · intimidador · vivo · genérico · sem graça · estressante · preciso · confiável · moderno · artesanal · imprevisível · ultrapassado · frágil · rápido · divertido · poderoso · complicado · lento · acolhedor · honesto · simples · frio · profissional · confuso · organizado
- **Ordem 3 (salto 13):** complicado · organizado · confiável · poderoso · confuso · preciso · divertido · profissional · estressante · rápido · frio · sem graça · frágil · simples · genérico · ultrapassado · honesto · vivo · imprevisível · acolhedor · intimidador · artesanal · lento · bonito · moderno
- **Ordem 4 (salto 17):** confiável · sem graça · intimidador · confuso · simples · lento · divertido · ultrapassado · moderno · estressante · vivo · organizado · frio · acolhedor · poderoso · frágil · artesanal · preciso · genérico · bonito · profissional · honesto · complicado · rápido · imprevisível
- **Ordem 5 (salto 19):** confuso · vivo · preciso · imprevisível · divertido · acolhedor · profissional · intimidador · estressante · artesanal · rápido · lento · frio · bonito · sem graça · moderno · frágil · complicado · simples · organizado · genérico · confiável · ultrapassado · poderoso · honesto

## 4. Análise e PASS (da régua D0 — contagem de palavras-alvo, NUNCA score)

| Métrica | PASS | Por quê assim |
|---|---|---|
| Acerto de alvo | **≥60% dos testers com ≥2 palavras-alvo no top-5** | Convenção declarada (D0/L2 — sem benchmark publicado; calibrar nas 2 primeiras rodadas e registrar como Decisão) |
| Kill-words | **Nenhuma kill-word em >1 tester** | 1 ocorrência é ruído; 2+ é padrão — e kill-word repetida derruba a rodada mesmo com alvos batidos |
| Sinais específicos | "intimidador" marcado por ≥2 → hipótese "assusta" do gate F2-2 tem evidência; "lento" marcado por ≥2 → cruzar com a medição da camada (d) | Leitura dirigida, definida ANTES da rodada (evita pescaria post-hoc) |

**Proibições (críticas MeasuringU, A1):** nunca usar % de palavras positivas como score (instável com n<14 — só bate com o conjunto completo ~70% das vezes em n=14); nunca reportar como número-benchmark. O entregável da análise é: **frequência por palavra + verbatims dos porquês**, lado a lado com SEQ/SUS (estes sim, instrumentos validados).

## 5. Limitações declaradas

- O deck mede a reação ao app rodando NO conjunto da sessão (depois das tarefas) — humor contaminado por sucesso/falha nas tarefas é inerente ao desenho; por isso a leitura é por rodada (5 testers), nunca por indivíduo.
- A tradução pt-BR não tem validação publicada (não existe deck pt-BR validado); mitigação: original em inglês rastreável por palavra + porquês verbatim que ancoram a interpretação.
- As 5 palavras-alvo não são neutras para quem conhece a F2-0-D — quem aplica (fundador) NÃO deve mencionar atributos da marca antes do bloco 5 da sessão.
