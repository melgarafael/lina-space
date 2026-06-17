# ADR 0031 — `model` e `effort` no envelope de spawn e na criação humana de terminal

- **Status:** ✅ Aceito (2026-06-16, pelo fundador) — destrava as stories F3-0-3/0-4/0-5/0-6
- **Onda/Story:** F2/F3 (orquestração inteligente) · depende do spawn governado (F1-3) e do teto de custo (F1-3-6)
- **Data:** 2026-06-16
- **Fontes:** spec `51 - SystemParams` (vault, doc-fonte da orquestração inteligente, linha 58) · matriz de autonomia (`guard.rs`) · ADR 0019 §6 (spawn governado: cap/cascata/custo) · ADR 0007 (origem×cascata via binding inforjável) · ADR 0005 (teto de custo) · ADR 0022 (admissão canônica de nó)

**Gate: stories que dependem desta decisão não iniciam até este ADR ser aceito.**

## Contexto

A orquestração inteligente da Lina (spec 51, linha 58) precisa que um pedido de
spawn — e a criação direta de um terminal pelo humano — **carregue qual motor de
raciocínio o novo terminal deve usar**: o `model` (qual modelo do CLI, ex.
`opus`/`sonnet`/`haiku`, ou `None` = default do CLI Profile) e o `effort`
(intensidade de raciocínio: `Low`/`Medium`/`High`). Hoje **nenhum dos dois
existe**: `grep -rn "effort"` na árvore retorna só ocorrências de `best-effort`; o
envelope de spawn carrega apenas `name`/`role`/`prompt`/`root_cause_id`/`hops`
(`SpawnRequested` em `events.rs:712-731`; `RouteOutcome::SpawnApproved` em
`router.rs:228-234`/`1946-1952`), e o modal humano de criar Agente expõe só **4
controles** (motor·nome·papel·pasta — `agent_modal.rs:3`).

Sem esses campos, a Lina não tem como modular custo e profundidade de raciocínio
por tarefa: todo terminal nasce com o default do CLI, e o orquestrador não
consegue, por exemplo, subir o `effort` numa story difícil ou rebaixá-lo numa
tarefa trivial para caber no teto de custo. É **fundação** da orquestração — por
isso é gate (toca o spawn governado do ADR 0019).

O risco de segurança é claro e já tem precedente: `effort` é um **multiplicador de
custo**. Se um agente pudesse escalar `effort=High` à vontade via o payload do
A2A, teríamos um vetor de gasto fora de controle — exatamente a classe de ataque
que o ADR 0007 fecha tratando todo campo escrito por agente como **dado
transportado, jamais autoridade**.

## Decisão

1. **Dois campos aditivos no envelope de spawn**, ambos `#[serde(default)]`:
   - `model: Option<String>` — id do modelo pedido; `None` = usa o default do CLI
     Profile TOML (mantém a neutralidade multi-CLI: o id é **opaco** ao core, só
     o Profile o interpreta).
   - `effort: Effort` — `enum Effort { Low, Medium, High }`, com
     `#[derive(Default)]` em `Medium` (default conservador). Enum (não String) por
     ser um conjunto **fechado e governado** pelo teto de custo; razões abertas
     (como `SpawnGated.reason`) ficam String, mas `effort` é fechado de propósito.

   Adicionados a `DomainEvent::SpawnRequested`, `RouteOutcome::SpawnApproved` e ao
   `MailMessage`/funil de spawn. Aditivos: replay de log F1-3-6 (sem os campos)
   reconstrói `model = None`, `effort = Medium` — **sem bump de versão nem upcast**
   (campo `Option<T>`/`enum: Default` dispensa upcast, precedente
   `NodeAdded.requested_by` em `events.rs:199` e `TerminalSpawned.cwd`, ADR 0001 §2).

2. **`effort`/`model` carimbados SERVER-SIDE a partir do binding inforjável** — o
   `requested_by` autenticado pelo dir-dono do outbox e o `(root_cause_id, hops)`
   de `derive_root_hops` (`router.rs:1859`), NUNCA campos `from`/`msg.hops`. O
   payload do agente é a **proposta**; a autoridade que decide se ela vale é o
   gate. Herda integralmente a doutrina do ADR 0007: contrato é dado, não
   autoridade.

3. **`effort` é governado pela matriz de autonomia + teto de custo**, no mesmo
   pipeline do `handle_spawn` (`router.rs:1833-1953`), ANTES do `SpawnApproved`:
   nenhum papel escala `effort` acima do que **cabe no teto** (`token_budget_day`,
   ADR 0005). Quando o `effort` pedido projetaria gasto acima do teto disponível,
   o spawn é **rebaixado ao maior `effort` que cabe** (degradação graciosa,
   logada) ou — se nem o `Low` couber — gated por custo
   (`SpawnGated{reason:"cost"}`, ramo já existente em `router.rs:1919-1932`). A
   matriz canônica de `guard.rs` (`decide`, nível × classe) é a fonte da decisão;
   o `effort` é um **parâmetro de custo do spawn**, não um novo eixo de
   autorização — reusa os gates existentes (manual/cascata/cap/custo), não cria um
   gate paralelo.

4. **Modal humano ganha os dois controles**, mantendo a tese anti-jargão do
   `agent_modal.rs` (4 → controles enxutos, sem YAML): `model` como seleção
   **derivada do CLI Profile do motor escolhido** (lista só os modelos que aquele
   Profile declara — neutralidade), `effort` como um seletor de 3 níveis com copy
   pt-br ("rápido / equilibrado / profundo", nunca "Low/Medium/High" na
   superfície). Criação direta pelo humano (`NodeAdded.requested_by = None`,
   `events.rs:200`) **não passa pelo cap/gate de agente**, mas o `effort` escolhido
   ainda respeita o teto de custo (o humano vê o aviso, não é silenciosamente
   barrado).

5. **Invariante #4 preservado:** os campos são META no log (`SpawnRequested`
   carrega o pedido EFETIVO; `SpawnApproved` o aprovado/rebaixado). O veredito de
   custo é projeção (`CostLedger::replay`), nunca cache. A criação física segue o
   seam do app (`admit_node`, ADR 0022) — o core só decide e loga.

## Consequências

- **Positivas:** a Lina passa a modular raciocínio por tarefa (fundação da
  orquestração inteligente, spec 51 l.58) sem abrir porta de gasto — o `effort`
  entra pelo mesmo funil governado do spawn (ADR 0019), reusando cap/cascata/custo
  já testados. Neutralidade multi-CLI intacta: `model` é opaco ao core, resolvido
  pelo CLI Profile; trocar de CLI não recompila nada.
- **Custo / risco:** mais um parâmetro no envelope e no modal — mitigado por ser
  aditivo (replay antigo nunca quebra) e por o `effort` ser enum fechado
  (superfície de ataque mínima). O rebaixamento gracioso (§3) precisa de uma
  função de projeção de custo por `effort` — depende do mapeamento
  effort→multiplicador da spec 51; enquanto a spec não fixar os multiplicadores, o
  gate degrada para o comportamento conservador (gated por custo) em vez de
  rebaixar.
- **A provar (testes do router):** (a) `effort=High` de agente sob teto estourado é
  rebaixado/gated, nunca aprovado cru; (b) `model`/`effort` forjados no payload não
  burlam o carimbo server-side; (c) replay de log F1-3-6 (sem os campos) reconstrói
  `None`/`Medium`; (d) criação humana respeita o teto sem o cap de agente.

## Alternativas rejeitadas

- **`effort` como String livre** — abriria valores arbitrários e dificultaria a
  governança de custo (cada valor novo seria um caso especial). Enum fechado é a
  decisão por o `effort` ser eixo governado, não razão aberta.
- **Campo `effort` decidido pelo payload do agente sem carimbo** — repõe o vetor de
  gasto que o ADR 0007 fechou; viola "contrato é dado, jamais autoridade". O carimbo
  server-side a partir do binding é inegociável.
- **Novo gate/eixo de autorização só para `effort`** — duplicaria a matriz de
  `guard.rs` e o pipeline de `handle_spawn`. `effort` é parâmetro de custo do spawn
  já governado; reusar os gates existentes é a mudança mínima.
- **Bump de versão + upcast para os campos** — desnecessário: `Option<T>` e
  `enum: Default` reconstroem sozinhos no replay (precedente `NodeAdded.requested_by`).
  Eventos aditivos não pagam upcast.
