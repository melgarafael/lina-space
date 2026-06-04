# Lina Windows — verificacao de ambiente de build (read-only). Rode:
#   powershell -ExecutionPolicy Bypass -File packaging\windows\check-env.ps1
$ErrorActionPreference = "Continue"
Write-Host "=== Lina Windows - check de ambiente ===" -ForegroundColor Cyan
Write-Host ""

Write-Host "[Arquitetura]"
Write-Host "  PROCESSOR_ARCHITECTURE = $env:PROCESSOR_ARCHITECTURE"
Write-Host "  (precisa AMD64/x86_64 p/ o binario dos alunos; se ARM64, registre e PARE)"
Write-Host ""

Write-Host "[Rust]"
if (Get-Command rustc -ErrorAction SilentlyContinue) {
    rustc -vV
    Write-Host "  targets instalados:"
    $targets = (rustup target list --installed 2>$null)
    $targets | ForEach-Object { Write-Host "    $_" }
    if (-not ($targets | Select-String "x86_64-pc-windows-msvc")) {
        Write-Host "  AVISO: target x86_64-pc-windows-msvc NAO instalado ->" -ForegroundColor Yellow
        Write-Host "         rustup target add x86_64-pc-windows-msvc"
    }
} else {
    Write-Host "  FALTA Rust -> instale via https://rustup.rs (toolchain stable-msvc)" -ForegroundColor Red
}
Write-Host ""

Write-Host "[MSVC / C++ Build Tools]"
if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Write-Host "  cl.exe no PATH (Developer Prompt ativo) - OK"
} else {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vc = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vc) { Write-Host "  VS C++ tools detectado: $vc - OK" }
        else { Write-Host "  FALTA o workload 'Desktop development with C++'" -ForegroundColor Red }
    } else {
        Write-Host "  FALTA: instale 'Build Tools for Visual Studio 2022' com 'Desktop development with C++'" -ForegroundColor Red
        Write-Host "  (gpui NAO suporta MinGW/MSYS2 - tem de ser MSVC)"
    }
}
Write-Host ""

Write-Host "[Windows SDK]"
$inc = "${env:ProgramFiles(x86)}\Windows Kits\10\Include"
if (Test-Path $inc) {
    $sdk = Get-ChildItem $inc -ErrorAction SilentlyContinue | Select-Object -Last 1
    if ($sdk) { Write-Host "  Windows SDK: $($sdk.Name) - OK" }
} else { Write-Host "  Windows SDK nao detectado (vem com o workload C++)" -ForegroundColor Yellow }
Write-Host ""

Write-Host "[git / gh]"
if (Get-Command git -ErrorAction SilentlyContinue) { Write-Host "  git OK" } else { Write-Host "  FALTA git" -ForegroundColor Red }
if (Get-Command gh -ErrorAction SilentlyContinue)  { Write-Host "  gh (GitHub CLI) OK - util p/ abrir o PR" } else { Write-Host "  gh ausente (opcional; PR pode ser pela web)" }
Write-Host ""
Write-Host "=== fim. Resolva os FALTA/AVISO em vermelho/amarelo antes de buildar. ===" -ForegroundColor Cyan
