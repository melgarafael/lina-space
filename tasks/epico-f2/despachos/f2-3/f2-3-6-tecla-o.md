# DESPACHO — F2-3-6 · Tecla "o": pula ao próximo que PEDE APROVAÇÃO e cicla · fatia `f2-3-6`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
O canvas tem uma fila de atenção (F1): nós com gate de custódia pendente ("precisa de você"). Hoje há
`attention_goto_node(node_name)` (main.rs ~1916) que faz `focus`+`reveal`. O épico F2-3-6 pede a tecla
**"o"** que **cicla** entre os nós que pedem aprovação (foca+revela o próximo a cada toque). Invariante da
onda: **a câmera só anda com gesto** — a tecla "o" É o gesto. `needs_human` por nó é computado no render
(main.rs ~3700: `lock(&self.desk).queue.iter().any(|p| p.requester()==nv.name)`). **ATENÇÃO: valide seu
módulo em ISOLAMENTO** se o bin estiver em fluxo (rustc standalone).

**FUNÇÃO** · Você é o dono da lógica de CICLAGEM da fila de atenção pela tecla "o".

**DIRECIONAMENTO** · Fronteira (SÓ estes): **crie `app/lina-gpui/src/canvas_cycle.rs`** (módulo puro) +
1 linha de registro em `canvas.rs`. NÃO toque `main.rs`/`bridge.rs` (a tecla em `handle_key` + a leitura
da fila/desk são costura do Maestro — entregue diff). Entregue função PURA testável:
- `next_pending(pending: &[NodeId], current: Option<NodeId>) -> Option<NodeId>`: dado os nós que pedem
  aprovação (em ordem estável) e o foco atual, retorna o PRÓXIMO na ordem cíclica (wrap-around). Se
  `current` não está na lista, retorna o primeiro. Lista vazia → `None` (no-op: nada pra ir). Ordem
  estável e documentada (a costura passa a lista já ordenada — ex.: por z ou por ordem da fila).
Testes não-vacuosos: cicla 1→2→3→1 (wrap); current fora da lista → primeiro; lista vazia → None;
lista de 1 → ele mesmo (ou None, decida e justifique — "já estou nele" pode ser no-op).

**OBJETIVO** · O fundador aperta "o" e a vista pula pro próximo terminal que está esperando uma resposta
dele, ciclando por todos — nunca varre um por um nem fica perdido.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-6.md`: arquivo:linha do módulo,
evidência (testes — nº + exit DIRETO), **diff de costura main.rs** (tecla "o" em handle_key: monta a
lista de `pending` da desk/fila, chama `next_pending`, faz `focus`+`reveal` no resultado; respeita o
gate "câmera só com gesto"), achados. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
