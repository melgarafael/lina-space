# Pedido de costura — r1 F2-1-1 (Terminal C → Maestro)

> Vocabulário F2-1-1/2/3 entregue em `app/lina-gpui/src/theme.rs` (testes 15/15 + clippy -D warnings
> limpos; só theme.rs tocado). Consumo nos arquivos de costura é decisão/aplicação SUA. Diff mínimo
> + avisos abaixo.

## Diff mínimo (3 ocorrências de "Menlo" → token)

```text
main.rs:416          .font_family("Menlo")
                   → .font_family(theme::active().typography.family.mono)

attention_ui.rs:774  .font_family("Menlo")
                   → .font_family(theme::active().typography.family.mono)

agent_modal.rs:2253  .font_family("Menlo")
                   → .font_family(theme::active().typography.family.mono)
```

Opcional na mesma passada: `main.rs:91 const FONT_PX: f32 = 13.0` pode derivar do token
(`f32::from(theme::active().typography.size.grid)`) — o teste `typography_vocabulary_integrity`
trava `grid=13`, então os dois caminhos são equivalentes hoje; derivar fecha a porta de drift.

## ⚠️ Avisos que importam (não aplicar às cegas)

1. **Métricas de célula:** `bridge.rs:2986` (`CELL_W=7.84`/`CELL_H=17.0`) foi MEDIDO para
   Menlo 13px. Trocar a família do grid sem re-derivar essas métricas desalinha cols×rows e
   hit-testing. Recomendo: aplicar o diff do grid JUNTO com a re-derivação para JetBrains Mono
   (ou medir via text system no boot).
2. **JetBrains Mono ainda não é embarcada** (empacotamento é story posterior; registro de pesos
   OFL no doc do módulo). Se a fonte não estiver instalada na máquina, o font-kit cai em fallback
   silencioso. Gate sugerido: aplicar o diff quando o embedding entrar, OU validar presença local
   na sessão de teste.
3. **Fiação do reduce-motion pronta:** `theme::set_reduce_motion(bool)` é o ponto único de
   mutação (sobrevive a `apply()` — teste cobre). O observer de sistema/Ajustes é costura sua;
   os `#[allow(dead_code)]` em `set_reduce_motion`/`MotionTokens::effective` saem ao fiar.

## Pergunta (1, não bloqueia)

Deixei no `tema.json` as seções novas `tipografia` (famílias, documentais/curadas) e `movimento`
(`reduzir`) — preferências reais do usuário. **Espaçamento/cantos NÃO entraram** no JSON: são
estrutura, não preferência; o desenho de override por token é a F2-1-4. Se você quiser as escalas
no arquivo mesmo assim, é 1 edit — me diga.

## Estado da árvore que observei (não é meu)

`cargo test -p lina-gpui` completo: 410 ok / 1 falha em `prof::tests::report_line_carries_gate_metrics`
(`excess_time=-0.00%` vs `0.00%` — zero negativo na formatação). `prof.rs` está modificado na árvore
(trabalho em curso do worker da sonda F2-0-1) — falha é do in-flight dele, meu módulo não toca prof.
