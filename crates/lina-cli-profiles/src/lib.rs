//! `lina-cli-profiles` — CLI Profiles declarativos (story **W0-8**).
//!
//! ## Âncora de continuidade (ver `CLAUDE.md`)
//! Toda especificidade de um CLI de IA vive em **TOML**, nunca compilada no core
//! (invariante de neutralidade). Novos CLIs (Claude Code, Codex, Gemini, OpenCode,
//! Copilot, shell puro) entram **sem recompilar** — trocar de CLI = trocar um `id`.
//!
//! Campos-chave (alimentam a entrega A2A faseada de W0-9 e o fim-de-resposta de
//! W0-10): [`Delivery`] (`pty_inject` | `session_resume`), `submit_delay_ms`,
//! `prompt_ready_regex`, [`EndSignal`].
//!
//! Gate de aceite (W0-8): parseia um `claude-code.toml` de exemplo e expõe os
//! quatro campos; um TOML inválido falha com erro claro (sem panic, sem `unwrap`).
//!
//! ### Escopo desta entrega vs. story canônica
//! Esta story implementa o **schema declarativo + loader (arquivo e diretório) +
//! registry com override por `id`**. Ficam como incrementos seguintes, com porta
//! aberta no design (ver `TODO`s no código):
//! - compilar/validar o `prompt_ready_regex` (aqui ele é mantido como `String`);
//! - profiles default **embutidos** (claude/codex/gemini/shell);
//! - resolver `CommandBuilder` (W0-1) a partir de `program`+`args`;
//! - `lina-core --list-profiles`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

/// Default de `submit_delay` quando o profile não declara — a A2A faseada do
/// `CLAUDE.md` (§Stack) usa ~0.3s entre colar o texto e enviar o Enter.
const DEFAULT_SUBMIT_DELAY_MS: u64 = 300;

fn default_submit_delay_ms() -> u64 {
    DEFAULT_SUBMIT_DELAY_MS
}

fn default_stream_json_event() -> String {
    "result".to_string()
}

/// Como o orquestrador entrega um novo prompt a um terminal vivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Injeta no PTY vivo: bracketed-paste → `submit_delay` → Enter (`0x0D`)
    /// separado, em fila serial por terminal. Estratégia primária (W0-9).
    PtyInject,
    /// Reinicia a sessão do CLI por uma flag de retomada (ex.: `--resume`).
    /// Fallback quando a injeção no PTY vivo não é confiável para aquele CLI.
    SessionResume,
}

/// Como detectar o **fim de uma resposta** do CLI (consumido por W0-10).
///
/// Internamente *tagged* por `kind` no TOML, p.ex.:
/// ```toml
/// [end_signal]
/// kind = "stream_json"
/// event_type = "result"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EndSignal {
    /// `--output-format stream-json`: a resposta termina quando chega uma linha
    /// JSON cujo `type == event_type` (o Claude Code emite `{"type":"result",…}`).
    StreamJson {
        #[serde(default = "default_stream_json_event")]
        event_type: String,
    },
    /// Genérico: termina quando este regex casa na saída viva.
    /// (Mantido como `String`; compilação é incremento de W0-10 — ver `TODO`.)
    Regex { pattern: String },
    /// Termina por silêncio do stream (a janela vem de `idle_ms`).
    Idle,
}

/// Capability flags declarativas de um CLI (F1-1-1, pesquisa 13.9): o que ESTE CLI
/// emite/suporta (`hooks`, `handoff_support`, `permission_detection`, `otel`, …).
/// Mapa **aberto por design** (invariante #3): uma capability nova entra por TOML,
/// sem recompilar — consumidores perguntam por nome e tratam ausência como `false`
/// ("nenhum consumidor espera evento que o CLI não emite" — F1-1-3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Capabilities(BTreeMap<String, bool>);

impl Capabilities {
    /// `true` somente se a capability foi declarada `true` no TOML.
    /// Ausente ou declarada `false` → `false` (nunca panica).
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.0.get(name).copied().unwrap_or(false)
    }
}

/// Profile declarativo de um CLI de IA. **Toda** especificidade do CLI vive aqui
/// (TOML), nunca no core — é a âncora de neutralidade multi-CLI do `CLAUDE.md`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliProfile {
    /// Identificador estável (ex.: `"claude-code"`). É a chave no [`ProfileRegistry`].
    pub id: String,
    /// Executável a lançar (ex.: `"claude"`). Único ponto que conhece o binário.
    pub program: String,
    /// Argumentos fixos passados ao CLI (ex.: `["--output-format", "stream-json"]`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Estratégia de entrega de prompt.
    pub delivery: Delivery,
    /// Espera (ms) entre colar o texto e enviar o Enter, na A2A faseada. Default 300.
    #[serde(default = "default_submit_delay_ms")]
    pub submit_delay_ms: u64,
    /// Regex que sinaliza "prompt pronto para receber input".
    ///
    /// Mantido como `String` nesta story; a compilação/validação do regex no load
    /// (erro acionável se inválido) é incremento de W0-9.
    // TODO(W0-9): compilar com a crate `regex` e validar no load.
    pub prompt_ready_regex: String,
    /// Como detectar fim-de-resposta.
    pub end_signal: EndSignal,
    /// F1-0-2: orçamento (ms) do `wait_ready` da entrega A2A — quanto esperar o prompt
    /// REAL do alvo antes de desistir desta tentativa (timeout → **não injeta**; nunca
    /// "injeta mesmo assim"). `None` = default 2s (o `READY_TIMEOUT` histórico de W0-9).
    /// Calibrável por CLI sem recompilar (invariante #3).
    #[serde(default)]
    pub ready_timeout_ms: Option<u64>,
    /// F1-0-2: marcadores de OCUPADO na região de input do grid — se QUALQUER um estiver
    /// presente, o alvo NÃO está pronto, mesmo que o glifo de prompt apareça (a TUI do
    /// Claude Code mostra `❯ ` o tempo todo; o que distingue ocupado é o rodapé
    /// "esc to interrupt" / o banner "Press up to edit queued messages" — grids REAIS
    /// da baseline F1-0). Vazio = só o `prompt_ready_regex` decide (CLIs simples).
    #[serde(default)]
    pub busy_markers: Vec<String>,
    /// Quietude (ms) que conta como "idle" para heurísticas de fim/timeout (W0-10).
    #[serde(default)]
    pub idle_ms: Option<u64>,
    /// Timeout (ms) para um `ask` A2A aguardar a resposta do destinatário (W0-9).
    #[serde(default)]
    pub ask_timeout_ms: Option<u64>,
    /// Padrão (glob; suporta `~`) dos session-files deste CLI em disco
    /// (ex.: `~/.claude/projects/*/*.jsonl`). Consumido pelo watch da camada 3
    /// (F1-1-2). `None` = CLI sem session-files (ex.: shell puro).
    #[serde(default)]
    pub session_dir_pattern: Option<String>,
    /// Assinaturas deste CLI observáveis no output do grid — telemetria da
    /// camada 4 do ADR de detecção (13.9): **nunca decisória**, só validação
    /// cruzada de confiança. Vazio = sem marcadores conhecidos.
    #[serde(default)]
    pub output_markers: Vec<String>,
    /// Variáveis de ambiente do **spawn** deste CLI (paralelo ao `env` do
    /// [`InstallRecipe`], que vale só para a instalação).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Capability flags deste CLI (ver [`Capabilities`]) — alimentam F1-1-2/3/6/9
    /// e os guards por CLI (decision table no TOML, 13.9 item 5).
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl CliProfile {
    /// `submit_delay_ms` como [`Duration`] — ergonomia para o agendador da A2A (W0-9).
    pub fn submit_delay(&self) -> Duration {
        Duration::from_millis(self.submit_delay_ms)
    }

    /// F1-0-2: orçamento do `wait_ready` como [`Duration`] (default 2s — comportamento
    /// histórico de W0-9 para perfis que não declaram `ready_timeout_ms`).
    pub fn ready_timeout(&self) -> Duration {
        Duration::from_millis(self.ready_timeout_ms.unwrap_or(2_000))
    }

    /// Parseia um profile a partir de uma string TOML.
    ///
    /// `label` aparece nas mensagens de erro (caminho do arquivo ou `"<inline>"`),
    /// para que um TOML inválido produza um erro **acionável** em vez de um panic.
    pub fn from_toml_str(toml_src: &str, label: &str) -> Result<Self, ProfileError> {
        let profile: CliProfile =
            toml::from_str(toml_src).map_err(|source| ProfileError::Parse {
                path: label.to_owned(),
                source: Box::new(source),
            })?;
        profile.validate(label)?;
        Ok(profile)
    }

    /// Carrega um profile de um arquivo `.toml`.
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let path = path.as_ref();
        let label = path.display().to_string();
        let src = std::fs::read_to_string(path).map_err(|source| ProfileError::Io {
            path: label.clone(),
            source,
        })?;
        Self::from_toml_str(&src, &label)
    }

    /// Validações semânticas mínimas — falham com erro claro, sem panic.
    fn validate(&self, label: &str) -> Result<(), ProfileError> {
        if self.id.trim().is_empty() {
            return Err(ProfileError::EmptyField {
                field: "id",
                path: label.to_owned(),
            });
        }
        if self.program.trim().is_empty() {
            return Err(ProfileError::EmptyField {
                field: "program",
                path: label.to_owned(),
            });
        }
        if self.prompt_ready_regex.trim().is_empty() {
            return Err(ProfileError::EmptyField {
                field: "prompt_ready_regex",
                path: label.to_owned(),
            });
        }
        // Presente porém vazio é typo de config — erro claro, não `Some("")` silencioso.
        if let Some(pattern) = &self.session_dir_pattern {
            if pattern.trim().is_empty() {
                return Err(ProfileError::EmptyField {
                    field: "session_dir_pattern",
                    path: label.to_owned(),
                });
            }
        }
        Ok(())
    }
}

// ═══════════════════════════ instaladores ("Instalar para mim") ═══════════════════════════

/// Nome canônico do SO atual (compile-time) usado como chave de SO nas receitas de instalação.
#[cfg(target_os = "macos")]
pub const CURRENT_OS: &str = "macos";
#[cfg(target_os = "windows")]
pub const CURRENT_OS: &str = "windows";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const CURRENT_OS: &str = "linux";

/// Receita de instalação **não-interativa** de um CLI para UM sistema operacional. Roda num PTY
/// oculto (onboarding T1). `env` injeta variáveis que garantem não-interatividade (ex.:
/// `CODEX_NON_INTERACTIVE=1`). `verify_paths` lista diretórios EXTRA a conferir após instalar —
/// vários instaladores põem o binário FORA do `PATH` atual (ex.: `~/.opencode/bin`, `~/.local/bin`)
/// e/ou só editam o rc do shell, então a verificação tem de olhar nesses lugares também.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRecipe {
    /// Programa a lançar (ex.: `"bash"`, `"npm"`, `"powershell"`).
    pub program: String,
    /// Argumentos (ex.: `["-c", "curl -fsSL https://opencode.ai/install | bash"]`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Variáveis de ambiente da instalação (não-interatividade, etc.).
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// Diretórios extra a conferir após instalar (suportam `~`). O onboarding ainda re-hidrata o
    /// `PATH` do shell de login e o `$(npm prefix -g)/bin`; estes cobrem os instaladores que põem
    /// o binário num lugar fixo fora do `PATH`.
    #[serde(default)]
    pub verify_paths: Vec<String>,
}

/// Receitas de instalação de UM CLI, por sistema operacional (chaves `macos`/`linux`/`windows`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallProfile {
    #[serde(default)]
    pub macos: Option<InstallRecipe>,
    #[serde(default)]
    pub linux: Option<InstallRecipe>,
    #[serde(default)]
    pub windows: Option<InstallRecipe>,
}

impl InstallProfile {
    /// Receita para um SO nomeado (`"macos"`/`"linux"`/`"windows"`); `None` se ausente.
    #[must_use]
    pub fn for_os(&self, os: &str) -> Option<&InstallRecipe> {
        match os {
            "macos" => self.macos.as_ref(),
            "windows" => self.windows.as_ref(),
            "linux" => self.linux.as_ref(),
            _ => None,
        }
    }

    /// Receita para o SO de compilação atual ([`CURRENT_OS`]).
    #[must_use]
    pub fn for_current_os(&self) -> Option<&InstallRecipe> {
        self.for_os(CURRENT_OS)
    }
}

/// Tabela de instaladores indexada pelo **id de descoberta** do CLI (`claude`/`codex`/`gemini`/
/// `opencode`/`copilot`) — NOTE que é o id que a varredura do `PATH` procura, que pode diferir do
/// `id` do [`CliProfile`] (ex.: o profile de orquestração tem `id = "claude-code"`). Parseada de um
/// TOML com seções `[<id>.<os>]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Installers(pub std::collections::BTreeMap<String, InstallProfile>);

impl Installers {
    /// Parseia a tabela de instaladores de uma string TOML (erro acionável, sem panic).
    pub fn from_toml_str(toml_src: &str, label: &str) -> Result<Self, ProfileError> {
        toml::from_str(toml_src).map_err(|source| ProfileError::Parse {
            path: label.to_owned(),
            source: Box::new(source),
        })
    }

    /// Receita do CLI `id` para o SO atual (`None` se o CLI ou o SO não tiver receita).
    #[must_use]
    pub fn recipe_for(&self, id: &str) -> Option<&InstallRecipe> {
        self.0.get(id).and_then(InstallProfile::for_current_os)
    }
}

/// Registry de profiles carregados, indexado por `id`.
///
/// Um profile de mesmo `id` carregado depois **sobrescreve** o anterior — é assim
/// que um profile custom (diretório do usuário) vence o default de mesmo `id` (W0-8).
#[derive(Debug, Default, Clone)]
pub struct ProfileRegistry {
    profiles: BTreeMap<String, CliProfile>,
}

impl ProfileRegistry {
    /// Registry vazio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insere (ou sobrescreve, por `id`) um profile.
    pub fn insert(&mut self, profile: CliProfile) {
        self.profiles.insert(profile.id.clone(), profile);
    }

    /// Profile por `id`, se carregado.
    pub fn get(&self, id: &str) -> Option<&CliProfile> {
        self.profiles.get(id)
    }

    /// `id`s carregados, em ordem estável (BTree).
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.profiles.keys().map(String::as_str)
    }

    /// Quantidade de profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// `true` se nenhum profile foi carregado.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Carrega todos os `*.toml` de um diretório (não recursivo), em ordem
    /// determinística. Falha no primeiro arquivo inválido, com o caminho no erro.
    // TODO(W0-8+): semear com os profiles default embutidos antes de varrer o
    //   diretório do usuário, para que custom sobrescreva default por `id`.
    pub fn load_dir(dir: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let dir = dir.as_ref();
        let read = std::fs::read_dir(dir).map_err(|source| ProfileError::Io {
            path: dir.display().to_string(),
            source,
        })?;

        let mut files: Vec<PathBuf> = Vec::new();
        for entry in read {
            let entry = entry.map_err(|source| ProfileError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                files.push(path);
            }
        }
        files.sort();

        let mut registry = Self::new();
        for path in files {
            registry.insert(CliProfile::load_file(&path)?);
        }
        Ok(registry)
    }
}

// ═══════════════════ CliDetector — camada 1 (F1-1-1 / ADR de detecção) ═══════════════════

/// Confiança da camada 1: o spawn É a evidência — determinística por construção.
pub const LAYER1_CONFIDENCE: f32 = 1.0;

/// Evidência canônica da camada 1 (campo `evidence` do `CliDetected`).
pub const LAYER1_EVIDENCE: &str = "spawn-profile";

/// Resultado tipado de uma detecção de CLI — o espelho 1:1 do evento canônico
/// `CliDetected{node, cli, confidence, evidence}` do ADR de detecção.
///
/// **Emissão desacoplada (fronteira de crates):** este crate NÃO conhece o event
/// store; o supervisor (`lina-core`) converte esta struct em `DomainEvent::CliDetected`
/// e apende ao log. `node_id` é `String` aqui pelo mesmo motivo (o `NodeId` real é
/// tipo do core — a autoridade de cunhagem continua lá).
#[derive(Debug, Clone, PartialEq)]
pub struct CliDetection {
    /// Nó (terminal) em que o CLI foi detectado.
    pub node_id: String,
    /// `id` do [`CliProfile`] detectado (ex.: `"claude-code"`).
    pub cli: String,
    /// Confiança agregada da detecção (camada 1 = [`LAYER1_CONFIDENCE`]).
    pub confidence: f32,
    /// Qual evidência sustenta a detecção (camada 1 = [`LAYER1_EVIDENCE`]).
    pub evidence: &'static str,
}

/// Detector de CLI — **camada 1 (spawn-time)**, a única determinística e a primária
/// (ADR de detecção; pesquisa 13.9).
///
/// Os CLIs "conhecidos" são EXATAMENTE os ids dos perfis TOML carregados no boot
/// ([`ProfileRegistry`]) — **nenhuma lista compilada** (resolve a tensão `KNOWN_CLIS`
/// hardcoded × invariante #3: novos CLIs entram sem recompilar). A camada 2 (inspeção
/// de processo externo) foi **refutada** pela pesquisa (env var falseável, `proc_pidinfo`
/// self-bound, sem caso de uso) — não existe e não deve ser construída; terminal externo,
/// se um dia virar requisito, entra por registro autorizado (`lina register-self`),
/// nunca por introspecção. Camadas 3/4 só AGREGAM confiança a jusante, nunca decidem.
#[derive(Debug, Clone, Copy)]
pub struct CliDetector<'r> {
    registry: &'r ProfileRegistry,
}

impl<'r> CliDetector<'r> {
    /// Detector sobre os perfis carregados no boot.
    #[must_use]
    pub fn new(registry: &'r ProfileRegistry) -> Self {
        Self { registry }
    }

    /// Camada 1, síncrona no spawn: o supervisor spawnou `node_id` com o profile
    /// `profile_id` → se o profile está carregado, a detecção é determinística
    /// (`confidence 1.0`, `evidence "spawn-profile"`). Profile desconhecido é erro
    /// estruturado e acionável (cita os ids carregados), nunca panic.
    pub fn detect_spawn(
        &self,
        node_id: impl Into<String>,
        profile_id: &str,
    ) -> Result<CliDetection, DetectError> {
        let Some(profile) = self.registry.get(profile_id) else {
            return Err(DetectError::UnknownProfile {
                id: profile_id.to_owned(),
                known: self.registry.ids().map(str::to_owned).collect::<Vec<_>>(),
            });
        };
        Ok(CliDetection {
            node_id: node_id.into(),
            cli: profile.id.clone(),
            confidence: LAYER1_CONFIDENCE,
            evidence: LAYER1_EVIDENCE,
        })
    }
}

/// Erros do detector — estruturados e acionáveis, nunca panic.
#[derive(Debug, Error)]
pub enum DetectError {
    /// O spawn referenciou um profile que não está no registry do boot.
    #[error("profile de CLI desconhecido '{id}' (carregados: {})", known.join(", "))]
    UnknownProfile { id: String, known: Vec<String> },
}

/// Erros do carregamento de CLI Profiles. Sempre acionáveis (carregam o caminho),
/// nunca um panic — o caller decide como reagir.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// Falha de I/O ao ler o arquivo/diretório de profiles.
    #[error("falha de I/O ao acessar '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// TOML malformado ou faltando campo obrigatório (a `toml` aponta linha/coluna).
    #[error("TOML de profile inválido em '{path}': {source}")]
    Parse {
        path: String,
        #[source]
        source: Box<toml::de::Error>,
    },

    /// Campo obrigatório presente porém vazio (ex.: `id = ""`).
    #[error("campo obrigatório '{field}' vazio no profile em '{path}'")]
    EmptyField { field: &'static str, path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Caminho do exemplo `profiles/claude-code.toml` na raiz do repo, robusto ao cwd.
    fn claude_code_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/claude-code.toml")
    }

    /// Diretório `profiles/` na raiz do repo.
    fn profiles_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles")
    }

    #[test]
    fn parses_claude_code_example_and_exposes_key_fields() {
        let profile = CliProfile::load_file(claude_code_path())
            .expect("o exemplo claude-code.toml deve parsear");

        // Identidade.
        assert_eq!(profile.id, "claude-code");
        assert_eq!(profile.program, "claude");

        // Os 4 campos-chave do gate de aceite (W0-8).
        // F1-0-2: submit_delay calibrado 300→400ms (story F1-0-2; margem contra o
        // paste-colapsado observado na baseline).
        assert_eq!(profile.delivery, Delivery::PtyInject);
        assert_eq!(profile.submit_delay_ms, 400);
        assert_eq!(profile.submit_delay(), Duration::from_millis(400));

        // F1-0-2 (vigência da calibração de prontidão): o perfil REAL declara o budget
        // do wait_ready e os busy markers extraídos dos grids da baseline.
        assert_eq!(
            profile.ready_timeout_ms,
            Some(30_000),
            "ready_timeout_ms calibrado (timeout → não injeta)"
        );
        assert_eq!(
            profile.busy_markers,
            vec!["esc to interrupt", "Press up to edit queued messages"],
            "busy markers dos grids reais (stuck-11/15 da baseline)"
        );
        assert!(
            !profile.prompt_ready_regex.trim().is_empty(),
            "prompt_ready_regex deve estar presente"
        );
        assert_eq!(
            profile.end_signal,
            EndSignal::StreamJson {
                event_type: "result".to_string()
            }
        );

        // F1-1-1 (regressão do bug de posicionamento): idle_ms/ask_timeout_ms são
        // top-level; escritos APÓS [end_signal] eram engolidos silenciosamente
        // (viravam chaves de end_signal, ignoradas) → profile.idle_ms dava None.
        assert_eq!(
            profile.idle_ms,
            Some(1500),
            "idle_ms deve chegar ao profile"
        );
        assert_eq!(profile.ask_timeout_ms, Some(120_000));

        // F1-1-1: o profile real declara os campos estendidos (13.9 item 1).
        assert_eq!(
            profile.session_dir_pattern.as_deref(),
            Some("~/.claude/projects/*/*.jsonl"),
            "session_dir_pattern do claude-code (consumido por F1-1-2)"
        );
        assert!(
            !profile.output_markers.is_empty(),
            "output_markers do claude-code (telemetria camada 4 — nunca decisória)"
        );
        assert!(profile.capabilities.has("hooks"), "claude-code tem hooks");
        assert!(profile.capabilities.has("otel"), "claude-code exporta OTel");
        assert!(
            profile.capabilities.has("permission_detection"),
            "claude-code sinaliza permissão via hook Notification"
        );
        assert!(
            profile.capabilities.has("handoff_support"),
            "claude-code suporta handoff A2A completo (13.9 item 5)"
        );
    }

    // ── F1-1-1: campos estendidos do CliProfile (13.9 item 1 — "o trabalho real") ──

    #[test]
    fn extended_fields_parse_from_toml() {
        let toml_src = r#"
            id = "fake"
            program = "fake"
            delivery = "pty_inject"
            prompt_ready_regex = "> "
            session_dir_pattern = "~/.fake/sessions/*/*.jsonl"
            output_markers = ["FAKE BANNER v", "fake> "]
            [env]
            FAKE_NON_INTERACTIVE = "1"
            [capabilities]
            hooks = true
            otel = false
            [end_signal]
            kind = "idle"
        "#;
        let p = CliProfile::from_toml_str(toml_src, "<inline>").expect("deve parsear");
        assert_eq!(
            p.session_dir_pattern.as_deref(),
            Some("~/.fake/sessions/*/*.jsonl")
        );
        assert_eq!(p.output_markers, vec!["FAKE BANNER v", "fake> "]);
        assert_eq!(
            p.env.get("FAKE_NON_INTERACTIVE").map(String::as_str),
            Some("1")
        );
        // Capability declarada true / declarada false / ausente — ausente NUNCA panica.
        assert!(p.capabilities.has("hooks"));
        assert!(!p.capabilities.has("otel"));
        assert!(!p.capabilities.has("nunca-declarada"));
    }

    #[test]
    fn extended_fields_default_when_absent() {
        // TOML antigo (pré-F1-1-1) segue válido: campos novos têm default vazio.
        let minimal = r#"
            id = "shell"
            program = "bash"
            delivery = "pty_inject"
            prompt_ready_regex = '\$\s$'
            [end_signal]
            kind = "idle"
        "#;
        let p = CliProfile::from_toml_str(minimal, "<inline>").expect("deve parsear");
        assert_eq!(p.session_dir_pattern, None);
        assert!(p.output_markers.is_empty());
        assert!(p.env.is_empty());
        assert!(
            !p.capabilities.has("hooks"),
            "sem declaração = sem capability"
        );
    }

    // ── F1-0-2: prontidão de entrega (ready_timeout_ms + busy_markers) ──

    #[test]
    fn readiness_fields_parse_from_toml() {
        let toml_src = r#"
            id = "fake"
            program = "fake"
            delivery = "pty_inject"
            prompt_ready_regex = "> "
            ready_timeout_ms = 30000
            busy_markers = ["esc to interrupt", "queued messages"]
            [end_signal]
            kind = "idle"
        "#;
        let p = CliProfile::from_toml_str(toml_src, "<inline>").expect("deve parsear");
        assert_eq!(p.ready_timeout_ms, Some(30_000));
        assert_eq!(p.ready_timeout(), Duration::from_millis(30_000));
        assert_eq!(p.busy_markers, vec!["esc to interrupt", "queued messages"]);
    }

    #[test]
    fn readiness_fields_default_when_absent() {
        // TOML antigo (pré-F1-0-2) segue válido: budget default de 2s (o READY_TIMEOUT
        // histórico de W0-9) e zero busy_markers (só o prompt decide).
        let minimal = r#"
            id = "shell"
            program = "bash"
            delivery = "pty_inject"
            prompt_ready_regex = '\$\s$'
            [end_signal]
            kind = "idle"
        "#;
        let p = CliProfile::from_toml_str(minimal, "<inline>").expect("deve parsear");
        assert_eq!(p.ready_timeout_ms, None);
        assert_eq!(
            p.ready_timeout(),
            Duration::from_millis(2_000),
            "default preserva o comportamento histórico (READY_TIMEOUT 2s)"
        );
        assert!(p.busy_markers.is_empty());
    }

    #[test]
    fn empty_session_dir_pattern_is_rejected_semantically() {
        // Presente porém vazio é typo/erro de config — falha clara, não None silencioso.
        let bad = r#"
            id = "x"
            program = "x"
            delivery = "pty_inject"
            prompt_ready_regex = "> "
            session_dir_pattern = "   "
            [end_signal]
            kind = "idle"
        "#;
        let err =
            CliProfile::from_toml_str(bad, "<inline>").expect_err("pattern vazio deve falhar");
        assert!(matches!(
            err,
            ProfileError::EmptyField {
                field: "session_dir_pattern",
                ..
            }
        ));
    }

    #[test]
    fn invalid_toml_is_a_clear_error_without_panic() {
        // TOML sintaticamente quebrado (chave sem valor).
        let broken = "id = \nprogram = \"x\"\n";
        let err =
            CliProfile::from_toml_str(broken, "<inline>").expect_err("TOML quebrado deve falhar");
        assert!(matches!(err, ProfileError::Parse { .. }));
        // A mensagem é acionável: cita o label.
        assert!(err.to_string().contains("<inline>"));
    }

    #[test]
    fn missing_required_field_is_a_clear_error() {
        // Falta `delivery`, `prompt_ready_regex`, `end_signal`.
        let incomplete = r#"
            id = "x"
            program = "x"
        "#;
        let err = CliProfile::from_toml_str(incomplete, "<inline>")
            .expect_err("faltar campo obrigatório deve falhar");
        assert!(matches!(err, ProfileError::Parse { .. }));
    }

    #[test]
    fn unknown_field_is_rejected() {
        // `deny_unknown_fields`: um typo (`programm`) não passa silenciosamente.
        let typo = r#"
            id = "x"
            programm = "claude"
            delivery = "pty_inject"
            prompt_ready_regex = "> "
            [end_signal]
            kind = "idle"
        "#;
        let err = CliProfile::from_toml_str(typo, "<inline>")
            .expect_err("campo desconhecido deve falhar");
        assert!(matches!(err, ProfileError::Parse { .. }));
    }

    #[test]
    fn empty_id_is_rejected_semantically() {
        let empty_id = r#"
            id = "   "
            program = "claude"
            delivery = "pty_inject"
            prompt_ready_regex = "> "
            [end_signal]
            kind = "idle"
        "#;
        let err = CliProfile::from_toml_str(empty_id, "inline-empty-id")
            .expect_err("id vazio deve falhar");
        assert!(matches!(err, ProfileError::EmptyField { field: "id", .. }));
    }

    #[test]
    fn delivery_and_end_signal_variants_parse() {
        // session_resume + end_signal regex (cobre os outros braços dos enums).
        let codex = r#"
            id = "codex"
            program = "codex"
            args = ["--json"]
            delivery = "session_resume"
            submit_delay_ms = 150
            prompt_ready_regex = '(?m)^codex>\s'
            idle_ms = 800
            [end_signal]
            kind = "regex"
            pattern = '(?m)^\[done\]\s*$'
        "#;
        let profile = CliProfile::from_toml_str(codex, "<inline>").expect("codex deve parsear");
        assert_eq!(profile.delivery, Delivery::SessionResume);
        assert_eq!(profile.submit_delay_ms, 150);
        assert_eq!(profile.idle_ms, Some(800));
        assert_eq!(
            profile.end_signal,
            EndSignal::Regex {
                pattern: r"(?m)^\[done\]\s*$".to_string()
            }
        );

        // submit_delay omitido cai no default 300.
        let minimal = r#"
            id = "shell"
            program = "bash"
            delivery = "pty_inject"
            prompt_ready_regex = '\$\s$'
            [end_signal]
            kind = "idle"
        "#;
        let profile = CliProfile::from_toml_str(minimal, "<inline>").expect("shell deve parsear");
        assert_eq!(profile.submit_delay_ms, DEFAULT_SUBMIT_DELAY_MS);
        assert_eq!(profile.end_signal, EndSignal::Idle);
        assert!(profile.args.is_empty());
    }

    #[test]
    fn registry_loads_the_profiles_directory() {
        let registry = ProfileRegistry::load_dir(profiles_dir()).expect("profiles/ deve carregar");
        assert!(!registry.is_empty());
        let claude = registry
            .get("claude-code")
            .expect("registry deve conter claude-code");
        assert_eq!(claude.program, "claude");
    }

    #[test]
    fn registry_same_id_overrides() {
        // Custom vence default de mesmo `id` (W0-8).
        let mut registry = ProfileRegistry::new();
        let default = CliProfile::from_toml_str(
            r#"
                id = "claude-code"
                program = "claude"
                delivery = "pty_inject"
                prompt_ready_regex = "> "
                [end_signal]
                kind = "idle"
            "#,
            "<default>",
        )
        .expect("default deve parsear");
        let custom = CliProfile::from_toml_str(
            r#"
                id = "claude-code"
                program = "claude-wrapper"
                delivery = "pty_inject"
                prompt_ready_regex = "> "
                [end_signal]
                kind = "idle"
            "#,
            "<custom>",
        )
        .expect("custom deve parsear");

        registry.insert(default);
        registry.insert(custom);
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.get("claude-code").map(|p| p.program.as_str()),
            Some("claude-wrapper")
        );
    }

    #[test]
    fn load_dir_on_missing_directory_errors_cleanly() {
        let err = ProfileRegistry::load_dir(profiles_dir().join("does-not-exist"))
            .expect_err("diretório inexistente deve falhar");
        assert!(matches!(err, ProfileError::Io { .. }));
    }

    // ── F1-1-1: CliDetector camada 1 (ADR de detecção: spawn-time, determinística, primária) ──

    #[test]
    fn detector_layer1_detects_from_spawn_profile() {
        let registry = ProfileRegistry::load_dir(profiles_dir()).expect("profiles/ carrega");
        let detector = CliDetector::new(&registry);
        let d = detector
            .detect_spawn("node-abc", "claude-code")
            .expect("spawn com profile carregado detecta");
        assert_eq!(d.node_id, "node-abc");
        assert_eq!(d.cli, "claude-code");
        assert!(
            (d.confidence - 1.0).abs() < f32::EPSILON,
            "camada 1 é determinística: confidence 1.0"
        );
        assert_eq!(d.evidence, "spawn-profile");
    }

    #[test]
    fn detector_unknown_profile_is_structured_actionable_error() {
        let registry = ProfileRegistry::load_dir(profiles_dir()).expect("profiles/ carrega");
        let detector = CliDetector::new(&registry);
        let err = detector
            .detect_spawn("node-abc", "cli-que-nao-existe")
            .expect_err("profile desconhecido deve falhar estruturado");
        assert!(matches!(err, DetectError::UnknownProfile { .. }));
        // Acionável: cita o id pedido E os ids realmente carregados.
        let msg = err.to_string();
        assert!(msg.contains("cli-que-nao-existe"));
        assert!(msg.contains("claude-code"));
    }

    /// Critério 3 da story (anti-hardcode × invariante #3): um TOML novo no diretório
    /// é SUFICIENTE para o detector conhecer o CLI — zero lista compilada, sem recompilar.
    #[test]
    fn detector_derives_known_clis_from_loaded_tomls_not_hardcode() {
        // Tempdir manual (sem dev-dep): nome único por processo + teste.
        let dir =
            std::env::temp_dir().join(format!("lina-clip-antihardcode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("criar tempdir");
        let fake = r#"
            id = "fake-cli"
            program = "fakecli"
            delivery = "pty_inject"
            prompt_ready_regex = "> "
            session_dir_pattern = "~/.fake/sessions/*.jsonl"
            [capabilities]
            hooks = true
            [end_signal]
            kind = "idle"
        "#;
        std::fs::write(dir.join("fake-cli.toml"), fake).expect("escrever fake-cli.toml");

        let registry = ProfileRegistry::load_dir(&dir).expect("dir com fake carrega");
        let detector = CliDetector::new(&registry);
        let d = detector
            .detect_spawn("node-x", "fake-cli")
            .expect("CLI novo descoberto só pelo TOML");
        assert_eq!(d.cli, "fake-cli");
        assert!((d.confidence - 1.0).abs() < f32::EPSILON);
        assert_eq!(d.evidence, "spawn-profile");

        let _ = std::fs::remove_dir_all(&dir); // cleanup best-effort
    }

    // ── instaladores ──

    const SAMPLE_INSTALLERS: &str = r#"
        [opencode.macos]
        program = "bash"
        args = ["-c", "curl -fsSL https://opencode.ai/install | bash"]
        verify_paths = ["~/.opencode/bin"]
        [opencode.windows]
        program = "npm"
        args = ["install", "-g", "opencode-ai"]
        [codex.macos]
        program = "bash"
        args = ["-c", "sh"]
        env = { CODEX_NON_INTERACTIVE = "1" }
    "#;

    #[test]
    fn installers_parse_recipe_env_and_verify_paths() {
        let inst = Installers::from_toml_str(SAMPLE_INSTALLERS, "<inline>").expect("parseia");
        let oc = inst.0["opencode"].for_os("macos").expect("opencode macos");
        assert_eq!(oc.program, "bash");
        assert_eq!(
            oc.args,
            vec!["-c", "curl -fsSL https://opencode.ai/install | bash"]
        );
        assert_eq!(oc.verify_paths, vec!["~/.opencode/bin".to_string()]);
        // SO ausente → None (não inventa receita).
        assert!(inst.0["opencode"].for_os("linux").is_none());
        // env vira parte da receita (não-interatividade do codex).
        let cx = inst.0["codex"].for_os("macos").expect("codex macos");
        assert_eq!(
            cx.env.get("CODEX_NON_INTERACTIVE").map(String::as_str),
            Some("1")
        );
        // recipe_for() resolve pelo SO de compilação corrente.
        assert!(inst.recipe_for("inexistente").is_none());
    }

    /// O `installers.toml` REAL do repo parseia e cobre os 5 ids com receita p/ os 3 SOs, e o
    /// copilot NÃO usa o pacote deprecado (regressão do bug do binário `github-copilot-cli`).
    #[test]
    fn real_installers_toml_is_valid_and_complete() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/installers/recipes.toml");
        let src = std::fs::read_to_string(&path).expect("ler installers.toml");
        let inst = Installers::from_toml_str(&src, &path.display().to_string())
            .expect("installers.toml deve parsear");
        for id in ["claude", "codex", "gemini", "opencode", "copilot", "agy"] {
            let p = inst
                .0
                .get(id)
                .unwrap_or_else(|| panic!("falta receita p/ {id}"));
            for os in ["macos", "linux", "windows"] {
                let r = p.for_os(os).unwrap_or_else(|| panic!("falta {id}.{os}"));
                assert!(!r.program.trim().is_empty(), "{id}.{os} sem program");
            }
        }
        // copilot moderno (binário `copilot`), nunca o deprecado.
        let cp = format!("{:?}", inst.0["copilot"]);
        assert!(
            cp.contains("@github/copilot"),
            "copilot deve usar @github/copilot"
        );
        assert!(
            !cp.contains("githubnext"),
            "copilot não pode usar o pacote deprecado"
        );
    }
}
