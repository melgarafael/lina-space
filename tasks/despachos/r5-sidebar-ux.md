# DESPACHO r5-sidebar-ux — Frontend (Terminal C)
**id:** `sidebar-ux` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Bugs 1, 2 e 4 da tela do fundador (2026-06-11 ~23h, uso real do rail M8). Rodada r5 (fix-sidebar). **Execute APÓS fechar o F1-4-6** (sua fila serial garante zero conflito).

## CONTEXTO
- `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"` (HEAD `be9a45b`). Você é o dono de `sidebar.rs`/`main.rs` na rodada.
- **BUG 1 — sem caminho para minimizar:** "⌘O deveria expandir E minimizar; deveria ter caminho com clique/mouse também." Hoje ⌘O só abre (`main.rs:707` → `open_create_space_modal`?? — confira o binding real do rail; o fundador descobriu que só o ESC fecha). Entregar: ⌘O = toggle; affordance de mouse para abrir/recolher (chevron/botão no próprio rail + área de borda clicável); estado do rail persistido entre sessões se barato (registro no log do Espaço ativo — opcional, não bloqueia).
- **BUG 2 — "não tem como apagar um workspace":** o ARQUIVAR existe por trás (`archive_workspace main.rs:1360` + seu toast com Desfazer da fatia 1) mas **sem entry point visível/compreensível**. Entregar: ação VISÍVEL por linha do rail (no hover/foco: botão ou menu "⋯" com rótulo leigo — ex.: "Arquivar" com subtexto "some da lista; nada é apagado do disco") + alcançável por teclado. **NÃO implemente apagar-do-disco**: destruição real de dados é ação custodiada com gate humano — registre como decisão pendente do Maestro/fundador na entrega (o arquivar com Desfazer cobre o caso de uso "sumir da lista").
- **BUG 4 — rail aberto ROUBA o teclado (o mais incapacitante):** "não consigo digitar nos terminais com o sidebar aberto, mesmo focando — digita na barra de busca; ESC fecha o sidebar, mas não pode ser assim." Causa provável: a busca do rail captura input global enquanto o rail está aberto (confira a precedência de teclado em `main.rs` — padrão M6/M9 de `stop_propagation`). Entregar: (a) rail aberto NÃO captura digitação — o terminal focado continua recebendo; (b) a busca só captura quando explicitamente focada (clique ou atalho próprio, ex.: ⌘F com o rail aberto / `/`); (c) `Esc` fecha o rail SÓ quando o foco está nele (busca/linhas) — nunca rouba o Esc do terminal; (d) clicar fora do rail também fecha (ou pelo menos o chevron do BUG 1 sempre visível); (e) abrir o rail NÃO move o foco do terminal por default.
- Referências: spec do rail `tasks/epico-f1/spec-m8-m9-fiacao.md` §1 (gramática das linhas) e §4 (precedência de teclado); seu próprio padrão de `archive_toast_key` (Tab+Enter/⌘Z) para consistência; tokens do theme; live-region para anunciar abrir/recolher (consistência com sua fatia 1).

## FUNÇÃO
Dono do shell gpui na r5 — os 3 bugs são todos seus (mesmos arquivos, fila serial).

## DIRECIONAMENTO
- Fronteira: `app/lina-gpui/src/{sidebar.rs, main.rs}` (+ `a11y.rs` se o announce exigir). **NÃO toque `runtime.rs`** (dono na r5: Core A2A, investigando a lentidão). Se o Core A2A pedir um botão "Descarregar" por linha do rail (costura da frente dele), ADICIONE o callback injetado no mesmo padrão dos `on_switch`/`on_archive` — o mecanismo é dele, o render é seu.
- Teclado é o coração do bug 4 — teste headless por combinação: rail aberto + tecla imprimível → vai ao terminal focado; busca focada + imprimível → vai à busca; Esc com foco no rail → fecha; Esc com foco no terminal → vai ao terminal (NUNCA fecha o rail). Testes não-vacuosos (removido o mecanismo, falham).
- Copy leiga pt-br em toda superfície nova; zero jargão ("workspace" na tela é "Espaço").

## OBJETIVO
O fundador não consegue USAR o rail hoje: não fecha por mouse, não acha como tirar um Espaço da lista e o teclado é sequestrado. Os 3 fixes devolvem o básico: abrir/fechar óbvio, ação visível por linha, digitação nunca roubada.

## RESULTADO ESPERADO
`tasks/epico-f1/.entrega-sidebar-ux.md`: arquivo:linha por bug, testes novos (em especial a matriz de teclado), suíte do app inteira + clippy/fmt exit direto, roteiro de 5 passos para a tela do fundador. Marcador `.iniciado-sidebar-ux`. Última linha `PRONTO:`/`BLOCKED:`. Reporte status ao @Terminal A (--intent status).
