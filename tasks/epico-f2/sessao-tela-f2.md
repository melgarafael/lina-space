# Sessão de Tela da F2 — seu roteiro único (pra rodar do começo ao fim)

> **O que é isto.** Tem um punhado de coisas que só ficam prontas com as **suas mãos e seus
> olhos** na tela — conferir se a fonte e os ajustes visuais ficaram bons, medir a velocidade de
> digitação, testar com o leitor de tela, e (se você tiver fôlego) ensaiar o kit de testes de
> usuário. Em vez de te chamar cinco vezes, juntei tudo numa **sessão só**, na ordem do mais
> rápido pro mais longo. É só seguir as paradas, marcar os quadradinhos ☐ e anotar o que pedir.
>
> **Quem montou:** seu QA (Terminal D) · 2026-06-12. **Tempo total:** ~45 min (paradas 1–4) ·
> +40–50 min se fizer a parada 5 (opcional).
>
> Você não precisa entender nada de técnico. Onde aparece algo de bastidor, é nota pra equipe —
> está marcada como *(nota pra equipe)* e você pode pular.

---

## ✅ Antes de começar (prepare uma vez, ~5 min)

- [ ] **O aplicativo aberto.** Use a versão do Lina que a equipe te entregar pra esta sessão —
  ela já tem tudo o que as paradas abaixo precisam. *(nota pra equipe: o build precisa conter a
  costura da fonte F2-1-1b + os visuais da r4 `ef6c7af` + o spike de leitor de tela `a11y_live.rs`.
  Confirme um build único antes de entregar.)*
- [ ] **Seu iPhone** (ou outro celular com câmera em câmera lenta / "slow-motion"). Vamos usar nas
  paradas 3 e 4.
- [ ] **Fones ou caixa de som ligados** (a parada 4 precisa que você ouça uma voz).
- [ ] **Este documento aberto** num lugar onde você consiga marcar os ☐ e escrever (no
  computador ou impresso).
- [ ] **Janela do Lina em tamanho normal** (sem zoom, sem tela cheia maluca). Se mexer no tamanho,
  anote.

**Como ler cada parada:** cada uma diz *o que precisa antes · quanto tempo · o que você faz ·
o que anotar · e o que significa "passou" ou "não passou".* Faça na ordem.

---

## 🅰 Parada 1 — A fonte e o visual do terminal  ·  ~5 min  ·  *(confirmar e assinar)*

> Esta você já viu funcionando numa olhada rápida ("fluiu tudo"). Aqui é só **confirmar com
> calma e registrar** que está certo — isso fecha oficialmente essa peça.

**O que você faz — 4 olhadas:**

1. **A fonte do terminal.** Olhe as letras no terminal. Devem estar **nítidas, sem borrão**. O
   teste-chave: escreva (ou ache na tela) um `l` (ele minúsculo), um `1` (número um) e um `I`
   (i maiúsculo) — os três têm que ser **claramente diferentes**, e o `l` **não** tem pingo/serifa
   em cima. (É a fonte JetBrains Mono — feita pra não confundir esses caracteres.)
   - [ ] As letras estão nítidas e o `l`, `1`, `I` dá pra distinguir? **SIM / NÃO**

2. **A seleção de texto.** Com o mouse, **clique e arraste** sobre uma palavra do terminal e solte.
   A marca da seleção tem que cobrir **exatamente** as letras — nem sobrando, nem faltando, coladinha.
   - [ ] A seleção fica alinhada certinho com as letras? **SIM / NÃO**

3. **A linha de baixo (onde se digita).** Olhe a **última linha** do terminal, no rodapé. Tem que
   estar **inteira e visível**, não cortada. Se antes parecia espremida, agora deve caber melhor.
   - [ ] A linha de baixo aparece inteira, sem corte? **SIM / NÃO**

4. **A mensagem de partida.** *(esta é meio técnica — se for fácil, faça; se não, pule sem culpa.)*
   Quando o app abre, ele escreve uma mensagenzinha de bastidor com o tamanho da célula de texto.
   O esperado é algo como **`cell 7.8x17.16`**. Se em vez disso aparecer um **aviso de medição**,
   pare e me avise — quer dizer que a fonte não foi medida direito.
   - [ ] Mensagem que apareceu (copie a linha com a palavra `cell`): `__________________`

**Passou =** as 4 olhadas SIM (fonte nítida e distinguível · seleção alinhada · linha de baixo
inteira · medida real, não aviso).
**Não passou =** qualquer uma falhou (fonte borrada/genérica, seleção torta, linha cortada, ou
aviso de medição). Se falhar, me chame antes de assinar — volta pro conserto.

- [ ] **Assino que vi as 4 olhadas passarem — Parada 1 FEITA em ____/____/2026.**

---

## 🅱 Parada 2 — Os ajustes visuais  ·  ~10 min  ·  *(conferir a olho)*

> Aqui você confere se 4 detalhes visuais ficaram do jeito da direção que você escolheu nos
> protótipos (a "fusão T1"). É tudo a olho nu — não precisa medir nada.

1. **O ✕ de fechar (botão "fantasma").** Abra qualquer caixa/janela que tenha um **✕** no canto
   superior direito. Esse ✕ deve ser **discreto**: só o símbolo, **sem aquele quadradinho de fundo
   colorido** — fundo aparece só quando você passa o mouse. Clique nele: deve fechar.
   - [ ] O ✕ é discreto (sem fundo fixo, cor neutra) e fecha ao clicar? **SIM / NÃO**

2. **O respiro das caixas (espaçamento).** Abra uma caixa com texto (ex.: criar um agente, ou
   configurações). O texto **não pode estar colado na borda** — tem que ter um espaço de respiro
   em volta, parelho. *(nota pra equipe: 24px nas laterais, 16px em cima/baixo.)*
   - [ ] As caixas têm respiro e o texto não encosta na borda? **SIM / NÃO**

3. **Os botões secundários (cara "quente").** Procure botões de ação reversível (cancelar, voltar,
   trocar). Eles devem ter um **fundo sutil mas visível** — dá pra perceber que são clicáveis, mas
   são **neutros** (sem cor forte de destaque).
   - [ ] Os botões secundários têm fundo sutil e neutro (não berrante, não invisível)? **SIM / NÃO**

4. **O campo de colar (chave/senha).** Se houver um lugar de **colar uma chave** (ex.: chave de
   API/credencial), clique nele. Deve ter: **borda fininha visível**, fundo sutil, bom espaço por
   dentro e **alto o bastante pra clicar sem mirar**. Vazio, mostra um texto cinza de dica; com
   conteúdo, mostra o valor **sem cursor piscando** (é campo de colar, não de digitar).
   - [ ] O campo de colar tem borda, respiro e é confortável de clicar? **SIM / NÃO**

5. **Identidade nos cards/topo *(pode ainda não estar pronto)*.** Se a equipe te avisar que a peça
   de **identidade visual nos cards e na barra de cima** (F2-2-3) já entrou, confira aqui que a cara
   de marca aparece nos cards. Se ainda não entrou, **deixe em branco** — entra na próxima sessão.
   - [ ] F2-2-3 já entrou? Se sim, a identidade aparece nos cards/topo? **SIM / NÃO / AINDA NÃO ENTROU**

**Passou =** os 4 (ou 5) itens disponíveis estão SIM.
**Não passou =** qualquer item visual fora do combinado — anote qual e como estava, que a equipe
acerta.

---

## 🇨 Parada 3 — A velocidade da digitação  ·  ~20–30 min  ·  *(medir o "antes")*

> Esta mede uma coisa que **nunca foi medida** no Lina: quanto tempo passa entre você **apertar a
> tecla** e a **letra aparecer** na tela. É a sensação de "responde na hora" vs. "tá lento".
>
> **Aviso honesto, de propósito:** a gente **espera que hoje NÃO passe no alvo** — o app ainda tem
> um atraso de desenho que já sabemos e vamos otimizar. O ponto desta parada é registrar o **"antes"**
> pra comparar depois da otimização. Falhar aqui hoje é o resultado esperado, não um susto.

**O alvo (pra referência):** apertar→aparecer em **até 25 milésimos de segundo** na média, e até
**50 milésimos** no pior caso quando o app está cheio de coisa rodando. (Milésimo de segundo = um
piscar de olho tem ~100 deles.)

### Caminho recomendado pra você: o celular (câmera lenta)

1. **Baixe no iPhone o app gratuito "Is It Snappy?"** (ele filma em câmera lenta e **conta os
   quadros sozinho**, te dando o tempo em milésimos — feito exatamente pra isto).
2. **Cena "tranquila":** abra o Lina com **um terminal só**, clique nele pra deixar o cursor
   piscando no lugar de digitar.
3. Apoie o celular (tripé ou encostado) enquadrando **o seu dedo + a tela** onde a letra vai
   aparecer. Filme e **aperte uma tecla solta** (ex.: a letra `j`), com calma, umas **30 vezes**,
   ~1 por segundo.
4. No app do celular, marque **o quadro em que a tecla começa a descer** e **o quadro em que a letra
   aparece** — ele te dá o tempo. Anote os números.
5. **Cena "cheia":** peça pra equipe deixar o Lina no modo com vários painéis trabalhando ao mesmo
   tempo *(nota pra equipe: comando do `baseline-input-latency.md` §2 — 12 painéis, 10 ativos —
   e criar 1 terminal extra com ⌘T pra digitar)*. Repita as 30 medições.

   - [ ] **Tranquilo** — tempo típico medido: `______ ms`  ·  pior caso: `______ ms`
   - [ ] **Cheio** — tempo típico medido: `______ ms`  ·  pior caso: `______ ms`
   - [ ] Modelo do celular e do monitor usados: `__________________`

### *(nota pra equipe — caminho alternativo, Typometer)*

Se um ajudante técnico estiver junto, dá pra medir por software (mais amostras) com o **Typometer
(fork `frarees`)** — o passo a passo completo, com a parte de permissões do macOS e o cálculo dos
percentis, **já está pronto** em `tasks/epico-f2/baseline-input-latency.md` §3–4. Use a câmera
como **âncora de verdade** de qualquer jeito (§3.3). Preencha a tabela do §7 daquele arquivo.

**Passou =** típico ≤25 ms e pior caso ≤50 ms (cheio).
**"Não passou" hoje = resultado esperado:** anote os números como o **"antes"**. *(nota pra equipe:
no FAIL, registrar quanto do atraso é a cadência de ~40ms do desenho — F1-5-1b — vs. o caminho de
digitação em si.)*

---

## 🇩 Parada 4 — O leitor de tela (VoiceOver)  ·  ~10–15 min  ·  *(testar a voz)*

> O Lina precisa funcionar pra quem usa o **leitor de tela** (a voz do macOS que descreve o que
> está na tela, pra pessoas com baixa visão). Aqui você confere se um aviso importante — *"a
> resposta do seu ajudante ficou pronta"* — é **falado em voz alta sozinho**, sem você precisar
> procurar o aviso na tela.

**O que você faz:**

1. **Ligue o VoiceOver** apertando **⌘ + F5** (a mesma tecla desliga). Vai começar a ouvir uma voz
   descrevendo a tela.
2. **Comece a gravar a tela com áudio:** **⌘ + Shift + 5** → "Gravar tela inteira" → confirme que
   o **microfone está ligado** (pra gravar a voz) → Gravar. *(Assim fica a prova do teste.)*
3. No Lina, **coloque um agente pra trabalhar** e mande um pedido curto (ex.: *"diga oi e nada
   mais"*).
4. **Enquanto a voz fala**, navegue com o VoiceOver pra **longe** daquele agente (use VO+setas pra
   ir pra outro canto do app). A ideia é você **não estar** olhando/focado no aviso.
5. Quando a resposta ficar pronta, aparece um **aviso verde no canto de baixo à direita** escrito
   **`🔊 <nome do agente>: resposta pronta`**. **No mesmo instante**, o VoiceOver **deve falar
   sozinho**: *"<nome do agente>: resposta pronta"* — **sem** você focar nele.
6. **Repita com OUTRO agente:** mande um pedido a um agente diferente. Quando a resposta dele ficar
   pronta, o aviso muda de nome e o VoiceOver **deve falar de novo**, com o nome novo.
7. **Pare a gravação** e guarde o vídeo.

**O que anotar:**
   - [ ] O VoiceOver falou *"<nome>: resposta pronta"* **no instante** em que o aviso verde
     apareceu (1ª vez)? **SIM / NÃO**
   - [ ] Falou **de novo** quando trocou de agente (2ª vez)? **SIM / NÃO**
   - [ ] Guardei o vídeo com a voz: **SIM / NÃO**
   - [ ] Se **ficou mudo**: em que passo exato? (ex.: "o aviso apareceu mas a voz não falou")
     `__________________`
   - [ ] Versão do macOS e idioma do VoiceOver: `__________________`

**Passou =** o VoiceOver falou o aviso **as duas vezes, sozinho**, sem você focar nele — e o vídeo
comprova.
**Não passou =** ficou mudo quando o aviso apareceu, ou só falou quando você focou nele na mão.
*(nota pra equipe: se mudo mesmo com `set_live(Polite)` + `value` setados, o problema é na ponte
AccessKit↔macOS, não na nossa parte — reabrir a investigação antes de seguir; é o gate armado do
ADR 0028. Roteiro técnico de origem: `tasks/epico-f1/spike-a11y-roteiro.md`.)*

---

## 🇪 Parada 5 *(OPCIONAL)* — Ensaio do kit de testes  ·  ~40–50 min  ·  *(só se tiver fôlego)*

> **Faça só se ainda tiver energia.** Aqui você roda o **kit de teste de usuário** em **você mesmo**,
> de ponta a ponta. **Atenção — isto NÃO é o teste de verdade e NÃO vale como aprovação:** você
> conhece o produto por dentro, então não é "leigo". O objetivo é só **ensaiar o kit** (ver se as
> tarefas, o cronômetro, o questionário e a planilha funcionam) **antes** de chamar os 5 testers
> reais. Se algo emperrar aqui, a gente conserta o kit antes da rodada real.

**Prepare:** grave a tela com áudio (QuickTime), tenha um cronômetro, e o questionário SUS + as
cartas de palavras à mão. Use um espaço de teste isolado *(nota pra equipe:
`LINA_WS_ROOT=/tmp/teste-piloto`)*.

**Rode estas tarefas, narrando em voz alta o que tenta, como se fosse a 1ª vez:**

- **T0 — o teste dos 5 segundos.** Olhe a tela inicial por **5 segundos exatos**, esconda, e
  responda em voz alta: *"o que esse app faz?"*. Anote sua resposta literal.
  - [ ] T0 — minha resposta em 1 frase leiga: `__________________`

- **T1 — primeira missão (≤10 min).** *"Coloque um ajudante pra trabalhar e peça a ele uma lista de
  3 nomes pra uma cafeteria."* Faça sem consultar nada. Repare: em que você clica primeiro, e qual
  sua **reação ao ver o texto correndo** no terminal (encanta ou assusta?).
  - [ ] T1 deu certo (agente criado **e** os 3 nomes na tela, sem ajuda)? **SIM / NÃO** · tempo: `___`
  - [ ] Onde cliquei primeiro: `______`  ·  minha reação ao texto correndo (frase literal): `______`
  - [ ] Facilidade de 1 a 7 (7 = muito fácil): `___`  (se ≤3, anote o que dificultou)

- **T2 — arrumar a bagunça (≤8 min).** *(depende de telas que talvez ainda não estejam prontas — se
  não der pra montar a cena dos 5 terminais, só observe a tentativa.)* *"Você tem 5 ajudantes
  trabalhando e a tela está bagunçada. Arrume do seu jeito e deixe dois deles maiores."*
  - [ ] T2 deu certo (5 reposicionados + 2 maiores, sem travar)? **SIM / NÃO** · tempo: `___`
  - [ ] Achei a ação que só aparece ao passar o mouse, sozinho? **SIM / NÃO**
  - [ ] Facilidade 1 a 7: `___`

- **T3 — quem está esperando você (≤5 min).** *(depende da cor de estado — se não houver, observe a
  tentativa.)* *"Um ajudante está parado esperando uma resposta SUA. Descubra qual e responda."*
  - [ ] T3 deu certo (achou o certo **e** destravou, sem ajuda)? **SIM / NÃO** · tempo: `___`
  - [ ] Descobri pela **cor** ou **varrendo um a um**? `______`  ·  Facilidade 1 a 7: `___`

- **Fechamento:**
  - [ ] **SUS** (questionário de 10 perguntas, nota 1–5 cada — marque a 1ª impressão). Escore final
    (0–100): `___` *(referência: média do mercado ~68; ótimo ≥80; reprovado <51)*
  - [ ] **Cartas de palavras:** marque as **5 palavras** que melhor descrevem o que usou, e por quê.
    *(alvo bom: vivo · honesto · acolhedor · preciso · artesanal · — sinal ruim: genérico · confuso ·
    complicado · sem graça · frio)* `__________________`
  - [ ] **Decepção (Sean Ellis):** *"como você se sentiria se não pudesse mais usar isso?"* —
    [ ] muito decepcionado  [ ] um pouco  [ ] tanto faz
  - [ ] **Melhor momento** dos últimos 40 min (deve ser um **resultado** avançando, não um efeito de
    tela): `__________________`

**"Passou" do ENSAIO** *(não vale pro projeto — só diz que o kit funciona):* (a) você rodou T0→T3 +
fechamento sem pular nada; (b) achou **pelo menos 1 problema** de usabilidade; (c) conseguiu
preencher **todas** as linhas acima sem ficar buraco. Se o app travou, ou não deu pra montar uma
cena, ou a gravação ficou inútil → **conserte e refaça o ensaio antes de chamar os 5 testers reais.**

---

## 📋 Onde tudo isto se encaixa (resumo pra você)

| Parada | O que prova | Se passar… | Se não passar… |
|---|---|---|---|
| 1 — Fonte/visual do terminal | a identidade da fonte ficou nítida e alinhada | fecha a peça da fonte | volta pro conserto antes de assinar |
| 2 — Ajustes visuais | os detalhes da fusão T1 entraram certo | fecha os ajustes da rodada | equipe acerta o item anotado |
| 3 — Velocidade de digitação | qual a sensação de resposta **hoje** (o "antes") | vira a base de comparação | **esperado falhar hoje** — é o ponto de partida da otimização |
| 4 — Leitor de tela | o aviso é falado sozinho (acessibilidade) | confirma o caminho do ADR 0028 | reabre investigação da ponte com o macOS |
| 5 — Ensaio do kit *(opcional)* | o kit de teste de usuário funciona | libera chamar os 5 testers reais | conserta o kit e re-ensaia |

> **Importante:** as paradas 3 e 5 **não decidem** o futuro do produto nesta sessão — a 3 é só medir
> o "antes", e a 5 é só ensaio. As paradas **1, 2 e 4** são as que **fecham pendências** de verdade.

---

*Dúvida em qualquer parada? Pare e me chame (seu QA) — melhor uma pergunta que um dado torto.*
