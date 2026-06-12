//! **Sonda de custo do SessionWatch (r5 perf-ws, BUG 3 da tela do fundador).**
//!
//! Mede o custo REAL de `poll_once` em corpus sintético com a forma da máquina do
//! fundador (~144 dirs de projeto, ~3.7k session-files): o conteúdo é incremental
//! (cursor por arquivo), mas cada poll paga a CAMINHADA (read_dir por subdir) +
//! 1 stat por arquivo — a cada 500ms, POR runtime (N Espaços = N tempestades).
//! Os números saem no stdout (`--nocapture`) e alimentam a entrega; as asserções
//! são de INVARIANTE (não de tempo — sem flake em CI).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lina_session_watch::SessionWatch;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("lina-swprobe-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("criar tempdir");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Linha JSONL mínima no formato real (usage + requestId) — ~230 bytes.
fn line(sid: &str, req: u32) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"{sid}","requestId":"req_{req}","cwd":"/work/x","timestamp":"2026-06-11T12:00:00.000Z","message":{{"id":"m{req}","model":"claude-opus-4-8","usage":{{"input_tokens":10,"output_tokens":5}}}}}}"#
    ) + "\n"
}

/// Corpus com a FORMA da máquina do fundador: `dirs` pastas × `files` arquivos × `lines` linhas.
fn build_corpus(home: &Path, dirs: usize, files: usize, lines: u32) {
    for d in 0..dirs {
        let proj = home.join(format!(".claude/projects/proj-{d:03}"));
        std::fs::create_dir_all(&proj).expect("dir");
        for f in 0..files {
            let mut content = String::new();
            for l in 0..lines {
                content.push_str(&line(&format!("sess-{d:03}-{f:02}"), l));
            }
            std::fs::write(proj.join(format!("sess-{d:03}-{f:02}.jsonl")), content)
                .expect("arquivo");
        }
    }
}

/// **ANTES (forma do bug):** N runtimes = N watchers independentes, cada um pagando
/// cold-parse INTEIRO + caminhada/stat completa por poll. Imprime ms por fase.
#[test]
fn probe_poll_costs_on_founder_shaped_corpus() {
    let tmp = TempDir::new("corpus");
    // 144×26 ≈ 3744 arquivos (forma real da máquina: 144 dirs, 3735 jsonl).
    build_corpus(tmp.path(), 144, 26, 6);

    let mut watch = SessionWatch::with_home(tmp.path());
    watch.add_source("claude-code", "~/.claude/projects/*/*.jsonl");

    let t0 = Instant::now();
    let cold = watch.poll_once().expect("poll frio");
    let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cold.files_scanned, 144 * 26, "cold parse cobre o corpus");

    // Steady-state: NADA mudou — o custo é a caminhada+stat pura (paga a cada 500ms).
    let t1 = Instant::now();
    let steady = watch.poll_once().expect("poll estável");
    let steady_ms = t1.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(steady.files_scanned, 0, "sem mudança → zero conteúdo lido");

    // 1 append num arquivo: o poll paga a caminhada + o delta de 1 linha.
    {
        let f = tmp
            .path()
            .join(".claude/projects/proj-000/sess-000-00.jsonl");
        let mut h = std::fs::OpenOptions::new()
            .append(true)
            .open(f)
            .expect("abrir");
        h.write_all(line("sess-000-00", 99).as_bytes())
            .expect("append");
    }
    let t2 = Instant::now();
    let delta = watch.poll_once().expect("poll delta");
    let delta_ms = t2.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(delta.files_scanned, 1, "só o arquivo tocado consome bytes");

    println!("[PERF-WS][ANTES] corpus 144 dirs x 26 files (3744 jsonl):");
    println!("[PERF-WS][ANTES] poll FRIO (parse total, pago POR RUNTIME no boot): {cold_ms:.1} ms");
    println!(
        "[PERF-WS][ANTES] poll ESTÁVEL (caminhada+stat, pago a cada 500ms POR RUNTIME): {steady_ms:.2} ms"
    );
    println!("[PERF-WS][ANTES] poll com 1 append: {delta_ms:.2} ms");
    println!(
        "[PERF-WS][ANTES] projeção: N=4 Espaços → {:.0} stats/s contínuos + 4x cold-parse no boot",
        4.0 * 2.0 * 3744.0
    );
}

/// **Sonda no HOME REAL da máquina (opt-in: `--ignored --nocapture`).** Não-hermética
/// por natureza (depende do disco do dono) — fica fora do CI; é a evidência de campo.
#[test]
#[ignore = "sonda de campo: depende do HOME real; rode com --ignored --nocapture"]
fn probe_real_home_poll_cost() {
    let Some(home) = std::env::var_os("HOME") else {
        eprintln!("[PERF-WS] sem HOME — sonda pulada");
        return;
    };
    let mut watch = SessionWatch::with_home(PathBuf::from(home));
    watch.add_source("claude-code", "~/.claude/projects/*/*.jsonl");

    let t0 = Instant::now();
    let cold = watch.poll_once().expect("poll frio");
    let cold_s = t0.elapsed().as_secs_f64();

    let t1 = Instant::now();
    let _ = watch.poll_once().expect("poll estável");
    let steady_ms = t1.elapsed().as_secs_f64() * 1000.0;

    println!("[PERF-WS][CAMPO] HOME real: cold-parse {cold_s:.2}s ({} arquivos tocados); steady {steady_ms:.2} ms/poll", cold.files_scanned);
}
