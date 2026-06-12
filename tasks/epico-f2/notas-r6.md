# Notas de abertura da r6 (consolidadas pelo Maestro, 2026-06-12 fim do dia)

Itens herdados com dono/lente já combinados:
1. **Toolbar contextual (F2-2-5)** — spec selada `47d083c`; constrói: Frontend (registry palette.rs = dev) · strings: @Redator (verbos já mapeados).
2. **Integração toast_stack no main.rs** — spec do E (campo+tick+render visible()+ArchiveToast migra com Desfazer 8s; dropar announce() duplicado).
3. **Migração chips/dots→Badge** — **P0 DO ARQUITETO-REVISOR: o call-site do card `needs_human` (main.rs:3774-3780) PRIMEIRO** — é o caso Assertive por excelência (gate humano) e hoje só fala ao focar (gap exato do 0028); a r5 reforçou o visual e ampliou a assimetria visual-vs-a11y justo nele. Badge::needs_you() resolve.
4. **Guard de conformidade 0028 (hardening, achado 1 do Arquiteto-revisor)** — teste que FALHE se componente comunicar estado fora do a11y_live (espírito da catraca/lint de cor; fecha a fronteira para componentes futuros).
5. **Strings dos toasts/badges** — @Redator (registro anti-alarme do vocabulário F2-2).
Revisões combinadas: D = contrato a11y empírico (alerta dele: migração preserva value+live/glifo+texto; toast_stack mantém teto-3 e id único) · @Arquiteto = revisão CEGA de completude (nenhum dot/chip de estado legado sem live()) · Terminal A = autor de specs · Maestro = validação de fora.
Vereditos de base: F2-2-2 a11y PASS prova-dupla (D) · conformidade 0028 PASS 90 (Arquiteto-revisor).
