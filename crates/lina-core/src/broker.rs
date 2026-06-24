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

use crate::channel::ChannelManifest;
use crate::events::{DomainEvent, EventStore, StoreError};
use crate::router::AutonomyLevel;

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

/// `true` se o pedido é uma ação custodiada conhecida — ação de **CANAL** (`channel:Some`, validada
/// upstream contra o manifesto) **OU** builtin do registry estático ([`CUSTODY_ACTIONS`]). A negação
/// é [`BrokerError::UnknownAction`]: o broker só medeia ações que dependem de um segredo do cofre.
/// **Fonte única** da regra, compartilhada por [`run_custody`] e [`run_webhook_custody`] — não
/// duplicar a condição nos dois pontos de entrada.
#[must_use]
pub fn is_custody_action(req: &BrokerRequest) -> bool {
    req.channel.is_some() || lookup_action(&req.action).is_some()
}

/// Prefixo do escopo de segredo de um **canal** no [`SecretVault`] (F4-0-2): a credencial de um canal
/// vive em `vault.get("channel:<nome>", key)`. É a MESMA convenção que `set_channel_credential`
/// (F40-CRED) usa ao gravar — fonte única do prefixo para o broker não divergir do cofre.
pub const CHANNEL_SECRET_SCOPE_PREFIX: &str = "channel:";

/// Pedido de uma ação custodiada, montado a partir de `lina do <action> [args]` (forma legada) ou
/// `lina do <canal> <ação> [args]` (forma de canal, F4-0-3). Carrega a **referência** do segredo
/// (escopo+chave), **nunca** o segredo em si.
#[derive(Debug, Clone)]
pub struct BrokerRequest {
    /// Nome da ação. Builtin: `deploy`/`pay`/`send`. Canal: a ação declarada no manifesto (`send`/…).
    pub action: String,
    /// Escopo do segredo no cofre. Builtin: de [`CustodyAction::scope`]. Canal: `channel:<nome>`.
    pub secret_scope: String,
    /// Chave do segredo dentro do escopo (ex.: o ambiente `prod` para deploy; o token p/ um canal).
    pub secret_key: String,
    /// Identidade auto-declarada do requisitante (A3 pendente — não-autenticada).
    pub requester: String,
    /// Resumo legível do comando (vai para `ActionGated.cmd`; SEM o segredo).
    pub display: String,
    /// `Some(canal)` = ação de um CANAL (F4-0-3), já validada upstream contra o manifesto do
    /// [`ChannelRegistry`](crate::channel) (canal declarado? ação no `scopes`/`tools`?). `None` =
    /// ação builtin do registry estático [`CUSTODY_ACTIONS`]. O discriminante diz a [`run_custody`]
    /// QUAL validação aplicar — a custódia (gate humano + segredo no cofre) é idêntica nos dois casos.
    pub channel: Option<String>,
}

impl BrokerRequest {
    /// Monta o pedido para uma [`CustodyAction`] **builtin** (registry estático), com a chave de
    /// segredo e o requisitante.
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
            channel: None,
        }
    }

    /// Monta o pedido para uma ação de **CANAL** (F4-0-3): `lina do <canal> <ação>`. O escopo do
    /// segredo é derivado do canal (`channel:<nome>`) — **server-side**, nunca ditado pelo agente
    /// (um agente que escolhesse o escopo apontaria para o segredo de outro canal). A validação
    /// "canal declarado + ação no manifesto" é feita por quem chama (contra o [`ChannelRegistry`]);
    /// aqui a ação NÃO passa pelo registry estático de builtins.
    #[must_use]
    pub fn for_channel(
        channel: impl Into<String>,
        action: impl Into<String>,
        secret_key: impl Into<String>,
        requester: impl Into<String>,
    ) -> Self {
        let channel = channel.into();
        let action = action.into();
        let secret_key = secret_key.into();
        let secret_scope = format!("{CHANNEL_SECRET_SCOPE_PREFIX}{channel}");
        let display = format!("lina do {channel} {action} [{secret_scope}/{secret_key}]");
        Self {
            action,
            secret_scope,
            secret_key,
            requester: requester.into(),
            display,
            channel: Some(channel),
        }
    }
}

/// `true` se a ação de canal está DECLARADA no manifesto (F4-0-1) — em `scopes` ou
/// `tools.default_enabled`. **Default-deny:** ação não-declarada → `false` (o supervisor recusa
/// antes de montar o pedido). É a tradução de "a ação no manifesto (`scopes`/`tools`)" do gate (b):
/// o manifesto é DADO (lista o que o canal expõe), jamais autoridade — quem libera o efeito é a
/// custódia (gate humano + segredo), não este check. O SUPERVISOR a chama ao resolver `<canal> →
/// manifesto` ([`ChannelManifest::load`] via `manifest_ref` do [`ChannelRegistry`](crate::channel)),
/// antes de [`BrokerRequest::for_channel`] — o lado-agente não carrega o manifesto (I/O não-confiável).
#[must_use]
pub fn channel_action_in_manifest(manifest: &ChannelManifest, action: &str) -> bool {
    manifest.scopes.iter().any(|s| s == action)
        || manifest.tools.default_enabled.iter().any(|t| t == action)
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
    // Builtin (deploy/pay/send): a ação TEM que estar no registry estático. Canal (`channel:Some`):
    // a validação "canal declarado + ação no manifesto" já ocorreu upstream (contra o
    // `ChannelRegistry`); o broker não re-valida contra os builtins. A custódia abaixo é idêntica:
    // mesmo com canal/ação forjados, sem o segredo no escopo `channel:<nome>` nada executa (no_secret).
    if !is_custody_action(req) {
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

// ───────────────────── F4-WA-4 · gate de ação irreversível por webhook (ADR 0035 §3 · 0004) ──────

/// Contexto do canal **webhook** para o gate de uma ação irreversível disparada por um disparo
/// externo. Agrupa o que distingue o gate-de-webhook do gate normal de custódia — sem inflar a
/// assinatura de [`run_webhook_custody`]. Todos os campos são metadados de prova; **nenhum** é
/// autoridade (a autoridade é o gate humano da custódia, ADR 0004).
#[derive(Debug, Clone, Copy)]
pub struct WebhookGateContext<'a> {
    /// `hook_id` opaco do webhook que disparou (carimbado server-side; vai ao log como prova).
    pub webhook_id: &'a str,
    /// Terminal dono que recebeu o input (`target_node` por NOME — ADR 0026).
    pub target_node: &'a str,
    /// Nível de AÇÃO declarado na config do webhook: `"autonomous"` | `"notify_before"`
    /// (`WebhookConfigured.autonomy_level`). Vocabulário do canal — distinto da escala do workspace.
    pub webhook_level: &'a str,
    /// Autonomia VIGENTE do workspace (`RouterConfig::autonomy`). O `min` com o nível do webhook é o
    /// `effective_level` — o webhook nunca afrouxa o que a autonomia do Espaço já fecha (ADR 0035 §3).
    pub workspace: AutonomyLevel,
}

/// Rank da escala de autonomia, do mais RESTRITIVO ao mais LIVRE: `Manual(0) < Assisted(1) <
/// Autonomous(2)`. Base do `min` de [`webhook_effective_level`]. Local ao broker (não em `router`,
/// território de WA1) — `AutonomyLevel` é importado, jamais alterado.
fn autonomy_rank(level: AutonomyLevel) -> u8 {
    match level {
        AutonomyLevel::Manual => 0,
        AutonomyLevel::Assisted => 1,
        AutonomyLevel::Autonomous => 2,
    }
}

/// Mapeia o nível de AÇÃO do webhook (vocabulário do canal) para a escala [`AutonomyLevel`].
/// `"autonomous"` → `Autonomous` ("pode agir"); QUALQUER outro valor — `"notify_before"`, vazio ou
/// desconhecido — → `Assisted` ("me avisa antes"/propõe). O default conservador é deliberado: um
/// nível não-reconhecido NUNCA fabrica autonomia (espelha `default_notify_before` da config).
/// `parse_autonomy` (guard.rs) NÃO serve aqui — ela fala `manual|assistido|autonomo`, não o
/// vocabulário do canal webhook.
fn webhook_level_rank(level: &str) -> AutonomyLevel {
    match level.trim().to_lowercase().as_str() {
        "autonomous" => AutonomyLevel::Autonomous,
        _ => AutonomyLevel::Assisted,
    }
}

/// Serializa o nível efetivo de volta ao vocabulário de AÇÃO do canal webhook (o que a config/UI e a
/// skill consomem): `Autonomous` → `"autonomous"` ("age"); `Manual`/`Assisted` → `"notify_before"`
/// ("propõe/avisa antes"). Manual e Assisted colapsam porque, para a ação do webhook, ambos
/// significam "não age sozinho" — a distinção deles é de DELEGAÇÃO (`blocks_delegation`), não da
/// ação custodiada (que é gated de qualquer forma).
fn webhook_level_label(level: AutonomyLevel) -> &'static str {
    match level {
        AutonomyLevel::Autonomous => "autonomous",
        AutonomyLevel::Manual | AutonomyLevel::Assisted => "notify_before",
    }
}

/// **O nível EFETIVO de um webhook = `min(webhook.autonomy_level, workspace.autonomy)`** (ADR 0035
/// §3) — função pura, sem efeito no log. O webhook **nunca afrouxa** o que a autonomia do Espaço já
/// fecha: um webhook `autonomous` num Espaço `assistido` resulta `Assisted` (propõe, não age — RT-4);
/// um webhook `notify_before` num Espaço `autonomo` resulta `Assisted` (o lado conservador vence). A
/// string canônica do resultado (vocabulário do canal) sai por [`webhook_level_label`].
#[must_use]
pub fn webhook_effective_level(webhook_level: &str, workspace: AutonomyLevel) -> AutonomyLevel {
    let webhook = webhook_level_rank(webhook_level);
    if autonomy_rank(webhook) <= autonomy_rank(workspace) {
        webhook
    } else {
        workspace
    }
}

/// **Gate de uma ação irreversível DISPARADA por webhook (F4-WA-4, ADR 0035 §3 · ADR 0004).**
///
/// Webhook é um disparo externo e automático — a classe que MAIS precisa do gate. Esta função é o
/// marcador do canal por cima da custódia: ela calcula o `effective_level`, emite
/// [`DomainEvent::WebhookActionGated`] (prova auditável de que o webhook **não furou** o gate) e
/// **delega a custódia INTEIRA a [`run_custody`]** — `ActionGated{ask}` + `BrokerDenied`/
/// `BrokerExecuted`, sem reinventá-los.
///
/// **O `effective_level` NÃO afrouxa esta ação.** Para `gated-hard-external`, o gate é fixo `ask` em
/// QUALQUER nível (piso ADR 0004 — a autonomia afrouxa `gated-soft`, jamais o externo irreversível);
/// o `effective_level` é registrado como prova/contexto e é o mesmo valor que um caminho `gated-soft`
/// consultaria. Por isso ele entra no `WebhookActionGated`, mas não desvia o fluxo de `run_custody`.
///
/// Fluxo:
/// 1. Ação não-custodiada ([`is_custody_action`] falso) → `UnknownAction` SEM tocar o log (não há
///    caminho de execução externo, logo nada que o webhook "não furou" — não emite o marcador).
/// 2. Apenda `WebhookActionGated{webhook_id, target_node, action_class:"gated-hard-external",
///    effective_level}` — só metadados; NUNCA o payload externo nem o segredo (ADR 0035 §5).
/// 3. Delega a [`run_custody`]: `confirmed == false` → `DeniedUnconfirmed` (0 execução sem "sim");
///    `confirmed == true` + segredo no cofre → `Executed` (a custódia mediou, com o gate humano).
///
/// # Errors
/// [`BrokerError::UnknownAction`] se a ação não for custodiada; demais erros propagam de [`run_custody`].
pub fn run_webhook_custody<S, F>(
    ctx: &WebhookGateContext,
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
    // Espelha a recusa de `run_custody`: sem ação custodiada não há efeito externo (sem segredo),
    // logo não há gate que o webhook "não furou" — não emite o marcador, não toca o log.
    if !is_custody_action(req) {
        return Err(BrokerError::UnknownAction(req.action.clone()));
    }

    let effective = webhook_effective_level(ctx.webhook_level, ctx.workspace);

    // Marca, ANTES da custódia, que esta ação custodiada nasceu de um webhook (prova de que o canal
    // externo não furou o gate). `action_class` é fixa `gated-hard-external`: a custódia é o piso.
    store.append(&DomainEvent::WebhookActionGated {
        webhook_id: ctx.webhook_id.to_string(),
        target_node: ctx.target_node.to_string(),
        action_class: CLASS_GATED_HARD_EXTERNAL.to_string(),
        effective_level: webhook_level_label(effective).to_string(),
    })?;

    // A custódia inquebrável segue idêntica a qualquer outro caminho — o webhook não cria atalho.
    run_custody(req, confirmed, vault, store, execute)
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
            channel: None,
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

    // ───────────────────── F4-0-3 · broker de canal (`lina do <canal> <ação>`, ADR 0004) ──────

    /// Cofre com um token do canal-stub no escopo `channel:slack-stub` (espelha o que F40-CRED grava
    /// via `set_channel_credential`). A chave dentro do escopo é o tipo de credencial (`token`).
    fn vault_with_channel_token() -> SecretVault<MockStore> {
        let v = SecretVault::with_store("lina-space/ws-teste", MockStore::new());
        v.set("channel:slack-stub", "token", "xoxb-TOKEN-DO-CANAL-SECRETO")
            .expect("semear token do canal");
        v
    }

    fn channel_req() -> BrokerRequest {
        // `send` é também um builtin — usado de propósito para provar que o caminho de canal NÃO
        // depende do registry estático: o discriminante `channel:Some` roteia a validação, não o nome.
        BrokerRequest::for_channel("slack-stub", "send", "token", "@Dev")
    }

    /// Gate (b) — lado supervisor: ação de canal SEM confirmação → bloqueia + ActionGated
    /// {class:"gated-hard-external", decision:"ask"}; o executor NÃO roda, e o escopo do segredo é o
    /// do CANAL (`channel:slack-stub`), derivado server-side — não um escopo ditado pelo agente.
    #[test]
    fn channel_unconfirmed_blocks_and_logs_gated_hard_external() {
        let tmp = TempDir::new("chan-unconfirmed");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_channel_token();
        let ran = Cell::new(false);

        let req = channel_req();
        assert_eq!(
            req.secret_scope, "channel:slack-stub",
            "o escopo do segredo de um canal é derivado do nome (server-side), não ditado pelo agente"
        );
        assert_eq!(req.channel.as_deref(), Some("slack-stub"));

        let outcome = run_custody(&req, false, &vault, &mut store, |_secret| {
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
            "ação de canal sem confirmação deve apendar BrokerDenied{{unconfirmed}}"
        );
        assert!(
            !events.iter().any(|r| r.kind == "BrokerExecuted"),
            "NUNCA pode haver BrokerExecuted sem confirmação"
        );
    }

    /// COM confirmação, o broker obtém o segredo DO ESCOPO DO CANAL (`channel:slack-stub`) e executa;
    /// o segredo do canal nunca vaza para o log (invariante #2). Prova que a custódia de canal reusa
    /// o mesmo motor — só o escopo muda.
    #[test]
    fn channel_confirmed_fetches_secret_from_channel_scope_and_executes() {
        let tmp = TempDir::new("chan-confirmed");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_channel_token();
        let seen_secret: Cell<Option<String>> = Cell::new(None);

        let outcome = run_custody(&channel_req(), true, &vault, &mut store, |secret| {
            seen_secret.set(Some(secret.to_string()));
            Ok(())
        })
        .expect("run_custody");

        assert_eq!(outcome, BrokerOutcome::Executed);
        assert_eq!(
            seen_secret.take().as_deref(),
            Some("xoxb-TOKEN-DO-CANAL-SECRETO"),
            "o segredo passado ao executor deve vir do escopo channel:<nome> do cofre"
        );

        let events = store.events().expect("eventos");
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerExecuted" && r.payload["action"] == "send"));
        let log_dump =
            serde_json::to_string(&events.iter().map(|r| &r.payload).collect::<Vec<_>>())
                .expect("serializar log");
        assert!(
            !log_dump.contains("xoxb-TOKEN-DO-CANAL-SECRETO"),
            "o segredo do canal NÃO pode aparecer em nenhum evento do log"
        );
    }

    /// Prova de custódia para canal: confirmado, mas o escopo `channel:<nome>` está VAZIO no cofre →
    /// sem caminho de execução (`BrokerDenied{no_secret}`). Sem credencial guardada (F40-CRED não
    /// rodou para este canal), o canal não dispara efeito externo — mesmo com o "sim" humano.
    #[test]
    fn channel_confirmed_without_secret_cannot_execute() {
        let tmp = TempDir::new("chan-nosecret");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        // Cofre sem o token do canal (existe um token de OUTRO escopo p/ provar que o lookup é por escopo).
        let vault = SecretVault::with_store("lina-space/ws-teste", MockStore::new());
        vault
            .set("channel:outro-canal", "token", "nao-e-este")
            .expect("semear outro escopo");
        let ran = Cell::new(false);

        let outcome = run_custody(&channel_req(), true, &vault, &mut store, |_secret| {
            ran.set(true);
            Ok(())
        })
        .expect("run_custody");

        assert_eq!(outcome, BrokerOutcome::DeniedNoSecret);
        assert!(
            !ran.get(),
            "sem segredo no escopo do canal, o executor NÃO roda"
        );
        let events = store.events().expect("eventos");
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "no_secret"));
        assert!(!events.iter().any(|r| r.kind == "BrokerExecuted"));
    }

    /// A ação de canal NÃO passa pelo registry estático de builtins: uma ação que NÃO existe em
    /// `CUSTODY_ACTIONS` (`post`) é aceita por `run_custody` quando vem por canal (a validação dela é
    /// o manifesto, upstream) — enquanto a MESMA ação como builtin seria `UnknownAction`.
    #[test]
    fn channel_action_outside_static_registry_is_accepted() {
        let tmp = TempDir::new("chan-bypass");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = SecretVault::with_store("lina-space/ws-teste", MockStore::new());
        vault
            .set("channel:notion-stub", "token", "secret-ntn")
            .expect("semear token");

        assert!(
            lookup_action("post").is_none(),
            "pré-condição: `post` não é uma ação builtin"
        );

        // Via canal: aceita (chega ao gate, não dá UnknownAction).
        let req = BrokerRequest::for_channel("notion-stub", "post", "token", "@Dev");
        let outcome =
            run_custody(&req, false, &vault, &mut store, |_| Ok(())).expect("run_custody de canal");
        assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);

        // A MESMA ação como builtin seria recusada de cara (prova que o discriminante é quem roteia).
        let builtin_like = BrokerRequest {
            action: "post".to_string(),
            secret_scope: "channel:notion-stub".to_string(),
            secret_key: "token".to_string(),
            requester: "@Dev".to_string(),
            display: "lina do post".to_string(),
            channel: None,
        };
        let err = run_custody(&builtin_like, false, &vault, &mut store, |_| Ok(())).unwrap_err();
        assert!(
            matches!(err, BrokerError::UnknownAction(_)),
            "sem o discriminante de canal, `post` é UnknownAction"
        );
    }

    /// A validação "ação no manifesto (`scopes`/`tools`)" do gate (b): ação declarada em `scopes`
    /// OU `tools.default_enabled` → permitida; qualquer outra → recusada (default-deny). É a regra
    /// que o supervisor aplica ao resolver `<canal> → manifesto` antes de montar o pedido de canal.
    #[test]
    fn channel_action_validated_against_manifest_scopes_and_tools() {
        let manifest = ChannelManifest::parse(
            r#"
name = "slack-stub"
transport = "stub"
auth = "api_key"
scopes = ["chat:write", "send"]
[tools]
default_enabled = ["post_message"]
[install]
ref = "v0.1.0"
"#,
        )
        .expect("manifesto válido");

        assert!(
            channel_action_in_manifest(&manifest, "send"),
            "ação declarada em scopes é permitida"
        );
        assert!(
            channel_action_in_manifest(&manifest, "post_message"),
            "ação declarada em tools.default_enabled é permitida"
        );
        assert!(
            !channel_action_in_manifest(&manifest, "delete_everything"),
            "ação não-declarada → default-deny"
        );
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

    // ───────────────── F4-WA-4 · gate de ação irreversível por webhook (ADR 0035 §3 · 0004) ──────

    fn webhook_ctx(webhook_level: &str, workspace: AutonomyLevel) -> WebhookGateContext<'_> {
        WebhookGateContext {
            webhook_id: "wh-abc123",
            target_node: "@Dev",
            webhook_level,
            workspace,
        }
    }

    /// O `effective_level` é o `min(webhook, workspace)` na escala `Manual<Assisted<Autonomous` — o
    /// webhook NUNCA afrouxa o que a autonomia do Espaço já fecha (ADR 0035 §3). Função pura.
    #[test]
    fn webhook_effective_level_is_min_of_both_scales() {
        use AutonomyLevel::{Assisted, Autonomous, Manual};
        // webhook `autonomous` num Espaço `assistido` → propõe (RT-4): o min é Assisted.
        assert_eq!(webhook_effective_level("autonomous", Assisted), Assisted);
        // ambos no topo → age.
        assert_eq!(
            webhook_effective_level("autonomous", Autonomous),
            Autonomous
        );
        // webhook `notify_before` num Espaço `autonomo` → o lado conservador (webhook) vence.
        assert_eq!(
            webhook_effective_level("notify_before", Autonomous),
            Assisted
        );
        // workspace `Manual` é o piso — o webhook não o ultrapassa.
        assert_eq!(webhook_effective_level("autonomous", Manual), Manual);
        // nível vazio/desconhecido NUNCA fabrica autonomia (default conservador).
        assert_eq!(webhook_effective_level("", Autonomous), Assisted);
        assert_eq!(webhook_effective_level("banana", Autonomous), Assisted);
    }

    /// Gate (c)/RT-3: instrução→irreversível em `notify_before` → `WebhookActionGated` (prova) +
    /// `ActionGated{gated-hard-external, ask}` (custódia reusada) + `BrokerDenied{unconfirmed}`, e
    /// **0 execução** sem o "sim". O webhook não fura o gate humano.
    #[test]
    fn webhook_irreversible_in_notify_before_gates_and_does_not_execute() {
        let tmp = TempDir::new("wh-notify");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let ran = Cell::new(false);
        let ctx = webhook_ctx("notify_before", AutonomyLevel::Assisted);

        let outcome = run_webhook_custody(&ctx, &deploy_req(), false, &vault, &mut store, |_| {
            ran.set(true);
            Ok(())
        })
        .expect("run_webhook_custody");

        assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);
        assert!(
            !ran.get(),
            "ação irreversível por webhook NÃO executa sem confirmação"
        );

        let events = store.events().expect("eventos");
        // Marcador do canal: prova de que o webhook chegou ao gate (com o nível efetivo).
        let wag = events
            .iter()
            .find(|r| r.kind == "WebhookActionGated")
            .expect("WebhookActionGated");
        assert_eq!(wag.payload["action_class"], "gated-hard-external");
        assert_eq!(wag.payload["effective_level"], "notify_before");
        assert_eq!(wag.payload["webhook_id"], "wh-abc123");
        assert_eq!(wag.payload["target_node"], "@Dev");
        // Custódia REUSADA (run_custody): ActionGated{ask} + BrokerDenied{unconfirmed}, 0 Executed.
        let gated = events
            .iter()
            .find(|r| r.kind == "ActionGated")
            .expect("ActionGated");
        assert_eq!(gated.payload["class"], "gated-hard-external");
        assert_eq!(gated.payload["decision"], "ask");
        assert!(events
            .iter()
            .any(|r| r.kind == "BrokerDenied" && r.payload["reason"] == "unconfirmed"));
        assert!(
            !events.iter().any(|r| r.kind == "BrokerExecuted"),
            "0 execução sem confirmação, mesmo originada por webhook"
        );
    }

    /// Gate (c)/RT-4: webhook `autonomous` num Espaço `assistido` → o nível efetivo é `notify_before`
    /// (PROPÕE, não age) — o min das autonomias manda. Prova no `effective_level` do marcador.
    #[test]
    fn webhook_autonomous_in_assisted_workspace_proposes() {
        let tmp = TempDir::new("wh-autonomous-assisted");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let ctx = webhook_ctx("autonomous", AutonomyLevel::Assisted);

        let outcome =
            run_webhook_custody(&ctx, &deploy_req(), false, &vault, &mut store, |_| Ok(()))
                .expect("run_webhook_custody");
        assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);

        let events = store.events().expect("eventos");
        let wag = events
            .iter()
            .find(|r| r.kind == "WebhookActionGated")
            .expect("WebhookActionGated");
        assert_eq!(
            wag.payload["effective_level"], "notify_before",
            "webhook autonomous num Espaço assistido PROPÕE (min das autonomias)"
        );
    }

    /// Piso ADR 0004: mesmo com o nível efetivo MAIS ALTO (`autonomous` em Espaço `autonomo`), a ação
    /// irreversível AINDA passa pelo gate `ask` — a autonomia jamais afrouxa `gated-hard` externo.
    /// Executa só porque houve o "sim", com o segredo vindo do COFRE (nunca do payload/log).
    #[test]
    fn webhook_autonomous_in_autonomous_workspace_still_gates_hard_external() {
        let tmp = TempDir::new("wh-autonomous-autonomous");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let seen: Cell<Option<String>> = Cell::new(None);
        let ctx = webhook_ctx("autonomous", AutonomyLevel::Autonomous);

        let outcome =
            run_webhook_custody(&ctx, &deploy_req(), true, &vault, &mut store, |secret| {
                seen.set(Some(secret.to_string()));
                Ok(())
            })
            .expect("run_webhook_custody");

        assert_eq!(outcome, BrokerOutcome::Executed);
        assert_eq!(
            seen.take().as_deref(),
            Some("DEPLOY-KEY-SUPER-SECRETA"),
            "o segredo veio do cofre (custódia), nunca do payload do webhook"
        );

        let events = store.events().expect("eventos");
        let wag = events
            .iter()
            .find(|r| r.kind == "WebhookActionGated")
            .expect("WebhookActionGated");
        assert_eq!(wag.payload["effective_level"], "autonomous");
        let gated = events
            .iter()
            .find(|r| r.kind == "ActionGated")
            .expect("ActionGated");
        assert_eq!(
            gated.payload["decision"], "ask",
            "gated-hard externo é SEMPRE ask, mesmo no nível efetivo autônomo"
        );
        assert!(events.iter().any(|r| r.kind == "BrokerExecuted"));

        // Gate (f)/RT-6: nenhum secret aparece no log do canal webhook (só metadados).
        let log_dump =
            serde_json::to_string(&events.iter().map(|r| &r.payload).collect::<Vec<_>>())
                .expect("dump");
        assert!(
            !log_dump.contains("DEPLOY-KEY-SUPER-SECRETA"),
            "o segredo NÃO pode vazar em nenhum evento do canal webhook"
        );
    }

    /// Ação não-custodiada disparada por webhook → `UnknownAction` SEM emitir `WebhookActionGated`
    /// (não há caminho de execução externo → nada que o webhook "não furou"; o log fica intocado).
    #[test]
    fn webhook_unknown_action_rejected_without_marker() {
        let tmp = TempDir::new("wh-unknown");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_deploy_token();
        let ctx = webhook_ctx("autonomous", AutonomyLevel::Autonomous);
        let req = BrokerRequest {
            action: "rm-the-universe".to_string(),
            secret_scope: "deploy".to_string(),
            secret_key: "prod".to_string(),
            requester: "@Dev".to_string(),
            display: "lina do rm-the-universe".to_string(),
            channel: None,
        };

        let err =
            run_webhook_custody(&ctx, &req, true, &vault, &mut store, |_| Ok(())).unwrap_err();
        assert!(matches!(err, BrokerError::UnknownAction(_)));
        assert!(
            kinds(&store).is_empty(),
            "ação não-custodiada não emite WebhookActionGated nem toca o log"
        );
    }

    /// Uma ação irreversível por webhook via CANAL (F4-0-3) reusa o mesmo gate: `WebhookActionGated`
    /// junto da custódia de canal (segredo no escopo `channel:<nome>`). Prova que o marcador é
    /// ortogonal ao tipo de custódia (builtin vs canal).
    #[test]
    fn webhook_channel_action_gates_and_marks() {
        let tmp = TempDir::new("wh-channel");
        let mut store = EventStore::open(tmp.path()).expect("open store");
        let vault = vault_with_channel_token();
        let ctx = webhook_ctx("notify_before", AutonomyLevel::Assisted);

        let outcome =
            run_webhook_custody(&ctx, &channel_req(), false, &vault, &mut store, |_| Ok(()))
                .expect("run_webhook_custody");
        assert_eq!(outcome, BrokerOutcome::DeniedUnconfirmed);

        let events = store.events().expect("eventos");
        assert!(
            events.iter().any(|r| r.kind == "WebhookActionGated"),
            "ação de canal por webhook também emite o marcador"
        );
        assert!(events
            .iter()
            .any(|r| r.kind == "ActionGated" && r.payload["decision"] == "ask"));
        assert!(!events.iter().any(|r| r.kind == "BrokerExecuted"));
    }
}
