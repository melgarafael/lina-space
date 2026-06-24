---
name: lina-design-doctrine
description: >-
  Direção VISUAL com opinião para qualquer interface — tela, página, componente, landing,
  dashboard, slide. Use ao desenhar, estilizar ou escolher
  fonte/cor/paleta/tipografia/espaçamento: 'faz a tela de X', 'estiliza esse componente', 'qual
  fonte/paleta?', 'monta a landing', 'escolhe as cores', 'tema claro/escuro'. Bane os defaults
  sem opinião (Inter/Roboto/Arial por inércia, gradiente roxo de IA, glassmorphism genérico,
  shadcn cru) e exige direção estética declarada antes de estilizar, tokens semânticos, escala
  tipográfica deliberada e movimento que respeita reduce-motion. É a aparência da interface; não
  a palavra escrita nem o parecer de revisão.
---

> **Skills irmãs:** texto/copy → `lina-copy-doctrine`; veredito de revisão → `lina-cold-review`.

# Lina Design Doctrine — design tem opinião

Saída visual **sem opinião é slop** (competência superficial, reprodutível para qualquer produto —
rubrica §0). O default que sai de graça do framework é o pior inimigo: parece pronto e não decide nada.
Sua régua é a dimensão **DESIGN** da rubrica anti-slop (`lina-cold-review/references/rubrica.md`).

## 1. Banir (marcadores duros — DES-1/2/3/5)
- **Fonte default como escolha** (`Inter`, `Roboto`, `Arial`, `system-ui`): proibida como decisão
  estética. Pode aparecer **só** como *fallback* honesto depois de uma fonte escolhida.
- **Gradiente "AI-purple"**: roxo→branco / roxo+azul decorativo (`#7c3aed`, `#a855f7`, `#6366f1`).
- **Glassmorphism genérico**: `blur()` + `rgba(255,255,255,.1)` como enfeite sem função.
- **Convergência shadcn-default**: tudo border-radius médio + shadow suave + espaçamento de fábrica.

## 2. Exigir (DES-4 — o invariante de design)
1. **Direção estética declarada ANTES de estilizar.** Um statement curto: um nome/escola
   ("Swiss editorial", "Brutalismo", "Solarpunk tech"), uma referência, ou 3 adjetivos +
   "por que não o default". **Sem direção declarada, o design não passa.**
2. **Leia o vault do usuário primeiro** — se há identidade/marca/voz visual registrada, ela manda
   (a direção vem do contexto dele, não da sua fábrica). Cite a origem ("vi em [[nota]]").
3. **Tokens semânticos**, não hex solto: `--ink`, `--paper`, `--accent` — cor com intenção.
4. **Escala tipográfica deliberada** (contraste de tamanho/peso/ritmo), não tamanhos avulsos.
5. **Motion com intenção** e sempre sob `@media (prefers-reduced-motion: reduce)`.

## 3. Processo
ler vault → declarar a direção (1 parágrafo) → escolher fonte/cor/ritmo como **decisão** →
conferir contra os banimentos (§1) → só então estilizar.

## 4. Checklist (antes de entregar)
- [ ] Direção estética declarada e visível (DES-4)? · [ ] Zero fonte default-como-escolha (DES-1)?
- [ ] Zero gradiente roxo/glass genérico (DES-2/3)? · [ ] Tokens semânticos + escala deliberada?
- [ ] A direção está **materializada** (o CSS reflete o statement), não só escrita?

## Notas por CLI
Corpo agnóstico (inv#3): nada aqui depende de um CLI específico. A única ação externa é **ler o vault**
(via a ferramenta de leitura/`lina vault` quando num Espaço Lina). Se o seu CLI não ativar por
`description`, o bootstrap turno-0 ordena o carregamento explícito.
