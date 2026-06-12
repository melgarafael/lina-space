# Notas de abertura da r6 (consolidadas pelo Maestro, 2026-06-12 fim do dia)

Itens herdados com dono/lente já combinados:
1. **Toolbar contextual (F2-2-5)** — spec selada `47d083c`; constrói: Frontend (registry palette.rs = dev) · strings: @Redator (verbos já mapeados).
2. **Integração toast_stack no main.rs** — spec do E (campo+tick+render visible()+ArchiveToast migra com Desfazer 8s; dropar announce() duplicado).
3. **Migração chips/dots→Badge** — **P0 DO ARQUITETO-REVISOR: o call-site do card `needs_human` (main.rs:3774-3780) PRIMEIRO** — é o caso Assertive por excelência (gate humano) e hoje só fala ao focar (gap exato do 0028); a r5 reforçou o visual e ampliou a assimetria visual-vs-a11y justo nele. Badge::needs_you() resolve.
4. **Guard de conformidade 0028 (hardening, achado 1 do Arquiteto-revisor)** — teste que FALHE se componente comunicar estado fora do a11y_live (espírito da catraca/lint de cor; fecha a fronteira para componentes futuros).
5. **Strings dos toasts/badges** — @Redator (registro anti-alarme do vocabulário F2-2).
Revisões combinadas: D = contrato a11y empírico (alerta dele: migração preserva value+live/glifo+texto; toast_stack mantém teto-3 e id único) · @Arquiteto = revisão CEGA de completude (nenhum dot/chip de estado legado sem live()) · Terminal A = autor de specs · Maestro = validação de fora.
Vereditos de base: F2-2-2 a11y PASS prova-dupla (D) · conformidade 0028 PASS 90 (Arquiteto-revisor).

## Régua de completude da migração (baseline do @Arquiteto, capturada ANTES da migração — escopo OFICIAL da story)
1. **attention_ui.rs** — O MAIOR E MAIS CRÍTICO: render_toast (l.508, Role::Status CRU na l.531 — o toast de permissão/custódia mudo sem foco; o som 1x/30s não DIZ o que pede) + render_badge/sino (l.454, contagem e escalação sem auto-anúncio).
2. **Card do nó** — P0 já nomeado: status_dot/card_border + node_label→aria_label, derivados de BadgeKind/aggregate_badge.
3. **Chips do título** (autonomy/kit_missing/cwd_shared ~3667/3712/3728) — verificar se comunicam MUDANÇA de estado.
CRITÉRIO: migrado = compõe LiveRegion com cortesia certa (needs_you/escalação→Assertive); ZERO Role::Status cru; ZERO só-aria_label para estado que muda sem foco.
