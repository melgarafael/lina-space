# Onda 2 — Walking Skeleton (o gate da Onda 2) · RESULTADO

> **TL;DR:** o primeiro Lina que se vê e se usa. Uma janela gpui é um **canvas** com **2
> terminais reais do core** (cards posicionáveis, **pan** arrastando o fundo), e o
> diferencial #1 — a metáfora **"sem fios"**: um clique em **⚡ Enviar A2A (A→B)** dispara
> `deliver_a2a` de A para B; ao escutar `BusEvent::Message` do **pub/sub do core**, a tela
> mostra um **pulso efêmero A→B** (pacote azul que viaja e some em ~1 s) + o selo **"Time
> conectado · todos se falam"**; e o **texto A2A chega no grid do terminal B** (round-trip
> faseado real). Os eventos são persistidos no **EventStore** (sobrevive a reinícios). A
> janela fica **aberta** até o usuário fechar. **Build passa; clippy `-D warnings` + fmt
> limpos; 2/2 testes-gate headless.**

- **Crate:** `app/lina-gpui` (standalone, excluído do workspace). gpui pinado (`09165c15…`) + `runtime_shaders` + `debug=false`.
- **Reúso do core (nada reimplementado):** `Supervisor`, `deliver_a2a`, `EventStore`, o pub/sub do `Bus`, `GridSense`, `VtBackend`, `A2aEnvelope`. **Sem commit.**

---

## 1. O que aparece na tela (e como funciona)

```
┌───────────────────────────────────────────────────────────────── (aura pontilhada "sem fios") ┐
│ Lina Space · walking skeleton   ● Time conectado · todos se falam   log: 11 eventos  [⚡ A2A A→B]│
│                                                                                                 │
│   ┌── Terminal A ──────────────┐                 ╲ pulso ╱        ┌── Terminal B ──────────────┐ │
│   │ ● Terminal · Running       │            (pacote azul A→B,     │ ● Terminal · Running       │ │
│   │ Terminal A — seu shell.    │             some em ~1s)         │ Terminal B — recebe A2A.   │ │
│   │ $ ls                       │  ● ───────────────────────▶      │ 📨 A2A de A→B · time…      │ │
│   │ ...                        │                                  │ ...                        │ │
│   └────────────────────────────┘                                  └────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

1. **Canvas + 2 cards posicionáveis + pan.** Fundo OLED `#0A0E27`; cada terminal é um card
   (título com nome + ● status + tipo) posicionado em `(x,y)`. **Arrastar o fundo** faz pan
   (`on_mouse_down/move/up`, `MouseMoveEvent::dragging()`), movendo os cards juntos. Clicar um
   card o **foca** (borda azul) — o teclado vai para o terminal focado.
2. **2 terminais REAIS do core.** `PtyManager` abre 2 PTYs (`sh -i` no A, `cat` no B);
   `take_writer → sup.register` (o **Supervisor é dono dos writers**), `clone_reader →` thread
   leitora que avança o grid `alacritty_terminal` e **emite `GridDelta`** → `UiHost` → render.
3. **A2A com PULSO visível (a metáfora "sem fios").** O botão dispara, numa thread:
   `sup.route(&env)` → publica **`BusEvent::Message`** no pub/sub; a thread-bomba o escuta,
   traduz para `HostEvent::BusMessage`, e a ponte acende o **pulso** (`Pulse{from,to,started}`)
   + a aura **"Time conectado"**. Em paralelo, **`deliver_a2a`** injeta o texto **faseado**
   (bracketed-paste → `submit_delay` → Enter separado) na MailQueue serial do Supervisor → o
   PTY de B → `cat` ecoa → grid de B → `GridDelta` → **o texto aparece no card de B**.
4. **Persistência.** `EventStore::open` num dir estável; grava `WorkspaceCreated` +
   `NodeAdded×2` + `TerminalSpawned×2` no boot e `BusMessageSent` a cada A2A (SQLite WAL +
   JSONL + snapshots). O contador "log: N eventos" sobe na tela e **sobrevive a reinícios**
   (provado: 6 eventos → relaunch → 11).
5. **Janela aberta** até o usuário fechar (sem auto-quit; `LINA_AUTOQUIT_MS` existe só para o smoke headless).

---

## 2. A ponte `UiHost` (core ⇄ shell), livre de gpui

`bridge.rs` não importa **nenhum** tipo de toolkit (porta de troca gpui↔Slint aberta):
- **`GpuiBridgeHost: UiHost`** — projeta cada `HostEvent` no `SharedModel` (só dados):
  `GridDelta{dirty_rows}` → lê **só** as linhas sujas do grid parseado pelo core (`row_text`);
  `BusMessage` → acende o **pulso** + a aura; `NodeStatusChanged` → cor do status;
  `Recovering/Recovered` → banner.
- **`CoreInput: InputSink`** — agora o Supervisor é dono do writer, então o input do humano vai
  pela **MailQueue serial** (`sup.write_human`) — o caminho canônico (o seam de "io::sink" da
  story 1 sumiu).
- **`A2aTrigger`** — reúne `deliver_a2a` + `EventStore` + o `Grid` de B (como `GridSense`).
- **`wire_terminal`** — o padrão do `gate_onda0`: spawn → writer ao Supervisor → reader thread
  que emite `GridDelta`.
- **`spawn_pump`** — a thread-bomba que drena o Bus (`subscribe`) + os `GridDelta` e chama
  `UiHost::on_event`. Um shell Slint reimplementa só a `WorkspaceView`, reusando tudo isto.

---

## 3. Core: zero mudanças nesta story

O walking skeleton **não tocou no core** — reusa `deliver_a2a`/`Supervisor`/`EventStore`/`Bus`/
`GridSense` como estão. A única mudança de core do épico foi a da story 1 (promover `row_text`
à trait `VtBackend`, backward-compatible). Os 57 testes do workspace seguem verdes.

`A2aTrigger`/`CoreInput`/`GpuiBridgeHost` são `Send`/`Send+Sync` porque `Supervisor: Send+Sync`,
`EventStore: Send` (atrás de `Arc<Mutex<…>>`) e `Grid = Arc<Mutex<Box<dyn VtBackend>>>` (que já
implementa `GridSense`).

---

## 4. Compila? Roda? (gate observável)

- **Compila:** `cargo build` → `Finished`, **0 erros**. Binário `target/debug/lina-gpui` (~32 MB).
- **Clippy:** `cargo clippy --all-targets -- -D warnings` → **limpo**.
- **Fmt:** `cargo fmt` limpo.
- **Testes-gate headless (2/2):**
  - `a2a_roundtrip_pulse_and_persist` — **o gate, sem display:** 2 terminais reais; `sup.route`
    publica `BusEvent::Message` → o **pulso liga + a aura**; `deliver_a2a` injeta → o texto
    `LINA_A2A_MARKER` **chega no grid de B**; e um evento é **persistido** (`event_count` sobe).
  - `recovery_pair_toggles_banner`.
- **Roda (smoke):** a janela abre com os 2 terminais, o A2A dispara (pulso + texto em B), e o
  EventStore persiste — `bus.jsonl` com `WorkspaceCreated`/`NodeAdded×2`/`TerminalSpawned×2`/
  **`BusMessageSent`**; relaunch → contador 6 → 11 (**estado sobrevive**); **sem panic**.

---

## 5. Como rodar
```bash
cd app/lina-gpui
cargo build                 # incremental se as deps gpui já estão cacheadas
./target/debug/lina-gpui    # abre o canvas; clique "⚡ Enviar A2A (A→B)" e veja o pulso + B receber
cargo test                  # 2/2 gate headless
```
Mexa no card A (clique p/ focar) e digite — é um shell real. O log persiste em
`$TMPDIR/lina-space-ws2` (sobrevive a reinícios). `runtime_shaders` dispensa o Metal Toolchain.

## 6. Próxima story (não nesta)
Zoom (scroll-to-zoom via `on_scroll_wheel`); arrastar cards individualmente (persistir `NodeMoved`);
N>2 terminais + presets; A2A dirigido por papel (`Recipient::Role`) com aura por papel; recuperação
visível ligada ao boot real do EventStore.
