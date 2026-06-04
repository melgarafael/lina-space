---
name: windows-bringup
description: Use quando estiver numa máquina WINDOWS para trazer o Lina Space à vida no Windows nativo (gpui + ConPTY) e produzir um .exe x86_64 distribuível. Orquestra setup → build → de-risk (validação na tela pelo humano) → ConPTY → empacotamento → PR. Invoque ao abrir este repo num PC Windows, ou quando o usuário disser "rodar o processo do Windows", "windows bring-up", "/windows-bringup".
---

# Windows Bring-up do Lina Space

Você está numa **máquina Windows**, numa sessão limpa, trazendo o Lina para Windows nativo.
**Leia `WINDOWS-BRINGUP.md` (raiz do repo) ANTES de tudo** — ele tem o contexto, as restrições
inegociáveis e a árvore de decisão. Esta skill é o passo-a-passo executável daquele plano.

## Regras que você NÃO pode quebrar (resumo — detalhe no WINDOWS-BRINGUP.md)

1. **NÃO re-arquitete os terminais.** ConPTY = LIGAR o backend Windows que o `portable-pty` já tem
   (vendor + patch 3 flags + bundle dll). A arquitetura fica idêntica.
2. **Fallback é SLINT, NUNCA WSL.** E você não implementa Slint sozinho — reporta a falha e PARA.
3. **Sem assinatura de código** (SmartScreen bypass documentado para o aluno).
4. **gpui NÃO roda headless → você NÃO vê a tela.** Toda afirmação visual (renderizou? acentos? cria
   terminal?) **TEM que ser confirmada pelo AMIGO olhando a tela.** Pergunte e registre a resposta
   dele. NUNCA declare "funciona" por build verde.
5. **Validar por evidência:** redirecione build/test p/ arquivo, leia, capture `$LASTEXITCODE`.
   Sem `unwrap()/expect()` em produção.
6. **Branch `windows-bringup` + commits pequenos + PR no fim.** Nunca force-push nem toque na `main`.

## Protocolo (use TodoWrite para rastrear as fases)

### Fase 0 — Setup e verificação de ambiente
1. `git checkout -b windows-bringup` (se já existir, use-o).
2. Rode `powershell -ExecutionPolicy Bypass -File packaging\windows\check-env.ps1` e LEIA a saída.
   Ele reporta: Rust/cargo, MSVC (cl.exe), Windows SDK, e **a arquitetura** (host + targets).
   - **Se faltar MSVC/SDK:** peça ao amigo instalar "Build Tools for Visual Studio 2022" com o workload
     **"Desktop development with C++"** (inclui MSVC + Windows 11 SDK). gpui NÃO suporta MinGW/MSYS2.
   - **Se faltar Rust:** instale via `rustup` (https://rustup.rs), toolchain `stable-msvc`.
   - **Se a arquitetura NÃO for x86_64** (ex.: máquina ARM): registre no relatório e PARE — o binário
     dos alunos precisa ser x86_64; uma máquina x86_64 é necessária.
3. Crie `WINDOWS-RESULT.md` a partir de `packaging\windows\WINDOWS-RESULT.template.md` e vá preenchendo.

### Fase 1 — Build do app gpui no Windows
1. Rode `powershell -ExecutionPolicy Bypass -File packaging\windows\build.ps1` e LEIA o log
   (`build-win.log`). Ele aplica/checa o cfg do Cargo (`runtime_shaders` = **macOS-only**) e roda
   `cargo build` em `app\lina-gpui`.
2. **Itere pelo compilador** se faltar feature do gpui-Windows: o `runtime_shaders` é Metal (macOS); no
   Windows o `gpui_platform` usa o backend Direct3D. Se o build reclamar de backend de render ausente,
   ajuste as features no bloco `[target.'cfg(not(target_os = "macos"))'.dependencies]` do
   `app/lina-gpui/Cargo.toml` (tente `default-features = true`, ou a feature que o erro indicar).
   **Anote no `WINDOWS-RESULT.md` o feature set que funcionar** — vira a config oficial do Windows.
3. Gate: `cargo build` termina com `$LASTEXITCODE == 0`. Registre.

### Fase 2 — DE-RISK (o checkpoint que decide tudo) 🔑
1. Rode o app: dentro de `app\lina-gpui`, `cargo run` (boot de produção → onboarding). Para ir direto a
   2 terminais de teste: `$env:LINA_DEMO=1; cargo run`.
2. **PEÇA AO AMIGO PARA OLHAR A TELA** e responder (registre cada resposta no `WINDOWS-RESULT.md`):
   - (a) **Renderiza?** Os terminais aparecem com texto/cores/cursor? (Tela preta/quebrada = backend
     DirectX do gpui não subiu = achado crítico.)
   - (b) **Acentos (IME)?** Num terminal, digitar `´` depois `a` sai **á**? Teste `ã ç ê õ`.
   - (c) **Cria terminal?** `Ctrl+T` cria um novo agente?
   - (d) **a11y (opcional):** abrir o **Narrator** (Win+Ctrl+Enter) — ele lê algo da janela?
3. **DECISÃO:**
   - ✅ **PASSA** (a+b+c OK): registre PASS e **siga para a Fase 3** (autorizado ir até o `.exe`).
   - ❌ **FALHA**: registre exatamente o que o amigo viu + os logs, escreva no `WINDOWS-RESULT.md` a
     recomendação **"pivotar para Slint"** (com o motivo), faça commit + push + `gh pr create`, e
     **PARE**. NÃO tente Slint nem WSL.

### Fase 3 — ConPTY (só se a Fase 2 PASSOU)
> Objetivo: A2A faseada (injeção no PTY vivo) funciona no Windows. ConPTY básico já vem no
> `portable-pty` 0.9 do crates.io — talvez o A2A já funcione no de-risk (`LINA_DEMO=1`, botão ⚡ A→B).
> Se já funciona, registre e pode pular o vendoring (ou fazê-lo só se houver bug de injeção).
1. Teste A2A primeiro (LINA_DEMO=1, dispare A→B, o amigo confirma que B reage). Se OK → registre e vá p/ Fase 4.
2. **Se houver bug de injeção/EOF** (ex.: `try_clone_reader` não fecha, input incorreto): vendorize o
   `portable-pty` read-only no repo e aplique o patch dos **3 flags ConPTY** (`PSEUDOCONSOLE_RESIZE_QUIRK`,
   `PSEUDOCONSOLE_WIN32_INPUT_MODE`, `PSEUDOCONSOLE_PASSTHROUGH_MODE` [só Win11 22H2+]). Referência:
   `wezterm/pty/src/win/conpty.rs`. Aponte `lina-pty` para o vendor. Bundle `conpty.dll`+`OpenConsole.exe`
   (NuGet `Microsoft.Windows.Console.ConPTY`, ou os do `wezterm`/`alacritty`).
3. Gate: A2A A→B entrega no Windows (amigo confirma na tela + log). `cargo test -p lina-pty` verde.

### Fase 4 — Empacotar o `.exe` distribuível (se passou)
1. `powershell -ExecutionPolicy Bypass -File packaging\windows\make-win.ps1` → monta
   `dist\Lina-win\` (com `lina-gpui.exe` + `lina.exe` + `assets\` + DLLs do ConPTY se bundladas) e um
   `Lina-windows-x64.zip`.
2. **Smoke do app empacotado:** copie o `dist\Lina-win\` para outra pasta (ex.: `C:\Temp\LinaSmoke`),
   rode o `lina-gpui.exe` de lá (sem o repo/cargo), e o amigo confirma que **abre e renderiza** (prova
   que é auto-contido: acha `lina.exe` ao lado + `assets\` por ancestral, igual no Mac).
3. Confirme o guia do aluno: `packaging\windows\INSTALAR-LINA-WINDOWS.md` (SmartScreen + pré-req claude).

### Fase 5 — Reportar e PR
1. Finalize o `WINDOWS-RESULT.md` com **evidência** (logs, exit codes, o que o amigo viu, FPS real se
   mediu, o feature set do Cargo que funcionou, o que foi bundlado).
2. `git add` (só os arquivos do bring-up), commits semânticos, `git push -u origin windows-bringup`.
3. `gh pr create --title "Windows bring-up" --body-file WINDOWS-RESULT.md` (base `main`).
4. Avise o amigo que terminou e que o dono vai revisar o PR.

## Se travar
- Build de gpui é grande (~10-20 min na 1ª vez, ~1.3 GB de target). Disco/tempo são normais.
- Erro de feature do gpui → itere pelo compilador (Fase 1, item 2).
- Qualquer dúvida que precise de decisão do dono (não coberta aqui) → registre no `WINDOWS-RESULT.md`,
  pare, e deixe o PR/relatório explicar. Não improvise fora do plano.
