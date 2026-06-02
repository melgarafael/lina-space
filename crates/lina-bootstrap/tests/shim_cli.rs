//! W3-6 **AC-6.4** (headless) — o PATH-shim (tier 2) intercepta o **comando real** antes de
//! executar. Com `.lina/bin/git` no `PATH` e o canal de confirmação STUB respondendo "não",
//! `git push --force` → o git REAL **NÃO** é exec'd: o wrapper consulta `lina guard`
//! (apenda `ActionGated` ao log) e sai com código != 0 SEM chamar o git real.
//!
//! Sentinela de não-execução: um **git falso** que escreve um arquivo quando rodado. Se o arquivo
//! NÃO aparece, o binário real não foi invocado. O caminho `allow` (ex.: `git status`) prova o
//! oposto — aí o git falso roda e escreve o sentinela —, garantindo que o shim não bloqueia tudo.
//!
//! **Furo conhecido (documentado no .entrega):** `/usr/bin/git push --force` (caminho absoluto)
//! ignora o shim — limite do tier 2 (só o hook PreToolUse é gate duro de verdade).

#![cfg(unix)]

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use lina_core::EventStore;

/// Caminho do template do shim no repo (resolvido a partir do manifesto do crate).
fn shim_template() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/lina-shim/lina-shim.sh")
        .canonicalize()
        .expect("assets/lina-shim/lina-shim.sh deve existir")
}

/// Sandbox temporária e isolada: `<base>/{home, bin, real}`. Removida no `Drop`.
struct Sandbox {
    base: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!(
            "lina-w36b-shim-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(base.join("home")).expect("home");
        std::fs::create_dir_all(base.join("bin")).expect("bin");
        std::fs::create_dir_all(base.join("real")).expect("real");
        Self { base }
    }

    fn bin(&self) -> PathBuf {
        self.base.join("bin")
    }
    fn real(&self) -> PathBuf {
        self.base.join("real")
    }
    fn home(&self) -> PathBuf {
        self.base.join("home")
    }
    fn sentinel(&self) -> PathBuf {
        self.base.join("real-git-ran.sentinel")
    }

    /// Instala o shim (`bin/lina-shim.sh` + link `bin/git`) e o **git falso** (`real/git`) que
    /// escreve o sentinela quando executado.
    fn install(&self) {
        // 1) shim genérico + link nomeado `git`.
        let shim_dst = self.bin().join("lina-shim.sh");
        std::fs::copy(shim_template(), &shim_dst).expect("copiar shim");
        make_executable(&shim_dst);
        symlink("lina-shim.sh", self.bin().join("git")).expect("link git → shim");

        // 2) git FALSO: marca execução escrevendo o sentinela (caminho absoluto embutido).
        let sentinel = self.sentinel();
        let fake = format!(
            "#!/bin/sh\nprintf 'REAL_GIT_RAN %s\\n' \"$*\" > '{}'\nexit 0\n",
            sentinel.display()
        );
        let fake_dst = self.real().join("git");
        std::fs::write(&fake_dst, fake).expect("git falso");
        make_executable(&fake_dst);
    }

    /// PATH do agente: shim ANTES do git falso; depois o dir do binário `lina` e os utilitários do SO.
    fn agent_path(&self) -> String {
        let lina_dir = Path::new(env!("CARGO_BIN_EXE_lina"))
            .parent()
            .expect("dir do binário lina")
            .to_path_buf();
        format!(
            "{}:{}:{}:/usr/bin:/bin",
            self.bin().display(),
            self.real().display(),
            lina_dir.display()
        )
    }

    /// Roda `sh -c "<cmdline>"` com o PATH do agente e o ambiente do gate. `confirm=true` injeta
    /// `LINA_CONFIRM=yes` (humano aprova); senão o stub recusa (default). Devolve o exit code.
    fn run(&self, cmdline: &str, autonomy: &str, confirm: bool) -> Option<i32> {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(cmdline)
            .env("PATH", self.agent_path())
            .env("LINA_SHIM_DIR", self.bin())
            .env("LINA_HOME", self.home())
            .env("LINA_AUTONOMY", autonomy);
        if confirm {
            cmd.env("LINA_CONFIRM", "yes");
        } else {
            cmd.env_remove("LINA_CONFIRM");
        }
        cmd.status().expect("rodar sh -c").code()
    }

    /// Quantos `ActionGated` foram apendados ao event log do `home`.
    fn action_gated_count(&self) -> usize {
        let events_dir = self.home().join("events");
        if !events_dir.join("lina.db").exists() {
            return 0;
        }
        let store = EventStore::open(events_dir).expect("abrir event store");
        store
            .events()
            .expect("ler eventos")
            .into_iter()
            .filter(|r| r.kind == "ActionGated")
            .count()
    }

    fn real_git_ran(&self) -> bool {
        self.sentinel().exists()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn make_executable(p: &Path) {
    let mut perm = std::fs::metadata(p).expect("metadata").permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(p, perm).expect("chmod +x");
}

/// AC-6.4 (núcleo): `git push --force` com stub "não" → git real NÃO roda, exit != 0,
/// e há um `ActionGated` no log (livro-razão da recusa).
#[test]
fn force_push_is_gated_real_git_not_executed() {
    let sb = Sandbox::new("deny");
    sb.install();

    let code = sb.run("git push --force origin main", "autonomo", false);

    assert!(
        !sb.real_git_ran(),
        "o git REAL não pode ter sido executado (sentinela não deve existir)"
    );
    assert_ne!(
        code,
        Some(0),
        "o shim deve sair com código != 0 ao bloquear"
    );
    assert_eq!(
        sb.action_gated_count(),
        1,
        "o gate deve ter apendado exatamente 1 ActionGated ao log"
    );
}

/// AC-6.4 (contraprova): `git status` é rotina → `allow` → o git REAL é exec'd (sentinela escrito),
/// exit 0, e nada vai ao log. Prova que o shim não bloqueia indiscriminadamente.
#[test]
fn routine_git_status_passes_through_to_real_git() {
    let sb = Sandbox::new("allow");
    sb.install();

    let code = sb.run("git status", "assistido", false);

    assert!(
        sb.real_git_ran(),
        "git status é rotina (allow) → o git real deve ter rodado (sentinela presente)"
    );
    assert_eq!(code, Some(0), "caminho allow → exit do git real (0)");
    assert_eq!(
        sb.action_gated_count(),
        0,
        "ação routine (allow) não polui o log"
    );
}

/// AC-6.4 (canal humano): a MESMA `git push --force`, mas com `LINA_CONFIRM=yes` (humano aprova),
/// passa para o git real. Mostra que o bloqueio é do STUB "não", não do shim em si.
#[test]
fn force_push_with_human_confirm_yes_reaches_real_git() {
    let sb = Sandbox::new("confirm");
    sb.install();

    let code = sb.run("git push --force origin main", "autonomo", true);

    assert!(
        sb.real_git_ran(),
        "com confirmação 'yes' o git real deve ser executado"
    );
    assert_eq!(code, Some(0), "git real (falso) sai 0");
    // O gate ainda registrou a passagem pelo gate duro (ActionGated do check-action).
    assert_eq!(
        sb.action_gated_count(),
        1,
        "mesmo aprovado, a passagem por gated-hard fica no livro-razão"
    );
}
