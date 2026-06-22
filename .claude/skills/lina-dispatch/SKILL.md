---
name: lina-dispatch
description: >-
  O template RICO de despacho: como o orquestrador monta o prompt de uma tarefa para um worker
  executar SOZINHO, sem perguntas de esclarecimento. Use SEMPRE que for despachar, delegar ou
  transferir trabalho de verdade a outro terminal — gatilhos: "monta o despacho", "manda o
  Fulano implementar/revisar/pesquisar X", "o que escrevo no handoff?", "delega essa story",
  "distribui o épico", "re-despacha o que falhou". Cobre os 5 campos canônicos (CONTEXTO ·
  FUNÇÃO · DIRECIONAMENTO · OBJETIVO · RESULTADO ESPERADO), o marcador PRONTO:/BLOCKED:, o
  padrão pull-then-context (a mensagem leva o ponteiro + o essencial; o worker puxa o pesado) e
  o re-despacho informado ("tentativas anteriores"). É o CONTEÚDO do payload — o verbo lina e a
  proteção do leigo são da lina-agent-bus. Agnóstica de CLI.
---

# Lina Dispatch — o template RICO de despacho (worker autossuficiente)

**O despacho É o harness.** O modo de falha nº1 da orquestração multi-agente não é o LLM
fraco — é o despacho **pobre**: "conserta o login" e o terminal fresco não sabe ONDE está o
código, COM QUE REGRAS pode mexer, nem O QUE conta como entregue. Este template é **doutrina
para você, o agente** (inv#1: quem preenche é o orquestrador; o app não renderiza nada) e faz
um terminal **fresco executar sem um único `ask` de esclarecimento**.

> Esta skill é o **CONTEÚDO** do despacho. Quem escolhe o verbo (`lina handoff`/`ask`),
> entrega no PTY do colega e **protege o leigo** (antídoto de eco) é a **`lina-agent-bus`**.
> O despacho viaja A2A **entre agentes** — pode (e deve) ser técnico e rico. Ao **USUÁRIO**
> você narra só o resultado em pt-br simples ("deleguei a API pro nosso backend").

## Os 5 campos canônicos (a checklist — todos obrigatórios)

| # | Campo | O que vai nele | De onde PUXAR |
|---|---|---|---|
| 1 | **CONTEXTO** | onde o trabalho está: `cwd`, **arquivos:linha** a ler, handoffs dos itens-pais, **tentativas anteriores** (se re-despacho) | `lina plan read <ID>`, vault, event log |
| 2 | **FUNÇÃO** | o papel DESTE terminal nesta tarefa: "você é o revisor isolado", "você é o dono do CSS" | o roster (`lina list`) |
| 3 | **DIRECIONAMENTO** | as regras do jogo: escopo ("mexa SÓ em X"), proibições, doutrina aplicável, orçamento | CLAUDE.md, ADRs |
| 4 | **OBJETIVO** | o resultado de **negócio** em 1-2 frases — o "por quê" que habilita decisão autônoma boa | a story / o pedido |
| 5 | **RESULTADO ESPERADO** | o **formato exato** da entrega + o marcador `PRONTO: <resumo>` / `BLOCKED: <motivo>` | você define |

**Sem o marcador de conclusão, é falha de protocolo.** O orquestrador trata um retorno sem
`PRONTO:`/`BLOCKED:` como worker travado (F1-3-4) — não como "deu certo, só não avisou".
Exija o marcador no campo 5, sempre.

## Pull-then-context (mande o mapa, não o território)

A mensagem A2A carrega o **PONTEIRO** + o essencial; o contexto pesado o worker **PUXA**.
Empurrar 50 KB pelo payload trunca (contexto grande corta) e polui o bus/log.

- **No envelope:** `ref: plan:T4` (o item) e `context: docs/api.md` (anexo, só em handoff) —
  campos que **já existem** no Envelope A2A; o template os USA, não inventa formato novo.
- **No payload:** o caminho + a ordem de puxar — *"leia `onda-3.md:128-146` e rode
  `lina plan read T4` ANTES de começar"*.
- **O worker abre** `ref`/`context` ao receber (`lina plan read`, `lina vault read`, ler o
  arquivo). Os 5 campos vão no `payload`; o campo 5 espelha-se no `[EXPECTED]` do envelope.

## Re-despacho informado (correção de trajeto)

Re-despachar SEM dizer o que falhou repete o erro. No 2º despacho, o campo **CONTEXTO** ganha
a seção **"tentativas anteriores"**: o que foi tentado, por que falhou, o que NÃO repetir.
(Breaker: item que falha 2× consecutivas não vai a uma 3ª tentativa automática — vira
escalada narrada ao usuário em pt-br simples.)

## BOM vs RUIM, por tipo de tarefa

| Tipo | ❌ RUIM (despacho pobre — gera `ask` de volta) | ✅ BOM (os 5 campos preenchidos) |
|---|---|---|
| **Build** | "implementa a API de leads" | **CTX:** `cd repo`; contrato em `docs/api.md:1-40`; `ref: plan:T4` · **FUNÇÃO:** dono do backend de leads · **DIR:** mexa SÓ em `api/leads.rs`; zero `unwrap` em prod; siga ADR 0012 · **OBJ:** o formulário do site precisa gravar o lead no CRM · **RES:** o arquivo + teste verde; termine com `PRONTO: <resumo>` ou `BLOCKED: <motivo>` |
| **Review** | "revisa isso aí" | **CTX:** artefato em `fixtures/bom/`; rubrica via skill `lina-cold-review` · **FUNÇÃO:** revisor ISOLADO (sem o contexto do autor) · **DIR:** aplique a rubrica; **não conserte**, só julgue · **OBJ:** barrar slop antes do merge · **RES:** `PASS`/`FAIL` + violações (`ID·sev·arq:linha`); `PRONTO:`/`BLOCKED:` |
| **Pesquisa** | "pesquisa sobre auth" | **CTX:** a dúvida é OAuth2 vs magic-link p/ nosso caso; veja `01-norte.md` · **FUNÇÃO:** pesquisador · **DIR:** ≥3 fontes confiáveis; cite; não invente · **OBJ:** decidir o login do MVP sem retrabalho · **RES:** tabela de trade-off + **1** recomendação decisiva; `PRONTO:`/`BLOCKED:` |
| **Bug** | "tá quebrado, arruma" | **CTX:** erro `X` em `auth.rs:88`; repro nos passos Y; **tentativa anterior:** subir o timeout NÃO resolveu (era race) · **FUNÇÃO:** bug-fixer · **DIR:** ache a causa-raiz; sem fix temporário nem `catch` que engole · **OBJ:** usuário não consegue logar · **RES:** diff + o teste que falhava agora verde; `PRONTO:`/`BLOCKED:` |

> **Exemplo vivo:** o próprio despacho que criou esta skill seguia o padrão
> (CONTEXTO/FUNÇÃO/DIRECIONAMENTO/OBJETIVO/RESULTADO ESPERADO) — e um terminal fresco o
> executou sem perguntar nada. É a prova de autossuficiência em ação.

## Antes de disparar — checklist mecânica

- [ ] Os **5 campos** estão preenchidos (nenhum vazio)?
- [ ] **CONTEXTO** aponta `arquivo:linha` + `ref:` em vez de colar o conteúdo todo?
- [ ] **DIRECIONAMENTO** declara a fronteira ("mexa SÓ em X") + proibições?
- [ ] **RESULTADO ESPERADO** tem formato exato **e** o marcador `PRONTO:`/`BLOCKED:`?
- [ ] Se é **re-despacho**: a seção "tentativas anteriores" está no CONTEXTO?

Faltou algum item? O worker vai te perguntar de volta — e essa pergunta É o modo de falha que
este template existe para eliminar. Despachos são eventos auditáveis (inv#4): o payload fica
no log para inspeção.
