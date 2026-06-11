# Proposta — Runtime-por-Espaço (switch VIVO do M8) · F1-4-4

> Autor: Terminal 79 (DEVELOPER, Espaço walking-skeleton) · 2026-06-11 · para revisão do ARQUITETO
> Contexto: decisão do fundador (2026-06-11): **Espaços de fundo VIVOS** (pendência #5 do épico 34 §VIII resolvida). Critério F1-4-4: troca A↔B **<1s com PIDs preservados**. Fundação já fiada em `ecda988` (`workspace_boot.rs`: boot pelo ponteiro global + switch_to carimbando foco — troca por reinício já funciona). Esta proposta é o passo seguinte: troca **em processo**, sem reinício.

## 1. Estado atual (singletons de UM Espaço — verificado no código)

O boot do `main.rs` (~3480-3770) monta, inline e uma vez: `EventStore` (`<ws_root>/.lina/events`), `Supervisor` (roster+bus), `NodeManager` (`bridge.rs:3915` — store+model+grids+keys+cwds+delta_tx+scrollback+bootstrap), `MailboxPump` sobre `<ws_root>/.lina`, `BrokerPump`/custódia, heartbeat, `FlushGuard` do scrollback, hooks listener (1× por processo, porta efêmera). O `WorkspaceView` renderiza de `self.nodes.model` e consome UM `delta_rx`.

## 2. Proposta

**Extrair o boot de workspace para um construtor reutilizável** e deixar o processo hospedar N runtimes:

```
struct WsRuntime {
    ws_root: PathBuf,
    store: Arc<Mutex<EventStore>>,
    sup: Arc<Supervisor>,
    nodes: Arc<NodeManager>,
    model: Model,
    grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    pumps: WsPumpHandles,        // mailbox/broker/heartbeat/flush — threads por-ws
}
fn boot_ws_runtime(ws_root: &Path, shared: &SharedInfra) -> Result<WsRuntime, String>
// SharedInfra = PtyManager global + hooks listener global + theme + profile registry
```

- O app guarda `BTreeMap<ws_id, WsRuntime>` + `active: ws_id`. O `WorkspaceView` passa a ler `active().model/grids/nodes`.
- **Switch** = (alvo ainda não montado? `boot_ws_runtime` + restore F1-4-3 daquele Espaço) → troca `active` → `cx.notify`. PIDs do Espaço que saiu de cena **não são tocados**: pumps/PTYs/threads continuam (decisão "vivos"); a tela só re-aponta.
- **Isolamento por construção** (critério F1-4-1 "bus escopado; roster/presença/A2A nunca cruzam"): `Supervisor`/`Router`/`MailboxPump` POR Espaço — não há tagging nem filtro; cruzar é impossível porque os barramentos são objetos distintos. `WorkspaceTrust` por pertencimento continua válido dentro de cada um.
- **Globais que permanecem globais:** `PtyManager` (processos do SO), hooks listener (token já identifica o nó), tema, registry de CLI profiles, ponteiro `~/.lina/workspaces.json`.

## 3. Pontos que preciso do teu veredito

1. **Multiplexação do `delta_rx`:** hoje a view consome um canal único. Opções: (a) um canal POR runtime e a view troca o receiver no switch — risco de perder deltas do fundo (aceitável? o grid re-pinta do estado ao voltar); (b) canal único compartilhado com `ws_id` no `GridDelta` — view descarta os do fundo. Proposta: **(a)**, mais simples; fundo não precisa pintar (culling já suspende fora da vista — F1-5-5 suspende ociosos sem parar drenagem).
2. **Token dos hooks por nome:** o listener registra token por NOME do nó (`bridge.rs:3244`). Dois Espaços podem ter "Terminal A" → colisão de atribuição na timeline. Proposta: registrar `"{ws_id}/{name}"` (mudança aditiva no register; o dashboard de cada ws filtra pelo prefixo). Alternativa: por NodeId.
3. **`LINA_HOME` é env do processo** (main.rs:3589) e os PTYs herdam no spawn — cada terminal nasce apontando para o `.lina` do SEU Espaço (correto), mas o env global do processo só aponta para um. Proposta: setar `LINA_HOME` POR spawn (no `node_identity_env`, junto do ADR 0026) e parar de depender do env global; spawns do Espaço B nascem com o `.lina` de B.
4. **Teto global de recursos:** alvo do produto = 8-12 terminais ATIVOS somando Espaços (decisão do fundador na spec M8 §1). O switch precisa consultar algo? Proposta: v1 não bloqueia nada — só o mini-status da sidebar mostra custo/agentes por Espaço (transparência primeiro, freio depois).
5. **Ordem das fatias:** (i) extração `boot_ws_runtime` sem mudança de comportamento (refactor puro, suíte intacta) → (ii) sidebar (rail esquerdo, lista do registry + mini-status honesto, troca chamando o switch novo) → (iii) M9 criar Espaço (galeria + `Workspace::create` + gating free=1 com stub Pro até F1-4-5). Cada uma commitada e verde isoladamente.

## 4. O que NÃO entra nesta fatia
- Licença real (F1-4-5/6/7 — fatia do time de licenças; uso o seam `can_create(tier)` com tier stub `Pro`, marcado como bloqueante de release na spec §6-B1).
- 🔔 por Espaço (gap §6-C1 — `AttentionItem.workspace_id` é costura EXTERNA via Maestro).
- `WorkspaceUnarchived` (costura quente em `events.rs` — §6-C2, pedirei quando a sidebar listar arquivados).

— fim —

---

## Veredito do ARQUITETO (Terminal #62, 2026-06-11) — PODE CODAR com ajustes

1. **delta_rx — AJUSTES (obrigatório na fatia i):** um canal POR runtime ✓, mas o fundo NÃO pode ficar sem dreno — o loop do delta_rx (bridge.rs:5100-5113) alimenta `meter.record_output` (teto de custo) e `watch.note_output` (Busy/Idle R2b), e o mpsc é unbounded (fundo sem consumo = memória sem teto + custo/idle cegos). Cada `WsRuntime` tem dreno próprio; no fundo roda em modo barato (consome, alimenta meter+watch, descarta pintura). A view lê só o grid do ativo; re-pintar do estado ao voltar.
2. **Token hooks `{ws_id}/{name}` — APROVADO** (aditivo; preserva o casamento por nome dentro do Espaço; NodeId quebraria esse contrato).
3. **LINA_HOME por spawn — APROVADO, com CONDIÇÃO:** auditar os leitores IN-PROCESS do env global (ex.: `events_dir()` do bin em main.rs:3473; `set_var` em main.rs:3641) — tudo que o APP lê de LINA_HOME migra para `ws_root` do runtime; só remover o `set_var` global quando o audit zerar.
4. **Teto global — APROVADO** (v1 transparência sem freio; o meter por-runtime do ponto 1 dá o número por Espaço de graça).
5. **Ordem das fatias — APROVADO**; a fatia (i) refactor-puro DEVE incluir o dreno por-runtime (1 runtime = comportamento idêntico ao atual).

Estrutura geral (WsRuntime + SharedInfra + isolamento por objeto distinto): **APROVADA**.
