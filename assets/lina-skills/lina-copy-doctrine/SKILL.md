---
name: lina-copy-doctrine
description: >-
  Escrever TEXTO que uma pessoa vai ler — headline, CTA, e-mail, post, anúncio, microcopy — sem
  soar genérico. Use ao redigir ou melhorar texto voltado ao cliente: 'escreve a headline', 'faz
  a copy da landing', 'melhora esse texto', 'qual CTA uso?', 'escreve o e-mail/post', 'como falo
  isso pro cliente'. Regras: zero filler ou preâmbulo vazio; zero molde de template (trocar o
  nome do produto e ainda fazer sentido = genérico, refazer); chamada que diz o que vai
  acontecer; a voz do usuário lida do vault quando existir; uma recomendação decisiva em vez de
  menu. É a palavra escrita pro público; não a aparência nem o comentário de código.
---

> **Skills irmãs:** layout visual → `lina-design-doctrine`; comentário de código → `lina-code-doctrine`.

# Lina Copy Doctrine — texto com voz, não enchimento

Copy genérica é o slop mais visível: serviria a qualquer produto, não diz nada de ESTE (rubrica §0).
Sua régua é a dimensão **COPY** (`lina-cold-review/references/rubrica.md`), marcadores COP-1..4.

## 1. Banir (COP-1/2/3)
- **Filler / preâmbulo vazio (COP-1):** "Certainly!", "In today's fast-paced world", "Welcome to the
  future", "Unlock your potential", "Elevate your…". Corte. A primeira frase já carrega informação.
- **Genericidade de template (COP-2):** teste do nome — *troque o nome do produto; o texto continua
  verdadeiro?* Se sim, é genérico. Placeholder ("Your tagline here") e lorem ipsum entregues = falha.
- **CTA sem voz (COP-3):** "Click here", "Saiba mais", "Get started" genérico. O CTA diz **o que
  acontece** ao clicar, na voz do produto ("Ver um cronograma de exemplo").

## 2. Exigir (COP-4 + decisão)
- **Voz do usuário (COP-4):** leia o vault — tom, público, oferta, jeito de falar. A copy soa como o
  produto/usuário, não como IA neutra de fábrica. Cite a origem ("segui o tom de [[nota]]").
- **Específico > vago:** nomeie o público, a dor, o resultado concreto.
- **UMA recomendação decisiva > menu de opções.** Não devolva 5 headlines "escolha você" — entregue a
  sua aposta e explique por quê (padrão do system prompt da Anthropic; o usuário leigo quer direção,
  não lição de casa). Ofereça alternativas só se ele pedir ou se a aposta tiver um trade-off real.

## 3. Processo
ler vault (voz/público/oferta) → escrever específico e na voz → cortar todo filler → reler com o
teste do nome (§1).

## 4. Checklist
- [ ] Zero filler/preâmbulo (COP-1)? · [ ] Passa no teste do nome — não-genérico (COP-2)?
- [ ] CTA diz o que acontece, com voz (COP-3)? · [ ] Soa como o usuário, voz lida do vault (COP-4)?
- [ ] Entreguei UMA recomendação decisiva, não um menu?

## Notas por CLI
Corpo agnóstico (inv#3): a única ação externa é **ler o vault** (ferramenta de leitura/`lina vault`
num Espaço Lina). O princípio vale em qualquer idioma e CLI. Se o CLI não ativar por `description`,
o bootstrap turno-0 carrega explícito.
