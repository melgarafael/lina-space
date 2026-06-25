# DESPACHO — Especialista em Telas · Refatorar a Área de Poderes (correção do fundador) · id: f2-4-ui-r2

> Leia `tasks/epico-f2/despachos/f2-4/_contexto.md` ANTES. Carregue `senior-frontend` + `lina-design-doctrine`.

## CONTEXTO (correção do fundador na tela)
O fundador abriu a Área de Poderes no app e **reprovou** (veredito dele, textual): *"Não consigo rolar
os poderes. E achei a organização deles muito ruim, pensei em uma área dedicada pra isso, com abas,
seções, separados por agents, hooks, commands, etc. Ficou péssimo. Refatore isso!"*

Na tela: o painel é o flanco direito espremido (`render_powers_panel` em main.rs usa `top_16().right_0()`,
`w(relative(0.42))`), com uma **lista plana** de 60+ cards onde os ~44 plugins afogam os 8 skills e o
resto, todos os cards idênticos, **e a lista NÃO ROLA** (corta na borda). O próprio `powers_panel.rs:481`
já admitia a dívida: *"lista direta (mock ~6 itens). Virtualização de 75+ cards é polimento de layout do
Maestro."* — essa porta agora é a tarefa.

## FUNÇÃO
Você é o **Especialista em Telas (FRONTEND)**. Refatora `app/lina-gpui/src/powers_panel.rs` (render:
abas + scroll + densidade). A costura de `main.rs` (campo de aba + filtro + container centralizado +
handler) é do Maestro 01 — entregue como **diff** na sua entrega; eu aplico e valido.

**Fronteira:** `app/lina-gpui/src/powers_panel.rs` (refatorar) + diff de costura para `main.rs`.

## DIRECIONAMENTO — a direção de design (eu defino o gosto; você materializa)
A identidade da casa **já está decidida** (épico 38 §VIII — "Instrumento de Estúdio com a temperatura do
Ateliê"): NÃO re-decida paleta/fonte. O problema é **ESTRUTURA**, não estética. Materialize:

1. **Área DEDICADA, centralizada e ampla** — pare de espremer no flanco direito. Um painel **central
   grande** (~`relative(0.72)` largura × ~`relative(0.82)` altura), com respiro, sobre um leve
   escurecimento do fundo (foco — modal-like, como o padrão de overlay do app). É a vitrine; merece espaço.

2. **Organização por ABAS de tipo** (o pedido do fundador): uma fileira no topo —
   **Todos · Skills · Plugins · Agentes · Comandos · Gatilhos · Conexões** — cada aba com o **contador**
   ("Plugins 44"). Aba ativa destacada (acento). Clicar troca o filtro. **Só renderize abas dos tipos
   PRESENTES** (count > 0) + a "Todos" — não polua com tipos vazios. O `kind_label` já dá o nome leigo
   pluralizado; reuse.

3. **SCROLL** — o cabeçalho (título Fraunces + resumo + abas) fica **FIXO**; a **lista da aba ativa
   ROLA** (`overflow_y_scroll`, idioma do app — ver `onboarding.rs:1162` e a pegadinha de layout em
   `:1153`: o pai precisa de altura constrita, não `size_full` sem bound; o body é `flex_1` rolável).

4. **Densidade + hierarquia** — com o filtro por aba, cada lista é homogênea. Para tipos com MUITOS itens
   (plugins), use um **grid de 2 colunas** (ou cards mais compactos: nome + estado + origem numa linha
   mais densa) para caber mais sem rolar infinito. O resumo geral ("60 Poderes no total") no cabeçalho;
   o foco vai para a lista do tipo. Use seu gosto — só **não** deixe a monotonia de 60 cards iguais empilhados.

5. **Zero literal de token** (o `token_ratchet` reprova): toda medida/cor/peso via `theme::active()` +
   `Space`/`Surface`/`radius`/tokens. Reuse `ui::{Panel, Badge, Button}` + os helpers de texto que já existem.

**Contrato de dados:** NADA muda no core. O painel já recebe `PowerInventory` + os `PowerCard` (que JÁ
vêm filtrados por `should_render`). A nova entrada que você precisa: a **aba ativa** + os **contadores por
tipo** (de `inventory.counts`) + um **handler de troca de aba**. O FILTRO dos cards por tipo o Maestro faz
no `main.rs` (monta só os cards do tipo ativo). Você expõe a API de abas (ex.: `.tabs(Vec<PowerTab>)`,
`PowerTab::new(id,label,active).on_select(handler)`) e renderiza cabeçalho-fixo + lista-rolável.

## OBJETIVO
Que o fundador abra a Área de Poderes e veja uma vitrine **organizada por tipo, que ROLA**, com respiro e
hierarquia — não a lista plana monótona espremida que ele reprovou. Um painel que um profissional sênior
de produto assinaria.

## RESULTADO ESPERADO
- `powers_panel.rs` com: cabeçalho fixo (título + resumo + **abas com contador**) + **lista rolável** da
  aba ativa + densidade melhor (grid/compacto). API de abas exposta para a costura.
- Estado vazio acolhedor preservado; 5 estados WCAG preservados; mostrar≠autorizar preservado.
- `token_ratchet` verde (arquivo segue zero-dívida); teste anti-jargão preservado; testes do módulo
  cobrindo a seleção de aba (pura).
- Diff de costura `main.rs` preciso: campo `powers_tab: Option<PowerKind>` + filtro + container central +
  handler `select_powers_tab` + as abas montadas dos `counts`.
- Validação: `cd app/lina-gpui && cargo test -- --test-threads=1` (suíte + ratchet), `cargo clippy
  --all-targets -- -D warnings`, exit direto.

> gpui não roda headless → a validação FINAL é a tela do fundador. Render provado por teste + costura que
> compila; eu integro e o fundador valida. **Esta é uma CORREÇÃO — o gosto dele é o juiz.**

Reporte ao @Maestro 01. Entrega `tasks/epico-f2/despachos/f2-4/.entrega-f2-4-ui-r2.md`. Última linha:
`PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
