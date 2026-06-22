# ADR 0041 — `paths` reservados no `PlanItem`: cruzar `CodeChanged.paths` × claims

- **Status:** 🟡 Proposto (2026-06-22, Maestro Terminal A) — gate da story F3-4-3
- **Onda/Story:** F3-4-3 (épico `39`, spec `36` §2)
- **Data:** 2026-06-22
- **Fontes:** spec `36` §2 (sinal de mudança + pertencimento) · ADR 0032 (parents/claims no `PlanItem`) — **este ADR estende o modelo de claim do 0032** · família ADR 0007 (campo de agente nunca decide autoridade) · invariante #4 (event log aditivo) · código: `crates/lina-core/src/plan.rs:90-112` (`PlanItem`), `plan.rs:206` (`try_claim`), `plan.rs:446` (`parse_item`), `router.rs:1973-1980` (`PlanClaimed { id, item, by }`)

> **Gate:** F3-4-3 não inicia até este ADR ser aceito.

## Contexto

A spec 36 §2 manda: ao chegar um `CodeChanged{paths}`, cada nó compara `paths` **× claims** — sem interseção ignora; com interseção PARA e pede direção. O problema: **o claim hoje reserva apenas o ID do item**, nunca QUAIS arquivos. O `PlanItem` (`plan.rs:90-112`) carrega `id`/`desc`/`owner`/`status`/`goal_id`/`parents`/`acceptance`/`budget_tokens` — sem campo de paths. O ADR 0032 adicionou `parents` mas não paths. Sem onde guardar os arquivos reservados, não há o que cruzar.

**Correção factual (achado do Terminal I, 2026-06-22):** uma versão anterior deste ADR supôs que `PlanClaimed` carregava o `PlanItem` completo. **Errado** — `PlanClaimed { id, item: String, by }` (`events.rs:569`) carrega apenas a *ref* do item (id), não o struct; e `PlanItemAttributed { item, goal_id, parents, acceptance, budget_tokens }` (`events.rs:654`) — o veículo dos metadados ricos do item — **não tinha paths**. Consequência: se `paths` vivesse só no `PlanItem` em memória, ele nunca entraria no event log → `store.project()` reconstruiria `paths: []` → a comparação `paths`×claims ficaria **inerte por replay** e o "replay idempotente" (F3-4-4) seria improvável. Por isso `paths` precisa ser **persistido no evento** (`PlanItemAttributed`), não só no struct projetado.

## Decisão

### 1. Estender `PlanItem` com `paths` aditivo

Em `crates/lina-core/src/plan.rs:90`, acrescentar:

```rust
#[serde(default)] pub paths: Vec<String>,  // arquivos/globs que o item reserva (relativos ao repo)
```

`#[serde(default)]` — invariante #4: plano/log anterior reconstrói com `paths: []`, round-trip byte-a-byte exato. O parser rígido (`parse_item` :446) e o serializer (`@paths:`, ao lado de `@parents:` ~:363) ganham o campo opcional (ausência ≡ vazio).

**E persistir no EVENTO** (sem isso a projeção fica inerte — ver Contexto): `PlanItemAttributed` (`events.rs:654`) ganha `#[serde(default)] paths: Vec<String>` (aditivo; replay F3-1 sem paths → `[]`), e o `apply()` (`events.rs:1603`) threada `paths` para o `PlanItem` projetado via `apply_item_attributed`. A projeção "quais paths o nó X reservou" junta `PlanClaimed` (item→`by`) com `PlanItemAttributed` (item→`paths`) por replay. **Decisão de fronteira (2026-06-22):** essa edição cirúrgica em `events.rs` é feita na branch da Trilha B (Terminal I), excepcionalmente, por estar acoplada ao `apply_item_attributed`/`PlanItem` (de I) — a Trilha A não toca `events.rs`/`PlanItemAttributed`, então não há risco de conflito entre workers; o Maestro especifica o contrato e revisa na integração.

### 2. Comparação determinística, ZERO LLM

A interseção `CodeChanged.paths` ∩ (paths dos itens com claim de **outro** nó) é de conjuntos de strings normalizadas (relativas ao repo), determinística. Vive no handler novo de `CodeChanged` (Trilha B, `router.rs`). Resultado:
- **∅** → o nó ignora em silêncio (não muda de estado).
- **≠ ∅** → o nó dono do claim PARA e abre item de atenção `CodeConflict` (ADR/story F3-4-4).

### 3. `paths` é DADO, jamais autoridade (família ADR 0007)

`paths` declara **o que um item cobre** — nunca identidade, posse ou autorização. Um agente não compra privilégio escrevendo `paths` num envelope: a posse segue por `owner` validado (`AlreadyOwned`), o escritor do `plan.md` segue sendo o supervisor (ADR 0001). A comparação só pode **abrir um alerta** (fail-safe restritivo: PARA e pergunta), **nunca** conceder posse nem liberar ação. O `author_node` do `CodeChanged` é server-side (ADR 0026), não do payload.

## Consequências

- **Positivas:** o claim deixa de ser "reservei um ID abstrato" e passa a "reservei estes arquivos" — a coordenação por pertencimento (spec 36) ganha o substrato que faltava. Reaproveita `PlanClaimed.item` (evento intocado).
- **Porta que se ABRE (registrada):** estende o significado de claim (de ID para ID+paths). É aditivo e fail-safe (só nega/alerta, nunca concede) — mas é mudança de modelo, daí o ADR como extensão explícita do 0032.
- **Custo / superfície:** 1 campo no struct + parser/render + a interseção no handler. Migração zero (plano antigo → `paths: []`, guard nunca morde).
- **A provar (red-team):** (a) round-trip exato de plano sem `paths`; (b) nó com claim que intersecta → PARA/atenção; sem interseção → estado inalterado; (c) `paths` forjado num envelope NÃO concede posse nem autoridade (posse segue `owner`/`AlreadyOwned`).

## Alternativas rejeitadas

- **Derivar pertencimento só por `owner` do item (sem paths):** grosseiro demais — não sabe QUAIS arquivos um item cobre, então não dá para dizer se um commit do colega toca o que você reservou. Perde a precisão que a spec 36 §2 exige.
- **Campo de paths no `PlanClaimed` (em vez de `PlanItemAttributed`):** `PlanClaimed` liga apenas `item`(id)→`by` — o "o que o item cobre" é propriedade do ITEM, não do ato de claim; duplicá-lo no claim espalharia a verdade por dois eventos. O lugar canônico é `PlanItemAttributed` (junto de `parents`/`acceptance`/`budget`), e a projeção junta os dois por id. Rejeitado.
- **`paths` não-aditivo:** quebraria replay de plano/log anterior (viola inv #4). Rejeitado de saída.
- **Inferir paths por LLM ("este item parece mexer nesses arquivos?"):** não-determinismo num guardrail (viola inv #1 e a doutrina 0007/0019). Os paths são **declarados**, a interseção é de conjuntos. Rejeitado.
