//! W3-6c (ADR 0004) — **broker de ação custodiada (`lina do`): a camada INQUEBRÁVEL do gate.**
//!
//! Deploy/pagamento/envio externo dependem de um **segredo** (deploy key, API key, token de
//! webhook). Se o **Lina** detém o segredo no [`SecretVault`] (W0-7) e **o agente NÃO o tem** no
//! env do seu PTY, o agente **não consegue executar a ação sozinho — por construção**. A ação vira
//! o verbo brokerado `lina do <action>`, que o supervisor executa **com o gate humano** e só ele
//! injeta o segredo.
//!
//! **Pattern-match (`guard.rs`) é a camada soft; a custódia é o piso inquebrável** (ADR 0004 §3):
//! sem o segredo, não há caminho de execução, independente de hook/shim.
//!
//! **Invariante #1 (zero LLM):** este broker é determinístico — registry estático de ações +
//! lookup no cofre + gate humano (booleano). Nenhuma interpretação semântica.
//! **Invariante #2 (local-first):** o segredo é lido do cofre e passado SÓ ao executor; **nunca**
//! entra em evento, log ou env do agente.
//!
//! O `requester` é a identidade **auto-declarada** do pedido — A3 (binding de identidade na
//! submissão) está pendente, então a autoria registrada NÃO é autenticada e **nenhum caminho de
//! auto-aprovação por identidade existe** (ADR 0004 §4): o gate humano confirma SEMPRE.

use lina_secrets::{SecretError, SecretStore, SecretVault};
use thiserror::Error;

use crate::events::{DomainEvent, EventStore, StoreError};

/// Classe canônica das ações custodiadas no evento `ActionGated`. É **broker-only**: NÃO sai do
/// `classify` de uma string de comando (o agente nunca a produz por pattern-match) — por isso vive
/// aqui, e não no enum `ActionClass` do `guard`. O gate dela é fixo: `ask` em qualquer nível (a
/// custódia é o piso que a autonomia jamais afrouxa — ADR 0004).
pub const CLASS_GATED_HARD_EXTERNAL: &str = "gated-hard-external";

/// Uma ação custodiada que o broker `lina do` media. Cada uma DEPENDE de um segredo que vive SÓ no
/// [`SecretVault`] do Lina (escopo `scope`); o agente não o tem. Registry estático (não há custódia
/// genérica para binário arbitrário — contrapartida de não usar sandbox; ADR 0004 §Consequências).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustodyAction {
    /// Verbo: `lina do <name>`.
    pub name: &'static str,
    /// Escopo do segredo no [`SecretVault`] (`vault.get(scope, key)`).
    pub scope: &'static str,
    /// Descrição legível — texto do gate humano.
    pub desc: &'static str,
}

/// Catálogo das ações externas custodiadas suportadas (ADR 0004 §3). Adicionar uma ação externa
/// nova é adicionar uma entrada aqui + o executor no supervisor (não há broker genérico).
pub const CUSTODY_ACTIONS: &[CustodyAction] = &[
    CustodyAction {
        name: "deploy",
        scope: "deploy",
        desc: "deploy a um ambiente (requer deploy key)",
    },
    CustodyAction {
        name: "pay",
        scope: "payment",
        desc: "pagamento (requer API key de pagamento)",
    },
    CustodyAction {
        name: "send",
        scope: "external",
        desc: "envio externo a terceiro (requer token)",
    },
];

/// Resolve o nome de uma ação custodiada no registry estático. `None` = ação não-custodiada
/// (o broker recusa: só medeia ações conhecidas com segredo associado).
#[must_use]
pub fn lookup_action(name: &str) -> Option<CustodyAction> {
    CUSTODY_ACTIONS.iter().copied().find(|a| a.name == name)
}

/// Pedido de uma ação custodiada, montado a partir de `lina do <action> [args]`. Carrega a
/// **referência** do segredo (escopo+chave), **nunca** o segredo em si.
#[derive(Debug, Clone)]
pub struct BrokerRequest {
    /// Nome da ação (`deploy`/`pay`/`send`).
    pub action: String,
    /// Escopo do segredo no cofre (de [`CustodyAction::scope`]).
    pub secret_scope: String,
    /// Chave do segredo dentro do escopo (ex.: o ambiente `prod` para deploy).
    pub secret_key: String,
    /// Identidade auto-declarada do requisitante (A3 pendente — não-autenticada).
    pub requester: String,
    /// Resumo legível do comando (vai para `ActionGated.cmd`; SEM o segredo).
    pub display: String,
}

impl BrokerRequest {
    /// Monta o pedido para uma [`CustodyAction`] resolvida, com a chave de segredo e o requisitante.
    #[must_use]
    pub fn new(
        action: CustodyAction,
        secret_key: impl Into<String>,
        requester: impl Into<String>,
    ) -> Self {
        let secret_key = secret_key.into();
        let action_name = action.name.to_string();
        let display = format!("lina do {action_name} [{}/{secret_key}]", action.scope);
        Self {
            action: action_name,
            secret_scope: action.scope.to_string(),
            secret_key,
            requester: requester.into(),
            display,
        }
    }
}

/// Resultado do broker. Os três caminhos provam a custódia (ADR 0004 / AC-6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerOutcome {
    /// Sem gate humano → bloqueado. A ação NÃO roda; o cofre nem é consultado.
    DeniedUnconfirmed,
    /// Confirmado, mas o segredo está AUSENTE do cofre → não há caminho de execução (custódia).
    DeniedNoSecret,
    /// Confirmado + segredo presente → executado COM o segredo do cofre.
    Executed,
}

/// Erros do broker.
#[derive(Debug, Error)]
pub enum BrokerError {
    /// `lina do <x>` para uma ação fora do registry custodiado.
    #[error("ação custodiada desconhecida: {0}")]
    UnknownAction(String),
    /// Falha ao consultar o cofre de segredos.
    #[error("cofre de segredos: {0}")]
    Secret(#[from] SecretError),
    /// Falha ao apendar ao event log.
    #[error("event store: {0}")]
    Store(#[from] StoreError),
    /// O executor da ação (com o segredo) falhou.
    #[error("execução da ação custodiada falhou: {0}")]
    Exec(String),
}

/// Executa o gate de custódia para um pedido brokerado (ADR 0004 §3) — a função-coração da W3-6c.
///
/// Fluxo determinístico:
/// 1. Valida que a ação é custodiada (registry).
/// 2. Apenda `ActionGated{class:"gated-hard-external", decision:"ask"}` — o gate SEMPRE pergunta
///    para ação externa custodiada (a custódia é o piso; nunca `allow`, em nenhum nível).
/// 3. `confirmed == false` → `BrokerDenied{unconfirmed}`; **o cofre nem é tocado** e a ação não roda.
/// 4. `confirmed == true` → o broker (não o agente) busca o segredo no cofre:
///    - ausente → `BrokerDenied{no_secret}` (prova de custódia: sem segredo, sem execução);
///    - presente → chama `execute(&secret)` e apenda `BrokerExecuted`.
///
/// O segredo é lido do `vault` e passado SÓ ao `execute` — **nunca** entra em evento ou log.
pub fn run_custody<S, F>(
    req: &BrokerRequest,
    confirmed: bool,
    vault: &SecretVault<S>,
    store: &mut EventStore,
    execute: F,
) -> Result<BrokerOutcome, BrokerError>
where
    S: SecretStore,
    F: FnOnce(&str) -> Result<(), String>,
{
    if lookup_action(&req.action).is_none() {
        return Err(BrokerError::UnknownAction(req.action.clone()));
    }

    // O gate de execução fala: ação externa custodiada → `ask` em QUALQUER nível (a custódia é o
    // piso que a autonomia jamais afrouxa — ADR 0004; nunca `allow`).
    store.append(&DomainEvent::ActionGated {
        cmd: req.display.clone(),
        class: CLASS_GATED_HARD_EXTERNAL.to_string(),
        decision: "ask".to_string(),
        // Custódia: NÃO carimba o nó. A pendência já vira item `Custody` na fila (espelho do desk);
        // um `node:Some` aqui a duplicaria como GuardAsk (FIX-2). O alerta dela é o canal existente.
        node: None,
    })?;

    // Sem confirmação humana → bloqueia. O cofre nem é consultado; a ação não roda.
    if !confirmed {
        store.append(&DomainEvent::BrokerDenied {
            action: req.action.clone(),
            reason: "unconfirmed".to_string(),
        })?;
        return Ok(BrokerOutcome::DeniedUnconfirmed);
    }

    // Confirmado: o BROKER (não o agente) obtém o segredo do cofre.
    let Some(secret) = vault.get(&req.secret_scope, &req.secret_key)? else {
        // Custódia inquebrável: sem o segredo no cofre, não há caminho de execução.
        store.append(&DomainEvent::BrokerDenied {
            action: req.action.clone(),
            reason: "no_secret".to_string(),
        })?;
        return Ok(BrokerOutcome::DeniedNoSecret);
    };

    // Executa COM o segredo do cofre (que o agente não tem no env do PTY).
    match execute(&secret) {
        Ok(()) => {
            store.append(&DomainEvent::BrokerExecuted {
                action: req.action.clone(),
                requester: req.requester.clone(),
            })?;
            Ok(BrokerOutcome::Executed)
        }
        Err(e) => {
            store.append(&DomainEvent::BrokerDenied {
                action: req.action.clone(),
                reason: "exec_failed".to_string(),
            })?;
            Err(BrokerError::Exec(e))
        }
    }
}

// ───────────────────────────── F3-5-8 · poda de disco custodiada (ADR 0043) ──────────────────────

/// Classe do gate de uma poda de disco — ação destrutiva LOCAL (apagar bytes do usuário), família
/// gated-hard (igual `git push`/`rm -rf`, #19). Distinta de [`CLASS_GATED_HARD_EXTERNAL`]: deploy/
/// pay/send dependem de um SEGREDO no cofre; a poda depende do GESTO humano (não há segredo a injetar).
pub const CLASS_GATED_HARD_DISK: &str = "gated-hard-disk";

/// Desfecho de uma poda custodiada (F3-5-8, ADR 0043). Prova a salvaguarda inegociável: SEM o gesto
/// (`confirmed=false`) → zero bytes, zero `DiskReclaimExecuted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskReclaimOutcome {
    /// Sem o gesto humano → bloqueado. A poda NÃO roda; nada é apagado.
    DeniedUnconfirmed,
    /// Gesto dado + poda executada → bytes recuperados (auditado em `DiskReclaimExecuted`).
    Executed {
        /// Bytes recuperados pela poda (do executor).
        reclaimed_bytes: u64,
    },
}

/// **A execução custodiada da poda de disco (F3-5-8, ADR 0043) — a porta IRREVERSÍVEL.**
///
/// Salvaguarda inegociável (ADR 0043 §2): **zero `DiskReclaimExecuted` sem o gesto humano**.
/// `approved_by` é EXIBIÇÃO (humaniza quem aprovou na tela), NUNCA autoridade — a autoridade é o
/// `confirmed` (o gesto custodiado `AttentionKind::Custody`), jamais um campo de payload (ADR 0007).
/// Forjar `DiskReclaimApproved` no log NÃO passa por aqui ⇒ não dispara poda alguma.
///
/// Fluxo (espelha a doutrina de [`run_custody`], mas a ação é LOCAL e destrutiva — sem segredo):
/// 1. Apenda `ActionGated{class:"gated-hard-disk", decision:"ask"}` — o gate de uma poda fala SEMPRE
///    `ask` (custódia é o piso; nunca `allow`, em nenhum nível).
/// 2. `confirmed == false` → apenda `BrokerDenied{disk_reclaim, unconfirmed}`; o executor NÃO roda.
/// 3. `confirmed == true` → apenda `DiskReclaimApproved` (gate passou; `approved_by` p/ exibição) →
///    chama `execute` (a poda real `rm`/`cargo clean`, feita pelo supervisor FORA do core) →
///    apenda `DiskReclaimExecuted{reclaimed_bytes}` (Ok) ou `BrokerDenied{exec_failed}` (Err).
///
/// `execute` recebe os caminhos aprovados e devolve os bytes recuperados — é o ÚNICO ponto que toca
/// o disco; o worker NÃO o exercita com poda real (lição #36), nos testes é injetado.
///
/// # Errors
/// [`BrokerError::Store`] se o append ao log falhar; [`BrokerError::Exec`] se a poda (`execute`) falhar.
pub fn run_disk_reclaim<F>(
    candidate_paths: &[String],
    approved_by: &str,
    confirmed: bool,
    store: &mut EventStore,
    execute: F,
) -> Result<DiskReclaimOutcome, BrokerError>
where
    F: FnOnce(&[String]) -> Result<u64, String>,
{
    // O gate de uma poda fala SEMPRE `ask` (a custódia é o piso que a autonomia jamais afrouxa).
    // `node:None`: a pendência já é item `Custody` (de `DiskReclaimProposed`, attention.rs); um
    // `node:Some` aqui a duplicaria como GuardAsk (mesmo motivo do `run_custody`).
    store.append(&DomainEvent::ActionGated {
        cmd: format!("lina disk reclaim [{} caminho(s)]", candidate_paths.len()),
        class: CLASS_GATED_HARD_DISK.to_string(),
        decision: "ask".to_string(),
        node: None,
    })?;

    // Sem o gesto humano → bloqueia. O executor NÃO roda; nada é apagado; nenhum `DiskReclaimApproved`.
    if !confirmed {
        store.append(&DomainEvent::BrokerDenied {
            action: "disk_reclaim".to_string(),
            reason: "unconfirmed".to_string(),
        })?;
        return Ok(DiskReclaimOutcome::DeniedUnconfirmed);
    }

    // Gesto dado: o gate PASSOU. `approved_by` é só exibição; a autoridade foi o `confirmed`.
    store.append(&DomainEvent::DiskReclaimApproved {
        candidate_paths: candidate_paths.to_vec(),
        approved_by: approved_by.to_string(),
        approved_at_ms: 0,
    })?;

    // A poda real (rm/cargo clean) — feita pelo executor injetado (supervisor), FORA do core.
    match execute(candidate_paths) {
        Ok(reclaimed_bytes) => {
            store.append(&DomainEvent::DiskReclaimExecuted {
                reclaimed_bytes,
                paths: candidate_paths.to_vec(),
                executed_at_ms: 0,
            })?;
            Ok(DiskReclaimOutcome::Executed { reclaimed_bytes })
        }
        Err(e) => {
            store.append(&DomainEvent::BrokerDenied {
                action: "disk_reclaim".to_string(),
                reason: "exec_failed".to_string(),
            })?;
            Err(BrokerError::Exec(e))
        }
    }
}

// ───────────────────────────────────── testes (AC-6.5, headless com MockStore) ──────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lina_secrets::MockStore;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Diretório temporário do event store; removido no `Drop`.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let p = std::env::temp_dir()
                .join(format!("lina-w36c-{tag}-{}-{nanos}", std::process::id()));
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

    fn vault_with_deploy_token() -> SecretVault<MockStore> {
        let v = SecretVault::with_store("lina-space/ws-teste", MockStore::new());
        v.set("deploy", "prod", "DEPLOY-KEY-SUPER-SECRETA")
            .expect("semear token");
        v
    }

    fn deploy_req() -> BrokerRequest {
        let action = lookup_action("deploy").expect("deploy é custodiada");
        BrokerRequest::new(action, "prod", "@Dev")
    }

    fn kinds(store: &EventStore) -> Vec<String> {
        store
            .events()
            .expect("ler eventos")
            .into_iter()
            .map(|r| r.kind)
            .collect()
    }

    /// AC-6.5 (1): token SÓ no cofre; `deploy` SEM confirmação → bloqueia + ActionGated
    /// {class:"gated-hard-external", decision:"ask"}; a ação real NÃO roda (executor não chamado).
    #[test]
    fn unconfirmed_blocks_and_logs_gated_hard_external() {
        let tmp = TempDir::new("unconfirmed");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let ran = Cell::new(false);

        let outcome = run_custody(&deploy_req(), false, &vault, &mut store, |_secret| {
            ran.set(true);
            Ok(())
        })
        .expect("run_custody");

        assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
        assert!(!ran.get(), "o executor NÃO pode rodar sem confirmação");

        let events = store.events().expect("eventos");
        let gated = events
            .iter()
            .find(|r| r.kind == "ActionGated")
            .expect("ActionGated");
        assert_eq!(gated.payload["class"], "gated-hard-external");
        assert_eq!(gated.payload["decision"], "ask");
        assert!(
            events
                .iter()
                .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "unconfirmed"),
            "deve apendar BrokerDenied{{unconfirmed}}"
        );
        assert!(
            !events.iter().any(|r| r.kind == "BrokerExecuted"),
            "NUNCA pode haver BrokerExecuted sem confirmação"
        );
    }

    /// AC-6.5 (2): COM confirmação humana, o broker obtém o segredo DO COFRE (prova: o executor
    /// recebe exatamente o valor do vault, que o agente não forneceu) e executa.
    #[test]
    fn confirmed_fetches_secret_from_vault_and_executes() {
        let tmp = TempDir::new("confirmed");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let seen_secret: Cell<Option<String>> = Cell::new(None);

        let outcome = run_custody(&deploy_req(), true, &vault, &mut store, |secret| {
            seen_secret.set(Some(secret.to_string()));
            Ok(())
        })
        .expect("run_custody");

        assert_eq!(outcome, BrokerOutcome::Executed);
        assert_eq!(
            seen_secret.take().as_deref(),
            Some("DEPLOY-KEY-SUPER-SECRETA"),
            "o segredo passado ao executor deve vir DO COFRE, não do agente"
        );

        let events = store.events().expect("eventos");
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerExecuted" && r.payload["action"] == "deploy"));
        // O segredo NUNCA pode vazar para o log (invariante #2).
        let log_dump =
            serde_json::to_string(&events.iter().map(|r| &r.payload).collect::<Vec<_>>())
                .expect("serializar log");
        assert!(
            !log_dump.contains("DEPLOY-KEY-SUPER-SECRETA"),
            "o segredo NÃO pode aparecer em nenhum evento do log"
        );
    }

    /// AC-6.5 (3) — prova de custódia: confirmado, mas segredo AUSENTE do cofre → não há caminho de
    /// execução (executor não chamado, BrokerDenied{no_secret}). Sem segredo + sem agente possuí-lo,
    /// a ação depende inteiramente do broker — e o broker não tem o que injetar.
    #[test]
    fn confirmed_without_secret_in_vault_cannot_execute() {
        let tmp = TempDir::new("nosecret");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        // Cofre VAZIO — o token não existe em lugar nenhum (nem no agente, nem no vault).
        let vault: SecretVault<MockStore> =
            SecretVault::with_store("lina-space/ws-teste", MockStore::new());
        let ran = Cell::new(false);

        let outcome = run_custody(&deploy_req(), true, &vault, &mut store, |_secret| {
            ran.set(true);
            Ok(())
        })
        .expect("run_custody");

        assert_eq!(outcome, BrokerOutcome::DeniedNoSecret);
        assert!(!ran.get(), "sem segredo no cofre, o executor NÃO roda");
        let events = store.events().expect("eventos");
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "no_secret"));
        assert!(!events.iter().any(|r| r.kind == "BrokerExecuted"));
    }

    /// Ação fora do registry custodiado é recusada (o broker só medeia ações conhecidas).
    #[test]
    fn unknown_action_is_rejected_without_touching_log() {
        let tmp = TempDir::new("unknown");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let req = BrokerRequest {
            action: "rm-the-universe".to_string(),
            secret_scope: "deploy".to_string(),
            secret_key: "prod".to_string(),
            requester: "@Dev".to_string(),
            display: "lina do rm-the-universe".to_string(),
        };
        let err = run_custody(&req, true, &vault, &mut store, |_| Ok(())).unwrap_err();
        assert!(matches!(err, BrokerError::UnknownAction(_)));
        assert!(
            kinds(&store).is_empty(),
            "ação desconhecida não apenda evento"
        );
    }

    /// O gate de custódia é sempre `ask` (piso): toda execução brokerada apenda
    /// `ActionGated{class:"gated-hard-external", decision:"ask"}`, nunca `allow`.
    #[test]
    fn custody_gate_is_always_ask() {
        let tmp = TempDir::new("alwaysask");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        run_custody(&deploy_req(), true, &vault, &mut store, |_| Ok(())).expect("run");
        let events = store.events().expect("eventos");
        let gated = events
            .iter()
            .find(|r| r.kind == "ActionGated")
            .expect("ActionGated");
        assert_eq!(gated.payload["class"], CLASS_GATED_HARD_EXTERNAL);
        assert_eq!(gated.payload["decision"], "ask");
    }

    // ───────────────────── F3-5-8 · poda de disco custodiada (ADR 0043) ─────────────────────

    fn target_paths() -> Vec<String> {
        vec!["/ws/target".to_string()]
    }

    /// Critério (e) — "zero bytes sem aprovação": SEM o gesto, o executor NÃO roda e nenhum
    /// `DiskReclaimExecuted`/`DiskReclaimApproved` é apendado; só o gate (`ask`) + `BrokerDenied`.
    #[test]
    fn disk_reclaim_unconfirmed_deletes_nothing() {
        let tmp = TempDir::new("disk-unconfirmed");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let ran = Cell::new(false);

        let outcome = run_disk_reclaim(&target_paths(), "@Fundador", false, &mut store, |_paths| {
            ran.set(true);
            Ok(30_000_000_000)
        })
        .expect("run_disk_reclaim");

        assert_eq!(outcome, DiskReclaimOutcome::DeniedUnconfirmed);
        assert!(
            !ran.get(),
            "o executor (a poda) NÃO pode rodar sem o gesto humano"
        );

        let events = store.events().expect("eventos");
        let gated = events
            .iter()
            .find(|r| r.kind == "ActionGated")
            .expect("ActionGated");
        assert_eq!(gated.payload["class"], CLASS_GATED_HARD_DISK);
        assert_eq!(gated.payload["decision"], "ask");
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "unconfirmed"));
        assert!(
            !events.iter().any(|r| r.kind == "DiskReclaimExecuted"),
            "NUNCA pode haver DiskReclaimExecuted sem o gesto"
        );
        assert!(
            !events.iter().any(|r| r.kind == "DiskReclaimApproved"),
            "sem o gesto não há nem DiskReclaimApproved"
        );
    }

    /// Critério (e) — "com gesto custodiado → DiskReclaimExecuted{reclaimed_bytes>0}": o gesto
    /// produz `DiskReclaimApproved` ANTES de `DiskReclaimExecuted` (zero Executed sem Approved), e o
    /// executor recebe os caminhos aprovados.
    #[test]
    fn disk_reclaim_confirmed_approves_then_executes() {
        let tmp = TempDir::new("disk-confirmed");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let seen: Cell<Option<usize>> = Cell::new(None);

        let outcome = run_disk_reclaim(&target_paths(), "@Fundador", true, &mut store, |paths| {
            seen.set(Some(paths.len()));
            Ok(30_000_000_000)
        })
        .expect("run_disk_reclaim");

        assert_eq!(
            outcome,
            DiskReclaimOutcome::Executed {
                reclaimed_bytes: 30_000_000_000
            }
        );
        assert_eq!(
            seen.get(),
            Some(1),
            "o executor recebe os caminhos aprovados"
        );

        let ks = kinds(&store);
        let approved = ks.iter().position(|k| k == "DiskReclaimApproved");
        let executed = ks.iter().position(|k| k == "DiskReclaimExecuted");
        assert!(approved.is_some() && executed.is_some());
        assert!(
            approved < executed,
            "DiskReclaimApproved deve PRECEDER DiskReclaimExecuted (zero Executed sem Approved)"
        );

        let events = store.events().expect("eventos");
        let exec = events
            .iter()
            .find(|r| r.kind == "DiskReclaimExecuted")
            .expect("DiskReclaimExecuted");
        assert_eq!(exec.payload["reclaimed_bytes"], 30_000_000_000_u64);
    }

    /// Poda autorizada mas a EXECUÇÃO falha: o gesto fica registrado (`Approved`), mas NÃO há
    /// `DiskReclaimExecuted` — só `BrokerDenied{exec_failed}`. Honesto e auditável.
    #[test]
    fn disk_reclaim_exec_failure_leaves_no_executed() {
        let tmp = TempDir::new("disk-execfail");
        let mut store = EventStore::open(tmp.path()).expect("open store");

        let err = run_disk_reclaim(&target_paths(), "@Fundador", true, &mut store, |_paths| {
            Err("rm falhou: permissão negada".to_string())
        })
        .unwrap_err();

        assert!(matches!(err, BrokerError::Exec(_)));
        let events = store.events().expect("eventos");
        assert!(events.iter().any(|r| r.kind == "DiskReclaimApproved"));
        assert!(
            !events.iter().any(|r| r.kind == "DiskReclaimExecuted"),
            "execução falha NÃO produz DiskReclaimExecuted"
        );
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "exec_failed"));
    }

    /// MUTAÇÃO da salvaguarda (família ADR 0007): um `DiskReclaimApproved` forjado DIRETO no log
    /// (sem passar por `run_disk_reclaim`) é inerte — `run_disk_reclaim` é o ÚNICO produtor de
    /// `DiskReclaimExecuted`, e ele exige `confirmed`. O campo de payload jamais executa a poda.
    #[test]
    fn forged_approved_by_in_log_executes_nothing() {
        let tmp = TempDir::new("disk-forged");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        // O agente forja o gate escrevendo o campo direto no log:
        store
            .append(&DomainEvent::DiskReclaimApproved {
                candidate_paths: target_paths(),
                approved_by: "atacante".to_string(),
                approved_at_ms: 0,
            })
            .expect("append forjado");
        // Nenhuma poda nasce de um campo de payload:
        assert!(
            !kinds(&store).iter().any(|k| k == "DiskReclaimExecuted"),
            "forjar approved_by NÃO dispara poda — só o gesto via run_disk_reclaim a produz"
        );
    }
}
