# Prompt de teste — Rodada F3-1 (Goal-and-Loop)

> Cole o bloco abaixo numa sessão NOVA (terminal de IA no Lina rebuildado). Ele é autocontido.

---

Você é um terminal de IA no Lina Space. Acaba de ser implementada e commitada a rodada **F3-1 (Goal-and-Loop)** — a primeira vez que o Lina tem uma **META como primitiva**: o usuário declara uma meta, o sistema interpreta, devolve o entendimento, espera confirmação, decompõe em itens e persegue até o critério de aceite, com um **juiz separado do executor**. Sua tarefa é **TESTAR isso ao vivo e reportar PASS/achados** — não confie em relato, rode e observe.

**Contexto (onde está o quê):** verbos `lina goal define/interpret/confirm/status` + `lina plan add/seed`; um card da Goal no canvas; o loop `ReviewVerdict` (juiz≠executor) + turn-budget + escala de effort. Spec-fonte: vault → `52 - SPEC Goal-and-Loop`. Plano e status: `tasks/epico-f3/onda-f3-1.md` (repo). Provado por 480 testes em `lina-core`.

**TESTE 1 — o ciclo da Goal funciona ao vivo (por dados):**
1. Confirme que o app foi rebuildado com o código novo: rode `lina goal status zzz`. Se voltar o **help geral**, o `lina` em uso é o antigo → peça pro Rafael **fechar e reabrir o Lina.app** (ele rebuilda sozinho) e tente de novo. Se voltar "nenhuma meta com esse id", está no ar.
2. Semeie uma meta e percorra o ciclo (anote o `goal_id` que o `define` devolve/loga):
   - `lina goal define "Deixar a landing de captura pronta pra subir" --accept "a pagina abre sem erro"`
   - `lina goal interpret <goal_id> --understanding "LP de 1 secao com formulario de email" --strategy "frontend monta, qa valida" --accept "a pagina abre sem erro"`
   - `lina goal confirm <goal_id>`   ← este é o **gate humano** (em autonomia assistido pode propor→confirmar; siga o que o `lina` indicar)
   - `lina plan seed <goal_id>`   ← decompõe em itens do plano
3. Leia a projeção: `lina goal status <goal_id>` → deve mostrar **fase** (confirmada/decomposta), a meta como foi dita, "o que entendi", os **critérios em pt-br**, e os itens. Zero erro/jargão técnico.
4. **Replay:** o estado vem do event log (não da memória) — confirme que `lina goal status` reconstrói o mesmo após qualquer reabertura.

**TESTE 2 — o card na tela (precisa do olho do Rafael):**
- Olhe o **canvas do Lina**: deve aparecer um **card** da Goal "Deixar a landing de captura pronta pra subir" com a meta, "o que entendi", os critérios em linguagem clara e uma barra de progresso.
- Valide a olho: (a) **zero jargão** (nada de `ReviewVerdict`/`effort`/`goal_id`/`root_cause_id` na tela); (b) **identidade da casa** (fonte de destaque ≠ corpo; tokens — não cara de site genérico/Inter por inércia); (c) dá pra **confirmar/ajustar**.

**REPORTE:** para cada teste, **PASS** (com a saída/observação) ou **ACHADO** (o que falhou + onde). Se um verbo `goal` voltar o help geral, o binário não tem o código novo — sinalize antes de seguir.
