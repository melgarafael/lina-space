//! **F3-CONF-3 · R3 (QA/RED) — repro do achado #23c/#14**: `lina list` MENTE vs `lina check`.
//!
//! `run_list()` (`lina.rs:3163`) imprime o roster cru de `agents.json`; quando um NOME foi reusado
//! entre sessões (homônimo de sessão MORTA + nó vivo), `list` mostra o status velho/morto, enquanto
//! `lina check` já resolve o VIVO via `resolve_check_node` (`lina.rs:511`). Divergência → o Maestro
//! que confia no `list` re-despacha quem está TRABALHANDO (gate (b) da onda-f3-conf-3).
//!
//! **Propriedade testada (agnóstica de implementação):** com um homônimo morto+vivo no log, `list` e
//! `check` CONCORDAM no nó VIVO. Cobre as **3 formas de morte** (`NodeStatusChanged(Dead)`,
//! `TerminalExited`, `NodeRemoved` — não só a 1ª) e o CONTRASTE: sem vivo (só morto), ambos exibem o
//! morto HONESTAMENTE (não inventam vivo). RED no HEAD (list = status morto de `agents.json`) → GREEN
//! com o fix do H (list reconcilia pelo log, reusando a semântica de morte do `resolve_check_node`).
//!
//! **Não-vacuosidade (recency trap):** o nó morto é renomeado para "Worker" DEPOIS do vivo, então a
//! recência o faria vencer SE a morte não fosse detectada (`resolve_check_node` desempata por índice).
//! Só a detecção correta de cada uma das 3 mortes escolhe o vivo — o teste falha se qualquer forma
//! de morte for ignorada.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempHome(PathBuf);

impl TempHome {
    /// tmpdir único por (processo, THREAD, nanos) — `thread::id` mata o flaky paralelo (lição
    /// CONF-HARNESS `history_f1_5_8`).
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tid: String = format!("{:?}", std::thread::current().id())
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        let p = std::env::temp_dir().join(format!(
            "lina-conf3-list-{tag}-{}-{tid}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&p).expect("criar LINA_HOME");
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(home: &TempHome, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    Command::new(exe)
        .args(args)
        .env("LINA_HOME", home.path())
        .output()
        .expect("executar o binário lina")
}

/// Escreve `events/log.jsonl` (= `event_log_path()`) com o conteúdo dado.
fn seed_log(home: &TempHome, lines: &[String]) {
    let dir = home.path().join("events");
    std::fs::create_dir_all(&dir).expect("events/");
    std::fs::write(dir.join("log.jsonl"), lines.join("\n") + "\n").expect("log.jsonl");
}

/// A primeira linha do `lina list` (texto) que menciona `name`.
fn linha_do(out: &str, name: &str) -> String {
    out.lines()
        .find(|l| l.contains(name))
        .unwrap_or_default()
        .to_string()
}

/// **#23c — as 3 formas de morte.** Para cada sinal de morte do homônimo, `list` e `check` têm que
/// concordar no nó VIVO ("Busy"), nunca no morto ("Dead"). O `agents.json` carrega o status STALE
/// (Dead) — a fonte do engano do HEAD; o fix reconcilia pelo log.
#[test]
fn list_concorda_com_check_no_vivo_nas_3_formas_de_morte() {
    // node vivo (L) e node morto (D), MESMO nome "Worker". D batizado DEPOIS de L (recency trap).
    // ids são UUIDs hex VÁLIDOS (o `live_member_ids` filtra no `.parse::<NodeId>()`), só distintos.
    let live = "0198a0f0-0000-7000-8000-000000000111";
    let dead = "0198a0f0-0000-7000-8000-000000000222";

    // (rótulo, evento de MORTE de D) — as 3 formas.
    let mortes: [(&str, String); 3] = [
        (
            "NodeStatusChanged(Dead)",
            format!(
                r#"{{"seq":5,"ts":500,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{dead}","status":"Dead","from":"Idle","reason":"app_reopened"}}}}"#
            ),
        ),
        (
            "TerminalExited",
            format!(
                r#"{{"seq":5,"ts":500,"kind":"TerminalExited","version":1,"payload":{{"event":"TerminalExited","node":"{dead}"}}}}"#
            ),
        ),
        (
            "NodeRemoved",
            format!(
                r#"{{"seq":5,"ts":500,"kind":"NodeRemoved","version":1,"payload":{{"event":"NodeRemoved","node":"{dead}"}}}}"#
            ),
        ),
    ];

    for (forma, morte) in mortes {
        let home = TempHome::new("3mortes");
        // roster STALE: o supervisor gravou o homônimo MORTO (status Dead) — o engano do HEAD.
        std::fs::write(
            home.path().join("agents.json"),
            r#"[{"name":"Worker","role":"dev","status":"Dead"}]"#,
        )
        .expect("agents.json");
        seed_log(
            &home,
            &[
                // L vivo, batizado PRIMEIRO, status Busy.
                format!(
                    r#"{{"seq":1,"ts":100,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{live}","name":"Worker"}}}}"#
                ),
                format!(
                    r#"{{"seq":2,"ts":200,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{live}","status":"Busy","from":"Idle","reason":"pty_output"}}}}"#
                ),
                // D, batizado DEPOIS (recency trap), e a morte da vez.
                format!(
                    r#"{{"seq":3,"ts":300,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{dead}","name":"Worker"}}}}"#
                ),
                format!(
                    r#"{{"seq":4,"ts":400,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{dead}","status":"Idle","from":"Starting","reason":"boot"}}}}"#
                ),
                morte,
            ],
        );

        // REFERÊNCIA: `check` já resolve o vivo (código existente) — prova que o vivo é "Busy".
        let check = run(&home, &["check", "@Worker"]);
        let check_txt = String::from_utf8_lossy(&check.stdout);
        assert!(
            check.status.success() && check_txt.contains("Busy"),
            "[{forma}] pré-condição: `check` resolve o VIVO (Busy). saída: {check_txt}"
        );

        // PROPRIEDADE: `list` tem que CONCORDAR com `check` — mostrar o vivo, não o morto.
        let list = run(&home, &["list"]);
        let list_txt = String::from_utf8_lossy(&list.stdout);
        let linha = linha_do(&list_txt, "Worker");
        assert!(
            linha.contains("Busy"),
            "[{forma}] RED no HEAD: `list` mostra o status do homônimo MORTO, não concorda com o \
             `check` (vivo=Busy). linha do list: {linha:?}"
        );
        assert!(
            !linha.contains("Dead"),
            "[{forma}] `list` não pode exibir o status do morto quando há vivo homônimo. linha: {linha:?}"
        );
    }
}

/// **#23c — CONTRASTE (não-vacuosidade do fix): sem vivo, exibe o morto HONESTAMENTE.** Um nome cujo
/// único nó está morto NÃO pode ser "promovido" a vivo pelo fix — `list` e `check` concordam no MORTO.
/// Garante que a regra "prefira o vivo" não vira "invente um vivo". Deve ficar VERDE no HEAD e no fix.
#[test]
fn so_morto_exibe_morto_honestamente_em_list_e_check() {
    let home = TempHome::new("so-morto");
    let dead = "0198a0f0-0000-7000-8000-000000000333";
    std::fs::write(
        home.path().join("agents.json"),
        r#"[{"name":"Ghost","role":"dev","status":"Dead"}]"#,
    )
    .expect("agents.json");
    seed_log(
        &home,
        &[
            format!(
                r#"{{"seq":1,"ts":100,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{dead}","name":"Ghost"}}}}"#
            ),
            format!(
                r#"{{"seq":2,"ts":200,"kind":"TerminalExited","version":1,"payload":{{"event":"TerminalExited","node":"{dead}"}}}}"#
            ),
        ],
    );

    let check = run(&home, &["check", "@Ghost"]);
    let check_txt = String::from_utf8_lossy(&check.stdout);
    assert!(
        check_txt.contains("Dead") || check_txt.to_lowercase().contains("morto"),
        "controle: sem vivo, `check` exibe o morto honestamente. saída: {check_txt}"
    );

    let list = run(&home, &["list"]);
    let list_txt = String::from_utf8_lossy(&list.stdout);
    let linha = linha_do(&list_txt, "Ghost");
    assert!(
        !linha.contains("Busy") && !linha.contains("Running"),
        "controle: o fix NÃO pode inventar um vivo onde só há morto. linha do list: {linha:?}"
    );
}
