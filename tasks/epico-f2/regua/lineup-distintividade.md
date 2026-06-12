# Protocolo do line-up de distintividade (kit da régua F2 · camada c)

> **Dono:** Terminal D (QA) · **Data:** 2026-06-12 · **Fontes:** régua D0 (`tasks/pesquisa-f2/entrega-d0-eval.md` camada c, achado A4 — mecanismo Romaniuk/Distinctive Asset Grid) · decisão F2-0-D (território "Instrumento de Estúdio com a temperatura do Ateliê")
> **O que mede:** se a cara do Lina é **reconhecível de memória** entre apps da mesma categoria — distintividade real, não opinião ("achei bonito" não conta aqui).

---

## 1. O mecanismo (por que é assim)

Distintividade se mede com estímulo **sem marca** → atribuição (A4): o tester vê 5 telas de-brandeadas e aponta qual é a do app que usou. Com chance de 20% (1 em 5), acerto consistente só acontece se a **combinação** de elementos (layout + paleta + tipografia + temperatura) ficou na memória — e o dado duro de Romaniuk diz que elemento isolado quase nunca distingue (4% das cores, 6% das taglines). Por isso o protocolo também pergunta **o que** guiou a escolha.

## 2. O line-up — 5 estímulos (os 4 lookalikes nomeados na D0)

| # | Estímulo | O que representa na categoria |
|---|---|---|
| 1 | **Lina** (canvas com 4-6 terminais, território F2-0-D aplicado) | nós |
| 2 | **Warp** | terminal moderno "bonito" — o lookalike mais perigoso |
| 3 | **Terminal genérico** (ex.: iTerm2/Terminal.app com tema escuro padrão) | a cara default da categoria |
| 4 | **Chat-IA genérico** (UI de chat com painéis, estética default de IA) | "é só mais um chat?" |
| 5 | **VS Code com splits** (4-6 painéis de terminal abertos) | "é só um editor com terminais?" |

## 3. Montagem dos estímulos (igualar TUDO que não é identidade)

A regra: as 5 imagens só podem diferir naquilo que estamos medindo — a identidade visual. Qualquer outra diferença vira pista falsa e invalida o acerto.

1. **De-brandear:** remover/borrar logo, nome do app, ícone do dock, menu bar com nome, URLs e qualquer texto que nomeie o produto (inclusive nos conteúdos de terminal).
2. **Conteúdo comparável:** todas as telas com 4-6 painéis/áreas visíveis, texto de trabalho genérico e equivalente (logs/lista/prosa — NUNCA o mesmo texto literal em duas telas).
3. **Mesmo formato:** mesma resolução de captura, mesmo crop (janela cheia, sem desktop ao fundo), mesmo zoom aparente, exportadas no mesmo tamanho final.
4. **Tema:** todos em dark (os terminais do Lina são sempre-dark por decisão F2-0-D; um line-up misto entregaria o Lina pelo tema, não pela identidade).
5. Nomear os arquivos de forma neutra (`tela-A.png` … `tela-E.png`) e guardar um gabarito separado (qual letra é o Lina em cada ordem).

## 4. Aplicação

- **Quem:** testers que usaram o Lina **1 vez** (a rodada da camada b), **no dia seguinte** (D+1) — mede memória, não percepção imediata.
- **Como:** mensagem assíncrona (WhatsApp/e-mail) com a prancha das 5 telas lado a lado, ~10 min.
- **Pergunta 1 (exata):** *"Uma destas 5 telas é do aplicativo que você usou ontem. Qual delas? (responda a letra)"*
- **Pergunta 2:** *"De 1 a 5, quão certo você está?"*
- **Pergunta 3 (a mais valiosa):** *"O que te fez escolher essa?"* — resposta literal; é ela que diz QUAL combinação de elementos carrega a identidade (e se alguma pista falsa vazou).
- **Sem feedback** sobre acerto/erro até todos responderem.

## 5. Randomização (documentada — exigência do despacho)

- A posição do Lina na prancha muda por tester: **5 ordens pré-geradas** (Lina na posição 1, 2, 3, 4, 5), atribuídas em rodízio na ordem em que os testers responderam a rodada (tester 1 → ordem 1, tester 2 → ordem 2, …). Com 10 testers, cada posição aparece 2×.
- O gabarito (tester → ordem → letra correta) fica na planilha do QA, fora da prancha enviada.
- Os 4 lookalikes também rodam de posição junto (rotação circular da mesma prancha — gera as 5 ordens sem novo trabalho de montagem).

## 6. PASS e leitura honesta

| Métrica | PASS | Natureza |
|---|---|---|
| Identificação correta | **≥8/10** testers acertam (acumulado 2 rodadas; chance = 20%) | Convenção declarada (D0/L2) — inspirada no limiar "Hero" 50/50 de Romaniuk, que é SUSPECT para n pequeno (A4); calibrar nas 2 primeiras rodadas e registrar como Decisão |
| Pista declarada (pergunta 3) | A maioria cita **combinação** (cor de estado + temperatura quente + organização do canvas), não um elemento só, e nenhuma resposta cita pista falsa (ex.: "era a única com 6 painéis") | Qualitativa — é o teste de sanidade da montagem §3 |

- **O que este teste NÃO mede:** preferência ("qual é mais bonita?"), usabilidade, ou fama de marca real (Distinctive Asset Grid pede amostra grande — aqui usamos só o MECANISMO, com alvo declarado como convenção).
- Erro sistemático para o MESMO lookalike (ex.: 2+ testers escolhem o Warp) é achado de design de primeira ordem: a identidade está colidindo exatamente ali — vai para a rodada de correção com prioridade.

## 7. Checklist de preparação (antes da 1ª rodada)

- [ ] Capturar a tela do Lina com o território F2-0-D aplicado (depende do gate F2-1/F2-2 — antes disso o line-up mediria a cara VELHA: não rodar)
- [ ] Capturar/obter as 4 telas lookalike e de-brandear as 5 (§3)
- [ ] Montar a prancha nas 5 ordens + gabarito na planilha
- [ ] Validar com 1 pessoa de fora (não-tester): "alguma tela se entrega por algo que não é o visual?" — se sim, refazer antes de gastar testers
