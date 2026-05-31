# Spike W1-2 — Slint · RESULTADO

> **Objetivo:** gerar **dado** para a decisão de framework de UI (W1-4), não construir o produto.
> **O que foi feito:** 1 nó-terminal **real** — `lina-pty::PtyManager` rodando `sh` → bytes parseados por `lina-vt::AlacrittyBackend` (`VtBackend`) → render numa **janela Slint** (texto monoespaçado nativo), com telemetria ao vivo e auto-quit.
> **Ambiente:** macOS (Apple Silicon), rustc 1.95, slint **1.16.1** estável (crates.io). Data: 2026-05-30.
> **Relacionado:** `../../CLAUDE.md` (decisão de stack) · `40 - Debate Arquitetura/46 - R2 Arquiteto.md` (minha posição: Slint vence em a11y/IME; é o fallback de chrome).

---

## 1. Esforço de setup — **BAIXO**

- **Dependência única:** `slint = "1.16"`, *features default*. Nada de `build.rs` nem arquivo `.slint`: usei o macro **`slint::slint!{}` inline** (markup dentro do `main.rs`). Da intenção ao primeiro `cargo build` verde: ~30 min, a maior parte esperando compilar.
- **Zero setup de a11y:** `accessibility` (AccessKit) **já está nas features default** (`std, backend-default, renderer-femtovg, renderer-software, accessibility, compat-1-2`) — não precisei ligar feature nenhuma (confirma a tese do debate).
- **Curva da DSL:** mínima para um spike (Window/VerticalLayout/Rectangle/Text + `in property`). Propriedades hifenizadas (`grid-text`) viram setters `set_grid_text` no Rust — mapeamento previsível.
- **Atrito real:** o macro de markup compila num passo do build; erros de markup aparecem como erro de macro (mensagens razoáveis). O único tropeço foi Rust puro (lifetime de `MutexGuard` no fim do `main`), não Slint.

## 2. Compila? Roda? — **SIM para os dois**

- **Compila:** ✅. Build **frio ~93s** (femtovg + cosmic-text + winit + deps), **incremental ~3s**.
- **Roda:** ✅. A janela **abriu no macOS** e o pipeline real rodou ponta a ponta: o `sh` emitiu 14 linhas + prompt, o `AlacrittyBackend` parseou **918 bytes**, e a UI atualizou via poll 30 Hz até o **auto-quit (3.5s)**. Saída real do binário:

```
--- METRICAS DO SPIKE ---
time_to_first_frame: Some(435.160625ms)
ui_ticks (poll 30Hz): 75
avg_property_update: 49.693µs
linhas capturadas (last_nonempty_line): 15
bytes do PTY processados por lina-vt: 918
max damaged_rows observado: 24
```

- O caminho de erro está tratado: se o ambiente fosse headless, `Main::new()` retornaria `PlatformError` e o spike reportaria "compilou, janela exige display" sem travar. Aqui a janela abriu de fato.

## 3. Qualidade de render de texto — **boa no nativo, com 1 limitação estrutural para terminal**

- **Texto monoespaçado nativo (`Text` + `font-family: "Menlo"`)**: nítido, layout correto, atualização fluida a 30 Hz sem engasgo. Para chrome (painéis, rótulos, onboarding) é mais que suficiente.
- **Limitação estrutural para um GRID de terminal:** o elemento `Text` do Slint é **estilo único por elemento** — não há atributo por-célula (fg/bg/bold/inverse) num só `Text`. Um terminal real (cores ANSI por célula, cursor, seleção) exigiria **(a)** compor N `Text`/`Rectangle` por run de estilo (caro com N terminais), ou **(b)** renderizar a superfície do terminal numa **textura wgpu** e compô-la (features `unstable-wgpu-27/28`) — o caminho de atlas-de-glifos GPU. Para o spike (mostrar texto parseado) o nativo bastou; para o produto, o terminal quer o caminho GPU.
- **Achado de integração (não é culpa do Slint):** o `VtBackend` público só expõe `last_nonempty_line()` — sem acessor multi-linha do grid. Sem tocar o core, renderizei uma **captura rolante** dessa linha a cada `advance` (15 linhas distintas capturadas). **Recomendação p/ W2:** adicionar `renderable_rows()` à trait `VtBackend` (já previsto no rascunho da W0-2), para o render consumir o grid inteiro.

## 4. Integração AccessKit — **SIM, de graça, por feature default**

- **Confirmado na prática:** `accessibility` é **feature default** do `slint` 1.16 e o build a incluiu sem nenhuma configuração. O Slint liga o **AccessKit** automaticamente através do **backend winit** (UIA no Windows / NSAccessibility no macOS / AT-SPI no Linux). Elementos nativos (`Text`, controles, `TextInput`) publicam nós de acessibilidade automaticamente; é possível enriquecer com propriedades `accessible-role`/`accessible-label`.
- **Isto valida o doc 46** (Slint = melhor a11y do Rust) e ataca diretamente o risco #2 que levantei na R2 ("a11y sem o Chromium"): no caminho de chrome nativo do Slint, a a11y vem **pronta**, não como trabalho manual.
- **Ressalva honesta (mesma do doc 46):** se a superfície do terminal/canvas for uma **textura wgpu custom**, ela é **opaca** ao AccessKit — a a11y daquele conteúdo teria que ser exposta manualmente (uma "accessible view" do grid via `accessible-*` ou role custom). Ou seja: a11y grátis para o chrome Slint; a11y manual para a superfície GPU.

## 5. IME — **suportado no framework (avaliado por capacidade, não por teste CJK ao vivo)**

- O Slint expõe **`TextInput`** com suporte a **input-method/preedit**; o backend winit encaminha eventos de IME (composição, preedit, commit) ao `TextInput`. Logo, entrada CJK/dead-keys/acentos compõe no nível do framework — o oposto dos toolkits que o Analista marcou como "IME incompleto" (iced/floem/egui).
- **Não testei IME ao vivo neste spike** (é render-only; não embuti um `TextInput` para o terminal). Para o produto, o caminho de input do humano cairia num `TextInput` (ou no `WriteOp::HumanKeys` do `UiHost` capturando teclas) e o preedit de IME seria consumido ali. **Confiança: média-alta** (capacidade documentada + a11y default observada), a confirmar com um teste de composição CJK na fase de build do shell.

## 6. Frame timing — **medido o que dá; sem bottleneck de UI-thread para esta carga**

| Métrica | Valor | Leitura |
|---|---|---|
| **time_to_first_frame** | **~435 ms** | Cold: init do backend winit/femtovg + 1º paint. Aceitável; não é cold-start de app empacotado (é `cargo run` debug). |
| **avg_property_update** | **~50 µs** | Custo de setar `grid-text` + `telemetry` e marcar dirty. Barato — o UI-thread não é o gargalo. |
| **poll sustentado** | **75 ticks @ 30 Hz** | O laço de atualização rodou liso pelos ~2.5s de janela útil. |
| **GPU frame time** | **não instrumentado** | O femtovg renderiza na própria cadência; medir frame-time real pede o callback de frame do renderer (não exposto trivialmente). Os ~50µs de update + poll liso indicam folga. |

> Nota: números em **debug** (sem `--release`) e com carga leve (1 terminal). Não são benchmark de produção — são sinal de viabilidade. Comparação justa de teto de FPS (vs gpui) precisa de release + N terminais + render real do grid.

## 7. Veredito honesto sobre Slint para o Lina

**Slint é uma escolha forte, de baixo esforço e governança saudável — e entrega o que prometi no debate (a11y + IME prontos).** Mas o spike confirma a *divisão* que a R2 (doc 46) já antecipava:

- ✅ **Como host de CHROME (painéis, modais, onboarding, inspetor):** Slint é excelente — render nítido, AccessKit **de graça** (o maior trunfo para o público não-técnico/inclusivo), IME no framework, setup trivial, governança comercial saudável + licença royalty-free. É o **fallback de esforço** do doc 46, e este dado o reforça.
- ⚠️ **Como render da SUPERFÍCIE de terminal/canvas:** o `Text` nativo é estilo-único — um grid de terminal com cores por célula e N terminais a 120fps quer o caminho **wgpu-texture** (`unstable-wgpu-27/28`), que reintroduz "construir o atlas de glifos" e torna a a11y daquela superfície **manual**. Ou seja, no eixo de **performance máxima de canvas** (a prioridade nº1 do fundador), Slint **não elimina** o trabalho de GPU — ele o empurra para a feature `unstable-wgpu-*`.

**Recomendação para a W1-4 (decisão):** o dado deste spike sustenta a arquitetura **híbrida** do doc 46 — **superfície de canvas/terminal em wgpu próprio** (teto de performance + onde o gpui também compete) **+ chrome em Slint** (a11y/IME prontos, baixo esforço, governança saudável). Slint **sozinho** não é o caminho de teto-máximo para o grid de terminais; Slint **como host de chrome** é o de menor risco e melhor acessibilidade. A decisão final depende do spike gpui (W1-1) medir o **mesmo** pipeline (PTY→VT→GPU) em release com N terminais; só então comparamos teto-de-FPS maçã-com-maçã.

---

### Resumo executável
- **Build:** `cd spikes/spike-slint && cargo build` (frio ~93s, incremental ~3s).
- **Run:** `./target/debug/spike-slint` (abre janela, roda o PTY real, auto-quit em 3.5s, imprime métricas).
- **Não toquei** no core nem no workspace principal; o spike é crate isolado (Cargo.lock próprio). Sem commit.
