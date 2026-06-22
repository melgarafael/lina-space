---
name: lina-spawn-terminal
description: >-
  O 4º passo da regra de 3 da doutrina: quando NINGUÉM no workspace tem o papel que a tarefa
  exige, o agente traz o especialista criando um terminal novo com lina spawn — sempre dentro
  dos gates. Gatilhos: "falta um QA no time", "ninguém aqui é backend/revisor", "cria/spawna um
  terminal de X", "traz um especialista de Y", "o plano pede um papel que o roster não tem".
  Cobre QUANDO spawnar (e quando não), como nomear o papel, o 1º prompt (= template
  lina-dispatch; use caminhos ABSOLUTOS — o terminal novo nasce em pasta própria, não no seu
  cwd), os GATES como física (cascata → aval humano SEMPRE; teto 2/turno; manual bloqueia; conta
  no custo) e a narração ao leigo. NÃO é para falar com terminais existentes (lina-agent-bus)
  nem redigir o despacho (lina-dispatch). Agnóstica de CLI.
---

# Lina Spawn — o time se completa sozinho (dentro dos gates)

**O 4º passo da regra de 3.** A doutrina (bloco 4) manda: 1) é do SEU papel? → faça;
2) precisa de outro papel que um colega TEM? → delegue; 3) ninguém tem? → antes, era "faça o
possível e avise". Agora existe o passo 4: **ninguém tem o papel E a tarefa o exige → crie o
terminal especialista**. O time se completa sozinho — e cada criação passa pelos gates do
Espaço, porque spawn é a capacidade mais sensível que você tem: cria um processo, consome
recursos e **gasta o dinheiro do usuário**.

## Quando spawnar (e quando NÃO)

Spawne só quando as TRÊS forem verdade:

1. **A falta é real AGORA**: confira o roster com `lina list` no instante da decisão (nomes
   mudam na sessão; memória de roster envelhece).
2. **A tarefa EXIGE o papel**: a qualidade do resultado depende dele (ex.: entrega sem QA
   quando o critério de aceite pede teste independente).
3. **"Fazer o possível" comprometeria o resultado** — senão o passo 3 clássico ainda vale.

**NÃO spawne quando:** um colega existente cobre o papel (→ delegue, passo 2); a falta é
pontual e você cobre bem o suficiente (passo 3); ou você só quer paralelizar/acelerar —
velocidade não é papel faltante, e cada terminal novo conta no teto de custo do Espaço.

## Como

```
lina spawn @<Nome> --role <papel> [--prompt "<1º prompt>"]
```

- **Nome = papel**: `@QA`, `@Backend`, `@Revisor`. Aceita sem `@` (o verbo normaliza).
- **`--role` é o NOME simples do papel** (`qa`, `backend`): o role-discovery do workspace
  resolve a definição canônica (`.lina/roles/<papel>.md`) e as skills do papel. Não descreva
  o papel no flag — nomeie-o.
- **`--prompt` é o 1º despacho — monte-o SEMPRE com o template `lina-dispatch`** (os 5 campos
  CONTEXTO · FUNÇÃO · DIRECIONAMENTO · OBJETIVO · RESULTADO ESPERADO + marcador
  `PRONTO:`/`BLOCKED:`). O terminal nasce FRESCO; sem despacho rico ele volta com perguntas.
  O flag é opcional na sintaxe; spawnar sem prompt é meio-trabalho — só se justifica quando o
  despacho virá em seguida por outro canal.

O novo terminal nasce DENTRO do time (bootstrap turno-0: doutrina + papel + skills), entra na
confiança do Espaço por pertencimento e aparece no roster.

## Leia a resposta do verbo (o ack não é o desfecho)

| Resposta | Significado | Sua ação |
|---|---|---|
| `APROVADO` | gate passou; o canvas traz o especialista; o 1º prompt já está na fila dele | confirme depois pelo roster (`lina list`) e pelo handshake — nunca pelo "ok" do verbo |
| `NAO foi automatica — <motivo>` | gate humano: pedido VÁLIDO aguardando aval (não é erro seu) | narre ao usuário POR QUE falta o papel e AGUARDE o aval; **nunca re-tente em loop** |
| sem desfecho no tempo de espera | o Espaço pode estar ocupado | confirme com `lina list`; não re-enfileire às cegas |

## Os gates são física do mundo — não obstáculos

| Gate | Regra |
|---|---|
| Autonomia `manual` | você NÃO cria terminais: o verbo recusa na hora → sugira a criação ao usuário |
| Autonomia `assistido` | o pedido vira proposta; nada nasce sem o "sim" do humano |
| **Cascata** | você recebeu uma tarefa de outro agente e quer spawnar → **aval humano SEMPRE** (anti-fork-bomb: worker que cria workers é tempestade em potencial) |
| Teto por turno | máximo **2 spawns por turno**; o 3º cai no gate humano |
| Custo | terminal novo conta no teto de custo do Espaço; teto atingido → só o usuário retoma (`lina resume --confirm`) — não insista |

**Não há contorno — por construção.** A distinção origem×cascata vem de um binding INTERNO do
router, nunca de campos da mensagem: forjar `hops`/root no envelope não muda nada. Pedir a um
colega que spawne por você também não funciona: o pedido viaja A2A, o colega herda a cadeia e
cai no MESMO gate de cascata. E todo pedido fica registrado no log (`intent: spawn` — o Espaço
vê o pedido, os args reais e a decisão). Um gate não é um bug a resolver: é o usuário no
controle do próprio dinheiro. Tentar contorná-lo é violação de doutrina.

## Narração ao leigo (obrigatória)

Spawn permitido na origem vem COM narração — sempre, em pt-br simples, sem jargão:

> "Trouxe um especialista de QA pro time porque essa entrega precisa de testes que ninguém
> aqui cobre. Ele já recebeu a primeira tarefa."

Caiu no gate? Narre o pedido e o porquê: *"Pedi pra trazer um especialista de X porque
[motivo]; o Espaço aguarda seu ok pra criar."* Nunca despeje o bloco técnico do verbo no leigo.

## Erros comuns

| Erro | Realidade |
|---|---|
| Spawnar para paralelizar | spawn responde a PAPEL faltante, não a pressa; custa dinheiro do usuário |
| `--role` com descrição longa | o role é um NOME; a definição vem do role-discovery |
| 1º prompt "vai fazendo X" | terminal fresco sem os 5 campos = ping-pong de perguntas; use `lina-dispatch` |
| Re-tentar após gate | o gate é decisão humana pendente; loop de spawn = tempestade |
| Tratar gate como falha sua | pedido válido aguardando aval — narre e aguarde |
| Spawn silencioso | a narração ao leigo é obrigatória mesmo quando o spawn é permitido |
| Confiar no "ok" como prova | o desfecho real é o terminal no roster + handshake, não o ack |
