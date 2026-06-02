# ADR 0001 — `plan.md` event-sourced + variantes de plano no `DomainEvent`

- **Status:** aceito
- **Onda/Story:** Onda 3 · W3-5 (`plan.md` — plano compartilhado, supervisor escritor único)
- **Data:** 2026-06-02

## Contexto

W3-5 pede um **plano compartilhado** (`.lina/plan.md`) com duas exigências que colidem
com invariantes do `CLAUDE.md`:

1. **"O event log é a fonte da verdade. Layout/buffers/tabelas são projeções
   reconstruíveis."** → o `plan.md` **não pode** ser estado autoritativo; tem de ser
   uma *projeção* reconstruível por replay do log.
2. **"Envelope A2A versionado — não invente formatos por feature."** → reivindicar
   (`claim`) / concluir (`check`) um item precisa viajar pelo envelope/log já
   existentes, sem um formato-fio paralelo.

O catálogo `DomainEvent` (W0-5) hoje não tem como representar mutações de plano, e o
`MailMessage` (`lina/msg@1`, W3-4) não carrega o item referenciado. Implementar W3-5
exige tocar nos dois — e ambos são **âncoras de continuidade** (portas que, uma vez
abertas errado, fragmentam o contrato). Daí este ADR antes de codar.

## Decisão

### 1. O `plan.md` é uma PROJEÇÃO do event log (não estado autoritativo)

O supervisor é o **escritor único** de `.lina/plan.md`. A cada mutação ele:
1. valida o comando contra a **projeção atual** (`EventStore::project().plan`);
2. **append** do evento no log (fonte da verdade) — *antes* de tocar o arquivo;
3. reescreve `.lina/plan.md` = `render` da projeção pós-evento (escrita atômica
   `tmp`+`rename`, mesmo padrão do `agents.json`).

Logo o arquivo é **sempre** derivável: `open(dir) → project() → plan.render()` reproduz
o `plan.md` byte-a-byte. Nenhum outro processo escreve nele (anti-corrupção
cross-platform sem `flock`). Qualquer processo pode **ler** direto (`lina plan read`).

### 2. Quatro variantes novas em `DomainEvent` (event_version=1)

- `PlanItemAdded { item, desc }` — semeia um item (`status:todo`, `@owner:?`).
- `PlanDecisionAdded { text }` — adiciona uma decisão viva.
- `PlanClaimed { id, item, by }` — `id`=id da mensagem de origem; `item`=ref do item;
  `by`=quem reivindicou. Projeta `@owner:by` + `status:doing`.
- `PlanChecked { id, item, by }` — idem; projeta `status:done`.

`id`/`item`/`by` ficam no MESMO registro do log → satisfaz o aceite ("asserir os
campos no registro") e dá a base do anti-loop por item de plano + `root_cause_id`
(design §4). São **aditivas**: o reducer `apply` ganha braços novos; os tipos antigos
não mudam de versão (sem upcasting). `DomainEvent` não é `#[non_exhaustive]`, mas o
único consumidor externo (`app/lina-gpui`) só **constrói** variantes (`append`), nunca
faz `match` exaustivo — adicionar variantes **não quebra o app**.

### 3. Campo `ref` no `MailMessage` (`lina/msg@1`) — aditivo, sem bump de versão

O envelope canônico (design §3.4) **já lista** `ref` como campo opcional
(`item do plano (plan:T4)`). Implementá-lo agora é fechar uma lacuna do contrato, não
alargá-lo: `#[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]`
— mesma política aditiva-retrocompatível já usada por `root_cause_id`. Mensagens antigas
(sem `ref`) continuam desserializando (`None`). Por isso **não** se bumpa `lina/msg@1`.

`lina plan claim T1` deposita `intent="plan.claim"`, `ref="plan:T1"`, `from=<terminal>`.
O supervisor, ao drenar, **intercepta** `plan.claim`/`plan.check` **antes** do pipeline
de delegação: NÃO entrega a PTY — aplica ao plano + loga.

## Alternativas consideradas

- **Projeção separada do plano (fora de `ProjectedState`)** com replay próprio: evitaria
  tocar `ProjectedState`, mas duplicaria a maquinaria de replay/snapshot/determinismo já
  testada (W0-5). Rejeitada — o plano *é* uma projeção do log; pertence a `ProjectedState`.
- **`plan.md` como estado autoritativo com `flock`**: viola o invariante #4 e a decisão
  cross-platform anti-`flock` (design §4). Rejeitada.
- **Campo novo no envelope em vez de reusar `ref`**: fragmentaria o contrato. Rejeitada —
  `ref` já estava desenhado para exatamente isto.
- **Reusar `payload` para o item id**: sobrecarrega semanticamente um campo de texto livre
  e ignora o `ref` spec'd. Rejeitada.

## Consequências

- **(+)** `plan.md` 100% reconstruível do log; teste de replay prova reconstrução idêntica.
- **(+)** Zero formato-fio novo; reusa `EventStore`, `Mailbox`, `A2aEnvelope`, `Router`.
- **(+)** App intacto (só construções de `DomainEvent`); API pública de `lina-core` só
  **cresce** (campos/variantes aditivos) — sem quebra de assinatura existente.
- **(−)** `ProjectedState` ganha o campo `plan` (`#[serde(default)]`): snapshots antigos
  desserializam com plano vazio; o fingerprint determinístico passa a incluir o plano
  (esperado — o plano é parte do estado).
- **(−)** Reescrever `plan.md` reprojetar o log a cada mutação é O(n) sem snapshot; aceitável
  no MVP (poucos eventos) e já mitigável pelos snapshots existentes.
