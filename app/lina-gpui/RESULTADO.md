# Onda 2 — Shell gpui ⟶ Core (a ponte `UiHost`) · RESULTADO

> **TL;DR:** o shell gpui `lina-gpui` projeta **1 terminal real do CORE** (Supervisor +
> pty-host + 1 PTY `sh`) numa janela nativa, com o output vindo **pelo pipeline do core**
> (`GridDelta`), input do humano voltando pelo `InputSink`, e o banner de recuperação
> ligado a `Recovering`/`Recovered`. **Compila** (`cargo build`, 0 erros), **roda** (janela
> abre, sem panic), **clippy `--all-targets -D warnings` limpo**, **fmt limpo**, **3/3 testes**
> (incluindo o loop completo input→core→grid headless). Única mudança de core: promover
> `row_text` à trait `VtBackend` (o render precisa ler as linhas sujas do grid).

- **Crate:** `app/lina-gpui` — standalone, **excluído do workspace** (um shell experimental não derruba o core).
- **gpui:** mesmo SHA auditado do spike W1-1 (`09165c15…`) + `runtime_shaders` + `[profile.dev] debug=false`.
- **Sem commit.**

---

## 1. A ponte `UiHost`/`InputSink` — como funciona

O contrato `lina-host` é bidirecional e a UI é uma **projeção descartável** do estado do core.

### Saída (core → UI → render), 100% pelo pipeline do core
```
PTY (sh)
  → pty-host (lina-core W0-3): thread leitora drena o PTY, aplica ao grid
    alacritty_terminal (lina-vt) e emite  GridDelta { node, rows: dirty, bytes, seq }
  → thread-bomba da ponte (bridge::spawn_pump): drena o canal de GridDelta
  → UiHost::on_event(HostEvent::GridDelta { node, dirty_rows })   ← GpuiBridgeHost
        ↳ lê SÓ as linhas sujas do grid PARSEADO PELO CORE via PtyHost::with_grid(node, |vt| vt.row_text(r))
        ↳ escreve no SharedModel.nodes[node].rows   (SÓ dados — zero tipos gpui)
        ↳ ack(node, bytes)  → libera o flow-control (high/low watermark) do pty-host
  → WorkspaceView (gpui): lê o SharedModel a cada frame (request_animation_frame) e desenha
    o grid como texto monoespaçado num nó AccessKit  Role::Terminal.
```
**Invariante respeitada:** o shell **nunca** lê o PTY. `HostEvent::GridDelta` carrega só os
**índices** das linhas sujas (`dirty_rows`); o texto vem de `PtyHost::with_grid` — o grid que
**o core** parseou. É o contrato "push-the-damage / pull-the-content" do `lina-host`.

### Entrada (UI → core)
```
tecla no gpui  → WorkspaceView.on_key_down → keystroke_to_bytes (imprimíveis, Enter, Backspace,
                 Tab, Esc, setas, Ctrl+letra) → InputSink::submit(node, WriteOp::HumanKeys(bytes))
  → CoreInput (impl InputSink) → PtyHost::write(node, &bytes) no master do PTY.
```

### Lifecycle / status / recuperação
- `Supervisor::register` (NodeSpawned) + `set_status` (NodeStatus) publicam **eventos reais no bus**
  (`tokio::sync::broadcast`); a thread-bomba os traduz com `bus_to_host` → `HostEvent::NodeAdded` /
  `NodeStatusChanged` e projeta no `SharedModel` (cor do status no header).
- `EventStore::open_or_recover` emite `Recovering`→`Recovered` no mesmo `UiHost` em caso de corrupção;
  o `GpuiBridgeHost` liga/desliga o banner (provado pelo teste `recovery_pair_toggles_banner` + pelo
  gate da W0-6).

### A porta de troca gpui↔Slint fica ABERTA
`bridge.rs` (SharedModel, `GpuiBridgeHost`, `CoreInput`, `bus_to_host`, a thread-bomba) é **livre de
tipos de toolkit** — zero `gpui`. Um shell Slint reimplementa **só** `WorkspaceView` (+
`keystroke_to_bytes`), reusando a ponte inteira. (Âncora de continuidade do `CLAUDE.md`.)

---

## 2. A única mudança de core (necessária e mínima)

`crates/lina-vt/src/lib.rs` (+23/-15): **`fn row_text(&self, viewport_line) -> String` promovido de
método inerente privado do `AlacrittyBackend` para a trait `VtBackend`**.

Por quê é necessário: `PtyHost::with_grid` empresta `&dyn VtBackend`, e a trait só expunha
`last_nonempty_line()` (uma linha). Renderizar o **viewport inteiro** a partir de `GridDelta.dirty_rows`
exige ler linha-a-linha pela trait. A mudança é **backward-compatible** (o corpo já existia; só foi
movido para a trait + stub no `GhosttyBackend` cfg). Verificado: `cargo test -p lina-vt` (6/6) e
`cargo build -p lina-core` seguem verdes — o workspace não quebra.

Nenhum outro arquivo do core/workspace foi tocado.

---

## 3. Seams conhecidos (documentados, resolvidos na próxima story)

Composição **Supervisor + pty-host** num único terminal expôs dois seams reais do core (os dois
subsistemas da Onda 0 cunham `NodeId` independentes e ambos querem o writer do PTY):

1. **Id canônico = `term_id` do pty-host** (GridDelta/with_grid usam ele). O `Supervisor::register`
   cunha um `sup_id` próprio; a ponte **remapeia `sup_id → term_id`** (um par) em `bus_to_host`, para
   os eventos reais do bus pousarem no nó canônico (sem nó-fantasma).
2. **Input via `PtyHost::write`** (o pty-host é dono do writer do master). O `Supervisor` é registrado
   com `io::sink()` (só presença/status no bus). O caminho canônico — input pela **MailQueue serial**
   do Supervisor (arbitragem humano+agente) — pede um pty-host **read-only** (sem `take_writer`); é a
   refatoração da próxima story (canvas / 2 terminais / pulso A2A).

Fora isto, a ponte está completa para 1 terminal.

---

## 4. Compila? Roda?

- **Compila:** `cargo build` → `Finished dev [unoptimized] in 2m11s`, **0 erros** (gpui + renderer Metal
  via `runtime_shaders` + lina-core). Binário `target/debug/lina-gpui` (28.5 MB).
- **Clippy:** `cargo clippy --all-targets -- -D warnings` → **limpo** (0 warnings).
- **Fmt:** `cargo fmt` limpo.
- **Testes (3/3):**
  - `grid_roundtrip_through_core` — **o loop inteiro, sem display:** `InputSink` digita `lina-bridge-ok\r`
    → `PtyHost::write` → `cat` ecoa → reader do pty-host → grid lina-vt → `GridDelta` →
    `UiHost::on_event` → `with_grid`/`row_text` → o texto aparece no `SharedModel`. Prova que o output
    chega à projeção **pelo pipeline do core**.
  - `recovery_pair_toggles_banner` — `Recovering`/`Recovered` ligam/desligam o banner.
  - `bus_events_map_and_remap` — `BusEvent` reais → `HostEvent` com remap `sup_id→term_id`.
- **Roda:** `./target/debug/lina-gpui` abre 1 janela nativa mostrando o terminal real do core
  (`sh -i`), aceita digitação (Enter executa), **sem panic** (smoke de 3.5s via `LINA_AUTOQUIT_MS`).

---

## 5. Como rodar

```bash
cd app/lina-gpui
cargo build                 # ~2 min; precisa de ~1.3 GB livres (debug sem debuginfo)
./target/debug/lina-gpui    # abre a janela; digite comandos no terminal real do core
# verificação headless (auto-encerra): LINA_AUTOQUIT_MS=3500 ./target/debug/lina-gpui
cargo test                  # 3/3 (a ponte ponta-a-ponta, sem display)
```

`runtime_shaders` dispensa o Metal Toolchain offline (ver `spikes/spike-gpui/RESULTADO.md §1`).
Disco apertado: rode `cargo clean` depois para devolver ~1.2 GB.

---

## 6. Próxima story (não nesta)
Canvas multi-terminal + pan/zoom; 2º terminal; pulso A2A (BusMessage → ghost wire); unificar o writer
(pty-host read-only + MailQueue serial do Supervisor como caminho único de input); migrar o redraw de
`request_animation_frame` (poll a 120 Hz) para **event-driven** (`cx.spawn` + canal) para não repintar
um canvas ocioso.
