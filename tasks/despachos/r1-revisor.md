# DESPACHO r1-ci-3so — Revisor
**id:** `ci-3so` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

## Story: F1-6-8 · CI 3-SO real — promover `core-windows` a gate bloqueante (começar CEDO na onda, por isso está na Rodada 1)
**Fonte integral:** `tasks/epico-f1/ondas-5-6.md` linhas 216-222. O próprio `ci.yml` traz a instrução: trocar `--no-run` por execução real e remover `continue-on-error`. O bring-up Windows foi mergeado (`8d21f55` + `WINDOWS-QA-POS-MERGE.md`).

## Fronteira de arquivos (sua, dono único nesta rodada)
- `.github/workflows/ci.yml`
- Anotações `#[cfg(unix)]`/`#[cfg(windows)]` em ARQUIVOS DE TESTE dos crates (SÓ atributos de cfg + comentário de 1 linha; ZERO mudança de lógica de teste)
- `tasks/epico-f1/ci-3so-triagem.md` (NOVO — a tabela teste×SO)
- **NÃO toque:** `crates/lina-core/src/events.rs`/`workspace.rs` (dono: Dados), `lina-webhooks/src` (dono: Arquiteto — os TESTES dele também são dele; se um teste de webhooks precisar de cfg, REGISTRE o pedido), `app/`.

## O quê
1. **Promover o job `core-windows`:** `cargo test --workspace -- --test-threads=1` executando de verdade; remover `continue-on-error`; `timeout-minutes` justo (atenção ao histórico: hang de ~70min em `flow_control_caps_memory_under_flood` — risco real).
2. **Triagem dos testes Unix-only:** varra os testes do workspace por dependências de Unix (`yes`, `sh`, sinais SIGTERM/SIGKILL, paths /tmp, PTY semantics). Cada um: `#[cfg(unix)]` com comentário do porquê OU equivalente Windows real OU skip explícito documentado. **Skip silencioso não conta como verde** — TUDO listado na tabela.
3. **`ci-3so-triagem.md`:** tabela teste×SO (roda / cfg-unix+porquê / pendente-windows), o diff do ci.yml explicado, e o checklist do que SÓ um run real no GitHub prova (o push é do Maestro; você entrega tudo pronto-para-push).
4. **Validação local:** a suíte completa macOS segue verde (workspace, --test-threads=1, exits diretos) — você não tem Windows local; o que for risco-Windows fica NOMEADO na tabela, não chutado.

## Entrega
`tasks/epico-f1/.entrega-ci-3so.md`. Marcador: `.iniciado-ci-3so`.
