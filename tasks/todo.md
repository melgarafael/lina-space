# Lina Space — Plano de Construção (Onda 0)

> Painel compartilhado. Backlog canônico: vault `Debriefing Vibe Coding/32 - Epico Fase 0 (MVP)`.
> Gate de saída da Onda 0: core roda **headless nos 3 SOs**, abre N PTYs, injeta A2A faseada, recupera de `kill -9`.

## Onda 0 — Core framework-agnóstico (Trilho A)

| Story | Crate | Status | Dono | Critério de aceite (resumo) |
|---|---|---|---|---|
| W0-1 PTY Manager | `lina-pty` | ✅ done (Mac) | LLM Engineer | 4 testes ok · 8 PTYs PID distinto · single-owner |
| W0-2 VtBackend + alacritty | `lina-vt` | ✅ done | Arquiteto | 6 testes ok · grid célula-a-célula · bracketed-paste · damage |
| W0-3 pty-host isolado | `lina-core` | ✅ done | LLM Engineer | flow-control + panic isola painel; flakiness corrigida (serial_test) |
| W0-4 Workspace Bus / Supervisor | `lina-core` | 🔲 a fazer | — | writes seriais sem interleave; ciclo detectado |
| W0-5 Event Store | `lina-core` | 🔲 a fazer | — | replay determinístico (hash igual 2x) |
| W0-6 Recuperação pós-crash visível | `lina-core` | 🔲 a fazer | — | reconstrói após kill -9, banner |
| W0-7 Secret Vault | `lina-secrets` | ✅ done | Pesquisador | 4 testes; SecretStore trait + MockStore headless; namespacing |
| W0-8 CLI Profiles TOML | `lina-cli-profiles` | ✅ done | Analista | 9 testes; Delivery/EndSignal enums; registry override; TOML inválido erra limpo |
| W0-9 Entrega A2A faseada | `lina-core` | 🔲 a fazer | — | bracketed-paste → delay → Enter separado; fila serial |
| W0-10 Fim-de-resposta | `lina-core` | 🔲 a fazer | — | result > idle(grid) > timeout, com guards |
| W0-11 trait UiHost | `lina-host` | 🟡 esqueleto | — | core não importa toolkit; shell dummy implementa |

## Log
- Scaffold do workspace criado (CLAUDE.md, Cargo workspace, 4 crates stub que compilam).
