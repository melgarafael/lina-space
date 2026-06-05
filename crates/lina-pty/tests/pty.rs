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
#[cfg(not(windows))]
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

#[cfg(not(windows))]
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

#[cfg(windows)]
fn test_shell() -> PtyCommand {
    PtyCommand::new("cmd.exe").args(["/Q", "/K"])
}

#[cfg(not(windows))]
fn test_shell() -> PtyCommand {
    PtyCommand::new("sh")
}

#[cfg(windows)]
fn line(s: &str) -> Vec<u8> {
    format!("{s}\r").into_bytes()
}

#[cfg(not(windows))]
fn line(s: &str) -> Vec<u8> {
    format!("{s}\n").into_bytes()
}

fn read_until(mut reader: Box<dyn Read + Send>, needle: &str) -> Option<String> {
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
                if acc.contains(needle) {
                    return Some(acc);
                }
            }
        }
    }
    acc.contains(needle).then_some(acc)
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
            .spawn(id.as_str(), test_shell(), 80, 24)
            .expect("spawn do PTY");
        spawn_pids.push(pid);
    }
    assert_eq!(manager.len(), N, "deve haver 8 PTYs vivos");

    #[cfg(windows)]
    thread::sleep(Duration::from_millis(500));

    let mut pending = Vec::with_capacity(N);
    for i in 0..N {
        let id = format!("node-{i}");
        let reader = manager.clone_reader(id.as_str()).expect("clonar reader");
        let mut writer = manager
            .take_writer(id.as_str())
            .expect("tomar writer (1 dono)");

        let (tx, rx) = mpsc::channel();
        #[cfg(not(windows))]
        let join = thread::spawn(move || {
            let _ = tx.send(read_pid(reader));
        });
        #[cfg(windows)]
        let marker = format!("LINA_NODE_{i}");
        #[cfg(windows)]
        let join = {
            let marker = marker.clone();
            thread::spawn(move || {
                let _ = tx.send(read_until(reader, &marker).map(|_| i as u32 + 1));
            })
        };

        // Escreve no master de cada e manda sair (gera EOF no reader).
        #[cfg(not(windows))]
        writer
            .write_all(b"echo $$\n")
            .expect("escrever echo no master");
        #[cfg(windows)]
        writer
            .write_all(&line(&format!("echo {marker}")))
            .expect("escrever echo no master");
        writer.write_all(&line("exit")).expect("escrever exit");
        writer.flush().expect("flush do master");
        pending.push((rx, join));
    }

    let mut observed = Vec::with_capacity(N);
    for (rx, join) in pending {
        let value = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("resposta dentro do timeout")
            .expect("resposta encontrada na saída");
        join.join().expect("join da thread de leitura");
        observed.push(value);
    }

    assert_eq!(observed.len(), N);
    #[cfg(not(windows))]
    assert!(
        all_distinct(&observed),
        "os PIDs lidos de `echo $$` devem ser distintos: {observed:?}"
    );
    #[cfg(windows)]
    assert!(
        all_distinct(&observed),
        "cada PTY Windows deve reportar seu marker: {observed:?}"
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
    manager.spawn("solo", test_shell(), 80, 24).expect("spawn");
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
    #[cfg(not(windows))]
    let cmd = test_shell();
    #[cfg(windows)]
    let cmd = PtyCommand::new("cmd.exe").args(["/Q", "/C", "echo 42"]);
    manager.spawn("calc", cmd, 80, 24).expect("spawn");
    let reader = manager.clone_reader("calc").expect("reader");
    #[cfg(not(windows))]
    let mut writer = manager.take_writer("calc").expect("writer");
    #[cfg(not(windows))]
    writer
        .write_all(b"echo $((6*7))\nexit\n")
        .expect("escrever comando");
    #[cfg(not(windows))]
    writer.flush().expect("flush");

    #[cfg(not(windows))]
    let out = {
        let mut reader = reader;
        let mut out = String::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
        out
    };
    #[cfg(windows)]
    let out = read_until(reader, "42").unwrap_or_default();
    assert!(
        out.contains("42"),
        "esperava 42 na saída do shell; veio: {out:?}"
    );
    let _ = manager.kill("calc", Duration::from_millis(500));
}

/// `resize` funciona em PTY existente; operações em nó inexistente erram limpo.
#[test]
fn resize_works_and_missing_node_errors_cleanly() {
    let mut manager = PtyManager::new();
    manager.spawn("r", test_shell(), 80, 24).expect("spawn");
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
