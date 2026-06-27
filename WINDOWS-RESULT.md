# WINDOWS-RESULT — relatório do bring-up Windows

> Confirmações visuais do gpui **devem** vir do Eduardo olhando a tela (o app não roda headless).
> Tudo mais é evidência por exit code + log.

## Sessão 2 — 2026-06-27 (Eduardo)

**Máquina:** Windows (`C:\Users\lucas\`) — `PROCESSOR_ARCHITECTURE=AMD64` (x86_64) ✅
**Branch:** `windows-bringup` (criada limpa de `origin/main`)
**Commit base:** `9787765 perf(a2a): entrega viva não congela mais a UI (freeze do lina ask)`
**Por que recomeçar:** a tentativa anterior (2026-06-04, abaixo) recomendava pivot pra Slint, mas
desde então o Rafael mergeou fixes mainstream que podem ter atacado o sintoma "terminal sem prompt"
da Fase 2 antiga (notadamente `9787765 perf(a2a)` e `24ad732 fix(render)`). Refazer a Fase 2 do zero
no main atual decide se ainda vale o pivot — se passar, segue até o `.exe`.

### Fase 0 — Ambiente ✅

- Rust: `rustc 1.95.0` (host `x86_64-pc-windows-msvc`, target instalado idem)
- MSVC: `Visual Studio 2022 BuildTools` em `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`
- Windows SDK: `10.0.26100.0`
- git OK, gh OK (`v2.92.0`, logado em `EduargoGrigolo`)
- **Sem FALTA pendente.** `check-env.ps1` retornou `EXIT=0`.
- Evidência: `packaging/windows/check-env.log`

### Fase 1 — Build do app gpui ✅

- `cargo build` → **BUILD_EXIT=0**
- `Finished dev profile [unoptimized] target(s) in 54.57s` (cache quente desde a tentativa de junho)
- Binário: `app/lina-gpui/target/debug/lina-gpui.exe` (48 195 584 bytes ≈ 48 MB)
- Crate `gpui_windows v0.1.0` (zed rev `09165c15`) compilou — sinal de que o backend Windows do gpui foi puxado
- Feature set: `target.'cfg(not(target_os = "macos"))'` herdado do `main` — sem ajuste necessário
- 4 warnings de `dead_code` em `app/lina-gpui/src/main.rs` (funções `path_looks_hydrated`, `known_cli_dir_candidates`, `existing_dirs`, `parse_marked_path`) — não bloqueiam; vivem fora do caminho de produção
- **Ajuste necessário pra rodar:** 1 linha em `packaging/windows/build.ps1` — `$ErrorActionPreference = "Stop"` → `"Continue"`. Sem isso, o PS 5.1 trata stderr do cargo como `NativeCommandError` fatal sob `Tee-Object` e mata o pipeline; o script reporta `BUILD_EXIT=0` mentiroso. Fix vai pro PR como parte do bring-up.
- Evidência: `packaging/windows/build-win-host.log` (host), `build-win.log` (tee do script)

### Fase 2 — DE-RISK (Eduardo olha a tela) 🔑

- (a iniciar; será o checkpoint que decide se segue ou para)

### Fase 3 — ConPTY

- (depende da Fase 2 PASSAR)

### Fase 4 — Empacotamento

- (depende da Fase 2 PASSAR)

### Fase 5 — PR

- (no fim)

---

## Histórico — Sessão 1 (2026-06-04, Gabriel olhando a tela)

> Tentativa anterior, em outra branch (`windows-bringup` original — descartada quando o merge
> ficou ruim e essa nova branch nasceu limpa de `origin/main`). Preservado aqui pra contexto.

**Resumo:** Fase 0 OK (instalou Rust + Build Tools 2022 + SDK 26100; Smart App Control teve que ser
desativado pra build scripts do Cargo passarem — antes disso bloqueava com `os error 4551`).
Fase 1 OK (`lina-gpui.exe` 35.6 MB gerado em `C:\Temp\lina-target\debug\`).
**Fase 2 FALHOU**: janela abriu e desenhou cards "Terminal A" / "Terminal B", mas a área do terminal
ficou vazia (sem prompt, texto ou cursor); input do Gabriel não chegava ao terminal visivelmente.
`Ctrl+T` criou novo agente (passa); acentos não testáveis sem input. FPS variou 26–120.
**Veredito daquela sessão:** pivotar pra Slint preservando `UiHost`.

Ajustes Windows tentados naquela sessão (não estão nesta branch, foram descartados com a branch ruim):
1. `shell_cmd` no Windows passou de `sh` (inexistente) → `powershell.exe` → `cmd.exe /K`
2. `Ctrl+T` passou a aceitar `control` no Windows
3. Fallback Windows pra `key_char` imprimível caso IME não chame `replace_text_in_range`

Se a Fase 2 desta sessão falhar com os MESMOS sintomas, esses ajustes valem ser reaplicados antes do
pivot — mas a ideia é primeiro rodar o main puro pra ter linha de base.
