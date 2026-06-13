# Backlog focado — F2-2-X: toast_stack geral (adiado da r6, decisão de engenharia do C)

## Por que adiado (não é dívida silenciosa — decisão documentada)
O ArchiveToast está tecido em **8 call-sites** com semântica de ARQUIVAMENTO que o `ui::Toast` genérico (E) não carrega: marcação da linha no sidebar (main.rs:910/4377), set (1716), commit/undo (1726/1742), expired (1760), animating (1808), render (3009/3025/4401). Migrar preservando o ponteiro pending-archive + commit/undo é refactor cuidadoso de 8 sites + gate-de-tela próprio (toasts empilhados + Desfazer) — melhor como peça focada que na cauda da r6.

## Escopo da peça
- `ToastStack` (campo no WorkspaceView) + tick(now_ms) no heartbeat (padrão dashboard) + render dos visible() empilhados.
- ArchiveToast → `Toast::with_action("Desfazer", 8s)` PRESERVANDO o pending-archive root p/ o sidebar (campo mínimo OU via closure da ação).
- DROPAR o announce() manual (o ToastView já anuncia via live_region — sem eco duplo).
- Gate-de-tela: empilhamento (teto 3, FIFO) + Desfazer funcional + id único por toast na árvore a11y (verificação do D já combinada).

## Pré-condições já prontas
- `ui::Toast`/`ToastStack`/`ToastView` (E, r5) com teto-3/tick/expiry testados.
- a11y por construção (live_region Polite) — guard 0028 cobre.

## Quando
Próxima sessão de UI ou junto da rodada de canvas (F2-3) — não bloqueia o gate da F2-2 (que é a rodada de testers na nova cara).
