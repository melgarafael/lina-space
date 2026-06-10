# DESPACHO r1-licenca — Especialista em Licenças (terminal spawnado via `lina spawn`)
**id:** `f1-4-5-7` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Duas stories IRMÃS da onda F1-4, ambas em CRATES NOVOS (você é dono único; zero colisão com o resto do time).

## Fronteira de arquivos (sua, dono único)
- `crates/lina-license/**` (NOVO)
- `crates/lina-keygen/**` (NOVO — ferramenta CLI separada, NUNCA entra no bundle do app)
- `Cargo.toml` da raiz: SÓ adicionar os 2 membros ao workspace (1 linha cada)
- **NÃO toque:** todo o resto (lina-core, app, webhooks, bootstrap, ci.yml).

## Story A — F1-4-5 · Licença local ed25519 + gating por nº de workspaces (free=1, PRO=N)
**Fonte integral:** `tasks/epico-f1/ondas-2-4.md` linhas 238-258 (LEIA — os 8 critérios são o contrato). Resumo do design:
- Crate `lina-license` (core, SEM UI): verificação ed25519 com **chave pública embarcada no binário**; parse de entitlements data-driven `{tier, workspace_limit, entitlements, expiry, last_validated, signature}` lido de `~/.lina/license.json`.
- Ausência de arquivo ⇒ free (limit=1). **Falha de assinatura ⇒ free** (degradação graciosa; nunca trava o app; Espaços existentes além do limite continuam ABRINDO — o que bloqueia é CRIAR).
- Campo opcional `machine_id` NÃO validado nesta fase (porta para node-locking futuro). `expiry` suportado (chaves de aluno); licença perpetual sem expiry imune a relógio retrocedido; expiry só re-avaliado em PONTOS DE GATING (boot/criação de Espaço) — nunca rebaixa no meio da sessão.
- **Zero rede em runtime** (não adicione dependência que abra socket).
- Use uma dependência ed25519 consolidada e auditável (ex.: `ed25519-dalek` — confirme a licença compatível com o `deny.toml` e rode `cargo deny check licenses` na validação).
**Critérios-chave (todos por teste headless):** PRO válida permite 2º+ Espaço; vetor `workspace_limit=3` permite 3 e bloqueia o 4º; **adversarial**: editar tier/limit sem re-assinar → assinatura inválida → free; keypair de terceiro não valida; perpetual imune a relógio; expiry degrada gracioso; pré-existentes seguem abrindo.
**Integração com F1-4-1:** o Especialista em Dados está construindo o multi-workspace em paralelo (despacho `r1-dados.md`). NÃO chame o módulo dele — exponha uma API limpa (`LicenseState::workspace_limit()` etc.) e registre na entrega o ponto de consumo proposto (a fiação é da próxima rodada).

## Story B — F1-4-7 · Batch de chaves para alunos (emissão em lote, offline, sem master key)
**Fonte integral:** `ondas-2-4.md` linhas 279-295. `lina-keygen gen --count 50 --tier pro --label turma-7 [--expiry 12m]` → CSV (chave, tier, validade, rótulo); cada chave única e auto-contida (sem master key compartilhada). Doc de operação do fundador (onde a privada vive — keyring/mídia offline; como rotacionar): `crates/lina-keygen/OPERACAO.md`, copy leiga.
**Critérios:** 50 chaves distintas, 2 execuções não repetem; 3 chaves amostradas validam no `lina-license` (teste E2E offline); chave adulterada 1 byte falha; **a privada não existe no repo nem no bundle** (teste/auditoria grep + regra de exclusão).

## Decisões do Maestro (contrato)
- free = **1** workspace. PRO default = ilimitado no copy, mas o MECANISMO é data-driven (qualquer N).
- Pricing (perpetual × subscription) é decisão de NEGÓCIO pendente do fundador — o mecanismo é agnóstico; não escreva copy de preço.

## Entrega
`tasks/epico-f1/.entrega-f1-4-5-7.md` (modelo nas regras comuns). Marcador: `.iniciado-f1-4-5-7`. Validação por-pacote nos 2 crates + `cargo deny check licenses` (exit direto).
