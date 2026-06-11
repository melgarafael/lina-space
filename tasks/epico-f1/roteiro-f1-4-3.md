# 🎬 ROTEIRO DE TELA — F1-4-3 · Restore de terminais vivos no quit limpo (2026-06-11)

> **O que este roteiro prova com o SEU olho** (o resto — posições/nomes/papéis/scrollback
> re-hidratado/comando de resume/badge honesto — já está provado em teste automático headless,
> ver `.entrega-f1-4-3.md`). Aqui você confirma as duas coisas que só a tela mostra:
> **(A)** o Claude REALMENTE volta lembrando da conversa de ontem; **(B)** o badge fala a verdade.
>
> Antes de começar: use o **`dist/Lina.app` reempacotado com esta fatia** (peça ao Maestro o
> repack — o restore só roda no boot do app, e a FIAÇÃO do boot é a costura que o Maestro liga).

## Pré-requisito
- Conta Claude com limite disponível (senão o agente não responde e o resume não tem o que provar).
- Um Espaço com pelo menos **2 terminais**: um **Claude Code** (motor com resume declarado no
  perfil) e um **Terminal puro** (shell, sem resume). Opcional: um terceiro com outro motor sem resume.

## 1. Crie conversa de verdade (para haver o que retomar)
1. No terminal **Claude Code**, converse de verdade: peça algo memorável, ex.:
   *"guarde este número: o código do projeto é 4731. Confirme que anotou."*
2. **VEJA:** o Claude responde confirmando. Role um pouco a conversa para gerar histórico.
3. No **Terminal puro**, rode alguns comandos (`ls`, `echo oi`) — só para ter scrollback.

## 2. Quit LIMPO e reabra
1. Feche o Lina pelo caminho normal (⌘Q / fechar a janela) — **quit limpo**, não force-kill.
2. Reabra o `Lina.app`.
3. **VEJA:** o Espaço volta com os **mesmos cards, nas mesmas posições, com os mesmos nomes e
   papéis**. O histórico de cada terminal está re-hidratado na tela — **role para cima** e a
   conversa de antes está lá, byte a byte.

## 3. (A) O Claude volta LEMBRANDO — a prova do resume
1. No terminal **Claude Code** restaurado, pergunte SEM dar o contexto de novo:
   *"qual era o código do projeto que te passei?"*
2. **VEJA:** ele responde **4731** — ele retomou a sessão anterior (`claude --resume` aplicado
   no boot, com o verbo vindo do perfil TOML, não do código).
3. *(Se ele NÃO lembrar — disser que não tem esse contexto — então o resume não aplicou, e o
   badge do passo 4 DEVE ser «Novo começo». O badge nunca pode mentir: lembrar↔«Sessão retomada»,
   não-lembrar↔«Novo começo». Se o badge disser «Sessão retomada» mas ele não lembrar, isso é o
   bug do risco #5 — anote.)*

## 4. (B) O badge honesto — lê antes de mandar a 1ª mensagem do dia
1. Ao reabrir, **antes de interagir**, olhe o canto de cada card:
   - No **Claude Code** que retomou: badge **«Sessão retomada»** (hover: *"O Agente continua de
     onde vocês pararam."*).
   - No **Terminal puro** (e em qualquer motor sem resume): badge **«Novo começo — o Agente não
     lembra da conversa anterior»** (hover: *"A conversa de antes continua guardada aqui na tela
     — é só rolar para cima."*).
2. **VEJA:** o badge é discreto e **some na primeira interação** com o agente — mas estava lá
   quando você precisou decidir se podia continuar a conversa de ontem.

## 5. (opcional) Trocar o verbo de resume sem recompilar
1. Edite `profiles/claude-code.toml`: troque `resume_args = ["--resume"]` por
   `resume_args = ["--continue"]` (ou outro verbo válido do seu Claude).
2. Reabra o Espaço. **VEJA:** o restore usa o novo verbo (o comando muda **sem recompilar** o
   app) — é a neutralidade multi-CLI: o verbo é dado de configuração, nunca constante de código.

---

### O que falha o roteiro (anote o item + o que apareceu)
- Card volta em posição/nome/papel errado, ou histórico não re-hidrata ao rolar para cima.
- Claude **não** lembra mas o badge diz «Sessão retomada» (ou o contrário) — **mentira de estado**.
- Badge ausente em algum card ao reabrir, ou que não some após a 1ª interação.
- Trocar o verbo no TOML não muda o comportamento (ficou preso no código).
