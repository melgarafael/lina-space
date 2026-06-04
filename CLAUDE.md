# CLAUDE.md — Lina Space (guardião do norte)

> **Leia isto antes de tocar em qualquer código.** Este arquivo é carregado automaticamente em toda sessão. Ele garante que você — humano ou IA — nunca construa uma parte do MVP sem ter em contexto **onde queremos chegar**.

> 🪟 **VOCÊ ESTÁ NUMA MÁQUINA WINDOWS?** Então sua missão é o **bring-up do Windows**: leia
> **`WINDOWS-BRINGUP.md`** (raiz) e invoque a skill **`/windows-bringup`**. NÃO use WSL (decidido);
> NÃO re-arquitete os terminais; fallback é Slint. Tudo está naquele doc + na skill.

## O que é o Lina Space

App **desktop Rust-nativo, GPU-first** (Windows/Mac/Linux): um **canvas para múltiplos terminais de IA** voltado a empreendedores **não-técnicos**. Cada agente é um terminal vivo rodando um CLI de IA (Claude Code, Codex…); todos cooperam **automaticamente e sem fios**, com tudo estruturado e nada se perdendo num crash. **Sem LLM/harness/chat próprios** — a inteligência vem do CLI de terceiro; o "chat" é sempre o terminal.

Visão completa, roadmap e features: **vault Obsidian → `Debriefing Vibe Coding/`** (este projeto nasceu de um debriefing completo). Norte canônico: `01 - Visao de Produto e Norte de Continuidade`. SPEC: `30 - SPEC Mestre`. Stack: `31 - Decisao de Stack`. Backlog atual: `32 - Epico Fase 0 (MVP)`.

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
- **Event Store** (`lina-core`) — event-sourcing com upcasting versionado; a Fase 1 inteira (Curador, webhooks, discovery) reusa o log.
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

## Como trabalhar

- O backlog é o épico **`32 - Epico Fase 0 (MVP)`** (vault): 6 ondas, 39 stories, cada uma com **critério de aceite observável** (não "implementado" — algo que *roda e se mede*).
- Estamos na **Onda 0** (core framework-agnóstico). Gate de saída: o core roda **headless nos 3 SOs**, abre N PTYs, injeta A2A faseada, e recupera de `kill -9`.
- Respeite os **pré-requisitos cross-onda** (norte §3 / épico cabeçalho).
- Convenções: edition 2021; `cargo fmt` + `cargo clippy` limpos; cada story entrega **testes que provam o critério de aceite**; sem `unwrap()` em caminho de produção; erros com `thiserror`/`anyhow`.
- Progresso compartilhado em `tasks/todo.md`.

> **Em uma frase:** construímos o MVP como **fundação do produto de 5 anos** — não como protótipo descartável. Toda story mantém uma porta aberta para o futuro.
