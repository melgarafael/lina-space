//! F2-0-6 — **synchronized output (DEC private mode 2026)** no emulador, provado pela API PÚBLICA.
//!
//! ## Por quê (fonte D3-A3, verificada)
//! Os CLIs de IA que o Lina roda (Claude Code et al.) redesenham a tela inteira a cada token — o
//! PIOR caso de *flicker* (frame rasgado no meio do redraw). A cura do lado do terminal é o **DEC
//! 2026**: o TUI envolve o redraw em `CSI ? 2026 h` … `CSI ? 2026 l` e o emulador **segura** a
//! apresentação até o batch fechar — zero frame intermediário. Pré-requisito do resize da F2-3.
//!
//! ## Onde mora o suporte (achado da investigação)
//! O `Term` do `alacritty_terminal` 0.26 trata `2026` como **no-op**; quem implementa o hold é o
//! parser `vte` 0.15 (`Processor`, que o `AlacrittyBackend` já usa): writes entre BSU e ESU são
//! **bufferizados** e aplicados ATÔMICOS no ESU — o grid em si não muda no meio do batch.
//!
//! ## Dois tetos anti-congelamento (defesa em profundidade)
//! 1. **Byte-cap (2 MiB)** — intrínseco ao `advance`, SEMPRE ativo, sem driver: um flood com ESU
//!    esquecido auto-libera (prova em [`runaway_sync_auto_flushes_at_intrinsic_byte_cap`]).
//! 2. **Time-cap (~150 ms)** — o `advance` só MARCA o deadline ([`VtBackend::sync_deadline`]); o
//!    flush no timeout é DIRIGIDO pelo reader via [`VtBackend::flush_sync`] (prova em
//!    [`flush_sync_presents_held_batch_when_esu_is_forgotten`]). É o teto que o reader agenda.
//!
//! Estes testes batem só na **API pública** da trait `VtBackend` — são o contrato fim-a-fim que a
//! porta `libghostty-vt` futura terá de honrar, não detalhes do alacritty.

use lina_vt::{AlacrittyBackend, VtBackend, VtScreen};
use std::time::{Duration, Instant};

/// Texto da linha `line` do viewport (sem espaços à direita) — lê só o snapshot público.
fn row(screen: &VtScreen, line: usize) -> String {
    screen
        .row(line)
        .iter()
        .map(|cell| cell.c)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// **Critério (a) — o HOLD.** Writes entre `CSI ? 2026 h` e `CSI ? 2026 l` NÃO vazam para o grid;
/// o batch inteiro vira visível DE UMA VEZ no ESU. É o guardião contra o flicker e contra uma
/// futura troca de backend/pin que perca o synchronized output (regressão muda visivelmente: o
/// `mid` passaria a mostrar "new").
#[test]
fn synchronized_output_holds_intermediate_frames_until_esu() {
    let mut b = AlacrittyBackend::new(20, 3);

    // Frame inicial visível: "old" na linha 0.
    b.advance(b"\x1b[2J\x1b[Hold");
    assert_eq!(row(&b.screen(), 0), "old", "estado inicial");

    // Abre o batch atômico e redesenha (limpa + home + "new") — TUDO retido pelo sync.
    b.advance(b"\x1b[?2026h\x1b[2J\x1b[Hnew");

    // SEM o ESU, o grid continua no frame anterior: nenhum byte do redraw vazou.
    assert_eq!(
        row(&b.screen(), 0),
        "old",
        "writes entre BSU e ESU ficam RETIDOS — o grid não muda no meio do batch"
    );

    // Fecha o batch: agora o redraw inteiro aparece atômico.
    b.advance(b"\x1b[?2026l");
    assert_eq!(
        row(&b.screen(), 0),
        "new",
        "no ESU o batch retido vira visível DE UMA VEZ (zero frame rasgado)"
    );
}

/// **Critério (b) parte 1 — o deadline exposto.** Abrir o sync publica um deadline no FUTURO
/// próximo (o teto de ~150 ms); fechar o sync zera o deadline. É o que o reader usa para AGENDAR o
/// flush do teto sem busy-poll.
#[test]
fn sync_deadline_is_published_while_held_and_cleared_on_esu() {
    let mut b = AlacrittyBackend::new(20, 3);
    assert!(
        b.sync_deadline().is_none(),
        "sem batch aberto, não há deadline"
    );

    b.advance(b"\x1b[?2026h\x1b[2J\x1b[Hheld");
    // Limite medido DEPOIS do advance: o vte arma o deadline em `Instant::now() + 150ms` no
    // instante do BSU (≤ `now_after`), logo `deadline ≤ now_after + 150ms` sempre vale.
    let now_after = Instant::now();

    let deadline = b
        .sync_deadline()
        .expect("abrir o sync (BSU) publica um deadline de apresentação");
    assert!(deadline > now_after, "o deadline é no futuro");
    assert!(
        deadline <= now_after + Duration::from_millis(150),
        "o teto anti-congelamento é ≤ 150 ms (DEC 2026 / SYNC_UPDATE_TIMEOUT)"
    );

    b.advance(b"\x1b[?2026l");
    assert!(
        b.sync_deadline().is_none(),
        "fechar o sync (ESU) zera o deadline"
    );
}

/// **Critério (b) parte 2 — o teto de tempo dirigido.** App abre o sync, redesenha e ESQUECE o
/// ESU. O grid fica retido (frame anterior). O reader, ao ver o deadline vencido, chama
/// `flush_sync()` → o batch retido é apresentado atômico e o grid destrava. Idempotente: sem nada
/// retido, é no-op e devolve `false`.
#[test]
fn flush_sync_presents_held_batch_when_esu_is_forgotten() {
    let mut b = AlacrittyBackend::new(20, 3);
    b.advance(b"\x1b[2J\x1b[Hold");

    // Abre + redesenha, sem fechar.
    b.advance(b"\x1b[?2026h\x1b[2J\x1b[Hnew");
    assert_eq!(row(&b.screen(), 0), "old", "ainda retido, sem o ESU");
    assert!(b.sync_deadline().is_some(), "há um sync pendente");

    // O teto: forçar o fim apresenta o que estava retido.
    assert!(
        b.flush_sync(),
        "havia um batch retido para liberar → devolve true"
    );
    assert_eq!(
        row(&b.screen(), 0),
        "new",
        "o teto apresenta o frame que o app deixou preso"
    );
    assert!(
        b.sync_deadline().is_none(),
        "após o flush não há mais sync pendente"
    );

    // Idempotência: sem nada retido, é no-op.
    assert!(
        !b.flush_sync(),
        "nada pendente → flush_sync é no-op e devolve false"
    );
}

/// **BSU aninhado NÃO vaza frame parcial.** Um 2º `CSI ? 2026 h` dentro de um sync já aberto não
/// causa apresentação parcial: o modelo do `vte` é PLANO (não-contado) — o BSU aninhado só RE-ARMA o
/// timeout; só o `CSI ? 2026 l` fecha. Logo TUDO segue retido até o ESU, quando aplica atômico.
/// Durante o aninhamento o grid permanece no frame pré-sync — nada vaza no meio. (Guarda contra a
/// hipótese de que `bsu_offset`/`stop_sync_internal` apresentaria o trecho anterior ao BSU aninhado:
/// esse caminho só dispara para `…ESU…BSU…` no MESMO chunk — fecha um sync e abre outro, correto.)
#[test]
fn nested_bsu_does_not_leak_intermediate_frame() {
    let mut b = AlacrittyBackend::new(20, 3);
    b.advance(b"\x1b[2J\x1b[Hinitial");
    assert_eq!(row(&b.screen(), 0), "initial", "frame inicial");

    b.advance(b"\x1b[?2026h"); // abre o sync externo
    b.advance(b"\x1b[2J\x1b[Hmiddle"); // retido
    assert_eq!(row(&b.screen(), 0), "initial", "nada aplicado ainda");

    b.advance(b"\x1b[?2026h"); // BSU ANINHADO — re-arma, NÃO fecha, NÃO apresenta o parcial
    assert_eq!(
        row(&b.screen(), 0),
        "initial",
        "BSU aninhado não vaza o parcial 'middle'"
    );

    b.advance(b"new"); // retido
    assert_eq!(row(&b.screen(), 0), "initial", "ainda retido");

    b.advance(b"\x1b[?2026l"); // fecha: aplica o batch inteiro atômico
    assert_eq!(
        row(&b.screen(), 0),
        "middlenew",
        "no ESU o batch retido inteiro (middle+new) aparece DE UMA VEZ — atomicidade preservada"
    );
}

/// **Caso `…ESU…BSU…` no mesmo chunk.** Aqui o `bsu_offset` do vte SIM atua — e corretamente:
/// o 1º ESU fecha o sync atual (apresenta o que veio antes dele, atômico) e o BSU seguinte abre um
/// NOVO sync (segura o resto). Não é vazamento: é dois batches atômicos distintos. Guarda o limite
/// do comportamento provado em [`nested_bsu_does_not_leak_intermediate_frame`].
#[test]
fn esu_then_new_bsu_in_one_chunk_closes_first_holds_second() {
    let mut b = AlacrittyBackend::new(20, 3);
    b.advance(b"\x1b[2J\x1b[Hbase");
    b.advance(b"\x1b[?2026h"); // abre sync 1

    // Um único chunk: redraw "one" + ESU (fecha sync 1) + BSU (abre sync 2) + redraw "two".
    b.advance(b"\x1b[2J\x1b[Hone\x1b[?2026l\x1b[?2026h\x1b[2J\x1b[Htwo");

    // sync 1 fechou → "one" apresentado; sync 2 está aberto → "two" RETIDO.
    assert_eq!(
        row(&b.screen(), 0),
        "one",
        "o 1º ESU apresenta o 1º batch ('one'); o 2º batch ('two') fica retido pelo novo BSU"
    );
    assert!(b.sync_deadline().is_some(), "o 2º sync está aberto");

    b.advance(b"\x1b[?2026l"); // fecha sync 2
    assert_eq!(
        row(&b.screen(), 0),
        "two",
        "fechado o 2º sync, 'two' aparece"
    );
}

/// **`damaged_rows()` durante o HOLD = só a linha do cursor (benigno).** O `Term` do alacritty
/// SEMPRE marca a linha do cursor em `damage()` (term/mod.rs:480), então um `advance` que abre o
/// sync reporta `[cursor_row]` mesmo sem o grid mudar. NÃO é frame rasgado: repintar a linha do
/// cursor re-renderiza o conteúdo INALTERADO (idempotente). O que garante a integridade visual do
/// hold é o CONTEÚDO do grid não mudar — provado nos testes acima —, não `damaged_rows` ser vazio.
/// Este teste documenta o comportamento real (a spec do reader reflete isto).
#[test]
fn damage_during_hold_is_only_the_benign_cursor_row() {
    let mut b = AlacrittyBackend::new(20, 3);
    b.advance(b"\x1b[2J\x1b[Hold");
    b.reset_damage();

    // Abre o sync e redesenha — retido. O conteúdo do grid não muda…
    b.advance(b"\x1b[?2026h\x1b[2J\x1b[Hnew");
    assert_eq!(
        row(&b.screen(), 0),
        "old",
        "conteúdo retido (sem frame rasgado)"
    );

    // …mas o dano reportado pode conter a linha do cursor (linha 0), e nada além dela.
    let dirty = b.damaged_rows();
    assert!(
        dirty.iter().all(|&r| r == 0),
        "no máximo a linha do cursor (0) é marcada durante o hold; nenhuma linha de conteúdo retido vaza: {dirty:?}"
    );
}

/// **O teto NÃO perde scrollback (capture ON).** Quando `flush_sync` libera um batch retido que
/// rolou linhas para fora do viewport, essas linhas DEVEM ser colhidas pelo cabo `append-on-scroll`
/// — senão o caminho do teto seria perda SILENCIOSA de histórico. Durante o sync nada é colhido
/// (tudo retido); após o `flush_sync`, as linhas roladas aparecem em `take_scrollback`, e o ring
/// fica no `cap` (não cresce indefinidamente). Guarda o reuso de `harvest_and_trim` no `flush_sync`.
#[test]
fn flush_sync_with_capture_harvests_rolled_lines_no_silent_loss() {
    let cap = 4usize;
    let mut b = AlacrittyBackend::with_scrollback_capture(20, 2, cap);
    b.advance(b"\x1b[2J\x1b[Hinitial");
    let _ = b.take_scrollback(); // drena a colheita do setup

    // Abre o sync e bufferiza muitas linhas (com 2 rows, cada linha extra rola 1 para o histórico).
    b.advance(b"\x1b[?2026h");
    for i in 0..10 {
        b.advance(format!("sync_line{i}\r\n").as_bytes());
    }
    assert!(
        b.take_scrollback().is_empty(),
        "DURANTE o sync nada é colhido — tudo retido no buffer do batch"
    );

    // Força o fim do sync (teto): o batch retido aplica e rola linhas → devem ser colhidas.
    assert!(b.flush_sync(), "havia sync aberto");
    assert!(
        !b.take_scrollback().is_empty(),
        "APÓS o flush do teto, as linhas que rolaram foram colhidas (sem perda silenciosa)"
    );
    assert_eq!(
        b.scrollback_len(),
        cap,
        "o ring fica no cap após o flush — não cresce indefinidamente"
    );
}

/// **Teto intrínseco (byte-cap, 2 MiB).** Mesmo SEM o driver de timeout, um app que abre o sync,
/// redesenha e nunca fecha NÃO congela o grid indefinidamente: ao exceder `SYNC_BUFFER_SIZE`
/// (2 MiB), o próprio parser força o flush. É a rede de segurança sempre-ligada contra DoS
/// (o leak classe-Ghostty: buffer de sync sem teto). O grid não pode mais ficar preso em "old".
#[test]
fn runaway_sync_auto_flushes_at_intrinsic_byte_cap() {
    let mut b = AlacrittyBackend::new(20, 3);
    b.advance(b"\x1b[2J\x1b[Hold");

    // Abre + redesenha "new", ESU esquecido.
    b.advance(b"\x1b[?2026h\x1b[2J\x1b[Hnew");
    assert_eq!(row(&b.screen(), 0), "old", "retido antes de estourar o cap");

    // Inunda > 2 MiB DENTRO do sync ainda aberto: o cap intrínseco força a liberação.
    let flood = vec![b'.'; 0x20_0000 + 16];
    b.advance(&flood);

    assert_ne!(
        row(&b.screen(), 0),
        "old",
        "byte-cap de 2 MiB impede congelamento indefinido mesmo sem ESU e sem driver de timeout"
    );
}
