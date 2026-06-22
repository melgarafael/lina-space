# Rubrica Anti-Slop — v1

> **Artefato versionado.** Fonte da verdade do que conta como *slop* no Lina Space. Lida e
> aplicada por um **agente** (inv#1: o juiz é o agente do terminal, nunca um LLM nosso).
> Consumida pela skill `lina-cold-review` e pelo gate de várias ondas do épico F1.
> **Versão:** v1 · **Limiar de PASS:** score ≥ 80 **e** zero violação ALTA.

## 0. O que é slop (definição operacional — fonte [[13.4]] achado 1)

Slop não é "código feio"; é uma assinatura mensurável de **trabalho gerado sem opinião**:

1. **Competência superficial** — parece certo de longe, desmonta de perto (nome plausível, lógica frouxa).
2. **Assimetria de esforço** — barato de produzir, caro de manter/revisar; empurra o custo pra frente.
3. **Reprodutibilidade em massa** — sairia idêntico para qualquer produto; nenhuma decisão é deste projeto.

A rubrica abaixo traduz essa definição em **marcadores concretos e detectáveis**. Cada item tem
**descrição · como verificar · severidade**. Os pesos guiam o *julgamento* do revisor — **não são
uma fórmula que um parser executa** (inv#1). O score é a síntese do revisor, justificada pelas
violações que ele aponta.

---

## 1. Modelo de veredito

### Severidades

| Severidade | Significado | Efeito no veredito |
|---|---|---|
| **ALTA** | Marcador **duro** de slop, OU invariante/critério de aceite quebrado. | **Qualquer ALTA ⇒ FAIL**, independente do score. |
| **MÉDIA** | Slop claro, corrigível, não-fatal isolado. | Acumula e derruba o score. (~8 pts cada, julgado.) |
| **BAIXA** | Observação informativa / questão de gosto defensável. | **Nunca causa FAIL. Nunca flipa o veredito.** |

> Os **marcadores duros** (ALTA por natureza) são os que o gate da onda chama de "slop duro":
> fonte Inter/default como escolha, gradiente roxo default, copy de template, `any`/erro engolido,
> abstração especulativa. São objetivos de propósito — é o que torna o veredito **reprodutível**.

### Cálculo

- **Score** (0–100, julgado pelo agente): começa em 100; cada **MÉDIA** tira ~8; **BAIXA** não tira
  ponto. O score é a *síntese* do revisor — deve ser coerente com a lista de violações (um FAIL com
  score 95 é incoerente; um PASS com 3 ALTA é proibido pela regra abaixo).
- **Veredito:**
  - **FAIL** se há **≥1 ALTA** **OU** **score < 80**.
  - **PASS** caso contrário.

### Formato de saída (o que o revisor devolve)

```
[EXPECTED]
VEREDITO: PASS | FAIL
SCORE: <0-100>
VIOLAÇÕES (do mais grave ao menos):
  - [<ID-rubrica>] <severidade> · <arquivo>:<linha> — <evidência apontável> — <por que é slop>
  ... (vazio se nenhuma)
JUSTIFICATIVA: <1 frase: o que decidiu o veredito>
```

Cada violação **deve** citar `arquivo:linha` (evidência apontável) e o ID da rubrica. Sem evidência
apontável, não é violação — é palpite, e palpite não entra no veredito.

---

## 2. Dimensão A — CÓDIGO

Fonte: [[13.4]] achado 1 + duplicação (GitClear) + erro engolido / `unwrap`.

| ID | Marcador | Como verificar | Severidade |
|---|---|---|---|
| **COD-1** | **Nome genérico/vago** (`handleData`, `processData`, `doStuff`, `data`, `temp`, `manager`, `util`, `info`) que não diz o *quê*. | O nome descreve a intenção/domínio? Para entender o que faz, você precisa ler o corpo? Se sim → slop. | MÉDIA (ALTA se pervasivo) |
| **COD-2** | **Comentário óbvio** que repete o código (`// incrementa i`, `// set the title`, `// Hero component`). | O comentário adiciona o *porquê* não-óbvio, ou só narra o *o quê* já visível na linha? | MÉDIA |
| **COD-3** | **Cast/escape de tipo** (`as any`, `: any`, `@ts-ignore`, `# type: ignore`, `unsafe`, `Object`/`dynamic` gratuito). | O tipo foi **modelado** ou **descartado** para calar o compilador? | ALTA |
| **COD-4** | **Erro engolido** — catch vazio, `except: pass`, `unwrap()`/`.expect()`/`!` em caminho de produção, fallback que mascara a falha. | O erro é tratado na causa, ou silenciado para "passar"? (norte: sem `try/except` que engole.) | ALTA |
| **COD-5** | **Duplicação** — bloco copy-paste / lógica repetida ≥2× (GitClear: duplicação é dívida). | O mesmo trecho aparece 2×+? Extrair removeria a repetição **sem inventar abstração** (ver ARQ-1)? | MÉDIA |

---

## 3. Dimensão B — DESIGN

Fonte: [[13.4]] achado 3 — paradigma duplo: **banir** o genérico **e** exigir direção declarada.

| ID | Marcador | Como verificar | Severidade |
|---|---|---|---|
| **DES-1** | **Fonte default banida como escolha** — `Inter`, `Roboto`, `Arial`, `system-ui` usada como *decisão estética* (não como fallback honesto). | A fonte é uma decisão do projeto, ou o default que vem de graça do framework? | **ALTA** (marcador duro) |
| **DES-2** | **Gradiente "AI purple"** — roxo→branco / roxo+azul decorativo (`#7c3aed`, `#a855f7`, `#6366f1` em `linear-gradient` de enfeite). | A paleta tem intenção e contraste, ou é o gradiente genérico de template de IA? | **ALTA** (marcador duro) |
| **DES-3** | **Glassmorphism genérico** — `backdrop-filter: blur()` + `rgba(255,255,255,.1)` como enfeite sem função. | O efeito serve à hierarquia/legibilidade, ou é decoração default? | MÉDIA |
| **DES-4** | **Sem direção estética declarada.** | Existe um *statement* explícito da direção (ex.: "Brutalismo editorial", "Swiss", uma referência) no artefato/README? **Sem direção declarada, design não passa** — é o sintoma nº1 de saída sem opinião. | **ALTA** |
| **DES-5** | **Convergência genérica** — tudo border-radius médio + shadow suave + espaçamento default ("shadcn sem opinião"). | Há uma decisão visível, ou é o preset que sai da caixa? | BAIXA (MÉDIA se total) |

---

## 4. Dimensão C — COPY

Fonte: filler · genericidade de template · voz.

| ID | Marcador | Como verificar | Severidade |
|---|---|---|---|
| **COP-1** | **Filler / preâmbulo vazio** — "Certainly!", "In today's fast-paced world", "Welcome to the future", "Unlock your potential", "Elevate your…". | A frase carrega informação específica, ou é enchimento que serve a qualquer coisa? | **ALTA** (marcador duro) |
| **COP-2** | **Genericidade de template** — texto que serviria a QUALQUER produto; placeholder ("Your tagline here"), lorem ipsum entregue como final. | Trocando o nome do produto, o texto continua "verdadeiro"? Se sim → genérico. | MÉDIA (ALTA se placeholder/lorem) |
| **COP-3** | **CTA sem voz** — "Click here", "Learn more", "Get started", "Saiba mais" genérico. | O CTA diz o que acontece ao clicar, na voz do produto? | MÉDIA |
| **COP-4** | **Voz ausente/divergente** — ignora a voz do usuário quando há direção (vault/spec/critérios). | O texto soa como o produto/usuário, ou como IA neutra de fábrica? | MÉDIA |

---

## 5. Dimensão D — ARQUITETURA

Fonte: abstração especulativa · complexidade não pedida (norte: "a menor mudança que resolve").

| ID | Marcador | Como verificar | Severidade |
|---|---|---|---|
| **ARQ-1** | **Abstração especulativa** — interface/factory/manager genérico para **um** caso de uso; `<T>` sem 2º consumidor; "para o futuro" sem requisito. | A abstração tem **≥2 implementações/consumidores reais HOJE**? Se não, é especulação. | ALTA |
| **ARQ-2** | **Complexidade não pedida** — sistema de config/eventos/plugin para algo estático; camadas que o critério de aceite não exige. | O critério de aceite pedia isso? A funcionalidade existiria sem a camada? | MÉDIA (ALTA se grave) |
| **ARQ-3** | **Solução maior que o problema** — over-engineering geral; a mudança certa seria menor. | Existe um caminho mais simples que entrega o mesmo critério? | MÉDIA |

---

## 6. Calibração nos DOIS sentidos (o que faz a rubrica não virar teatro)

A rubrica existe para puxar o revisor ao **centro** — os dois modos de falha da Fase 0 são opostos
complementares, e ambos invalidam o cold-review.

### 6.1 Contra o FALSO-POSITIVO (revisor paranoico) — [[revisao-adversarial-confunde-feature-com-bug]]

O cético sem o spec super-sinaliza e rateia **a feature pedida** como bug. Antes de registrar QUALQUER violação:

1. **Triar contra o intent.** Pergunte: *"isto contradiz o que os critérios de aceite pediram?"* Se o
   comportamento **é** a feature pedida, **não é violação** — documente o raciocínio e siga.
2. **Distinga o invariante REAL da versão mais forte que você inventou.** (Ex.: "campo forjável nunca
   decide a cadeia" é o invariante; "binding imutável para sempre" é uma invenção sua que conflita com a feature.)
3. **Não rateie gosto como defeito.** Uma escolha estética declarada e coerente é PASS, mesmo que você
   faria diferente. Divergência de gosto, no máximo, é **BAIXA**.
4. **MAS** leve a sério o achado **ortogonal** ao spec (algo que o spec não pediu nem proíbe e ainda é
   slop/risco). Esse é o achado que justifica a revisão existir.

### 6.2 Contra o FALSO-NEGATIVO (revisor otimista) — [[verificador-adversarial-otimista-demais]]

O otimista assina "ok" confirmando que **o mecanismo existe**, sem desafiar se a propriedade **vale**.
**Existência de um check ≠ enforcement da invariante.** Antes de dar PASS:

1. **Desafie o enforcement.** Para cada critério de aceite, pergunte: *"isto está ENFORCED no artefato,
   ou delegado a convenção/runtime/SO não-verificado aqui?"* Se delegado e não-verificável no artefato →
   é furo condicional, **não "limpo"**.
2. **Nunca aprove "porque o código está correto".** Código correto que não **garante** a propriedade pedida
   ainda falha o critério.
3. **PASS é uma afirmação, não a ausência de objeção.** Você só dá PASS se *afirma* que os critérios estão
   satisfeitos com evidência — não por não ter encontrado nada em uma leitura rasa.

---

## 7. Procedimento de aplicação (resumo — detalhe na skill `lina-cold-review`)

1. Leia os **critérios de aceite** do artefato (o *intent*) — antes do artefato.
2. Leia o **artefato** inteiro.
3. Percorra as 4 dimensões; para cada marcador, decida com "como verificar".
4. **Triar** cada candidato a violação pela §6.1 (é a feature? é gosto?) e pela §6.2 (está enforced?).
5. Atribua severidade, calcule o score, aplique a regra de veredito (§1).
6. Devolva no formato `[EXPECTED]`. O veredito vira **evento** no log (inv#4) — o orquestrador o registra;
   você só entrega no formato.

> **Isolamento (inv crítico do mecanismo):** você recebe **só** artefato + critérios + esta rubrica.
> Você **não tem e não pede** o contexto/histórico do autor. Se ele vier junto, **ignore-o** — o
> isolamento é o que impede o auto-endosso ([[13.4]] achado 2). Julgue o artefato, não a intenção do autor.

---

## 8. Changelog

- **v1** (2026-06-06) — 17 marcadores em 4 dimensões (CÓDIGO·DESIGN·COPY·ARQUITETURA); modelo de veredito
  score+ALTA com limiar 80; calibração de dois sentidos. Promovida a artefato transversal do épico F1
  (onda-3 §5 proposta 3). Mudança de versão exige bump aqui + nota no changelog.
