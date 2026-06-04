# 🪟 WINDOWS BRING-UP — leia isto primeiro (handoff para uma sessão limpa)

> **PARA O CLAUDE CODE QUE ABRIU ESTE REPO NUMA MÁQUINA WINDOWS:** este documento é
> auto-suficiente. Você está numa **máquina diferente** da do dono do projeto e **não tem** o
> histórico da conversa que originou esta missão — tudo o que precisa está aqui e na skill.
> **Comece invocando a skill `/windows-bringup`** (ela orquestra o processo passo a passo).
> Responda em **pt-br**. O dono (Rafael) é **não-técnico**; o amigo que está rodando você fez o favor
> de emprestar a máquina Windows — seja claro com ele e econômico no que pede.

---

## 1. O QUE É O LINA SPACE (contexto mínimo)

App **desktop Rust-nativo, GPU-first** (framework **gpui**, o do Zed): um **canvas de múltiplos
terminais de IA** para empreendedores **não-técnicos**. Cada agente é um terminal vivo rodando um CLI
de IA (Claude Code, Codex…); todos cooperam **automaticamente e "sem fios"** (A2A). **O Lina NÃO traz
a IA dentro** — ele orquestra o CLI de terceiro (invariante #1). Namespace `lina`/`.lina`.

**Estado atual:** as Ondas 0→5 (parte-Mac) estão **fechadas e validadas na tela no macOS**. Existe um
`Lina.app` (macOS) baixável. O **core Rust é 100% agnóstico de SO** (verificado: `cargo tree -p
lina-core` não tem toolkit de UI; `cfg(windows)` só aparece em `cli_discovery.rs`). **Falta o
Windows** — que é o que VOCÊ vai trazer à vida nesta máquina.

**Por que Windows importa:** ~80% dos alunos do dono usam Windows. Hoje o app só roda no Mac (20%).

---

## 2. A MISSÃO

Trazer o Lina para **Windows x86_64 nativo**, produzindo um **`.exe` distribuível** que um aluno
baixa e roda. Em fases, com um **checkpoint de de-risk** logo no início.

**Caminho escolhido = NATIVO (gpui-Windows + ConPTY). NÃO use WSL.** (Investigação já feita: o gpui
renderiza via Vulkan e o WSLg **não acelera Vulkan** → ficaria lento; + instalar WSL atrita demais
para alunos não-técnicos. Detalhe em `packaging/DE-RISK-WINDOWS.md`.)

---

## 3. RESTRIÇÕES INEGOCIÁVEIS (decisões do dono — NÃO viole)

1. **NÃO re-arquitete os terminais.** O `portable-pty` (em `crates/lina-pty`) **já é** a abstração
   cross-platform e **já tem** o backend Windows (ConPTY). Sua tarefa é **LIGAR** esse backend
   (vendorizar + aplicar o patch dos 3 flags + bundlar `conpty.dll`/`OpenConsole.exe`), **não**
   redesenhar a camada de terminal. A arquitetura (`PtyManager` → `VtBackend`/alacritty → pty-host →
   Bus) fica **idêntica**.
2. **Se o gpui-Windows falhar no de-risk, o fallback é SLINT — NUNCA WSL.** (Slint: render mais leve,
   IME/a11y melhores, terminal intocado, troca via a trait `UiHost`. WSL foi descartado com evidência.)
   E você **não** implementa o Slint sozinho — você **reporta a falha e PARA** para o dono decidir.
3. **Sem assinatura de código.** O dono abriu mão de certificado Authenticode. O `.exe` sai **não
   assinado**; os alunos passam pelo aviso do **SmartScreen** ("Mais informações → Executar assim
   mesmo"). Documente isso para eles (`packaging/windows/INSTALAR-LINA-WINDOWS.md`).
4. **gpui NÃO roda headless.** Você **não vê a tela**. Qualquer afirmação visual (renderizou? acentos
   funcionam? o terminal aparece?) **TEM que ser confirmada pelo amigo OLHANDO a tela** — peça a ele e
   registre a resposta dele. **Nunca declare "renderiza/funciona" por build verde.** (Esta é a lição
   #1 do projeto inteiro — custou vários fixes errados no Mac.)
5. **Validar por evidência, não por vibe.** Redirecione a saída de build/test para arquivo e LEIA;
   capture `$LASTEXITCODE` direto; registre números/erros reais no `WINDOWS-RESULT.md`.
6. **Convenções de código** (do `CLAUDE.md` na raiz): edition 2021; `cargo fmt`/`clippy` limpos; **sem
   `unwrap()`/`expect()` em caminho de produção** (erros com `thiserror`/`anyhow`).

---

## 4. GIT / COLABORAÇÃO

- Você está num **clone**. Trabalhe num branch dedicado: **`windows-bringup`** (a skill cria).
- **Commits pequenos e semânticos** conforme avança. **NÃO** force-push, **NÃO** toque na `main`
  diretamente, **NÃO** rode `git reset --hard`/`git clean` destrutivos.
- Ao final (ou ao parar por falha), **push do branch + abra um PR** (`gh pr create`) com o
  `WINDOWS-RESULT.md` no corpo. O dono revisa e mergeia. Assim as mudanças Windows entram **validadas
  em Windows real**, não às cegas.
- Mensagem de commit termina com: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## 5. PLANO EM FASES (a skill `/windows-bringup` executa; árvore de decisão embutida)

| Fase | O quê | Gate |
|---|---|---|
| **0. Setup** | branch `windows-bringup`; `packaging/windows/check-env.ps1` verifica MSVC + Rust + Windows SDK + **arquitetura (precisa x86_64)** | toolchain OK |
| **1. Build** | aplicar o cfg do Cargo (`runtime_shaders` vira **macOS-only**); `cargo build` no app gpui; iterar features do gpui-Windows pelo compilador | compila |
| **2. DE-RISK** 🔑 | rodar o app; **o AMIGO olha a tela** e confirma: (a) renderiza terminais? (b) acentos `´+a=á`? (c) `Ctrl+T` cria agente? (d) a11y (Narrator) opcional | **decide tudo** |
| **3. ConPTY** *(só se Fase 2 PASSA)* | vendorizar `portable-pty` + patch 3 flags + bundle `conpty.dll`/`OpenConsole.exe`; validar A2A faseada (A→B) | A2A entrega |
| **4. Empacotar** *(se passa)* | `make-win.ps1`: `.exe` + `lina.exe` + `assets/` + `.zip` distribuível; smoke do app empacotado | `.exe` abre |
| **5. Reportar** | preencher `WINDOWS-RESULT.md` com evidência; commit; push branch; `gh pr create` | PR aberto |

**Árvore de decisão da Fase 2 (de-risk):**
- ✅ **PASSA** (renderiza + acentos + cria terminal): **siga sozinho** até a Fase 5 (o dono autorizou
  "ir até o `.exe` se passar").
- ❌ **FALHA** (não renderiza, ou acentos/a11y quebrados sem conserto trivial): **PARE**. Preencha o
  `WINDOWS-RESULT.md` explicando exatamente o que falhou (com o que o amigo viu na tela + logs) e
  **recomende o caminho Slint** (não tente Slint nem WSL agora). Push + PR. O dono decide.
- ⚠️ **Não compila / arquitetura errada** (ARM em vez de x86_64): registre e pare; a máquina precisa
  ser x86_64 Windows.

---

## 6. PRÉ-REQUISITO DO ALUNO (vale pra qualquer entrega)

O Lina **orquestra** a IA. Cada aluno precisa do **Claude Code (`claude`) instalado e logado** no
Windows. Isso entra no guia de instalação deles. (Você, agente, também precisa do ambiente de build —
ver Fase 0.)

---

## 7. ONDE ESTÁ CADA COISA

- **Skill executável:** `.claude/skills/windows-bringup/SKILL.md` → invoque `/windows-bringup`.
- **Scripts:** `packaging/windows/{check-env.ps1, build.ps1, make-win.ps1}`.
- **Template do relatório:** `packaging/windows/WINDOWS-RESULT.template.md` (copie p/ `WINDOWS-RESULT.md`).
- **Guia do aluno (saída):** `packaging/windows/INSTALAR-LINA-WINDOWS.md`.
- **Por que nativo e não WSL (investigação):** `packaging/DE-RISK-WINDOWS.md`.
- **Norte do projeto:** `CLAUDE.md` (raiz) — invariantes + stack.
- **Empacotamento Mac (referência/analogia):** `packaging/make-app.sh` + `packaging/Info.plist`.

> **Em uma frase:** ligue o backend Windows que o terminal já tem, prove o trio render/IME/a11y do
> gpui na tela (olho do amigo), e — se passar — entregue um `.exe` x86_64 não-assinado que o aluno roda.
