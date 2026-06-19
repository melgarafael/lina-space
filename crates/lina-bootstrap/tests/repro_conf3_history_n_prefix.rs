//! **F3-CONF-3 · R3 (QA/RED) — repro do achado #27**: `lina history "@colega"` rejeita o
//! `LINA_NODE_ID` que o spawn de fato carimba (`n-<uuidv7>`, `bridge.rs:5001`/`workspace.rs:147`).
//!
//! `reader_node_id()` (`lina.rs:608`) faz `raw.trim().parse::<NodeId>()` — e `NodeId` é um newtype
//! sobre `Uuid`, que **não** aceita o prefixo `n-`. Resultado: o verbo de observabilidade #15 (a
//! "tela do worker" que a F3-2 entregou) morre no caminho REAL do spawn com `LINA_NODE_ID invalido`,
//! e o Maestro fica cego (achado #27, gate (a) da onda-f3-conf-3).
//!
//! Roda o BINÁRIO real (única superfície de teste do bin — as funções de parse vivem em `src/bin/`,
//! fora da lib). O discriminador é cirúrgico: `reader_node_id()` é o PRIMEIRO passo de `run_history`
//! (`lina.rs:745`), antes de qualquer scrollback/permissão — então a presença/ausência de
//! `"LINA_NODE_ID invalido"` isola exatamente o parse, independente do resto da máquina.
//!
//! **Estado esperado:** RED no HEAD (o `n-<uuid>` é rejeitado) → GREEN com o fix do H (strip do
//! prefixo `n-` no leitor, mantendo o uuid puro e rejeitando lixo). Cada caso tem controle positivo
//! (uuid puro / n-uuid aceitos) E negativo (lixo / `n-` sozinho rejeitados): a matriz é não-vacuosa —
//! o fix só FLIPA o caso `n-<uuid>` de reject→accept; os controles ficam fixos.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempHome(PathBuf);

impl TempHome {
    /// tmpdir único por (processo, THREAD, nanos) — `thread::id` mata o flaky sistêmico que envenena
    /// o gate paralelo do Maestro (lição CONF-HARNESS `history_f1_5_8` / `f3_conf2_seguranca`).
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
            "lina-conf3-hist-{tag}-{}-{tid}-{nanos}",
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

/// Roda `lina history <args>` com o `LINA_NODE_ID` (identidade do LEITOR) controlado pelo env.
fn run_history(home: &TempHome, node_id_env: &str, args: &[&str]) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut argv = vec!["history"];
    argv.extend_from_slice(args);
    Command::new(exe)
        .args(&argv)
        .env("LINA_HOME", home.path())
        .env("LINA_NODE_ID", node_id_env)
        .output()
        .expect("executar o binário lina")
}

/// `true` se a saída acusou identidade inválida (a falha do #27). Olha stderr E stdout por robustez.
fn rejeitou_identidade(out: &std::process::Output) -> bool {
    let err = String::from_utf8_lossy(&out.stderr);
    let txt = format!("{}{}", err, String::from_utf8_lossy(&out.stdout));
    txt.contains("LINA_NODE_ID invalido") || txt.contains("invalido")
}

/// **#27 — paridade de parse (matriz não-vacuosa).** O formato que o spawn carimba (`n-<uuidv7>`,
/// `bridge.rs:5001`) DEVE ser aceito como identidade do leitor, igual ao uuid puro; lixo e `n-`
/// sozinho continuam REJEITADOS (a segurança não relaxa — só se tolera o FORMATO real do app).
///
/// HEAD (RED): o caso `n-<uuid>` cai em "LINA_NODE_ID invalido" (junto do lixo). Com o fix (GREEN):
/// só o lixo/`n-`-sozinho caem; `n-<uuid>` e uuid puro passam o parse. QUEBRA SE… o fix aceitasse
/// QUALQUER string (o caso `lixo`/`n-naoehuuid` falharia) — provando que não viramos um bypass.
#[test]
fn history_aceita_n_prefixo_do_spawn_e_rejeita_lixo() {
    let home = TempHome::new("paridade");
    // `n-<uuidv7>` — EXATAMENTE o formato de `format!("n-{}", Uuid::now_v7())` (bridge.rs:5001).
    // uuid de exemplo do próprio achado #27.
    let uuid = "019edde3-faa9-7c51-bd5a-9f53c18addc0";
    let n_prefixado = format!("n-{uuid}");

    // — CONTROLES POSITIVOS: devem ser ACEITOS como identidade (parse passa) —
    let out_n = run_history(&home, &n_prefixado, &["@Worker"]);
    assert!(
        !rejeitou_identidade(&out_n),
        "FOCO DO #27: o `n-<uuid>` que o spawn carimba tem que ser aceito como identidade do leitor \
         (RED no HEAD — vira 'LINA_NODE_ID invalido'). stderr: {}",
        String::from_utf8_lossy(&out_n.stderr)
    );
    let out_puro = run_history(&home, uuid, &["@Worker"]);
    assert!(
        !rejeitou_identidade(&out_puro),
        "controle de paridade: o uuid PURO continua aceito (não pode regredir). stderr: {}",
        String::from_utf8_lossy(&out_puro.stderr)
    );

    // — CONTROLES NEGATIVOS: devem ser REJEITADOS (a tolerância é só ao formato real do app) —
    let out_lixo = run_history(&home, "lixo", &["@Worker"]);
    assert!(
        rejeitou_identidade(&out_lixo),
        "controle: identidade-lixo continua REJEITADA (o fix não pode virar bypass). stderr: {}",
        String::from_utf8_lossy(&out_lixo.stderr)
    );
    let out_so_prefixo = run_history(&home, "n-", &["@Worker"]);
    assert!(
        rejeitou_identidade(&out_so_prefixo),
        "controle: `n-` SOZINHO (sem uuid) não é identidade válida. stderr: {}",
        String::from_utf8_lossy(&out_so_prefixo.stderr)
    );
    let out_n_lixo = run_history(&home, "n-naoehuuid", &["@Worker"]);
    assert!(
        rejeitou_identidade(&out_n_lixo),
        "controle: `n-` + não-uuid é REJEITADO (não basta ter o prefixo). stderr: {}",
        String::from_utf8_lossy(&out_n_lixo.stderr)
    );
}

/// **#27 — caminho real ponta-a-ponta.** Com um log onde o LEITOR (`n-<R>`) e o ALVO (`@Worker`)
/// são membros vivos, `lina history "@Worker"` com `LINA_NODE_ID=n-<R>` tem que PASSAR da identidade
/// e RESOLVER o alvo (a "tela do worker" que o #15 prometeu). HEAD (RED): morre em "invalido" antes
/// de resolver. GREEN: identidade aceita (sem "invalido") E alvo resolvido (sem "não encontrei").
#[test]
fn history_n_prefixo_resolve_alvo_no_caminho_real() {
    let home = TempHome::new("e2e");
    // O log guarda o `node` como uuid PURO (NodeId serializa sem `n-`); o env do leitor é `n-<R>`.
    let reader = "0198a0f0-0000-7000-8000-0000000000ab";
    let worker = "0198a0f0-0000-7000-8000-0000000000cd";
    let events_dir = home.path().join("events");
    std::fs::create_dir_all(&events_dir).expect("events/");
    let log = [
        // leitor vivo (membro da fronteira de pertencimento, ADR 0006)
        format!(r#"{{"seq":1,"ts":100,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{reader}","name":"Maestro"}}}}"#),
        format!(r#"{{"seq":2,"ts":110,"kind":"TerminalSpawned","version":1,"payload":{{"event":"TerminalSpawned","node":"{reader}"}}}}"#),
        // alvo vivo
        format!(r#"{{"seq":3,"ts":120,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{worker}","name":"Worker"}}}}"#),
        format!(r#"{{"seq":4,"ts":130,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{worker}","status":"Busy","from":"Idle","reason":"pty_output"}}}}"#),
    ]
    .join("\n");
    std::fs::write(events_dir.join("log.jsonl"), log + "\n").expect("log.jsonl");

    let out = run_history(&home, &format!("n-{reader}"), &["@Worker", "--tail", "5"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !rejeitou_identidade(&out),
        "RED no HEAD: a identidade `n-<R>` do leitor é recusada antes de resolver o alvo. stderr: {err}"
    );
    assert!(
        !err.contains("não encontrei") && !err.contains("nao encontrei"),
        "com a identidade aceita, o alvo @Worker tem que RESOLVER (vivo no log). stderr: {err}"
    );
}
