# Pedido de costura — r2 F2-1-4 (Terminal C → Maestro)

> Lado token da F2-1-4 entregue em `theme.rs` (suíte completa 415/0 + clippy -D warnings + fmt só
> no meu arquivo; só theme.rs na árvore). O que existe pronto para fiar + decisões que são suas.

## O que o theme.rs agora oferece (API pronta, testada)

1. **`ModeSetting`** (`Sistema` default | `Escuro` | `Claro`): `from_setting`/`as_setting`
   (strings `"sistema"`/`"escuro"`/`"claro"`) + **`resolve(system_is_dark) -> Mode`**
   (matriz testada). `Mode`/`apply()` intactos — zero quebra nos consumidores atuais.
2. **Ajustes parciais por token**: `set_overrides(BTreeMap<String,String>)` (mapa inteiro,
   atômico) · `Theme::refine()` · `TOKEN_PATHS` (30 caminhos pt-br, teste 1:1 escreve-e-lê) ·
   seção `ajustes` no `tema.json` (`"superficie.canvas": "#0a0e27"`); desconhecido/inválido
   ignorado; ajustes SOBREVIVEM a `apply()` (teste) e saem no export quando existem.
3. **`Theme.accent_name`**: o nome canônico do acento vivo (p/ exibir nos Ajustes e re-construir).
4. `parse_prefs(json) -> ThemePrefs` completo (modo/acento/tipografia/movimento/ajustes).

## Diffs de fiação (arquivos seus)

```text
main.rs (boot, ~linha do theme::apply atual)
  let setting = theme::ModeSetting::from_setting(&s.theme);
  let is_dark = matches!(cx.window_appearance(),
      WindowAppearance::Dark | WindowAppearance::VibrantDark);
  theme::apply(setting.resolve(is_dark), &s.accent);

main.rs (observer — o "segue o sistema" AO VIVO; gpui: Window::observe_window_appearance)
  // no setup da janela: se setting == Sistema, re-resolver e theme::apply no callback.

persistence_ui.rs (Ajustes): 3º estado no toggle de modo → persistir "sistema".
persistence_ui.rs (import_theme): trocar parse_json por parse_prefs e aplicar:
  modo (ModeSetting) · acento · movimento.reduzir → theme::set_reduce_motion ·
  ajustes → theme::set_overrides (mapa ausente ⇒ BTreeMap::new(), limpa — import é atômico).
```

`#[allow(dead_code)]` removíveis ao fiar: `ModeSetting`, `set_overrides`, `set_reduce_motion`,
`MotionTokens::effective`, `TOKEN_PATHS`.

## Decisões que são SUAS (2)

1. **Persistência dos ajustes entre sessões:** sugiro gravar `ajustes` no `settings.json`
   (fonte única; `tema.json` segue sendo veículo de export/import) — alternativa é reler
   `tema.json` no boot, mas aí o arquivo vira estado vivo sem o usuário saber.
2. **Migração do legado:** `settings.json` com `"escuro"` é indistinguível entre
   default-nunca-escolhido e escolha explícita. Opções: (a) migração única `escuro→sistema`
   (honra a decisão F2-0-D nº2 como default; quem queria escuro fixo re-escolhe em 1 clique);
   (b) legado intacto, `sistema` só para instalações novas. A decisão do fundador diz "chrome
   SEGUE O SISTEMA" como default do produto — (a) é a leitura forte; (b) a conservadora.

## Régua (gate F2-1 — estado)

WCAG estendido verde nos 2 modos × 8 acentos (gate intacto; ajustes de usuário são liberdade
"Avançado" — o gate protege os DEFAULTS curados, doc do `refine` explica). Pendentes da onda:
F2-1-1b (fonte do grid, atômica) e F2-1-5 (catraca, r2 — base de tokens pronta).
