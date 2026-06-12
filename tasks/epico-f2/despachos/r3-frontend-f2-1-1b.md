# Despacho r3 — Terminal C (FRONTEND): F2-1-1b — fonte do grid ATÔMICA (embedding + consumo + célula)

## CONTEXTO
Story atômica criada pela decisão r1 do Maestro (`tasks/epico-f2/decisoes-maestro-r1.md` §1) a partir do SEU aviso: trocar a fonte do grid sem embarcar os arquivos e sem re-derivar a célula = fallback silencioso + grid desalinhado (viola "a tela nunca mente"). Terreno que você mesmo preparou: pesos/caminhos OFL no doc do `theme.rs`, contrato `grid=13` travado por teste, aviso de `CELL_W=7.84`/`CELL_H=17.0` (`bridge.rs:2986`, medidos para Menlo 13px).

## FUNÇÃO
Frontend — dono da story inteira (é atômica: ou aterrissa completa, ou não aterrissa).

## DIRECIONAMENTO
1. **Embedding:** embarcar os estáticos OFL no binário/.app — JetBrains Mono (grid; pesos que o grid usa) + IBM Plex Sans + Fraunces (UI/momentos — já embarca junto, a F2-2 consome). Caminho gpui: asset source / `embedded_fonts` no boot (o Zed embarca por assets — modele). Licenças OFL acompanham os arquivos (arquivo LICENSE por família, padrão OFL).
2. **Célula:** re-derivar `CELL_W`/`CELL_H` para JetBrains Mono 13px — preferência: MEDIR via text system no boot (à prova de troca futura de fonte/tamanho) com fallback constante; se medir no boot for invasivo demais no pin, constantes re-medidas + teste que valida contra o text system. O grid não pode desalinhar: cols×rows e hit-testing têm que bater (teste que prova).
3. **Consumo:** os 3 diffs Menlo→token (`main.rs:416`, `attention_ui.rs:774`, `agent_modal.rs:2253`) + `FONT_PX` derivado do token (3 usos). 
4. **Fronteiras desta story (excepcionalmente largas por ser atômica):** `theme.rs`, `bridge.rs` (célula), `attention_ui.rs`, `agent_modal.rs`, `assets/` (fontes) são SEUS. `main.rs` segue COSTURA minha — pedido com diff exato como sempre. Ninguém mais está em código nesta janela (D/E em prontidão; A em docs/adr/) — confirmei o roster antes deste despacho.
5. **Catraca:** os 3 diffs e o FONT_PX derivado DEVEM baixar contagens do snapshot — rode `LINA_RATCHET_UPDATE=1` e commite o snapshot apertado JUNTO (na minha validação eu confiro que só desceu).
6. **Validação:** suíte + clippy -D warnings + fmt nos seus arquivos; na tela = eu cuido (repack + roteiro curto para o fundador: grid nítido, alinhamento, fallback ausente).
7. Se a medição via text system no pin travar em API inacessível: pare nessa metade, entregue com constantes+teste e registre a limitação — não gaste o orçamento da story lutando contra o pin.

## OBJETIVO
O terminal do Lina passa a renderizar na fonte da identidade (ADR 0019 §7 cumprido de verdade), com célula correta, fontes embarcadas (zero dependência da máquina do usuário) e catraca apertada.

## RESULTADO ESPERADO
Fatia completa + pedido de costura do main.rs + snapshot da catraca apertado. Reporte começo/fim com `--intent status`. Última linha `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
