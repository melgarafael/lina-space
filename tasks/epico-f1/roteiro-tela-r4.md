# 🎬 ROTEIRO DE TELA v2 (rodada r4) — Rafael, é só seguir na ordem (2026-06-11)

> **Curto de propósito:** a sessão anterior (11/06 ~12h) validou quase tudo — este roteiro só
> tem o que MUDOU depois dela + o que a rodada r4 vai entregar. Cada item diz o que fazer e o
> que você deve **VER**. Se algo não acontecer como descrito, anote qual item e o que apareceu —
> isso já é o relatório. Antes de começar: **feche o Lina antigo e abra o `dist/Lina.app` NOVO**
> (reempacotado 11/06 à noite, 20:49 — confira que não é uma cópia antiga).

## 1. Criar Espaço — re-teste pós-conserto (era o que travou na sua última sessão)

A tela de "Criar Espaço" que falhou com você foi consertada em duas frentes
(commits `ef6acd6` + `f1d4810`). Refaça o caminho inteiro:

1. Abra o painel lateral esquerdo (o trilho de Espaços) e clique em **Criar Espaço**.
2. **VEJA:** a janelinha abre INTEIRA dentro da tela — nada cortado embaixo; se o conteúdo
   for maior que a janela, aparece rolagem interna (experimente reduzir a janela do app
   antes de abrir, para forçar).
3. Clique no campo **Nome** e digite; depois clique no campo **Diretório** e digite.
4. **VEJA:** cada campo aceita o clique e o cursor vai para ele na hora (antes, o clique
   não focava).
5. Escolha um Diretório de Trabalho real (uma pasta de projeto sua) e confirme a criação
   com 2-3 terminais.
6. **VEJA:** o Espaço novo abre com os terminais **lado a lado em grade, sem um em cima do
   outro** — e cada terminal já nasce DENTRO da pasta que você escolheu (peça a um deles
   "em que pasta você está?" para conferir).
7. **VEJA (troca viva):** volte ao Espaço anterior pelo trilho e retorne ao novo — os
   terminais continuam vivos nos dois, ninguém reinicia.

## 2. Badge EoL do Gemini (sai NESTA rodada r4)

1. Abra o criar-agente e escolha **Gemini** como CLI.
2. **VEJA:** o aviso de fim-de-vida (badge EoL) aparece no modal/dashboard, com a copy que
   você já aprovou. *(Se não aparecer: a entrega desta rodada ainda não chegou ao seu
   `.app` — confirme com o Maestro antes de reprovar.)*

## 3. Announcement do Criar Espaço com VoiceOver (sai NESTA rodada r4)

1. Ligue o VoiceOver **ANTES** de abrir o Lina (⌘F5).
2. Percorra o fluxo de Criar Espaço do item 1.
3. **OUÇA:** o VoiceOver anuncia a abertura da janela e os campos pelo nome ("Nome",
   "Diretório de Trabalho"), sem você precisar adivinhar onde está.

## 4. Ativação de licença (SLOT — só quando F1-4-6 sair; o Maestro avisa)

*Reservado: quando a tela de licença entrar, o passo será: abrir o app sem licença →
**VER** a tela pedir a chave → colar a chave → **VER** o app destravar e lembrar disso
no próximo open. Não teste antes do aviso do Maestro.*

---

## ✅ O que você JÁ validou e NÃO precisa repetir

Validado por você em **2026-06-11 ~12h** (roteiro consolidado percorrido; gate F1-1 declarado):

- **Spawn em cascata pede seu aval** (anti-fork-bomb) — gate F1-3 já estava declarado em 10/06.
- **Aprovação pelo toast destrava o agente travado em y/n** (AC-0021.7 — era a trava do F1-1).
- **Render-scale / zoom-out com 12+ terminais** (os números `[PROF]` o Maestro lê do log).
- **VoiceOver anuncia resposta de agente sem foco** (spike F1-2-7).
- **Fixes do time externo:** dois agentes na mesma pasta conversam · pedido barrado aparece
  no sininho · etiqueta de autonomia no card.

**Continua adiado (não é deste roteiro):** A2A Claude→Codex na tela — a conta do Codex
volta em **09/jul/2026**; é também o que fecha o 2º output criativo do gate anti-slop (§3.2
do `relatorio-saida-f1.md`).

---
*Roteiro v2 escrito pelo QA (despacho `r4-validacao-saida`), 2026-06-11. Itens 2-4 dependem
das entregas da rodada r4 chegarem ao repack — o Maestro confirma antes da sua sessão.*

---

## ADENDO r5 — os 4 bugs que você reportou (consertados; valide nesta ordem)

> Antes: feche o Lina antigo e abra o `dist/Lina.app` NOVO (repack **11/06 23:58** — tem TUDO
> das rodadas r4+r5).

### A. Velocidade (era o bug 3 — "impossível de usar")
1. Abra 3+ Espaços e deixe-os de fundo. Abra o Monitor de Atividade (Aplicações › Utilitários).
2. **VEJA:** o processo "Lina" em repouso perto de **0% de CPU** (antes: dezenas de % contínuos).
3. Crie um Espaço novo. **VEJA:** sem o engasgo de ~10 segundos de antes.
4. Abra/feche o rail e troque de Espaço várias vezes. **VEJA:** sem travadinhas a cada clique.

### B. Rail — abrir/fechar (bug 1)
1. Pressione **⌘O** repetidas vezes. **VEJA:** o rail abre E recolhe (alterna).
2. Com o rail aberto, clique na **setinha ◀** no topo dele. **VEJA:** recolhe pelo mouse.

### C. Rail — tirar um Espaço da lista (bug 2)
1. Abra o rail e passe o mouse sobre a linha de um Espaço que NÃO é o atual.
2. **VEJA:** as ações **Renomear** e **Arquivar** visíveis na linha (com o subtexto
   "some da lista; nada é apagado do disco").
3. Arquive um. **VEJA:** o toast com **[ Desfazer ]** por 8s (clique, Tab+Enter ou ⌘Z desfazem).

### D. Rail — o teclado é seu (bug 4)
1. Abra o rail (⌘O) e, SEM clicar nele, digite no terminal focado.
   **VEJA:** o texto vai para o TERMINAL (antes ia para a busca do rail).
2. Clique na busca do rail (ou ⌘F com o rail aberto) e digite. **VEJA:** agora sim filtra.
3. Com o foco num terminal, pressione **Esc**. **VEJA:** o rail NÃO fecha (o Esc é do terminal);
   com o foco no rail, **Esc** fecha.

### E. Descarregar (novo — modelo "Maestri")
1. Com um Espaço de fundo cujos agentes estão OCIOSOS, abra o rail → ação **Descarregar** na linha.
2. **VEJA:** a narração diz quantos desligaram (e, se alguém estava trabalhando, que ficou ligado).
3. Volte o foco para esse Espaço (⌘número ou clique). **VEJA:** os terminais religam sozinhos.
