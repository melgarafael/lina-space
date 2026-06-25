# DESPACHO — Terminal B (Ultra Code) · Scanner de Poderes (F2-4-1 + F2-4-2) · id: f2-4-core

> Leia `tasks/epico-f2/despachos/f2-4/_contexto.md` + `tasks/despachos/_regras-comuns.md` +
> **`docs/adr/0052-area-de-poderes-scan-determinista.md`** (o contrato — ESPERE o Arquiteto selá-lo) ANTES.

## CONTEXTO
A Área de Poderes precisa de um motor que leia o disco e devolva o inventário de poderes
(skills/plugins/agents/commands/hooks/MCPs) com origem rotulada e os 5 estados leigos. **Nada disso
existe hoje** (grep confirmou: zero leitura de `installed_plugins.json`/`mcpServers`/`.mcp.json`/agents/commands).
Mas há **moldes prontos** no core para copiar — você COMPÕE padrões provados, não inventa (a regressão
conhecida é re-decidir caminhos hardcoded por CLI: ADR 0008 §5 já viveu e resolveu isso).

## FUNÇÃO
Você é o **Dev Core (BACKEND)**. Entrega o **scanner puro + projeção** no core, mais os campos de caminho
no perfil de CLI. **Não toca app, não toca `events.rs`/`lib.rs`** (o Maestro fez a largada — `pub mod powers;`
e o evento já existem como stub).

**Fronteira (LEI):**
- CRIA: `crates/lina-core/src/powers.rs`
- EDITA: `crates/lina-cli-profiles/src/lib.rs` (só os campos aditivos do CliProfile)
- NÃO toca: `events.rs`, `lib.rs`, app, router. Precisa de 1 linha lá? Registre na entrega → Maestro aplica.

## DIRECIONAMENTO
Leia o inventário REAL (`entrega-d4 §I`) e o contrato de tipos do **ADR 0052**. Implemente:

### F2-4-1 — Scanner manifest-first (`powers.rs`)
1. **Tipos** exatamente como o ADR 0052 fixou: `PowerKind`, `PowerOrigin`, `PowerState`, `Power`,
   `PowerInventory` (todos `Serialize+Deserialize`, derive `Debug,Clone,PartialEq`).
2. **Função de scan pura-ish** `scan_powers(roots: &PowerScanRoots, profiles: &[CliProfile], focused_cli: Option<&str>) -> PowerInventory`:
   - **Skills:** varre `<root>/<nome>/SKILL.md`, lê frontmatter com **`crate::skill_index::parse_frontmatter`**
     (REUSE — `skill_index.rs:409`). Frontmatter inválido/arquivo faltando → `PowerState::NeedsRepair`.
     Skill na pasta de um CLI ≠ do terminal focado → `PowerState::InertHere` com a origem do CLI dono.
   - **Plugins:** lê **SÓ** `installed_plugins.json` (manifesto 13KB). **PROIBIDO** abrir a árvore `~/.claude/plugins/` (1,9GB).
   - **MCPs:** lê `~/.claude.json → mcpServers` (global) + por-projeto; `.mcp.json` do projeto aberto se existir.
     Codex: `~/.codex/config.toml → [mcp_servers.*]`. Origem rotulada (Global/Project/Cli).
   - **Agents/Commands:** scan raso de `~/.claude/agents/*.md` e `~/.claude/commands/*.md` (nome + descrição se houver).
   - **Hooks:** lê `~/.claude/settings.json → hooks` (config JSON; conta os tipos configurados).
   - **Caminhos por CLI** vêm dos campos novos do `CliProfile` (NÃO hardcode `~/.claude/...` por CLI; o claude-code
     é um perfil, gemini outro, etc. — inv#3). Para o que não tiver perfil, um default explícito documentado.
3. **Molde a copiar:** `cli_discovery.rs:167 discover_clis_in` (varredura→struct→projeção) e
   `channel.rs:314 from_records` (projeção pura, skip silencioso em erro de decode). A `description` do `Power`
   é a CRUA do manifesto (a tradução leiga é da copy do painel — não traduza aqui).
4. **Evento de auditoria** (se o ADR 0052 mandar): exponha uma função pura que deriva os contadores do
   `PowerInventory` para o Maestro emitir `PowerScanned { counts, total }` na costura. Você NÃO emite (não toca
   `events.rs`); você só fornece os contadores. Só metadados — nunca nome/conteúdo.

### F2-4-2 — Watcher raso (parte PURA no core)
- Função pura `watch_targets(profiles: &[CliProfile], project_dir: Option<&Path>) -> Vec<PathBuf>`: devolve os
  **~5 diretórios-PAI** dos manifestos a observar (pai de `installed_plugins.json`, de `~/.claude.json`, do
  `.mcp.json` do projeto, do `config.toml` do Codex, da pasta `skills/`). **Diretório-PAI, não o inode**
  (atomic saves trocam o arquivo). Documente a política de debounce (≥750ms) como constante/comentário.
- A fiação do `notify` real + integração no loop é do **Maestro (app)**; você entrega só a lista determinística
  + o teste de que ela NUNCA inclui a árvore pesada (`~/.claude/plugins/<repo>`), só os pais rasos.
- Antes de adicionar dep: **cheque se `lina-session-watch`/`lina-hooks` já têm motor de watch reutilizável**
  (relate na entrega o que achou).

### Campos no CliProfile (`cli-profiles/lib.rs`)
- Adicione os campos que o ADR 0052 nomeou (ex.: `skills_dir`, `mcp_config_path`), **aditivos `#[serde(default)]`**.
  O struct é `#[serde(deny_unknown_fields)]` — confirme que os perfis `profiles/*.toml` ainda carregam (teste de round-trip).

## OBJETIVO
Um motor determinístico que, dado o disco real desta máquina, devolve o inventário de poderes com os 5
estados corretos em ~0ms, **sem nunca tocar a árvore de 1,9GB** e **sem nenhum campo do disco virando autoridade**.

## RESULTADO ESPERADO (critérios observáveis — testes não-vacuosos)
- `scan_powers` sobre fixtures temp dirs: skills válidas → `Ready`; frontmatter quebrado → `NeedsRepair`;
  skill em pasta de outro CLI → `InertHere`; plugins lidos só do JSON (teste prova que um arquivo gigante
  na árvore NÃO é aberto — ex.: caminho inexistente/contador de leitura).
- `watch_targets` retorna só os pais rasos; teste prova que `~/.claude/plugins/<qualquer-repo>` **nunca** está na lista.
- Perfis TOML reais ainda carregam com os campos novos (round-trip).
- `cargo test -p lina-core -p lina-cli-profiles -- --test-threads=1`, `cargo clippy -p lina-core -p lina-cli-profiles --all-targets -- -D warnings`,
  `cargo fmt --check` — todos **verdes por pacote**, exit direto.

Reporte ao Maestro ao começar/terminar/travar: `lina ask "@Maestro 01" "<status>" --intent status`.
Entrega `tasks/epico-f2/despachos/f2-4/.entrega-f2-4-core.md` com arquivo:linha + evidência + **a costura
que o Maestro precisa aplicar** (assinatura de `scan_powers` para o bridge; contadores para o evento).
Última linha: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
