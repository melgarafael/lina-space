# SPEC — Contrato de eventos do roteamento de skills (ADR 0045 C2/C3)

- **Status:** Spec de contrato **fechada** (decisão A3 do ADR-gate LOOP; Arquiteto, 2026-06-24). Implementa o "evento de domínio novo" do ADR 0045 §52.
- **Dono da aplicação:** **Terminal J** aplica em `crates/lina-core/src/events.rs` na Onda-1 (dono único de `events.rs` nesta rodada — item `R45-OUT`). **Esta spec NÃO toca `events.rs`** — é o desenho que o J segue.
- **Invariantes:** eventos **aditivos** (`#[serde(default)]` em todo campo novo → replay de log antigo nunca quebra, inv #4/"eventos aditivos"); skill é **DADO, jamais autoridade** (ADR 0045 §Segurança); ZERO LLM no core.

## 1. Princípio de conciliação (ler antes de codar)

O `SkillSelected` **já existe** (`events.rs:1372`, F3-5-4) com `{ node, skill, trigger, source }` e já é emitido server-side (`router.rs:2475`, `node` = remetente autenticado). O ADR 0045 §52 foi escrito em termos genéricos (`{task_kind, candidates, chosen, by_role, rank_reason}`) **antes** de checar o evento existente. Esta spec concilia:

- **`chosen` NÃO é adicionado** — o campo `skill` existente **é** a skill escolhida. Adicionar `chosen` duplicaria (duplicação é dívida). A projeção de ranking lê `skill` como o chosen.
- Os 4 campos atuais permanecem; os campos novos entram **aditivos**.

## 2. `SkillSelected` — campos novos (aditivos)

| Campo | Tipo | Estado | Semântica |
|---|---|---|---|
| `node` | `String` | existe | NodeId que selecionou (instância) — server-side |
| `skill` | `String` | existe | **a skill escolhida** (o `chosen` do ADR) |
| `trigger` | `Option<String>` | existe | gatilho que casou (`#[serde(default)]`) |
| `source` | `String` | existe | catálogo/hub/fábrica |
| `task_kind` | `String` | **novo** | tipo de tarefa (§4). `#[serde(default)]` → `""` em log antigo |
| `candidates` | `Vec<String>` | **novo** | os skill_ids do top-k de C1 (retrieval). `#[serde(default)]` → `[]` |
| `by_role` | `String` | **novo** | papel do nó (agregação cross-instância, multi-workspace ADR 0010). `#[serde(default)]` → `""` |
| `rank_reason` | `String` | **novo** | por que esta venceu (retrieval-only no MVP; outcome quando C2 amadurece). `#[serde(default)]` → `""` |
| `selection_id` | `String` | **novo** | id determinístico da seleção — chave que o `SkillOutcome` referencia (§3.1). `#[serde(default)]` → `""` |

`node` (instância) e `by_role` (papel) coexistem de propósito: `node` para auditoria local, `by_role` para o ranking AGREGAR entre instâncias/workspaces.

## 3. `SkillOutcome` — evento NOVO

```
SkillOutcome {
    selection_ref: String,          // = selection_id do SkillSelected casado (§3.1)
    signal: SkillSignal,            // Success | Failure | Reworked | HumanOverride
    #[serde(default)] evidence: String,   // observabilidade/painel; opcional
}

enum SkillSignal { Success, Failure, Reworked, HumanOverride }
```

Aditivo, **META** (no-op no `apply` — reconstruído por projeção dedicada via replay, padrão `CostLedger`/`Mentality`; NÃO toca `ProjectedState`).

### 3.1 `selection_id` / `selection_ref` (ligação determinística)

`selection_id = "sel-" || sha256_hex(canon(node) || "\x1f" || canon(skill) || "\x1f" || canon(task_kind) || "\x1f" || root_cause_id)[..16]`  (`sha2::Sha256`, já em `lina-core` — mesma primitiva do ADR 0048)

- `root_cause_id` = id do turno do usuário (já existe no envelope A2A) — desambigua seleções repetidas da mesma skill/task_kind ao longo do tempo.
- `canon(x)` = `trim().to_lowercase()` com espaços colapsados (mesma normalização do ADR 0048 — consistência entre os dois sistemas).
- Determinístico → replay reconstrói a mesma ligação; o `SkillOutcome.selection_ref` casa com exatamente um `SkillSelected.selection_id`. **Não inventar id aleatório** (quebraria replay — mesma razão do `belief_id` no ADR 0048).

### 3.2 `signal` — inferência PASSIVA (ADR 0045 §4; a Lina NUNCA pergunta)

A projeção infere o sinal varrendo a janela seguinte à seleção — **sem nenhum prompt ao usuário**:

| Observação na janela seguinte | `signal` |
|---|---|
| humano/Maestro trocou a skill | `HumanOverride` → conta como **Failure** |
| entrega devolvida / re-trabalho | `Reworked` → conta como **Failure** |
| skill usada, sem troca e sem re-trabalho | `Success` (reforçado por gate/cold-review PASS, que já existem) |

`Reworked`/`HumanOverride` são variantes distintas de `Failure` para **observabilidade** (o painel distingue "trocaram" de "devolveram"); no ranking, ambas pesam como negativo. Nenhum 👍 explícito é emitido (respeita o TOM não-técnico).

## 4. `task_kind` — definição operacional (ADR 0045 §3 das decisões fechadas)

`task_kind` **emerge do retrieval**, sem classificador a priori: tarefas que recuperam o mesmo conjunto de candidatos são "do mesmo tipo".

- **MVP determinístico:** `task_kind = canon(role) || ":" || top_terms(query, n=3)` onde `top_terms` são os termos dominantes da query após stop-words (mesma tokenização do índice BM25 de C1), ordenados e juntados por `-`. Ex.: `backend:api-leads-validacao`.
- A granularidade auto-ajusta com dados; sem balde genérico forçado nem over-fitting.
- **É o MESMO `task_kind` do ADR 0048** (unidade de "situação" do `situation_hash`). Um conceito, dois usos — costura LOOP ↔ 0045.

## 5. Regras de aplicação (checklist para o Terminal J em `events.rs`)

1. Adicionar os 5 campos novos ao `SkillSelected` existente, **todos `#[serde(default)]`** (`events.rs:1372`). Não remover nem renomear os 4 atuais.
2. Adicionar a variante `SkillOutcome` + o enum `SkillSignal` (derive `Serialize/Deserialize/Clone/Debug/PartialEq`, como os demais).
3. Braço em `kind()` (`events.rs:~1665`): `SkillOutcome { .. } => "SkillOutcome"`.
4. Incluir `SkillOutcome` na lista META/no-op do `apply` (junto de `SkillSelected`, `events.rs:~2133`) — reducer no-op; projeção por replay externo.
5. **Não** criar projeção aqui — o score por replay é item separado (`R45-OUT` C2), padrão `CostLedger`.

## 6. Verificação (observável)

- **Aditividade:** replay de um log **sem** os campos novos / sem `SkillOutcome` reidrata sem erro (`#[serde(default)]`); um `SkillSelected` antigo (4 campos) desserializa com `task_kind=""`, `candidates=[]`, etc.
- **Ligação determinística:** dois `selection_id` com os mesmos `(node, skill, task_kind, root_cause_id)` batem; um `SkillOutcome.selection_ref` casa com exatamente um `SkillSelected` (replay reconstrói a ligação).
- **Outcome muda escolha (C2, quando ligado):** `SkillOutcome{Failure}` para a skill B + `{Success}` para a A, no mesmo `task_kind`, muda a escolha de B→A; replay reconstrói o score idêntico (cf. ADR 0045 §Verificação).
- **Sinal passivo:** uma seleção sem `HumanOverride` e sem `Reworked` na janela seguinte projeta `Success`; uma com troca humana projeta `Failure` — varre-se o caminho: **nenhuma pergunta de feedback é emitida ao leigo**.
- **Segurança:** skill com score máximo de outcome **não** dispensa gate humano em ação irreversível (mutação: subir o score ao máximo → o gate ainda dispara). Skill é DADO, nunca autoridade.
