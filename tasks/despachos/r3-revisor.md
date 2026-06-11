# DESPACHO r3-f1-6-5 — Revisor
**id:** `f1-6-5` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md`

## F1-6-5 · Doc-vs-código: reconciliar TODAS as promessas (fonte: `ondas-5-6.md` linhas 192-198)
As pré-condições FECHARAM (F1-5-6/5-9 em 1bdaea3 · F1-6-1/6-2 em 6e9f50c · F1-6-3 em 56488e1): varra `scrollback.rs`, `lina-vt` (harvest/captura), `lina-webhooks` e docs `WINDOWS-*`/CLAUDE.md do repo que descrevam durabilidade/replay/retention — ajuste cada promessa para EXATAMENTE o que o código pós-F1 garante (fronteiras: janela kill -9 ~1 tick+idle; perda SINALIZADA sob SU; retenção 30d; replay de WebhookConfigured EXISTE agora). **Diff SÓ de comentários/docs** (achou divergência que exige código → registre como achado, não conserte). Tabela promessa→teste na entrega (toda garantia citada nomeia o teste verde que a prova, ou foi reescrita).

## Fronteira (sua)
Comentários/docstrings em `crates/lina-core/src/scrollback.rs`, `crates/lina-vt/src/lib.rs`, `crates/lina-webhooks/src/lib.rs` · `WINDOWS-BRINGUP.md`/`WINDOWS-QA-POS-MERGE.md` (o aviso 'não rode --workspace' está OBSOLETO pós-CI-3SO — atualize) · `CLAUDE.md` do repo SÓ se citar garantia defasada.
**⛔ NÃO toque:** código executável (diff de docs!) · território externo · tasks/epico-f1/ondas-*.md (peças são históricas).
Entrega: `tasks/epico-f1/.entrega-f1-6-5.md` · Marcador: `.iniciado-f1-6-5`.
