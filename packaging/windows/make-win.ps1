# Lina Windows — empacota o .exe distribuivel (SEM assinatura). Rode (apos o de-risk PASSAR):
#   powershell -ExecutionPolicy Bypass -File packaging\windows\make-win.ps1
# Monta dist\Lina-win\ (lina-gpui.exe + lina.exe + assets\ [+ DLLs ConPTY se vendorizadas]) + .zip.
# O resolver do app acha 'lina.exe' ao lado do exe e 'assets\' por ancestral - ZERO mudanca de codigo
# (mesmo modelo do Mac). Executavel = 'lina-gpui.exe' (NAO 'Lina.exe' - evita confusao com o CLI).
$ErrorActionPreference = "Stop"
$repo   = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$ver    = "0.1.0"
$dist   = Join-Path $repo "dist\Lina-win"
$gpui   = Join-Path $repo "app\lina-gpui\target\release\lina-gpui.exe"
$lina   = Join-Path $repo "target\release\lina.exe"
$assets = Join-Path $repo "assets"

Write-Host "=== [1/3] build release dos 2 binarios ===" -ForegroundColor Cyan
Set-Location $repo
cargo build --release -p lina-bootstrap --bin lina
Set-Location (Join-Path $repo "app\lina-gpui")
cargo build --release
Set-Location $repo

if (-not (Test-Path $gpui)) { throw "binario ausente: $gpui (build release do app falhou?)" }
if (-not (Test-Path $lina)) { throw "binario ausente: $lina" }
if (-not (Test-Path $assets)) { throw "assets\ ausente: $assets" }

Write-Host "=== [2/3] montando $dist ===" -ForegroundColor Cyan
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item $gpui (Join-Path $dist "lina-gpui.exe")
Copy-Item $lina (Join-Path $dist "lina.exe")
Copy-Item -Recurse $assets (Join-Path $dist "assets")
if (Test-Path (Join-Path $dist "assets\.git")) { Remove-Item -Recurse -Force (Join-Path $dist "assets\.git") }

# Se voce vendorizou o ConPTY (Fase 3), copie as DLLs p/ o lado do exe (descomente + ajuste o caminho):
# Copy-Item (Join-Path $repo "vendor\conpty\conpty.dll") $dist
# Copy-Item (Join-Path $repo "vendor\conpty\OpenConsole.exe") $dist

# Copia o guia do aluno junto
Copy-Item (Join-Path $repo "packaging\windows\INSTALAR-LINA-WINDOWS.md") (Join-Path $dist "LEIA-ME - Instalar.md") -ErrorAction SilentlyContinue

Write-Host "=== [3/3] zip distribuivel ===" -ForegroundColor Cyan
$zip = Join-Path $repo "dist\Lina-windows-x64-$ver.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$dist\*" -DestinationPath $zip
Write-Host ""
Write-Host "PRONTO:" -ForegroundColor Green
Write-Host "  pasta: $dist"
Write-Host "  zip:   $zip"
Write-Host "SMOKE (faca!): copie a pasta dist\Lina-win p/ C:\Temp\LinaSmoke e rode lina-gpui.exe de la"
Write-Host "  (sem o repo/cargo) - o amigo confirma que ABRE e RENDERIZA = auto-contido."
Write-Host "LEMBRE: app SEM assinatura - 1o open passa pelo SmartScreen ('Mais informacoes -> Executar assim mesmo')."
