# Prompt de teste ao vivo — F3-1 (Goal-and-Loop) + F3-2 (Loop do Maestro + Tradutor)

> Cole o bloco abaixo num **terminal de IA NOVO** dentro do Lina. É autocontido.

---

Você é um terminal de IA dentro do Lina Space. Acabaram de ser validadas as rodadas **F3-1 (Goal-and-Loop)** — a META vira primitiva (o usuário declara, o sistema interpreta, devolve o entendimento, espera confirmar, decompõe e persegue até o aceite, com **juiz separado do executor**) — e **F3-2 (Loop do Maestro + Tradutor)**. Sua tarefa: **TESTAR ao vivo e reportar PASS/ACHADO por EVIDÊNCIA** — rode e observe, nunca confie em relato.

**LEIA OS SPECS ANTES (obrigatório):**
- Repo: `tasks/epico-f3/onda-f3-1.md`, `tasks/epico-f3/onda-f3-2.md`, `tasks/epico-f3/onda-f3-val.md` (plano e critérios de aceite).
- Vault Obsidian: `52 - SPEC Goal-and-Loop` e `39 - Epico Fase 3 — O Maestro Nativo` (§F3-1/§F3-2). ⚠️ o `lina vault index` está stale p/ a 52/39 — se `lina vault read "52 ..."` falhar, leia pelo path direto em `/Users/rafaelmelgaco/Documents/Obsidian Vault/Ecossistema Labs - Operação/Debriefing Vibe Coding/`.

**PRÉ-CHECK (o binário tem o código novo?):** rode `lina goal status zzz`.
- Voltou o **help geral** → o Lina está com binário antigo: peça pro Rafael **fechar e reabrir o Lina** (ele rebuilda sozinho) e tente de novo.
- Voltou **"nenhuma meta com esse id"** → está no ar, siga.

**TESTE 1 — o ciclo da Goal funciona ao vivo (por evento, não por relato):**
1. `lina goal define "Criar uma página de vendas pro meu curso de confeitaria" --accept "a página abre sem erro"`
   - O `goal_id` é cunhado **server-side** (o comando só devolve "enfileirado"). Para achá-lo, leia o último `GoalDefined` do log:
     `GID=$(grep '"GoalDefined"' "$LINA_HOME/events/log.jsonl" | tail -1 | python3 -c "import sys,json;print(json.loads(sys.stdin.read())['payload']['goal_id'])"); echo "$GID"`
2. `lina goal interpret "$GID" --understanding "Página de 1 seção com as aulas e um botão de compra" --strategy "frontend monta, qa valida no fim" --team "Terminal A,Terminal B" --accept "a página abre sem erro"`
3. `lina goal confirm "$GID"`   ← gate humano
4. `lina plan seed "$GID"`   ← decompõe a meta em itens do plano
5. `lina goal status "$GID"` → DEVE mostrar: **fase "decomposta"**, a meta como foi dita, **"o que entendi"**, os **critérios em pt-br**, e os **itens**. **ZERO jargão** técnico (nada de `ReviewVerdict`/`effort`/`root_cause_id` na superfície).
6. Confirme no log (`lina goal status "$GID" --json` + o event log): `origin` = `@Maestro` (ou `@Tradutor` se houver um no roster), `by` = identidade **server-side**, `proposed_team` registrado com o time que você passou.
7. **REPLAY:** rode `lina goal status "$GID"` de novo → idêntico (o estado vem do **event log**, não da memória).

**TESTE 2 — os freios (o coração: turn-budget, escala de effort, juiz≠executor):** prova repetível no caminho real.
```
cd "/Users/rafaelmelgaco/einstein workspace/lina-space"
cargo test -p lina-core --test goal_loop_aceite_f3val
```
→ deve dar **5 passed; 0 failed** (exit 0). Cobre: 3 `Fail` → `GoalEscalated{turn_budget_exhausted}` e nada depois; caminho feliz → exatamente 1 `GoalAchieved`; todo veredito com `reviewer != target` (forja inexpressável); re-spawn com effort **estritamente maior** por `Ord`; breaker sticky suprime re-spawn idêntico.

**TESTE 3 — o card na tela (olho humano):** olhe o canvas do Lina. Deve aparecer um **card da meta** no topo-centro, em pt-br, com: a meta como dita, "O que entendi", "Como vou fazer", "Como vou saber que deu certo" (critérios), barra de progresso, e os botões **"Sim, é isso" / "Quero ajustar"**. Valide a olho: (a) **zero jargão** técnico; (b) **identidade da casa** (fonte de destaque ≠ corpo — não cara de site genérico); (c) dá pra **confirmar/ajustar** em 1 toque.

**REPORTE:** para cada teste, **PASS** (com a saída/observação que prova) ou **ACHADO** (o que falhou + onde). Se um verbo `goal` voltar o help geral, sinalize ANTES de seguir (binário antigo). Termine com **`PRONTO: <resumo dos 3 testes>`** ou **`BLOCKED: <motivo>`**.

> **Gap conhecido (não é regressão — já registrado como story residual):** na fase de **escalada** (depois de 3 tentativas), os botões "Reforçar o time"/"Olhar com você" ainda não fazem exatamente o que prometem. O loop automático e a escala de effort estão 100%. Não precisa testar isso.
