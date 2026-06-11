# A/B CEGO da doutrina gênio-criativo (F1-3-1) — RESULTADO · 2026-06-10/11

> **Protocolo:** 3 tarefas criativas (hero de landing · e-mail de lançamento · naming+manifesto)
> × 2 seeds × 2 braços (COM a doutrina `assets/lina-doctrine/CLAUDE.md` lida antes vs SEM) =
> 12 geradores independentes; 6 juízes CEGOS (ordem X/Y alternada por índice — o juiz nunca
> sabe qual braço é qual) avaliando pela rubrica anti-slop (`lina-cold-review/references/
> rubrica.md`). Workflow `ab-cego-doutrina-f1-3-1` (run `wf_196801af-b66`, retomado por cache
> após o incidente de limite de conta; 18 agentes).

## Placar: **COM 5 × 1 SEM** (0 empates)

| Par | Vencedor | Score COM | Score SEM | Marcador duro no perdedor |
|---|---|---|---|---|
| landing/0 | **COM** | 90 | 72 | DES-4 ALTA (sem direção estética declarada) → FAIL |
| landing/1 | **COM** | 92 | 62 | DES-4 ALTA + entrega truncada (MÉDIA grave) → FAIL |
| copy/0 | **COM** | 93 | 74 | COP-2 ALTA (placeholder "[Seu nome]" como final) → FAIL |
| copy/1 | **COM** | 96 | 91 | — (ambos PASS; COM mais específico/comprometido) |
| naming/0 | **COM** | 94 | 86 | — (ambos PASS; SEM com MÉDIA de jargão vazando) |
| naming/1 | **SEM** | 88 | 92 | — (ambos PASS; única vitória do SEM, por menor ritmo-de-template) |

**Média de score: COM ≈ 92,2 · SEM ≈ 79,5** (o SEM, na média, nem passa o limiar 80 da rubrica).

## Leitura (o que o dado diz)

1. **A doutrina elimina os marcadores DUROS:** zero violação ALTA em 6/6 outputs COM; o braço
   SEM reprovou na rubrica em 3/6 (metade!) — sempre pelos marcadores que a doutrina bane
   nominalmente (direção estética não-declarada; placeholder entregue). Não é ganho de "gosto":
   é a diferença entre PASSAR e FALHAR o gate de qualidade do produto.
2. **Quando ambos passam, a margem encolhe** (96×91, 94×86, 88×92) — a doutrina não é mágica
   em cima de output já bom; o valor dela está em impedir o piso ruim. Consistente com a tese
   da F1-3 (anti-slop como GATE, não como tempero).
3. **A derrota única (naming/1)** foi por "ritmo de template" (tríades anafóricas) num par em
   que ambos passaram — sinal honesto de que a doutrina não imuniza contra maneirismo de IA;
   candidata de refinamento v2 da doutrina (banir cadência anafórica empilhada), NÃO bloqueia.

## Veredito

**Item 6 do conselho F1-3 (A/B cego da doutrina): ✅ FECHADO — a doutrina melhora o output de
forma mensurável e reprodutível pela própria rubrica do produto.** Refinamento v2 sugerido
(cadência anafórica) registrado para o ciclo do `lina retro` — sugere-nunca-aplica.
