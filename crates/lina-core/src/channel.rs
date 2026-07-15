//! F4-0-1 · frente CANAIS (dono: Terminal B — Ultra Code) — **registro de canais**.
//!
//! STUB DA LARGADA F4-0 (preencher nesta frente). Um `Channel` é uma **porta declarada** (âncora
//! de continuidade do doc 01 §3): a abstração que permite WhatsApp/e-mail/ferramentas entrarem como
//! impls de trait + manifesto, sem re-arquitetar o core (invariante #7 core/shell split).
//!
//! ## O que esta frente entrega (critérios F4-0-1, épico 41 §III)
//! - **trait `Channel`** — porta de continuidade: `id()`, acesso ao manifesto, `transport`/`auth`/
//!   `scopes`/`tools.default_enabled`/`install.ref`. O core NÃO importa tipo de canal concreto.
//! - **manifesto declarativo** (`channels/<nome>/manifest.toml`) lido e validado por `serde` ANTES de
//!   qualquer código de canal executar (gate de segurança barato, doc 40 §4/§6). Manifesto malformado
//!   → falha no schema (teste de schema), **nunca executa**.
//! - **`ChannelRegistry`** — projeção pura por replay de `ChannelRegistered` (padrão `ClueSet`/
//!   `CostLedger`: o último registro de um `channel` vence). `register_channel(...)` emite o evento.
//! - **trust default-deny por pertencimento** (espelha ADR 0006): registrar ≠ conectar; o canal nasce
//!   "declarado, não conectado". `trust_tier ∈ {core,curado,comunidade}` (F4-0-6 reusa este campo).
//!
//! ## Invariantes inegociáveis (red-team do gate vai re-derivar no código)
//! - **Manifesto é DADO, JAMAIS autoridade.** O gate de qualquer efeito externo é o broker
//!   (`broker.rs`, F4-0-3) + custódia de segredo (ADR 0004) — nenhum campo do manifesto (nome,
//!   transport, scope) decide identidade/ordem/autorização. **`trust_tier` não vem do manifesto**:
//!   é decidido pela curadoria (F4-0-6) e passado a `register_channel`; um manifesto que tente
//!   declarar o próprio tier falha no schema (`deny_unknown_fields`).
//! - **`install_ref` PINADO** (SHA/tag), nunca HEAD flutuante (doc 40 §6: ref explícito).
//! - **Projeção do log (inv #4):** `from_records` PURA sobre `EventRecord`s; replay reconstrói
//!   byte-a-byte (sem relógio, sem I/O); eventos indecodificáveis são pulados (projeção é derivada,
//!   não validadora do log — espelha `Mentality`/`ApprovalExecutor::replay`).
//!
//! Evento consumido/emitido: `DomainEvent::ChannelRegistered { channel, manifest_ref, trust_tier,
//! install_ref }` (congelado na largada — não re-editar `events.rs`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};

// ───────────────────────────────── manifesto declarativo (DADO) ─────────────────────────────────

/// Manifesto declarativo de um canal (`channels/<nome>/manifest.toml`), já parseado e validado.
///
/// É **DADO de descrição, jamais autoridade**: descreve como o canal fala (`transport`), como
/// autentica (`auth`), o que pede (`scopes`) e o que pré-habilita (`tools.default_enabled`). Não
/// concede acesso — quem libera efeito externo é o broker (F4-0-3) + custódia (ADR 0004).
///
/// `deny_unknown_fields` é parte do gate: um manifesto com campo desconhecido (ex.: uma tentativa
/// de declarar `trust_tier`, que é prerrogativa da curadoria) **falha no schema antes de executar**.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelManifest {
    /// Nome/identidade do canal — a chave no [`ChannelRegistry`] e no evento `ChannelRegistered`.
    pub name: String,
    /// Como o canal fala com o mundo (ex.: `"whatsapp_cloud_api"`, `"smtp"`). DADO, não autoridade.
    pub transport: String,
    /// Método de autenticação declarado (ex.: `"api_key"`, `"oauth2"`). O SEGREDO vive no cofre
    /// (ADR 0004), nunca aqui — este campo só nomeia o método.
    pub auth: String,
    /// Escopos que o canal pede (ex.: `"messages:send"`). Declarar ≠ autorizar (F4-0-4).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Ferramentas pré-habilitadas do canal.
    #[serde(default)]
    pub tools: ToolsManifest,
    /// De onde/em que ponto o canal é instalado — com a ref PINADA.
    pub install: InstallManifest,
}

/// Bloco `[tools]` do manifesto.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsManifest {
    /// Ferramentas habilitadas por padrão ao conectar o canal (lista vazia = nenhuma).
    #[serde(default)]
    pub default_enabled: Vec<String>,
}

/// Bloco `[install]` do manifesto — a ORIGEM pinada do canal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallManifest {
    /// Ref de instalação **PINADA** (SHA ou tag), nunca uma ref flutuante (HEAD/branch/latest):
    /// uma ref móvel deixaria o que será instalado mudar sob os pés (doc 40 §6). Validado em
    /// [`ChannelManifest::validate`]. Mapeado do campo TOML `ref` (palavra reservada em Rust).
    #[serde(rename = "ref")]
    pub pinned_ref: String,
    /// De onde instalar (opcional — URL/caminho). DADO; não decide nada por si.
    #[serde(default)]
    pub source: Option<String>,
}

/// Refs FLUTUANTES bem-conhecidas: apontam para um alvo MÓVEL, então o que seria instalado mudaria
/// a cada `git fetch` (doc 40 §6 exige ref explícito). Distinguir um branch arbitrário de uma tag
/// exige contexto git (fora do escopo do core) — o catálogo/CI (F4-0-6) é o gate mais profundo;
/// aqui barramos as flutuantes canônicas + o vazio, que é o ataque que a invariante nomeia.
const FLOATING_REFS: &[&str] = &["head", "main", "master", "develop", "latest", "trunk"];

/// `true` se `r` é um pino aceitável (SHA/tag) — i.e., não-vazio e não uma ref flutuante conhecida.
fn install_ref_is_pinned(r: &str) -> bool {
    let trimmed = r.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    !FLOATING_REFS.contains(&lower.as_str())
}

impl ChannelManifest {
    /// Parseia + valida o manifesto a partir do texto TOML. O parse (`serde`) cobre o SCHEMA
    /// (campo faltando/tipo errado/campo desconhecido); [`validate`](Self::validate) cobre a
    /// semântica (campos não-vazios + `install.ref` pinado). Malformado → `Err` ANTES de executar.
    ///
    /// # Errors
    /// [`ManifestError::Schema`] se o TOML não casa o schema; [`ManifestError::EmptyField`] /
    /// [`ManifestError::FloatingInstallRef`] se a validação semântica falha.
    pub fn parse(toml_src: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            toml::from_str(toml_src).map_err(|e| ManifestError::Schema(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Lê e valida o manifesto de um arquivo (I/O). Reusa [`parse`](Self::parse) para o schema.
    ///
    /// # Errors
    /// [`ManifestError::Io`] se a leitura falha; senão os erros de [`parse`](Self::parse).
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let src = std::fs::read_to_string(path).map_err(|e| ManifestError::Io(e.to_string()))?;
        Self::parse(&src)
    }

    /// Validação semântica (após o schema do `serde`): identidade/transport/auth não-vazios e
    /// `install.ref` pinado. Separada do parse para também blindar um manifesto montado à mão.
    ///
    /// # Errors
    /// [`ManifestError::EmptyField`] para um campo obrigatório vazio; [`ManifestError::FloatingInstallRef`]
    /// se `install.ref` é flutuante.
    pub fn validate(&self) -> Result<(), ManifestError> {
        for (field, value) in [
            ("name", &self.name),
            ("transport", &self.transport),
            ("auth", &self.auth),
        ] {
            if value.trim().is_empty() {
                return Err(ManifestError::EmptyField(field));
            }
        }
        if !install_ref_is_pinned(&self.install.pinned_ref) {
            return Err(ManifestError::FloatingInstallRef(
                self.install.pinned_ref.clone(),
            ));
        }
        Ok(())
    }
}

// ───────────────────────────────────── trait `Channel` (porta) ─────────────────────────────────────

/// **Porta de continuidade** (doc 01 §3): a abstração que um canal concreto (WhatsApp, e-mail,
/// ferramenta) implementa para entrar no Lina sem re-arquitetar o core (inv #7 core/shell split).
/// O core depende da trait, **nunca** de um tipo de canal concreto de outro crate.
///
/// Em F4-0 o único implementador é [`ManifestChannel`] (um canal DECLARADO por manifesto); F4-1+
/// adiciona impls com transporte vivo. Esta trait é a porta que mantém isso possível.
pub trait Channel {
    /// Identidade do canal — a chave no [`ChannelRegistry`].
    fn id(&self) -> &str;

    /// O manifesto declarativo (já parseado e validado) que descreve o canal.
    fn manifest(&self) -> &ChannelManifest;

    /// Nome humano do canal (default: o `name` do manifesto; uma impl viva pode sobrescrever com
    /// um rótulo mais amigável sem mudar o `id`).
    fn name(&self) -> &str {
        &self.manifest().name
    }
}

/// Canal **declarado por manifesto** — o implementador de [`Channel`] em F4-0 (registrar ≠ conectar).
/// Carrega só o manifesto validado; não fala com o mundo (efeito externo é o broker, F4-0-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestChannel {
    manifest: ChannelManifest,
}

impl ManifestChannel {
    /// Constrói o canal a partir de um manifesto já validado (use [`ChannelManifest::parse`]).
    #[must_use]
    pub fn new(manifest: ChannelManifest) -> Self {
        Self { manifest }
    }
}

impl Channel for ManifestChannel {
    fn id(&self) -> &str {
        &self.manifest.name
    }

    fn manifest(&self) -> &ChannelManifest {
        &self.manifest
    }
}

// ─────────────────────────────── trust default-deny por pertencimento ───────────────────────────────

/// Tier de confiança de um canal (ADR 0006 — default-deny por pertencimento). **Não é declarado pelo
/// manifesto**: é decidido pela curadoria (F4-0-6, catálogo). A variante padrão é a MENOS confiável.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrustTier {
    /// Canal da casa (auto-confiável). Curadoria explícita.
    Core,
    /// Curado por revisão humana (PR). Curadoria explícita.
    Curado,
    /// Comunidade — opt-in, o piso de confiança. **Default-deny:** o que não foi curado fica aqui.
    #[default]
    Comunidade,
}

impl TrustTier {
    /// Forma canônica em texto (a que vai no evento `ChannelRegistered.trust_tier`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            TrustTier::Core => "core",
            TrustTier::Curado => "curado",
            TrustTier::Comunidade => "comunidade",
        }
    }

    /// Lê um tier do log. **Default-deny:** uma string desconhecida (tier inválido, ou uma versão
    /// futura do catálogo) cai no tier MENOS confiável (`Comunidade`), JAMAIS sobe para `core`. O
    /// log é DADO — nunca concede confiança por si (espelha a regra-mãe da onda).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "core" => TrustTier::Core,
            "curado" => TrustTier::Curado,
            _ => TrustTier::Comunidade,
        }
    }
}

/// Estado de conexão derivado de um canal registrado (puro do log — nenhum campo de agente decide
/// conexão, inv #4).
///
/// `Declared` é o estado de nascimento (default-deny do ADR 0006: registrar não conecta).
/// `Connected` é derivado de `ChannelConnected` (gesto humano de tela, F4-1) e carrega a
/// [`RegisteredChannel::session_ref`] custodiada; `ChannelDisconnected` zera o cano e volta a
/// `Declared`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    /// Registrado mas sem conexão estabelecida — o estado de nascimento de todo canal.
    Declared,
    /// Conexão estabelecida por gesto humano de tela (`ChannelConnected`) — há uma sessão custodiada
    /// ativa, referenciada por [`RegisteredChannel::session_ref`].
    Connected,
}

impl ChannelStatus {
    /// Rótulo humano (pt-br) para a tela honesta / monitor de rede (F4-0-5). Zero jargão.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ChannelStatus::Declared => "declarado, não conectado",
            ChannelStatus::Connected => "conectado",
        }
    }
}

// ──────────────────────────────── projeção: ChannelRegistry (PURA) ────────────────────────────────

/// Um canal como reconstruído do log: os fatos do último `ChannelRegistered` daquele `channel`.
///
/// Note que a projeção só vê o que está NO evento (`manifest_ref` é uma REFERÊNCIA ao manifesto, não
/// o conteúdo) — para ler `transport`/`auth`/`scopes` carregue o manifesto via
/// [`ChannelManifest::load`] com o `manifest_ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredChannel {
    /// Nome/identidade do canal (a chave no registry).
    pub channel: String,
    /// Referência ao manifesto lido no registro (caminho relativo ao dir de canais).
    pub manifest_ref: String,
    /// Tier de confiança reidratado com default-deny (string desconhecida → `Comunidade`).
    pub trust_tier: TrustTier,
    /// Ref de instalação pinada registrada (SHA/tag).
    pub install_ref: String,
    /// Estado derivado — `Declared` ao registrar; `Connected` após `ChannelConnected`.
    pub status: ChannelStatus,
    /// REFERÊNCIA à sessão custodiada (cofre): `Some` enquanto `Connected`; `None` ao registrar ou
    /// desconectar. JAMAIS o token — só o ponteiro para o cofre (ADR 0004).
    pub session_ref: Option<String>,
}

/// Projeção dos canais registrados por NOME (padrão `ClueSet`/`CostLedger`: o último
/// `ChannelRegistered` de um `channel` vence). `BTreeMap` garante ordem estável → relatório/
/// fingerprint determinístico (o monitor de F4-0-5 lê esta projeção).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelRegistry {
    by_channel: BTreeMap<String, RegisteredChannel>,
}

impl ChannelRegistry {
    /// Reconstrói a projeção do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log e reconstrói o estado
    /// corrente de cada canal. Testável com log sintético — não embute relógio nem I/O.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut by_channel: BTreeMap<String, RegisteredChannel> = BTreeMap::new();
        for record in records {
            // Registro indecodificável (versão futura): a projeção é DERIVADA, não validadora do
            // log (espelha `Mentality`/`ClueSet`) — pula sem panicar.
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            match event {
                DomainEvent::ChannelRegistered {
                    channel,
                    manifest_ref,
                    trust_tier,
                    install_ref,
                } => {
                    // Último-vence (padrão `ClueSet`): a re-registração substitui o estado anterior
                    // — nova declaração volta a `Declared`, sem sessão.
                    by_channel.insert(
                        channel.clone(),
                        RegisteredChannel {
                            channel,
                            manifest_ref,
                            // Default-deny ao reidratar: tier desconhecido NUNCA vira `core`.
                            trust_tier: TrustTier::parse(&trust_tier),
                            install_ref,
                            status: ChannelStatus::Declared,
                            session_ref: None,
                        },
                    );
                }
                // Conectar/desconectar atualiza um canal JÁ registrado (registrar ≠ conectar): um
                // evento de conexão órfão NÃO materializa canal — falta manifesto/tier/install para
                // construí-lo, e o log é DADO, jamais autoridade que fabrica estado.
                DomainEvent::ChannelConnected {
                    channel,
                    session_ref,
                    ..
                } => {
                    if let Some(ch) = by_channel.get_mut(&channel) {
                        ch.status = ChannelStatus::Connected;
                        ch.session_ref = Some(session_ref);
                    }
                }
                DomainEvent::ChannelDisconnected { channel, .. } => {
                    if let Some(ch) = by_channel.get_mut(&channel) {
                        ch.status = ChannelStatus::Declared;
                        ch.session_ref = None;
                    }
                }
                _ => {}
            }
        }
        Self { by_channel }
    }

    /// O canal de `name`, se registrado.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&RegisteredChannel> {
        self.by_channel.get(name)
    }

    /// Os canais registrados, em ordem estável de nome.
    pub fn channels(&self) -> impl Iterator<Item = &RegisteredChannel> {
        self.by_channel.values()
    }

    /// `true` se nenhum canal foi registrado (a base do "0 canais ativos" do monitor, F4-0-5).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_channel.is_empty()
    }

    /// Quantidade de canais registrados.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_channel.len()
    }
}

// ─────────────────────────── handler do verbo `lina channel` (Maestro fia o dispatch) ───────────────────────────

/// Registra um canal a partir do manifesto: emite `ChannelRegistered`. Registrar ≠ conectar — o
/// canal nasce "declarado, não conectado". O `trust_tier` é decidido pela CURADORIA (F4-0-6) e
/// passado aqui (manifesto é DADO, jamais autoridade) — na ausência de curadoria, passe
/// [`TrustTier::default`] (`Comunidade`, default-deny). Revalida o manifesto antes de persistir
/// (gate barato ANTES de qualquer efeito).
///
/// # Errors
/// [`ChannelError::Manifest`] se o manifesto não passa na validação; [`ChannelError::Store`] se a
/// persistência do evento falha.
pub fn register_channel(
    store: &mut EventStore,
    manifest: &ChannelManifest,
    manifest_ref: &str,
    trust_tier: TrustTier,
) -> Result<u64, ChannelError> {
    manifest.validate()?;
    let seq = store.append(&DomainEvent::ChannelRegistered {
        channel: manifest.name.clone(),
        manifest_ref: manifest_ref.trim().to_string(),
        trust_tier: trust_tier.as_str().to_string(),
        install_ref: manifest.install.pinned_ref.clone(),
    })?;
    Ok(seq)
}

/// Marca um canal já registrado como CONECTADO: emite `ChannelConnected`. A projeção passa a derivar
/// [`ChannelStatus::Connected`] + a `session_ref`. O `session_ref` é a REFERÊNCIA à sessão custodiada
/// no cofre (o agente nunca vê o token, ADR 0004); `scope` é DADO declarativo (ex.: instância Waha),
/// jamais autoridade. Conectar pressupõe registrar — conectar um canal não registrado é no-op na
/// projeção (espelha `register_channel`: o helper só apenda o fato; o broker é quem gate o efeito).
///
/// # Errors
/// [`ChannelError::Store`] se a persistência do evento falha.
pub fn connect_channel(
    store: &mut EventStore,
    channel: &str,
    session_ref: &str,
    scope: &str,
) -> Result<u64, ChannelError> {
    let seq = store.append(&DomainEvent::ChannelConnected {
        channel: channel.to_string(),
        session_ref: session_ref.to_string(),
        scope: scope.to_string(),
    })?;
    Ok(seq)
}

/// Desfaz a conexão de um canal: emite `ChannelDisconnected`. A projeção volta o canal a
/// [`ChannelStatus::Declared`] e zera a `session_ref` (o cano fecha). `session_ref` correlaciona com
/// o `ChannelConnected` que se encerra.
///
/// # Errors
/// [`ChannelError::Store`] se a persistência do evento falha.
pub fn disconnect_channel(
    store: &mut EventStore,
    channel: &str,
    session_ref: &str,
) -> Result<u64, ChannelError> {
    let seq = store.append(&DomainEvent::ChannelDisconnected {
        channel: channel.to_string(),
        session_ref: session_ref.to_string(),
    })?;
    Ok(seq)
}

// ───────────────────────────────────────── erros ─────────────────────────────────────────

/// Falha ao parsear/validar um manifesto de canal.
#[derive(thiserror::Error, Debug)]
pub enum ManifestError {
    /// O TOML não casa o schema (campo faltando, tipo errado, campo desconhecido). Mensagem do parser.
    #[error("manifesto malformado (schema): {0}")]
    Schema(String),
    /// Um campo obrigatório veio vazio.
    #[error("campo obrigatório vazio no manifesto: '{0}'")]
    EmptyField(&'static str),
    /// `install.ref` é uma ref FLUTUANTE — exige-se um pino (SHA ou tag), nunca HEAD/branch.
    #[error(
        "install.ref flutuante ('{0}') — exija um pino (SHA ou tag), nunca HEAD/branch/latest"
    )]
    FloatingInstallRef(String),
    /// Falha de I/O ao ler o arquivo de manifesto.
    #[error("falha ao ler o manifesto: {0}")]
    Io(String),
}

/// Falha ao registrar um canal (validação do manifesto ou persistência do evento).
#[derive(thiserror::Error, Debug)]
pub enum ChannelError {
    /// O manifesto não passou na validação — nada foi persistido.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Falha ao persistir `ChannelRegistered` no event store.
    #[error(transparent)]
    Store(#[from] StoreError),
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    /// Diretório temporário único; removido no `Drop` (best-effort) — molde de `events.rs`.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-f4chan-{tag}-{}", Uuid::now_v7()));
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

    /// Um manifesto bem-formado de referência (texto TOML).
    const WELL_FORMED: &str = r#"
        name = "whatsapp"
        transport = "whatsapp_cloud_api"
        auth = "api_key"
        scopes = ["messages:send", "messages:read"]

        [tools]
        default_enabled = ["send_message"]

        [install]
        ref = "v0.1.0"
    "#;

    /// Constrói um `EventRecord` de `ChannelRegistered` serializado COMO no log: a tag interna
    /// `event` (enum `#[serde(tag = "event")]`) precisa estar no payload, senão `from_record` não
    /// decodifica. Serializamos o `DomainEvent` REAL (via `kind`/`current_version`/`serde_json`),
    /// nunca um JSON montado à mão sem a tag.
    fn channel_record(
        seq: u64,
        channel: &str,
        manifest_ref: &str,
        trust_tier: &str,
        install_ref: &str,
    ) -> EventRecord {
        let event = DomainEvent::ChannelRegistered {
            channel: channel.to_string(),
            manifest_ref: manifest_ref.to_string(),
            trust_tier: trust_tier.to_string(),
            install_ref: install_ref.to_string(),
        };
        EventRecord {
            seq,
            ts: seq,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa o evento"),
        }
    }

    /// `EventRecord` de `ChannelConnected` serializado COMO no log (mesma disciplina de tag de
    /// `channel_record`).
    fn connected_record(seq: u64, channel: &str, session_ref: &str, scope: &str) -> EventRecord {
        let event = DomainEvent::ChannelConnected {
            channel: channel.to_string(),
            session_ref: session_ref.to_string(),
            scope: scope.to_string(),
        };
        EventRecord {
            seq,
            ts: seq,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa o evento"),
        }
    }

    /// `EventRecord` de `ChannelDisconnected` serializado COMO no log.
    fn disconnected_record(seq: u64, channel: &str, session_ref: &str) -> EventRecord {
        let event = DomainEvent::ChannelDisconnected {
            channel: channel.to_string(),
            session_ref: session_ref.to_string(),
        };
        EventRecord {
            seq,
            ts: seq,
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(&event).expect("serializa o evento"),
        }
    }

    /// Schema: um manifesto bem-formado parseia com todos os campos.
    #[test]
    fn well_formed_manifest_parses_with_all_fields() {
        let m = ChannelManifest::parse(WELL_FORMED).expect("bem-formado parseia");
        assert_eq!(m.name, "whatsapp");
        assert_eq!(m.transport, "whatsapp_cloud_api");
        assert_eq!(m.auth, "api_key");
        assert_eq!(m.scopes, ["messages:send", "messages:read"]);
        assert_eq!(m.tools.default_enabled, ["send_message"]);
        assert_eq!(m.install.pinned_ref, "v0.1.0");
    }

    /// Critério: manifesto malformado → **falha no schema, NUNCA executa** (campo `install` faltando).
    #[test]
    fn malformed_manifest_fails_schema() {
        let malformed = r#"
            name = "x"
            transport = "smtp"
            auth = "oauth2"
        "#; // sem [install] → campo obrigatório faltando
        let err = ChannelManifest::parse(malformed).expect_err("malformado falha");
        assert!(matches!(err, ManifestError::Schema(_)), "got {err:?}");
    }

    /// Segurança (manifesto não é autoridade): um manifesto que tenta declarar o PRÓPRIO `trust_tier`
    /// é REJEITADO no schema (`deny_unknown_fields`) — a confiança é prerrogativa da curadoria, não
    /// auto-declarável. Sem isso, um canal subiria sozinho para `core`.
    #[test]
    fn manifest_cannot_self_declare_trust_tier() {
        let sneaky = r#"
            name = "evil"
            transport = "smtp"
            auth = "none"
            trust_tier = "core"

            [install]
            ref = "v1.0.0"
        "#;
        let err = ChannelManifest::parse(sneaky).expect_err("campo desconhecido falha");
        assert!(matches!(err, ManifestError::Schema(_)), "got {err:?}");
    }

    /// Invariante `install_ref` PINADO: refs flutuantes (HEAD/branch/latest/vazio) são recusadas;
    /// SHA/tag passam.
    #[test]
    fn floating_install_ref_is_rejected_pinned_is_accepted() {
        for floating in [
            "HEAD", "main", "master", "develop", "latest", "trunk", "", "   ",
        ] {
            let toml_src =
                format!("name=\"c\"\ntransport=\"t\"\nauth=\"a\"\n[install]\nref=\"{floating}\"\n");
            assert!(
                matches!(
                    ChannelManifest::parse(&toml_src),
                    Err(ManifestError::FloatingInstallRef(_))
                ),
                "ref flutuante '{floating}' deveria ser recusada"
            );
        }
        for pinned in ["v0.1.0", "9f1c2ae", "release-2024.06"] {
            let toml_src =
                format!("name=\"c\"\ntransport=\"t\"\nauth=\"a\"\n[install]\nref=\"{pinned}\"\n");
            assert!(
                ChannelManifest::parse(&toml_src).is_ok(),
                "pino legítimo '{pinned}' deveria passar"
            );
        }
    }

    /// Critério-coração: `register_channel` emite `ChannelRegistered` → o canal aparece no registry
    /// como "declarado, não conectado", PROVADO por replay do log real (append → projeta → re-acha).
    #[test]
    fn register_then_replay_rediscovers_channel_as_declared() {
        let tmp = TempDir::new("roundtrip");
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        let manifest = ChannelManifest::parse(WELL_FORMED).expect("parse");

        register_channel(
            &mut store,
            &manifest,
            "channels/whatsapp/manifest.toml",
            TrustTier::Curado,
        )
        .expect("registra");

        let registry = ChannelRegistry::replay(&store).expect("replay");
        let ch = registry
            .get("whatsapp")
            .expect("canal re-encontrado no log");
        assert_eq!(ch.channel, "whatsapp");
        assert_eq!(ch.manifest_ref, "channels/whatsapp/manifest.toml");
        assert_eq!(ch.install_ref, "v0.1.0");
        assert_eq!(ch.trust_tier, TrustTier::Curado);
        assert_eq!(ch.status, ChannelStatus::Declared);
        assert_eq!(ch.status.label(), "declarado, não conectado");
        assert_eq!(ch.session_ref, None);
        assert_eq!(registry.len(), 1);
    }

    /// Critério F4-1-2: ciclo de vida real pelo log — registrar (`Declared`, sem sessão) → conectar
    /// (`Connected` + `session_ref`) → desconectar (volta a `Declared`, zera `session_ref`). Provado
    /// por append→replay (caminho real, sem montar projeção à mão).
    #[test]
    fn connect_then_disconnect_drives_status_and_session_ref() {
        let tmp = TempDir::new("connect-lifecycle");
        let mut store = EventStore::open(tmp.path()).expect("abrir store");
        let manifest = ChannelManifest::parse(WELL_FORMED).expect("parse");
        register_channel(
            &mut store,
            &manifest,
            "channels/whatsapp/manifest.toml",
            TrustTier::Curado,
        )
        .expect("registra");

        // Registrado, ainda não conectado.
        let registry = ChannelRegistry::replay(&store).expect("replay");
        let ch = registry.get("whatsapp").expect("registrado");
        assert_eq!(ch.status, ChannelStatus::Declared);
        assert_eq!(ch.session_ref, None);

        // Conectar → Connected + session_ref + rótulo pt-br "conectado".
        connect_channel(
            &mut store,
            "whatsapp",
            "keyring:channel:whatsapp",
            "waha@127.0.0.1:3000",
        )
        .expect("conecta");
        let registry = ChannelRegistry::replay(&store).expect("replay");
        let ch = registry.get("whatsapp").expect("registrado");
        assert_eq!(ch.status, ChannelStatus::Connected);
        assert_eq!(ch.status.label(), "conectado");
        assert_eq!(ch.session_ref.as_deref(), Some("keyring:channel:whatsapp"));

        // Desconectar → o cano zera: volta a Declared, session_ref some.
        disconnect_channel(&mut store, "whatsapp", "keyring:channel:whatsapp").expect("desconecta");
        let registry = ChannelRegistry::replay(&store).expect("replay");
        let ch = registry.get("whatsapp").expect("registrado");
        assert_eq!(ch.status, ChannelStatus::Declared);
        assert_eq!(ch.session_ref, None);
    }

    /// Último-vence (padrão `ClueSet`) sobre conexão + replay determinístico: a sequência
    /// connect→disconnect→connect converge no último estado, idêntica em duas execuções.
    #[test]
    fn connect_disconnect_last_wins_and_replay_deterministic() {
        let log = [
            channel_record(1, "wa", "m.toml", "core", "v1"),
            connected_record(2, "wa", "sref-1", "waha"),
            disconnected_record(3, "wa", "sref-1"),
            connected_record(4, "wa", "sref-2", "waha"),
        ];
        let registry = ChannelRegistry::from_records(&log);
        let ch = registry.get("wa").expect("registrado");
        assert_eq!(ch.status, ChannelStatus::Connected);
        assert_eq!(
            ch.session_ref.as_deref(),
            Some("sref-2"),
            "último connect vence"
        );
        assert_eq!(
            ChannelRegistry::from_records(&log),
            ChannelRegistry::from_records(&log),
            "replay determinístico (inv #4)"
        );
    }

    /// Segurança/invariante "registrar ≠ conectar": um `ChannelConnected` sem `ChannelRegistered`
    /// prévio NÃO materializa canal — o log é DADO, jamais autoridade que fabrica estado/sessão.
    #[test]
    fn connect_without_prior_registration_is_noop() {
        let log = [
            connected_record(1, "ghost", "sref", "waha"),
            disconnected_record(2, "ghost", "sref"),
        ];
        let registry = ChannelRegistry::from_records(&log);
        assert!(
            registry.get("ghost").is_none(),
            "conectar sem registrar não cria canal"
        );
        assert!(registry.is_empty());
    }

    /// Default-deny: na ausência de curadoria, registra-se com [`TrustTier::default`] (`Comunidade`).
    #[test]
    fn uncurated_registration_defaults_to_least_privileged_tier() {
        let tmp = TempDir::new("default-deny");
        let mut store = EventStore::open(tmp.path()).expect("abrir");
        let manifest = ChannelManifest::parse(WELL_FORMED).expect("parse");
        register_channel(&mut store, &manifest, "m.toml", TrustTier::default()).expect("registra");
        let registry = ChannelRegistry::replay(&store).expect("replay");
        assert_eq!(
            registry.get("whatsapp").unwrap().trust_tier,
            TrustTier::Comunidade
        );
    }

    /// Segurança (default-deny ao reidratar): um `trust_tier` desconhecido no log NUNCA sobe para
    /// `core` — cai no piso `Comunidade`; tiers válidos reidratam corretamente.
    #[test]
    fn unknown_trust_tier_in_log_defaults_to_community_never_core() {
        let log = [channel_record(1, "x", "m.toml", "superuser", "v1.0")];
        let registry = ChannelRegistry::from_records(&log);
        assert_eq!(
            registry.get("x").unwrap().trust_tier,
            TrustTier::Comunidade,
            "tier desconhecido jamais vira core"
        );
        let log = [
            channel_record(1, "a", "m", "core", "v1"),
            channel_record(2, "b", "m", "curado", "v1"),
        ];
        let registry = ChannelRegistry::from_records(&log);
        assert_eq!(registry.get("a").unwrap().trust_tier, TrustTier::Core);
        assert_eq!(registry.get("b").unwrap().trust_tier, TrustTier::Curado);
    }

    /// Padrão `ClueSet`: o ÚLTIMO `ChannelRegistered` de um canal vence (substitui, não acumula).
    #[test]
    fn last_registration_per_channel_wins() {
        let log = [
            channel_record(1, "wa", "old.toml", "comunidade", "v0.1.0"),
            channel_record(2, "wa", "new.toml", "core", "v0.2.0"),
        ];
        let registry = ChannelRegistry::from_records(&log);
        let ch = registry.get("wa").unwrap();
        assert_eq!(ch.manifest_ref, "new.toml");
        assert_eq!(ch.install_ref, "v0.2.0");
        assert_eq!(ch.trust_tier, TrustTier::Core);
        assert_eq!(registry.len(), 1, "re-registro substitui, não duplica");
    }

    /// Replay determinístico (inv #4): mesma sequência de eventos → projeção idêntica.
    #[test]
    fn replay_is_deterministic() {
        let log = [
            channel_record(1, "a", "a.toml", "core", "v1"),
            channel_record(2, "b", "b.toml", "curado", "v2"),
            channel_record(3, "a", "a2.toml", "comunidade", "v3"),
        ];
        assert_eq!(
            ChannelRegistry::from_records(&log),
            ChannelRegistry::from_records(&log)
        );
    }

    /// Registry vazio = nenhum canal (base do "0 canais ativos → 0 sockets" do monitor F4-0-5).
    #[test]
    fn empty_registry_has_no_channels() {
        let registry = ChannelRegistry::default();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.channels().next().is_none());
    }

    /// A trait `Channel` (porta de continuidade) expõe identidade e manifesto.
    #[test]
    fn channel_trait_exposes_identity_and_manifest() {
        let manifest = ChannelManifest::parse(WELL_FORMED).expect("parse");
        let ch = ManifestChannel::new(manifest);
        assert_eq!(ch.id(), "whatsapp");
        assert_eq!(ch.name(), "whatsapp"); // default: name = id
        assert_eq!(ch.manifest().transport, "whatsapp_cloud_api");
        assert_eq!(ch.manifest().tools.default_enabled, ["send_message"]);
    }

    /// I/O: `load` lê e valida um manifesto de disco; arquivo inexistente → erro (não panica).
    #[test]
    fn load_reads_and_validates_from_disk() {
        let tmp = TempDir::new("load");
        let path = tmp.path().join("manifest.toml");
        std::fs::write(&path, WELL_FORMED).expect("escreve manifesto");
        let m = ChannelManifest::load(&path).expect("carrega do disco");
        assert_eq!(m.name, "whatsapp");
        let missing = tmp.path().join("nope.toml");
        assert!(matches!(
            ChannelManifest::load(&missing),
            Err(ManifestError::Io(_))
        ));
    }

    /// Os manifestos de exemplo ENVIADOS na árvore (`channels/*/manifest.toml`) são válidos — este
    /// teste é também a semente da validação de manifesto em CI (gate (e), F4-0-6).
    #[test]
    fn shipped_example_manifests_are_valid() {
        let whatsapp = include_str!("../../../channels/whatsapp-stub/manifest.toml");
        let email = include_str!("../../../channels/email-stub/manifest.toml");
        let wa = ChannelManifest::parse(whatsapp).expect("whatsapp-stub válido");
        assert_eq!(wa.name, "whatsapp");
        assert!(install_ref_is_pinned(&wa.install.pinned_ref));
        let em = ChannelManifest::parse(email).expect("email-stub válido");
        assert_eq!(em.name, "email");
        assert!(install_ref_is_pinned(&em.install.pinned_ref));
    }
}
