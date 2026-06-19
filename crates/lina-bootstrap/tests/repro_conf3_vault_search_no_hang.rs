//! **F3-CONF-3 · R3 (QA/RED) — repro do achado #21/#21c**: `lina vault search` PENDURA em vault
//! grande (50min57s sem 1 byte, medido — pid 85700).
//!
//! `vault_search()` (`lina.rs:3361`) varre TODO o vault recursivamente, síncrono, lendo cada `.md`
//! linha-a-linha, SEM teto de tempo/arquivos (o `VAULT_SEARCH_MAX_HITS=40` limita HITS, não a
//! varredura). Em vault grande (o cenário real do fundador) o verbo de contexto do "segundo cérebro"
//! fica inutilizável (gate (c) da onda-f3-conf-3).
//!
//! **Estratégia de teste (determinística, não wall-clock):** o fix do H deve impor um **teto de
//! ARQUIVOS VISITADOS** (mais determinístico no CI que tempo — alinhado no despacho R1/§(c) e com o H)
//! e, ao estourar, emitir uma **parada honesta** ("parei em N arquivos — …"). Então:
//!
//!   - vault GRANDE + termo com ZERO hits → o teto de 40 *resultados* do HEAD NUNCA dispara; logo
//!     "parei" só pode vir do teto de ARQUIVOS (o fix). RED no HEAD (varre tudo, imprime
//!     "(sem resultados…)" sem marcador) → GREEN com o fix (parada honesta).
//!   - vault PEQUENO → resultados COMPLETOS, sem truncar (não-regressão; verde no HEAD e no fix).
//!
//! Todo subprocess roda sob um GUARD DE TIMEOUT que MATA o processo se pendurar — assim o achado #21
//! nunca trava o `cargo test` (na pior hipótese o teste falha rápido por timeout, não pendura 50min).
//!
//! NOTA AO MAESTRO: o discriminador "parei" assume que o fix reusa o vocabulário de parada honesta
//! ("parei em N arquivos…", sugerido no próprio despacho R1) e um teto de ARQUIVOS. Se o H escolher
//! teto de TEMPO, 5000 arquivos minúsculos varrem em <1s e o teto não dispara — então o knob deve ser
//! por arquivos (recomendado ao H) OU o marcador deve ser ajustado. Coordenado com o H via `lina ask`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Mais arquivos do que qualquer teto de varredura plausível — para FORÇAR a parada honesta do fix.
/// (Deve exceder o teto de arquivos do H; coordenado via `lina ask`.)
const ARQUIVOS_VAULT_GRANDE: usize = 5000;

/// Teto wall-clock só como GUARD anti-pendura do test-runner (não é o critério — o critério é a
/// parada honesta). Folgado: a varredura de 5000 arquivos minúsculos é <1s; 30s só pega um HANG real.
const GUARD_TIMEOUT: Duration = Duration::from_secs(30);

struct TempHome(PathBuf);

impl TempHome {
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
            "lina-conf3-vault-{tag}-{}-{tid}-{nanos}",
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

/// Escreve `vault.json` (formato de `obsidian.rs::write_vault_config`) apontando para `vault_dir`.
fn seed_vault_config(home: &TempHome, vault_dir: &Path) {
    let p = vault_dir.to_string_lossy().replace('\\', "/");
    let cfg = format!(r#"{{"primary":"{p}","vaults":[{{"name":"test","path":"{p}"}}]}}"#);
    std::fs::write(home.path().join("vault.json"), cfg).expect("vault.json");
}

/// Gera `n` notas `.md` (espalhadas em subpastas p/ não estourar um único diretório), nenhuma
/// contendo `termo_proibido` — assim o teto de 40 HITS do HEAD nunca dispara.
fn gerar_vault_grande(vault_dir: &Path, n: usize, termo_proibido: &str) {
    for i in 0..n {
        let sub = vault_dir.join(format!("d{:03}", i / 100));
        let _ = std::fs::create_dir_all(&sub);
        let conteudo =
            format!("# nota {i}\nlinha de preenchimento {i}\nmais contexto sem o termo buscado\n");
        debug_assert!(!conteudo.contains(termo_proibido));
        std::fs::write(sub.join(format!("n{i}.md")), conteudo).expect("nota .md");
    }
}

/// Roda `lina vault search <termo>` sob guard de timeout: se o processo não terminar em
/// `GUARD_TIMEOUT`, é MORTO e devolvemos `None` (pendura = falha, mas NUNCA trava o runner).
fn search_com_timeout(home: &TempHome, termo: &str) -> Option<(std::process::Output, Duration)> {
    let exe = env!("CARGO_BIN_EXE_lina");
    let mut child: Child = Command::new(exe)
        .args(["vault", "search", termo])
        .env("LINA_HOME", home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lina vault search");

    let inicio = Instant::now();
    loop {
        if let Some(_status) = child.try_wait().expect("try_wait") {
            let out = child.wait_with_output().expect("colher saída");
            return Some((out, inicio.elapsed()));
        }
        if inicio.elapsed() >= GUARD_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return None; // PENDUROU — o achado #21 reproduzido (e o runner foi protegido).
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `true` se a saída sinalizou uma PARADA HONESTA por teto de varredura (o marcador do fix). Casa o
/// verbo de parada da casa ("parei") OU uma menção explícita a teto/limite de arquivos — SEM casar a
/// mensagem de zero-resultados do HEAD (que cita "lina vault index" mas nunca "parei"/"teto").
fn parada_honesta(saida: &str) -> bool {
    let s = saida.to_lowercase();
    s.contains("parei")
        || (s.contains("arquivo")
            && (s.contains("teto") || s.contains("limite") || s.contains("parcial")))
}

/// **#21 — não pendura + parada honesta em vault grande.** RED no HEAD: varre os 5000 arquivos e
/// imprime "(sem resultados…)" SEM marcador de parada → `parada_honesta` falsa. GREEN com o fix: o
/// teto de arquivos dispara → "parei em N arquivos…". Em AMBOS o guard garante que não pendura o
/// runner. QUEBRA SE… o fix removesse a parada honesta (varreria tudo de novo) → marcador ausente.
#[test]
fn vault_search_grande_para_honesto_e_nao_pendura() {
    let home = TempHome::new("grande");
    let vault_dir = home.path().join("vault-obsidian");
    std::fs::create_dir_all(&vault_dir).expect("vault dir");
    let termo = "zzqxraretokenausente";
    gerar_vault_grande(&vault_dir, ARQUIVOS_VAULT_GRANDE, termo);
    seed_vault_config(&home, &vault_dir);

    let Some((out, dur)) = search_com_timeout(&home, termo) else {
        panic!(
            "#21 reproduzido: `lina vault search` PENDUROU >{}s em vault de {} arquivos (morto pelo \
             guard). É exatamente o hang do achado #21 — o fix do H tem que impor o teto.",
            GUARD_TIMEOUT.as_secs(),
            ARQUIVOS_VAULT_GRANDE
        );
    };
    let txt = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "vault search deve sair com sucesso (saída parcial honesta, não erro). saída: {txt}"
    );
    assert!(
        parada_honesta(&txt),
        "RED no HEAD: sem teto de ARQUIVOS, varre os {ARQUIVOS_VAULT_GRANDE} e não dá parada honesta \
         (retornou em {dur:?}). Esperava marcador tipo 'parei em N arquivos…'. saída: {txt}"
    );
}

/// **#21 — CONTRASTE (não-regressão): vault PEQUENO devolve resultados COMPLETOS.** O fix não pode
/// truncar o caso comum. 2 notas casam o termo → as 2 saem; nenhuma parada honesta prematura. Verde
/// no HEAD e no fix — prova que o teto não estraga o comportamento atual em vault pequeno.
#[test]
fn vault_search_pequeno_continua_completo() {
    let home = TempHome::new("pequeno");
    let vault_dir = home.path().join("vault-obsidian");
    std::fs::create_dir_all(&vault_dir).expect("vault dir");
    let termo = "agulhanopalheiro";
    std::fs::write(
        vault_dir.join("a.md"),
        format!("# a\numa linha com {termo} aqui\n"),
    )
    .expect("a.md");
    std::fs::write(
        vault_dir.join("b.md"),
        format!("# b\noutra linha com {termo} ali\n"),
    )
    .expect("b.md");
    std::fs::write(vault_dir.join("c.md"), "# c\nsem o termo\n").expect("c.md");
    seed_vault_config(&home, &vault_dir);

    let Some((out, _dur)) = search_com_timeout(&home, termo) else {
        panic!("vault pequeno não pode pendurar");
    };
    assert!(out.status.success(), "sucesso em vault pequeno");
    let txt = String::from_utf8_lossy(&out.stdout);
    let hits = txt.matches(termo).count();
    assert!(
        hits >= 2,
        "controle: os 2 hits do vault pequeno saem COMPLETOS (não truncados pelo teto). saída: {txt}"
    );
    assert!(
        !parada_honesta(&txt),
        "controle: vault pequeno NÃO dispara parada honesta (não há o que truncar). saída: {txt}"
    );
}
