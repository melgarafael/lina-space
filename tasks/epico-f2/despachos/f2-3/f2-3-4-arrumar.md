# DESPACHO — F2-3-4 · "Arrumar" como verbo + preset lado-a-lado · fatia `f2-3-4`

**CONTEXTO** · Leia `tasks/epico-f2/despachos/f2-3/_contexto.md` + `tasks/despachos/_regras-comuns.md`.
Posição do card em `NodeView.x/y` (mundo); `CARD_W=680, CARD_H=500, CARD_GAP=30` (bridge.rs ~140). A
inicialização HOJE usa uma grade 4-colunas (`grid_slot` em bridge.rs ~1560). O mover (F2-3-1) já existe
(`canvas_drag.rs`), com ordem-z por `fractional_between`. **ATENÇÃO: o bin do app está temporariamente
QUEBRADO** por outra fatia em voo (resize) — valide seu módulo EM ISOLAMENTO (harness `rustc` standalone),
não dependa do `cargo test` do bin linkar agora.

**FUNÇÃO** · Você é o dono da lógica pura de ORGANIZAÇÃO automática do canvas ("arrumar a bagunça").

**DIRECIONAMENTO** · Fronteira (SÓ estes): **crie `app/lina-gpui/src/canvas_layout.rs`** (módulo puro) +
1 linha de registro `#[path] pub mod layout;` em `canvas.rs` (precedente de `focus`/`zoom`). NÃO toque
`main.rs`/`bridge.rs`. Entregue funções PURAS testáveis:
- `arrange_grid(n: usize, card_w, card_h, gap, origin) -> Vec<(f32,f32)>`: posições de uma grade limpa
  (colunas calculadas para ficar quadrada-ish, ou fixe colunas e justifique), começando em `origin`,
  com `gap` canônico. Determinística (mesma entrada = mesma saída).
- `arrange_side_by_side(n, card_w, card_h, gap, origin) -> Vec<(f32,f32)>`: preset "lado a lado" (1
  linha horizontal). Invariantes **niri** (do épico): o novo nasce PREVISÍVEL (ordem estável); **nada se
  redimensiona sozinho** (só reposiciona — NÃO mexe em tamanho). 
- A ordem dos cards (qual vai em qual slot) deve ser ESTÁVEL e documentada (ex.: ordem-z atual, ou ordem
  de criação — escolha e justifique; o leigo não pode ver os cards embaralharem a cada "arrumar").
Testes não-vacuosos: N cards → N posições sem sobreposição (cada par respeita gap); grade é
determinística; side-by-side é 1 linha; origem respeitada; n=0 → vazio, n=1 → [origin].

**OBJETIVO** · O fundador clica "Arrumar" (e acha na paleta ⌘K) e a bagunça vira uma grade limpa, ou
"lado a lado" — previsível, sem nada redimensionar sozinho e sem os cards embaralharem.

**RESULTADO ESPERADO** · `tasks/epico-f2/despachos/f2-3/.entrega-f2-3-4.md`: arquivo:linha do
`canvas_layout.rs`, evidência (testes em isolamento — nº + comandos exit DIRETO), **diff de costura
main.rs** (botão/atalho "Arrumar" + entrada na paleta que aplica `arrange_*` emitindo `NodeMoved` por
card movido — reusa o `append_to_workspace`/`NodeMoved` da costura do mover), achados. Última linha
`PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
