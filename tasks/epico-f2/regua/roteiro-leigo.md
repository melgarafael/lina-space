# Roteiro de tarefas do leigo — v1 (kit da régua F2 · camada b)

> **Dono:** Terminal D (QA) · **Data:** 2026-06-12 · **Fontes:** régua D0 (`tasks/pesquisa-f2/entrega-d0-eval.md` camada b, A2/A3) · épico 38 (gates F2-2/F2-3) · despacho `r1-qa-regua.md`
> **Para quê:** este é o roteiro que o moderador (fundador) segue na rodada de 5 testers. Ele espelha os gates F2-2 e F2-3 — o resultado da rodada **é** o veredito do gate, sem tradução intermediária.

---

## 1. Perfil do tester e recrutamento

- **Quem:** empreendedor/autodidata **não-técnico** (não programa, não usa terminal) que **nunca viu o Lina** — nem screenshot. Quem já viu qualquer demo está queimado para a camada (b) e para o teste de 5 segundos (serve ainda para o line-up D+1, se usou o app 1×).
- **Quantos:** 5 por rodada (sucesso vira GATE binário com n=5 — D0 camada b). Fluxos centrais exigem **≥2 rodadas com correção entre elas** (Faulkner A2: rodada única acha no pior caso 55% dos problemas).
- **Queima de tester:** cada tester participa de **1 rodada** da camada (b). Para o line-up (D+1) ele continua válido.

## 2. Setup da sessão (antes do tester chegar)

| Item | Como |
|---|---|
| Build | `dist/Lina.app` da rodada (o MESMO build para os 5 testers — anotar data/commit do build na planilha) |
| Workspace | **Sempre isolado:** `LINA_WS_ROOT=/tmp/teste-rodada-N-tester-M` — nunca o workspace de produção do fundador (regra herdada de `prof-baseline.md`; estado limpo = todo tester vê a MESMA tela inicial) |
| CLI de IA | 1 CLI real instalado e autenticado na máquina de teste (o tester não pode travar em login de terceiro — isso seria medir o onboarding do CLI, não o nosso) |
| Gravação | Tela + áudio da sessão inteira (QuickTime basta). Consentimento gravado no início |
| Cronômetro | O da gravação (timestamp de início/fim de tarefa lido do vídeo — zero ferramenta extra) |
| Moderador | 1 pessoa (fundador). **Não ajuda** (ver §5) |

## 3. Ordem da sessão (~40 min/tester — integra os 4 artefatos do kit)

| # | Bloco | Artefato | Tempo |
|---|---|---|---|
| 1 | Boas-vindas + consentimento + "pense em voz alta" | este roteiro | 3 min |
| 2 | **Teste de 5 segundos** (ANTES de qualquer contato com o app) | §4-T0 abaixo | 2 min |
| 3 | **Tarefas T1→T3** (sem ajuda; SEQ verbal após cada uma) | §4 | 20-25 min |
| 4 | **SUS pt-BR** (formulário de 10 itens) | `regua/sus-ptbr.md` | 5 min |
| 5 | **Reaction cards** (top-5 + por quês) | `regua/reaction-cards.md` | 5 min |
| 6 | Pergunta de decepção (Sean Ellis) + melhor momento | §6 | 2 min |
| 7 | Agendar o line-up do dia seguinte | `regua/lineup-distintividade.md` | 1 min |

> A ordem importa: o 5s mede primeira impressão (queimaria depois das tarefas); os reaction cards vêm DEPOIS do uso (o tester reage ao app *rodando*, não a um screenshot — D0 camada c).

## 4. As tarefas (o moderador lê em voz alta, exatamente assim)

### T0 — Teste de 5 segundos *(gate do "zero jargão", inv#6)*
Mostrar o screenshot padrão do canvas (o mesmo para todos os testers da rodada) por **5 segundos exatos**, depois esconder.
> **Ler:** "Vou te mostrar uma tela por 5 segundos. Depois eu pergunto o que você viu."
> **Perguntar:** "O que esse aplicativo faz?"
- **Registrar:** a resposta literal. **PASS da rodada:** ≥8/10 (acumulado em 2 rodadas) descrevem o propósito corretamente em linguagem leiga.
- Escopo estrito: primeira impressão/clareza — nunca julgar comportamento por este teste (D0).

### T1 — Criar o 1º agente *(espelha o caminho crítico do GATE F2-2; golden-path do inv#6)*
Estado inicial: app aberto, workspace limpo, tela inicial padrão.
> **Ler:** "Esse aplicativo te dá ajudantes de inteligência artificial que trabalham para você. Sua primeira missão: **coloque um ajudante para trabalhar e peça a ele uma lista de 3 nomes para uma cafeteria.** Diga em voz alta o que você vai tentando."
- **Sucesso (binário):** o tester cria o agente E recebe os 3 nomes na tela, **sem nenhuma ajuda**.
- **Tempo máximo:** 10 min (estourou = falha; anotar onde travou).
- **Observar (sem intervir):** em qual elemento ele clica primeiro · se o estado vazio o guia · reação à "cara de terminal" quando o agente responde — **hipótese do gate F2-2: encanta vs assusta** (anotar a frase literal do tester ao ver o texto correndo).

### T2 — Organizar o canvas *(espelha o GATE F2-3)*
Estado inicial: moderador carrega a cena preparada com **5 terminais** espalhados/sobrepostos (mesma cena para todos — montar 1× e restaurar por workspace isolado novo).
> **Ler:** "Agora você tem cinco ajudantes trabalhando ao mesmo tempo e a tela ficou bagunçada. **Arrume a tela do jeito que ficaria bom para você** — e deixe **dois** deles maiores, do tamanho que preferir."
- **Sucesso (binário):** os 5 terminais reposicionados de forma deliberada (o tester declara que "ficou bom") E 2 redimensionados — **sem ajuda, sem jank que o tester comente** ("travou", "engasgou" = anotar como falha de fluidez, conta para a camada d).
- **Tempo máximo:** 8 min.
- **Observar:** acha o gesto de mover sozinho? · usa "Arrumar"/preset se existir? · **tie-breaker do hover-only (gate F2-2): se ≥1/5 não achar a ação que só aparece no hover, a pista permanente vence** — anotar literalmente quem não achou.

### T3 — Achar quem pede aprovação *(espelha o GATE F2-3 / cor semântica F2-2-3)*
Estado inicial: a mesma cena de T2, com 1 terminal em estado **precisa-de-você** (vermelho, conforme o território F2-0-D) e os demais trabalhando/prontos.
> **Ler:** "Um dos seus ajudantes está parado, esperando uma resposta SUA para continuar. **Descubra qual é e dê a resposta para ele seguir.**"
- **Sucesso (binário):** o tester identifica o terminal certo E destrava o agente, sem ajuda.
- **Tempo máximo:** 5 min.
- **Observar:** ele acha pela COR (âmbar/verde/vermelho — acoplamento OP-1) ou varrendo um a um? · usa a tecla/atalho de "pular ao que pede aprovação" se existir? · anotar a pista que de fato o guiou.

### Após CADA tarefa — SEQ verbal *(D0 camada b, A3)*
> **Perguntar:** "De 1 a 7, sendo 7 muito fácil: quão fácil ou difícil foi essa tarefa?"
- Toda nota **≤3** dispara na hora: "o que a tornou difícil?" (registrar literal).
- **PASS da rodada:** mediana ≥5,5 E nenhuma nota ≤3 sem causa identificada. Mediana, não média — n=5 não aguenta média.

## 5. Regras do moderador (a régua só vale se isto for cumprido)

1. **Não ajude.** Nem dica, nem "tá quente/frio", nem apontar com o mouse. Frases permitidas: "o que você esperava que acontecesse?" · "continue tentando do jeito que faz sentido pra você" · "lembre de pensar em voz alta".
2. Tester pediu ajuda explícita → "não posso ajudar nessa parte; se você desistir, é só dizer". Desistência = falha da tarefa (anotar o ponto exato).
3. Estourou o tempo máximo → encerrar a tarefa com gentileza e seguir ("perfeito, vamos para a próxima") — falha anotada, sessão continua.
4. Bug que IMPEDE a tarefa (crash, terminal morto) → anotar como **bug bloqueante** (vai para o dogfooding), reiniciar a cena e **repetir a tarefa 1×**; se repetir, a tarefa é falha por bug, não por UX — registrar separado.
5. Nunca explicar jargão. Se o tester perguntar "o que é um terminal?", responder: "o que você acha que é, olhando para a tela?" (a resposta é dado).

## 6. Fechamento da sessão

- **Pergunta de decepção (Sean Ellis, D0 camada f):** "Como você se sentiria se não pudesse mais usar isso que acabou de usar?" [muito decepcionado / um pouco / tanto faz] — tendência entre rodadas, nunca veredito com n=5.
- **Fim lembrado:** "Qual foi o melhor momento dos últimos 40 minutos?" — PASS qualitativo da camada (f): o melhor momento citado é *resultado avançando* (o agente entregou algo), não estímulo de interface.
- **Arrependimento:** "O tempo que você passou aqui valeu?" [valeu / mais ou menos / me arrependi].

## 7. Planilha de registro (1 linha por tester)

| Tester | Build | T0 (resposta literal) | T1 ✓/✗ + tempo | SEQ T1 | T2 ✓/✗ + tempo | SEQ T2 | T3 ✓/✗ + tempo | SEQ T3 | SUS (0-100) | Top-5 cards | Decepção | Hover-only achado? | Frase "cara de terminal" |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|

## 8. PASS da rodada (consolidado da D0 — para o Maestro ler de uma vez)

- [ ] **5/5** completam T1-T3 sem ajuda (gate duro; qualquer falha → conserta e re-roda)
- [ ] SEQ mediana ≥5,5 E toda nota ≤3 com causa identificada
- [ ] SUS: tendência não-decrescente entre rodadas (alvo de fase ≥68; aspiração ≥80,3)
- [ ] Fluxos centrais (T1, T2) com **2ª rodada 5/5 + zero problema crítico novo**
- [ ] Hipótese F2-2 respondida com frases literais (encanta vs assusta) + tie-breaker hover-only decidido
- [ ] Tempo de tarefa: só comparação ENTRE rodadas (n=5 não dá tempo absoluto)

## 9. Limitações declaradas (honestidade)

- Roteiro v1 escrito ANTES da cara nova existir (F2-2/F2-3 em execução): os estados iniciais de T2/T3 assumem capacidades das stories F2-3-1/2 e F2-2-3 — se a rodada rodar antes delas, T2/T3 degradam para "observar a tentativa" (dado qualitativo, sem gate).
- "Criar 1º agente <2h" (inv#6) cobre o onboarding completo (instalar → 1º agente); T1 mede só o trecho dentro do app — o invariante completo se mede na primeira instalação real de um tester, fora desta sessão.
- A cena de T2/T3 precisa ser montada 1× pelo fundador e clonada por workspace isolado — se a restauração de layout (F2-3-7) ainda não existir, montar manualmente antes de cada tester (anotar variação).
