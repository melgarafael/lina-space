# Roteiro de validação na tela — SPIKE F1-2-7 (live-region via custom Element)

> **Objetivo (critério 2 da F1-2-7):** gravar o **VoiceOver anunciando uma frase SEM o foco
> estar no elemento**. Quem executa: Maestro/fundador (o gpui não roda headless).
> O que está em jogo: se este roteiro passar, o caminho (a) do ADR 0028 (custom `Element`,
> sem patch no fork) vira a decisão; se falhar, a evidência aqui coletada realimenta o ADR.

## Por que o teste é assim (30 segundos de contexto)

- O gpui **só monta a árvore de acessibilidade quando um leitor de tela está ativo**
  (`a11y.is_active()`). Ligar o VoiceOver ANTES de abrir o app garante a árvore desde o 1º frame.
- O adapter do macOS anuncia em DOIS momentos (lido na fonte, `accesskit_macos-0.26.1/src/event.rs`):
  1. quando o nó live **nasce** na árvore já com texto (`node_added`) — é o caso do banner
     "resposta pronta", que só entra na cena quando há mensagem;
  2. quando o **texto muda** num nó live já presente (`node_updated`).
  O roteiro cobre os dois (cenários A e B).
- O VoiceOver fala o **value** do nó (a mensagem crua, sem o emoji 🔊 — ele é só visual).

## Pré-requisitos

- Mac com VoiceOver disponível (`Cmd+F5` liga/desliga).
- Branch com o spike (`app/lina-gpui/src/a11y_live.rs` presente).
- Gravação de tela **com áudio** pronta: `Cmd+Shift+5` → gravar tela inteira com microfone
  ligado (o microfone capta a voz do VoiceOver pelas caixas) — ou QuickTime > Nova Gravação de Tela.

## Passo-a-passo

1. **Ligue o VoiceOver primeiro**: `Cmd+F5`. Espere ele falar a janela atual.
2. (Opcional, diagnóstico) Abra o app com log: `cd app/lina-gpui && RUST_LOG=info cargo run`.
   No terminal de origem deve aparecer **"Accessibility activated"** — é o gpui confirmando
   que a árvore a11y está sendo construída. Sem essa linha, nada vai ser anunciado.
3. **Comece a gravar** a tela (com áudio).
4. No app, crie/use um agente e mande um prompt curto (ex.: "diga oi e nada mais").
5. **Mova o cursor do VoiceOver para LONGE do banner** — ex.: navegue com `VO+setas` até o
   canvas ou outro painel. O ponto da prova é o anúncio chegar **sem foco** no elemento.
6. Espere a resposta terminar. O banner **"🔊 <nome>: resposta pronta"** aparece no canto
   inferior direito.

### ✅ Resultado esperado (cenário A — nó nasce com texto)

No instante em que o banner aparece, o VoiceOver **fala** "<nome>: resposta pronta"
(em prioridade cortês — ele termina a frase que estiver falando antes), **sem** o foco
estar no banner e **sem** falar "alto-falante"/emoji.

7. **Cenário B (texto muda em nó vivo):** com o banner ainda na tela, mande um prompt para
   OUTRO agente e espere terminar. O texto do banner muda para o novo nome →
   o VoiceOver deve anunciar a nova frase.
8. Pare a gravação e guarde o arquivo (anexar à entrega da story/ADR).

## Se NÃO falar nada (troubleshooting, em ordem)

1. O VoiceOver estava ligado **antes** de abrir o app? Se ligou depois: feche e reabra o app
   (a ativação a quente existe via callback, mas o "antes" elimina uma variável).
2. A linha "Accessibility activated" apareceu no log (passo 2)? Se não: o problema é a
   ativação do adapter, não a live-region — registre isso (muda o diagnóstico).
3. Verbosidade do VoiceOver: Utilitário do VoiceOver (`VO+F8`) → Verbosidade → Anúncios —
   confira que anúncios não estão suprimidos.
4. Foque o banner com `VO+setas`: ele deve ler "<nome>: resposta pronta" (role Status +
   label). Se ler ao focar mas não anunciar sozinho, a live-region não está sendo honrada —
   **isso é evidência para o ADR** (registrar: macOS versão, VoiceOver falou X ao focar).

## O que reportar

- **PASS**: gravação + 1 linha ("anunciou sem foco nos cenários A e B").
- **FAIL**: o passo exato que divergiu + o que o VoiceOver fez/não fez + log do passo 2 —
  essa evidência muda a recomendação do ADR 0028 para o caminho (b) (patch no pin) e é
  exatamente o valor do spike.
