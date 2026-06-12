# Vocabulário leigo — Onda F2-2

> **Autor:** Redator (WRITER) · **Data:** 2026-06-12
> **Âncora de voz:** território F2-0-D "Instrumento de Estúdio com a temperatura do Ateliê" — caloroso, honesto, artesanal, sem jargão. O leigo é empreendedor, não engenheiro.
> **Regra-mãe (REGRA DE OURO da superfície):** zero jargão. Nunca expor "skill", "MCP", "agent", "runtime", "load", "tecla", "binding" para o usuário.
> **Coerência:** o tom segue o vocabulário de estado JÁ existente (`spec-f2-2-5-toolbar.md`: verbos de 1 clique — Atender · Editar · Centralizar · Encerrar). Estado sem ação não existe (princípio D4-A7).

---

## 1. O termo de superfície para skills / agents / MCPs

### Recomendação: **Poderes** — validado. (com refino de uso)

O candidato "Poderes" **vence** e deve ser adotado. A decisão não é de gosto: é o único guarda-chuva que cobre, sem mentir, três coisas tecnicamente heterogêneas que o leigo não distingue (nem deveria):

| O que é por baixo | O que o leigo entende | "Poder" funciona? |
|---|---|---|
| **skill** (sabe-fazer X) | "ele sabe pesquisar na web" | ✅ um poder que o terminal ganhou |
| **MCP** (conexão a ferramenta externa) | "ele consegue ler meu Google Drive" | ✅ um poder de alcançar algo de fora |
| **agent/subagente** (especialista) | "ele chama um especialista pra isso" | ✅ um poder reforçado |

**Por que "Poderes" e não os concorrentes:**

| Candidato | Por que NÃO |
|---|---|
| **Habilidades** | Morno, e mente sobre MCP: uma *conexão* ao Google Drive não é uma "habilidade". Cobre só o caso skill. |
| **Extensões** | Jargão de navegador/IDE — exatamente a palavra técnica que a regra-mãe bane. Genérico (slop). |
| **Capacidades** | Burocrático, sem temperatura. Soa a planilha, não a ateliê. |
| **Ferramentas** | Já é usado para outra coisa na cabeça do leigo (a ferramenta = o app inteiro); confunde. |
| **Superpoderes** | A skill interna se chama "superpowers", mas na superfície "super-" infantiliza demais para um empreendedor. "Poderes" guarda a elasticidade sem o exagero. |

**Trade-off honesto (registrado):** "Poderes" tem um risco de soar lúdico/gamer. Ele é **aceitável e até desejável** aqui — a temperatura quente do território pede uma palavra com vida, não um termo de catálogo. O antídoto ao risco não é trocar a palavra, é a **microcopy sóbria** dos estados (§2): a embalagem é calorosa, o conteúdo é claro e adulto.

**Refino de uso (como a palavra aparece na tela):**
- Coletivo / título de painel: **"Poderes"**.
- Item individual: **"um poder"** (masculino — define a concordância de todos os estados em §2).
- Em frase de resultado ao leigo: *"Esse terminal ganhou o poder de acessar seu Google Drive."* / *"Ativei o poder de pesquisar na web."*
- **Nunca** misturar com a palavra técnica: o usuário nunca lê "skill"/"MCP" — só "poder".

---

## 2. Os 5 estados de um Poder — rótulo + microcopy + ação acoplada

Eixo: estes são estados **de um Poder** (uma extensão), distintos do eixo de estado **de um nó/terminal** (pede-aprovação/trabalhando/pronto — `spec-f2-2-5-toolbar.md`). São vocabulários irmãos, não concorrentes; o tom é o mesmo (calmo, sem alarme técnico), o gênero é **masculino** ("o poder").

Cada estado carrega uma **ação de 1 clique** (D4-A7: estado sem ação é parede). Rótulo curto = chip/badge; microcopy = 1 linha de apoio; ação = verbo no botão.

| Estado interno (despacho) | Rótulo (chip) | Microcopy (1 linha) | Ação (botão) |
|---|---|---|---|
| **disponível** | **Pronto** | "Está aqui — é só ativar." | **Ativar** |
| **ativa** | **Ativo** | "Funcionando agora." | **Desativar** |
| **quebrada** | **Deu erro** | "Parou de funcionar — não é culpa sua." | **Tentar de novo** |
| **não-carrega-aqui** | **Não roda aqui** | "Esse poder não funciona neste terminal." | **Entender por quê** |
| **precisa-de-conserto** | **Falta um passo** | "Falta uma configuração pra ativar — leva um minuto." | **Resolver** |

**Decisões de copy (o porquê, para o dev não reabrir):**

- **"Ativar/Desativar"** em vez de "Ligar/Desligar": "ativar" é universal e adulto; "ligar" infantiliza. Casa com o termo "Poder" ("ativar um poder").
- **`quebrada` vs `precisa-de-conserto` — a distinção que importa:** são erros de natureza oposta e a ação prova isso.
  - **Deu erro** = estava/deveria estar funcionando e *falhou sozinho* → a ação é **Tentar de novo** (a bola está com o sistema). O "não é culpa sua" desarma a ansiedade do leigo — temperatura do território.
  - **Falta um passo** = nunca chegou a funcionar porque *falta uma ação do usuário* (uma conexão, uma senha, uma autorização) → a ação é **Resolver** (a bola está com ele, e a microcopy promete que é rápido).
- **`não-carrega-aqui` — único estado sem ação corretiva:** o poder existe mas é incompatível com *este* terminal/CLI. Não há o que consertar; mentir com um botão "Tentar" seria cruel. A ação é só **explicar** ("Entender por quê"). É honesto — atributo da marca.
- **Tom anti-alarme:** nenhum "ERRO", nenhum "FALHA CRÍTICA", nenhum ícone vermelho de pânico. O pior estado ("Deu erro") ainda fala como um colega calmo, não como um sistema gritando.

---

## 3. Overlay de atalhos (tecla `?`) — agrupado por TAREFA, não por tecla

**Princípio:** o leigo não procura "o que a tecla ⌘K faz?" — ele procura "como eu **faço** uma busca?". Então o overlay se organiza por **intenção** (verbo do usuário), e a tecla é o detalhe à direita, nunca o título. Títulos de grupo = a tarefa; cada linha = uma frase de ação + a tecla.

> **Teclas confirmadas no código** (`app/lina-gpui/src/main.rs`, roteamento de `handle_key`) — não inventadas:
> ⌘N · ⌘T · ⌘K · ⌘J · ⌘1–9 · ⌘⏎ · ⌘⇧⏎ · ⌘R · ⌘W · ⌘+/− · ⌘0 · ⌘, · Esc · `?`

### Texto do overlay

**Título:** **Atalhos — o que você quer fazer?**
**Rodapé:** *"Aperte `?` a qualquer hora pra abrir esta lista."*

---

**▸ Começar um trabalho**
| O que você quer | Atalho |
|---|---|
| Trazer um novo terminal de IA pra mesa | ⌘N |
| Abrir um espaço de anotação | ⌘T |
| Buscar tudo e abrir o menu de ações | ⌘K |

**▸ Andar pela mesa**
| O que você quer | Atalho |
|---|---|
| Aproximar / afastar a visão | ⌘+ / ⌘− |
| Enquadrar tudo de volta na tela | ⌘0 |
| Pular direto pro terminal 1, 2, 3… | ⌘1 a ⌘9 |

**▸ Cuidar do que o time te pede**
| O que você quer | Atalho |
|---|---|
| Abrir a fila do que espera por você | ⌘J |
| Dizer **sim** ao pedido da frente | ⌘⏎ |
| Dizer **não** / dispensar o pedido | ⌘⇧⏎ |

**▸ Mexer no terminal em foco**
| O que você quer | Atalho |
|---|---|
| Ajustar este agente | ⌘R |
| Encerrar este terminal | ⌘W |
| Sair de dentro do terminal (voltar pra mesa) | Esc |

**▸ Ajustes & ajuda**
| O que você quer | Atalho |
|---|---|
| Abrir as preferências | ⌘, |
| Abrir / fechar esta lista | `?` |

**Decisões de copy do overlay:**
- **Cada linha começa pelo verbo do usuário** ("Trazer", "Abrir", "Pular", "Dizer sim") — a coluna de tecla é secundária, alinhada à direita. Quem lê varre por *intenção*, não por símbolo.
- **Grupos por momento de uso**, não por categoria técnica: "Cuidar do que o time te pede" reúne fila + sim + não porque é UMA tarefa mental (decidir pendências), embora sejam 3 teclas diferentes. Isso é o oposto de uma "tabela de keybindings".
- **⌘⏎ / ⌘⇧⏎** descritos por consequência ("dizer sim/não ao pedido"), nunca por mecânica — o leigo nunca precisa saber que é um gate de custódia.
- **Sem jargão de tecla:** "⌘" aparece como símbolo (o usuário de Mac reconhece); nenhum texto diz "modificador", "chord" ou "platform key".
- **Lacuna honesta:** se a UI ganhar atalho dedicado de *falar com um colega* (hoje isso passa pela paleta ⌘K), ele entra num grupo **"▸ Falar com o time"** — deixei o gancho nomeado para a r6 plugar sem reabrir a estrutura.

---

## 4. Resumo para quem implementa

1. **Termo único de superfície: "Poderes"** (item = "um poder", masculino). Nunca expor skill/MCP/agent.
2. **5 estados** = par {chip + microcopy + 1 verbo de ação}, gênero masculino, tom anti-alarme. `quebrada` e `precisa-de-conserto` são erros opostos — a ação (Tentar de novo × Resolver) carrega a diferença.
3. **Overlay `?`** = agrupado por tarefa, verbo primeiro, tecla à direita; só teclas confirmadas no código. Gancho "Falar com o time" reservado.
