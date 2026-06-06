# Windows QA pós-merge — roteiro pro Gabriel

> **Contexto:** enquanto você fazia o bring-up, a main ganhou ~5.600 linhas novas (onboarding
> "Segundo cérebro" Obsidian, instaladores de CLI por SO, tela de ferramentas dev, `lina vault`,
> fixes de A2A). Tudo isso foi mergeado junto com o seu PR #1 (`8d21f55`) e **compila verde no
> Windows** (CI 5/5, MSVC + clippy), mas **nunca foi executado num Windows real** — o seu teste
> de 100% foi na branch antes dessas features. Este roteiro cobre só o que precisa do seu olho.
> Tempo estimado: **~30–40 min**.

## Preparação

```powershell
git checkout main && git pull
powershell -ExecutionPolicy Bypass -File packaging\windows\make-win.ps1
```

Rode o `lina-gpui.exe` de `dist\Lina-win\` (o resolver acha `lina.exe` e `assets\` ao lado, como no seu de-risk).

⚠️ **NÃO rode `cargo test --workspace` esperando verde** — os testes de PTY ainda spawnam
`yes`/`sh` (Unix) e penduram no Windows (o CI compila com `--no-run` por isso). Esse gate é um
item futuro, não deste roteiro.

---

## Teste 1 — Onboarding "Segundo cérebro" (detecção do Obsidian) · ~10 min

O código lê `%APPDATA%\obsidian\obsidian.json` e lista seus vaults (lógica coberta por teste
unitário, mas nunca executada num Windows de verdade).

1. No onboarding, avance até o passo **Segundo cérebro**.
2. **Observar:**
   - [ ] Seus vaults do Obsidian aparecem listados com os caminhos certos (formato `C:\...`)?
   - [ ] Selecionar um vault e confirmar gera o índice (PageIndex) **sem travar a UI** (o scan é off-thread e paralelo)?
   - [ ] Se algum vault estiver no **OneDrive**: o scan termina em tempo razoável (minutos, não dezenas de minutos)? Anote a duração e o nº de notas.
   - [ ] Caso você NÃO tenha Obsidian instalado nesta máquina: a tela degrada com mensagem clara (sem crash)?

## Teste 2 — Instaladores de CLI no ConPTY oculto · ~10 min ⭐ o mais importante

Primeira interseção real dos dois trabalhos: o instalador silencioso roda **dentro do ConPTY que
você ligou**. Receitas Windows usam `powershell`/`winget`/`npm` (nunca executadas).

1. No onboarding, na etapa de assistentes/CLIs, escolha **1 ou 2 CLIs que você NÃO tem** (ex.: `gemini` ou `copilot` — vão de `npm install -g`; requer Node ≥ 20).
2. Clique **"Instalar para mim"**.
3. **Observar:**
   - [ ] O pulso de progresso anda (prova de que o ConPTY oculto está entregando bytes)?
   - [ ] Ao final, a verificação acusa o CLI como instalado (o app re-checa `$(npm prefix -g)` etc.)?
   - [ ] O binário funciona num terminal seu depois (`gemini --version`)?
   - [ ] Se a instalação falhar: a UI mostra erro acionável (não trava em "instalando..." pra sempre)?

## Teste 3 — Tela "Ferramentas de desenvolvimento" · ~5 min

Detecção de git/gh/vercel/node/python no PATH do Windows + fallback de instalação.

- [ ] As ferramentas que você TEM aparecem como detectadas (versões certas)?
- [ ] Para uma que falte: o botão de instalar **não crasha** — no Windows ele deve cair na instrução "rode manualmente" (abrir terminal automático é só macOS por enquanto, by design)?

## Teste 4 — `lina vault` por um agente · ~5 min

Requer o Teste 1 concluído (vault linkado → `<LINA_HOME>\vault.json`).

Num terminal de agente dentro do Lina (ou shell com `LINA_HOME` apontado):

```
lina vault path      # → imprime o caminho C:\... do vault
lina vault search <termo-que-existe-nas-suas-notas>
lina vault read <uma-nota.md>
```

- [ ] Os três respondem com conteúdo correto e caminhos Windows íntegros (sem `\` quebrado/mojibake)?

## Teste 5 — Atalhos (rápido) · ~2 min

- [ ] **Ctrl+T** abre novo terminal (seu `shortcut_modifier` — deve seguir funcionando).
- [ ] Zoom: hoje é **Win+`=`/`-`/`0`** (limitação conhecida — esses atalhos novos da main ainda usam só a tecla plataforma; follow-up registrado pra migrar pro seu padrão). Só confirme que **não interferem** em nada do terminal.

---

## Como reportar

Mesmo modelo do seu `WINDOWS-RESULT.md`: para cada item, **PASS/FAIL + 1 linha de evidência**
(print ou colagem do output). FAIL em qualquer item → abra issue com o título
`win-qa: <teste> — <sintoma>` e me marque. Nada aqui bloqueia o que já foi mergeado; é o
checklist final antes de abrirmos distribuição pra usuários Windows.
