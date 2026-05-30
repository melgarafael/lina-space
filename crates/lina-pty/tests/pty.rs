//! Testes de aceite da story **W0-1** (PTY Manager).
//!
//! Rodam em Mac/Linux direto. O gate cross-SO completo (Windows/ConPTY) chega
//! com o CI da **W5-6**; aqui provamos o critério no host atual.
//!
//! Estratégia do teste-âncora: cada nó é um `sh` **interativo**. Escrevemos
//! `echo $$` no master e lemos de volta o PID do shell — isso exercita o caminho
//! inteiro (spawn → writer 1-dono → reader → output) e prova **PID distinto por
//! PTY**, mais forte que `sh -c "echo $$"` (que sairia antes de qualquer escrita).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lina_pty::{PtyCommand, PtyError, PtyManager};

/// Lê do `reader` até achar uma linha só-dígitos (o PID que `echo $$` imprime) ou
/// EOF. O eco do input (`echo $$`) nunca é só-dígitos, então não confunde.
fn read_pid(mut reader: Box<dyn Read + Send>) -> Option<u32> {
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if let Some(pid) = find_pid(&acc) {
                    return Some(pid);
                }
            }
        }
    }
    find_pid(&acc)
}

fn find_pid(text: &str) -> Option<u32> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()))
        .find_map(|line| line.parse::<u32>().ok())
        .filter(|&pid| pid > 1)
}

fn all_distinct(values: &[u32]) -> bool {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.len() == values.len()
}

/// Critério de aceite W0-1: 8 PTYs concorrentes => 8 PIDs distintos.
#[test]
fn eight_concurrent_ptys_report_distinct_pids() {
    const N: usize = 8;
    let mut manager = PtyManager::new();

    let mut spawn_pids = Vec::with_capacity(N);
    for i in 0..N {
        let id = format!("node-{i}");
        let pid = manager
            .spawn(id.as_str(), PtyCommand::new("sh"), 80, 24)
            .expect("spawn do PTY");
        spawn_pids.push(pid);
    }
    assert_eq!(manager.len(), N, "deve haver 8 PTYs vivos");

    let mut pending = Vec::with_capacity(N);
    for i in 0..N {
        let id = format!("node-{i}");
        let reader = manager.clone_reader(id.as_str()).expect("clonar reader");
        let mut writer = manager
            .take_writer(id.as_str())
            .expect("tomar writer (1 dono)");

        let (tx, rx) = mpsc::channel();
        let join = thread::spawn(move || {
            let _ = tx.send(read_pid(reader));
        });

        // Escreve no master de cada: pede o PID e manda sair (gera EOF no reader).
        writer
            .write_all(b"echo $$\n")
            .expect("escrever echo no master");
        writer.write_all(b"exit\n").expect("escrever exit");
        writer.flush().expect("flush do master");
        pending.push((rx, join));
    }

    let mut echoed_pids = Vec::with_capacity(N);
    for (rx, join) in pending {
        let pid = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("PID dentro do timeout")
            .expect("linha de PID encontrada na saída");
        join.join().expect("join da thread de leitura");
        echoed_pids.push(pid);
    }

    assert_eq!(echoed_pids.len(), N);
    assert!(
        all_distinct(&echoed_pids),
        "os PIDs lidos de `echo $$` devem ser distintos: {echoed_pids:?}"
    );
    assert!(
        all_distinct(&spawn_pids) && spawn_pids.iter().all(|&p| p > 0),
        "os PIDs reportados no spawn devem ser reais e distintos: {spawn_pids:?}"
    );
}

/// O writer do master tem **um único dono**: a 2ª chamada falha.
#[test]
fn writer_is_single_owner() {
    let mut manager = PtyManager::new();
    manager
        .spawn("solo", PtyCommand::new("sh"), 80, 24)
        .expect("spawn");
    let _writer = manager.take_writer("solo").expect("1º take_writer ok");
    let second = manager.take_writer("solo");
    assert!(
        matches!(second, Err(PtyError::WriterAlreadyTaken(_))),
        "2º take_writer deve falhar com WriterAlreadyTaken"
    );
    let _ = manager.kill("solo", Duration::from_millis(500));
}

/// O comando realmente roda no PTY: `$((6*7))` só aparece como `42` na SAÍDA,
/// nunca no eco do input — prova de execução, não só de echo.
#[test]
fn spawn_runs_command_and_streams_output() {
    let mut manager = PtyManager::new();
    manager
        .spawn("calc", PtyCommand::new("sh"), 80, 24)
        .expect("spawn");
    let mut reader = manager.clone_reader("calc").expect("reader");
    let mut writer = manager.take_writer("calc").expect("writer");
    writer
        .write_all(b"echo $((6*7))\nexit\n")
        .expect("escrever comando");
    writer.flush().expect("flush");

    let mut out = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
    }
    assert!(
        out.contains("42"),
        "esperava 42 na saída do shell; veio: {out:?}"
    );
}

/// `resize` funciona em PTY existente; operações em nó inexistente erram limpo.
#[test]
fn resize_works_and_missing_node_errors_cleanly() {
    let mut manager = PtyManager::new();
    manager
        .spawn("r", PtyCommand::new("sh"), 80, 24)
        .expect("spawn");
    manager
        .resize("r", 120, 40)
        .expect("resize do PTY existente");

    assert!(matches!(
        manager.resize("inexistente", 80, 24),
        Err(PtyError::NotFound(_))
    ));
    assert!(matches!(
        manager.take_writer("inexistente"),
        Err(PtyError::NotFound(_))
    ));
    assert!(matches!(
        manager.clone_reader("inexistente"),
        Err(PtyError::NotFound(_))
    ));

    let _ = manager.kill("r", Duration::from_millis(500));
}
