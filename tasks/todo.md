# Lina Space — Plano de Construção

> ✅ **ONDA 0 COMPLETA (11/11)** — 55 testes verdes, 6 commits. Core headless: PTY+VT+Bus+EventStore+Recovery+A2A.

> Painel compartilhado. Backlog canônico: vault `Debriefing Vibe Coding/32 - Epico Fase 0 (MVP)`.
> Gate de saída da Onda 0: core roda **headless nos 3 SOs**, abre N PTYs, injeta A2A faseada, recupera de `kill -9`.

## Onda 0 — Core framework-agnóstico (Trilho A)

| Story | Crate | Status | Dono | Critério de aceite (resumo) |
|---|---|---|---|---|
| W0-1 PTY Manager | `lina-pty` | ✅ done (Mac) | LLM Engineer | 4 testes ok · 8 PTYs PID distinto · single-owner |
| W0-2 VtBackend + alacritty | `lina-vt` | ✅ done | Arquiteto | 6 testes ok · grid célula-a-célula · bracketed-paste · damage |
| W0-3 pty-host isolado | `lina-core` | ✅ done | LLM Engineer | flow-control + panic isola painel; flakiness corrigida (serial_test) |
| W0-4 Workspace Bus / Supervisor | `lina-core` | ✅ done | LLM Engineer | 5 testes bus_tests · serial sem interleave · cycle_detected · role→nó vivo · pub/sub |
| W0-5 Event Store | `lina-core` | ✅ done | LLM Engineer | replay determinístico + upcasting; SQLite WAL + JSONL |
| W0-6 Recuperação pós-crash visível | `lina-core` | ✅ done | LLM Engineer | corrupt_db recupera do JSONL; Recovering→Recovered |
| W0-7 Secret Vault | `lina-secrets` | ✅ done | Pesquisador | 4 testes; SecretStore trait + MockStore headless; namespacing |
| W0-8 CLI Profiles TOML | `lina-cli-profiles` | ✅ done | Analista | 9 testes; Delivery/EndSignal enums; registry override; TOML inválido erra limpo |
| W0-9 Entrega A2A faseada | `lina-core` | ✅ done | LLM Engineer | sequência faseada · fila serial · sanitiza ESC[201~ · allow-list |
| W0-10 Fim-de-resposta | `lina-core` | ✅ done | LLM Engineer | result>idle>timeout · nunca trunca silencioso |
| W0-11 trait UiHost | `lina-host` | ✅ done | Arquiteto | 5 testes · UiHost+InputSink+HeadlessUiHost · zero deps toolkit |

## Log
- Scaffold do workspace criado (CLAUDE.md, Cargo workspace, 4 crates stub que compilam).
