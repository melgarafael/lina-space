# DESPACHO r2-spawn-polish — Bug Finder
**id:** `spawn-polish` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md`

## Achados 6/7/9 do dogfooding de hoje (tabela: `tasks/despachos/achados-dogfooding-sessao.md`)
1. **#6 — `lina spawn` não normaliza o sigil `@`:** o nó nasceu NOMEADO "@Especialista em Licenças" (evento seq 20326 do workspace walking-skeleton). O @ é endereçamento, não nome. Normalize no caminho do spawn (strip de @ inicial ao criar o nó — ache o ponto: router `handle_spawn`/`SpawnRequested.name` → `execute_spawn`/`admit_node` no bridge). Teste: spawn com "@X" e com "X" → nó SEMPRE "X"; endereçar @X depois funciona.
2. **#7 — `TerminalSpawned.cli` recebeu o NOME do nó** em vez do CLI ("@Especialista em Licenças" vs "Claude Code" dos nós ⌘N). Ache o call-site da admissão via spawn que passa o campo errado (bridge `execute_spawn`→`admit_node`) e corrija + teste de paridade com o caminho ⌘N.
3. **#9 (metade restante) — ficha do spawnado cita o cwd:** o 1º prompt do spawn chega a um terminal numa pasta NOVA que não sabe onde está. Garanta que a doutrina/ficha gerada pro spawnado inclua o cwd dele (se a ficha já imprime cwd, valide e aponte; senão adicione 1 linha no gerador da ficha que o admit_node usa).

## Fronteira (sua)
`app/lina-gpui/src/bridge.rs` (dono único de novo) · `crates/lina-core/src/router.rs` (hunks do handle_spawn SÓ se o fix for lá) · testes.
**⛔ NÃO toque:** events.rs/attention.rs/broker.rs (externo!) · main.rs/agent_modal/attention_ui (externo) · bin/lina.rs + lib.rs do bootstrap (externo). Se o fix pedir mudança de EVENTO: pare e registre (campo cli é dado do payload — o fix certo é no call-site).
Entrega: `tasks/epico-f1/.entrega-spawn-polish.md` · Marcador: `.iniciado-spawn-polish`.
