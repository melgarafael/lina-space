# Despacho r4 — Terminal C (FRONTEND): F2-2-1 — catálogo de componentes núcleo

## CONTEXTO
Onda F2-2 aberta (gate F2-1 validado na tela pelo fundador). Épico vault `38` §F2-2. Tudo que você precisa existe: tokens 4 famílias (`theme.rs`), decisão de território (fusão T1+temperatura-T3 — §VIII do épico), ADR 0028 selado (live-region), catraca armada. A pesquisa D2(c) mapeou o caminho: consolidar os componentes ESPALHADOS no shell num módulo próprio consumindo tokens — padrão builder+RenderOnce (modelado no Zed por DESCRIÇÃO, nunca código — GPL).

## FUNÇÃO
Frontend — dono do catálogo. Esta é a story-base da onda (as outras 5 consomem).

## DIRECIONAMENTO
1. **Módulo novo `app/lina-gpui/src/ui/`** (ou `ui.rs` com submódulos — sua chamada): núcleo v1 = **botão · painel/card · input · modal** (toast/badge é a F2-2-2, SEPARADA — nasce sobre o Element de live-region; não a faça aqui).
2. **Consolidar, não criar do zero:** inventarie as instâncias existentes (agent_modal, sidebar, persistence_ui, attention_ui têm botões/cards/inputs repetidos); cada componente novo SUBSTITUI todas as instâncias do padrão que consolida — critério do épico: zero duplicação viva do padrão consolidado. A catraca te protege: substituições devem BAIXAR contagens de px() (rode LINA_RATCHET_UPDATE=1 e commite o snapshot apertado junto).
3. **Identidade da fusão aplicada nos componentes:** superfícies quentes (tokens), flat honesto (zero gradiente/sombra decorativa), cor semântica acoplada (um botão de ação destrutiva usa o vermelho semântico etc.), tipografia Plex/Fraunces dos tokens, motion dos MotionTokens (nunca animar input frequente).
4. **Builder+RenderOnce** com API enxuta; cada componente com teste (estados/variantes); a11y: foco visível (FocusTokens), alvo ≥24px (régua camada e), Name/Role na árvore.
5. **Fronteira:** `src/ui/` (novo) + edição dos call-sites que você consolidar (declare a lista no reporte — arquivos que outros workers não estão tocando nesta rodada: confirmado, só você está em código de UI; E está em palette.rs/sidebar.rs — NÃO consolide componentes DESSES 2 arquivos nesta rodada para evitar colisão; deixe-os para a r5). `main.rs` segue costura minha.
6. Validação: suíte + catraca + clippy -D warnings + fmt nos seus arquivos. Não commite.

## OBJETIVO
A F2-2 inteira (identidade na tela, paleta, toolbar) constrói SOBRE componentes com a cara da fusão — e a dívida de magic numbers começa a CAIR de verdade.

## RESULTADO ESPERADO
Módulo ui/ + call-sites consolidados + snapshot da catraca apertado + lista do que consolidou. Reporte começo/fim com `--intent status`; "reporte E continue no mesmo turno". Última linha `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
