# ADR 0028 — Live-region: o caminho do auto-anúncio (custom Element × patch no gpui pinado × upstream)

- **Status:** **Proposto (DRAFT) — decisão PROVISÓRIA.** O spike que tira esta decisão do papel
  está com o Especialista em Telas NESTA rodada (despacho `tasks/despachos/r1-telas.md`; entrega
  `tasks/epico-f1/.entrega-spike-a11y.md` + roteiro `tasks/epico-f1/spike-a11y-roteiro.md`).
  A validação final é NA TELA (VoiceOver anunciando 1 frase **sem foco**), executada pelo
  Maestro/fundador — gpui não roda headless. O selo vem com essa evidência.
- **Onda/Story:** F1-2 · F1-2-7 (ação #1 P0 da pesquisa 13.15)
- **Data:** 2026-06-10
- **Fontes:** pesquisa **13.15** (inteira) · comentário **"GAP CONHECIDO (red-team)"** na função
  `live_region_element` em `app/lina-gpui/src/a11y.rs` (citado pelo símbolo — estável; hoje ≈
  l.194) · `.entrega-w46.md` · doc `33 - Decisao de Framework de UI` (governança do pin do gpui
  e custo da porta gpui↔Slint) · `tasks/epico-f1/ondas-2-4.md` l.138-153.

## Contexto (fatos estabelecidos pela 13.15 — nenhum é hipótese)

1. 🟢 O **AccessKit 0.24 pinado JÁ expõe** `Node::set_live(Live::Polite/Assertive)` — API
   estável desde a 0.5.0 (2022; no 0.24 `NodeBuilder` já foi fundido em `Node`, e é `Node` que o
   spike desta rodada usa em `a11y_live.rs`). O bloqueio **não** é o AccessKit.
2. 🟢 O **gpui do nosso SHA (`09165c1`) NUNCA chama `set_live`** em `write_a11y_info`: a struct
   `Interactivity` tem **15 campos `aria_*`** e **nenhum `aria_live`** (contagem verificada no
   checkout pinado, `crates/gpui/src/elements/div.rs`; a fonte 13.15/story diz "18" — divergência
   registrada na entrega desta fatia para correção da fonte; a conclusão não muda). Logo
   `Role::Status` sozinho **não** vira live-region — `live()` é `Off` na árvore inteira e
   VoiceOver/NVDA/Orca **não auto-anunciam** mudança de texto sem foco.
3. 🔴 (parcial) Afirmar "conforme ARIA live-region spec" hoje é **falso**. O que JÁ está pronto
   e testado é a mecânica de **coalescing 1×/turno** (`a11y.rs:79-131`); falta exclusivamente a
   politeness ativa. O que funciona hoje: o nó é legível AO FOCAR e visível na tela (banner).

O auto-anúncio "resposta pronta" sem foco é P0 de produto (invariante #6 — a11y não é
polimento), e a decisão de caminho mexe na âncora **UiHost/porta gpui↔Slint** (doc 33) → ADR.

## As 3 opções e seus custos REAIS

| Opção | O que é | Custo | Impacto na porta gpui↔Slint (doc 33) |
|---|---|---|---|
| **(a) custom `Element`** | Elemento nosso que sobrescreve `write_a11y_info` e chama `node.set_live(Polite)` direto no AccessKit | Implementar `request_layout`/`prepaint`/`paint` + tipos associados (13.15 achado 2); código nosso, shell-side | **Neutro**: vive no shell (que é substituível por definição); na porta Slint vira outra implementação atrás do `UiHost`, como todo o resto do shell. **Não toca o fork.** |
| **(b) patch no gpui pinado** | Expor `.live()` no builder de `Interactivity` dentro do vendoring | **Permanente e recorrente**: re-aplicar (e re-validar) a cada bump de SHA do pin; mais um delta nosso na governança do vendoring | **Encarece**: cada patch no pin aumenta o custo de manter o fork E de eventualmente sair dele — é exatamente o tipo de dívida que o doc 33 manda precificar |
| **(c) rastrear upstream** | Issue/PR em zed-industries; re-avaliar por release | ~Zero agora; **não entrega o auto-anúncio** em prazo nosso (roadmap de terceiro) | Neutro — e se o upstream entregar, qualquer caminho local é descomissionado |

## Decisão (PROVISÓRIA — condicionada ao spike)

1. **Recomendação: caminho (a) — custom `Element`**, salvo evidência contrária do spike. É o
   único que entrega o auto-anúncio sem criar dívida recorrente no pin: o custo é pagar UMA vez
   a implementação do Element (camada que já é nossa responsabilidade no shell).
2. **(c) corre SEMPRE em paralelo, qualquer que seja o vencedor:** abrir/rastrear a issue
   upstream e, **a cada bump de SHA do gpui** (momento que a governança do doc 33 já prevê),
   verificar se `write_a11y_info`/`Interactivity` ganhou suporte a live-region. Upstream
   entregou ⇒ o caminho local é removido (critério objetivo de re-avaliação, sem "um dia").
3. **Se o spike FALHAR tecnicamente** — a API do gpui não permitir o override sem patch, com
   evidência arquivo:linha do vendorado (formato exigido no despacho do spike) — a recomendação
   **vira (b)**, com duas condições de contorno: o patch é o MÍNIMO (expor `set_live`, nada
   além) e nasce com plano de descomissionamento atrelado ao rastreio (c).

## Critério de reversão (explícito, por gatilho)

- **(a)→(b):** o Element custom exigir API privada do gpui OU quebrar em 2 bumps de SHA
  consecutivos (manutenção provou ser tão cara quanto o patch que evitava).
- **(a|b)→(c) [descomissionar local]:** upstream expõe live-region utilizável no nosso pin.
- **Spike reprovado na tela** (VoiceOver não anuncia sem foco mesmo com `set_live(Polite)` **E
  `value` setados** — o contrato do adapter macOS exige `value().is_some() && live() != Off`,
  Descoberta #2 da entrega do spike: `accesskit_macos` anuncia o `node.value()`, e um nó live só
  com `label` é ignorado em silêncio): PARAR — nenhum caminho local resolve; reabrir
  investigação (adaptador AccessKit↔SO é outra camada) antes de gastar em (a) ou (b). Atenção:
  "anúncio mudo" com `value` AUSENTE é falha LOCAL e corrigível — não dispara este gatilho.

## Honestidade de comunicação (ação #2 P0 do 13.15 — vale DESDE JÁ)

- **Nenhuma doc/copy do produto afirma "conforme ARIA live-region"** enquanto o spike não
  passar NA TELA — auditável por grep nas docs (critério 3 da story).
- O comentário **"GAP CONHECIDO (red-team)"** em `live_region_element` (`a11y.rs`) permanece no
  código até a implementação real anunciar; ele é a fonte da verdade do estado atual.
- `aria-busy` durante output torrencial: refinamento **P2 futuro** (13.15), fora deste ADR —
  registrado aqui só para não se perder.

## Alternativas rejeitadas

- **"Já está pronto, é só usar `Role::Status`"** — falso pelo achado 2 da 13.15 (nenhum
  `set_live` no gpui do pin); foi exatamente a super-promessa que o red-team de W4-6 pegou.
- **Implementar o auto-anúncio em todos os nós já** — escopo da story seguinte/Fase 2; este ADR
  decide o CAMINHO; a implementação ampla só após a decisão selada (evita retrabalho em N nós).
- **Trocar de framework por causa disso** — desproporcional: a porta gpui↔Slint existe (doc 33)
  e o caminho (a) não a encarece; live-region não é o caso que justificaria acioná-la.
