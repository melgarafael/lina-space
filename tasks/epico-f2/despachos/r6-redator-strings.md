# Despacho r6 — @Redator (WRITER): strings da toolbar + copy de toasts/badges

## CONTEXTO
Rodada 6. A toolbar (spec tasks/epico-f2/spec-f2-2-5-toolbar.md) nasce com 4 verbos que você já mapeou (Atender/Editar/Centralizar/Encerrar) — confirme-os ou melhore (registro anti-alarme do seu vocabulario-f2-2.md; "Atender" NAVEGA até quem pede, nunca aprova — o rótulo não pode prometer aprovação). Os toasts/badges entraram na r5 com copy provisória de dev.

## FUNÇÃO
Writer — fronteira: tasks/epico-f2/regua/strings-r6.md (novo; os devs consomem de lá — você não edita .rs).

## DIRECIONAMENTO
1. **Toolbar**: os 4 rótulos finais + tooltip curto de cada (1 linha leiga) + o aria_label se diferir do rótulo.
2. **Toasts**: revise as mensagens existentes (grep "Toast::new\|with_action" em src/ — liste e proponha); padrão: fato + próximo passo opcional, nunca alarme.
3. **Badges de estado do card**: as palavras finais dos estados (trabalhando/pronto/precisa de você/encerrado/chegando — alinhe com o vocabulario-f2-2 e a decisão OP-1 das cores).
4. Cada string: proposta + porquê em 1 linha quando a escolha não for óbvia.
5. Reporte E continue; PRONTO:/BLOCKED:.

## OBJETIVO / RESULTADO ESPERADO
A rodada fala a língua do leigo desde o primeiro pixel. strings-r6.md pronto para consumo. 

## Tentativas anteriores
Nenhuma.
