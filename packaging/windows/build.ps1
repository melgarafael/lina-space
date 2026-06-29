# Lina Windows — build do app gpui (debug). Rode:
#   powershell -ExecutionPolicy Bypass -File packaging\windows\build.ps1
# O Cargo.toml ja gateia `runtime_shaders` p/ macOS-only (bloco por-SO). Se o build reclamar de
# feature/backend do gpui-Windows, ajuste o bloco [target.'cfg(not(target_os = "macos"))'.dependencies]
# do app/lina-gpui/Cargo.toml (tente default-features=true, ou a feature que o erro indicar) e re-rode.
$ErrorActionPreference = "Continue"   # cargo escreve em stderr; "Stop" + Tee-Object dispara NativeCommandError e mata o pipeline antes do build terminar
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)   # packaging\windows -> repo
$log  = Join-Path $repo "build-win.log"
Set-Location (Join-Path $repo "app\lina-gpui")

Write-Host "=== cargo build (app gpui, Windows) - log: build-win.log ===" -ForegroundColor Cyan
Write-Host "    (1a vez e LENTA: a arvore gpui e grande, ~10-20 min)"
cargo build *>&1 | Tee-Object -FilePath $log
$code = $LASTEXITCODE
Write-Host ""
Write-Host "BUILD_EXIT=$code"
if ($code -eq 0) {
    Write-Host "Build OK. Proximo: rode 'cargo run' (ou '`$env:LINA_DEMO=1; cargo run') e faca o DE-RISK" -ForegroundColor Green
    Write-Host "(o AMIGO olha a tela: renderiza? acentos '`'+a=a'? Ctrl+T cria terminal?). Registre no WINDOWS-RESULT.md."
} else {
    Write-Host "Build FALHOU. Veja build-win.log." -ForegroundColor Red
    Write-Host "Se for feature do gpui-Windows ausente, ajuste o bloco por-SO no Cargo.toml e re-rode."
}
exit $code
