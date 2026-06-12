# F2-0-6 — Synchronized output (DEC 2026): resultado + spec do diff do reader

**Terminal D (QA·ultracode) · rodada r4 · fronteira: `crates/lina-vt` (+ testes).**
Status do código em `lina-vt`: **COMPLETO e verde** (TDD). O caminho de apresentação do app
(`lina-core`) **não foi editado** — abaixo a spec exata do diff para o dono da costura aplicar.

---

## 1. Investigação (ANTES de implementar) — o suporte já existe, numa camada inesperada

O nosso pin é `alacritty_terminal = 0.26.0`, que puxa `vte = 0.15.0` (Cargo.lock confirmado).

- **O `Term` do alacritty trata `2026` como NO-OP.** `set_private_mode`/`unset_private_mode`
  para `NamedPrivateMode::SyncUpdate` são `=> ()` (`term/mod.rs:1992` e `:2041`), e
  `report_private_mode` devolve `ModeState::Reset` (`:2084`). Quem olhasse só o `Term` concluiria
  "não suporta".
- **Quem implementa o synchronized output é o parser `vte` (`Processor`) — o MESMO que o
  `lina-vt` já usa** (`alacritty_terminal::vte::ansi::Processor`). `vte/src/ansi.rs`:
  - `BSU_CSI = b"\x1b[?2026h"`, `ESU_CSI = b"\x1b[?2026l"` (`ansi.rs:45/48`).
  - Ao ver o BSU em parsing normal, arma o timeout e **interrompe** o parse (`ansi.rs:1606-1608`).
  - A partir daí, `Processor::advance` roteia para `advance_sync`, que **bufferiza** os bytes sem
    tocar no `Term` (`ansi.rs:304-305`, `:370-387`). O grid **não muda** no meio do batch.
  - No ESU (detectado em `advance_sync_csi`, `ansi.rs:413-415`), `stop_sync` processa o buffer
    inteiro de uma vez → grid atualiza **atômico**, zera o sync (`ansi.rs:327-357`).

**Conclusão:** é mais forte que "segurar a apresentação" — o *grid em si* não muda entre BSU e ESU.
Logo a story vira **(a) PROVAR o hold + (b) garantir que nada no caminho de leitura fura o hold, e
costurar o teto de tempo**.

## 2. Dois tetos anti-congelamento (defesa em profundidade)

| Teto | Valor | Onde | Precisa de driver? |
|---|---|---|---|
| **Byte-cap** | 2 MiB (`SYNC_BUFFER_SIZE`, `ansi.rs:39`) | intrínseco ao `advance`/`advance_sync` (`ansi.rs:375-381`) | **Não** — sempre ativo. Flood com ESU esquecido auto-libera. |
| **Time-cap** | ~150 ms (`SYNC_UPDATE_TIMEOUT`, `ansi.rs:36`) | gravado em `StdSyncHandler` no BSU | **Sim** — o consumidor precisa observar o deadline e chamar o flush. |

O byte-cap já cobre o caso **sync + output contínuo** (a CLI mandou `2026h`, esqueceu o `l`, mas
segue cuspindo bytes → estoura 2 MiB → libera). O **único** caso descoberto sem o time-cap é
**sync + silêncio total** (mandou `2026h` e parou): aí o grid fica preso no frame anterior até o
próximo byte. O time-cap fecha isso em ≤150 ms.

## 3. O que entrou no `lina-vt` (fronteira desta story) — ADITIVO, atrás da trait

Dois métodos novos na trait `VtBackend`, com **default** (a porta `libghostty-vt` herda o contrato
sem quebrar; `GhosttyBackend` não precisa implementá-los):

```rust
/// Some(instant) = há batch sync RETIDO que deve ser apresentado até esse instante (teto). None = sem sync.
fn sync_deadline(&self) -> Option<Instant> { None }

/// Força o fim do sync pendente, apresentando atômico o batch retido. true = havia sync (grid pode ter
/// mudado → colete damaged_rows e emita delta). false = no-op idempotente.
fn flush_sync(&mut self) -> bool { false }
```

Impl no `AlacrittyBackend`:
- `sync_deadline` → `self.parser.sync_timeout().sync_timeout()` (expõe o deadline do `vte`).
- `flush_sync` → se há deadline, mede `history_size` (quando `capture`), chama
  `self.parser.stop_sync(&mut self.term)`, **colhe o scrollback que o batch rolou** via
  `harvest_and_trim` (senão o teto seria perda SILENCIOSA do cabo `append-on-scroll`) e dobra o
  dano via `accumulate_damage`.
- Refactor de higiene: extraí `harvest_and_trim`/`accumulate_damage` (antes inline em
  `advance_capturing`/`advance`) — **byte-idêntico** (os 35 testes unitários + o cabo cross-crate
  `scrollback_cable_w52` seguem verdes).

**Testes (`crates/lina-vt/tests/synchronized_output.rs`, API pública = contrato fim-a-fim):**
1. `synchronized_output_holds_intermediate_frames_until_esu` — o HOLD (critério a).
2. `sync_deadline_is_published_while_held_and_cleared_on_esu` — deadline ≤150 ms publicado/zerado.
3. `flush_sync_presents_held_batch_when_esu_is_forgotten` — o teto de tempo dirigido + idempotência.
4. `runaway_sync_auto_flushes_at_intrinsic_byte_cap` — o byte-cap de 2 MiB, sem driver.
5. `nested_bsu_does_not_leak_intermediate_frame` — BSU aninhado não vaza parcial (modelo plano do vte).
6. `esu_then_new_bsu_in_one_chunk_closes_first_holds_second` — o caminho `…ESU…BSU…` (2 batches atômicos).
7. `damage_during_hold_is_only_the_benign_cursor_row` — dano no hold = só a linha do cursor (benigno).

Validação (exits diretos): `cargo test -p lina-vt` = **43/0** (35 unit + 7 sync + 1 perf ignored),
`cargo clippy -p lina-vt --all-targets -D warnings` limpo, `cargo fmt -p lina-vt --check` limpo,
`cargo check --workspace` ok, `cargo test -p lina-core --test scrollback_cable_w52` = **1/0**.

### 3a. Verificação adversarial (ultracode — 4 verificadores céticos independentes)

Rodei um passo de refutação adversarial (4 lentes lendo a fonte vte/alacritty + o diff). 3 achados,
todos resolvidos por EVIDÊNCIA, nenhum exigiu mudar o código de produção do slice:

- **"Vazamento por BSU aninhado" (alegado CRÍTICO) → FALSO POSITIVO, refutado por teste.** O modelo
  de sync do vte é PLANO: um 2º `2026h` só re-arma o timeout; o `bsu_offset` de `stop_sync_internal`
  só atua em `…ESU…BSU…` no mesmo chunk (fecha um sync, abre outro — correto). O teste 5 prova que o
  grid fica retido através do aninhamento e aplica atômico só no ESU. O "middlenew" que o verificador
  leu como vazamento é o estado FINAL pós-ESU, não um parcial. (Teste 6 fixa o limite real do
  `bsu_offset`.) Nenhum "contador de profundidade" é necessário.
- **"`damaged_rows` não-vazio no hold" (MÉDIO) → REAL mas BENIGNO.** Corrigido: ver a ressalva no §4.
  Vira o teste 7 + redação precisa. Não é frame rasgado.
- **"Time-cap órfão em produção" (ALTO) → CORRETO e ESPERADO.** É exatamente o diff do reader
  especificado no §4 (fronteira do slice = `lina-vt`; a costura é "especifique, não edite"). O
  mecanismo está entregue (exposto + testado); ativá-lo em produção é o diff 4a/4b. Sem ele, só o
  byte-cap de 2 MiB protege — por isso o §4 marca a fiação como NECESSÁRIA, não opcional.

---

## 4. SPEC do diff do caminho de apresentação (NÃO editei — dono: costura C/Maestro)

**O HOLD já passa intacto pelo `flush`/`reader_loop` sem nenhuma mudança.** Durante um sync retido,
`vt.advance(batch)` bufferiza o redraw — **o conteúdo do grid não muda**. Quando o ESU chega num
batch posterior, `advance` libera e reporta o dano numa única passada. **Nenhuma edição é necessária
para o hold.**

> **Ressalva de precisão (achado da verificação adversarial):** `vt.damaged_rows()` durante o hold
> NÃO é necessariamente vazio — o `Term` do alacritty SEMPRE marca a linha do cursor em `damage()`
> (`term/mod.rs:480`), então um `advance` que abre o sync pode reportar `[cursor_row]`. Isso é
> **benigno**: repintar a linha do cursor re-renderiza o conteúdo INALTERADO (idempotente) — nenhum
> frame rasgado vaza. A integridade visual do hold vem do **conteúdo do grid não mudar** (provado
> por teste), não de `damaged_rows` ser vazio. Guarda de regressão:
> `damage_during_hold_is_only_the_benign_cursor_row` em `tests/synchronized_output.rs`. Otimização
> opcional (dívida, não-bloqueante): suprimir o dano espúrio do cursor quando o `advance` não
> aplicou bytes ao grid — economiza um repaint por batch retido, sem mudar a corretude.

**A ÚNICA costura necessária é o teto de tempo (≤150 ms).** Hoje o `reader_loop`
(`crates/lina-core/src/lib.rs:696`) faz `reader.read(&mut buf)` **bloqueante**: se a CLI manda
`2026h` e fica em silêncio, o read trava e o batch retido nunca apresenta. Proposta:

### 4a. Helper novo em `lina-core` (espelha `flush`, para o caminho do teto)

```rust
/// F2-0-6: serviço do teto de tempo do synchronized output. Se há um batch retido cujo deadline já
/// venceu, libera-o (apresenta atômico) e emite o GridDelta. Devolve o PRÓXIMO deadline a observar
/// (Some = ainda há sync pendente; None = nenhum), para o reader agendar o próximo wake.
fn service_sync_ceiling(
    shared: &TermShared,
    delta_tx: &Sender<GridDelta>,
    node: NodeId,
    seq: &AtomicU64,
) -> Option<Instant> {
    let (next_deadline, flushed, rows, scrolled) = {
        let mut vt = lock(&shared.vt);
        match vt.sync_deadline() {
            None => return None,                              // sem sync
            Some(dl) if Instant::now() < dl => return Some(dl), // ainda não venceu
            Some(_) => {
                let flushed = vt.flush_sync();
                let rows = vt.damaged_rows();
                vt.reset_damage();
                let scrolled = vt.take_scrollback();
                (vt.sync_deadline(), flushed, rows, scrolled)  // next_deadline normalmente None
            }
        }
    };
    // Persiste o scrollback colhido FORA do lock do vt (idêntico ao bloco de `flush`).
    if let Some(sink) = &shared.scrollback {
        if !scrolled.is_empty() {
            let mut store = lock(&sink.store);
            for line in scrolled {
                if let Err(e) = store.push_line(&sink.panel, line) {
                    tracing::warn!(panel = %sink.panel, error = %e,
                        "scrollback push (teto sync) falhou; linha fica no cache, re-tenta");
                }
            }
        }
    }
    if flushed {
        let s = seq.fetch_add(1, Ordering::Relaxed);
        let _ = delta_tx.send(GridDelta { node, rows, bytes: 0, seq: s });
    }
    next_deadline
}
```

### 4b. Integração no `reader_loop` — limitar o read pelo deadline pendente

O `reader` é `Box<dyn Read + Send>` do portable-pty (`lina-pty`), **bloqueante e sem timeout
exposto**. Para acordar no deadline há duas opções; **recomendo a 1**:

- **(Recomendado) Poll do fd do master com timeout.** Plumbar o `RawFd` do master (Unix:
  `OwnedFd`/`as_raw_fd`; Windows: o handle do ConPTY) até o `reader_loop` e, **quando
  `vt.sync_deadline()` for `Some(dl)`**, fazer `poll()/select()` por legibilidade com timeout
  `dl.saturating_duration_since(Instant::now())`. Se o poll **expira** sem bytes, chamar
  `service_sync_ceiling(...)` e voltar ao topo do loop. Se há bytes, segue o `read` normal.
  Custo: ~1 chamada extra a `lina-pty` para expor o fd; zero busy-poll.

- **(Fallback simples) Read curto + cadência.** Manter o read bloqueante mas, enquanto
  `sync_deadline()` for `Some`, dormir em fatias curtas (ex.: `min(deadline-now, 10ms)`) checando o
  deadline a cada wake. Mais simples, mas faz polling ativo enquanto um sync está aberto (raro e
  curto, então tolerável no MVP). **Não recomendado** para regime estável.

Esboço da integração (opção recomendada), no topo do `loop` do `reader_loop`, antes do `read`:

```rust
// F2-0-6: se há um batch sync retido, o read não pode bloquear além do teto — acorda no deadline.
if let Some(dl) = lock(&shared.vt).sync_deadline() {
    let timeout = dl.saturating_duration_since(Instant::now());
    if !pty_fd_readable_within(master_fd, timeout)? {     // poll/select com timeout
        service_sync_ceiling(shared, delta_tx, node, seq);
        continue;                                         // re-avalia (sync zerado → read normal)
    }
}
match reader.read(&mut buf) { /* ...inalterado... */ }
```

> **Importante:** o `bytes: 0` no `GridDelta` do teto é intencional (o batch já foi contado no
> `inflight` quando entrou via `advance`; o teto não lê bytes novos do PTY). Se o contador de
> `inflight`/flow-control precisar de ajuste, é decisão do dono da costura — sinalizo, não decido.

### 4c. Sobre o `bridge.rs` da UI (gpui)

O `app/lina-gpui/src/bridge.rs` (citado no despacho) **consome** `GridDelta` e repinta as
`dirty_rows`. Com a costura 4a/4b, o teto emite um `GridDelta` normal → o bridge **não precisa de
mudança**: ele já repinta linhas sujas vindas do delta. O hold e o teto são transparentes para a UI.

---

## 5. Âncora de continuidade

`sync_deadline`/`flush_sync` estão na trait `VtBackend` em termos do CONTRATO (synchronized output),
não de internals do alacritty. A porta `libghostty-vt` futura implementa os mesmos dois métodos
(default = "sem sync, sem deadline") e herda o teste de integração como contrato. **Não soldei o
synchronized output ao alacritty.**
