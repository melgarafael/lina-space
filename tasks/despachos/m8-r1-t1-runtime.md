# Despacho M8-T1 — Fatia (i): extrair `boot_ws_runtime` (refactor PURO)

> Rodada M8-r1 · 2026-06-11 · Maestro externo → Terminal J (DEVELOPER)
> Marque início: `echo INICIADO > "/Users/rafaelmelgaco/einstein workspace/lina-space/.iniciado-m8-t1"`

## CONTEXTO
Decisão do fundador: Espaços de fundo VIVOS; critério F1-4-4 = troca A↔B <1s com PIDs preservados.
A proposta runtime-por-Espaço foi **aprovada pelo Arquiteto com ajuste obrigatório** — leia INTEIRO:
`tasks/epico-f1/proposta-runtime-por-espaco.md` (proposta §2 + veredito no rodapé).
Mapas prontos do código (linhas exatas, NÃO re-derive): `/tmp/mapas-m8/boot-main.md` (sequência
[1]-[18] do boot), `/tmp/mapas-m8/bridge-runtime.md` (NodeManager/spawn_pump/hooks),
`/tmp/mapas-m8/roteiro-story.md` (21 testes que NÃO podem quebrar). HEAD atual: `a3cb75e`.

## FUNÇÃO
Dev sênior Rust fazendo um refactor cirúrgico de extração — ZERO mudança de comportamento.

## DIRECIONAMENTO
1. `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"`.
2. Crie `app/lina-gpui/src/runtime.rs` com:
   - `pub struct SharedInfra` — o que é POR-PROCESSO: `pty: Arc<Mutex<PtyManager>>`,
     `cmd_factory: CmdFactory`, `cols/rows: u16`. (Tema é global mas aplicado no main; deixe fora.)
   - `pub struct WsRuntime` — TUDO que é por-Espaço e que a view/switch precisa re-apontar:
     `ws_root`, `mailbox_dir`, `store`, `sup`, `nodes: Arc<NodeManager>`, `model`, `grids`,
     `input: Arc<CoreInput>`, `attention: Arc<AttentionHub>`, `desk`, `brake`, `autonomy`,
     `injection_profile`, `profile_registry`, `hooks` (ver item 4), `bus`/pump handles
     (`_pump`, `_mailbox_pump`, `_broker_pump` — handles VIVOS dentro do struct, nunca dropados).
   - `pub fn boot_ws_runtime(ws_root: PathBuf, shared: &SharedInfra, demo: bool) -> Result<WsRuntime, String>`
     — mova para cá os passos [2]-[17] do mapa `boot-main.md` (3553→4096 do main.rs), EXCETO:
     resolução de ws_root/registry-pick (fica no main), tema (main), demo-seed/load-gen/attention-demo
     (main, operam sobre o runtime retornado), criação da janela gpui/WorkspaceView (main).
3. **Dreno por-runtime (ajuste OBRIGATÓRIO do veredito §1):** o `spawn_pump` é o dreno — ele JÁ
   consome `delta_rx`+`bus_rx` e alimenta `meter.record_output`/`watch.note_output`. Cada
   `WsRuntime` carrega o SEU pump vivo. NADA de pump global.
4. **Hooks listener (DESVIO PROPOSTO do veredito §2 — registre na entrega):** em vez de listener
   global + token `{ws_id}/{name}`, mova `HooksShared::start()` para DENTRO do `boot_ws_runtime`
   (1 listener POR runtime, porta efêmera própria; kits do Espaço apontam pra porta do SEU runtime).
   Racional: isolamento por OBJETO DISTINTO (o princípio da própria proposta §2) > tagging por
   prefixo; colisão de nome entre Espaços fica impossível por construção. O Arquiteto valida na
   revisão da entrega — se recusar, voltamos ao prefixo.
5. **LINA_HOME:** mantenha o `std::env::set_var` global NESTA fatia (1 runtime = comportamento
   idêntico). Anote na entrega a costura futura (per-spawn via `node_identity_env`, veredito §3).
6. `main()` passa a: resolver ws_root → montar `SharedInfra` → `let rt = boot_ws_runtime(...)` →
   demo/load-gen sobre `rt` → construir a view com os handles de `rt`. A assinatura do
   `WorkspaceView` NÃO muda nesta fatia.
7. Sem `unwrap()` em produção; preserve TODOS os comentários/eprintlns existentes (são doutrina).

## FRONTEIRA DE ARQUIVOS (exclusiva sua nesta rodada)
- `app/lina-gpui/src/main.rs` e `app/lina-gpui/src/runtime.rs` (novo). Mais NADA.
- NÃO toque: bridge.rs, dashboard.rs, workspace_boot.rs, gallery.rs, persistence_ui.rs (colegas
  têm fatias lá AGORA). Precisa de algo neles? `BLOCKED:` na entrega + me avise.
- **NÃO COMMITE.** NÃO rode git checkout/reset/stash. O Maestro valida de fora e commita.

## OBJETIVO
`boot_ws_runtime` existe, o main usa, e NADA muda de comportamento: app compila, a suíte INTEIRA
do app passa (349+), clippy `-D warnings` e `cargo fmt --check` EXIT=0 (redirecione para arquivo e
leia o exit DIRETO; os 21 testes nomeados no roteiro-story.md intactos).

## RESULTADO ESPERADO
Escreva `.entrega-m8-t1.md` na raiz do repo com: assinaturas finais de WsRuntime/SharedInfra/
boot_ws_runtime; o que moveu e o que ficou no main; o desvio do item 4 com racional; costuras
nomeadas para a fatia (ii) (esboço de `switch_runtime`); evidência (tail dos exits de test/clippy/
fmt). Termine com `PRONTO:` ou `BLOCKED: <motivo>`.
