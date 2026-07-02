---
name: lina-modal-doctrine
description: >-
  Contenção de overflow em QUALQUER superfície flutuante do shell — modal, dialog, painel,
  drawer, popover, vitrine, overlay. Use SEMPRE que for criar ou mexer num modal/painel que
  mostra conteúdo: 'cria o modal de X', 'faz o painel de Y', 'abre um dialog', 'a janelinha de
  Z', 'monta o overlay', 'o modal tá cortando/comendo as opções', 'não dá pra rolar', 'os botões
  somem'. Garante que um modal NUNCA cresce além da janela: o conteúdo rola DENTRO (corpo com
  min_h(0)), header e footer ficam fixos e sempre visíveis, nada é cortado em silêncio. Cobre a
  armadilha do flex (filhos encolhidos = 'comidos' em vez de rolar), o teto relativo à viewport,
  o scroll no corpo (não no painel) e o teste obrigatório de 3× a altura da tela. É a estrutura
  de contenção de um modal em gpui; não é a estética (lina-design-doctrine) nem a cópia.
---

# DOUTRINA — Contenção de overflow em modais e painéis (gpui)

> Regra permanente. Vale para **todo** modal, dialog, drawer, popover, vitrine ou painel
> flutuante do shell. Se uma superfície flutuante mostra conteúdo, ela **obrigatoriamente** segue
> isto. Sem exceção. (A doutrina é de toolkit gpui — não CSS; os princípios são os mesmos.)

## 0. Atalho: use o componente que já resolve

`app/lina-gpui/src/ui/modal.rs` (`ui::Modal`) **já encoda esta doutrina** (clamp à janela +
corpo `min_h(0)` + header/footer fixos + `role=Dialog`/aria + oclusão de clique de graça).
**Antes de escrever um modal à mão, use `ui::Modal`.** Os modais de webhook/credencial/WhatsApp já
o usam — copie. Só caia para o padrão manual (§3) quando o `ui::Modal` genuinamente não couber, e
aí siga a estrutura à risca.

## 1. Princípio inegociável

Um modal **nunca** cresce além da janela. Quando o conteúdo é maior que o espaço, o **conteúdo rola
dentro do modal** — o modal não vaza pra fora da tela, não corta conteúdo em silêncio e não empurra
o footer pra fora da viewport. O usuário sempre vê: o **header** (título + fechar), o **footer**
(ações) e tem acesso a **todo** o corpo via scroll. Se qualquer um dos três se perde quando o
conteúdo é grande, o modal está **quebrado** e a tarefa **não está pronta**.

## 2. Por que quebra (causas-raiz, nesta ordem — ~99% dos casos)

1. **`min_h(px(0.))` ausente no corpo rolável (a armadilha do flex).** Num `flex_col`, o filho que
   deveria rolar tem min-height "automático" e **se recusa a encolher** abaixo do conteúdo. O taffy
   então **espreme os filhos** ("comidos") em vez de gerar transbordo — e `overflow_y_scroll` nunca
   arma. **Sempre** `.min_h(px(0.))` no filho rolável (eixo horizontal: `.min_w(px(0.))`).
2. **Painel sem teto relativo à janela.** Sem `.max_h(px(viewport_h - margem))`, o painel tem
   altura "natural" = conteúdo e cresce infinito. Calcule o teto a partir do `viewport_size()`.
3. **`overflow` no lugar errado.** O scroll vai no **corpo**, não no painel inteiro. Painel que rola
   a si mesmo faz header e footer rolarem junto e sumirem (e cai na armadilha #1: filhos espremidos).
   O painel **clipa** (`.overflow_hidden()`); o **corpo** rola.

## 3. Padrão canônico em gpui (copie esta estrutura ao fazer à mão)

Três regiões: **header fixo · corpo rolável · footer fixo.**

```rust
let max_h = (f32::from(viewport.height) - TOP - MARGIN).max(FLOOR); // teto relativo à janela
let panel = div()
    .id("meu-modal")
    .max_h(px(max_h))          // nunca maior que a viewport
    .overflow_hidden()         // o PAINEL clipa; ele NÃO rola
    .flex().flex_col()
    .child(                    // HEADER fixo
        div().flex_none().child("Título"),
    )
    .child(                    // CORPO rolável — a linha que quase sempre falta é o min_h(0)
        div()
            .id("meu-modal-body")
            .flex_1()
            .min_h(px(0.))     // ⬅ destrava o encolhimento do filho flex-col
            .overflow_y_scroll()
            .child(conteudo_longo),
    )
    .child(                    // FOOTER fixo
        div().flex_none().child(botoes_de_acao),
    );
```

- Largura também relativa: `.max_w(px(min(viewport_w - 2·margem, largura_pref)))`.
- O overlay/backdrop tem respiro (não cola na borda) e oclui o clique (não vaza pro canvas/terminal
  atrás). `ui::Modal` faz isso por default.
- Scroll por roda funciona com `.id(...)` + `.overflow_y_scroll()` (não precisa de `track_scroll`;
  use-o só pra persistir/controlar a posição). Uma viewport-folha (lista) pode usar `.max_h(...)`
  própria em vez de `flex_1 + min_h(0)` — as duas formas valem, contanto que o scroll seja do corpo.

## 4. Definição de Pronto (DoD) — nenhum modal entrega sem os 7

- [ ] Painel com `.max_h(px(...))` derivado da **viewport** (`window.viewport_size()`).
- [ ] Painel com `.max_w(px(...))` derivado da viewport (encolhe em janela estreita, com piso).
- [ ] **Corpo** com `.overflow_y_scroll()` **e** `.min_h(px(0.))` (ou viewport-folha com `.max_h`).
- [ ] Painel **clipa** (`.overflow_hidden()`) — header e footer **não rolam** (`.flex_none()`).
- [ ] Overlay com respiro (não encosta na borda) e oclusão de clique (não vaza pro canvas atrás).
- [ ] Indicação de mais conteúdo (a barra de scroll basta; opcional: fade nas bordas do corpo).
- [ ] Esc/✕/clique-fora fecham (o shell roteia Esc; o Modal expõe os hooks — não sequestre o teclado).

### Teste obrigatório antes de fechar

Encha o modal com conteúdo proposital de **3× a altura da tela** e valide: (a) o painel continua
dentro da viewport, (b) o corpo rola, (c) header e footer permanecem fixos e visíveis. Falhou um?
**Não está pronto.** Sem GUI à mão, prove por auditoria de código contra os 7 itens do DoD acima e
contra o gate `modal_overflow_gate` (`app/lina-gpui/tests/`), que recusa scroll vertical sem teto.
