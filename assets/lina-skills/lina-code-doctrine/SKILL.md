---
name: lina-code-doctrine
description: >-
  Anti-slop ao ESCREVER ou alterar código. Use SEMPRE que for implementar, codar ou corrigir — e
  reconheça: "implementa essa função", "escreve o código de X", "como nomeio essa variável/função?",
  "posso silenciar esse erro?", "esse try/catch tá ok?", "tem código repetido aqui", "adiciona um
  comentário", "conserta esse bug", "esse fix resolve?", "como trato esse erro". Encarna a dimensão
  CÓDIGO da rubrica anti-slop (COD-1..5): nomes que dizem o QUÊ (nada de handleData/processData/
  manager/temp), comentário só para o PORQUÊ não-óbvio, ZERO cast/escape de tipo (any/@ts-ignore/
  type:ignore), ZERO erro engolido (catch vazio, except:pass, unwrap/expect em produção), causa raiz
  acima de fix temporário, e duplicação tratada como dívida. NÃO é para decisão de estrutura/abstração
  (use lina-architecture-doctrine) nem para dar veredito de revisão de uma entrega alheia (use
  lina-cold-review). Agnóstica de linguagem e de CLI.
---

# Lina Code Doctrine — código com intenção, não slop

Slop de código é competência superficial: parece certo de longe, empurra o custo de manutenção pra
frente (rubrica §0). Sua régua é a dimensão **CÓDIGO** (`lina-cold-review/references/rubrica.md`),
marcadores COD-1..5.

## 1. As cinco regras (= COD-1..5)
- **Nomes que dizem o quê (COD-1).** `handleData`, `processData`, `doStuff`, `data`, `temp`,
  `manager`, `util` são proibidos quando escondem a intenção. Se você precisa ler o corpo para
  saber o que a função faz, o nome falhou.
- **Comentário só para o PORQUÊ (COD-2).** Nada de `// incrementa i` / `// set the title` /
  `// Hero component`. Comentário explica a decisão não-óbvia; nunca narra o que o código já diz.
- **Zero cast/escape de tipo (COD-3).** Sem `as any`, `: any`, `@ts-ignore`, `# type: ignore`,
  `unsafe` gratuito. Modele o tipo — não descarte para calar o compilador.
- **Zero erro engolido (COD-4).** Sem `catch {}` vazio, `except: pass`, `unwrap()`/`.expect()`/`!`
  em caminho de produção, nem fallback que mascara a falha. Trate na causa ou propague com contexto.
- **Duplicação é dívida (COD-5).** Bloco copy-paste ≥2× → extraia — **sem inventar abstração**
  especulativa (isso é ARQ-1; veja lina-architecture-doctrine).

## 2. Postura (sem preguiça)
**Causa raiz > fix temporário.** Nada de silenciar warning, `try/except` que engole, ou paliativo
que esconde o problema. Padrão de engenheiro sênior: ache a causa e resolva. Impacto mínimo — toque
só no necessário; não reescreva o que não pediram nem adicione docstring em código que você não mudou.

## 3. Checklist
- [ ] Cada nome diz a intenção (COD-1)? · [ ] Nenhum comentário óbvio (COD-2)?
- [ ] Zero `any`/escape de tipo (COD-3)? · [ ] Nenhum erro engolido/`unwrap` em prod (COD-4)?
- [ ] Duplicação extraída sem abstração especulativa (COD-5)? · [ ] Resolvi a causa, não o sintoma?

## Notas por CLI
Corpo agnóstico (inv#3) e independente de linguagem — os marcadores valem em TS, Rust, Python, etc.
(os exemplos citam a sintaxe de cada uma). Nenhuma dependência de CLI. Se o CLI não ativar por
`description`, o bootstrap turno-0 carrega explícito.
