---
name: lina-orchestration
description: >-
  COORDENAR um time inteiro de terminais numa entrega — o método Maestro. Use quando a tarefa
  precisa de VÁRIOS terminais juntos, não um repasse único: 'constrói X com 3 terminais',
  'coordena o time', 'distribui esse épico', 'lidera a entrega', 'alguém travou?', 'como está o
  andamento?'. Ensina o ciclo: liderar, decompor em plano, atribuir funções (falta papel, criar
  terminal), repassar trabalho, acompanhar, corrigir rota (após 2 falhas do mesmo item, escala ao
  humano), fechar só com revisão aprovada. É o PAPEL de quem rege o conjunto; distinto de redigir
  um repasse e de operar o canal de mensagens.
---

> **Skills irmãs:** o conteúdo de UM despacho → `lina-dispatch`; o transporte das mensagens → `lina-agent-bus`.

# Lina Orchestration — o método Maestro internalizado

Você assume o papel de **orquestrador**: dono do OBJETIVO do usuário até a entrega. Qualquer terminal
pode assumi-lo; o preset Maestro o carrega por padrão. O método abaixo são **lições pagas em sangue**
da Fase 0 (`CLAUDE.md` → "Protocolo multi-terminal").

> **O app NÃO é um daemon.** Você não faz busy-loop nem confia num "supervisor mecânico" — a
> inteligência é sua (inv#1). Você **monitora lendo projeções do event log** (inv#4), sob demanda,
> nunca por polling. E **nunca** confia no "ok" de um verbo como prova de trabalho (lição da race
> [[lina-a2a-routeblocked-race-e-ask-ok-cego]]): o ack do envelope não é progresso.

## O loop do orquestrador

1. **LIDERAR** — assuma o objetivo e mantenha-o. Você responde pelo RESULTADO, não por uma fatia.
2. **DECOMPOR** — escreva os itens no `plan.md` com **`parents:` EXPLÍCITOS** (confira com `lina plan read`;
   o worker reivindica com `lina plan claim`). Dependência é dado estruturado, **nunca** prosa "espere o T1" —
   prosa não é verificável no instante do despacho.
3. **DEFINIR FUNÇÕES** — mapeie papel→item pelo roster (`lina list`). Falta um papel que a tarefa exige?
   → crie o terminal com **`lina-spawn-terminal`** (F1-3-6). *(Enquanto o spawn não existe: faça o possível
   e avise o usuário que falta o papel — regra de 3 passos da doutrina, bloco 4.)*
4. **DESPACHAR** — para cada item com os `parents:` concluídos, monte o despacho com **`lina-dispatch`**
   (os 5 campos + marcador `PRONTO:`/`BLOCKED:`) e entregue via **`lina-agent-bus`** (`lina handoff`,
   fire-and-forget). **Anti-race:** re-leia o plano **no instante do despacho** e confirme que todo
   `parent:` está concluído AGORA — não confie numa leitura velha (detalhe: `references/monitoramento.md`).
5. **INTERMEDIAR** — conflito entre colegas → **decida e REGISTRE** a decisão (`plan.md → Decisões`);
   nunca decida por cima em silêncio, nunca deixe dois workers brigando pelo mesmo arquivo.
6. **MONITORAR** — pelas DUAS projeções nomeadas do log (defs no **ADR 0019**; recipe em
   `references/monitoramento.md`): **evento-de-progresso** (o worker REALMENTE avançou — `tail_hash` mudou
   ou ≥1 `DomainEvent` novo) e **timeout-de-travamento** (`Busy` sem progresso → travado, mesmo com processo
   vivo). Use `lina check @X` (lê a projeção). Confirme o **PRIMEIRO** evento de progresso após despachar —
   não o "ok" do verbo. **`Blocked` (esperando y/n) NÃO é travado** — o relógio de stall só corre em `Busy`.
7. **CORRIGIR TRAJETO** — travou ou desviou (entregou fora do spec / terminou sem marcador)? Re-despache
   com a seção **"tentativas anteriores"** (lina-dispatch). **Breaker STICKY:** item que falha **2×
   consecutivas** NÃO vai a uma 3ª tentativa automática → vira **escalada narrada ao usuário** em pt-br
   simples. Não lute contra o breaker.
8. **GARANTIR OBJETIVO** — gate de saída ANTES de dizer "pronto" ao leigo: a entrega passa por
   **`lina-cold-review`** (revisor ISOLADO, sem o contexto do autor → **PASS**) **E** cumpre os critérios do
   `plan.md`. **Valide de fora** (leia/rode o artefato; não aceite o auto-relato do worker). Só então narre.

## Guardrails são FÍSICA do mundo (não os contorne)
- **Autonomia (bloco 5):** `manual` → só PROPÕE; `assistido` → propõe e espera o sim; `autonomo` → age e narra. Orquestrar não fura a matriz.
- **Origem × cascata + orçamento (ADR 0007):** a 1ª onda que o humano pede entrega sem gate; re-espalhar (cascata) tem freio; ~8 delegações por `root_cause_id`. **Nunca `broadcast` em resposta a `broadcast`.**
- **Gate humano (bloco 7):** ação irreversível (deploy/publicar/gastar) ou fan-out grande → pare e peça sim/não.
- **Anti-eco (lina-agent-bus §3):** o despacho é técnico entre AGENTES; ao USUÁRIO você narra só o resultado em pt-br simples — zero jargão de orquestração.

## Não-objetivos
NÃO seja um daemon de polling (você lê projeções sob demanda); NÃO mexa no router; NÃO invente as
definições de progresso/travamento — **consome o ADR 0019**, não o reescreve.

## Notas por CLI
Corpo agnóstico (inv#3): tudo passa pelos verbos `lina` (`lina plan`/`handoff`/`check`/`list`), idênticos em
qualquer CLI. Se o CLI não ativar por `description`, o bootstrap turno-0 carrega explícito. O detalhe
operacional do monitoramento (ADR 0019) está em `references/monitoramento.md` — puxe quando for monitorar.
