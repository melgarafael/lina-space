# ADR 0055 — Geometria reta como decisão de sistema (chrome `radius.none` / conteúdo `radius.sm`; elevação por hairline)

- **Status:** **Aceito** (onda **Refatoração de Design**, Maestro 00, 2026-06-25). **ADR-gate** da story **D1-1** (migração de geometria) e referência para D1-2 (hairline), D3-1 (rail flat) e D4-2 (terminais). Decisões do fundador (épico §0) têm precedência. Item de plano: `ADR-DSGN`.
- **Escopo:** comprometer o shell com **uma** linguagem de raio — **chrome reto** (`radius.none` = 0px) e **conteúdo tocável levemente arredondado** (`radius.sm` = 4px) — e tokenizar os ~100+ cantos hoje fixos em `.rounded_md()`, **sem** literais `px(0.)` (que furariam o `token_ratchet`). Decide também a **elevação por hairline** (borda 1px) no lugar de sombra difusa. **DISJUNTO** de layout (ADR 0053) e cor (ADR 0054).
- **Relacionados:** invariante **#7** (geometria é token do shell, o core não muda); **`token_ratchet`** (catraca anti-número-mágico: literal `px`/`FontWeight` é regressão — esta é a porta que mais este ADR protege); gate WCAG (borda divisora ≥ 3:1). **Decisão do fundador** (épico §0.1): chrome reto + cards 4px — linguagem de **dois degraus**, **não** 0px universal. Fontes externas (acesso 2026-06-25): PostHog *vibe-designing* / Linear *design refresh 2024* (o "moderno de precisão" é raio **pequeno** + estrutura por **hairline**, raramente 0px universal; o slop é **misturar** raios sem intenção) — achado confirmado na verificação adversarial da pesquisa (a hipótese "0px = moderno" foi **parcialmente refutada** por fonte primária).

## Contexto

`RadiusTokens::CANONICAL` (`theme.rs:470-477`) já declara a escala: `none:0, sm:4, md:6, lg:12, full:9999`. O comentário acima dela (`theme.rs:457-458`) é uma confissão: *"`md`=6 preserva o degrau dominante MEDIDO no shell (103 usos de `.rounded_md()`); o 'flat honesto' da fusão entra na APLICAÇÃO (F2-2), não inflando a escala."* Ou seja: o flat foi **planejado e nunca aplicado**. Hoje **a esmagadora maioria dos cantos** usa `.rounded_md()` (6px) — um helper, **invisível ao `token_ratchet`** (que morde literais `px`, não helpers) — dando o ar "macio/shadcn-default" que destoa de um instrumento de precisão.

O fundador pediu "cantos retos". A pesquisa (PostHog/Linear, verificada adversarialmente) **refinou** o pedido: o que faz essas ferramentas parecerem modernas/precisas **não** é o canto a 0px — é **raio pequeno + estrutura sentida por linha de 1px (hairline), não por sombra**. O fundador, posto diante disso, **decidiu** (épico §0.1): **chrome reto (0px), conteúdo a 4px** — dois degraus. Este ADR crava essa linguagem como decisão de sistema (toda tela futura herda).

## Decisão

### (1) Linguagem de dois degraus (a régua de raio do shell)

| Camada | Token | Valor | O que recebe |
|---|---|---|---|
| **Chrome** | `radius.none` | **0px** | molduras, painéis, colunas, rail, topbar/footer, divisores, abas |
| **Conteúdo tocável** | `radius.sm` | **4px** | cards, inputs, botões, chips, menus |
| Pílulas/badges | `radius.full` | 9999 | só onde a forma É a pílula (badge de status) |

`radius.md` (6px) e `radius.lg` (12px) **saem da aplicação de chrome**: ficam como degraus de legado de conteúdo até a migração fechar, depois reservados. **`radius.none` é o default do chrome**, não exceção.

### (2) Migração tokenizada — `.rounded_md()` → token, NUNCA `px(0.)` literal

Os ~100+ sites de `.rounded_md()` (comentário cita **103**; grep atual conta mais — a migração os zera no chrome) passam a aplicar o **token** correto:

```rust
// chrome:   .rounded(px(f32::from(t.radius.none)))   // 0px, via token — nunca .rounded(px(0.))
// conteúdo: .rounded(px(f32::from(t.radius.sm)))     // 4px, via token
```

O literal `px(0.)` é **proibido** (o `token_ratchet` o conta como número mágico = regressão). Arquivo novo nasce em **zero dívida**. A contagem de literais `px`/`FontWeight` do shell **não pode subir**.

### (3) Elevação por **hairline**, não por sombra difusa

A separação estrutural entre colunas/painéis/cards é uma **borda de 1px** via `surface.border` (`scale::BORDER`, `theme.rs:227` — `0x3a3128` dark / `0xcbb99c` light), **não** `shadow`. Sombra difusa é o maior vetor de "cara de SaaS genérico" (Linear removeu no refresh; *"structure should be felt, not seen"*). `grep 'shadow'` em chrome/painéis tende a **0**. (Esta é a frente D1-2; o ADR a ancora porque é parte da mesma linguagem geométrica.)

## Segurança (portas que NÃO fecham)

- **`token_ratchet` é a porta protegida:** a migração **abaixa ou mantém** a contagem de literais `px`; `px(0.)` é banido. O critério é inforjável: a catraca morde se um literal reaparecer.
- **WCAG:** onde o hairline é **divisor estrutural**, a borda precisa de contraste ≥ 3:1 — re-verificado no gate (risco sinalizado, não suposto).
- **`UiHost`:** geometria é token do shell; o core não muda. Reduce-motion (ADR 0019 §7) inalterado.

## Por quê assim (alternativas descartadas)

- **0px universal (tudo reto, inclusive cards)** — **considerado e rejeitado pelo fundador** (épico §0.1): mais agressivo/industrial que as referências; PostHog/Linear não fazem isso. A decisão é dois degraus (chrome 0 / conteúdo 4).
- **Manter `.rounded_md()` (6px) "porque é o degrau dominante"** — rejeitado: é justamente a fonte do ar macio que destoa do instrumento de precisão; o flat já estava planejado e nunca chegou.
- **`px(0.)` literal direto nos sites** — rejeitado: fura o `token_ratchet` (número mágico); a geometria tem que vir do token para a régua valer em toda tela futura.
- **Elevação por sombra "para dar profundidade"** — rejeitado: sombra difusa = marcador de SaaS genérico; estrutura por hairline é a linguagem de precisão.

## Consequências

- **Habilita** D1-1 (migração de geometria) e dá a régua para D1-2 (hairline), D3-1 (rail flat) e D4-2 (moldura de terminal a `radius.sm`).
- **Costura de dono único:** `theme.rs` (a régua) é coordenado pelo Maestro; a aplicação espalha por arquivos de chrome (donos por story).
- **Custo:** migração mecânica dos sites de `.rounded_md()` + troca de sombra por borda. Nenhuma dependência nova; nenhuma mudança no core.
- **Porta que fecha se ignorado:** sem tokenizar, a geometria continua invisível à catraca e cada tela nova volta ao 6px macio — a "cara de precisão" nunca se fixa.

## Verificação (observável)

- `grep -rc 'rounded_md\|rounded_lg' app/lina-gpui/src` nos arquivos de **chrome** = **0**; chrome usa `radius.none`, conteúdo tocável usa `radius.sm`.
- **Nenhum** `px(0.)` literal (grep = 0); `token_ratchet` **verde** e a contagem de literais `px` **não sobe**.
- `grep 'shadow'` em superfícies de chrome/painel = **0**; separação visível por hairline 1px (`surface.border`).
- Gate WCAG **verde** (borda divisora ≥ 3:1 onde estrutural).
- `cargo test -p lina-gpui` verde (token_ratchet global incluído).
