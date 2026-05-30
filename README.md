# Lina Space

> O estúdio de IA para quem não programa. Canvas desktop de múltiplos terminais de IA, **Rust-nativo GPU**, cross-platform (Win/Mac/Linux), local-first.

**Comece por `CLAUDE.md`** (o norte). Visão, roadmap e specs completas no vault Obsidian → `Debriefing Vibe Coding/` (`01 - Visao de Produto e Norte de Continuidade`, `30 - SPEC Mestre`, `31 - Decisao de Stack`, `32 - Epico Fase 0`).

## Estado

**Fase 0 — MVP** (em construção). Gate de saída: o core roda headless nos 3 SOs, abre N PTYs, injeta A2A faseada e recupera de `kill -9`.

## Workspace

| Crate | Story | Papel |
|---|---|---|
| `lina-host` | W0-11 | trait `UiHost` — fronteira core↔shell (porta de troca de framework) |
| `lina-pty` | W0-1 | gerenciamento de PTYs cross-platform |
| `lina-vt` | W0-2 | emulação VT atrás de `VtBackend` |
| `lina-core` | W0-3..10 | pty-host, Workspace Bus, Event Store, A2A faseada, recuperação |

## Build

```sh
cargo build
cargo test
cargo clippy --all-targets
```
