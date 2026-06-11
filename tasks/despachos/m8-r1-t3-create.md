# Despacho M8-T3 — Criação de Espaço core (workspace_boot.rs)

> Rodada M8-r1 · 2026-06-11 · Maestro externo → Terminal #36 (BUG_FIXER atuando como DEV — papel
> de implementação core; falta DEVELOPER livre, o J está na T1)
> Marque início: `echo INICIADO > "/Users/rafaelmelgaco/einstein workspace/lina-space/.iniciado-m8-t3"`

## CONTEXTO
O M9 (modal Criar Espaço) precisa de um seam core ÚNICO que crie um Espaço novo de verdade:
pasta + store + eventos canônicos + registro no ponteiro global + gating free=1 ANTES do esforço.
Spec congelada: `tasks/epico-f1/spec-m8-m9-fiacao.md` §2 (composição/validação de diretório,
strings de erro leigas) e §3 (gating — o seam `can_create` JÁ existe no core). Mapas:
`/tmp/mapas-m8/persistence-switch.md` (registry/license/resolve_spawn_cwd com linhas exatas) e
`/tmp/mapas-m8/gallery-m9.md` (presets/apply_preset/strings congeladas). HEAD: `a3cb75e`.

## FUNÇÃO
Dev core: lógica de criação robusta, validação cedo, eventos como fonte da verdade.

## DIRECIONAMENTO
1. `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"`.
2. Em `app/lina-gpui/src/workspace_boot.rs`, crie:
   - `pub fn validate_workdir(path: &Path) -> Result<PathBuf, WorkdirError>` — canonicaliza;
     se existe, precisa ser diretório E passar teste REAL de escrita (criar+remover arquivo
     temporário); se não existe, NÃO é erro (o caminho feliz cria). `WorkdirError` com as
     variantes mapeadas para as 3 strings leigas da spec §2 (não acessível / sem permissão /
     falha ao criar).
   - `pub fn create_workspace(parent: &Path, name: &str, preset: &gallery::FocusPreset,
     default_cwd: Option<&Path>, registry: &mut WorkspaceRegistry, now_ms: u64,
     tier: LicenseTier) -> Result<PathBuf, String>`:
     (a) **gating PRIMEIRO** (`can_create_workspace(tier, registry.active_count())` — limite
     aparece ANTES do esforço, nenhuma pasta criada em caso Blocked);
     (b) ws_root = `parent/<slug do nome>`; colisão de pasta/nome → sufixo « (2)» (padrão M6);
     (c) cria a pasta, abre `EventStore` em `<ws_root>/.lina/events`, apende a sequência:
     `WorkspaceCreated{name, focus_preset}` → `apply_preset(...)` (time nasce — mesmo contrato
     W4-5) → `WorkspaceDefaultCwdSet{cwd}` quando houver;
     (d) registra no `WorkspaceRegistry` (reusando `register_boot_workspace` ou equivalente —
     NÃO duplique a lógica de id) e devolve o ws_root. O FOCO (`WorkspaceFocusSet`/carimbo) é do
     CALLER (o switch da fatia ii) — não carimbe aqui.
3. TDD: criação feliz (eventos na ordem, registry ganhou a entrada, replay reconstrói nome/
   preset/default_cwd), Blocked no Free com 1 ativo (NENHUMA pasta criada), colisão de nome
   (sufixo), validate_workdir nos 4 casos (ok-existe, ok-não-existe, é-arquivo, sem-permissão
   via chmod 000 em temp dir).

## FRONTEIRA DE ARQUIVOS (exclusiva sua nesta rodada)
- `app/lina-gpui/src/workspace_boot.rs`. Mais NADA. (`gallery.rs` você IMPORTA, não edita;
  main.rs/runtime.rs são do Terminal J; dashboard.rs é do #40.)
- **NÃO COMMITE.** O Maestro valida de fora e commita.

## OBJETIVO
Seam de criação completo e testado headless: suíte do app verde, clippy `-D warnings` e fmt
EXIT=0 (exit direto). Os 5 testes existentes de workspace_boot.rs intactos.

## RESULTADO ESPERADO
`.entrega-m8-t3.md` na raiz: assinaturas finais, sequência de eventos apendada, decisões (slug/
sufixo), evidência dos exits + lista de testes novos. Termine com `PRONTO:` ou `BLOCKED: <motivo>`.
