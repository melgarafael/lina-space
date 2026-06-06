# CLAUDE.md — Lina Space (guardião do norte)

> **Leia isto antes de tocar em qualquer código.** Este arquivo é carregado automaticamente em toda sessão. Ele garante que você — humano ou IA — nunca construa uma parte do MVP sem ter em contexto **onde queremos chegar**.

> 🪟 **VOCÊ ESTÁ NUMA MÁQUINA WINDOWS?** Então sua missão é o **bring-up do Windows**: leia
> **`WINDOWS-BRINGUP.md`** (raiz) e invoque a skill **`/windows-bringup`**. NÃO use WSL (decidido);
> NÃO re-arquitete os terminais; fallback é Slint. Tudo está naquele doc + na skill.

## O que é o Lina Space

App **desktop Rust-nativo, GPU-first** (Windows/Mac/Linux): um **canvas para múltiplos terminais de IA** voltado a empreendedores **não-técnicos**. Cada agente é um terminal vivo rodando um CLI de IA (Claude Code, Codex…); todos cooperam **automaticamente e sem fios**, com tudo estruturado e nada se perdendo num crash. **Sem LLM/harness/chat próprios** — a inteligência vem do CLI de terceiro; o "chat" é sempre o terminal.

Visão completa, roadmap e features: **vault Obsidian → `Debriefing Vibe Coding/`** (este projeto nasceu de um debriefing completo). Norte canônico: `01 - Visao de Produto e Norte de Continuidade`. SPEC: `30 - SPEC Mestre`. Stack: `31 - Decisao de Stack`. **Backlog atual: `34 - Epico Fase 1`** (a Fase 0/MVP está **fechada e validada na tela** — `32 - Epico Fase 0 (MVP)`).

## Invariantes inegociáveis (valem em TODAS as fases — quebrar é mudar o produto)

1. **Sem LLM/harness/chat próprios.** Orquestramos CLIs de terceiros; o chat é o terminal.
2. **Local-first.** Nada sai da máquina por padrão; exposições são opt-in sinalizado.
3. **Neutralidade multi-CLI.** Nunca refém de um CLI/LLM. Trocar de CLI = trocar um `CLI Profile` (TOML).
4. **O event log é a fonte da verdade.** Layout/buffers/tabelas são projeções reconstruíveis.
5. **Pertencimento = conexão.** Topologia derivada de quem está no Espaço, não de cabos.
6. **Não-técnico-first.** Zero jargão na superfície; 1º agente <2h; nunca tela em branco; estado sempre salvo e visível.
7. **Core/shell split.** O core Rust é o ativo durável; a UI é substituível (via trait `UiHost`).

## Âncoras de continuidade — NÃO feche estas portas

Estas abstrações existem na Fase 0 **para o futuro caber**. Mantenha-as vivas:

- **trait `UiHost`** (`lina-host`) — fronteira core↔shell; o core **não importa** tipos de toolkit. Permite trocar o framework de UI (gpui ↔ Slint) sem reescrever o core.
- **trait `VtBackend`** (`lina-vt`) — abstrai o motor VT; permite migrar `alacritty_terminal` → `libghostty-vt` depois.
- **trait `PortalEngine` + `ExternalTextureLayer`** (futuro) — o browser navegável-pela-IA (Fase 1+) encaixa sem refazer o render. Não simplifique a cena a ponto de excluí-lo.
- **CLI Profiles TOML** (`lina-cli-profiles`) — toda especificidade de CLI vive em config. Novos CLIs entram sem recompilar.
- **Event Store** (`lina-core`) — event-sourcing com upcasting versionado; toda a Fase 1 (observabilidade, retenção por estado, inteligência da Lina) é evento/projeção sobre o log — e o backlog F2 (Curador-feed, webhooks ampliados, discovery; ver ADR 0010) também o reusará.
- **Workspace Bus / Supervisor** (`lina-core`) — broker in-process; toda orquestração futura pendura aqui.
- **Envelope A2A versionado** — campos `id/root_cause_id/from/to/intent/hops/await`; não invente formatos por feature.

**Regra de processo:** antes de qualquer decisão arquitetural, pergunte *"isto fecha uma porta acima?"*. Se sim, **pare e registre** um ADR curto em `docs/adr/` — não decida no impulso da story.

## Stack (decidida na Onda 4 do debriefing — ver `31 - Decisao de Stack`)

- **Linguagem:** Rust, **processo único**, GPU-first. Sem runtime JS, sem IPC renderer↔core. **Tauri e Electron descartados.**
- **Terminal:** `portable-pty` (vendorizado, patch 3 flags ConPTY) + `alacritty_terminal` (atrás de `VtBackend`).
- **Render:** `wgpu` (1 Device/Surface; N terminais = geometria, atlas de glifos + instancing) + `vello` (vetor do canvas) + `cosmic-text`/`swash` (texto) + `accesskit` (a11y) + `winit` (janela/IME).
- **Framework de UI:** ✅ **gpui** (decidido na Onda 1 por spike medido — 120Hz + AccessKit 1ª classe; ver vault `33 - Decisao de Framework de UI`). Pin de SHA `09165c15…` + vendoring + `runtime_shaders` no macOS. O core fica atrás de `UiHost` (porta de troca p/ Slint preservada se a governança do gpui piorar). Shell em gpui.
- **Persistência:** `rusqlite` (SQLite WAL) + JSONL append-only + snapshots. Webhooks: `axum`. Segredos: `keyring`.
- **A2A:** injeção no PTY vivo **faseada** (bracketed-paste → `submit_delay` ~0.3s → Enter `0x0D` separado; fila serial por terminal); `--resume` como fallback; declarado por CLI Profile.

## Como trabalhar (Fase 1 — EM EXECUÇÃO desde 2026-06-06)

- O backlog vivo é o épico **`34 - Epico Fase 1`** (vault): 7 ondas F1-0..F1-6, 60 stories, cada uma com **critério de aceite observável** (não "implementado" — algo que *roda e se mede*). **Regra de precedência:** o 34 consolida; as 7 peças em `tasks/epico-f1/` são a fonte da verdade story-a-story; em conflito, a peça vence o 34 — e o ADR aceito vence ambos.
- **Fios condutores do fundador** (toda story deve *operacionalizá-los*, não citá-los): governança · observabilidade · inteligência da Lina (orquestração/cognição do trabalho) · auto-aprimoramento (**sugere, nunca aplica** — gate humano) · gates de qualidade.
- **ADRs-gate:** stories marcadas "antes de" no §V do 34 **não iniciam** sem o ADR aceito em `docs/adr/` (0008 detecção de CLI · 0009 aprovação remota · 0010 multi-workspace + escopo F1 · 0019 definições operacionais).
- **Protocolo multi-terminal:** cada worker recebe uma **fronteira de arquivos** no despacho; costuras (`events.rs`, `router.rs`, `lib.rs`) têm **dono único por rodada** — precisa de algo lá? Peça ao Maestro; nunca edite, nunca reverta linha de peer. Workers **não commitam**; o Maestro valida de fora (**exit codes diretos** — pipe mascara; `touch` fura cache de lint) e commita por fatia.
- **Doutrina de segurança (não regrida):** nenhum campo escrito por agente (`from`/payload/contrato/filename) decide identidade, ordem ou autorização — a autenticação em duas camadas é a única autoridade; **contrato é dado transportado, jamais autoridade**. Ação irreversível exige gate humano. Toda story que tocar `deliver_a2a`/`Router` tem como critério implícito a suíte de segurança do router verde.
- Cada story carrega **"Por quê (fonte 13.x)"** — a pesquisa está embutida na story. Se a sua implementação contradisser a fonte citada, **pare e escale**; não "adapte" em silêncio.
- Convenções: edition 2021; `cargo fmt` + `clippy -D warnings` limpos **por pacote**; testes que provam o critério; sem `unwrap()` em produção; **eventos aditivos** (`serde(default)`) — replay de log antigo nunca quebra.
- Estado vivo: board `lina-space-board-sessao` (nota Maestri) + status §II do 34 + `_HANDOFF` (dever permanente: atualizar a cada marco).

> **Em uma frase:** construímos o MVP como **fundação do produto de 5 anos** — não como protótipo descartável. Toda story mantém uma porta aberta para o futuro.
