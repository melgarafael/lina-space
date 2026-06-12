# Entrega D2 — Design system em gpui (viabilidade e arquitetura)

> **Dono:** Terminal A (ARQUITETO) · **Data:** 2026-06-12 · **Método:** interna-primeiro (13.3 §5, ADR 0019/0028, código real do shell) → externa com fonte primária (código do Zed, gpui-component, COSMIC/Slint/Iced/DTCG), tudo fetch real datado, refutação tentada por achado.
> **Restrições confrontadas:** R1 (gpui pinned), R2 (core/shell split — tokens não soldam o core ao gpui), R7 (perf 120Hz), R8 (anti-slop), R10 (ADR 0019 §7 estética operacional).

---

## Achados

### A1 — O Lina JÁ TEM um design system de COR maduro; a dívida real é tipografia/espaçamento [LOAD-BEARING]
- **CLAIM:** `app/lina-gpui/src/theme.rs` (695 linhas, F1-2-1 entregue) é fonte única de cor: 6 grupos de tokens (`SurfaceTokens`/`TextTokens`/`AccentTokens`/`StateTokens`/`FocusTokens`/`TerminalTokens`), `ColorScale` dark/light, 8 acentos curados, tema vivo sem restart, export/import JSON local tolerante a campos futuros. Gates em CI: WCAG AA por modo×acento, focus ring ≥3:1, **lint que falha o build se `rgb(0x…)` aparecer fora de theme.rs** (hoje: 0 violações). O que NÃO existe: tokens de tipografia (68 `.text_size(px(…))` inline; 31 `FontWeight::` inline; grid em **Menlo 13px hardcoded**, `main.rs:90-91`), de espaçamento (58 `w/h(px(…))` + 322 utilitários sem escala; top ofensores: `agent_modal.rs` 61, `main.rs` 47, `persistence_ui.rs` 26), de radius/elevação/motion.
- **FONTE:** auditoria direta do repo (`app/lina-gpui/src/theme.rs`, `sidebar.rs`, `agent_modal.rs`, `main.rs`, `persistence_ui.rs`; `.github/workflows/ci.yml` L116-137) · **DATA:** 2026-06-12 · **CONFIANÇA:** alta (grep + leitura).
- **REFUTAÇÃO:** tentei achar cor hardcoded fora de theme.rs (invalidaria "fonte única") — 0 ocorrências; o lint de CI garante que não regride. O furo tipografia é real: contradiz o ADR 0019 §7 ("JetBrains Mono via tokens do design system, nunca fonte hard-coded").

### A2 — Modelo Zed: DEFINIÇÃO de tokens é código; VALORES são dados (JSON refinável) [LOAD-BEARING]
- **CLAIM:** O Zed separa em duas camadas: (a) structs Rust definem o vocabulário — `ThemeColors` com ~150 campos `Hsla` semânticos em `crates/theme/src/styles/colors.rs`; componentes consomem via `cx.theme().colors().<token>` (trait `ActiveTheme`, tema como Global do gpui); (b) valores vêm de JSON (`assets/themes/*.json` embarcados; user themes em `~/.config/zed/themes/`, schema gerado por schemars), aplicados por **`Refineable`** (`theme_overrides` sobrescreve tokens individuais sem trocar o tema). Dark/light/system: `ThemeSelection::Dynamic { mode: Light|Dark|System, light, dark }` — System é default, dirigido por `WindowAppearance` do gpui. Consequência: **tema novo entra sem recompilar; token novo exige recompilar** — fronteira nítida e deliberada.
- **FONTE:** https://github.com/zed-industries/zed/blob/main/crates/theme/src/styles/colors.rs · /crates/theme/src/registry.rs · /crates/settings_content/src/theme.rs · /docs/src/themes.md · /assets/themes/one/one.json · **DATA:** fetch 2026-06-12 (main) · **CONFIANÇA:** alta.
- **REFUTAÇÃO:** procurei "tokens 100% dados" no Zed (invalidaria o híbrido) — não existe: o vocabulário é sempre struct Rust; o JSON só preenche. 13.3 §5 (interno, 2026-06-05) já apontava "200+ tokens nomeados" — confirmado e refinado (~150 cores + status/players/syntax).

### A3 — Licenças: crates `theme` e `ui` do Zed são GPL-3.0 → MODELAR, jamais copiar; gpui é Apache-2.0 [LOAD-BEARING]
- **CLAIM:** `crates/theme` e `crates/ui` do Zed: `license = "GPL-3.0-or-later"` (copyleft forte — copiar código contaminaria o Lina). `crates/gpui`: Apache-2.0 (uso livre). Arquitetura/ideias (registry, tokens semânticos, Refineable, StyledExt/elevations, builder+RenderOnce) não são protegidas — reimplementação limpa a partir do design é o caminho.
- **FONTE:** https://github.com/zed-industries/zed/blob/main/crates/theme/Cargo.toml · /crates/ui/Cargo.toml · /crates/gpui/Cargo.toml · **DATA:** fetch 2026-06-12 · **CONFIANÇA:** alta (campos lidos direto).
- **REFUTAÇÃO:** zona cinzenta seria tradução linha-a-linha disfarçada — mitigação: implementar a partir DESTA entrega (descrição de arquitetura), não com o código aberto ao lado.

### A4 — gpui-component (longbridge): vivo e usado em produção, mas tracka o HEAD do Zed → incompatível com nossa filosofia de pin; MODELAR, não DEPENDER [LOAD-BEARING]
- **CLAIM:** 60+ componentes, theming multi-tema, Table/List virtualizados; 11,6k stars, 102 contributors, commit em 2026-06-12 (hoje); produção real: Longbridge Pro; autor é contributor ativo do Zed. PORÉM: `Cargo.toml` declara `gpui = { git = "…/zed" }` **sem rev** (persegue HEAD); o issue tracker confessa quebras recorrentes por upstream (jun/2025, jul/2025, dez/2025, jan/2026, mai/2026); breaking renames entre minors (0.4.0: Modal→Dialog etc.); releases crates.io paradas desde fev/2026 (0.5.1) — só main anda. Nosso contrato é o oposto: pin de SHA + vendoring. **Correção de premissa:** nosso pin `09165c15` é o Zed de **2026-05-30** (não "de 2025") — a 3 dias do lock atual deles (`b077f41a`, 2026-06-02), então um spike de compatibilidade é plausível HOJE, mas a janela degrada com o tempo.
- **FONTE:** https://github.com/longbridge/gpui-component (+ issues via API) · https://crates.io/api/v1/crates/gpui-component/versions · https://zed.dev/blog/community-champion-jason-lee (2026-04-27) · raw Cargo.toml/Cargo.lock · GitHub API commit 09165c15 · **DATA:** 2026-06-12 · **CONFIANÇA:** alta nos fatos; **SUSPECT:** compatibilidade pin↔main inferida por datas, **não compilada**; licença Apache-2.0 do gpui-component lida só no README, não confirmada no manifest.
- **REFUTAÇÃO:** tentei refutar "vivo" (procurei sinais de abandono) — falhou: commits diários. Tentei refutar "frágil para nós" — confirmou: 5 issues históricas de quebra por upstream em 12 meses.

### A5 — Padrão convergente multi-backend: VALORES de tokens como dados serializados versionados; derivação e aplicação como código por shell [LOAD-BEARING]
- **CLAIM:** COSMIC DE (System76): `cosmic-theme::Theme` é struct serde `#[version = 1]` persistida como RON; usuário exporta/importa tema-arquivo; `ThemeBuilder` deriva paleta completa de poucas cores-base (OKLCH) **em código**. Slint: um shell Slint consome tokens por propriedades/globals setados do lado Rust (confirmado em discussion oficial #5860) — ou seja, tokens-como-dados no Rust alimentam Slint de graça; estilos builtin são compile-time. Iced: tokens como código (enum Theme + Palette derivada). W3C DTCG: spec **estável desde 2025-10-28** (JSON `.tokens.json`), mas **zero tooling Rust maduro** (único crate é de out/2023, pré-spec) — adotar o formato integral hoje é aspiracional; emprestar o vocabulário é barato.
- **FONTE:** https://raw.githubusercontent.com/pop-os/libcosmic/master/cosmic-theme/src/model/theme.rs (código lido) · https://github.com/slint-ui/slint/discussions/5860 · https://docs.rs/iced/latest/iced/enum.Theme.html · https://www.w3.org/community/design-tokens/2025/10/28/design-tokens-specification-reaches-first-stable-version/ · **DATA:** fetch 2026-06-12; DTCG 2025-10-28 · **CONFIANÇA:** alta (COSMIC/Slint/DTCG primárias); média-alta Makepad (não-essencial).
- **REFUTAÇÃO:** verifiquei se o RON do COSMIC era só export cosmético (invalidaria "tokens-como-dados") — não: o runtime carrega da entidade serializada.

### A6 — ADR 0028 constrange o catálogo: componente que anuncia mudança precisa nascer sobre o custom Element de live-region [LOAD-BEARING interno]
- **CLAIM:** O gpui do pin não expõe `aria_live` (15 campos `aria_*`, zero live) → auto-anúncio a11y exige custom `Element` no shell (caminho (a) do ADR 0028, DRAFT). Qualquer componente do catálogo F2 que comunica estado (toast, badge de status, progresso) deve compor com esse Element desde o design — retrofit depois é caro. Complementa R6 (a11y 1ª classe não regride).
- **FONTE:** `docs/adr/0028-live-region-caminho.md` (DRAFT 2026-06-10) + spike live-region da r1 ("sem patch no pin") · **DATA:** 2026-06-12 · **CONFIANÇA:** alta no constrangimento; ADR ainda DRAFT (selar antes do épico).
- **REFUTAÇÃO:** o upstream pode ter ganhado `set_live` desde o pin — não coletado (lacuna L3; checar a cada bump, como o próprio ADR manda).

### A7 — Ecossistema gpui além do gpui-component: nada para depender [confiança média]
- **CLAIM:** awesome-gpui (lista oficial Zed) não tem lib de design tokens dedicada; alternativas (adabraka-ui parado ~4 meses; gpui-storybook 6 stars; create-gpui-app parado desde abr/2025) são embriões — no máximo modelar. A ideia "storybook/galeria de componentes" é útil como ferramenta interna nossa (o Zed tem `RegisterComponent`/preview; o 13.3 §5 já marcava o right-click inspector como "UX gold", semente P2).
- **FONTE:** https://github.com/zed-industries/awesome-gpui + GitHub API stats por repo · **DATA:** 2026-06-12 · **CONFIANÇA:** média (ausência provada por busca, não exaustiva).
- **REFUTAÇÃO:** busquei "gpui design tokens/theme lib" além da lista — nada novo encontrado.

---

## Recomendação de arquitetura (posições a–d)

### (a) Tokens: DEFINIÇÃO em código toolkit-free + VALORES como dados — e NÃO no lina-core
**Posição:** o falso dilema é "core vs shell". O padrão vencedor (Zed A2, COSMIC A5) é: **vocabulário de tokens = structs Rust toolkit-free; valores = documento serializado versionado**. Nosso `theme.rs` já está 80% lá (gpui-free, testável sem gpui, export/import JSON tolerante). O que a F2 faz:
1. **Completar o vocabulário**: `TypographyTokens` (família/tamanhos/pesos — JetBrains Mono como manda o ADR 0019 §7), `SpacingTokens` (escala 4/8/12/16/24/32), `RadiusTokens`, `MotionTokens` (durações + reduce-motion) — mesmas garantias da cor (gates CI).
2. **Valores como dados**: o JSON de export/import vira o documento canônico de tema (campos `serde(default)`, aditivo — replay/import antigo nunca quebra, mesma doutrina dos eventos). Vocabulário inspirado no DTCG (nomes/tipos), **sem** adotar o formato integral (A5: sem tooling Rust, ganho nulo hoje).
3. **Onde mora**: extrair `theme.rs` para um crate `lina-theme` **fora de `crates/lina-*` do core** (ex.: `app/lina-theme` ou um diretório `ui/` irmão) — toolkit-free ≠ assunto do core. Tema não é domínio event-sourced; é config de apresentação. Isso preserva o invariante da F1-2-1 ("nenhum token atravessa o UiHost") E a porta Slint: um shell Slint futuro consome o MESMO crate setando globals via Rust (A5, confirmado viável). **Trade-off aceito:** extração pode esperar a 1ª onda da F2 (hoje só o shell gpui consome; extrair cedo é cerimônia sem cliente) — mas as famílias novas de tokens já nascem no módulo gpui-free.
**Refuta a alternativa:** tokens 100% no shell-como-código (status quo de spacing/fonts) fere R2 na prática — a porta Slint pagaria a reescrita inteira da paleta; tokens no `lina-core` é a cerimônia oposta (sem cliente, e suja o core com apresentação).

### (b) Dark/light + customização futura
Já temos dark/light vivo (2 modos × 8 acentos, sem restart). O que falta e o gpui do pin suporta: **modo "sistema"** via `WindowAppearance` (o Zed usa exatamente isso no nosso rev — A2). Customização do usuário: adotar o padrão **refinement** do Zed — overrides parciais por token sobre um tema base, persistidos no nosso JSON (nosso import tolerante já aponta para isso). Roadmap: F2 entrega modo sistema + overrides; theme builder visual/inspector fica como semente (13.3 §5 "UX gold", P2) — o ranqueamento final é do épico com o eval D0.

### (c) Catálogo de componentes: construir o nosso, pequeno, modelado no Zed — não adotar lib
**Custo real é baixo**: o padrão gpui é builder + `RenderOnce` (A2), e os componentes JÁ EXISTEM espalhados no shell (modal, sidebar rows, badges, toasts) — o trabalho da F2 é **consolidar** os repetidos num módulo `ui/` interno consumindo tokens, não criar do zero. Núcleo proposto: botão, painel/card, menu/dropdown, badge/indicator, toast (sobre o Element de live-region — A6), input, modal. O que o Zed dá de graça **por modelagem** (A3: GPL → só design): elevações semânticas (`StyledExt`: Surface→Elevated→Modal), trait `ActiveTheme`, padrão builder, taxonomia de ~60-70 componentes como mapa do que existe. gpui-component (A4) é o 2º corpus de referência; **exceção tática** se um componente caro for necessário (Table virtualizada): spike de 1 dia com `[patch]` para o nosso vendor + **vendor-and-own** do componente específico — nunca dep viva.

### (d) Migração incremental, sem big-bang (refatoração por onda — espelha o mecanismo provado da cor)
1. **Onda 1 — vocabulário**: famílias novas de tokens (typography/spacing/radius/motion) + modo sistema. Gate: testes de integridade + WCAG estendido. Nada de UI muda ainda.
2. **Onda 2 — catálogo**: consolidar componentes núcleo no módulo `ui/` consumindo tokens (live-region embutido). Gate: cada componente novo substitui TODAS as instâncias do padrão que consolida.
3. **Onda 3 — varredura com ratchet**: substituir magic numbers arquivo-a-arquivo, top ofensores primeiro (`agent_modal.rs` 61 → `main.rs` 47 → `persistence_ui.rs` 26); **teste-catraca** no espírito do lint de cor: a contagem de `px(…)` inline fora de tokens só pode CAIR (snapshot por arquivo; CI falha se subir). É o mesmo mecanismo que manteve cor 100% limpa (A1) — provado no repo.
4. **Onda 4 (condicional)**: extração do crate `lina-theme` quando houver 2º cliente OU quando o épico decidir antecipar a porta Slint. R7: cada onda passa pela sonda `[PROF]` (token nunca vira indireção em hot path de render do grid — resolver no build da cena, não por célula).

---

## CONFLITOS
- **ADR 0019 §7 × realidade do código**: o ADR manda JetBrains Mono "via tokens", o grid usa Menlo 13px hardcoded (`main.rs:90-91`). Não é conflito de pesquisa — é dívida nomeada; a Onda 1(d) fecha.
- **13.3 §5 (interno) recomendava "modelar GPUI Component theming"** × A4 desta entrega: confirmado e REFINADO — modelar sim, mas o tracking-de-HEAD deles agora está documentado como razão dura para nunca depender (o 13.3 não tinha lido o issue tracker).
- **Invariante F1-2-1 "tema é assunto exclusivo do shell" × recomendação (a)**: sem conflito real — "toolkit-free" mantém o tema fora do core e fora do UiHost; só muda o endereço do módulo quando houver 2º cliente.

## LACUNAS
1. Spike de compilação gpui-component@main contra nosso pin `09165c15` + vendor (`gpui_platform`/`gpui_macros` inclusos?) — inferido por datas, não provado (só importa se a exceção tática (c) for acionada).
2. `theme_importer` do Zed (conversor VS Code→Zed) não avaliado — relevante se quisermos importar temas de ecossistemas existentes.
3. Upstream gpui pós-pin: ganhou `set_live`/theming nativo? Checar a cada bump (ADR 0028) — ninguém coletou.
4. ADR 0028 ainda DRAFT — selar antes da Onda 2(c) (toast/badge dependem do Element).
5. Tema claro de terminal: decisão interna "terminal sempre escuro" sem benchmark externo (Warp/Ghostty) — pesquisa fina se o épico contestar.
6. Licença do gpui-component lida só no README (SUSPECT) — confirmar no manifest antes de qualquer vendor-and-own.

## RECÊNCIA
Fetches externos: todos 2026-06-12 (Zed main; gpui-component com commit do próprio dia; APIs GitHub/crates.io cruas). DTCG: spec estável 2025-10-28. Internos: 13.3 de 2026-06-05; ADR 0019 aceito 2026-06-06; ADR 0028 DRAFT 2026-06-10; auditoria de código sobre o HEAD `a061222` (CI 3-SO verde de 2026-06-12). Domínio rápido (ecossistema gpui): tudo 2025+, regra atendida.

PRONTO: arquitetura recomendada — vocabulário de tokens em código toolkit-free (completar typography/spacing/radius/motion no theme.rs gpui-free existente), valores como JSON versionado aditivo, catálogo próprio pequeno modelado no Zed (GPL proíbe copiar; gpui-component vivo mas tracka HEAD → modelar, nunca depender), migração em 4 ondas com teste-catraca espelhando o lint de cor já provado.
