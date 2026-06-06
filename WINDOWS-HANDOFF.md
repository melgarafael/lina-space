# WINDOWS-HANDOFF - continuidade do bring-up Windows

Data: 2026-06-04
Branch: `windows-bringup`
Repo: `C:\Users\gabri\OneDrive\Desktop\lina\lina-space`
PR parcial ja aberto: https://github.com/melgarafael/lina-space/pull/1

Este documento existe para uma sessao zerada continuar exatamente de onde paramos. Leia tambem `CLAUDE.md`, `WINDOWS-BRINGUP.md` e `.claude/skills/windows-bringup/SKILL.md` antes de mexer.

## Regra operacional

- Nao declarar que gpui funciona por build verde. gpui nao roda headless; Gabriel precisa olhar a tela.
- Nao usar WSL.
- Nao re-arquitetar terminal. ConPTY e para ligar/ajustar o backend Windows do `portable-pty`.
- Se for continuar nativo/gpui, precisa passar visualmente: render de texto/cursor, input/acento, Ctrl+T, depois A2A.
- O PR #1 foi aberto cedo por engano/protocolo anterior. Trate-o como PR parcial; continue no mesmo branch e atualize o PR quando o resultado final estiver correto.

## Ambiente instalado

Foram instalados via `winget`:

- Rustup/Rust: `rustc 1.96.0`, host `x86_64-pc-windows-msvc`
- Visual Studio Build Tools 2022 com workload C++ (`Microsoft.VisualStudio.Workload.VCTools`)
- Windows SDK `10.0.26100.0`
- GitHub CLI `gh`

`packaging/windows/check-env.ps1` passou depois da instalacao:

- Evidencia: `packaging/windows/check-env.after-install.log`
- Exit: `packaging/windows/check-env.after-install.exit`

Smart App Control bloqueou build scripts Rust inicialmente:

- Erro: `os error 4551`
- Registro antes: `VerifiedAndReputablePolicyState=0x1`
- Gabriel desativou visualmente o Smart App Control.
- Registro depois: `VerifiedAndReputablePolicyState=0x0`

Build usa frequentemente:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_NET_OFFLINE="false"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
```

`CARGO_NET_OFFLINE` estava `true` no ambiente da sessao em alguns momentos; sempre sobrescrever para `false` quando precisar baixar deps.

## O que ja foi commitado/pushado

Commit local e remoto ja feito:

- `f647bb5 Report Windows gpui de-risk failure`

Esse commit inclui:

- `WINDOWS-RESULT.md`
- ajuste em `app/lina-gpui/src/bridge.rs`: shell Windows deixou de usar `sh`; foi tentado `cmd.exe /K`
- ajuste em `app/lina-gpui/src/main.rs`: `Ctrl+T` passou a aceitar `control` no Windows; fallback de `key_char` em Windows

Push ja feito:

```powershell
git push -u origin windows-bringup
```

PR aberto:

```text
https://github.com/melgarafael/lina-space/pull/1
```

## Estado atual do working tree

Ha mudancas NAO commitadas depois do PR parcial. Elas sao importantes e devem ser revisadas, nao descartadas:

- `Cargo.lock` modificado
- `crates/lina-pty/Cargo.toml` modificado para apontar `portable-pty` para `../../vendor/portable-pty`
- `crates/lina-pty/tests/pty.rs` modificado para testes Windows
- `vendor/portable-pty/` adicionado por copia do crate `portable-pty 0.9.0`
- `vendor/conpty/` adicionado com pacote NuGet extraido
- `conpty.dll` e `OpenConsole.exe` copiados para a raiz do repo para teste de sideload
- muitos logs `.log` e `.exit` untracked
- `.codex/` apareceu untracked; nao mexi/conheco

Antes de continuar, rode:

```powershell
git status --short --branch
```

Nao limpe logs destrutivamente sem decidir o que vai para evidencia. Nao commitar todos os logs cegamente.

## Resultados confirmados

### Fase 0 - ambiente

PASSOU.

### Fase 1 - build gpui

PASSOU.

Comando que passou:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
cd app\lina-gpui
cargo build
```

Evidencia:

- `build-cargo-after-sac.log`
- `packaging/windows/build-cargo-after-sac.exit` com `LASTEXITCODE=0`

### Fase 2 - de-risk visual gpui

FALHOU ate o momento, mas ainda estavamos investigando, nao finalizando.

O que Gabriel viu:

1. Primeira tentativa com `sh`: app abriu, mas os PTYs A/B falharam porque `sh` nao existe no Windows.
2. Depois de trocar para `powershell.exe`/`cmd.exe`: janela abriu e mostrou os cards `Terminal A` e `Terminal B`, mas sem texto/prompt/cursor dentro da area do terminal.
3. Gabriel tentou digitar: nada apareceu.
4. `Ctrl+T` passou depois do ajuste: criou novo terminal/agente.

Importante: logs do app mostravam `panels_live=2` e `panels_drawn=2`, mas a tela estava vazia. Isso nao e suficiente para aprovar.

### Testes core/VT/PTT

`lina-vt` passou:

```powershell
cargo test -p lina-vt
```

Evidencia:

- `test-lina-vt-online.log`
- `packaging/windows/test-lina-vt-online.exit` com `LASTEXITCODE=0`

`lina-pty` inicialmente falhou porque os testes assumiam `sh`. Foram feitas mudancas nao commitadas em `crates/lina-pty/tests/pty.rs` para Windows.

## Investigacao ConPTY atual

Hipotese atual: o problema principal esta no ConPTY/portable-pty, nao no render gpui.

Sinais:

- `lina-vt` parseia/renderiza bytes corretamente em teste.
- gpui desenha cards/topbar corretamente.
- `lina-pty` com `sh` falhava no Windows por `sh` inexistente.
- Depois de adaptar testes para Windows, `resize` e `writer_is_single_owner` passam.
- Output nao interativo so passou quando removemos `PSUEDOCONSOLE_INHERIT_CURSOR` do vendored `portable-pty`.

Mudanca atual no vendor:

Arquivo:

```text
vendor/portable-pty/src/win/psuedocon.rs
```

Flags testadas:

1. Original crates.io: `INHERIT_CURSOR | RESIZE_QUIRK | WIN32_INPUT_MODE`
   - Resultado: app sem prompt/input; testes travam/falham.
2. `RESIZE_QUIRK` apenas
   - Resultado: teste de output nao interativo passou.
   - Comando que passou:
     ```powershell
     cargo test -p lina-pty spawn_runs_command_and_streams_output -- --nocapture
     ```
   - Evidencia: `test-lina-pty-no-inherit-output.log`; exit `packaging/windows/test-lina-pty-no-inherit-output.exit` com `LASTEXITCODE=0`
3. `RESIZE_QUIRK | PASSTHROUGH_MODE`
   - Resultado: input interativo ainda falhou.
4. `RESIZE_QUIRK | WIN32_INPUT_MODE` sem `INHERIT_CURSOR`
   - Estava sendo testado quando a sessao foi interrompida. Nao considerar conclusivo; pode ter ficado log parcial.

Sideload ConPTY:

- Baixado NuGet `CI.Microsoft.Windows.Console.ConPTY 1.22.250314001`
- Fonte: NuGet package oficial encontrado como `CI.Microsoft.Windows.Console.ConPTY`
- Extraido em `vendor/conpty/pkg`
- Copiado x64 para:
  - `vendor/conpty/win10-x64/conpty.dll`
  - `vendor/conpty/win10-x64/OpenConsole.exe`
- Tambem copiado para raiz:
  - `conpty.dll`
  - `OpenConsole.exe`

Quando `conpty.dll`/`OpenConsole.exe` estavam na raiz, processo local `OpenConsole.exe` apareceu, entao o sideload funcionou pelo menos em parte. Mesmo assim, com flags ainda ruins, teste travou.

## Arquivos principais modificados nao commitados

### `crates/lina-pty/Cargo.toml`

Mudou de:

```toml
portable-pty = "0.9.0"
```

para:

```toml
portable-pty = { path = "../../vendor/portable-pty" }
```

### `vendor/portable-pty/src/win/psuedocon.rs`

Esta e a area critica:

```rust
(CONPTY.CreatePseudoConsole)(
    size,
    input.as_raw_handle() as _,
    output.as_raw_handle() as _,
    FLAGS_AQUI,
    &mut con,
)
```

No momento da interrupcao, o ultimo patch estava testando:

```rust
PSEUDOCONSOLE_RESIZE_QUIRK | PSEUDOCONSOLE_WIN32_INPUT_MODE
```

Mas esse teste nao terminou. Volte a um estado conhecido se necessario:

```rust
PSEUDOCONSOLE_RESIZE_QUIRK
```

Esse estado pelo menos fez o teste de output nao interativo passar.

### `crates/lina-pty/tests/pty.rs`

Mudancas em andamento:

- helper `test_shell()` com Windows usando PowerShell
- helper `line()` com CR
- helper `read_until()`
- teste `spawn_runs_command_and_streams_output` no Windows usa PowerShell nao interativo `Write-Output 42` e `read_until(reader, "42")`
- teste concorrente ainda falha por timeout no input interativo

Ha uma possivel linha `manager.kill("calc")` ja corrigida para ficar no teste de output; verifique antes de continuar.

## Proximos passos recomendados

1. Reabrir a sessao e ler:
   - `CLAUDE.md`
   - `WINDOWS-BRINGUP.md`
   - `.claude/skills/windows-bringup/SKILL.md`
   - este `WINDOWS-HANDOFF.md`

2. Checar processos vivos:

```powershell
Get-Process | Where-Object { $_.ProcessName -match '^cargo$|^rustc$|^pty-|^lina-gpui$|^cmd$|^OpenConsole$' } | Select-Object Id,ProcessName,StartTime
```

Se houver processo de teste claramente travado, parar por PID especifico, nao por nome amplo:

```powershell
Stop-Process -Id <PID> -Force
```

3. Checar git:

```powershell
git status --short --branch
git diff -- crates/lina-pty/Cargo.toml crates/lina-pty/tests/pty.rs vendor/portable-pty/src/win/psuedocon.rs app/lina-gpui/src/bridge.rs app/lina-gpui/src/main.rs
```

4. Voltar o flag ConPTY para o estado que comprovou output:

```rust
PSEUDOCONSOLE_RESIZE_QUIRK
```

5. Rodar:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_NET_OFFLINE="false"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
cargo test -p lina-pty spawn_runs_command_and_streams_output -- --nocapture *> test-lina-pty-output-resume.log
$code=$LASTEXITCODE
"LASTEXITCODE=$code" | Tee-Object -FilePath packaging\windows\test-lina-pty-output-resume.exit
```

Esperado no estado conhecido: `LASTEXITCODE=0`.

6. Resolver input interativo no `lina-pty` antes de voltar ao gpui.

Teste alvo:

```powershell
cargo test -p lina-pty eight_concurrent_ptys_report_distinct_pids -- --nocapture
```

Hoje falha por timeout: input escrito no writer nao chega/nao produz marker.

Pistas tecnicas:

- `WIN32_INPUT_MODE` pode exigir eventos Win32, nao bytes VT. O Lina escreve bytes (`HumanKeys`, A2A faseado), entao esse modo talvez seja inadequado.
- Sem `WIN32_INPUT_MODE`, output passou, mas input ainda nao chegou nos testes interativos.
- Pode ser preciso estudar como `portable-pty`/WezTerm escreve input no ConPTY quando nao usa Win32 mode. Nao trocar arquitetura; corrigir o backend vendorizado.
- Evitar `INHERIT_CURSOR`: causou travamento/output ausente em sonda.
- Sideload com `conpty.dll`/`OpenConsole.exe` local funcionou parcialmente; `OpenConsole.exe` local apareceu no processo, mas ainda depende dos flags corretos.

7. Quando `cargo test -p lina-pty` passar no Windows, rebuildar app:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_NET_OFFLINE="false"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
cd app\lina-gpui
cargo build *> ..\..\build-after-conpty.log
$code=$LASTEXITCODE
cd ..\..
"LASTEXITCODE=$code" | Tee-Object -FilePath packaging\windows\build-after-conpty.exit
```

8. Rodar app demo e pedir Gabriel validar visualmente:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
$env:LINA_DEMO="1"
cd app\lina-gpui
cargo run
```

Checks visuais obrigatorios:

- aparecem Terminal A/B com prompt/texto/cursor?
- digitar texto aparece?
- `´` + `a` vira `á`? testar `ã ç ê õ`
- `Ctrl+T` cria terminal?
- clicar `Enviar A2A (A->B)` faz B reagir?

9. So se Fase 2 passar:

- seguir Fase 3 ConPTY/A2A
- Fase 4 empacotar `.exe`
- smoke fora do repo
- atualizar `WINDOWS-RESULT.md`
- commit/push para o PR #1

## Comandos uteis

Build app:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_NET_OFFLINE="false"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
cd app\lina-gpui
cargo build
```

Rodar demo:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
$env:LINA_DEMO="1"
cd app\lina-gpui
cargo run
```

Testes:

```powershell
$env:Path="$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_NET_OFFLINE="false"
$env:CARGO_TARGET_DIR="C:\Temp\lina-target"
cargo test -p lina-vt
cargo test -p lina-pty
```

PR atual:

```powershell
& "C:\Program Files\GitHub CLI\gh.exe" pr view 1
```

## Prompt para nova sessao

Copie e cole este prompt na proxima sessao:

```text
Voce esta no projeto Lina Space em Windows, branch windows-bringup. Responda em pt-br.

Antes de qualquer coisa, leia CLAUDE.md, WINDOWS-BRINGUP.md, .claude/skills/windows-bringup/SKILL.md e WINDOWS-HANDOFF.md. O handoff e o estado mais atualizado.

Contexto essencial: o ambiente Windows ja foi instalado e o build gpui compila. O PR #1 existe mas e parcial, nao final. O de-risk visual ainda NAO passou: gpui desenha cards, Ctrl+T passou, mas terminal ficou sem prompt/texto/cursor e sem input. Estamos investigando ConPTY/portable-pty, nao empacotando ainda.

Continue do ponto atual:
- cheque git status e processos vivos;
- nao descarte mudancas nao commitadas;
- foque primeiro em fazer cargo test -p lina-pty passar no Windows;
- ha vendor/portable-pty e vendor/conpty em andamento;
- o estado de ConPTY conhecido que fez output nao interativo passar foi usar apenas PSEUDOCONSOLE_RESIZE_QUIRK em vendor/portable-pty/src/win/psuedocon.rs;
- input interativo ainda falha por timeout no teste eight_concurrent_ptys_report_distinct_pids;
- nao use WSL, nao implemente Slint sozinho, nao re-arquitete terminais;
- quando lina-pty passar, rode o app demo e peca Gabriel validar visualmente render/input/acento/Ctrl+T/A2A;
- so depois de passar no de-risk siga para empacotar .exe e atualizar o PR #1.

Use logs e LASTEXITCODE direto. Nao declare funcionamento por build verde.
```
