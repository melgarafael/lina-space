# Copy congelada — Onda F1-4 (Espaços & Lina PRO)

> Papel UIUX_DESIGNER · despacho r1-ux-f14 · 2026-06-10 · v1.1 (revisada por banca adversarial de 5 lentes: glossário, offline, honestidade, completude, reconciliação)
> Wireframes-fonte: fluxo (e) do `ux-flows.md` (M8 · M9 · T7§P) + fluxo (b) (Fila de Atenção, para o aviso de validade). Critérios-fonte: F1-4-3/4/5/6 em `ondas-2-4.md` · gate 100% offline da onda F1-4 · [[13.6]].
> **Status: CONGELADA.** As stories da onda F1-4 colam estas strings literalmente; o T7§P do `ux-flows.md` aponta para este doc como fonte literal (os wireframes de lá são ilustrativos). Mudança daqui em diante passa pelo Maestro. Exceções variáveis estão na tabela "Pendências" no fim.

## Lei do glossário (inegociável em TODA string de superfície)

| Use sempre | NUNCA use na superfície |
|---|---|
| **Agente** | terminal, processo, PTY |
| **Espaço** | workspace, projeto técnico |
| **motor de IA** / nome próprio («Claude Code») | CLI, binário |
| **Chave do Lina PRO** (assim, com C — nome próprio em toda ocorrência da frase completa) / «chave» / «plano» | licença, license key, ativação de licença |
| **Pedido** / **Fila de Atenção** | permission prompt, gate |
| **venceu / vencida** (validade da chave — verbo único em todas as superfícies) | expirou, expirada, inválida-por-tempo |

## Convenções deste documento

- Texto entre «…» ou em bloco = **string final de superfície** (congelada, literal).
- `{…}` = variável preenchida em runtime (`{mês/ano}`, `{N}`, `{nome}`). Slots nunca carregam frase técnica embutida.
- `[[ … ]]` = botão primário · `[ … ]` = botão/link secundário · `↗` = abre o navegador padrão (único toque externo, sempre por clique explícito).
- Cada item fecha com **Racional** (1 linha), como pede o despacho.
- **Anti-padrões guardiões** (valem para tudo abaixo): paywall surpresa · jargão técnico · mentir estado. Lição da onda: badge honesto é REQUISITO, não polish (risco #5, `ondas-2-4.md`).
- **Regra de honestidade offline** (saída da banca): nenhuma string pode implicar semântica de servidor — não existe "invalidar a chave antiga", "destravar ao renovar" nem "associar ao computador". O que o app faz de verdade: **substitui um arquivo local e destrava quando uma chave válida é colada**. Toda frase descreve isso.

---

## 1 · Upsell honesto — o bloqueio do 2º Espaço no Free

### 1a. Antes do clique (a vitrine — o limite nunca surpreende)

| Onde | String congelada |
|---|---|
| Item do M8 | `+ Novo Espaço…` com selo «PRO» ao lado (selo pequeno, sem tooltip de venda) |
| Card de criação no M9 | «Você já usa o Espaço do plano Free (1 de 1)» · `[ Conhecer o PRO ]` |
| M8 com um só Espaço (CTA) | «Crie um segundo Espaço para separar este projeto do resto — cada um com seu time.» + selo «PRO» quando o usuário é Free |
| Busca do M8 sem resultado (ver item 5) | o botão `[ + Criar “{texto}” ]` também carrega o selo «PRO» para usuário Free no limite — é affordance de criação como qualquer outra |

*(As três primeiras já estavam congeladas no `ux-flows.md` — consolidadas aqui como fonte única; a quarta fecha a última vitrine que faltava.)*

### 1b. A tela do bloqueio (usuário Free clicou em criar o 2º Espaço)

```
Seu segundo Espaço vem com o Lina PRO

O plano Free inclui 1 Espaço — o seu, que já está em uso.
Para ter um Espaço para cada projeto, cada um com seu
próprio time, é só ativar o Lina PRO.

[[ Já tenho uma chave ]]      [ Quero o Lina PRO ↗ ]

[ Agora não ]
```

- `[[ Já tenho uma chave ]]` → expande o campo de colar do item 2 **ali mesmo** (sem trocar de tela).
- `[ Quero o Lina PRO ↗ ]` → abre o site no navegador. Nota fixa sob os botões: «A compra acontece no site. Aqui você só cola a chave que chega por e-mail — sem cadastro.»
- `[ Agora não ]` → volta para onde estava. Nada é perdido, nada insiste depois.

**Racional:** o limite é apresentado como fato do plano (sem culpa, sem "você atingiu/excedeu") e sem afirmar nada sobre o conteúdo do Espaço do usuário (pode estar sem Agentes), com benefício concreto no lugar de marketês e três saídas — as duas pedidas + uma neutra; nunca beco, nunca esforço desperdiçado antes do aviso.

---

## 2 · Ativação — colar a Chave do Lina PRO

Vale para as duas superfícies de F1-4-6: Ajustes › Plano (T7§P) e a tela do bloqueio (1b).

| Elemento | String congelada |
|---|---|
| Rótulo do campo (e rótulo lido por leitor de tela) | «Sua Chave do Lina PRO» |
| Placeholder | «cole aqui a chave do seu e-mail de compra» |
| Botão | `[[ Ativar ]]` |

- Em Ajustes › Plano (estado Free), o rótulo **visível** acima do campo é a pergunta «Já tem uma chave?» (E1 do ux-flows); «Sua Chave do Lina PRO» segue sendo o rótulo do campo para leitor de tela. Na tela do bloqueio (1b), o rótulo visível é «Sua Chave do Lina PRO».
- Comportamento (não vira texto): colar com espaços/quebras de linha **nunca falha por formatação** — trim e normalização silenciosos (acerto do `ux-flows.md` mantido). A verificação é local e instantânea; **não existe estado «verificando…» demorado nem qualquer estado que dependa de internet**.

### Os 4 erros acionáveis (todos verificáveis offline — assinatura e validade vivem dentro da chave)

| # | Quando acontece | Mensagem congelada | Saída |
|---|---|---|---|
| E1 | `Ativar` com o campo vazio | «Falta colar a chave — ela chegou no seu e-mail de compra.» | foco volta ao campo |
| E2 | Chave incompleta/malformada (truncada, faltando pedaço) | «Essa chave está incompleta — confira no e-mail de compra se copiou tudo, do começo ao fim.» | campo mantém o texto para corrigir |
| E3 | Assinatura não confere (chave alterada ou não emitida para o Lina) | «Essa chave não confere — copie de novo do e-mail de compra, sem mudar nada no texto.» | campo mantém o texto para corrigir |
| E4 | Chave com validade vencida (`expiry` dentro da própria chave) | «Essa chave venceu em {mês/ano}.» | `[ Renovar no site ↗ ]` · `[ Agora não ]` |

- Microcopy fixa sob o E4 (fecha o loop offline da renovação): «A renovação chega como uma chave nova no seu e-mail — é só colar aqui de novo.»
- O E4 do T7§P no `ux-flows.md` foi **espelhado nesta rodada** (tabela em 3 linhas, verbo «venceu», sem "incluindo os traços" — o formato da chave ainda não está decidido; ver Pendências).

### Sucesso (sem restart — o destrave é imediato)

```
✓ Lina PRO ativo — obrigado!

O que mudou agora:
 ★ Espaços: crie quantos precisar
 ★ {demais itens}

Válida até {mês/ano}

                                   [[ Continuar ]]
```

- A lista vem **dos entitlements assinados na própria chave** (gating data-driven de F1-4-5) — o bloco acima congela só o item garantido pelo mecanismo (`workspace_limit`); todo o resto entra pelo slot «★ {demais itens}», renderizado da chave. O wireframe E3 do `ux-flows.md` é ilustrativo; a fonte literal é este doc.
- A linha «Válida até {mês/ano}» só existe quando a chave tem validade; chave sem validade **omite a linha** (não escrever "vitalícia" — depende do pricing, ADR 0011).
- `[[ Continuar ]]` volta para onde o usuário estava; se veio do bloqueio do 2º Espaço, a criação já está liberada — colar → confirmar → criar em ≤3 interações (critério 1 de F1-4-6).

**Racional:** cada erro nomeia a causa real + a próxima ação em linguagem de leigo (sem código, sem culpa) e cobre todo o ciclo offline — vazio → incompleta → não confere → vencida; o sucesso só afirma o que está assinado na chave, e a renovação é explicada como o que ela realmente é (chave nova por e-mail), nunca como destrave automático.

---

## 3 · Painel do plano (Ajustes › Plano, T7§P)

### Estado Free (antes de ativar)

Estrutura: o E1 do `ux-flows.md` (duas colunas honestas). Strings congeladas aqui (fonte única):

| Elemento | String congelada |
|---|---|
| Título | «Seu plano: Lina Free» |
| Coluna esquerda (cabeçalho) | «O que você tem hoje:» |
| Itens da esquerda | «✓ 1 Espaço com seu time completo» · «✓ estado e atividade dos Agentes» |
| Coluna direita (cabeçalho) | «Com o Lina PRO você destrava:» |
| Itens da direita | «★ Espaços: crie quantos precisar» + slot «★ {demais itens}» (mesma regra data-driven do sucesso; ver Pendências) |
| Pergunta sobre o campo | «Já tem uma chave?» |
| Campo + botão | item 2 |
| Link de compra | «Ainda não tem? `[ Conhecer o Lina PRO ↗ ]`» |

### Estado PRO ativo

```
Seu plano: Lina PRO ✓
Válida até {mês/ano}                 ← omitida p/ chave sem validade
Espaços: {usados} de {limite} em uso

[ Trocar chave… ]        [ Remover chave ]
```

- Variante sem teto (entitlement ilimitado): «Espaços: {usados} em uso — sem limite».
- `[ Trocar chave… ]` → mesmo campo de colar do item 2, com a frase: «Cole a nova chave — ela entra no lugar da antiga.»
- `[ Remover chave ]` → confirmação honesta:

```
Remover a chave volta o app para o plano Free.
Seus Espaços continuam abrindo. Criar novos fica
pausado até colar uma chave de novo.

        [ Cancelar ]            [[ Remover ]]
```

- Arquivo da chave corrompido/adulterado depois de ativo (degradação graciosa de F1-4-5): o painel mostra, sem alarme:

```
Sua chave precisa ser colada de novo.
Cole do e-mail de compra para reativar o seu plano —
nada do seu trabalho se perdeu enquanto isso.
( cole aqui a chave do seu e-mail de compra____ )  [[ Ativar ]]
```

**Racional:** o painel afirma só o que o `lina-license` garante (plano, validade, usados/limite — critério 3 de F1-4-6) e cada consequência é a real: trocar chave **substitui** (não "invalida" — sem revogação no esquema offline), remover nunca ameaça (nada fecha, nada se perde), e a re-colagem promete só o que sempre é verdade (o trabalho está salvo — se a chave tiver vencido nesse meio-tempo, o E4 assume).

---

## 4 · Badges honestos do restore (F1-4-3) + aviso de validade vencida (F1-4-5 critério 7)

### Badge por Agente, ao reabrir o app

| Situação real | Badge (string congelada) | Ao passar o mouse |
|---|---|---|
| A retomada **aconteceu de verdade** (motor com resume declarado no perfil E a retomada aplicada com sucesso no boot) | «Sessão retomada» | «O Agente continua de onde vocês pararam.» |
| Motor sem retomada de sessão — **ou retomada que falhou** (sessão perdida/vencida no motor) | «Novo começo — o Agente não lembra da conversa anterior» | «A conversa de antes continua guardada aqui na tela — é só rolar para cima.» |

- **O badge segue o que aconteceu, nunca o que foi prometido:** declarar resume no perfil do motor não basta — se a retomada falhar no boot, o badge é o de «Novo começo». É exatamente o risco #5 da onda ("restore que mente destrói a confiança"); o caminho infeliz tem o mesmo peso do feliz.
- O badge é **discreto e temporário** (some na primeira interação com o Agente), mas **nunca é omitido**: a diferença entre os dois estados é o que o leigo precisa saber antes de mandar a primeira mensagem do dia.
- Bônus (opt-out por Espaço, escopo de F1-4-3): rótulo do toggle — «Abrir este Espaço sem religar os Agentes».

### Variação: aviso de validade vencida, não-bloqueante (F1-4-5 critério 7)

Entra como **item na Fila de Atenção** (fluxo b) na primeira re-avaliação pós-vencimento (boot ou tentativa de criar Espaço) — **nunca modal, nunca no meio da sessão**:

```
Sua Chave do Lina PRO venceu em {mês/ano}.
Tudo o que está aberto segue normal — criar novos
Espaços fica pausado até você colar a chave nova.

[ Renovar no site ↗ ]        [ Agora não ]
```

- Microcopy fixa sob os botões (variante para a Fila, longe do campo de colar): «A renovação chega como uma chave nova no seu e-mail — cole em Ajustes › Plano.»

**Racional:** o badge afirma apenas o que o sistema observou (scrollback persiste sempre; a memória do Agente, só com retomada confirmada), e o aviso de vencimento repete o critério 7 com fidelidade literal — nada fecha, nada rebaixa no meio do trabalho, e o destrave é descrito como o que é: colar a chave nova.

---

## 5 · Switcher de Espaços (M8, F1-4-4) — mini-status e vazios

### Gramática da linha (mini-status de governança)

```
▣ {nome do Espaço}     {N} Agentes ●    ~R$ {valor}  🔔  ⌘{n}
```

| Pedaço | Regra + strings congeladas |
|---|---|
| Agentes vivos | «{N} Agentes» (singular: «1 Agente») + ● na cor do **estado dominante** (mesmas 5 palavras/cores do vocabulário fixo do fluxo c) |
| Estado dominante — palavra junto da cor | tooltip/aria-label da dupla contagem+dot: «{N} Agentes — {Estado}» (ex.: «3 Agentes — Trabalhando») — cor nunca é o único indicador |
| Todos dormindo | a dupla contagem+dot vira a palavra «dormindo» (caixa baixa **intencional** em posição de mini-status, seguindo o wireframe M8; mesma palavra/cor do vocabulário fixo) |
| Espaço sem Agentes | «sem Agentes ainda» |
| Custo do dia | «~R$ {valor}» — o `~` é obrigatório (fluxo c: nunca fingir precisão) · tooltip: «gasto estimado de hoje neste Espaço» · «~R$ 0,00» aparece (zero é informação boa, não vazio) |
| Pendência de atenção | 🔔 quando há Pedido naquele Espaço · tooltip: «{N} Pedidos esperando você» (singular: «1 Pedido esperando você») |
| Item da Fila vindo de outro Espaço | prefixo «{Espaço} · {Agente} pede permissão» (herdado do ux-flows, congelado) |

### Ações de linha e arquivados

| Elemento | String congelada |
|---|---|
| Ações da linha (menu/hover) | `[ Renomear ]` · `[ Arquivar ]` |
| Arquivar (ação direta + toast com arrependimento) | «“{nome}” arquivado · [ Desfazer ]» |
| Link no rodapé do M8 | «Espaços arquivados ▸» |
| Linha de arquivado | «{nome} — arquivado em {data}» · `[ Trazer de volta ]` |

### Estados vazios

| Onde | String congelada |
|---|---|
| Vista de arquivados vazia | «Nenhum Espaço arquivado. Quando você arquivar um, ele fica guardado aqui — nada se perde.» |
| Busca sem resultado | «Nenhum Espaço com esse nome.» · `[ + Criar “{texto}” ]` — com selo «PRO» para usuário Free no limite (vitrine antes do clique, item 1a) |
| Só um Espaço | CTA do item 1a (herdado, congelado) |

**Racional:** cada linha responde em um relance "quem trabalha, o que custa, o que me espera" com o MESMO vocabulário de 5 estados do resto do app — palavra junto da cor (acessibilidade e leigos não leem só dots), custo com a honestidade do `~`, e todo vazio explica/recompensa em vez de constranger; «Trazer de volta» em vez de "desarquivar" mantém o tom de coisa guardada, não de operação técnica.

---

## Pendências que tocam esta copy (decisões fora do meu papel — strings variantes já prontas)

| Pendência (dono) | String afetada | Estado |
|---|---|---|
| PRO = ilimitado × teto N (Dúvida #4 de `ondas-2-4.md`, fundador) | «crie quantos precisar» × variante «crie até {N} Espaços»; «sem limite» × «{usados} de {limite}» | ambas as variantes congeladas acima; o entitlement da chave decide qual renderiza |
| Lista fechada do que o PRO destrava (ADR 0011/pricing, fundador — Dúvida #10 do ux-flows) | slot «★ {demais itens}» no sucesso e no estado Free | slot demarcado; não inventei itens |
| Fronteira fina da observabilidade Free×PRO (Dúvida #8 do ux-flows, fundador) | candidata pronta para o slot: «★ Medidor completo: custo por Agente, teto e histórico» — só entra quando a fronteira fechar | fora do bloco congelado de propósito (a banca pegou a contradição "data-driven com item hard-coded") |
| Formato visual da chave (terá traços?) (F1-4-5) | erro E2 | se o formato final tiver traços, trocar por «— confira se copiou tudo, incluindo os traços» (mais concreta); a menção a traços foi removida do ux-flows até lá |
| Pricing perpetual × subscription (fundador, Dúvida #3) | nenhuma string desta copy nomeia preço/modelo; o site absorve | copy imune por construção |

— fim da copy congelada —
