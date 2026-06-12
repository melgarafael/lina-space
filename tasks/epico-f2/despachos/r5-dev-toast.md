# Despacho r5 — Terminal E: F2-2-2 — toast/badge/progresso sobre o Element de live-region

## CONTEXTO
Rodada 5 (épico `38` §F2-2). ADR 0028 SELADO: todo componente que comunica estado nasce compondo com o custom Element de live-region — retrofit PROIBIDO. O spike existe (`a11y_live.rs`, F1-2-7: custom Element SEM patch no pin; descoberta: adapter macOS anuncia value(), não label). O Arquiteto fará revisão de conformidade 0028; o QA (D) fará verificação de a11y na sequência.

## FUNÇÃO
Developer — fronteira: `a11y_live.rs` (promover de spike a componente) + `src/ui/toast.rs` + `src/ui/badge.rs` (novos). NÃO toque main.rs (C é dono único nesta rodada — a integração você ESPECIFICA no reporte e eu aplico quando ele devolver o arquivo).

## DIRECIONAMENTO
1. **Promova o spike a Element de produção:** API limpa (announce(text, politeness)), o truque do value() documentado, teste do caminho.
2. **Toast** (ui/): consome tokens (Panel/MotionTokens do catálogo; surface quente; flat); auto-dismiss com duração de MotionTokens (e SEM auto-dismiss quando reduce-motion? NÃO — reduce-motion corta animação, não duração; o que corta duração é política própria: defina e documente); empilhamento máx 3; AÇÃO opcional de 1 clique (padrão "Desfazer" do toast de arquivar da F1). TODO anúncio passa pelo Element (politeness=polite).
3. **Badge de estado** (ui/): texto+ícone+cor SEMPRE (WCAG 1.4.1; cores semânticas dos tokens); mudança de estado anuncia via Element (assertive só para "precisa de você").
4. Os consumidores reais (attention_ui, cards) NÃO migram nesta story (são fronteira do C nesta rodada) — entregue os componentes + a spec de migração por call-site.
5. Honestidade do ADR 0028: nenhuma copy/doc afirma "conforme ARIA" — o selo de conformidade é o smoke de VoiceOver na sessão de tela (D prepara).
6. Validação: suíte + catraca (componentes novos = dívida zero) + clippy + fmt nos seus. Não commite. Reporte E continue.

## OBJETIVO
O Lina ganha o vocabulário de AVISO da identidade — acessível por construção, nunca por remendo.

## RESULTADO ESPERADO
Element de produção + toast + badge + spec de integração/migração. PRONTO:/BLOCKED:.

## Tentativas anteriores
Nenhuma.
