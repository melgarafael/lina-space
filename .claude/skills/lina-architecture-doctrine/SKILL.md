---
name: lina-architecture-doctrine
description: >-
  Decisões de ESTRUTURA com simplicidade primeiro: quando criar (ou não) uma abstração, camada,
  interface, dependência ou refactor. Use ao organizar código ou escolher entre abordagens: 'como
  organizo isso?', 'vale criar uma abstração?', 'qual arquitetura pra X?', 'isso é
  over-engineering?', 'como deixar extensível?'. Regras: a MENOR mudança que resolve; abstração
  só com 2+ consumidores reais hoje; nada de complexidade especulativa; e a pergunta de
  continuidade 'isto fecha uma porta?' antes de decidir (se fecha, registrar a decisão antes).
  Trata a forma do sistema, não o detalhe de implementação nem a aparência.
---

> **Skills irmãs:** detalhe de implementação e nomes → `lina-code-doctrine`; aparência/layout → `lina-design-doctrine`.

# Lina Architecture Doctrine — a menor mudança que resolve

Abstração especulativa e complexidade não pedida são slop arquitetural: caras de manter, baratas de
gerar, justificadas por um futuro que talvez não venha (rubrica §0 + dimensão **ARQUITETURA**,
`lina-cold-review/references/rubrica.md`). Sua régua é ARQ-1..3.

## 1. Princípios
- **A mudança certa é a MENOR que resolve o problema.** Toque só no necessário.
- **Sem abstração especulativa (ARQ-1):** interface/factory/manager/`<T>` só com **≥2 consumidores
  ou implementações reais HOJE**. "Pode ser útil depois" não é requisito — é especulação.
- **Sem complexidade não pedida (ARQ-2):** nada de config/eventos/plugin/camadas que o critério de
  aceite não exige. A funcionalidade existiria sem essa camada? Então ela não entra.
- **Solução do tamanho do problema (ARQ-3):** existe um caminho mais simples que entrega o mesmo
  critério? Use-o. Over-engineering é dívida, não capricho.

## 2. A pergunta de continuidade — "isto fecha uma porta?"
Antes de **qualquer** decisão arquitetural não-trivial, pergunte: *"isto fecha uma porta acima?"*
(espelha o norte §3 do `CLAUDE.md`). Se a decisão tranca uma evolução futura (troca de UI, de motor,
de CLI; o event log como fonte da verdade; fronteiras core↔shell), **pare e registre um ADR curto** —
não decida no impulso da story. Mantenha vivas as âncoras de continuidade.

## 3. Elegância sob demanda
Antes de implementar algo não-trivial, pause: *"existe um caminho mais elegante?"*. **Pule essa etapa
para fixes óbvios** — não over-engineer um conserto trivial. Elegância é a simplicidade certa, não
camada extra.

## 4. Checklist
- [ ] É a menor mudança que resolve? · [ ] Toda abstração tem ≥2 consumidores reais hoje (ARQ-1)?
- [ ] Removi camada/config que o critério não pediu (ARQ-2)? · [ ] Existe caminho mais simples (ARQ-3)?
- [ ] Esta decisão fecha uma porta de continuidade? Se sim → ADR antes de seguir.

## Notas por CLI
Corpo agnóstico (inv#3): nenhuma dependência de CLI. O registro de ADR é um arquivo em `docs/adr/`
(escrita normal de arquivo). Se o CLI não ativar por `description`, o bootstrap turno-0 carrega explícito.
