# Entrega UX flow — Fluxos da Fase 1

> Papel UX FLOW · Épico Fase 1 do Lina Space · 2026-06-06
> Formato herdado de [[22 - Fluxo de Telas e Mapa de Cliques]] (Fase 0). Princípios citados de [[13.3 - Pesquisa F1 - UX Canvas e Design Systems]] (abrev. **13.3**) e [[13.13 - Pesquisa F1 R2 - UX de Notificacao de Permissao]] (abrev. **13.13**).
> Referência visual a SUPERAR (não copiar): modal "Novo Terminal" do Maestri — 4 prints de 2026-06-03 no vault.

## Sumário

Este documento desenha os fluxos de tela das cinco frentes de UX da Fase 1 — (a) Modal Novo Agente v2 + edição de Agente vivo, (b) Fila de Atenção unificada (permissão + custódia), (c) observabilidade por Agente e do Espaço, (d) tema e personalização, (e) Espaços (switcher, criação, ativação PRO) — em wireframe textual estado-a-estado, com mapas de cliques, atalhos, estados vazios e de erro, no mesmo formato do doc 22 da Fase 0.

O fio condutor da Fase 1: **governança e observabilidade visíveis** (o usuário VÊ o que seus Agentes fazem e quanto custam), **atenção sem ansiedade** (nada interrompe; tudo sinaliza e espera), e **anti-slop visual** (nenhuma superfície genérica de IA). Os três princípios da Fase 0 continuam lei: primeiro resultado rápido, zero jargão na superfície, nunca tela em branco / estado sempre salvo.

## Como ler este documento

- **IDs**: a Fase 0 reservou T0–T8 (telas), M1–M5 (modais), P1–P4 (painéis), W1+ (wireframes). A Fase 1 continua a numeração: **M6/M7** (modais novos), **P5/P6** (painéis novos), **B1** (elemento de barra), **T7§** (seções novas dentro de T7 Ajustes), **TO** (toast).
- Cada fluxo declara: **objetivo → estados (E0, E1…) com wireframe textual → mapa de cliques → atalhos → estados vazios → estados de erro → princípio de design aplicado (com citação) → anti-padrões (o que NÃO fazer)**.
- O **glossário sem jargão** do doc 22 permanece lei: *Agente* (nunca "terminal" na superfície), *Motor de IA* (nunca "CLI"), *Espaço*, *Foco*, *Time conectado*. Termos novos da Fase 1, na mesma disciplina:

| Termo na UI (use sempre) | O que é por baixo (técnico) |
|---|---|
| **Fila de Atenção** | Fila unificada de permission prompts + custódia (BrokerPump/CustodyDesk) |
| **Pedido** | Permission request (yes/no no stdin) ou custody gate |
| **Painel do Agente** | Observabilidade por terminal (tokens, custo, atividade, estado) |
| **Medidor do Espaço** | Agregado de custo/teto/pausa do workspace |
| **Esperando você** | Terminal bloqueado em prompt de permissão |
| **Sem resposta** | Terminal travado/hung de verdade |
| **Chave do Lina PRO** | Licença (license key) |

### Superfícies novas da Fase 1 (tabela rápida)

| ID | Superfície | Fluxo | Substitui / estende |
|---|---|---|---|
| M6 | Modal Novo Agente v2 | a | Evolui o M2 da Fase 0 |
| M6-E | M6 em modo Editar (Agente vivo) | a | Novo |
| TO | Toast de Atenção | b | Novo |
| P5 | Fila de Atenção (mailbox lateral) | b | Estende o gate Cmd+Enter |
| P6 | Painel do Agente (seção do inspetor + modo expandido) | c | Estende o inspetor da Fase 0 |
| B1 | Medidor do Espaço (barra superior) | c | Novo |
| T7§A | Ajustes › Aparência | d | Nova seção em T7 |
| M8 | Switcher de Espaços (dropdown) | e | Novo |
| M9 | Criar novo Espaço | e | Reusa T3 (Galeria de Focos) em modal |
| T7§P | Ajustes › Plano (ativação PRO) | e | Nova seção em T7 |

Cada superfície traz, na abertura da sua seção, o trio **Tamanho (S/M/L) · Dependências · Critério de aceite** no formato do doc 32 da Fase 0.

### Atalhos novos da Fase 1 (tabela consolidada)

| Atalho | Ação | Observação |
|---|---|---|
| `Cmd+N` | Abrir M6 (Novo Agente) | — |
| `Cmd+Enter` | Aprovar o Pedido **em foco** | Gesto existente da custódia, estendido — nunca aprova às cegas |
| `Cmd+Shift+Enter` | Negar/dispensar o Pedido em foco | Conforme 13.13 (backlog 2: "dismiss via Esc ou Cmd+⇧⏎") |
| `Esc` | Fechar toast/painel **sem decidir** | Esc nunca nega nem aprova |
| `Cmd+J` | Abrir/fechar P5 (Fila de Atenção) | Proposta — validar colisões |
| `Cmd+,` | T7 Ajustes | Padrão macOS |
| `Cmd+O` | Abrir M8 (Switcher de Espaços) | Proposta |
| `Cmd+1…9` | Trocar direto para o Espaço n | Proposta |
| `Cmd+Shift+N` | Criar novo Espaço (M9) | Proposta |
| `Tab` | Aceitar sugestão do co-piloto de papel (M6) | Padrão ghost-text |

Tudo que está aqui também existe na paleta `Cmd-K` por nome humano ("Novo Agente", "Fila de Atenção", "Trocar de Espaço…", "Aparência") — a paleta é o caminho descobrível, o atalho é o caminho rápido.

---

## Fluxo (a) — M6 · Modal Novo Agente v2 + M6-E · Editar Agente vivo

### Objetivo

Criar um Agente em menos de 30 segundos com no máximo **uma decisão real** (o papel), usando o que JÁ está instalado no computador, com o Lina sugerindo papel e prompt a partir do nome digitado (role-discovery existente). Superar o modal do Maestri em três frentes: detecção real de motores (vs. lista fixa de 5), co-piloto de papel (vs. textarea de YAML cru), e um formulário de coluna única com 4 controles visíveis (vs. 3 abas que escondem o essencial).

> **M6 — Tamanho:** M · **Dependências:** F1-2-1 (design system), F1-2-3 (`RoleSuggester`), sondagem de motores de T1 (Fase 0), galeria de presets (Fase 0) · **Critério de aceite:** num computador com um único motor instalado, um leigo cria um Agente funcional digitando apenas um nome e clicando `Criar` — sem ver YAML, comando ou a palavra "terminal".

### Pontos de entrada

| De onde | Gesto |
|---|---|
| Trilho esquerdo | Clique no `+` |
| Canvas | Clique-direito em área vazia → "Adicionar Agente aqui" (nasce na posição do clique) |
| Paleta | `Cmd-K` → "Novo Agente" |
| Teclado | `Cmd+N` |
| Galeria de presets (existente) | "Usar este papel" num preset → abre M6 já com papel preenchido |

### M6-E1 — Quick-start por motor DETECTADO

A linha de início rápido mostra **somente os motores `✓ Pronto`** (reusa a sondagem de T1 — resultado cacheado + re-scan silencioso em background ao abrir o modal). O que não está instalado não aparece como par igual: vive atrás de um único chip `+ Instalar outro motor…`.

```
┌──────────────────────────────────────────────────────────────────────┐
│                          Novo Agente                            ✕    │
├──────────────────────────────────────────────────────────────────────┤
│  Início rápido — motores prontos no seu computador:                  │
│                                                                      │
│   ┌──────────────┐  ┌────────────┐  ┌──────────────────────┐         │
│   │ ◉ Claude Code│  │ ○ Gemini   │  │ + Instalar outro     │         │
│   │   «recomendado»│ │   CLI      │  │   motor…             │         │
│   └──────────────┘  └────────────┘  └──────────────────────┘         │
│                                                                      │
│  Nome           ( Ajudante de Marketing___________ )                 │
│                 ↳ co-piloto: «Parece um [Criador de Conteúdo] —      │
│                    usar este papel? (Tab)»                           │
│                                                                      │
│  Papel          ┌────────────────────────────────────────────┐      │
│                 │ ✎ Criador de Conteúdo                       │      │
│                 │ Escreve posts, roteiros e ideias no seu tom.│      │
│                 │ [ Ver e ajustar instruções ▸ ]   [ Trocar ] │      │
│                 └────────────────────────────────────────────┘      │
│                                                                      │
│  Pasta de       ( ~/EspaçosDeTrabalho/MeuNegocio ▾ )  [ Trocar… ]   │
│  trabalho       «já é a pasta deste Espaço»                          │
│                                                                      │
│  ▸ Avançado                                                          │
│                                                                      │
│                          [ Cancelar ]      [[ Criar Agente ]]        │
└──────────────────────────────────────────────────────────────────────┘
```

**Os 4 controles visíveis** (13.3 achado 4: "3-5 controles, não 20 campos"):
1. **Motor** — chips dos detectados, recomendado pré-selecionado (default inteligente: se só há um, já vem escolhido e a linha vira um selo discreto, não uma decisão).
2. **Nome** — campo de texto, foco inicial do teclado. É O controle que dispara o co-piloto.
3. **Papel** — cartão humano (ícone + nome do papel + 1 frase "o que ele faz"). Nunca YAML, nunca textarea em branco.
4. **Pasta de trabalho** — pré-preenchida com a pasta do Espaço; trocar é exceção, não regra.

**Dentro de `▸ Avançado` (dobrado, divulgação progressiva):** comando do motor (o campo "Comando: claude" que o Maestri exibe como primário mora AQUI), argumentos/flags, prompt de sistema completo (texto puro, com aviso "modo técnico"), variáveis de ambiente, permissões iniciais do Agente. Quem nunca abrir o Avançado cria Agentes perfeitos mesmo assim.

### M6-E2 — Co-piloto de papel (role-discovery)

Enquanto o usuário digita o nome, o Lina sugere papel + prompt (mecanismo de role-discovery já existente na Fase 0):

- **Disparo:** pausa de digitação (~600 ms) com 3+ caracteres no campo Nome.
- **Forma:** linha-fantasma sob o campo — «Parece um **[Vendedor de Conteúdo]** — usar este papel? (Tab)». Nunca um popup que rouba foco.
- **Aceitar (`Tab` ou clique):** preenche o cartão de Papel com nome do papel, frase humana e prompt por baixo. Mostra "por que sugeri" num tooltip ⓘ (13.3 item 8: "sempre mostrar por que o agente sugeriu").
- **Ajustar:** `[ Ver e ajustar instruções ▸ ]` expande o cartão num editor de **linguagem natural** — bullets editáveis ("O que ele faz", "O que ele nunca faz", "Tom"), com botão `▸ ver texto técnico completo` dobrado no rodapé para quem quiser o prompt cru.
- **Ignorar:** continuar digitando dispensa a sugestão sem fricção. O co-piloto re-sugere apenas se o nome mudar substancialmente (não a cada tecla).
- **Trocar:** abre a galeria de presets de papel (a galeria da Fase 0, embutida) — template-driven: duplicar + modificar, nunca partir do zero.

### M6-E3 — Criação

- `[[ Criar Agente ]]` → modal fecha, o nó nasce no canvas (na posição do clique-direito, ou em espaço livre perto do centro) com animação curta de "materialização", já dentro da aura do Time, com o estado amigável da Fase 0: *"Pronto. É só me dizer o que fazer."*
- O foco do teclado vai direto para o input do Agente recém-criado — o caminho até o primeiro prompt é zero cliques.
- `Cmd+Enter` dentro do modal = atalho de `[[ Criar Agente ]]` apenas se não houver Pedido pendente na Fila (precedência da Fila é absoluta — ver fluxo b).

### M6-E — modo Editar (Agente vivo)

> **M6-E — Tamanho:** M · **Dependências:** M6, F1-2-4 (editar terminal vivo / respawn), event-sourcing (Fase 0) · **Critério de aceite:** renomear um Agente vivo dispara re-sugestão de papel que só aplica com confirmação explícita, e a troca de motor só acontece no reinício anunciado — verificável na tela, nunca silencioso.

**Pontos de entrada:** duplo-clique no cabeçalho do nó · inspetor → `[ Editar ]` · `Cmd-K` → "Editar Agente…" · clique-direito no nó → "Editar".

Mesmo formulário do M6, com três diferenças de estado:

```
┌──────────────────────────────────────────────────────────────────────┐
│  Editar — Ajudante de Marketing                                 ✕    │
│  ● Trabalhando agora: «pesquisando concorrentes…»                    │
├──────────────────────────────────────────────────────────────────────┤
│  Motor          [ ◉ Claude Code ]   ⚠ trocar reinicia o Agente      │
│  Nome           ( Ajudante de Marketing_____ )                       │
│                 ↳ «O novo nome sugere outro papel: [Analista de      │
│                    Mercado]. Atualizar o papel também? [Sim] [Não]»  │
│  Papel          ┌─ Criador de Conteúdo ──────────────────────┐      │
│                 │ [ Ver e ajustar ▸ ]  [ Trocar ]  [ Salvar   │      │
│                 │                        como preset… ]       │      │
│                 └─────────────────────────────────────────────┘      │
│  Pasta          ( ~/EspaçosDeTrabalho/MeuNegocio )  [ Trocar… ]      │
│  ▸ Avançado                                                          │
│                                                                      │
│  Mudanças de nome/cor valem agora. Motor e instruções valem          │
│  no próximo reinício do Agente.  «selo: aplica ao reiniciar»         │
│                          [ Cancelar ]      [[ Salvar ]]              │
└──────────────────────────────────────────────────────────────────────┘
```

1. **Banner de estado vivo** no topo: o que o Agente está fazendo AGORA (governança visível — nunca editar às cegas algo que está trabalhando).
2. **Renomear re-resolve o papel** — o co-piloto roda de novo, mas **pergunta antes de trocar** («O novo nome sugere outro papel: […]. Atualizar o papel também?»). Nunca troca papel silenciosamente: o usuário pode ter renomeado por carinho, não por função.
3. **Trocar motor/instruções exige reinício** → diálogo claro com duas opções: `[[ Trocar quando ele terminar ]]` (default — agenda a troca para o fim da tarefa atual) e `[ Trocar agora (interrompe a tarefa) ]`. As mudanças pendentes ficam com selo «aplica ao reiniciar» visível no inspetor.
4. **Preset de prompt:** `[ Salvar como preset… ]` adiciona o papel atual à galeria (nome + descrição de 1 frase pedidos num mini-diálogo).

### Mapa de cliques (M6 / M6-E)

| # | Elemento | Gesto | Resultado |
|---|---|---|---|
| 1 | Chip de motor | clique | Seleciona motor; nome/comando por baixo se ajustam |
| 2 | `+ Instalar outro motor…` | clique | Expande lista dos não-instalados com `[ Instalar para mim ]` (mesmo subfluxo de T1, com a mesma janela de progresso humana) |
| 3 | Campo Nome | digitar + pausa | Linha-fantasma do co-piloto aparece |
| 4 | Linha-fantasma | `Tab` / clique | Aceita papel sugerido; cartão de Papel preenche |
| 5 | `Ver e ajustar instruções ▸` | clique | Expande editor em linguagem natural (bullets) |
| 6 | `▸ ver texto técnico completo` | clique | Mostra o prompt cru (read-write, monospace) |
| 7 | `[ Trocar ]` (papel) | clique | Galeria de presets embutida (busca + cards) |
| 8 | `[ Trocar… ]` (pasta) | clique | Seletor de pasta do SO |
| 9 | `▸ Avançado` | clique | Expande comando/flags/prompt/env/permissões |
| 10 | `[[ Criar Agente ]]` | clique / `Cmd+Enter`* | Cria nó, fecha modal, foco no input do Agente |
| 11 | `[ Cancelar ]` / `Esc` | clique / tecla | Fecha sem criar; se houver texto digitado, pergunta «Descartar este Agente?» |
| 12 | (M6-E) `[[ Salvar ]]` | clique | Aplica imediato o que pode; agenda o que exige reinício |
| 13 | (M6-E) `[ Trocar agora ]` | clique | Interrompe tarefa + reinicia com novo motor/prompt |

\* só sem Pedido pendente na Fila (fluxo b).

### Estados vazios

- **Nenhum motor detectado:** o quick-start não fica vazio nem culpa — vira o card da Fase 0: «Vamos preparar tudo para você» + `[[ Instalar o motor recomendado ]]` + link discreto "ver todos os motores". Idêntico a T1 — consistência reduz aprendizado.
- **Sem presets de papel na galeria:** a galeria mostra «Comece pelo nome — eu sugiro o papel pra você» apontando o campo Nome (o co-piloto É o caminho principal; preset é acelerador).

### Estados de erro

| Erro | Apresentação | Saída |
|---|---|---|
| Instalação de motor falha | «Não consegui instalar o [motor]. Sua internet pode ter oscilado.» — nunca stack trace | `[ Tentar de novo ]` `[ Escolher outro motor ]` |
| Pasta sem permissão de escrita | Inline, sob o campo: «Não consigo trabalhar nessa pasta.» | `[ Escolher outra ]` |
| Nome duplicado no Espaço | Sufixo automático "(2)" + aviso leve inline | Não bloqueia |
| Co-piloto indisponível (motor ocupado/sem rede) | Linha-fantasma silenciosamente ausente; campo Papel com placeholder «Escolha um papel da galeria ou descreva você mesmo» | Criação NUNCA bloqueia por causa do co-piloto |
| Motor desinstalado entre detecção e criação | Re-scan + volta ao quick-start: «O [motor] não está mais disponível.» | Escolher outro / instalar |
| (M6-E) Agente morreu durante a edição | Banner muda: «Este Agente parou. Suas mudanças valem quando ele voltar.» | `[[ Salvar ]]` continua funcionando |

### Princípios de design aplicados

- **13.3 achado 4 (Agent Skills / Skill-Based Editor):** "3-5 controles, não 20 campos"; "template-driven (duplicar + modificar) em vez de blank canvas"; "Claude co-piloto: sugerir papel/comando a partir de linguagem natural"; Progressive Disclosure em camadas (essencial → avançado → texto técnico).
- **13.3 item 8 (Express Intent → AI Executa):** o usuário descreve em linguagem natural (o nome), o Lina sugere o resto — com transparência do "por quê".
- **Doc 22, princípio-mestre:** uma decisão real por superfície (aqui: o papel); defaults inteligentes em tudo o mais.

### O que NÃO fazer (anti-padrões — superando o Maestri, print a print)

1. **NÃO mostrar YAML/frontmatter cru na superfície** (print "Nova responsabilidade": textarea com `name: backend-architect / description: …`). No Lina, instrução técnica só existe atrás de `▸ ver texto técnico completo`.
2. **NÃO expor "Comando: claude" como controle primário** (print "Detalhes"). O comando é consequência do motor escolhido — mora no Avançado.
3. **NÃO listar motores não-instalados como pares iguais** (print "Início Rápido": 5 ícones sem estado). Só o detectado é opção de 1 clique; o resto é "instalar".
4. **NÃO fragmentar o essencial em abas** (Detalhes/Aparência/Agente). Coluna única; o que não é essencial dobra, não vira aba.
5. **NÃO exibir notas de implementação na superfície** (print: «uma cópia das instruções será criada em uma pasta .maestri… Considere adicionar ao .gitignore»). Detalhe de implementação vai para tooltip ⓘ, em linguagem humana, se for mesmo necessário.
6. **NÃO usar controles sem explicação** (print: checkbox «Maestro 🪄» com ícone misterioso). Todo toggle tem 1 frase de consequência visível.
7. **NÃO pedir textarea em branco para "responsabilidade"** — blank canvas é o anti-padrão central (13.3 item 8: "Anti-padrão: forçar entender YAML. Pró: 2-3 controles + co-piloto").
8. **NÃO trocar papel silenciosamente ao renomear** — re-sugerir sim, decidir pelo usuário não.

---

## Fluxo (b) — TO + P5 · Fila de Atenção unificada (permissão + custódia)

### Objetivo

Quando um Agente trava esperando um yes/no (permissão) ou um envio precisa do gate de custódia, o usuário **fica sabendo sem ser interrompido**, entende **o que** está sendo pedido e **por quem**, e decide **vendo o estado real do terminal** — nunca às cegas. Arquitetura em 4 camadas do 13.13 (panorama): **detectar → sinalizar → contextualizar → agir**. Uma fila só para tudo que pede atenção humana; a custódia (gate Cmd+Enter existente) entra na mesma fila com precedência.

> **TO — Tamanho:** S · **Dependências:** F1-1-6 (detecção de permissão multi-camada), F1-1-1 (CliDetector) · **Critério de aceite:** um CLI real travado em yes/no gera toast com Agente + frase humana + comando exato em <2 s (meta do 13.13, validada por medição), e dispensá-lo nunca perde o Pedido — ele segue em P5.
>
> **P5 — Tamanho:** L · **Dependências:** TO, F1-1-6, F1-1-8 (ADR de segurança da injeção), custódia round5/Fase 0 (BrokerPump/CustodyDesk) · **Critério de aceite:** aprovar pela fila destrava um Claude real bloqueado em yes/no, com o estado atual do terminal visível antes do clique e a decisão auditável no `log.jsonl` (gate da onda F1-1).

### Anatomia (onde cada peça mora)

```
┌─ Barra superior ─────────────────────────────────────────────────────┐
│  MeuNegócio ▾        Tudo salvo ✓        R$ 4,20 hoje    🔔(2)       │
└──────────────────────────────────────────────────────────────────────┘
        canvas …                                    ┌─ P5 Fila ─────┐
                                                    │ (mailbox)     │
                                                    └───────────────┘
   ┌─ TO toast ──────────────────────────┐
   │ (canto inferior direito do canvas)  │
   └─────────────────────────────────────┘
```

- **TO (toast):** sinal transiente, canto inferior direito — não cobre nós nem o input do Agente em foco.
- **🔔 + badge:** contador persistente na barra superior; espelhado no ícone do dock (macOS) / taskbar.
- **P5 (mailbox):** painel lateral direito, lista persistente — nada some sem registro.

### Estados

**E0 — Tudo em dia.** Sino sem badge. Hover: tooltip «Tudo em dia ✓». Nenhum som, nenhum movimento. *Atenção sem ansiedade começa pelo silêncio do estado bom.*

**E1 — Pedido chega → TO (toast).**

```
┌──────────────────────────────────────────────────────┐
│ ⚙ Ajudante Dev  ·  pede permissão                    │
│ Quer enviar mudanças para a internet                 │
│ `git push origin master`                             │
│                                                      │
│ [[ Ver e decidir ]]          [ Depois ]   ▁▁▁▁▁▁ 6s  │
└──────────────────────────────────────────────────────┘
```

- **Conteúdo (contextualizar — 13.13 achado 1):** nome do Agente + tipo do pedido + **frase humana primeiro** («Quer enviar mudanças para a internet») + **comando exato em monospace menor** (transparência: segurança exige precisão, então o comando real é visível — mas nunca é a primeira linha). Jamais o genérico «Claude precisa da sua permissão».
- **Countdown 6 s com pause-on-hover** (13.13 achado 7); o toast some, **o item NÃO** — vai para P5 + badge.
- **Se a fila tem >1 item:** o countdown é desativado (13.13 achado 7: "o timer não é crítico e pode ser desativado se a fila tiver mais de um item") e o toast colapsa em «+N pedidos aguardando — [[ Ver fila ]]». Nunca empilhar toasts.
- **Custódia** chega pelo mesmo toast, com selo visual distinto (borda na cor de acento + rótulo «custódia»): «Mensagem para [Agente B] aguarda seu OK».

**E2 — Badge persistente.** 🔔(N) na barra; dock badge com N. O badge conta **Pedidos pendentes**, não notificações lidas/não-lidas — quando zera, é porque o usuário decidiu tudo.

**E3 — P5 aberto (mailbox).** `Cmd+J`, clique no sino, ou clique em «Ver e decidir».

```
┌─ Fila de Atenção ────────────────────────────── ✕ ──┐
│  [ Tudo (3) ]  [ Pedidos (2) ]  [ Problemas (1) ]   │
├──────────────────────────────────────────────────────┤
│ ▌custódia · Pesquisadora → Redator         há 10 s  │
│ ▌«Resumo da pesquisa pronto p/ redação»  [decidir ▾] │
├──────────────────────────────────────────────────────┤
│  permissão · Ajudante Dev                  há 2 min  │
│  Enviar mudanças para a internet         [decidir ▾] │
├──────────────────────────────────────────────────────┤
│  problema · Analista                       há 5 min  │
│  Parou com um erro                    [ver o quê ▾]  │
├──────────────────────────────────────────────────────┤
│  ⌛ expirado · Testador — não espera mais   [limpar]  │
├──────────────────────────────────────────────────────┤
│  Histórico de decisões ▸                             │
└──────────────────────────────────────────────────────┘
```

- **Abas Tudo / Pedidos / Problemas** — adaptação direta do padrão Warp All/Unread/Errors (13.13 achado 2), com rótulos humanos.
- **Ordem:** custódia acima de permissão, acima de gates custom (precedência do 13.13, backlog 4: «custódia externa > permissão > custom gates»); dentro da mesma classe, FIFO. Com múltiplos Agentes pedindo, o atendimento sugerido é round-robin entre Agentes para nenhum ficar esquecido (13.13 achado 6 — evitar starvation).
- Cada item: tipo (selo), Agente, resumo humano, idade («há 2 min»), e **estado do Agente AGORA** (mini-dot: esperando/trabalhando/morto).

**E4 — Decidir com segurança (o coração do fluxo).** Clique em `[decidir ▾]` expande o item — NÃO abre modal por cima do canvas:

```
├──────────────────────────────────────────────────────┤
│  permissão · Ajudante Dev                  há 2 min  │
│  Quer enviar mudanças para a internet                │
│  `git push origin master`                            │
│                                                      │
│  É isto que ele mostra NESTE momento:                │
│  ┌────────────────────────────────────────────────┐  │
│  │ $ git push origin master                       │  │
│  │ Push 3 commits to origin/master? (y/n) █       │  │ ← recorte VIVO
│  └────────────────────────────────────────────────┘  │
│  ✓ confere com o pedido                              │
│                                                      │
│  [[ ✓ Aprovar  Cmd+Enter ]]  [ ✗ Negar  Cmd+⇧Enter ] │
│  [ Ir até o Agente ]                                 │
├──────────────────────────────────────────────────────┤
```

- **Recorte vivo da tela do terminal** (espelho do VT, não screenshot velho): o usuário vê o que o Agente vê **agora**. Antes de habilitar os botões, o Lina valida que o prompt na tela ainda corresponde ao pedido registrado — a linha «✓ confere com o pedido» é essa validação visível (13.13 achado 5: a race de injeção `y/n` no PTY é risco real/CVE-documentado; a validação do estado da tela antes do write é a postura do Lina, registrada em ADR próprio).
- **Aprovar** injeta a resposta no PTY com chave idempotente estável `(sessão, pedido, momento)` — clicar duas vezes não injeta duas (13.13 achados 4 e 6).
- **`[ Ir até o Agente ]`** centra o canvas no nó e foca o terminal — para quem prefere decidir "lá dentro". Decidir no terminal resolve o item da fila automaticamente (detecção de que o prompt foi respondido).
- **Custódia (E4-c):** mesma anatomia, mas o recorte mostra a **mensagem** que vai de A para B (o gate Cmd+Enter existente) — aprovar = liberar o envio. O gesto `Cmd+Enter` é o mesmo de sempre; a Fila só dá a ele um endereço quando o usuário não está com o terminal em foco.

**E5 — Pedido mudou (stale/race).**

```
│  ⚠ A tela desse Agente mudou desde o pedido.         │
│  Veja o estado atual antes de decidir:               │
│  ┌ recorte vivo (diferente do pedido original) ┐     │
│  [[ Decidir pelo que vejo agora ]]  [ Ignorar ]      │
```

Os botões de aprovação re-validam contra a tela atual; o pedido antigo nunca é injetado contra um prompt novo. *Wrong approval por race é o pior erro possível do fluxo — preferimos um clique a mais.*

**E6 — Escalação (lembrete + som).** Pedido sem resposta:
- **30 s:** pulso suave no sino (1 ciclo, não loop) + o nó do Agente no canvas ganha contorno "esperando você" (âmbar suave).
- **Som opcional:** curto (~100 ms), no máximo 1× a cada 30 s e só se a fila > 1; OFF por padrão? — **ON discreto por padrão, mute em Ajustes (`Cmd+,` › Notificações)**, conforme 13.13 achado 9.
- **App em background:** notificação nativa do SO (macOS Notification Center) com o mesmo conteúdo contextualizado do toast.
- **N min sem resposta (SLA, default 15 min):** o item ganha selo «esperando há muito tempo»; **nunca auto-aprova**. Auto-negar só se o usuário ativou explicitamente nos Ajustes (13.13 backlog 5).

### Relação com o gate Cmd+Enter existente

| Situação | O que `Cmd+Enter` faz |
|---|---|
| Terminal com gate de custódia em foco | O que sempre fez: libera o envio (comportamento Fase 0, intocado) |
| Toast TO visível | Aprova o Pedido do toast (o toast é "foco implícito") |
| P5 aberto com item expandido | Aprova o item expandido |
| Nenhum dos acima + fila > 0 | **Abre a P5** com o item prioritário expandido — NUNCA aprova o que não está na tela |

`Esc` fecha toast/painel sem decidir. `Cmd+Shift+Enter` nega o item em foco. A regra de ouro: **nenhum gesto decide algo que o usuário não está vendo.**

### Mapa de cliques (TO / P5)

| # | Elemento | Gesto | Resultado |
|---|---|---|---|
| 1 | Toast TO | clique no corpo | Abre P5 com o item expandido (E4) |
| 2 | `[[ Ver e decidir ]]` | clique | Idem 1 |
| 3 | `[ Depois ]` | clique / `Esc` | Toast some; item permanece em P5 + badge |
| 4 | 🔔 sino | clique / `Cmd+J` | Abre/fecha P5 |
| 5 | Item da fila | clique / `[decidir ▾]` | Expande E4 (recorte vivo + botões) |
| 6 | `[[ ✓ Aprovar ]]` | clique / `Cmd+Enter` | Valida tela → injeta resposta → item vai pro histórico com selo ✓ |
| 7 | `[ ✗ Negar ]` | clique / `Cmd+⇧Enter` | Valida tela → injeta negação → histórico com selo ✗ |
| 8 | `[ Ir até o Agente ]` | clique | Centra canvas no nó + foca terminal |
| 9 | Item expirado `[limpar]` | clique | Remove da fila (vai pro histórico como "expirado") |
| 10 | `Histórico de decisões ▸` | clique | Lista de decisões passadas (audit trail do event-sourcing: quem pediu, o quê, decisão, quando) |
| 11 | «Não era um pedido» (item de fallback regex) | clique | Remove + marca como falso positivo (alimenta a calibragem do detector) |

### Estados vazios

- **P5 vazia:** ilustração calma + «Tudo em dia ✓ — seu time está trabalhando sem precisar de você.» + link `Histórico de decisões ▸`. O estado vazio é uma RECOMPENSA, não um vazio.
- **Histórico vazio:** «Quando seus Agentes pedirem algo, suas decisões ficam registradas aqui.»

### Estados de erro

| Erro | Apresentação | Saída |
|---|---|---|
| Agente morreu antes da decisão | Item vira «⌛ expirado — o Agente não está mais esperando» (cinza) | `[limpar]`; histórico registra "expirou" |
| Prompt mudou entre pedido e clique (race) | Estado E5 — bloqueia botões, mostra tela atual | «Decidir pelo que vejo agora» |
| Injeção falhou (PTY fechou no meio) | «Não consegui entregar sua resposta. O Agente mostra isto agora: [recorte]» | `[ Tentar de novo ]` `[ Ir até o Agente ]` |
| Falso positivo do detector (CLI sem hook, regex) | Item normal, mas com botão discreto «Não era um pedido» | Remove + feedback ao detector. **Nunca prometer zero falso positivo** (13.13 achado 8 — meta aspiracional, contradita pela issue #28174) |
| Som não pôde tocar (mute do SO) | Silencioso — badge e pulso visual já cobrem | — |
| Duas decisões simultâneas no mesmo item (duplo clique, 2 janelas) | Chave idempotente: a segunda é no-op com aviso sutil «já decidido ✓» | — |

### Princípios de design aplicados

- **13.13 panorama:** as 4 camadas detectar → sinalizar → contextualizar → agir; reúso da infra de custódia (BrokerPump/CustodyDesk) — a Fila é extensão, não sistema paralelo.
- **13.13 achado 2 (Warp):** toast + mailbox + badge é o trio validado de mercado; categoria única "Request" para bloqueios.
- **13.13 achado 7:** countdown 6 s, pause-on-hover, desativar timer com fila > 1.
- **13.13 achado 9:** escalação som curto + dock badge + bell, com mute.
- **13.13 achados 5/6 + backlog 3/5:** validação do estado real antes da injeção; idempotência por chave estável; precedência custódia > permissão; SLA sem auto-aprovação.
- **13.3 achado 3:** 93% dos prompts são aprovados — o desenho reduz o CUSTO de cada aprovação (1 clique, contexto à vista) em vez de multiplicar avisos; boundaries/sandboxing continuam sendo a defesa primária, a Fila é a janela humana sobre eles.
- **Doc 22:** zero jargão (frase humana primeiro, comando em segundo plano); estado sempre visível (badge = pendências reais).

### O que NÃO fazer (anti-padrões)

1. **NÃO usar modal bloqueante central** para pedido de permissão — é a materialização do modal fatigue (13.3 achado 3). O canvas nunca é sequestrado.
2. **NÃO notificar sem contexto** («Claude precisa da sua permissão» — exatamente o payload genérico criticado no 13.13 achado 1). Sempre: quem + o quê + comando exato.
3. **NÃO deixar toast sumir sem rastro** — todo sinal transiente tem um registro persistente (P5 + histórico).
4. **NÃO auto-aprovar por padrão**, nem oferecer "aprovar tudo sempre" como botão de 1 clique no toast. Delegação progressiva é conversa para Ajustes, com escopo explícito.
5. **NÃO aprovar às cegas por atalho global** — `Cmd+Enter` sem item visível abre a fila, não decide.
6. **NÃO empilhar N toasts** — colapsar em contador.
7. **NÃO usar som de alarme/loop** — som é convite, não sirene (atenção sem ansiedade).
8. **NÃO esconder o comando real** atrás só da frase humana — segurança pede transparência literal além da tradução.
9. **NÃO tratar custódia e permissão como sistemas separados** com duas filas e dois badges — uma fila, uma precedência, um gesto.

---

## Fluxo (c) — P6 · Painel do Agente + B1 · Medidor do Espaço (observabilidade)

### Objetivo

Um empreendedor não-técnico olha e entende **em 5 segundos**: o que cada Agente está fazendo, quanto está custando, e se algo precisa dele. Hierarquia de leitura fixa: **1) estado (palavra + cor) → 2) custo de hoje (dinheiro, não tokens) → 3) atividade**. Dinheiro é a língua universal do empreendedor; token é jargão e vai para o tooltip.

### Vocabulário de estados (fixo, 5 palavras, sempre as mesmas)

| Estado na UI | Cor | O que é por baixo | Ação oferecida |
|---|---|---|---|
| **● Trabalhando** | verde | Processo ativo, output fluindo | — (deixe ele trabalhar) |
| **● Esperando você** | âmbar | Bloqueado em permissão/custódia | Link direto pro Pedido na Fila (fluxo b) |
| **● Ocioso** | cinza | Vivo, sem tarefa | «pronto para a próxima tarefa» |
| **● Sem resposta** | vermelho suave | Hung real (sem output nem prompt há X min) | `[ Espiar ]` `[ Reiniciar ]` |
| **● Dormindo** | azul escuro | Pausado pelo usuário/teto | `[ Acordar ]` |

O estado aparece em **3 lugares com a mesma palavra e cor** (consistência absoluta): no nó do canvas (periferia, como já desenhado na Fase 0), no P6, e nos itens da Fila. *«Esperando você» substitui «travado» na superfície — descreve o que fazer, não o que temer; «Sem resposta» fica reservado ao hang real.*

### P6 — Painel do Agente

> **P6 — Tamanho:** M · **Dependências:** F1-1-1 (badge do motor), F1-1-2 (sessões/tokens), F1-1-5 (dashboard vivo), inspetor (Fase 0) · **Critério de aceite:** diante de um terminal em atividade real, um leigo responde "o que ele está fazendo e quanto custou hoje" em ≤5 segundos só com o inspetor aberto — validado na tela pelo fundador (lição da Fase 0: visibilidade ≠ lógica).

**Pontos de entrada:** clique no nó → inspetor (existente, Fase 0) → seção **Atividade** no topo · `Cmd-K` → "Atividade do [nome]" · clique no valor de custo no cabeçalho do nó.

```
┌─ Inspetor — Ajudante Dev ────────────────────────────┐
│  ● Trabalhando — «rodando os testes do app»          │
│  ⚙ Claude Code «motor»          desde 14:02 (1h 12m) │
├──────────────────────────────────────────────────────┤
│  Hoje                                                │
│  R$ 3,10 ~estimado ⓘ            ▂▃▅▂▇▃▁▂ última hora │
│                                                      │
│  O que ele fez:                                      │
│   14:58  terminou os testes — tudo verde             │
│   14:31  pediu sua permissão (você aprovou ✓)        │
│   14:05  começou a tarefa «arrumar o cadastro»       │
│   ver tudo ▸                                         │
├──────────────────────────────────────────────────────┤
│  ▸ Detalhes técnicos (tokens, sessão, comando)       │
└──────────────────────────────────────────────────────┘
```

- **Linha 1 — estado + o que está fazendo agora** (última ação humanizada, derivada do event-sourcing).
- **Badge do motor:** chip com ícone + nome («Claude Code») — qual motor roda é informação de governança, sempre visível, nunca escondida.
- **Custo de hoje:** `R$ 3,10 ~estimado` — o `~` e a palavra «estimado» são obrigatórios; o ⓘ explica em 1 frase: «calculado pelo uso de IA deste Agente × preço do motor. O valor exato aparece na fatura do seu provedor.»
- **Sparkline de atividade** (última hora) — forma, não números: dá a sensação de "ele está vivo e produzindo" num relance.
- **Linha do tempo humana:** eventos do event-sourcing traduzidos («terminou os testes», «pediu sua permissão»), com decisões da Fila integradas (governança visível: a decisão que você tomou aparece na história do Agente).
- **`▸ Detalhes técnicos` (dobrado):** tokens de entrada/saída, id de sessão, comando completo, uptime preciso — divulgação progressiva para quem quiser.
- **Atualização por delta** — números e sparkline atualizam suavemente, sem repaint/reload jarring (13.3 item 7).

### B1 — Medidor do Espaço (barra superior)

> **B1 — Tamanho:** S · **Dependências:** P6/F1-1-5 (agregação de custo), P5 (item de teto na Fila) · **Critério de aceite:** atingir o teto pausa o time suavemente e gera item na Fila de Atenção, e o total do dia na barra confere com a soma do ranking por Agente.

Mora na barra superior, ao lado de «Tudo salvo ✓»: um número discreto e clicável.

```
barra:   MeuNegócio ▾      Tudo salvo ✓      [ R$ 4,20 hoje ]      🔔
                                                  │ clique
                                                  ▼
┌─ Espaço: MeuNegócio — hoje ──────────────────────────┐
│  Total      R$ 4,20 ~estimado                        │
│  ▓▓▓▓▓▓▓░░░░░░░░░░░  42% do teto (R$ 10,00/dia)      │
│                                                      │
│  Por Agente:                                         │
│   Ajudante Dev        R$ 3,10  ●Trabalhando          │
│   Pesquisadora        R$ 0,90  ●Ociosa               │
│   Redator             R$ 0,20  ●Dormindo             │
│                                                      │
│  [ Definir teto… ]        [[ ⏸ Pausar todo o time ]] │
│  Este mês: R$ 61,30 · ver histórico ▸                │
└──────────────────────────────────────────────────────┘
```

- **Teto (orçamento):** barra de progresso visual; definir teto é opcional (default: sem teto, com sugestão gentil na primeira vez que o dia passa de um valor notável — ver Dúvidas).
- **Ao atingir o teto:** o time é pausado automaticamente (estado «Dormindo» em todos) **e** entra um item na Fila de Atenção: «Seu teto de R$ 10,00 foi atingido — pausei o time. [[ Aumentar o teto ]] [ Manter pausado ]». Governança: o app age sozinho para proteger o bolso, mas avisa imediatamente e a decisão de retomar é humana.
- **`⏸ Pausar todo o time`:** pausa SUAVE — cada Agente recebe o sinal e para de aceitar tarefa nova; o que está no meio de uma resposta conclui a frase atual (segundos), nunca kill -9 na cara do usuário. O botão vira `[[ ▶ Retomar o time ]]`.
- **Ranking por Agente** com estado ao lado — liga custo a comportamento num olhar («quem gasta é quem trabalha — ok» ou «por que o ocioso gastou?»).

### Mapa de cliques (P6 / B1)

| # | Elemento | Gesto | Resultado |
|---|---|---|---|
| 1 | Nó do Agente | clique | Inspetor abre com seção Atividade (P6) no topo |
| 2 | «Esperando você» (estado, em qualquer lugar) | clique | Abre P5 com o Pedido desse Agente expandido |
| 3 | `[ Espiar ]` (Sem resposta) | clique | Centra canvas + foca o terminal (ver com os próprios olhos) |
| 4 | `[ Reiniciar ]` | clique | Confirmação 1-clique «Reiniciar o [nome]? A tarefa atual será perdida.» → reinicia |
| 5 | `ver tudo ▸` (linha do tempo) | clique | Linha do tempo completa do Agente (scroll, busca) |
| 6 | `▸ Detalhes técnicos` | clique | Expande tokens/sessão/comando |
| 7 | `[ R$ 4,20 hoje ]` (B1) | clique | Popover do Medidor do Espaço |
| 8 | Linha de Agente no ranking | clique | Fecha popover + abre P6 daquele Agente |
| 9 | `[ Definir teto… ]` | clique | Mini-form: valor por dia OU por mês, 1 campo + confirmar |
| 10 | `[[ ⏸ Pausar todo o time ]]` | clique | Pausa suave de todos + botão vira Retomar |
| 11 | `ver histórico ▸` | clique | Vista mensal (por dia, por Agente) — leitura, sem edição |

### Estados vazios

- **Agente recém-criado:** «Ainda nada por aqui. Peça algo a ele que eu começo a medir.» — sparkline plana com convite, não vazio constrangedor.
- **B1 zerado (dia novo):** `R$ 0,00 hoje` — o zero é informação boa, mostra que o medidor está vivo.
- **Histórico mensal vazio (primeira semana):** «Seu primeiro mês está sendo medido — volte aqui em alguns dias.»

### Estados de erro

| Erro | Apresentação | Saída |
|---|---|---|
| Motor não reporta uso (CLI sem dados de tokens) | «custo não disponível para este motor ⓘ» — NUNCA inventar número | ⓘ explica quais motores reportam |
| Estimativa divergente do real | Mitigado por design: tudo é `~estimado`, com ⓘ apontando para a fatura do provedor como fonte da verdade | — |
| Crash do app | Linha do tempo e custos sobrevivem (event-sourcing é a fonte; replay no boot) — o usuário reabre e vê o dia inteiro | — |
| Teto atingido durante tarefa crítica | A pausa é suave (conclui a frase) + item na Fila com retomada de 1 clique | «Aumentar o teto» |
| Relógio do sistema mudou (fuso/ajuste) | Agregação diária pelo dia local; mudanças geram no máximo um dia com nota «dia ajustado» | — |

### Princípios de design aplicados

- **13.3 item 7 (Observabilidade de sessões):** «dashboard real-time por terminal via delta patches (sem reloads jarring)»; «integrar com o event-sourcing como source-of-truth (logs imutáveis, replay)»; observabilidade completa como **diferencial PRO** (ver fluxo e — no Free, P6 mostra estado + atividade; custo/teto/histórico completo é PRO).
- **13.3 achado 2 (transparência):** «se o usuário não entende por que algo foi sugerido/feito, a confiança quebra» — a linha do tempo humana é a resposta direta.
- **Doc 22 (glossário + princípio 3):** zero jargão (dinheiro primeiro, token no dobrado); estado sempre salvo e visível.
- **Honestidade epistêmica (13.3, nota de calibragem):** custos são estimativas — a UI assume isso com `~` em vez de fingir precisão.

### O que NÃO fazer (anti-padrões)

1. **NÃO fazer dashboard de engenheiro** — latência p99, gráficos de barras múltiplos, grids de métricas. Um número, uma palavra, uma forma (sparkline).
2. **NÃO usar token como métrica primária** — «1,2M tokens» não significa nada para o público; R$ significa tudo. Token mora no `▸ Detalhes técnicos`.
3. **NÃO alarmar por custo normal** — vermelho piscando porque gastou R$ 5 cria a ansiedade que o produto promete eliminar. Vermelho é só para «Sem resposta» e teto estourado.
4. **NÃO esconder o custo até doer** — o medidor está SEMPRE na barra, pequeno e honesto, antes de virar problema.
5. **NÃO pausar matando processos** — pausa suave com conclusão da frase; «Pausar» nunca pode significar «perder trabalho».
6. **NÃO fingir precisão** — sem `~estimado`, o primeiro choque com a fatura real destrói a confiança no medidor inteiro.
7. **NÃO criar uma janela/tela separada de "dashboard"** — observabilidade mora onde o trabalho mora: no nó, no inspetor, na barra.

---

## Fluxo (d) — T7§A · Ajustes › Aparência (tema + personalização)

### Objetivo

Trocar entre escuro/claro e escolher a cor de acento (bordas, botões, seleções) **vendo o efeito ao vivo no próprio app**, sem botão "Salvar", sem flash, sem jargão de design system. A personalização é curada — liberdade dentro de limites que preservam a identidade anti-slop do Lina.

> **T7§A — Tamanho:** S · **Dependências:** F1-2-1 (design system tokenizado — `Theme` structs/ColorScaleSet) · **Critério de aceite:** trocar modo ou acento atualiza o app inteiro ao vivo, sem flash perceptível e sem botão Salvar, e a escolha sobrevive a fechar e reabrir o app.

### Pontos de entrada

| De onde | Gesto |
|---|---|
| Teclado | `Cmd+,` → T7 Ajustes → seção **Aparência** |
| Paleta | `Cmd-K` → "tema", "cor", "aparência", "modo claro/escuro" |
| Barra superior | menu do app → Ajustes |

### T7§A-E1 — A seção

```
┌─ Ajustes ────────────────────────────────────────────────────────┐
│  Geral                                                           │
│ ▸Aparência   ┌────────────────────────────────────────────────┐  │
│  Notificações│  Modo                                          │  │
│  Plano       │  ┌─────────┐ ┌─────────┐ ┌──────────────┐      │  │
│  Avançado    │  │ ▣ Escuro│ │ ☐ Claro │ │ ◐ Automático │      │  │
│              │  │ (atual) │ │         │ │ «segue o Mac»│      │  │
│              │  └─────────┘ └─────────┘ └──────────────┘      │  │
│              │   ↳ cards com miniatura do canvas, não dropdown│  │
│              │                                                │  │
│              │  Cor de acento                                 │  │
│              │   ● ● ● ● ● ● ● ●        «8 cores curadas»     │  │
│              │   ▲ a pastilha ativa pulsa 1x ao trocar        │  │
│              │                                                │  │
│              │  Veja como fica:                               │  │
│              │  ┌──────────────────────────────────────────┐  │  │
│              │  │  ┌─Agente─┐   [[ Botão ]]  ▌seleção      │  │  │
│              │  │  │ ● nome │   ░░ borda ░░                │  │  │
│              │  │  └────────┘                              │  │  │
│              │  └──────────────────────────────────────────┘  │  │
│              │   ↳ mini-canvas de amostra — mas o app TODO    │  │
│              │     já mudou junto (o ajuste é ao vivo)        │  │
│              │                                                │  │
│              │  ▸ Avançado: exportar/importar tema (arquivo)  │  │
│              └────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

- **Modo como cards visuais com miniatura** (não dropdown) — escolher tema é decisão visual, o controle deve ser visual. «Automático» segue o SO, com a frase «segue o Mac».
- **Aplicação imediata, ao vivo, no app inteiro** — clicou, mudou (tokens centralizados em `Theme` structs do GPUI: troca global sem flash — 13.3 achado 5). Nada de botão Aplicar/Salvar; aparece um toast discreto «Aparência atualizada · [ Desfazer ]» por 6 s.
- **Mini-canvas de amostra dentro da seção** — concentra num retângulo os elementos onde o acento aparece (borda de nó, botão primário, seleção), para o usuário ver O QUE a cor de acento governa sem caçar pelo app.
- **8 cores de acento curadas** — paleta desenhada para funcionar nos dois modos (stepping consistente light↔dark via ColorScaleSet — 13.3 achado 5) e dentro da identidade anti-slop. Roda de cor livre NÃO fica na superfície (ver anti-padrões); um seletor fino pode morar no `▸ Avançado`.
- **`▸ Avançado`:** exportar tema (arquivo local — formato JSON em `~/.lina/`, local-first, 13.3 item 1), importar tema, ajustes finos (overrides pontuais). Quem nunca abrir vive feliz com os 8 acentos.

### Mapa de cliques

| # | Elemento | Gesto | Resultado |
|---|---|---|---|
| 1 | Card de modo | clique | App inteiro troca na hora, sem flash; toast «Desfazer» |
| 2 | Pastilha de acento | clique | Acento troca ao vivo (bordas/botões/seleção); toast «Desfazer» |
| 3 | `[ Desfazer ]` (toast) | clique | Volta ao estado anterior (1 nível) |
| 4 | `▸ Avançado` | clique | Expande exportar/importar/ajustes finos |
| 5 | `Exportar tema` | clique | Salva arquivo `.json` onde o usuário escolher |
| 6 | `Importar tema` | clique | Seletor de arquivo → valida → aplica ao vivo com «Desfazer» |
| 7 | `Esc` / fechar Ajustes | tecla / clique | Fecha — tudo já está salvo (estado sempre salvo, doc 22) |

### Estados vazios

Não há estado vazio real — os defaults da Fase 0 (Dark OLED + acento padrão) estão sempre presentes. O único quase-vazio: nunca importou tema → a área de import mostra «Temas que você importar aparecem aqui».

### Estados de erro

| Erro | Apresentação | Saída |
|---|---|---|
| Acento com contraste insuficiente no modo atual | Aviso inline na pastilha (não bloqueio): «essa cor fica difícil de ler no modo claro» | `[ Usar mesmo assim ]` `[ Ver parecida que funciona ]` (sugere a vizinha acessível) |
| Arquivo de tema inválido/corrompido | «Este arquivo não parece um tema do Lina.» | `[ Escolher outro arquivo ]` |
| Tema importado de versão futura (campos desconhecidos) | Aplica o que entende + nota «este tema veio de uma versão mais nova — alguns detalhes foram ignorados» | — |
| Falha ao persistir preferência (disco) | Tema segue aplicado em memória + aviso «não consegui salvar sua escolha — ela pode se perder ao fechar» | `[ Tentar de novo ]` |

### Princípios de design aplicados

- **13.3 achado 5 (Design Systems Rust/GPU):** «centralizar tokens em `Theme` structs — nunca cores hard-coded»; «dark/light via ColorScaleSet com stepping consistente»; «personalização via theme_overrides + export JSON local-first»; «live preview ao mudar» sem o «janky flash da web». O *right-click inspector* do Zed (200+ tokens nomeados) é referência para a EQUIPE construir o design system — não vira UI para o leigo.
- **13.3 item 1 (P0):** menor risco, maior alavancagem — o design system tokenizado é a fundação que torna este fluxo trivial de implementar e impossível de quebrar.
- **Doc 22 princípio 3:** estado sempre salvo — por isso não existe botão Salvar; existe Desfazer.

### O que NÃO fazer (anti-padrões)

1. **NÃO expor 200 tokens ao leigo** — isso é ferramenta de quem constrói tema (Zed/devs), não de quem usa o Lina. A superfície tem 2 decisões: modo e acento.
2. **NÃO usar botão Aplicar/Salvar** — fricção desnecessária que contradiz «Tudo salvo ✓»; preview ao vivo + Desfazer cobre o arrependimento.
3. **NÃO permitir flash/reload na troca** — o "janky flash" quebra a sensação de app nativo premium (13.3 achado 5); se a engenharia não garantir troca suave, é bug, não trade-off.
4. **NÃO oferecer roda de cor livre na superfície** — acento aleatório quebra contraste, acessibilidade e a identidade anti-slop. Curadoria primeiro; liberdade no Avançado, com aviso de contraste.
5. **NÃO esconder o modo escuro/claro em submenu profundo** — é a personalização mais desejada; primeira coisa da seção, acessível via paleta.
6. **NÃO acoplar tema a conta/nuvem** — preferência é local-first (arquivo em `~/.lina/`), exportável como arquivo, na filosofia do produto.

---

## Fluxo (e) — M8 · Switcher de Espaços + M9 · Criar Espaço + T7§P · Ativação PRO

### Objetivo

Trocar de Espaço com a mesma naturalidade de trocar de aba — sabendo de relance o que acontece em cada um (Agentes ativos, custo, pendências) —, criar um Espaço novo sem reaprender nada (reusa a Galeria de Focos do onboarding), e ativar o PRO colando uma chave com feedback inequívoco do que aconteceu e do que destravou.

### M8 — Switcher de Espaços

> **M8 — Tamanho:** S · **Dependências:** F1-4-1 (multi-workspace real), F1-4-4 (evolui o T6/W4-4 da Fase 0), F1-4-5 (vitrine do gating) · **Critério de aceite:** alternar entre 2 Espaços com terminais vivos restaura cada canvas como deixado e prova isolamento — mensagem broadcast de A não aparece em B (gate da onda F1-4).

**Pontos de entrada:** clique no nome do Espaço (barra superior, à esquerda) · `Cmd+O` · `Cmd+1…9` (troca direta) · `Cmd-K` → "Trocar para [nome]".

```
barra:  [ MeuNegócio ▾ ]   Tudo salvo ✓   R$ 4,20 hoje   🔔(1)
            │ clique
            ▼
┌─ Espaços ───────────────────────────────────────────┐
│  ( buscar… )                                        │
│                                                     │
│  ▣ MeuNegócio            3 Agentes ●  R$ 4,20   ⌘1  │
│  ☐ Loja Virtual          2 Agentes ●  R$ 1,10 🔔 ⌘2 │
│  ☐ Projetos Pessoais     dormindo     R$ 0,00   ⌘3  │
│                                                     │
│  + Novo Espaço…                                ⌘⇧N  │
└─────────────────────────────────────────────────────┘
```

- **Cada linha é um mini-status:** N Agentes + dot do estado dominante, custo de hoje, e **🔔 se há Pedido pendente naquele Espaço** — governança atravessa Espaços: nada fica esquecido num workspace de fundo.
- **Buscar digitando** (o dropdown abre com foco no campo); `↑↓` navega, `Enter` troca.
- **A troca é instantânea e segura:** tudo é event-sourced («Tudo salvo ✓»), nada para confirmar. Os Agentes do Espaço que você deixou **continuam no estado em que estavam** (ver Dúvidas — manter trabalhando vs. adormecer é decisão de produto/perf em aberto).
- **Pedido de outro Espaço:** entra na mesma Fila de Atenção com prefixo do Espaço («Loja Virtual · Ajudante pede permissão») — aprovar dali NÃO exige trocar de Espaço (o recorte vivo do fluxo b funciona cross-Espaço).
- **Free = 1 Espaço (decisão do fundador, 2026-06-06):** para usuário free com o Espaço único criado, o item `+ Novo Espaço…` carrega um selo «PRO» — o limite aparece ANTES do clique, e o switcher faz o papel de vitrine honesta do gating (F1-4-4: «o switcher é também vitrine do gating»). O wireframe acima, com 3 Espaços, retrata um usuário PRO.

### M9 — Criar novo Espaço

> **M9 — Tamanho:** S · **Dependências:** T3/Galeria de Focos (Fase 0), F1-4-1, F1-4-5 (gating free=1 no fluxo de criação) · **Critério de aceite:** usuário free com seu Espaço único vê o selo/limite ANTES do clique, e usuário PRO cria o 2º Espaço já povoado pelo Foco — nunca tela em branco.

**Pontos de entrada:** `+ Novo Espaço…` no M8 · `Cmd+Shift+N` · `Cmd-K` → "Novo Espaço".

Reusa **T3 — Galeria de Focos** (Fase 0) em modal, idêntica à do onboarding: cards de Foco («Construir um App», «Criar Landing Page», «Pesquisa & Conteúdo», «Automatizar Rotinas», «Espaço em Branco»), nome pré-preenchido, pasta sugerida em `~/EspaçosDeTrabalho/<nome>`. Uma decisão real (o Foco); o resto é default.

- Ao criar: troca para o novo Espaço já povoado pelos Agentes do Foco (nunca tela em branco) — o mesmo aterrissar de T4.
- **Free vs. PRO (decisão do fundador, 2026-06-06: Free = 1 Espaço):** o plano Free inclui **1 Espaço**; com ele criado, o card de criação mostra o estado ANTES do clique (ver anti-padrões): «Você já usa o Espaço do plano Free (1 de 1) · [ Conhecer o PRO ]» — sem deixar montar tudo para negar no fim. O gating é o da F1-4-5 (`workspace_limit` assinado na chave; PRO = N) e o bloqueio é gracioso: nada trava, a saída é clara.

### T7§P — Ajustes › Plano (ativação PRO)

> **T7§P — Tamanho:** S · **Dependências:** F1-4-5 (`lina-license`, ed25519); esta superfície é o wireframe/copy da F1-4-6 · **Critério de aceite:** colar uma chave válida ativa o PRO com a lista do que mudou, com um monitor de rede provando **zero conexão** durante todo o ciclo (gate da onda F1-4).

**Pontos de entrada:** `Cmd+,` → Plano · `Cmd-K` → "ativar PRO", "licença", "plano" · link «Conhecer o PRO» em qualquer limite do Free.

**E1 — Estado Free (antes de ativar):**

```
┌─ Plano ──────────────────────────────────────────────────────┐
│  Seu plano: Lina Free                                        │
│                                                              │
│  O que você tem hoje:        Com o Lina PRO você destrava:   │
│   ✓ 1 Espaço com seu          ★ Espaços: crie quantos        │
│     time completo               precisar                     │
│   ✓ estado e atividade        ★ Medidor completo: custo por  │
│     dos Agentes                 Agente, teto, histórico      │
│                               ★ (demais itens conforme       │
│                                  pricing — ADR 0011)         │
│                                                              │
│  Já tem uma chave?                                           │
│  ( cole aqui a sua Chave do Lina PRO_______________ )        │
│                                       [[ Ativar ]]          │
│                                                              │
│  Ainda não tem? [ Conhecer o Lina PRO ↗ ]  «abre o site»     │
└──────────────────────────────────────────────────────────────┘
```

- **Duas colunas honestas:** o que você TEM (sem diminuir o Free) e o que destrava — concreto, sem marketês («Espaços ilimitados», não «produtividade infinita»).
- **Campo único de chave:** aceita colar com espaços/quebras de linha acidentais (trim e normalização silenciosos — colar de e-mail nunca falha por formatação).

**E2 — Verificando (100% offline):** a chave é conferida **no próprio computador, na hora** — assinatura ed25519 verificada contra a chave pública embarcada no app (F1-4-5; 13.6 ação #1). **Nenhuma conexão de rede acontece, nunca** — o gate da onda F1-4 prova isso com monitor de rede durante todo o ciclo (criação, gating, ativação). A verificação é instantânea (<100 ms); um «verificando…» de segundos seria bug, não rede.

**E3 — Sucesso:**

```
│  ✓ Lina PRO ativo — obrigado!                                │
│                                                              │
│  O que mudou agora:                                          │
│   ★ Espaços: crie quantos precisar                           │
│   ★ Medidor completo ativado                                 │
│                                                              │
│  Válida até 06/2027                                          │
│                                       [[ Continuar ]]        │
```

Celebração discreta (sem confete invasivo) e lista do que mudou **agora** (não promessas) — **gerada dos entitlements da própria chave** (gating data-driven da F1-4-5: `workspace_limit` e afins vêm assinados dentro da chave, nunca hard-coded na UI). O único fato administrativo exibido é a validade (o `expiry` também vive dentro da chave assinada — lido localmente). Um selo `PRO` pequeno passa a existir em Ajustes › Plano — e em nenhum outro lugar (anti-slop: o app não vira outdoor do próprio plano).

**E4 — Falhas específicas** (cada erro com causa + ação, nunca genérico — e **todas verificáveis offline**, porque assinatura e `expiry` vivem dentro da própria chave):

| O que houve | Mensagem | Saída |
|---|---|---|
| Chave incompleta ou incorreta (malformada OU assinatura ed25519 não confere — indistinguíveis para o leigo, mesma ação) | «Essa chave não parece válida — confira se copiou tudo, incluindo os traços.» | campo mantém o texto para corrigir |
| Chave expirada (o `expiry` está assinado dentro da chave — lido localmente) | «Essa chave expirou em 03/2026.» | `[ Renovar no site ↗ ]` (abre o navegador; o app segue sem tocar a rede) |

**Estados que NÃO existem nesta tela, por decisão de arquitetura** (gate 100% offline da onda F1-4; ADR 0011; 13.6 — sem phone-home, sem node-locking na F1, sem revogação/blocklist): «chave já ativa em outro computador», «sem internet», «chave revogada». Se um estado novo de licença exigir rede, ele está errado por definição — volta para o ADR antes de virar UI.

**Pós-ativação — degradação graciosa (F1-4-5):** se o arquivo de licença for corrompido/adulterado depois de ativo, o app **volta a free graciosamente**: nunca trava, Espaços existentes além do limite **continuam abrindo** (o que bloqueia é criar novos), e Ajustes › Plano mostra «sua chave precisa ser colada de novo» — re-colar resolve.

### Mapa de cliques (M8 / M9 / T7§P)

| # | Elemento | Gesto | Resultado |
|---|---|---|---|
| 1 | Nome do Espaço (barra) | clique / `Cmd+O` | Abre M8 com foco na busca |
| 2 | Linha de Espaço | clique / `Enter` / `Cmd+1…9` | Troca instantânea; canvas do destino restaurado como deixado |
| 3 | 🔔 na linha | clique | Troca + abre P5 já no Pedido daquele Espaço |
| 4 | `+ Novo Espaço…` | clique / `Cmd+Shift+N` | Abre M9 (Galeria de Focos em modal) |
| 5 | Card de Foco (M9) | clique | Seleciona; nome/pasta pré-preenchem |
| 6 | `[[ Criar Espaço ]]` (M9) | clique | Cria + troca + aterrissa povoado (T4) |
| 7 | Campo de chave (T7§P) | colar | Normaliza espaços/quebras silenciosamente |
| 8 | `[[ Ativar ]]` | clique / `Enter` | E2 verificação local (offline) → E3 sucesso ou E4 falha específica |
| 9 | `[ Conhecer o Lina PRO ↗ ]` | clique | Abre o site no navegador (nunca webview embutida de checkout) |
| 10 | `[[ Continuar ]]` (E3) | clique | Volta de onde veio (se veio de um limite, o limite já está destravado) |

### Estados vazios

- **Só um Espaço:** M8 mostra o atual + CTA com benefício concreto: «Crie um segundo Espaço para separar este projeto do resto — cada um com seu time.» Para usuário free, o CTA carrega o selo «PRO» (Free = 1 Espaço) — o switcher se apresenta antes de ser necessário e é honesto sobre o que destrava.
- **Busca sem resultado (M8):** «Nenhum Espaço com esse nome. [ + Criar "[texto]" ]» — o erro de busca vira atalho de criação.

### Estados de erro

| Erro | Apresentação | Saída |
|---|---|---|
| Pasta do Espaço sumiu (drive externo, renomeada) | Linha do M8 com ⚠ «pasta não encontrada»; ao trocar: tela de reconexão «A pasta deste Espaço não está acessível.» | `[ Localizar pasta… ]` `[ Voltar ]` — nunca abre canvas quebrado |
| Espaço corrompido (recuperação W0-6 da Fase 0) | O fluxo de recuperação existente assume; o M8 não esconde o Espaço | conforme Fase 0 |
| Criação falha (disco cheio, permissão) | No M9, inline: «Não consegui criar a pasta do Espaço.» | `[ Escolher outro lugar ]` |
| Limite Free atingido | Estado mostrado ANTES de montar (no card/CTA), nunca após o esforço | `[ Conhecer o PRO ]` |

### Princípios de design aplicados

- **13.3 item 9 (Multi-tenant workspace + tiers):** «licensing local-first (`~/.lina/workspaces/`)»; «Free: N terminais / M skills, sem observabilidade avançada; PRO: ilimitado + observabilidade full»; e a ressalva de mercado — 2026 é *Trough of Disillusionment*: «vender pragmatismo e resultado, não hype» → a tela PRO lista destravas concretas, não promessas.
- **13.6 + ADR 0011 + gate da onda F1-4 (ondas-2-4.md):** a ativação é **assinatura ed25519 verificada 100% localmente**, com chave pública embarcada no binário — sem servidor de contas, sem cadastro, sem phone-home, sem node-locking, sem revogação na F1 («sem assinatura, o feature-gating é teatro»; o offline puro é **diferencial deliberado**, não limitação). Esta tela é a face não-técnica dessa decisão: nenhum estado dela pode depender de rede.
- **13.3 achado 4 (template-driven):** criar Espaço = duplicar um Foco, nunca blank canvas — por isso M9 É a T3, não uma tela nova.
- **13.13 (fila unificada):** Pedidos cross-Espaço entram na mesma fila com contexto — governança não tem fronteira de workspace.
- **Doc 22:** uma decisão real por superfície (M9: o Foco; T7§P: colar a chave); nunca tela em branco (Espaço novo nasce povoado).

### O que NÃO fazer (anti-padrões)

1. **NÃO fazer paywall surpresa** — limite do Free aparece ANTES do esforço do usuário (no card, no CTA), nunca como tapa depois de preencher tudo.
2. **NÃO usar nag screens** — o PRO existe em Ajustes › Plano e nos pontos de limite real; zero modais periódicos «faça upgrade!».
3. **NÃO «fale com vendas»** — o caminho é chave → colar → ativar; humano só no suporte de erro.
4. **NÃO erros de licença com jargão ou culpa** — «HTTP 403», «assinatura inválida», «você inseriu errado» são proibidos; cada erro diz a causa real em linguagem humana + a saída.
5. **NÃO degradar o Free retroativamente** — se o usuário criou 3 Espaços quando podia, eles continuam abrindo; o limite vale para criar novos (confiança > coerção).
6. **NÃO esconder Espaços com problema no switcher** — pasta sumida aparece com ⚠; esconder é mentir sobre o estado.
7. **NÃO embutir checkout dentro do app** — compra acontece no site (navegador); o app só ativa a chave. Local-first também é fronteira de confiança.
8. **NÃO espalhar o selo PRO pela UI** — anti-slop inclui não transformar o produto em propaganda de si mesmo.
9. **NÃO fazer phone-home nem qualquer validação online de licença** — «sem internet» não é um erro possível da ativação; qualquer estado desta tela que dependa de rede contradiz o gate 100% offline da onda F1-4 e o ADR 0011 (13.6 ação #2: o offline puro é decisão documentada, não acidente).

---

## Dúvidas para o Maestro

1. **Agentes de Espaços em background:** ao trocar de Espaço, os Agentes do anterior continuam trabalhando (PTYs vivos, custo correndo) ou adormecem? O M8 foi desenhado assumindo que **continuam** (com 🔔 cross-Espaço na Fila), mas isso tem custo de RAM/CPU/dinheiro — precisa de decisão de produto + benchmark próprio (13.3, item 6: provar o target de N agentes por medição interna, não herdar o «20+» do AgentsRoom).
2. **Atalhos propostos** (`Cmd+J` fila, `Cmd+O` switcher, `Cmd+1…9` troca, `Cmd+N` novo Agente, `Cmd+Shift+N` novo Espaço): validar colisões com o que a Fase 0 já reservou e com atalhos de sistema/CLIs dentro do terminal focado (quando um terminal tem foco, o que vence — o Lina ou o CLI?).
3. **Semântica do `Cmd+Enter` global:** confirmar a tabela do fluxo b (em especial: «sem item visível → abre a fila, nunca aprova»). É uma extensão de comportamento do gate existente — precisa de aval de quem é dono da custódia (round5).
4. **Som de escalação default:** desenhei **ON discreto** (1× por 30 s, mute em Ajustes) seguindo 13.13 achado 9 — mas «atenção sem ansiedade» poderia argumentar por OFF default. Decisão fina de produto.
5. **Teto de custo inicial:** sem teto por default + sugestão gentil no primeiro dia que passar de um valor notável (R$ 5? R$ 10?), ou teto sugerido já no onboarding do Espaço? O valor do gatilho é decisão de produto.
6. **Recorte vivo do terminal na Fila (E4):** espelho vivo do VT (mais seguro contra race, mais caro) vs. snapshot no momento da abertura + re-validação no clique (mais barato). O 13.13 deixa claro que a mitigação exata é **decisão de ADR do Lina**, não padrão de indústria — o fluxo desenhado acomoda ambos, mas o ADR precisa fechar antes da story de injeção remota.
7. **Co-piloto de papel sem Agente vivo:** quando o usuário cria o primeiro Agente do Espaço, o role-discovery roda em quê? (motor recomendado em sessão efêmera? heurística local de templates?) Afeta a latência e o fallback do M6-E2.
8. **Free vs. PRO — fronteira exata da observabilidade:** desenhei «estado + atividade no Free; custo/teto/histórico no PRO» (alinhado a 13.3 itens 7 e 9), mas a fronteira fina (sparkline é Free? custo do dia é Free e só o histórico é PRO?) é pricing, não UX — preciso da decisão para fechar o P6 em duas variantes.
9. **Nomenclatura na superfície:** o doc segue o glossário da Fase 0 («Agente», nunca «terminal»; «motor de IA», nunca «CLI») — inclusive no título do M6 («Novo Agente», onde o Maestri diz «Novo Terminal»). Confirmar que a Fase 1 mantém essa lei, pois as stories vão herdar os rótulos daqui.
10. **Itens «★ demais conforme pricing» do T7§P:** a lista do que o PRO destrava precisa vir fechada do pricing para a story da tela não nascer com placeholder.

---

## Decisões

Registro vivo: as 10 dúvidas acima + decisões do fundador que afetam este doc. Nada implícito — toda linha tem dono. Para linhas PENDENTES, a coluna Votante indica **quem decide**.

| Dúvida | Status | Decisão | Votante | Data |
|---|---|---|---|---|
| Limite de Espaços no Free (afeta M8/M9/T7§P) | ✅ Decidida | Free = **1 Espaço**; PRO = N data-driven (`workspace_limit` na chave, F1-4-5) — copy deste doc corrigido | Fundador | 2026-06-06 |
| Alvo de render/escala (contexto da dúvida 1) | ✅ Decidida | **Medir primeiro**: profiling bloqueante (F1-5-1) antes de qualquer otimização; alvo honesto **8–12 terminais ativos** (28 painéis refutado pela 13.7) | Fundador | 2026-06-06 |
| Auto-aprimoramento v0 (F1-3) | ✅ Decidida | Curator-like **com gate humano**: sugere, nunca aplica sozinho | Fundador | 2026-06-06 |
| Escopo da Fase 1 | ✅ Decidida | Esqueleto **F1-0..F1-6** confirmado | Fundador | 2026-06-06 |
| 1 · Agentes de Espaços em background continuam trabalhando ao trocar? | ⬜ PENDENTE | — (a decisão de render acima limita o teto global: 8–12 ativos somando todos os Espaços) | Fundador | — |
| 2 · Colisões dos atalhos propostos (Cmd+J/O/1…9/N/⇧N) | ⬜ PENDENTE | — | Maestro | — |
| 3 · Semântica do `Cmd+Enter` global (tabela do fluxo b) | ⬜ PENDENTE | — | Maestro | — |
| 4 · Som de escalação: default ON discreto ou OFF | ⬜ PENDENTE | — | Fundador | — |
| 5 · Teto de custo inicial (valor do gatilho da sugestão) | ⬜ PENDENTE | — | Fundador | — |
| 6 · Recorte vivo vs. snapshot na Fila (E4) | ⬜ PENDENTE | — (fecha no ADR de segurança da injeção — F1-1-8) | Maestro | — |
| 7 · Co-piloto de papel sem Agente vivo | ⬜ PENDENTE | — (restrição já fixada pela F1-2-3: offline e sem LLM embutido) | Maestro | — |
| 8 · Fronteira fina da observabilidade Free vs. PRO no P6 | ⬜ PENDENTE | — | Fundador | — |
| 9 · Nomenclatura «Agente» (vs. «terminal») na superfície | ⬜ PENDENTE | — | Fundador | — |
| 10 · Lista fechada do que o PRO destrava | ⬜ PENDENTE | — (depende do pricing — ADR 0011) | Fundador | — |



