# De-risk do Windows nativo (gpui + ConPTY) — guia turnkey p/ a VM

> **Decisão (2026-06-04):** Windows é prioridade (80% dos alunos). Caminho escolhido = **nativo**
> (gpui-Windows DirectX + ConPTY), NÃO WSL (Vulkan não acelera sob WSLg + atrito de instalação para
> leigos). ConPTY **não** re-arquiteta os terminais — só liga o backend Windows que o `portable-pty`
> já tem. Assinatura: **dispensada** (bypass do SmartScreen, como no Mac).
>
> **Este doc é o PRIMEIRO passo: um spike de de-risk de 3-5 dias que decide go/no-go ANTES de
> qualquer investimento maior.** Ele responde a única pergunta que mata o projeto Windows:
> **o trio `terminal-GPU + acentos(IME) + acessibilidade` funciona no gpui em Windows?**
> (Nunca foi testado — o spike original rodou só no macOS.)

---

## ⚠️ Leia primeiro: o que a VM Parallels consegue e o que NÃO consegue

A Parallels num Mac Apple Silicon roda **Windows 11 ARM64**. Consequências:

| Serve para | Não serve para |
|---|---|
| **De-risk do trio** (renderiza? acentos? a11y?) — o que importa agora | **FPS real** (a GPU é virtualizada → números não confiáveis) |
| Trabalho de **ConPTY** + smoke de A2A | O **binário dos alunos** (eles usam **x86_64**; a VM builda **ARM64**) |
| Testar a interface, o boot, o onboarding | Validação final de performance/qualidade |

**Risco específico da VM (seja honesto no resultado):** o gpui-Windows é primariamente **x86_64**;
o suporte a **ARM64 Windows** é menos certo. Se o `cargo build` falhar por ser ARM64 (não por bug
nosso), isso é um achado: o de-risk de verdade exigiria uma máquina **x86_64 Windows**. Tente o build
ARM64 nativo primeiro (mais barato); se ele não compilar o gpui, anote e me avise.

**Para o binário que vai pros alunos (x86_64):** de dentro do Windows ARM dá pra cross-buildar
(`rustup target add x86_64-pc-windows-msvc` + `cargo build --target x86_64-pc-windows-msvc`), mas a
GPU desse binário só se valida numa máquina x64 real (ou na máquina de um aluno). Isso é fase
posterior — **o de-risk vem primeiro**.

---

## Passo 1 — Preparar a VM (uma vez, ~30-40 min)

Dentro do Windows 11 (na Parallels):

1. **Visual Studio Build Tools 2022** (MSVC — o gpui NÃO suporta MinGW/MSYS2):
   - Baixe "Build Tools for Visual Studio 2022" da Microsoft.
   - No instalador, marque **"Desktop development with C++"** (inclui MSVC + Windows 11 SDK ≥ 10.0.20348).
2. **Rust:** instale o `rustup` (https://rustup.rs) — ele detecta o MSVC. Toolchain default = `stable-msvc`.
3. **Git** (https://git-scm.com).
4. Reinicie o terminal (PowerShell) pra pegar o PATH.

## Passo 2 — Levar o código pra VM

Opção simples: na Parallels, **compartilhe a pasta** do repo do Mac (`/Users/rafaelmelgaco/einstein workspace/lina-space`) com a VM — mas builde numa **cópia LOCAL** dentro da VM (build em pasta compartilhada é lento e dá problema de lock). Ou clone via Git se você subir pro GitHub.

```powershell
# dentro da VM, numa pasta local (ex.: C:\dev)
cd C:\dev
# copie a pasta lina-space pra cá (do compartilhamento) OU git clone
cd lina-space\app\lina-gpui
```

## Passo 3 — Ajustar o Cargo.toml (a mudança que falta)

O `runtime_shaders` é **macOS-only** (compila shaders Metal). Em Windows ele precisa sair.
Em `app/lina-gpui/Cargo.toml`, **substitua** a linha do `gpui_platform` (que hoje é única, em
`[dependencies]`) por blocos por-SO:

```toml
# (deixe a linha do `gpui` cheio onde está, em [dependencies] — ela vale p/ todos os SOs)

# macOS: runtime_shaders (Metal, dispensa toolchain offline)
[target.'cfg(target_os = "macos")'.dependencies]
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "09165c15dc5d1fea93604231eaf30ca4c25f1cd6", default-features = false, features = ["font-kit", "runtime_shaders"] }

# Windows/Linux: SEM runtime_shaders (não é Metal). features do backend = AJUSTAR pelo compilador.
[target.'cfg(not(target_os = "macos"))'.dependencies]
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "09165c15dc5d1fea93604231eaf30ca4c25f1cd6", default-features = false, features = ["font-kit"] }
```

> **Itere aqui pelo compilador:** se o build reclamar que falta o backend de render (ex.: nenhum
> Direct3D/Vulkan habilitado), tente `default-features = true` no bloco Windows, ou adicione a feature
> de backend que o erro indicar. É exatamente o que eu NÃO consigo adivinhar do Mac — o `cargo` da VM
> te diz a verdade. Anote o feature set que funcionar (vira a config oficial do Windows).

## Passo 4 — Buildar e RODAR (o de-risk)

```powershell
cargo build           # ARM64 nativo; primeira vez é LENTA (árvore gpui é grande, ~10-20 min)
cargo run             # abre a janela
```

Sem `LINA_DEMO`, o boot de produção mostra o **onboarding** (1ª execução). Para ir direto a 2
terminais de teste, rode com a env de demo:

```powershell
$env:LINA_DEMO=1; cargo run
```

## Passo 5 — Observar o TRIO (o que decide tudo)

Com a janela aberta, confira **na tela**:

1. **Renderiza?** Os terminais aparecem, com texto/cores/cursor? (Se a tela fica preta/quebrada, o
   backend DirectX do gpui não subiu — achado crítico.)
2. **Acentos (IME)?** Num terminal, digite `´` depois `a` → deve sair **á**. Teste `ã ç ê õ`.
   (É o caminho de IME do gpui no Windows, nunca exercitado.)
3. **Cria terminal?** `Ctrl+T` (no Windows o atalho pode ser Ctrl, não Cmd) cria um novo agente?
4. **Acessibilidade (opcional, se algum aluno usa leitor de tela):** abra o **Narrator** (Win+Ctrl+Enter)
   e veja se ele lê algo da janela. (a11y do grid pode ser eixo manual — não é bloqueador do de-risk.)
5. **A2A (se usou LINA_DEMO):** clique no botão "⚡ Enviar A2A (A→B)" → o terminal B reage?
   (ConPTY básico já vem no `portable-pty` 0.9 do crates.io — o vendoring+patch dos 3 flags é
   refinamento POSTERIOR p/ robustez da injeção, não precisa pro de-risk.)

## Critério de PASSA / FALHA

- ✅ **PASSA** (renderiza + acentos + cria terminal): o gpui-Windows serve. **Próximos passos:**
  (a) vendorizar `portable-pty` + patch 3-flags ConPTY + bundle `conpty.dll`/`OpenConsole.exe` (robustez A2A);
  (b) cross-build x86_64 + empacotar `.exe`/`.zip` (análogo ao `make-app.sh` do Mac) + guia de SmartScreen p/ alunos;
  (c) validar FPS/qualidade numa máquina x64 real.
- ❌ **FALHA** (não renderiza, ou acentos/a11y quebrados sem conserto rápido): **pivote para Slint**
  — render mais leve, IME/a11y melhores, e o **terminal fica intocado** (alacritty + PTY agnósticos).
  NÃO volte pro WSL (Vulkan-software + atrito de leigo são piores). Slint = ~2-3 semanas de casca nova
  sobre a MESMA trait `UiHost` (core intocado — foi desenhado pra esse swap).
- ⚠️ **INCONCLUSIVO** (não buildou por ser ARM64): precisa de uma máquina **x86_64 Windows** p/ o
  de-risk real. A VM ARM não foi suficiente — anote e busque um PC x64 (emprestado/barato/CI).

---

## Pré-requisito dos alunos (vale pra qualquer caminho)

O Lina **orquestra** a IA, não a traz dentro (invariante #1). Cada aluno precisa do **Claude Code
(`claude`) instalado e logado** no Windows. Isso entra no material de onboarding deles.

---

*Eu (o assistente no Mac) NÃO consigo rodar dentro da VM — o build/run do Windows é você quem executa.
Me mande o resultado do Passo 5 (renderizou? acentos? buildou ARM64?) e eu sigo: se PASSA, preparo o
vendoring do ConPTY + o empacotamento Windows; se FALHA, montamos o plano Slint.*
