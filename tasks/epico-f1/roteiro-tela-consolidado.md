# 🎬 ROTEIRO DE TELA CONSOLIDADO — Rafael, é só seguir na ordem (2026-06-10)

> **Um roteiro só, com TUDO que depende do seu olho.** Cada item diz o que fazer e o que você
> deve VER. Se algo não acontecer como descrito, anote qual item e o que apareceu — isso já é
> o relatório. Antes de começar: **feche o Lina antigo e abra o `dist/Lina.app` NOVO**
> (reempacotado hoje à noite com a sessão inteira).

## 0. Pré-requisito de conta (5 min, resolve o time inteiro)
O limite mensal da conta Claude foi atingido hoje à tarde — os terminais do canvas param de
responder até isso subir. Acesse **claude.ai/settings/usage** e aumente o limite mensal.
Sem isso, os itens 1-2 abaixo não funcionam (os agentes não respondem).

## 1. Spawn — a metade que falta: a CASCATA pede seu aval (gate F1-3, item final)
A metade "origem" já foi provada HOJE em uso real (o Maestro trouxe um "Especialista em
Licenças" e o terminal NASCEU na tela — você talvez tenha visto). Falta a CASCATA:
1. Crie um agente novo (⌘N, autonomia **autônomo**).
2. Peça a ele: *"faz uma pesquisa de mercado sobre cursos de IA — se faltar alguém no time, traz"*.
3. **VEJA:** ele percebe a lacuna e um terminal novo NASCE no canvas com narração.
4. Agora peça **ao terminal NOVO** que traga mais alguém ("traz um designer pro time").
5. **VEJA:** desta vez NÃO nasce direto — **um aviso entra na fila de atenção pedindo o SEU
   aval** (anti-fork-bomb vivo: criação em cadeia exige humano). Aprove e veja nascer;
   ou negue e veja a recusa narrada.

## 2. Aprovação pelo toast destrava um agente de verdade (AC-0021.7 — declara o gate F1-1)
1. Peça a um agente algo que dispare pedido de permissão (ex.: rodar um comando que o
   Claude Code pergunte "y/n").
2. **VEJA:** o avisinho (toast) aparece na fila de atenção em segundos.
3. Aprove PELO TOAST (sem clicar no terminal).
4. **VEJA:** o terminal destrava sozinho e continua o trabalho. *(É a única pendência que
   segura a declaração formal do gate F1-1 — o conselho de auditoria já passou com zero falha.)*

## 3. Render-scale — a célula que só o seu canvas tem (onda F1-5)
1. No seu Espaço real (~27 terminais), dê **zoom-out até ver 12+ terminais ao mesmo tempo**.
2. Deixe rodando ~1 minuto com atividade (peça algo a 2-3 agentes).
3. **PRONTO — nada para julgar:** o app grava os números sozinho no log (`[PROF]` no stderr);
   o Maestro lê depois. (Medição de hoje: com a janela padrão o app está fluido e o custo por
   painel é mínimo; falta só esta célula com muitos painéis visíveis.)

## 4. VoiceOver anuncia sem foco (spike de acessibilidade F1-2-7)
Roteiro detalhado em `tasks/epico-f1/spike-a11y-roteiro.md` (resumo):
1. Ligue o VoiceOver **ANTES** de abrir o Lina (⌘F5).
2. Abra o Lina e faça um agente responder algo.
3. **OUÇA:** o VoiceOver anuncia a novidade SEM você ter focado o elemento.

## 5. A2A universal — Claude fala com Codex (pendente desde 06-09)
⚠️ A conta do Codex também está no limite (volta 09/jul) — **adie este item** ou teste com
um terminal Codex já logado em outra conta: criar agente Codex + agente Claude → pedir ao
Claude "manda oi pro <nome do Codex>" → **VEJA** a mensagem chegar no terminal Codex.

## 6. Badge EoL do Gemini (cosmético, rodada 2)
Ao criar um agente Gemini, **VEJA** o aviso de fim-de-vida (copy já aprovada por você).
*(Se ainda não aparecer: o render é item da rodada 2 — só confirme depois dela.)*

---
*Itens 1-2 destravam a DECLARAÇÃO dos gates F1-3 (fechamento total) e F1-1. O item 3 destrava
as decisões de otimização da onda F1-5. Roteiro escrito pelo Maestro, sessão de fechamento da F1.*
