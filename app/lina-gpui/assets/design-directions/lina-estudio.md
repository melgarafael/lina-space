# Instrumento de Estúdio

> Category: Lina · Studio Instrument (warm)
> A direção oficial do Lina: a fusão T1 + T3 decidida no Épico Fase 2 §VIII.1 —
> "Instrumento de Estúdio com a temperatura do Ateliê". Centro de gravidade no
> instrumento (cor semântica fixa acoplada ao controle, viewer vivo no foco, flat
> honesto); temperatura no ateliê (superfícies quentes, acolhimento na moldura).
> Palavras-alvo: vivo · honesto · acolhedor · preciso · artesanal.

## 1. Visual Theme & Atmosphere

Uma estação de trabalho que parece um **instrumento de precisão com calor de
ateliê** — não um app frio nem um brinquedo colorido. O canvas é uma bancada onde
vários terminais de IA trabalham lado a lado; cada um é um **viewer vivo** quando
está no foco e congela honestamente na periferia (a tela nunca finge atividade que
não existe). A hierarquia vem de tom e linha de 1px, nunca de sombra dramática ou
gradiente. O leigo opera um time de IAs e **sente que está no comando**, não
perdido num painel de nave.

- **Postura:** flat honesto, denso governado, top-biased.
- **Centro de gravidade (T1):** o instrumento — cor é significado, o foco é um
  viewer vivo, a periferia é congelada e assumida.
- **Temperatura (T3):** o ateliê — superfícies quentes, presença nomeada dos
  colegas, celebração contida só quando algo de fato conclui.
- **Mood:** vivo, honesto, acolhedor, preciso, artesanal.

Rejeitadas na decisão: o frio minimalista (afasta o leigo), o denso de sala de
controle (pesado como default) e o T1 puro (perderia o calor que aproxima).

## 2. Color

A cor **nunca é decoração** — ela é o estado do trabalho, fixo e aprendível. O
leigo memoriza quatro sinais e lê o Espaço de relance, sem legenda:

- **Âmbar — "trabalhando".** O terminal está produzindo agora.
- **Verde — "pronto".** Concluído, saudável, conectado.
- **Vermelho — "precisa de você".** Pede uma decisão humana; nunca decorativo.
- **Azul — "mensagem do time".** Um colega falou com este terminal.

### Regras de cor

- Cada cor de estado aparece **só** quando aquele estado é verdade. Cor sem
  significado é proibida (vira ruído e treina o olho a ignorar o sinal).
- **Superfícies são quentes:** carvão quente no escuro, papel quente no claro —
  **nunca preto frio (`#000`) nem branco puro (`#fff`)**. O valor mais escuro é um
  carvão com pigmento; o mais claro, um papel com creme.
- **Terminais são sempre-dark**, mesmo no tema claro: o grid de código vive melhor
  no escuro, e a moldura clara o emoldura como uma peça sobre a bancada.
- Acessibilidade não é opcional: todo estado é **texto + ícone + cor** (WCAG
  1.4.1), nunca cor sozinha.

## 3. Typography

Três famílias, cada uma com um trabalho — nenhuma escolhida por inércia:

- **IBM Plex Sans — a voz da casa (UI/chrome).** Humanista-técnica, com calor e
  pt-br impecável. Carrega rótulos, narração e corpo.
- **Fraunces — os momentos (display).** Só em conclusões e aberturas — quando o
  fundador merece um instante de celebração contida. Nunca em corpo de UI.
- **JetBrains Mono — o grid (terminal e código).** O tecido do trabalho vivo.

A hierarquia é **deliberada**: tamanho e peso marcam o ritmo (rótulo → corpo →
momento), não um amontoado de tamanhos avulsos.

## 4. Motion & Craft

- **Celebração contida:** o movimento aparece na conclusão, breve, e nunca rouba a
  cena. Nada pisca à toa.
- **Acolhimento na moldura:** os colegas têm nome e presença; a narração é leiga e
  em primeira pessoa. O artesanato está nos detalhes — a borda de 1px, o degrau de
  tom, o tempo da transição — não em efeito cosmético.
- **Reduce-motion respeitado:** quem pede menos movimento recebe menos movimento.

## 5. Anti-patterns (o que esta direção recusa)

- Gradiente decorativo, glassmorphism, neon — competência superficial de "default
  de IA".
- Cor como enfeite (sem estado por trás).
- Preto frio / branco puro como superfície.
- Sombra dramática para fingir profundidade — a hierarquia é tom + linha.
- Densidade de sala de controle empurrada ao leigo como padrão.
