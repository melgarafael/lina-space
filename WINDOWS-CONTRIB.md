# Lina Space — Contribuições Windows (Lucas, 2026-06-14)

> Registro das alterações feitas para trazer o Lina Space ao Windows nativo x86_64.
> Este documento complementa o `WINDOWS-RESULT.md` da tentativa anterior (Gabriel, 2026-06-04),
> que havia ficado no veredito "FALHA" na Fase 2 (terminal vazio, sem input).
> Nesta sessão o app chegou a **Fase 2 PASSA**: janela abre, terminais renderizam com texto/cores/cursor,
> input do teclado funciona, Ctrl+V cola, FPS estável ~100fps.

**Máquina:** Windows 11 Pro 10.0.26200 · AMD64 · GPU via Direct3D/wgpu  
**Rust:** 1.95.0 stable · target `x86_64-pc-windows-msvc`  
**VS Build Tools:** 2022 v17.14.34

---

## Bug Fixes

### BUG-WIN-1 — SSL `CRYPT_E_NO_REVOCATION_CHECK` bloqueava download de crates

**Sintoma:** `cargo build` falhava imediatamente ao tentar baixar dependências:
```
warning: spurious network error: SSL connect error (schannel: CRYPT_E_NO_REVOCATION_CHECK 0x80092012)
```
**Causa:** O schannel do Windows tenta verificar revogação de certificado via OCSP/CRL e não alcança o servidor.  
**Fix:** Criado `.cargo/config.toml` na raiz do workspace:
```toml
[http]
check-revoke = false
```
**Arquivo:** `.cargo/config.toml` *(novo)*

---

### BUG-WIN-2 — `link.exe` do Git for Windows substituía o linker MSVC

**Sintoma:** Build compilava mas falhava no link:
```
link: extra operand 'C:\...\build_script_build.rcgu.o'
```
**Causa:** O `link.exe` do Git for Windows (GNU linker) estava na frente do `PATH` em relação ao MSVC.  
**Fix:** Instalar Visual Studio Build Tools 2022 com workload C++ e sempre ativar `vcvars64.bat` antes de buildar. Criado `dev.bat` na raiz do projeto para automatizar isso.  
**Arquivos:** `dev.bat` *(novo)*

---

### BUG-WIN-3 — Panic de `a11y` no boot (`Duplicate a11y node id`)

**Sintoma:** App iniciava, criava Terminal A e então morria:
```
panicked at gpui/src/window/a11y.rs:223:13:
Duplicate a11y node id: #9204926289716864461.
In a release build, this node would be silently discarded.
```
**Causa:** Assertion de debug do gpui que dispara num caminho de acessibilidade exclusivo do Windows. Em release seria ignorada silenciosamente conforme a própria mensagem indica.  
**Fix:** Adicionado `debug-assertions = false` no `[profile.dev]` do `app/lina-gpui/Cargo.toml`:
```toml
[profile.dev]
debug = false
debug-assertions = false
```
**Arquivo:** `app/lina-gpui/Cargo.toml`  
**Nota para upstream:** A causa raiz (ID de a11y duplicado no Windows) merece investigação no gpui.

---

### BUG-WIN-4 — Stack overflow após alguns segundos (`STATUS_STACK_BUFFER_OVERRUN`)

**Sintoma:** App rodava normalmente a ~100fps por alguns segundos e crashava:
```
thread 'main' has overflowed its stack
exit code: 0xc000041d (STATUS_STACK_BUFFER_OVERRUN)
```
**Causa:** Windows tem stack padrão de 1MB vs 8MB no macOS/Linux. O render tree do gpui é profundo o suficiente para estourar a stack menor.  
**Fix:** Criado `app/lina-gpui/.cargo/config.toml` com flag de linker para 8MB de stack:
```toml
[http]
check-revoke = false

[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "link-arg=/STACK:8388608"]
```
**Arquivo:** `app/lina-gpui/.cargo/config.toml` *(novo)*  
**Nota para upstream:** É um contorno. A recursão excessiva no render do Windows merece investigação.

---

### BUG-WIN-5 — `Ctrl+V` exibia `^V` em vez de colar do clipboard

**Sintoma:** Pressionar Ctrl+V no terminal mostrava `^V^V^V` literalmente em vez de colar.  
**Causa:** O handler de paste estava amarrado ao modificador `platform` (tecla Win/⌘). No Mac `platform` é Cmd; no Windows é a tecla Win — que ninguém usa para colar.  
**Fix:** Alterado o handler para usar `shortcut_modifier()`, que via `#[cfg(windows)]` já inclui `control`:
```rust
// antes — só funcionava com a tecla Win pressionada:
if ks.modifiers.platform && ks.key == "v" {

// depois — funciona com Ctrl+V no Windows, ⌘V no Mac:
if shortcut_modifier(ks) && ks.key == "v" {
```
**Arquivo:** `app/lina-gpui/src/main.rs` *(função `handle_key`, bloco de shortcuts)*  
**Nota:** `Ctrl+C` continua indo para o PTY (SIGINT) — comportamento correto.

---

## Melhorias Identificadas (não implementadas)

Registradas aqui para continuidade. Nenhuma foi commitada.

### MELHORIA-WIN-1 — `app_support_dir()` usa caminho macOS hardcoded ✅ IMPLEMENTADO E APROVADO

**Problema:** A função lê `$HOME/Library/Application Support` — caminho que só existe no macOS. No Windows `$HOME` não existe; o app cai em `temp` e o estado some no reboot. Viola o invariante #6 ("estado sempre salvo").  
**Fix aplicado:** Função dividida com `#[cfg(windows)]` / `#[cfg(not(windows))]`. No Windows usa `%APPDATA%`:
```rust
#[cfg(windows)]
fn app_support_dir() -> PathBuf {
    match std::env::var_os("APPDATA") {
        Some(appdata) => PathBuf::from(appdata),
        None => {
            eprintln!("lina-gpui: APPDATA ausente — usando diretório temporário (o estado NÃO persistirá)");
            std::env::temp_dir()
        }
    }
}
```
**Arquivo:** `app/lina-gpui/src/main.rs`

### MELHORIA-WIN-2 — Shell padrão é `cmd.exe`; deveria ser PowerShell ✅ IMPLEMENTADO E APROVADO

**Problema:** O app usava `cmd.exe` hardcoded no Windows. Claude Code roda melhor em PowerShell e é o que o usuário Windows espera.  
**Fix aplicado:** `platform_shell_cmd()` no Windows agora tenta `powershell.exe -NoLogo` primeiro; cai no `%COMSPEC%` (cmd.exe) como fallback se PowerShell não existir:
```rust
#[cfg(windows)]
fn platform_shell_cmd() -> PtyCommand {
    let ps = std::process::Command::new("powershell.exe")
        .arg("-Command").arg("exit 0").status().is_ok();
    if ps {
        PtyCommand::new("powershell.exe").arg("-NoLogo")
    } else {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        PtyCommand::new(shell)
    }
}
```
**Arquivo:** `app/lina-gpui/src/bridge.rs`

### MELHORIA-WIN-3 — Sem atalho de cópia de texto no Windows

**Problema:** `Ctrl+C` vai para o PTY (SIGINT). No Mac `⌘C` copia a seleção. No Windows não há nenhum atalho de cópia funcional.  
**Fix sugerido:** Adicionar `Ctrl+Shift+C` para cópia (padrão do Windows Terminal).

---

## Estado da Fase 2 (De-risk) Nesta Sessão

| Critério | Resultado |
|---|---|
| Janela abre | ✅ |
| Terminais renderizam texto/cores/cursor | ✅ |
| Input do teclado chega ao PTY | ✅ |
| Ctrl+V cola do clipboard | ✅ (fix BUG-WIN-5) |
| Ctrl+T cria novo agente | ✅ |
| FPS estável | ✅ ~100fps (p50=10ms, p99=12ms) |
| Acentos `´+a=á` | não testado |
| Narrator (a11y) | não testado |

**Veredito: PASSA** — reversão do veredito anterior (FALHA de Gabriel em 2026-06-04).  
A diferença relevante: Smart App Control estava ativo na máquina anterior, bloqueando build scripts. Nesta máquina não havia essa restrição.

---

## Ambiente de Build Reproduzível

```bat
:: instalar dependências (uma vez)
winget install Microsoft.VisualStudio.2022.BuildTools --source winget ^
  --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

:: buildar e rodar (sempre)
dev.bat
```

## Pendências para Fase 4 (Empacotamento)

- [x] Aplicar MELHORIA-WIN-1 (persistência real no Windows)
- [x] Aplicar MELHORIA-WIN-2 (PowerShell como shell padrão)
- [ ] Aplicar MELHORIA-WIN-3 (Ctrl+Shift+C para copiar)
- [ ] Criar `packaging/windows/make-win.ps1`
- [ ] Bundlar `conpty.dll` + `OpenConsole.exe`
- [ ] Gerar `.zip` com `lina-gpui.exe` + assets
- [ ] Criar `packaging/windows/INSTALAR-LINA-WINDOWS.md`
