---
name: lina-retro
description: >-
  Retrospectiva do Espaço: ler o relatório do comando 'lina retro' e PROPOR melhorias com
  evidência apontável. Use para olhar para trás e melhorar o time/setup: 'roda o retro',
  'retrospectiva', 'o que dá pra melhorar no Espaço?', 'que skills criar ou arquivar?', 'que
  papéis faltam?', 'analisa custos/histórico', 'onde o time trava?'. Lê as 5 seções do relatório
  (skills, coordenação, custos, pedidos, lacunas) e propõe em três tipos — skills, papéis,
  presets — cada proposta citando o número ou evento que a justifica. Tudo passa por gate humano:
  sugere, o humano decide, nunca aplica. É olhar para trás, não a regência de agora.
---

> **Skill irmã:** coordenar o trabalho de AGORA → `lina-orchestration` (aqui é olhar para trás).

# Lina Retro — observar o trabalho do Espaço e SUGERIR (o humano decide)

`lina retro` é um verbo **determinístico, ZERO LLM**: ele projeta o event log num **relatório** de
fatos (inv#1 — quem pensa é você; inv#4 — tudo é projeção reconstruível do log). Seu papel: **ler o
relatório e propor melhorias com evidência**. O verbo **só observa e sugere** — toda mutação
(arquivar/fixar skill, criar papel, mudar preset) é **gate humano**, fora do verbo.

> **Rode `lina retro`** (sem argumentos). Saída magra? É honesto: log novo dá "dados insuficientes" —
> nesse caso **NÃO invente sugestões**, avise que falta histórico e siga. Nunca confie em achismo: a
> evidência é o relatório, não a sua memória.

## Como ler o relatório (5 seções)
- **SKILLS** — uso por skill, "última atividade há Xd" e estado: `ATIVA` / `STALE (>30d)` /
  `ARCHIVE (>90d)`. Mais a lista de **consolidação** (nomes próximos: `growthos-sales`,
  `growthos-narrative` → cluster `growthos`).
- **COORDENAÇÃO** — onde o time trava: roteamentos **bloqueados** por motivo, **spawns** barrados,
  **re-delegações** da mesma tarefa (mesmo `root_cause_id` rodou de novo) e **breaker** disparado.
- **CUSTOS** — gasto por terminal (tokens), com **papel** e **OUTLIER** (gasto muito acima da média).
- **PEDIDOS DO USUÁRIO** — o que o humano mais pede nos **turnos de origem** (`hops==0`), por intenção.
- **LACUNAS DE PAPEL** — papéis/alvos **pedidos e ausentes** (ninguém tinha o papel → `no_target`).

## Proponha nos TRÊS tipos — cada um com evidência APONTÁVEL
Toda proposta cita o **número/evento** do relatório que a sustenta. Sem evidência ⇒ sem proposta.

1. **SKILLS**
   - *Criar* skill X porque uma sequência se repete — evidência: PEDIDOS ("`ask` 14×") ou re-delegações
     da mesma tarefa em COORDENAÇÃO. Ex.: "criar skill `revisao-rapida`: o pedido de revisão apareceu
     14× e hoje cada um vira 3 idas-e-voltas (re-delegação `r1`: 4)".
   - *Arquivar* Z — evidência: SKILLS `ARCHIVE (>90d)`. Arquivar **≠ deletar** (reversível).
   - *Consolidar* um cluster — evidência: a lista de consolidação. Ex.: "unificar `growthos-*` (2 skills)".
2. **PAPÉIS**
   - Papel **ausente recorrente** — evidência: LACUNAS ("`designer`: 2 pedidos"). Ex.: "adicionar papel
     Designer ao preset: foi pedido 2× e caiu em `no_target`".
   - **Quebrar tarefas** de um papel que trava — evidência: breaker/re-delegações em COORDENAÇÃO.
3. **PRESETS** (o conjunto papéis+skills que o padrão de pedidos sugere)
   - Evidência: PEDIDOS + LACUNAS juntos. Ex.: "o usuário sempre pede landing+copy → preset 'Landing'
     com papéis Designer+Copywriter e as skills `growthos-sales`/`lina-copy-doctrine` já carregadas".

## Gate humano é FÍSICA do mundo (não o contorne)
- Você **PROPÕE em pt-br simples** ("Sugiro arquivar a skill X — sem uso há 100 dias. Topa?"). O
  **humano decide**. Você **nunca** arquiva/fixa/cria sozinho.
- `lina retro` **não tem** subcomando de mutação — `lina retro archive ...`/`--apply` é **recusado** de
  propósito. Quando o relatório sugerir prune, isso é **insumo para o humano**, não uma ação sua.
- **Anti-eco:** ao usuário, narre só a proposta e o porquê em linguagem leiga — zero jargão de evento.

## Não-objetivos
NÃO é orquestrar o trabalho de agora (use **lina-orchestration**); NÃO rode em loop/daemon (é
**on-demand** — você roda `lina retro` quando pedem); NÃO **aplique** nenhuma mudança; NÃO invente
números — todo dado vem do relatório.

## Notas por CLI
Agnóstica (inv#3): `lina retro` é idêntico em qualquer CLI. Se o CLI não ativar por `description`, o
bootstrap turno-0 carrega explícito. Use `lina retro --json` se preferir ler os dados estruturados.
