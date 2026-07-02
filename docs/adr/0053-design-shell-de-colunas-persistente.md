# ADR 0053 — Shell de colunas persistente: overlays flutuantes viram layout dedicado (modal só para o irreversível)

- **Status:** **Aceito** (onda **Refatoração de Design**, conduzida pelo Maestro 00; decisão de arquitetura de UI, 2026-06-25). **ADR-gate** da story **D2-1** (shell de colunas) e de toda a cascata que migra para ele (**D2-2…D2-6, D4-1**): nenhuma começa sem este contrato. As **decisões do fundador** (épico `tasks/epico-design/00-epico-refatoracao-de-design.md` §0) têm precedência. Item de plano: `ADR-DSGN`.
- **Escopo:** trocar a arquitetura de layout do shell `app/lina-gpui` de *"um canvas com tudo flutuando em `.absolute()` + 9 modais"* para um **shell de colunas persistente** (rail · canvas · coluna utilitária colapsável · topbar/footer flat). Decide: (1) a estrutura de colunas, (2) o que vira coluna/inspetor e o que **continua** modal, (3) as portas de continuidade preservadas. **DISJUNTO** da estética — cor → ADR 0054; geometria → ADR 0055.
- **Relacionados:** invariante **#7** (core/shell split, trait `UiHost` — esta reforma é 100% camada gpui; o core **não** muda); âncora de continuidade **`PortalEngine` + `ExternalTextureLayer`** (o slot do browser navegável-pela-IA da Fase 1+ **não** pode sair da cena); ADR 0019 §7 (movimento/reduce-motion); CLAUDE.md §6 (gate humano para ação irreversível — o único caso que sobrevive como modal). Fontes externas (acesso 2026-06-25): NN/g *Modal & Nonmodal Dialogs* (o modal desabilita o conteúdo — reservado ao destrutivo); PostHog (navegação por produtos em sidebar + painéis); Raycast Eng (Settings em painel nativo, nunca modal volátil); Userpilot (*"never use more than one modal consecutively"*).

## Contexto

O root do shell é **um canvas** `relative().size_full()` (`main.rs:5002-5011`) sobre o qual topbar, footer, painéis e diálogos flutuam em `.absolute()` (enxame medido: `main.rs:1394,2393,2528,2575,2734,2822,2944,3181,5291,5736,5867,6179`). Disso nascem **9 superfícies modais** construídas sobre `ui/modal.rs:286-318` (véu 0.5 + card centralizado + armadilha de teclado): `agent_modal`, `credential_modal`, `webhook_modal`, `whatsapp_modal`, criação de Espaço, e mais. A Área de Poderes é overlay no canto (`powers_panel.rs`: `div().absolute().top_16().right_0()`), **não** as abas que o fundador pediu.

A queixa — *"tudo é modal, navega por seta, parece Windows antigo"* — é, na raiz, **arquitetural**: modal prende o foco, empilha e bloqueia o canvas; a literatura de UX reserva o modal à confirmação destrutiva (NN/g). As referências do fundador (PostHog, e os pares Linear/Raycast/Arc) fazem o oposto — **navegação espacial por colunas e painéis persistentes**, onde nada bloqueia e o estado fica sempre visível (invariante #6: estado sempre salvo e visível). Este ADR fixa o novo contrato de layout antes do código porque várias telas assumem hoje a fronteira `.absolute()` (posição dos overlays, palette como nav).

## Decisão

### (1) Layout de colunas dedicado (substitui o root `.absolute()`)

O root passa a ser um **flex de colunas**, não um canvas com overlays:

```
┌──────┬───────────────────────────┬────────────────┐
│ rail │   topbar (flat, no-absolute)               │
│ (wks)├───────────────────────────┼────────────────┤
│      │                           │  coluna        │
│ ▮    │      canvas central       │  utilitária    │
│ ▮    │   (terminais — sempre      │  (colapsável:  │
│ ▮    │    interativo)            │   Poderes/      │
│      │                           │   Ajustes/      │
│      │                           │   inspetor)     │
│      ├───────────────────────────┴────────────────┤
│      │   footer (flat, no-absolute)               │
└──────┴────────────────────────────────────────────┘
```

- **Rail de workspaces** à esquerda (estrutura de D3-1). **Canvas central** com os terminais (o produto). **Coluna utilitária** à direita, **colapsável**, que hospeda Poderes (abas), Ajustes (seções) e os inspetores de criação. **Topbar e footer** são linhas flat do flex — **não** mais `.absolute()` sobre o canvas.
- **Regra dura (affordance):** abrir a coluna utilitária **não bloqueia o canvas** — o usuário continua arrastando/focando terminais com Poderes ou Ajustes abertos. É o que separa "painel" de "modal".

### (2) Modal retido **só** para confirmação destrutiva/irreversível (1 pergunta sim/não)

`ui/modal.rs:286-318` (véu + armadilha de teclado) **permanece**, mas sua única razão de existir passa a ser a **confirmação de ação irreversível** — exatamente o gate humano que a doutrina já exige (CLAUDE.md §6: apagar, deploy, publicar, gastar). Modal aqui é **correto**: a ação É bloqueante por natureza. Todo o resto migra:

| Hoje (modal) | Vira |
|---|---|
| `agent_modal` (criar terminal/agente) | **inspetor** na coluna direita (canvas vivo) — D2-4 |
| `credential_modal` | seção **inline** em Ajustes (coluna) — D2-3 |
| `webhook_modal` / `whatsapp_modal` | seção/aba na coluna (Poderes ou Ajustes) |
| criação de Espaço | inspetor lateral não-bloqueante — D2-4 |
| Área de Poderes (overlay) | **coluna com abas** (Skills·Plugins·Canais·MCP·Automações) — D2-2 |

Cada destino é **contexto auto-contido que salva sozinho** (ressalva Baymard: aba/seção nunca é um formulário fatiado); erro vai **inline**, ao lado do campo, nunca num modal.

### (3) Portas de continuidade preservadas (critério inforjável)

- **`UiHost` (inv#7):** a reforma é 100% `app/lina-gpui`. **Nenhum** arquivo cruza para `lina-core`/`lina-host`; o core não conhece coluna, painel nem layout. A única story com persistência (cor de workspace, D3-3) guarda **um nome de acento** como dado opaco no core (ver ADR 0054), sem o core importar tipo de UI.
- **`PortalEngine`/`ExternalTextureLayer`:** o slot da camada de textura externa (browser navegável-pela-IA, Fase 1+) **continua na cena**. Re-arquitetar para colunas **não** pode removê-lo — o canvas central é justamente onde ele encaixa. Verificado por grep do tipo presente na árvore de render.
- **Teclado + AccessKit (não regridem):** Raycast é amado **por ser** keyboard-first — o arcaico é a **ausência de affordance de mouse**, não a presença do teclado. A navegação por teclado (F1-2-6) e os papéis AccessKit Name/Role/Value seguem; o ponteiro é **promovido** ao lado dele (D2-6), não substitui.
- **Movimento (ADR 0019 §7):** abrir/fechar coluna usa `MotionTokens::instant` (0ms) — é input frequente; nunca animar. Subordinação a reduce-motion mantida.

## Segurança (portas que NÃO fecham)

- **Gate humano destrutivo intacto:** o modal sobrevive **exatamente** para a confirmação irreversível (CLAUDE.md §6). Migrar os demais para coluna **não** remove nenhum gate de execução (custódia ADR 0004 / WorkspaceTrust ADR 0006/0010 inalterados).
- **Layout é apresentação, não autoridade:** nenhuma decisão deste ADR toca identidade, ordem ou autorização. Coluna aberta ≠ permissão concedida.

## Por quê assim (alternativas descartadas)

- **Manter modais, só "modernizar" a aparência** — rejeitado: trata o sintoma (cor/raio) e deixa a causa-raiz (foco preso, canvas bloqueado). O fundador continuaria sentindo "arcaico".
- **Remover o teclado/palette para "parecer app de consumidor"** — rejeitado: regride F1-2-6 e a11y; o teclado não é o problema (Raycast prova). Promove-se o mouse **ao lado**, não no lugar.
- **Simplificar a cena removendo o slot Portal** — rejeitado: fecha a porta da Fase 1+ (CLAUDE.md, âncoras de continuidade). O canvas de colunas é desenhado **com** o slot.

## Consequências

- **Habilita** D2-1 (constrói o shell de colunas) e, por dependência, D2-2 (Poderes em abas), D2-3 (Ajustes em coluna), D2-4 (inspetores de criação), D2-5 (modal só destrutivo), D2-6 (teclado+ponteiro), D4-1 (botões da topbar como toggles das colunas).
- **Costura de dono único:** `main.rs` (root/layout/topbar) — dono único na onda do shell; workers não commitam, o Maestro valida de fora e committa por fatia.
- **Custo:** re-arquitetura do root + migração de 9 superfícies. Nenhuma dependência nova; nenhuma mudança no core.
- **Porta que fecha se ignorado:** sem o shell de colunas, toda story de tela continuaria empilhando overlay — a reforma viraria maquiagem.

## Verificação (observável)

- O app sobe com **layout de colunas**; `grep '.absolute()'` em topbar/footer = **0** (não flutuam mais sobre o canvas).
- Coluna utilitária faz toggle com o **canvas ainda interativo** (arrastar um terminal com Poderes aberto — verificado por interação).
- O **slot `PortalEngine`/`ExternalTextureLayer`** continua na árvore de render (grep do tipo presente).
- Navegação por teclado **verde** (teste F1-2-6 não removido); AccessKit Name/Role/Value presentes.
- `cargo test -p lina-gpui` verde; `token_ratchet` não regride.
