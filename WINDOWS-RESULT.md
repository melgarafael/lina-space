# WINDOWS-RESULT - relatorio do bring-up Windows

Relatorio preenchido com evidencia. Confirmacoes visuais do gpui devem vir do Gabriel olhando a tela, porque o app nao roda headless.

**Maquina:** Windows (versao/CPU/GPU nao consultados: CIM bloqueado por permissao) - arquitetura AMD64
**Data:** 2026-06-04 - **Branch:** windows-bringup - **Commit base:** e203c9476726a80eafbc99184df040b16b9ee5f8

## Fase 0 - Ambiente

- Rust/cargo: OK (`rustc 1.96.0`, host `x86_64-pc-windows-msvc`; `cargo 1.96.0`)
- MSVC: OK (`C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`)
- Windows SDK: OK (`10.0.26100.0`)
- target x86_64-pc-windows-msvc: sim
- `check-env.ps1` sem FALTA pendente? sim
- Instalado nesta sessao: Rustup via `winget`; Visual Studio Build Tools 2022 via `winget` com workload `Microsoft.VisualStudio.Workload.VCTools`, `--includeRecommended` e SDK `Microsoft.VisualStudio.Component.Windows11SDK.26100`.
- Outros achados: `PROCESSOR_ARCHITECTURE=AMD64`; `git` OK; `gh` ausente/opcional.
- Evidencia inicial: `packaging/windows/check-env.log`; `LASTEXITCODE=0` em `packaging/windows/check-env.exit`.
- Evidencia apos instalacao: `packaging/windows/check-env.after-install.log`; `LASTEXITCODE=0` em `packaging/windows/check-env.after-install.exit`.

## Fase 1 - Build

- `cargo build` do app gpui: BUILD_EXIT = 0
- Feature set do `gpui_platform` que funcionou no Windows: bloco `cfg(not(target_os = "macos"))` atual, com `features = ["font-kit"]` e default-features implicitas.
- Erros/ajustes que precisei fazer: antes do build passar, o Cargo baixou dependencias e iniciou a compilacao, mas o Windows bloqueou a execucao dos build scripts Rust gerados localmente. Erro: `Uma politica de Controle de Aplicativo bloqueou este arquivo. (os error 4551)`. Ocorreu tanto no target padrao dentro do repo/OneDrive quanto com `CARGO_TARGET_DIR=C:\Temp\lina-target`.
- Diagnostico do bloqueio inicial: Smart App Control / Code Integrity em enforcement. `reg query HKLM\SYSTEM\CurrentControlSet\Control\CI\Policy` retornou `VerifiedAndReputablePolicyState=0x1` e `SAC_EnforcementReason=0x1`. Tentativa de mudar para Evaluation via `reg add` falhou com `Acesso negado`; `citool.exe -lp` falhou com `0x80070005`.
- Gabriel desativou o Smart App Control visualmente. Nova evidencia: `VerifiedAndReputablePolicyState=0x0`, `SAC_PreviousState=0x1`.
- Build bem-sucedido apos isso com `CARGO_TARGET_DIR=C:\Temp\lina-target`. Evidencia: `build-cargo-after-sac.log`; `LASTEXITCODE=0` em `packaging/windows/build-cargo-after-sac.exit`; binario `C:\Temp\lina-target\debug\lina-gpui.exe` gerado com 35.590.144 bytes.

## Fase 2 - DE-RISK

- (Gabriel na tela) Renderiza terminais com texto/cores/cursor? FALHA. A janela renderiza os cards "Terminal A" e "Terminal B" com bordas/topbar, mas a area do terminal fica vazia: sem prompt, sem texto e sem cursor visivel.
- (Gabriel na tela) Acentos: `'` + `a` = `a` acentuado? Teste `a c e o` com acentos: FALHA. Gabriel nao conseguiu digitar no terminal; nada aparece ao teclar.
- (Gabriel na tela) Ctrl+T cria um novo agente? PASSA apos ajuste Windows: `Ctrl+T` criou novo terminal/agente.
- (Gabriel na tela, opcional) Narrator le algo da janela? pendente
- VEREDITO: FALHA
- O que exatamente quebrou: gpui Windows abriu a janela e desenhou os cards, mas o conteudo do terminal nao apareceu e input humano nao chegou de forma observavel ao terminal. Logs confirmam PTYs vivos/desenhados (`panels_live=2`, `panels_drawn=2`) e processos `cmd.exe` filhos rodando, mas a tela validada por Gabriel mostrou terminal vazio e sem digitação. Evidencia visual: screenshot enviada pelo Gabriel em 2026-06-04.
- Ajustes triviais tentados antes do veredito: (1) `shell_cmd` no Windows deixou de usar `sh` inexistente e passou por `powershell.exe`; depois foi simplificado para `cmd.exe /K`; (2) `Ctrl+T` passou a aceitar `control` no Windows; (3) fallback Windows para `key_char` imprimivel foi adicionado caso o IME nao chame `replace_text_in_range`. Mesmo assim, texto/prompt/input nao apareceram.
- Recomendacao: pivotar para Slint, preservando o core e a trait `UiHost`. Nao tentei Slint nem WSL.

## Fase 3 - ConPTY

- A2A A->B no Windows funciona sem vendoring? nao executado, porque a Fase 2 falhou.
- Precisei vendorizar + patch 3 flags? nao executado.
- (Gabriel na tela) B reage ao A2A disparado? nao executado.
- `cargo test -p lina-pty`: EXIT nao executado.

## Fase 4 - Empacotamento

- `make-win.ps1`: gerou `dist\Lina-win\` + `.zip`? nao executado, porque a Fase 2 falhou.
- DLLs ConPTY bundladas? nao executado.
- (Gabriel na tela) Smoke fora do repo abre e renderiza? nao executado.
- FPS real medido: durante Fase 2, logs variaram em torno de 100-120 FPS com 2 paineis quando ativo; houve janelas de 26-27 FPS em alguns periodos. Como o de-risk visual falhou, estes numeros nao aprovam o bring-up.

## Pendencias / decisoes para o dono

- Decisao recomendada: pivotar a UI Windows para Slint usando a fronteira `UiHost`, mantendo core/PTY/event log. O gpui Windows nao passou no checkpoint minimo: terminal sem conteudo e sem input observavel.
- Smart App Control foi desativado por Gabriel para permitir build Rust local. Antes disso, bloqueava build scripts do Cargo com `os error 4551`.
- O instalador do Build Tools informou "Reiniciar seu PC para terminar a instalacao", mas o `check-env.ps1` detectou MSVC e Windows SDK com sucesso nesta sessao.

## Resumo em 1 linha

- Windows build compila, gpui abre a janela e desenha cards, mas o de-risk falhou: terminal sem prompt/texto/cursor e sem input; recomendacao e pivotar para Slint.
