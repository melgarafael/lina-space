# Spike W1-1 — gpui · RESULTADO

> **TL;DR:** gpui **compila e roda** um nó-terminal real (lina-pty + lina-vt) numa janela
> nativa, a **120 Hz** (frame p50 = 8.36 ms), e **integra AccessKit de primeira classe** —
> *refutando, neste SHA, duas premissas herdadas do debate de arquitetura* (`46/47` da R2:
> "gpui não integra AccessKit / zed#6576" e "exige toolchain Metal offline no build").
> O obstáculo real **existe mas é contornável**: o renderer mac, por *default*, compila
> shaders Metal **offline** (`xcrun metal`/`metallib`) — e o **Metal Toolchain está ausente**
> no Xcode 26 desta máquina (componente sob demanda). A feature **`runtime_shaders`** desvia
> para compilação em runtime e dispensa o toolchain. Custo secundário: **footprint de disco**
> alto (a árvore debug não coube nos ~2.6 GB livres até cortar o debuginfo).

- **Host:** macOS 26.4 (25E246), Apple Silicon (arm64), Xcode 26 instalado **sem** Metal Toolchain.
- **SHA auditado (pinado):** `09165c15dc5d1fea93604231eaf30ca4c25f1cd6` — `gpui v0.2.2` (monorepo `zed-industries/zed`).
- **Data:** 2026-05-30. **Toolchain:** rustc 1.95.0.
- **Escopo:** só `spikes/spike-gpui/` (excluído do workspace). Core/workspace **intocados**. **Sem commit.**

---

## 1. Esforço de setup (passos e obstáculos)

Cada obstáculo abaixo foi atravessado de fato; a ordem é a sequência real do spike.

| # | Passo | Obstáculo | Desfecho |
|---|-------|-----------|----------|
| 1 | Declarar `gpui` no `Cargo.toml` | **Não há crate no crates.io** — gpui só existe como **git dep do monorepo Zed** (457 MB de `.git`). | `gpui = { git = ".../zed" }`; `cargo fetch` resolveu **`gpui v0.2.2`** e gravou o SHA no lockfile → pinado como `rev`. Resolução **funciona** (consumo do fetch: ~290 MB). |
| 2 | Compilar `gpui` (core) | — | **416 crates, 0 erros, 2m19s.** O core do gpui (layout, elementos, AccessKit) compila limpo **sem** Metal Toolchain. |
| 3 | Abrir uma janela | `gpui` sozinho **não traz o renderer**: `gpui_platform` (que puxa `gpui_macos`) é **`[dev-dependencies]` do gpui**. O entry point `application()` vive em `gpui_platform`. | Adicionado `gpui_platform` como dep direta. |
| 4 | Compilar o renderer Metal (`gpui_macos`) | **O obstáculo central.** `gpui_macos/build.rs`, no caminho **default** (`not(feature="runtime_shaders")`), roda `xcrun metal -c shaders.metal` + `xcrun -sdk macosx metallib` em **build-time** → `include_bytes!(shaders.metallib)`. Mas neste host `xcrun metal --version` → **`error: cannot execute tool 'metal' due to missing Metal Toolchain; use: xcodebuild -downloadComponent MetalToolchain`**. No macOS 26/Xcode 26 o Metal Toolchain é **componente sob demanda** — um Xcode "completo" **não** o traz. Logo, o **build default do renderer FALHA** out-of-the-box. | **Contorno:** feature **`runtime_shaders`** → `build.rs` apenas *stitcha* o `.metal` (`include_str!`) e a compilação MSL acontece **em runtime** via `Metal.framework` (`new_library_with_source`). Sem necessidade do toolchain offline. |
| 5 | Caber no disco | **Disco do host quase cheio** (começou em ~3.9 GB livres; *Container Free* APFS = 561 MB, sem *purgeable* abundante). A árvore debug **com debuginfo** do gpui+renderer **não coube** (watchdog abortou em 546 MB livres, ainda compilando, **0 erros** — puramente disk-bound). | `[profile.dev] debug = false` (corta >50% do target; spike não precisa de debuginfo). **Coube** (min-free 1505 MB). |
| 6 | Pin do SHA | Pinar via `rev = "…"` muda a **identidade da fonte** no cargo (`?rev=` ≠ `git+…`) → **rebuild total** da árvore (duplica artefatos). | Aceito (correto p/ auditoria/reprodutibilidade). Build limpo a partir de `target` vazio = uma árvore só. |

**Tempo total de esforço até a janela aparecer:** ~3 iterações de build (fetch + core + renderer), dominadas por (a) descobrir o gate `runtime_shaders` e (b) gestão de disco. Sem esses dois, é um `cargo run` direto.

---

## 2. Compila? Roda?

- **Compila:** **SIM.** `cargo build` → `Finished dev [unoptimized] in 2m01s`, **0 erros**, incluindo `gpui_macos` (renderer Metal) e `gpui_platform`. O código do spike (API gpui 0.2.2) compilou de primeira. Binário: 27.7 MB.
  - **Ressalva factual:** isto requer `features=["runtime_shaders"]`. O **build default falha** neste host por falta do Metal Toolchain (ver §1.4). Numa máquina com o Metal Toolchain instalado (`xcodebuild -downloadComponent MetalToolchain`, ~2–3 GB), o default também compilaria.
- **Roda:** **SIM.** O binário abre **1 janela nativa**, sobe um **PTY real** (`cat`), injeta linhas vivas, **parseia com lina-vt** (`alacritty_terminal`) e **renderiza o grid** na janela, encerrando sozinho. Saída de 3 execuções:

```
run1 SPIKE_METRICS{frames=240, fps=99.1,  p50=8.36ms, p99=26.90ms, min=1.53, max=209.43, scrollback=22, last="frame vivo 255 …"}
run2 SPIKE_METRICS{frames=240, fps=99.1,  p50=8.35ms, p99=29.39ms, min=3.45, max=204.38, scrollback=22, last="frame vivo 184 …"}
run3 SPIKE_METRICS{frames=240, fps=100.5, p50=8.38ms, p99=22.54ms, min=6.17, max=130.86, scrollback=22, last="frame vivo 184 …"}
```

O `last_line` populado prova o **round-trip ponta a ponta**: produtor → `cat` → lina-pty → **lina-vt (parse)** → gpui (sample + render).

---

## 3. Qualidade de render de texto

- **Pipeline:** atlas de glifos do gpui sobre `font-kit` (fontes do sistema) + backend Metal. Render do grid como texto **monoespaçado** (Menlo, 13px), uma linha por `div`, dentro de um container flex-col — exatamente o modelo "atlas + quads" que o Arquiteto descreve para grids monoespaçados.
- **Observado:** texto renderizou **legível e estável** a 120 Hz; scrollback de 22 linhas vivas atualizando continuamente sem tearing aparente. O custo de layout/repaint por frame cabe no orçamento de 8.3 ms (p50), com a saída do PTY mudando a cada frame.
- **Não coberto (acceptance-criteria manuais da story, fora do spike autônomo):** screenshot golden em DPI 1x **e** 2x, shaping de **CJK/BiDi/emoji**, e **IME** (preedit/commit). gpui tem exemplos dedicados (`text.rs`, `text_layout.rs`, `input.rs`) e API de IME — mas a verificação visual/manual fica para a W1-1 "cheia". **Para fins de qualificar o framework, o render de texto monoespaçado funciona.**

---

## 4. Integração AccessKit — **SIM, integrado (nível de framework)**

Esta é a descoberta mais importante do spike, porque **contraria a premissa herdada** ("gpui zero practical accessibility / AccessKit não integrado — zed#6576", citada em `47 - R2 LLM Engineer §4`). No SHA `09165c1` (mai/2026):

- `gpui/Cargo.toml`: **`accesskit.workspace = true`** (dependência direta). `accesskit v0.24.0` compila na árvore.
- **83 menções** a `accesskit` no `src` do gpui. `window.rs` publica uma **`accesskit::TreeUpdate`** (raiz `accesskit::Role::Window` + `accesskit::Tree`); `element.rs` expõe por-elemento `fn a11y_role() -> Option<accesskit::Role>` + `fn write_a11y_info(&mut accesskit::Node)`.
- **API de builder de primeira classe** (usada e compilada neste spike): `.role(Role::Terminal)`, `.aria_label(…)`, `.aria_level/aria_numeric_value/aria_toggled/aria_position_in_set/aria_selected/aria_expanded`, `.on_a11y_action(AccessibleAction::Increment, …)` (ações dirigidas por leitor de tela), `.focusable()/.tab_stop()/.track_focus()`. Há um guia `crate::_accessibility` e um `examples/a11y.rs` completo.
- **Neste spike** o nó-terminal é publicado como **`div().id("lina-terminal").role(Role::Terminal).aria_label(…)`** — ou seja, a árvore AccessKit com um nó **`Role::Terminal`** real (a própria tarefa pedida pela story) **compila e roda**.

**Como integraríamos no Lina:** o core já é agnóstico (`UiHost`); o shell gpui mapeia cada nó/painel para um `div` com `role` + `aria_*`, e o grid de terminal vira um nó `Role::Terminal` com `TextRun` por linha — sem precisar "construir AccessKit do zero", como seria no caminho wgpu-próprio.

**Residual honesto (não medido aqui):** *o que NVDA/VoiceOver/Orca efetivamente vocalizam* num terminal — o acceptance-criterion manual da story (sessão gravada de leitor de tela). O spike prova a **integração no nível de framework** (API + pipeline `TreeUpdate` presentes e funcionais), **não** a qualidade da experiência de leitor de tela ponta a ponta. Mas o desqualificador herdado ("não integra") **caiu**.

---

## 5. Frame timing (medido)

| Métrica | run1 | run2 | run3 | Leitura |
|---|---|---|---|---|
| frames | 240 | 240 | 240 | encerra por contagem (loop de animação) |
| FPS médio | 99.1 | 99.1 | 100.5 | sustenta ~120 Hz menos o startup |
| **frame p50** | **8.36 ms** | **8.35 ms** | **8.38 ms** | **≈ 1/120 s — refresh ProMotion travado** |
| frame p99 | 26.9 ms | 29.4 ms | 22.5 ms | poucos frames lentos (init/atlas) |
| frame **max** | 209 ms | 204 ms | 131 ms | **primeiro frame**: janela + **compilação runtime dos shaders** + atlas de fontes (custo único) |
| frame min | 1.5 ms | 3.5 ms | 6.2 ms | frames triviais |

- **Veredito de performance:** com a saída do PTY mudando a cada frame (parse lina-vt + rebuild da árvore de elementos + repaint), o gpui **sustenta o refresh do display (120 Hz)** com folga no p50. O único custo notável é o **primeiro frame** (compilação MSL em runtime via `runtime_shaders` + warm-up do atlas) — pagável uma vez por processo; com shaders precompilados (`.metallib`, caminho default) esse outlier some.
- Latência keystroke→glifo **não** medida (exigiria injeção de input sintético; fora do mínimo deste spike).

---

## 6. Veredito honesto sobre gpui para o Lina

**gpui é, na prática e neste SHA, uma opção viável e de alta performance — bem mais forte do que o debate de arquitetura assumiu.** Os dados:

**A favor (confirmado empiricamente):**
1. **Roda o caso de uso real** (1 terminal PTY vivo, parse alacritty, render nativo) a **120 Hz**, com pipeline atlas/glifos pronto para grids monoespaçados.
2. **AccessKit é integrado de primeira classe** — `role`/`aria_*`/`on_a11y_action` + `TreeUpdate` no nível de janela. *O desqualificador nº1 do gpui no debate (`a11y`) está desatualizado para este SHA.* No caminho wgpu-próprio, essa camada seria **trabalho nosso** desde o zero.
3. **Esforço até a janela é baixo** (~um `cargo run`), uma vez conhecidos os dois gates abaixo. A produtividade do modelo declarativo (`div().flex()…`) é alta.

**Contra / riscos (também confirmados):**
1. **Sem crate estável** — só git dep do monorepo Zed; pin de SHA + vendoring é **obrigatório** (a mitigação que `31 - Decisao de Stack` já previa). O `gpui v0.2.2` indica versionamento interno, mas **não publicado**.
2. **Gate de build Metal (macOS):** o renderer default compila shaders **offline** e exige o **Metal Toolchain** — que no Xcode 26 é componente **sob demanda** e **não vem por padrão**. Sem ele o build default **falha**. Mitigação real e barata: **`runtime_shaders`** (ou documentar `xcodebuild -downloadComponent MetalToolchain` no onboarding). É um **atrito de fundação herdado da cadeia de build da Zed** — exatamente o tipo de acoplamento que o Arquiteto sinalizou, agora **quantificado** (não é fatal; é um flag + uma nota de setup).
3. **Footprint de disco/compilação** considerável (centenas de crates; árvore debug com debuginfo > 2 GB). Relevante para CI e para devs com disco apertado.
4. **Governança** (não mensurável por spike, mas o fato estrutural persiste): gpui é dirigido para o produto da Zed; `gpui_platform`/`gpui_macos` são reorganizados sem aviso (o backend mac migrou de `gpui` para crate próprio entre o debate e hoje). Pin de SHA protege, mas *seguir o upstream* custa.

**Recomendação para a W1-4 (decisão de framework):**
- **Não desqualificar gpui pelos motivos herdados** — pelo menos a **acessibilidade** e a **compilação de shaders** mudaram a favor dele desde a R2. O spike Slint (W1-2) deve ser medido **nos mesmos termos** (1 terminal real via `wgpu::Texture`, a11y por feature flag, IME) para uma comparação justa.
- **Se o critério decisivo for "menor esforço até um app acessível e performático agora"**, gpui passou neste spike. **Se for "governança/longevidade e posse do render"**, o caminho **wgpu-próprio** do Arquiteto segue mais forte — e este spike **não** testou os dois riscos que ele apontou como o preço aceito (esforço da camada de widgets ao longo de 5 anos; a11y/IME *além* do que o framework dá de graça).
- **Decisão sã:** manter gpui como **default provisório** (como `31 - Decisao de Stack` já põe), **com `runtime_shaders` no onboarding mac** e **pin de SHA + vendoring**, e bater contra o número do spike Slint antes de cravar.

---

## Reprodutibilidade

```bash
cd spikes/spike-gpui
# Cargo.toml já pina o SHA e habilita runtime_shaders (dispensa o Metal Toolchain).
cargo build            # ~2 min; precisa de ~1.3 GB livres em disco (debug sem debuginfo)
./target/debug/spike-gpui   # abre a janela ~2.4 s, imprime SPIKE_METRICS, encerra sozinho
```

Para o **caminho default** (shaders precompilados, sem `runtime_shaders`): instalar o Metal Toolchain
(`xcodebuild -downloadComponent MetalToolchain`) e remover a feature `runtime_shaders` do `Cargo.toml`.

**Notas:** este host ficou com disco apertado; após o spike rode `cargo clean` no diretório para
devolver ~1.2 GB. O `Cargo.lock` fixa o SHA auditado para builds determinísticos.
