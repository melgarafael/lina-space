# Pedido de costura — r2 F2-1-4 v2 (Terminal C → Maestro) — ATUALIZA o pedido anterior

> Sua costura `47d52d8` (boot + observer no main.rs) aterrissou ENQUANTO eu fechava o lado
> persistence_ui — obrigado, ela já funciona. Este v2 substitui o pedido anterior: sobraram
> **3 trocas de linha no main.rs** que eliminam uma janela de estado VELHO, + 2 linhas de boot
> para os ajustes/reduce-motion persistidos. Meu lado (theme.rs + persistence_ui.rs) está
> completo: suíte 417/0 + catraca 2/2 + clippy -D warnings + fmt só nos meus 2 arquivos.

## Por que trocar (bug real de staleness no observer atual)

O observer de `main.rs:5073-5078` captura `setting` e `s.accent` NO BOOT (closure `move`).
Se o usuário trocar modo/acento nos Ajustes depois, o observer continua com os valores velhos:
na próxima mudança de aparência do SO ele re-aplica a PREFERÊNCIA DO BOOT por cima da escolha
nova (ex.: usuário escolheu "claro" explícito → SO muda → tela volta a seguir o sistema).
Além disso, o `apply_setting` novo resolve contra um carimbo interno (`SYSTEM_IS_DARK`) que o
observer atual não atualiza — os dois mundos precisam convergir no MESMO ponto de entrada.

## Diff mínimo (main.rs, 2 pontos)

```text
BOOT (main.rs:~4792)
  ANTES  theme::apply(setting.resolve(true), &s.accent);
  DEPOIS theme::set_overrides(s.theme_overrides.clone());   // ajustes persistidos (F2-1-4)
         theme::set_reduce_motion(s.reduce_motion);          // a11y persistido (F2-1-3)
         theme::apply_setting(setting, &s.accent);           // carimbo interno default=dark
                                                             // (mesmo chute honesto de hoje)

OBSERVER (main.rs:~5073-5078)
  ANTES  .observe_window_appearance(move |window, _cx| {
             ... theme::apply(setting.resolve(is_dark), &s.accent);   // setting/accent VELHOS
         })
  DEPOIS .observe_window_appearance(|window, _cx| {
             let is_dark = matches!(
                 window.appearance(),
                 WindowAppearance::Dark | WindowAppearance::VibrantDark
             );
             theme::set_system_appearance(is_dark);   // sem captura: o estado vivo decide
         })
  (set_system_appearance só re-aplica quando a preferência VIVA é Sistema — testado; com
   Escuro/Claro explícitos apenas guarda o carimbo. O 1º fire do observer corrige o chute
   dark do boot, como hoje. Remover o #[allow(dead_code)] de set_system_appearance ao fiar.)
```

## O que JÁ entreguei no meu lado (não precisa de ação sua além de validar)

- `persistence_ui.rs`: ciclo de modo 3 estados (sistema→escuro→claro) com rótulo leigo
  ("segue o computador"); pastilhas de acento pintam no modo RESOLVIDO; `apply_theme` via
  `apply_setting` (preferência viva sempre fresca); import de `tema.json` COMPLETO (modo
  incl. sistema, acento, reduzir→`set_reduce_motion`, ajustes→`set_overrides`, atômico);
  export grava a preferência ("sistema" sai "sistema"); **migração única** `escuro`→`sistema`
  com marcador `theme_migrated_to_system` ("claro" explícito intocado; "escuro" re-escolhido
  pós-migração respeitado p/ sempre); `theme_overrides` persistem no `settings.json` (decisão
  nº1 do pedido anterior, confirmada por você no chat da fronteira).
- `theme.rs`: `apply_setting`/`set_system_appearance` (estado vivo da preferência + carimbo do
  SO; teste "segue o sistema ao vivo / explícito ignora"); API legada de parse do `Mode`
  REMOVIDA (um único caminho: `ModeSetting`; `Settings::theme_mode()` saiu junto — o último
  consumidor era o boot antigo).

## Testes que provam o critério (novos nesta fatia)

`system_mode_follows_appearance_only_when_sistema` · `legacy_escuro_migrates_to_sistema_once` ·
`import_applies_reduzir_and_ajustes_and_persists` · `prefs_export_import_roundtrip` (preferência
"sistema" preservada no export) + atualizações dos existentes para o ciclo de 3 estados.

## Nota de árvore

A catraca da F2-1-5 (worker da r2) flakeou 1× durante meu run (snapshot sendo gravado em
paralelo); re-run limpo 2/2. Não é regressão minha nem dela — é o custo conhecido de validar
em árvore compartilhada; a validação de fora (sua) decide.
