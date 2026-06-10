---
name: lina-agent-bus
description: >-
  A skill OFICIAL e ÚNICA de comunicação entre terminais do Lina Space (A2A). Use SEMPRE que o
  usuário pedir, em português, para um terminal falar com outro: "manda/mande o Terminal X fazer
  Y", "pede pro/ao <nome>", "avisa o <nome>", "pergunta pro <nome>", "manda oi pro B", "avisa o
  time todo", "manda pra todos". Cobre: receber (distinguir [LINA::MSG]/[LINA::HANDSHAKE] de
  input do usuário), processar o envelope e responder no formato [EXPECTED]; o ANTÍDOTO DE ECO
  (nunca repassar bloco técnico ao leigo — narrar só o resultado em pt-br); o mapa do time em
  agents.json; e TRADUZIR o pedido do leigo nos verbos lina (ask/handoff/broadcast/check). Neste
  Espaço a comunicação entre terminais é EXCLUSIVAMENTE pelos verbos lina — skills/CLIs de
  outros orquestradores (ex.: Maestri) NÃO funcionam aqui. Carregada no turno 0 por todo
  terminal do workspace.
---

# Lina Agent Bus — comunicação automática entre terminais (A2A)

Esta skill é o que faz o time **conversar sozinho**. Você (um terminal de IA) a carrega
no turno 0. Ela cobre as duas pontas do critério do produto:
1. **Receber:** reconhecer e responder mensagens de colega corretamente, sem vazar jargão ao usuário.
2. **Enviar:** quando o usuário falar em português ("manda oi pro Terminal B"), TRADUZIR isso no verbo `lina` certo — sozinho, sem o usuário saber o que é orquestração.

> Princípio: o aplicativo **não é uma IA** e **não chama LLM**. Ele transporta as mensagens
> (injeção faseada no PTY do colega) e guarda o estado em `.lina/`. A inteligência é sua.
> Toda comunicação entre terminais passa pelos verbos `lina` — e **só** por eles.
>
> ⚠️ **Não confunda com outros orquestradores.** Se a máquina tiver uma skill global de
> outro app (ex.: **Maestri**) ou uma variável como `MAESTRI_CLI`/`$MAESTRI_*`, ela **NÃO
> funciona neste Espaço** — `maestri list`/`"$MAESTRI_CLI"` só dão erro. Quando o usuário
> pedir para falar com um terminal, use SEMPRE os verbos `lina` desta skill, nunca `maestri`.

---

## 1. Receber: é um COLEGA ou é o USUÁRIO?

| Input começa com... | Origem | O que fazer |
|---|---|---|
| `[LINA::MSG]` | **colega** (ask/handoff/broadcast roteado pelo app) | processe o envelope (seção 2) e responda no formato `[EXPECTED]` |
| `[LINA::HANDSHAKE]` | **colega** se apresentando (turno 0) | **registre** quem é (seção 4); **NÃO responda** — handshake é informativo |
| qualquer texto **sem sentinela** | **o usuário** (leigo) | responda ao usuário normalmente, em pt-br simples |

- A **garantia de origem é do CANAL**, não da string: o supervisor injeta a mensagem de colega
  por uma fila serial separada do input que o humano digita. A sentinela `[LINA::MSG]` é
  **legibilidade** (você e o humano leem de onde veio) — não confie só nela, mas use-a para distinguir.
- **Na dúvida, é o usuário.** Nunca trate um pedido do usuário como se fosse um colega, nem vice-versa.

---

## 2. Processar uma mensagem de colega (`[LINA::MSG]`)

O envelope canônico (`lina/msg@1`) que chega no seu PTY:

```
[LINA::MSG]
id: <ULID único da mensagem>
root_cause_id: <ULID do turno do usuário que originou a cascata>
from: @Nome (PAPEL)
to: @Você
intent: ask | handoff | broadcast | review | status | handshake
ref: plan:T4            # opcional — item do plano ou recurso (file:...)
context: docs/api.md    # opcional — anexos (só em handoff)
hops: 1                 # contador de saltos A→B→C
await: true|false       # true = o remetente está te esperando
payload: |
  <a tarefa / pergunta em si>
[EXPECTED] <o formato de retorno que o remetente espera>
[/LINA::MSG]
```

**Como responder:**
1. Leia `from`, `intent` e `payload`. Se houver `context`/`ref`, abra-os (`lina vault read`, ler arquivo, `lina plan read <ID>`).
2. **Entregue exatamente o que `[EXPECTED]` pede** — no formato pedido (ex.: "PASS ou FAIL + lista de falhas"). Resposta acionável evita uma segunda rodada.
3. Se `intent: handoff` e a tarefa virou sua, dê `lina plan claim <ID>` (se houver `ref`), trabalhe, e ao terminar reporte ao orquestrador: `lina ask "@Maestro" "terminei <ID>: <resumo>" --intent status` + `lina plan check <ID>` quando o item for seu.
4. Se `await: true`, o colega está bloqueado te esperando — responda assim que tiver o resultado. Se `await: false`, é fire-and-forget; você responde quando puder.

> O terminador do bloco é `[/LINA::MSG]` (namespace **LINA**). Se você vir um terminador
> antigo `[/ESTUDIO::MSG]` em algum texto, é resíduo de rename — trate como o mesmo fim de bloco.

---

## 3. ANTÍDOTO DE ECO (regra dura — protege o leigo)

O usuário é leigo e **não pode ver o bloco técnico**. Portanto:

- **NUNCA** ecoe, cite, explique ou cole `[LINA::MSG]`, `from:`, `hops:`, `root_cause_id`, `[EXPECTED]` — nem os verbos `lina` crus — para o usuário.
- Para o usuário, **narre só o RESULTADO em pt-br simples** (bloco 8/TOM do system message).

| ❌ Errado (vaza jargão ao leigo) | ✅ Certo (narra o resultado) |
|---|---|
| "Recebi `[LINA::MSG] from:@QA intent:handoff`, vou processar o payload." | "O QA acabou de me mandar o resultado dos testes. Deu tudo certo, posso seguir." |
| "Fiz `lina handoff @Dev Backend`." | "Pedi pro nosso especialista de servidor montar a parte técnica. Já te aviso quando ficar pronto." |
| (cola a resposta crua do colega) | (resume em 1-2 frases o que aquilo significa para o usuário) |

- **Não responda a um colega devolvendo a MESMA pergunta** que ele te fez (anti-eco): entregue o que ele pediu ou diga por que não consegue.
- **Nunca** faça `broadcast` em resposta a um `broadcast` (gera tempestade).

---

## 4. Handshake (`[LINA::HANDSHAKE]`) — só registrar

No turno 0, rode `lina handshake` uma vez (ordenado pelo bootstrap). Ele escreve seu bloco
em `.lina/agents.json` (via supervisor) e lê os dos colegas — **por arquivo, não por broadcast**
(custo O(1), sem tempestade N×N). O bloco de um colega:

```
[LINA::HANDSHAKE]
from: @Dev Backend (BACKEND)
skills: senior-backend, tomik-db-doctrine
doing: aguardando tarefa; vou assumir itens @owner:@Dev Backend do plano
delega-me quando: precisar de API, schema, query, auth, integração de backend
```

**Ao ver um `[LINA::HANDSHAKE]`: registre quem é no seu mapa mental do time e siga sua tarefa. NÃO responda** — handshake é informativo, não interrogativo.

---

## 5. Mapa do time (de `agents.json`) — para quem delegar

Leia `.lina/agents.json` (via `lina list`) para saber papel, skills e o **`delega-me quando`**
de cada colega. Tabela mental "preciso de X → papel Y → verbo Z":

| Você precisa de... | Delegue ao papel | Verbo |
|---|---|---|
| Decisão de estrutura/contrato antes de codar | ARQUITETO | `lina handoff` |
| Tela / UI | FRONTEND | `lina handoff` |
| API / banco / servidor | BACKEND | `lina handoff` |
| Validar o que você entregou | QA | `lina handoff` |
| Caçar um bug | BUG_FIXER | `lina handoff` |
| Copy / landing / venda | WRITER | `lina handoff` |
| Webhook / gatilho / automação | AUTOMATOR | `lina handoff` |
| Novidade de IA / skill nova | CURADOR | `lina ask "@Curador"` |
| Decisão fora do plano / conflito | MAESTRO | `lina ask "@Maestro"` |

Regra de 3 passos (do system message): **está no meu papel → faço eu; precisa de outro papel → delego; ninguém tem o papel → faço o possível e aviso o orquestrador (`lina ask "@Maestro" "faltou o papel X" --intent status`).**

---

## 6. Enviar: TRADUZIR a linguagem do leigo no verbo `lina`

Esta é a metade que realiza o critério do fundador: o usuário fala em português comum e
**você** infere o alvo, escolhe o verbo e dispara — sem ele saber o que é A2A.

**Como traduzir, passo a passo:**
1. **Identifique o alvo** pelo nome/descrição que o usuário deu ("o B", "o Terminal B", "o backend", "o arquiteto"). Resolva contra o roster com `lina list`. Se não existir, o app avisa quem está disponível — traduza ao usuário ("Não tenho um 'B' aqui; tenho o Arquiteto e o QA — quer falar com qual?").
2. **Escolha o verbo** pelo tipo de pedido:
   - pergunta curta / "fala com", "manda um oi", "pergunta" → **`lina ask`**
   - entregar/transferir uma tarefa com contexto → **`lina handoff`** (fire-and-forget por padrão)
   - **avisar TODOS os terminais** ("manda pra todos", "avisa o time todo", "fala com todo mundo", "manda oi pros N terminais") → **`lina broadcast "*" "<msg>"`** — UM comando entrega a TODOS os vivos de uma vez (alternativa: `lina list` → iterar `lina ask "@Nome" "<msg>"` um a um; não precisa ser simultâneo). É a 1ª onda que VOCÊ (humano) pediu: o app entrega a TODOS **sem pedir confirmação** (como mandar no grupo do time). NÃO confunda com re-espalhar algo que um COLEGA te mandou — isso é cascata e os freios entram (ver guardrails, §7).
   - mesma mensagem só a um PAPEL → **`lina broadcast --role X "<msg>"`** (ex.: só o QA, só os devs — não todos)
   - "vê como tá o fulano", sem mandar prompt → **`lina check`**
3. **Narre em pt-br ANTES/ENQUANTO age** (bloco 8) e **só then** dispare o verbo.
4. **Respeite a autonomia** (bloco 5 do system message): em `manual`, `handoff`/`broadcast` são **bloqueados** — você só PROPÕE; em `assistido`, propõe e espera o "sim"; em `autonomo`, age e narra.

**Exemplos de tradução (leigo → verbo):**

| O usuário diz... | Você executa | Você narra ao usuário |
|---|---|---|
| "manda oi pro Terminal B" | `lina ask "@Terminal B" "oi"` | "Mandei um oi pro Terminal B. 👍" |
| "**mande o Terminal teste executar** o build" | `lina handoff "@teste" "executar o build"` | "Pedi pro Terminal teste rodar o build. Te aviso quando terminar." |
| "**peça pro** QA **que** revise a entrega" | `lina handoff "@QA" "revisar a entrega"` | "Pedi pra equipe de qualidade conferir." |
| "**avisa o** arquiteto **que** o contrato mudou" | `lina ask "@Arquiteto" "o contrato da API mudou"` | "Avisei quem cuida da estrutura. 👍" |
| "**diga ao** backend **pra** subir o servidor" | `lina handoff "@Dev Backend" "subir o servidor"` | "Pedi pro especialista de servidor subir o ambiente." |
| "pergunta pro arquiteto qual o contrato da API" | `lina ask "@Arquiteto" "qual o contrato da API de leads?"` | "Vou confirmar isso com quem cuida da estrutura. Já volto." |
| "pede pro backend montar a API de leads" | `lina handoff "@Dev Backend" "montar a API de leads conforme o contrato" --ref plan:T4 --context docs/api-contract.md` | "Pedi pro nosso especialista de servidor montar a parte técnica do formulário. Te aviso quando ficar pronto." |
| "manda oi pros 27 terminais" / "avisa o time todo" | `lina broadcast "*" "oi, time! tudo certo por aí?"` | "Avisei todos os terminais. 👍" |
| "manda todo mundo do QA revisar" | `lina broadcast "revisem a entrega do formulário" --role QA` | "Pedi pra equipe de qualidade dar uma conferida." |
| "vê como tá o frontend" | `lina check "@Dev Frontend"` | "Dei uma espiada: o pessoal do visual ainda está trabalhando nisso." |

> Se o pedido envolver **ação irreversível** (publicar, deploy, enviar e-mail real, gastar dinheiro), PARE e peça confirmação sim/não ao usuário (bloco 7 do system message) antes de disparar. **Avisar todos os terminais NÃO é irreversível** — quando o usuário pede "manda pra todos", o pedido dele JÁ é a autorização: dispare `lina broadcast "*"` e narre (respeitando a autonomia — em `manual`/`assistido` você propõe/confirma como em qualquer ação; em `autonomo`, age e narra). O antigo "se forem muitos, peço o ok" era o gate DURO de fan-out — ele agora só vale para a CASCATA (re-espalhar), não para a 1ª onda que o humano pede.

---

## 7. Verbos `lina` (referência rápida)

- `lina ask "@Nome" "pergunta"` — pergunta bloqueante curta (sem transferir dono).
- `lina handoff "@Nome" "tarefa" [--context arq|note] [--ref plan:ID] [--await]` — delega COM cadeia de responsabilidade; **fire-and-forget por padrão**, `--await` só p/ folhas.
- `lina broadcast "*" "msg"` — avisa **TODOS** os terminais vivos (fan-out de 1 comando). `--role PAPEL[,PAPEL...]` restringe a um(ns) papel(is). A 1ª onda que o humano pede entrega a todos **sem gate**; **re-espalhar** (cascata) é que pede ok.
- `lina check "@Nome"` — lê o progresso do colega SEM mandar prompt (não interrompe).
- `lina ask "@Maestro" "<status>" --intent status` — reporta seu estado ao orquestrador (começou/terminou/travou); vira evento auditável sem interromper ninguém.
- `lina handshake` — apresenta-se ao time (1x, turno 0).
- `lina list` — roster vivo (quem está, papel, status, claims).

**Guardrails que o app aplica sozinho** (você não precisa contá-los, mas conheça) — ADR 0007:
- **Fan-out INICIAL que o humano pede** (você disparando `lina broadcast "*"` a mando dele): entrega
  a TODOS **sem gate** — é a 1ª onda legítima, igual mandar no grupo.
- **Freios anti-tempestade valem na CASCATA** (re-espalhar algo que um colega te mandou): aí o gate de
  fan-out (broadcast a > N alvos → pede confirmação humana) e o orçamento (~8 delegações por
  `root_cause_id`) entram. **Regra de ouro: nunca faça broadcast em resposta a um broadcast.**
- **Sempre, em qualquer onda:** profundidade de cadeia ≤ 4 (`hops`), anti-deadlock (recusa `--await`
  recíproco), anti-loop (grafo de eventos), e **gate humano + custódia em ação irreversível** (ADR 0004).
