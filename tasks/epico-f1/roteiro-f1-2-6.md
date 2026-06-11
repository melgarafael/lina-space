# Roteiro de validação na tela — F1-2-6 (canvas navegável por teclado)

> **Objetivo (critério 1 da story):** gravar o **percurso só-teclado** — do boot, criar um
> agente, navegar entre 3+ nós com setas, entrar/sair de um terminal, abrir o Inspetor —
> **zero mouse**. Executor: Maestro/fundador (gpui não roda headless).
>
> ⚠️ **Pré-requisito:** este roteiro só roda DEPOIS da costura em `main.rs` (ver pedido de
> costura na `.entrega-f1-2-6.md` — main.rs é do time externo nesta rodada). A LÓGICA já está
> testada headless (13 testes em `canvas_focus.rs`); a tela valida a fiação + a sensação.

## Modelo mental (30 segundos)

O teclado agora tem DOIS níveis:
- **Nível terminal** (estado inicial — igual a hoje): tudo que você digita vai para o agente
  focado. **Esc sobe ao nível canvas.**
- **Nível canvas**: ⬅️➡️⬆️⬇️ movem o foco entre os cartões (anel de foco acompanha e a câmera
  centraliza o cartão); **Enter** entra no terminal focado; **n** abre Novo Agente; **o** pula
  ao próximo terminal ocupado. Digitar letras soltas aqui NÃO faz nada (não vaza ao agente).

## Percurso para a gravação (zero mouse)

1. **Boot**: abra o app (`cd app/lina-gpui && cargo run`). Comece a gravar (Cmd+Shift+5).
2. **Criar agente** (⌘N): o modal abre; digite o nome; Enter cria. Repita até ter **3+ nós**.
3. **Subir ao canvas**: Esc. (O anel de foco do cartão corrente deve ficar evidente.)
4. **Navegar**: setas ⬅️➡️⬆️⬇️ entre os 3+ nós — confira:
   - a ordem é **estável** (ir e voltar repete o caminho; nada "pula");
   - a câmera **centraliza** o cartão focado a cada passo;
   - o **anel de foco** (token `focus.ring`, o acento do tema) marca exatamente 1 cartão.
5. **Entrar no terminal**: Enter no cartão focado → digite algo (ex.: "diga oi") e mande.
6. **Sair**: Esc → de volta ao canvas; navegue a outro nó com as setas.
7. **Próximo ocupado**: com um agente ainda respondendo, pressione **o** — o foco pula direto
   para ele.
8. **Inspetor**: ⌘K (paleta) → digite "inspetor" → Enter. Zero mouse até aqui.
9. Pare a gravação.

## Reduce-motion (critério 3)

10. Feche o app. Reabra com `LINA_REDUCE_MOTION=1 cargo run`.
11. Esc → navegue com as setas: a câmera deve **SALTAR** ao cartão (sem animação). Sem a
    variável, o movimento anima até o alvo.

## Focus ring nos 2 temas (critério 4)

12. ⌘, (Ajustes) → alterne para o tema claro → repita 2 passos de navegação: o anel precisa
    continuar **visível** (o gate de contraste ≥3:1 nos 2 temas × 8 acentos já é teste de CI:
    `theme::tests::focus_ring_visible_in_both_themes`).

## ⚠️ Caso de arbitragem — Esc dentro do CLI (sentir o trade-off)

13. No nível terminal, com o **Claude Code rodando um turno**, pressione Esc com a intenção de
    INTERROMPER o agente (uso real do CLI). Com a política da fonte ("Esc volta ao canvas"),
    o Esc **sobe ao canvas em vez de interromper**. Se isso doer na prática:
    o gatilho é **1 linha na fiação** — a máquina expõe `ExitToCanvas` como comando nomeado;
    troque para `Shift+Esc` (ou outro) sem tocar a lógica. Registrar o veredito do fundador.

## O que reportar

- **PASS**: gravação + 1 linha por critério (navegação estável ✓ · reduce-motion salta ✓ ·
  ring visível nos 2 temas ✓) + veredito do passo 13.
- **FAIL**: o passo exato + o que aconteceu (e o log do terminal, se houver pânico).
