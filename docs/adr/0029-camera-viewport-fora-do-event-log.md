# ADR 0029 — Câmera/viewport FORA do event log; layout de terminal como evento TRANSACIONAL

- **Status:** **Aceito (2026-06-12, F2-0-5).** Nota-ADR-gate da **F2-3-7** (persistência/
  restauração de layout por replay + câmera em snapshot de sessão).
- **Onda/Story:** F2-0 · F2-0-5 (épico `38 - Epico Fase 2` §F2-0)
- **Data:** 2026-06-12
- **Fontes:** pesquisa D3-Canvas-UX achado **A9** + resposta **(d)**
  (`tasks/pesquisa-f2/entrega-d3-canvas-ux.md`), com veredito **CONFIRMADO** na verificação
  cética (`tasks/pesquisa-f2/entrega-v-verificacao.md` §II — jsoncanvas.org/spec/1.0
  re-fetchado, tldraw verbatim, APIs nominais conferidas) · visão nota `22 - Fluxo de Telas`
  ("mesmo zoom pós-crash") · precedente `last_focus` no addendum do ADR 0010.

## Contexto

A F2-3 vai persistir e restaurar o layout do canvas. A pergunta que esta nota fecha: **o que é
FATO do Espaço (entra no event log, invariante #4) e o que é estado de SESSÃO local (fica
fora)?** A pesquisa D3-A9 levantou como 5 organizações independentes resolvem isso — tldraw,
Excalidraw, Figma, Liveblocks e JSON Canvas (Obsidian) — e o resultado é **unânime** (veredito
CONFIRMADO na verificação):

1. **Posição/tamanho/organização dos NÓS** é documento — o que você salvaria num servidor.
2. **Câmera/zoom/seleção do viewport** é sessão — local, por usuário, **fora do documento**.
   JSON Canvas 1.0 persiste `id, type, x, y, width, height, color` e **zero câmera no arquivo**
   (decisão deliberada da spec); tldraw separa `document` de `session` nominalmente; câmera não
   entra nem no undo.
3. **Granularidade de evento = a TRANSAÇÃO de interação**, nunca o frame: tldraw acumula o drag
   até um *mark* e colapsa em 1 step (`markHistoryStoppingPoint`/`squashToMark`, cancelamento
   via `bailToMark` sem poluir o redo); Liveblocks documenta `history.pause()` no pointerdown /
   `resume()` no pointerup. **Ninguém faz event-sourcing por frame de drag.**

## Decisão

### 1. Layout de TERMINAL = evento transacional no log (R5 / invariante #4)

Posição, tamanho e organização de terminal são **fatos do Espaço** — entram no event log e são
re-deriváveis por replay. Contrato dos eventos (aditivos, `serde(default)` como toda a série):

- **`TerminalMovido { terminal_id, x, y }`** — emitido **no FIM do gesto**, com o estado FINAL.
- **`TerminalRedimensionado { terminal_id, cols, rows, w, h }`** — idem: estado final da
  transação (px para o canvas, cols/rows para o PTY — as duas verdades do mesmo gesto).
- **Regra dura: 1 gesto = 1 evento.** Jamais por frame; o arrasto vivo é estado de render, não
  fato. **Gesto cancelado = ZERO evento** (padrão `bailToMark`): o log só registra o que
  aconteceu, não o que quase aconteceu.

### 2. Ordem-z por fractional index (no evento, append-only-friendly)

A ordem de empilhamento viaja como **fractional index** (padrão Excalidraw/Figma, linhagem
rocicorp/fractional-indexing): "trazer para frente" gera um índice novo entre dois existentes —
**1 evento altera 1 terminal**, sem reindexar vizinhos nem emitir eventos colaterais. É o único
esquema de z-order que não briga com um log append-only.

### 3. Câmera/zoom do viewport = estado de SESSÃO local, FORA da stream do Espaço

Pan, zoom e enquadramento do viewport **não entram no event log do Espaço**. Para honrar o
"mesmo zoom pós-crash" da visão (nota `22`), a câmera é persistida em **snapshot de sessão
local** (ex.: SQLite local, fora da stream de eventos do Espaço) — a garantia de recuperação se
mantém **sem poluir o log compartilhado** com ruído por-usuário. A própria D3 verificou que as
duas coisas são compatíveis: persistir ≠ event-sourcear.

## Limite explícito (a porta que NÃO estamos fechando)

Se foco/câmera um dia precisar virar **fato auditável** (ex.: telemetria de atenção,
observabilidade da Fase 3), o caminho é **evento aditivo NOVO em story própria** — nunca a
promoção do snapshot de sessão a autoridade. É o espelho exato do precedente `last_focus`
(addendum do ADR 0010): "perder o snapshot perde no máximo o enquadramento, nunca um fato de
Espaço". Esta nota decide onde a câmera vive HOJE; não proíbe que um evento de câmera exista
amanhã — exige só que ele nasça como evento, com ADR/story própria.

## Consequências

- **F2-3-7** implementa replay de layout consumindo `TerminalMovido`/`TerminalRedimensionado` e
  restaura a câmera do snapshot de sessão — zero re-decisão no meio da onda.
- Undo espacial (se vier na F2+): 1 transação = 1 step de undo, alinhado por construção.
- Critério auditável: nenhum evento de câmera/zoom aparece na stream do Espaço; nenhum
  `TerminalMovido` é emitido durante um drag em curso (só no mouse-up).

## Alternativas rejeitadas

- **Câmera no event log** — sem precedente shipped (0 de 5 organizações); poluiria o log com
  eventos por-usuário de alta frequência sem valor de replay para o Espaço.
- **Eventos de drag por frame** — ninguém faz; transformaria o log em trace de mouse e
  inviabilizaria replay/auditoria legível (a D3 tentou refutar e não achou um único caso).
- **Z-order por inteiros reindexados** — cada "trazer para frente" emitiria N eventos
  colaterais; fractional index resolve com 1.
