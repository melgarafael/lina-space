# ADR 0024 — Atomicidade do write de aprovação no app (writer compartilhado + deliver_approval_with_grid)

**Status:** Aceito (Maestro, 2026-06-08). Refina o ADR 0021 (modelo de segurança da aprovação y/n) no nível de **implementação da atomicidade** no app. Decide o seam adiado em F1-1-8 #1.

## Contexto

O ADR 0021 §1 exige que a aprovação de permissão faça **check_screen + write no MESMO turno, sem `advance` do reader intercalado** (senão reabre a race CVE-2024-32477). O núcleo (`approval.rs` executor/ledger/check_screen) e a porta atômica `PtyHost::deliver_approval` (lina-core/lib.rs:510 — co-loca VT+writer no `Terminal`, segura o lock do VT durante check+write) já existem e estão provados (gate_f1_1_8 AC-0021.7).

**O blocker (F1-1-8 #1):** o APP não usa `PtyHost`. Usa `Supervisor` + `PtyManager`, onde:
- o VT/grid de cada terminal é um `Arc<Mutex<Box<dyn VtBackend>>>` no app (bridge.rs `wire_terminal` ~1660); o reader-loop trava esse grid antes de `advance` (bridge.rs ~1672);
- o **writer está movido** para dentro da thread `serial_writer` (lib.rs:1068), inacessível para um write síncrono; `write_human`/`enqueue_write` são assíncronos (enfileiram na channel);
- o `PtyLock` (lib.rs:1367) é um lock **lógico** ("quem escreve"), não o lock do grid.

VT e writer estão em camadas/estruturas desacopladas → não há, hoje, escrita atômica-com-o-grid no caminho do app.

## Opções consideradas

- **A — migrar os terminais do app para `lina_core::PtyHost`.** Convergiria na abstração do core (alinha invariante #7) e reusaria o `deliver_approval` atômico pronto. **Rejeitada:** blast-radius enorme — `wire_terminal`, roteamento A2A, broker/custódia, MailboxPump, a re-injeção F1-2-4 (`enqueue_write`/`lock_pty`), FPS/culling e o tratamento de `GridDelta` estão pendurados no Supervisor/PtyManager; migrar arrisca regredir o terminal I/O de um app já validado na tela.
- **B — `Supervisor::deliver_approval` síncrono sem o grid.** **Rejeitada:** o Supervisor não tem o grid (vive no app); fazer `check_screen` exigiria o Supervisor importar tipos de `lina-vt` → viola o core/shell split (invariante #7).
- **C.2 — writer compartilhado + `deliver_approval_with_grid(node, grid, …)` (ESCOLHIDA).**

## Decisão

Portar o padrão atômico do `PtyHost` para o `Supervisor` de forma cirúrgica:

1. **Writer compartilhado.** `TermChannel` (lib.rs:1059) passa a guardar `sync_writer: Arc<Mutex<Box<dyn Write + Send>>>`, compartilhado entre a thread `serial_writer` (que passa a `lock()` o writer a cada op) e um novo método síncrono. `Supervisor::register` envolve o writer no `Arc<Mutex<…>>` e clona para o consumidor. A ordem serial dos bytes A2A é preservada (o Mutex serializa).
2. **Método atômico que recebe o grid do app.** `Supervisor::deliver_approval_with_grid(node, grid: &Mutex<Box<dyn VtBackend>>, expected_hash, keys, region_rows) -> PortOutcome`: trava o `grid` (o MESMO lock que o reader-loop trava antes de `advance` → bloqueia o advance), faz `approval::check_screen`, e **só em `Match`** trava o `sync_writer` e `write_all`+`flush` — tudo com o grid-lock vivo. Tela divergiu ⇒ zero bytes. O core recebe o grid como `&Mutex<Box<dyn VtBackend>>` (tipo de `lina-vt`, que já é dep do core) — **nenhum tipo de toolkit/gpui entra no core**.
3. **Wrapper no app.** `SupervisorApprovalPort { sup, grid, node }` em bridge.rs implementa `ApprovalPort` chamando `deliver_approval_with_grid` → o `ApprovalExecutor::deliver` (núcleo, já pronto) o usa. O `ApprovalExecutor`/`ApprovalLedger` são alimentados pelo event log (boot) e pelo canal de eventos (runtime), na `AttentionHub`. O call-site do clique do toast (`AttentionHub::resolve`) passa a usar `ApprovalExecutor::deliver` em vez do `write_human` audit-only de hoje. O auto-deny (driver `auto_deny_due`, F1-1-8 #2) usa o MESMO ponto com gesto `Deny/Timeout`.

## Invariantes (não regredir)

- **Ordem de locks SEMPRE `grid → writer`, nunca invertida.** O reader-loop segura só o grid; o `serial_writer` segura só o writer; `deliver_approval_with_grid` segura grid e depois writer. Sem ciclo → sem deadlock. Qualquer código futuro que queira ambos respeita essa ordem.
- **Atomicidade:** o write da aprovação chega ao master enquanto o grid-lock está vivo → nenhum `advance` entre check e write (a janela irredutível é só o buffer do kernel, ADR 0021 §4 R2).
- **`approval.rs` inalterado** — a trait `ApprovalPort` e `check_screen` já encaixam.
- **Identidade/autorização vêm do LOG** (`ApprovalLedger.node_of`), nunca da UI/fila (ADR 0021 §5; gate_f1_1_8 AC-0021.3/.4).
- **Supervisor NÃO migra para PtyHost na Fase 1** — a coexistência (PtyHost no core/tests, Supervisor no app) é a decisão ativa; a convergência futura é um ADR próprio se/quando valer.

## Consequências

- Diff cirúrgico no core (TermChannel + serial_writer + register + 1 método) + bridge.rs (wrapper + call-site + integração executor/ledger na AttentionHub). Sem tocar A2A/F1-2-4/broker.
- Fecha o gate F1-1 (AC-0021.7 na tela: aprovar pelo toast destrava Claude real) após validação do fundador.

## Relacionados

ADR 0021 (segurança da aprovação y/n — modelo), ADR 0023 (re-injeção human-proxy — outro write ao PTY, caminho distinto), ADR 0006/0010 (WorkspaceTrust). `gate_f1_1_8` (AC-0021.1–.7).
