# WINDOWS-RESULT — relatório do bring-up (preencha com EVIDÊNCIA, não vibe)

> Copie este arquivo para a RAIZ do repo como `WINDOWS-RESULT.md` e preencha conforme avança.
> Ele vira o corpo do PR. Onde diz "(amigo na tela)", é uma observação VISUAL humana — gpui não
> roda headless, então essas linhas só o amigo confirma olhando.

**Máquina:** Windows ___ (versão) · CPU ___ · GPU ___ · arquitetura ___ (precisa x86_64/AMD64)
**Data:** ____  ·  **Branch:** windows-bringup  ·  **Commit base:** ____

## Fase 0 — Ambiente
- Rust/cargo: ____  · MSVC: ____  · Windows SDK: ____  · target x86_64-pc-windows-msvc: (sim/não)
- `check-env.ps1` sem FALTA pendente? (sim/não) — se não, o quê faltou: ____

## Fase 1 — Build
- `cargo build` do app gpui: **BUILD_EXIT = ___**
- Feature set do `gpui_platform` que funcionou no Windows (o bloco cfg-not-macos final): ____
- Erros/ajustes que precisei fazer: ____

## Fase 2 — DE-RISK (o checkpoint) 🔑
- (amigo na tela) **Renderiza** terminais com texto/cores/cursor? ____
- (amigo na tela) **Acentos**: `´`+`a` = `á`? `ã ç ê õ`? ____
- (amigo na tela) **Ctrl+T** cria um novo agente? ____
- (amigo na tela, opcional) **Narrator** lê algo da janela (a11y)? ____
- **VEREDITO:** ☐ PASSA  ☐ FALHA  ☐ não-compilou(arch)
- Se FALHA: o que exatamente quebrou (com o que o amigo viu + trecho do log): ____
  → **Recomendação: pivotar para Slint** (motivo). NÃO tentei Slint nem WSL. **PAREI aqui.**

## Fase 3 — ConPTY (só se PASSOU)
- A2A A→B no Windows funciona sem vendoring (portable-pty 0.9 do crates.io)? (sim/não): ____
- Precisei vendorizar + patch 3 flags? (sim/não). Se sim, o que mudou: ____
- (amigo na tela) B reage ao A2A disparado? ____ · `cargo test -p lina-pty`: EXIT ___

## Fase 4 — Empacotamento (se passou)
- `make-win.ps1`: gerou `dist\Lina-win\` + `.zip`? (sim/não) · tamanho: ____
- DLLs ConPTY bundladas? (sim/não/n-a): ____
- (amigo na tela) **Smoke**: copiei p/ C:\Temp e o `lina-gpui.exe` ABRE e RENDERIZA fora do repo? ____
- FPS real medido (GPU real desta máquina), se observei: N=4 ___ · N=16 ___ · N=28 ___

## Pendências / decisões para o dono
- ____

## Resumo em 1 linha
- ____
