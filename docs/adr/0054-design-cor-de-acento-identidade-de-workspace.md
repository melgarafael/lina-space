# ADR 0054 — Cor de acento como identidade de workspace (modelo Arc) + recalibração 9→8 (sai o roxo-IA)

- **Status:** **Aceito** (onda **Refatoração de Design**, Maestro 00, 2026-06-25). **ADR-gate** das stories **D1-5** (recalibração cromática) e **D3-3** (cor de workspace): nenhuma começa sem este contrato. Decisões do fundador (épico §0) têm precedência. Item de plano: `ADR-DSGN`.
- **Escopo:** decidir (1) a recalibração da paleta de acentos de **9 → 8** (remover o `violeta`, o roxo-IA que a própria doutrina bane), (2) a **promoção** da cor de acento de "decoração de chrome" a **identidade de workspace** (1 acento por Espaço, escolhido e persistido), (3) a fronteira de persistência que mantém a porta `UiHost`, (4) o limite **acento-de-marca ≠ status**. **DISJUNTO** de layout (ADR 0053) e geometria (ADR 0055).
- **Relacionados:** invariante **#2** (local-first — a escolha é JSON local), **#4** (event log fonte da verdade do que é do app), **#7** (`UiHost` — o core não importa tipo de UI); doutrina de design (DES-2: gradiente/roxo-IA banido); `lina-design-doctrine`. Fontes externas (acesso 2026-06-25): Arc *Spaces* (a cor é pista de *"onde estou"*, não decoração); PostHog *visual-identity* (*"quanto mais cor, menos cada cor significa"* — **um** acento por tela).

## Contexto

A paleta de acentos é declarada em `theme.rs:182-192` como `pub const ACCENTS: [(&str, ColorScale); 9]` — **9 entradas**. Mas o cabeçalho do módulo e o comentário de seção dizem **"8 acentos curados"** (`theme.rs:10,172`): contradição real no código. A 3ª entrada é `("violeta", ColorScale::new(0xbb9af7, 0x6b3fd4))` (`theme.rs:185`) — `0xbb9af7` é exatamente o **roxo-de-IA** que a doutrina bane (DES-2) e que já foi removido das cores de **ação** (`theme.rs:242-247`: *"o roxo-IA banido pela doutrina (DES-2) sai"*) — mas permaneceu **escolhível** como acento pelo usuário. Some à percepção de "stand colorido": 8+ acentos saturados disponíveis sem disciplina de "um por tela".

Ao mesmo tempo, o Lina é **multi-workspace** (vários Espaços) e hoje a cor não diz **em qual Espaço você está**. Arc resolveu isso há anos: 1 cor por Space = orientação instantânea. É uma promoção barata e de alto valor — mas muda a **semântica** da cor (de enfeite a identidade), então precisa de contrato: como persiste sem furar `UiHost`, e como nunca colide com as cores de **estado**.

## Decisão

### (1) Recalibração 9 → 8: remover `violeta`

`ACCENTS` passa a ter **8 entradas**; `violeta`/`0xbb9af7` sai (reconcilia com o header que já diz 8). Default permanece `terracota` (a argila do "Ateliê", `theme.rs:175,180`). O gate WCAG re-itera sobre **8 × 2 modos** (superfície menor = mais seguro). As cores de **estado** (`STATE_SUCCESS/WARNING/DANGER`, `theme.rs:250-252`) e de **ação** (`ACCENT_ACTION/CREATE/CONFIRM`, `:244-247`) **não** são tocadas — já estão fora do array de acentos escolhíveis.

### (2) Cor de acento = identidade de workspace (1 por Espaço)

O usuário escolhe **1 acento por workspace**. Esse acento pinta a **identidade** do Espaço — o item do rail (D3-1) e o **único** botão primário "Novo" (D4-1). O restante do chrome fica nas superfícies **neutras** de terra (carvão/papel — `scale::CANVAS/PANEL/CHROME`). Disciplina PostHog: **um** acento por tela; o resto é neutro. Isso troca "stand colorido" por "cor com função".

### (3) Persistência sem furar `UiHost` — o core guarda um **nome**, não uma cor

A escolha persiste como **evento + projeção no core** (inv#4), mas o core **nunca** importa tipo de UI:

- O core armazena, por workspace, uma **`String` opaca** (o nome canônico do acento, ex. `"terracota"`) — dado, não cor. Reusa o contrato de preferências por-workspace já existente; é o mesmo tipo de string que já trafega (modo de tema). **Nenhum** `ColorScale`/`Rgba`/tipo gpui cruza a fronteira.
- A **resolução** `nome → ColorScale` acontece **só no shell** (`theme.rs::accent_entry`, `:197`). O core não sabe o que `"terracota"` pinta; a UI sabe.
- **Local-first (inv#2):** a escolha é JSON local (mesmo canal de `settings.json`/`tema.json`); nada sai da máquina.

### (4) Limite inforjável: acento-de-marca **≠** status

O acento escolhido pelo usuário pinta **identidade** (rail, primário "Novo"). Ele **nunca** é resolvido como **estado**: `success`/`warning`/`danger` vêm de `StateTokens` (tokens fixos, separados), jamais do acento ativo. Um Espaço com acento `verde` **não** faz um chip de erro parecer sucesso. Teste prova a separação por mutação (trocar o acento do workspace **não** muda nenhuma cor de estado).

## Segurança (portas que NÃO fecham)

- **Nome de acento é DADO, não autoridade:** a `String` persistida não entra em caminho de identidade/ordem/autorização; mutá-la só muda a cor de marca, nada de comportamento.
- **`UiHost` intacto:** o core guarda string opaca; zero tipo de UI atravessa. A porta de troca de toolkit (gpui ↔ Slint) continua aberta.
- **WCAG não regride:** os 8 acentos × 2 modos passam o gate como texto/fill; a cor de workspace escolhida é re-verificada nos dois modos.

## Por quê assim (alternativas descartadas)

- **Manter `violeta` "porque já existe"** — rejeitado: é o marcador DES-2 (roxo-IA) que alimenta a percepção de slop; a doutrina já o baniu da ação, manter como acento é incoerência.
- **Persistir um `ColorScale`/hex no core** — rejeitado: fura `UiHost` (core passaria a conhecer cor de UI). O core guarda **nome**; a UI resolve.
- **Acento do workspace reaproveitado como cor de status** — rejeitado: colapsa identidade e estado; um Espaço "verde" mentiria sobre erros. Eixos separados, sempre.
- **Paleta livre (roda de cor) por workspace** — rejeitado: roda de cor livre é anti-slop reverso (T7§A) e fura o gate WCAG; curadoria de 8 escalas provadas é o limite.

## Consequências

- **Habilita** D1-5 (recalibração 9→8 + reconciliar header) e D3-3 (escolha+persistência da cor de workspace).
- **Costura de dono único:** `theme.rs` (tokens/`ACCENTS`) e o contrato de persistência no core (string por-workspace) — coordenados pelo Maestro.
- **Custo:** remover 1 entrada do array + reusar o contrato de preferência por-workspace + a regra "um acento por tela" nas telas. Nenhuma dependência nova.
- **Porta que fecha se ignorado:** deixar o roxo-IA e 9 acentos sem disciplina mantém o "stand colorido"; persistir cor no core fecharia a porta `UiHost`.

## Verificação (observável)

- `ACCENTS.len() == 8` (teste); `violeta`/`0xbb9af7` **ausente** do array; header do módulo bate com 8.
- Usuário escolhe acento de um workspace, fecha e **reabre** o app → escolha **persistida** (assertável); o core guardou uma `String`, não um tipo de UI (grep: nenhum `ColorScale`/`Rgba`/`gpui::` no contrato do core).
- Mutação: trocar o acento do workspace **não** altera nenhuma cor de `StateTokens` (acento ≠ status).
- Gate WCAG **verde** para os 8 acentos × 2 modos; escolha de workspace re-verificada.
- Replay-safe: workspace sem acento escolhido cai no default `terracota` (best-effort, nunca falha).
