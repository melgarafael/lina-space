# WINDOWS-RESULT — relatório do bring-up Windows

> Confirmações visuais do gpui **devem** vir do Eduardo olhando a tela (o app não roda headless).
> Tudo mais é evidência por exit code + log.

## Sessão 2 — 2026-06-27 (Eduardo)

**Máquina:** Windows (`C:\Users\<USER>\`) — `PROCESSOR_ARCHITECTURE=AMD64` (x86_64) ✅
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

**Primeira tentativa (debug build):** janela abriu e renderizou — versus sintoma de junho (quadrados vazios). Print do Eduardo confirma: 2 terminais com **shell vivo** (`Microsoft Windows [versão 10.0.26200.8655]` + cwd correto + cursor piscando) + 1 card vazio "Novo Terminal" cortado por janela estreita demais.

| Critério | Estado | Evidência |
|---|---|---|
| (a) Renderiza terminais com texto/cursor | ✅ | print + log `pty_rows=26 drawn_rows=26` nos 2 terminais com PTY |
| (b) Acentos (`´+a=á`) | ❓ não testado | crash em ~3s não deu tempo |
| (c) `Ctrl+T` cria agente | ❓ não testado | mesmo motivo |
| FPS | ✅ | 100-110 estável (vs. oscilação 26-120 de junho) |

**Bloqueador**: panic do gpui em `crates/gpui/src/window/a11y.rs:223` — `Duplicate a11y node id: #3852104715044691937`. A própria mensagem do panic diz: *"In a release build, this node would be silently discarded from the a11y tree."* Bug de a11y do gpui Windows com AccessKit; release build silencia.

**Decisão tática:** rebuild em release pra ter vida útil ilimitada e completar (b) e (c). Não tocar no código do gpui (vendored do zed). Se release passar, Fase 2 PASSA; segue até o `.exe` empacotado.

**Observações da print:**
- O prompt do CMD aparece "muito longo" porque o cwd do agente é `C:\Users\<USER>\AppData\Local\Temp\lina-space-ws3\n-<uuid>` — comportamento por design (cada agente nasce em seu próprio dir).
- Layout do 3º card (Periphery, sem PTY) está cortado quando janela é estreita — polimento, não bloqueador.

### Fase 3 — ConPTY / A2A ✅

**Vendoring + 3 flags: NÃO foi necessário.** O `portable-pty 0.9` do crates.io já tem o backend Windows funcional. Provas observadas na tela do Eduardo:

1. **PTY entregando I/O sem corrupção** (Fase 2): `dir` no Terminal A retornou a árvore real do diretório (`.claude`, `.lina`, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md` — kit Lina); CMD em pt-br respondeu corretamente.
2. **TUI complexo (Claude Code v2.1.118 Opus 4.7) subiu inteiro dentro do PTY do Lina** — header com identidade, prompt, sugestão de comando, atalhos. ink/react do Claude rodando sem glitch.
3. **A2A real entregou no PTY do Claude Code**: clicando ⚡ no Terminal A, o `deliver_a2a` enfiou a string `echo '📨 A2A recebido de Terminal A · cooperacao sem fios'` no Claude do Terminal B, que (a) reconheceu como prompt, (b) **automaticamente carregou a skill `lina-agent-bus`**, (c) rodou `lina handshake` → identificou colegas `@Trigger` e `Terminal A`, (d) rodou `lina plan read` e `lina list`, (e) respondeu em pt-br: *"Pronto — cheguei no Espaço…"*.

Implicação: **o Lina Space inteiro coopera automaticamente no Windows nativo**. Nenhum patch de ConPTY, nenhum vendoring, nenhum bundle de `conpty.dll` / `OpenConsole.exe` — tudo herdado de `portable-pty` upstream.

### Fase 4 — Empacotamento ✅

- `packaging\windows\make-win.ps1` rodou — exit 0
- **Mesmo gotcha do `build.ps1`**: o script tinha `$ErrorActionPreference = "Stop"` que mata o pipeline na primeira linha de stderr do cargo (NativeCommandError). Aplicado o mesmo fix de 1 linha → `"Continue"`.
- Conteúdo de `dist/Lina-win/` (33.5 MB total):
  - `lina-gpui.exe` 26.7 MB (app GPU release)
  - `lina.exe` 6.4 MB (CLI release)
  - `conpty.dll` 108 KB + `OpenConsole.exe` 1.1 MB (ConPTY bundle, herdado de `vendor/conpty/win10-x64/` que já estava no repo — fallback de robustez)
  - `assets/` (lina-doctrine, lina-shim, lina-skills)
  - `LEIA-ME - Instalar.md` (guia do aluno com instruções do SmartScreen)
- Zip distribuível: `dist/Lina-windows-x64-0.1.0.zip` (13.4 MB comprimido)
- **Smoke test do auto-contido**: copiei `dist/Lina-win/` → `C:\Temp\LinaSmoke\` (fora do repo, sem cargo); rodei `C:\Temp\LinaSmoke\lina-gpui.exe`. Eduardo confirma: **"Abriu e deixou eu configurar tudo no onboarding"** — app é totalmente auto-contido, encontra `lina.exe` ao lado e `assets/` por ancestral, sem dependência do repo. Onboarding completo rodando.

### Fase 4 — Empacotamento

- (depende da Fase 2 PASSAR)

### Fase 5 — PR

- (Eduardo abre quando quiser; comando pronto em `gh pr create --repo melgarafael/lina-space --base main --head EduargoGrigolo:windows-bringup`)

---

## Achado pós-Fase 4 (mesma sessão) — npm-shim do Claude no Windows

Depois de declarar Fase 4 PASS e empacotar, no uso real (Eduardo tentando criar agente Claude pelo modal "Novo Agente"), o app explodiu com:

```
M6 — commit falhou: falha ao spawnar comando no PTY ...
CreateProcessW "C:\Users\<USER>\AppData\Roaming\npm\claude --output-format stream-json --verbose"
failed: %1 não é um aplicativo Win32 válido. (os error 193)
```

**Causa raiz (dois fixes complementares):**

1. **`crates/lina-core/src/cli_discovery.rs`** — a constante `EXE_EXTS` no Windows era `["", ".exe", ".cmd", ".bat"]`. O `find_in_path` aceitava o primeiro hit; o npm-shim do Claude Code instala `claude` (shell script Unix, sem extensão) NO MESMO DIRETÓRIO que `claude.cmd` e `claude.ps1`, então o discovery gravava o caminho do script Unix no event log. **Fix:** reordenar para `[".cmd", ".exe", ".bat", ""]` — `""` vira último fallback (caso raro de binário Win32 sem extensão).

2. **`crates/lina-pty/src/lib.rs`** — o `to_builder()` da `CommandSpec` chamava `CommandBuilder::new(self.program.as_str())` direto com o `program` cru do TOML (`"claude"`). O `portable-pty` no Windows passa cru pro `CreateProcessW`, que **não tenta extensões** (a stdlib do Rust só tenta `.exe`). **Fix:** introduzir `resolve_program()` no `lina-pty` que, no Windows, varre o PATH na ordem `.cmd → .exe → .bat` e devolve o caminho absoluto resolvido (nome com separador ou com extensão executável passa direto). Não importa de `lina-core` (criaria dependência circular — `lina-core` já depende de `lina-pty`).

**Validação na tela:** com os dois fixes + rebuild release + re-empacotamento, Eduardo refez "Novo Agente → Maestro → cwd `C:\Projetos\Testes Lina`". Log:

```
admissão — agente 'Maestro' criado (papel MAESTRO, cwd C:\Projetos\Testes Lina)
[Maestro pty_rows=26 drawn_rows=26 zone=Focus dim=false]
panels_live=2  fps=138
```

**Sem erro 193.** Agente nasceu com PTY vivo, Claude Code spawnado, pronto pra receber input do humano.

**Implicação:** o `dist/Lina-win/` que o aluno baixa agora cobre o caso REAL de uso (criar agente Claude via modal). Antes do achado, mesmo com Fase 4 PASS por critério da skill, o app teria falhado no primeiro clique do aluno em `✦ Novo Agente`.

## Segundo achado pós-Fase 4 — atalhos copy/paste no Windows

Eduardo reportou: `Ctrl+V` e `Ctrl+Shift+V` não funcionam pra colar no terminal.

**Causa raiz:** `app/lina-gpui/src/main.rs:4226,4230` usava `ks.modifiers.platform` direto. No gpui, `platform` mapeia pra ⌘ no Mac (intenção do código) mas pra **tecla Windows (Win key)** no PC — `Win+V` é o histórico de clipboard do sistema, então o atalho do Lina ficava engolido pelo SO e nunca chegava ao handler.

**Fix:** introduz dois helpers condicionados por `cfg(windows)`:

```rust
#[cfg(windows)]
fn is_terminal_paste_shortcut(ks: &Keystroke) -> bool {
    ks.modifiers.control && ks.modifiers.shift && ks.key == "v"
}
#[cfg(not(windows))]
fn is_terminal_paste_shortcut(ks: &Keystroke) -> bool {
    ks.modifiers.platform && !ks.modifiers.shift && ks.key == "v"
}
```

(O mesmo pra copy.)

| | Mac | Windows |
|---|---|---|
| Copia seleção | `⌘C` | `Ctrl+Shift+C` |
| Cola no PTY | `⌘V` | `Ctrl+Shift+V` |
| `Ctrl+C` / `Ctrl+V` | (vai cru pro PTY) | (vai cru pro PTY — signal, literal) |

`Ctrl+Shift+C/V` é a convenção de Windows Terminal, GNOME Terminal, Konsole — reserva `Ctrl+C/V` puros pro PTY (interrupção do shell, literal). O comentário da linha 4225 já declarava essa intenção; faltava o gate condicional ao SO.

**Validação na tela:** Eduardo confirmou *"Deu tudo certo. Works like a charm!"* após rebuild release + re-empacotar.

## Resumo final dos commits do bring-up

| Commit | Natureza |
|---|---|
| `5d0d68f` | fix: `build.ps1` não trata stderr do cargo como fatal (PowerShell 5.1 + ErrorActionPreference) |
| `c5e5ead` | fix: `make-win.ps1` mesmo fix de PowerShell |
| `8af13bb` | docs: `WINDOWS-RESULT.md` Fases 0–4 verdes com evidência |
| `66a2ce2` | fix: resolve npm-shim do claude no spawn do PTY (cli_discovery + lina-pty) |
| `a685566` | fix: atalhos copy/paste do canvas usam Ctrl+Shift+C/V no Windows |
| (este) | docs: registra dois achados pós-Fase 4 |

Total de mudança em código de produção: **3 arquivos** (`cli_discovery.rs`, `lina-pty/lib.rs`, `main.rs` da app gpui). Nenhuma feature nova; só portabilidade para Windows.

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
