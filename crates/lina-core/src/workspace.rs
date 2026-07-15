//! F1-4-1 — Multi-workspace de verdade: **Espaço = projeto**, tudo event-sourced.
//!
//! ## Layout de storage (mini-ADR na entrega `.entrega-f1-4-1.md`)
//! **Um event store POR Espaço**, em `<root>/.lina/events` — o layout que o ADR 0022 §6
//! já canoniza como "a única fonte da verdade do Espaço". Isolamento físico = isolamento
//! de falha: o log de B não contém eventos de A **por construção** (critérios 1-4 do
//! gate), e a contenção de lock SQLite não cresce com o nº de Espaços — a lição W5
//! registra que foi a UNIFICAÇÃO de stores que ativou a concorrência multi-escritor;
//! `busy_timeout` + retry de `seq` (events.rs, "FURO A") seguem cobrindo o caso real de
//! 2 conexões no MESMO store (critério 5).
//!
//! ## Registry global = PONTEIRO, nunca autoridade
//! `~/.lina/workspaces.json` lista os Espaços conhecidos (id, nome, path, last_focus).
//! id/nome/arquivado vêm do LOG do próprio Espaço ([`crate::DomainEvent::WorkspaceIdAssigned`]
//! / `WorkspaceCreated` / `WorkspaceArchived`); corrupção do registry nunca perde um
//! Espaço — a entrada se reconstrói com [`WorkspaceRegistry::rederive_entry`]
//! (invariante #4: o log é a verdade; projeções e ponteiros são re-deriváveis).
//!
//! ## Seam ÚNICO de cwd (decisão 4 do despacho; precedência do ADR 0022)
//! [`resolve_spawn_cwd`] é a função PURA que o app consome ao admitir um nó:
//! `default_cwd` do Espaço ativo → senão fallback dir gerenciado virgem `n-<key>`.
//! ⚠️ O seam **não assume cwd-único-por-nó**: N nós no mesmo `default_cwd` é o
//! comportamento PADRÃO do produto (colaboração no mesmo projeto); identidade de
//! terminal vem do spawn (env `LINA_NODE_ID`, ADR 0026 em rascunho) — nunca do cwd.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::events::{DomainEvent, EventStore, ProjectedState, StoreError};
use lina_host::UiHost;

/// Erros do módulo de multi-Espaço.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// O registry é PONTEIRO: corrupção é visível (nunca engolida) e recuperável —
    /// re-derive as entradas dos stores via [`WorkspaceRegistry::rederive_entry`].
    #[error("registry ilegível (ponteiro corrompido — re-derive dos stores): {0}")]
    Json(#[from] serde_json::Error),
    /// Gating free/PRO: o plano atual não permite criar mais Espaços ATIVOS.
    #[error("limite de Espaços do plano atingido ({limit})")]
    LimitReached { limit: usize },
    /// Já existe um Espaço neste diretório (o log tem `WorkspaceCreated`).
    #[error("já existe um Espaço em {root}")]
    AlreadyExists { root: PathBuf },
    /// O diretório não contém um Espaço (sem store ou log sem `WorkspaceCreated`).
    #[error("{root} não é um Espaço (log sem WorkspaceCreated)")]
    NotAWorkspace { root: PathBuf },
    /// Uma criação interrompida chegou a gravar o nome, mas não a identidade que
    /// fecha a publicação. Ela permanece no disco para recuperação, sem entrar no
    /// catálogo nem ser montada como se fosse um Espaço pronto.
    #[error("{root} contém uma criação incompleta (sem WorkspaceIdAssigned)")]
    IncompleteWorkspace { root: PathBuf },
    /// Caminho não-UTF8 recusado VISIVELMENTE (revisão F1-4-1): o evento persiste
    /// String/JSON — mutilar para U+FFFD gravaria um caminho que não existe no disco
    /// (degradação silenciosa, proibida). A validação roda ANTES de qualquer append.
    #[error("caminho não é UTF-8 válido: {path:?}")]
    NonUtf8Path { path: PathBuf },
    /// Red-team (b) da revisão F1-4-1: a entrada do registry (PONTEIRO, adulterável
    /// por quem escreve o disco) não bate com a identidade RE-DERIVADA do store (a
    /// autoridade) — nunca abrir o Espaço errado em silêncio.
    #[error("ponteiro do registry não bate com o Espaço no disco (esperado {expected}, encontrado {found})")]
    PointerMismatch { expected: String, found: String },
    /// A reconstrução aceita somente fatos já conferidos nos stores. IDs ou caminhos
    /// duplicados tornariam o ponteiro ambíguo e, portanto, não podem ser publicados.
    #[error("registry não pôde ser reconstruído: {reason}")]
    InvalidRegistryRebuild { reason: String },
    /// A foto que motivou uma reconstrução ficou velha antes do write. Sob o lock,
    /// o caller precisa reler e reconciliar; sobrescrever apagaria uma criação/foco
    /// que outro processo acabou de publicar.
    #[error("o registry mudou durante a reconstrução de {path}; releia o catálogo")]
    RegistryChangedDuringRebuild { path: PathBuf },
    /// O rename já publicou um store completo, mas a abertura de confirmação falhou.
    /// O caller deve adotar `root` quando o recurso voltar; nunca iniciar outra criação.
    #[error("Espaço completo publicado em {root}, mas indisponível para reabertura: {detail}")]
    PublishedButUnavailable { root: PathBuf, detail: String },
    /// A criação falhou antes da publicação e a remoção do staging também
    /// falhou. O erro conserva as duas causas: esconder a segunda deixaria lixo sem
    /// dono e tornaria impossível diagnosticar uma futura colisão.
    #[error(
        "falha ao preparar Espaço em {root}: {operation}; a limpeza de {staging} também falhou: {cleanup}"
    )]
    CreationCleanupFailed {
        root: PathBuf,
        staging: PathBuf,
        operation: String,
        cleanup: String,
    },
    /// O journal de foco tem campos vazios, diverge da intenção esperada ou
    /// aponta para um store cuja identidade mudou.
    #[error("intenção de foco inválida: {reason}")]
    InvalidFocusIntent { reason: String },
    /// O journal foi preparado, mas o evento que constitui o commit ainda não
    /// existe no store alvo. Só o recovery pode abortar e limpar este estado.
    #[error("o foco {focus_id} ainda não foi confirmado no log do Espaço alvo")]
    FocusNotCommitted { focus_id: String },
    /// Outra instância mantém a transação prepare→append→commit. O timeout é zero:
    /// o caller recebe a contenção imediatamente, sem congelar a navegação.
    #[error("outra instância está concluindo uma troca de Espaço; tente novamente")]
    FocusTransactionBusy { lock: PathBuf },
    /// O banco dedicado de exclusão não pôde ser aberto/iniciado. Ele não guarda
    /// dados de domínio; existe apenas para o lock interprocesso liberado pelo SO.
    #[error("não consegui proteger a troca de Espaço em {lock}: {detail}")]
    FocusTransactionLock { lock: PathBuf, detail: String },
}

impl WorkspaceError {
    /// Raiz que já foi publicada apesar da falha retornada. `Some` significa que
    /// repetir semanticamente a criação seria incorreto; o caminho seguro é reabrir.
    #[must_use]
    pub fn published_root(&self) -> Option<&Path> {
        match self {
            Self::PublishedButUnavailable { root, .. } => Some(root),
            _ => None,
        }
    }
}

/// Conversão UTF-8 VISÍVEL de paths que entram em eventos (String/JSON): falha alto
/// em vez de mutilar (ver [`WorkspaceError::NonUtf8Path`]).
fn utf8_path(p: &Path) -> Result<&str, WorkspaceError> {
    p.to_str().ok_or_else(|| WorkspaceError::NonUtf8Path {
        path: p.to_path_buf(),
    })
}

// ───────────────────────────── gating free=1 / PRO=N ─────────────────────────────

/// Tier de licença para o gating de criação de Espaços. A VALIDAÇÃO da licença
/// (ed25519, ativação offline) é outra story da onda F1-4 — este módulo recebe o
/// tier já resolvido e aplica o limite. A licença gateia o NÚMERO de workspaces,
/// nunca a segurança (ADR 0010 §Limite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseTier {
    /// Plano gratuito: **1 Espaço ativo** (decisão do Maestro nesta fatia; o gate da
    /// onda testa free=1).
    Free,
    /// Licença PRO: N Espaços (sem limite imposto pelo core).
    Pro,
}

/// Limite de Espaços ATIVOS do tier (`None` = ilimitado).
#[must_use]
pub fn workspace_limit(tier: LicenseTier) -> Option<usize> {
    match tier {
        LicenseTier::Free => Some(1),
        LicenseTier::Pro => None,
    }
}

/// Seam PURO de gating: pode criar mais um Espaço com `active_count` ativos?
/// Arquivados não contam (a vaga libera — ver `WorkspaceArchived`).
pub fn can_create_workspace(tier: LicenseTier, active_count: usize) -> Result<(), WorkspaceError> {
    match workspace_limit(tier) {
        Some(limit) if active_count >= limit => Err(WorkspaceError::LimitReached { limit }),
        _ => Ok(()),
    }
}

// ───────────────────────────── seam único de cwd ─────────────────────────────

/// Resultado do seam de cwd — carrega a POLÍTICA, não só o caminho, para o app mapear
/// ao seu `CwdPolicy` (kit de integração) sem re-decidir a precedência.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCwd {
    /// O `default_cwd` do Espaço — compartilhamento INTENCIONAL: todo nó novo do
    /// Espaço nasce aqui (colaboram no mesmo projeto).
    WorkspaceDefault(PathBuf),
    /// Fallback do ADR 0022: dir gerenciado VIRGEM `n-<key>` sob a raiz do Espaço
    /// (isola contra herança de slot reciclado). Único por nó.
    ManagedVirgin(PathBuf),
}

impl ResolvedCwd {
    /// O caminho resolvido (qualquer que seja a política).
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            ResolvedCwd::WorkspaceDefault(p) | ResolvedCwd::ManagedVirgin(p) => p,
        }
    }
}

/// **Seam ÚNICO de resolução de cwd** (decisão 4 do despacho F1-4-1; precedência do
/// ADR 0022): com `default_cwd` definido no Espaço → todo nó novo nasce NELE; sem →
/// fallback dir gerenciado virgem `n-<key>` (comportamento atual do app). Função PURA
/// (sem filesystem, sem relógio): o app consome no funil de admissão; trocar o
/// `default_cwd` (novo `WorkspaceDefaultCwdSet`) afeta só chamadas FUTURAS.
///
/// ⚠️ NÃO assume cwd-único-por-nó: chamadas com o mesmo `default_cwd` devolvem o MESMO
/// caminho para nós distintos, por design (BUG-1 do dogfooding/ADR 0026: identidade de
/// terminal vem do env do spawn, nunca do cwd).
///
/// Defesa em profundidade (doutrina §7): a `node_key` vira UM componente literal —
/// separadores são sanitizados, então `..` embutido nunca escapa da raiz do Espaço.
/// Hoje só o app gera a key (uuid), mas o seam não confia nisso.
///
/// **Contrato de `node_key` (fiação M8/M9, atenção):** id opaco do nó **SEM** o
/// prefixo `n-` — o SEAM prefixa. O app hoje gera `format!("n-{uuid}")` em
/// `bridge.rs` (`alloc` do dir gerenciado); plugar essa key aqui sem tirar o
/// prefixo produziria `n-n-<uuid>` — a rodada de fiação troca a geração da key.
#[must_use]
pub fn resolve_spawn_cwd(
    default_cwd: Option<&Path>,
    ws_root: &Path,
    node_key: &str,
) -> ResolvedCwd {
    match default_cwd {
        Some(dir) => ResolvedCwd::WorkspaceDefault(dir.to_path_buf()),
        None => {
            let safe_key: String = node_key
                .chars()
                .map(|c| {
                    if std::path::is_separator(c) || c == '\\' {
                        '-'
                    } else {
                        c
                    }
                })
                .collect();
            ResolvedCwd::ManagedVirgin(ws_root.join(format!("n-{safe_key}")))
        }
    }
}

// ───────────────────────────── worktree por agente (spec 36 §1) ─────────────────────────────

/// Decisão PURA do NOME da branch de worktree de um agente (spec 36 §1): `lina/<slug>`, com
/// `<slug>` = o nome do agente reduzido a um refname git válido (minúsculas; cada corrida de
/// caracteres não-alfanuméricos-ASCII vira um único `-`; sem `-` nas pontas). O app EXECUTA
/// `git worktree add -b <branch>`; o core só decide o nome (inv#7, core git-free).
///
/// A unicidade da branch vem da unicidade do nome do agente no Espaço — nomes distintos geram
/// branches distintas (base do aceite F3-4-1). Nome vazio / só-símbolos cai no fallback
/// `lina/agente` (nunca um refname inválido como `lina/` ou `lina/.`).
#[must_use]
pub fn worktree_branch_name(agent_name: &str) -> String {
    let mut slug = String::with_capacity(agent_name.len());
    let mut pending_dash = false;
    for c in agent_name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c.to_ascii_lowercase());
        } else {
            // Qualquer corrida de não-alfanuméricos-ASCII (espaço, `/`, `.`, acento, símbolo)
            // colapsa em UM `-` — e só é materializada quando vier outro alfanumérico, então
            // nunca sobra `-` nas pontas (refname inválido).
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        "lina/agente".to_string()
    } else {
        format!("lina/{slug}")
    }
}

// (O PATH físico da worktree — `<ws_root>/wt-<key>` — é plumbing do APP: vive em
// `bridge.rs::worktree_path`, ao lado de `effective_managed_policy` e `ensure_cwd`, que já
// resolvem caminhos lá. O core decide o NOME da branch — semântica; o app resolve o path —
// plumbing. Mantém o core git-free e a fronteira de re-export intocada nesta rodada.)

// ───────────────────────────── o Espaço ─────────────────────────────

/// Um Espaço ABERTO: raiz do projeto + seu event store (`<root>/.lina/events`).
/// Toda mutação é um evento no log do PRÓPRIO Espaço — nada vive só em memória ou
/// em arquivo paralelo (invariante #4).
pub struct Workspace {
    root: PathBuf,
    store: EventStore,
}

impl Workspace {
    /// O diretório do event store de um Espaço — **o** seam de layout (ADR 0022 §6:
    /// "`<ws>/.lina/events` é a única fonte da verdade do Espaço; sem terceiro store").
    #[must_use]
    pub fn events_dir(root: &Path) -> PathBuf {
        root.join(".lina").join("events")
    }

    /// Cria um Espaço novo em `root`: abre o store e apenda a sequência de criação —
    /// `WorkspaceCreated` (+nome/preset, como a galeria T3/W4-5 emite) +
    /// `WorkspaceIdAssigned` (identidade durável) + `WorkspaceDefaultCwdSet` quando o
    /// Diretório de Trabalho foi escolhido na criação (M9). Recusa criar por cima de
    /// um Espaço existente (o log já tem `WorkspaceCreated`).
    ///
    /// O GATING free/PRO é do chamador (ver [`can_create_workspace`] /
    /// [`WorkspaceRegistry::can_create`]) — criar exige saber quantos Espaços ATIVOS a
    /// máquina tem, e essa contagem vive no registry-ponteiro, não aqui.
    pub fn create(
        root: impl AsRef<Path>,
        name: &str,
        focus_preset: &str,
        default_cwd: Option<&Path>,
    ) -> Result<Self, WorkspaceError> {
        Self::create_seeded(
            root.as_ref().to_path_buf(),
            name,
            focus_preset,
            default_cwd,
            &[],
        )
    }

    /// Construtor atômico compartilhado pelo fluxo vazio e pelos gabaritos. Todos
    /// os fatos fornecidos em `seed_events` entram na mesma transação dos fatos
    /// de criação, antes do rename que torna o store catalogável.
    pub(crate) fn create_seeded(
        root: PathBuf,
        name: &str,
        focus_preset: &str,
        default_cwd: Option<&Path>,
        seed_events: &[DomainEvent],
    ) -> Result<Self, WorkspaceError> {
        Self::create_seeded_with_persist(
            root,
            name,
            focus_preset,
            default_cwd,
            seed_events,
            |store, events| {
                store.append_batch(events)?;
                Ok(())
            },
        )
    }

    /// Seam interno usado pelos testes de crash do construtor de gabaritos. A
    /// persistência é injetável, mas cleanup e reabertura seguem o caminho real.
    pub(crate) fn create_seeded_with_persist(
        root: PathBuf,
        name: &str,
        focus_preset: &str,
        default_cwd: Option<&Path>,
        seed_events: &[DomainEvent],
        persist: impl FnMut(&mut EventStore, &[DomainEvent]) -> Result<(), WorkspaceError>,
    ) -> Result<Self, WorkspaceError> {
        Self::create_seeded_with_seams(
            root,
            name,
            focus_preset,
            default_cwd,
            seed_events,
            persist,
            remove_creation_staging,
            |published_root| Self::open(published_root),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_seeded_with_seams(
        root: PathBuf,
        name: &str,
        focus_preset: &str,
        default_cwd: Option<&Path>,
        seed_events: &[DomainEvent],
        mut persist: impl FnMut(&mut EventStore, &[DomainEvent]) -> Result<(), WorkspaceError>,
        mut cleanup: impl FnMut(&Path) -> std::io::Result<()>,
        mut reopen: impl FnMut(&Path) -> Result<Self, WorkspaceError>,
    ) -> Result<Self, WorkspaceError> {
        // Valida ANTES de qualquer append: recusa de input não deixa Espaço meio-criado.
        let default_cwd = default_cwd.map(utf8_path).transpose()?.map(str::to_string);
        let projected_focus_preset = (!focus_preset.is_empty()).then_some(focus_preset);
        let events_dir = Self::events_dir(&root);
        if events_dir.try_exists()? {
            return Err(WorkspaceError::AlreadyExists { root });
        }

        // O store nasce num irmão oculto do destino e só aparece por rename depois de
        // projetar nome + seed + identidade (+ cwd quando pedido). Assim nenhum funil
        // publica um gabarito pela metade.
        let lina_dir = root.join(".lina");
        std::fs::create_dir_all(&lina_dir)?;
        let staging = lina_dir.join(format!(".events-create-{}", Uuid::now_v7()));
        let prepared = (|| -> Result<(), WorkspaceError> {
            let mut store = EventStore::open(&staging)?;
            let workspace_id = Uuid::now_v7().to_string();
            let mut events =
                Vec::with_capacity(seed_events.len() + 2 + usize::from(default_cwd.is_some()));
            events.push(DomainEvent::WorkspaceCreated {
                name: name.to_string(),
                focus_preset: focus_preset.to_string(),
            });
            events.extend_from_slice(seed_events);
            events.push(DomainEvent::WorkspaceIdAssigned {
                workspace_id: workspace_id.clone(),
            });
            if let Some(cwd) = default_cwd.as_ref() {
                events.push(DomainEvent::WorkspaceDefaultCwdSet { cwd: cwd.clone() });
            }
            persist(&mut store, &events)?;
            let state = store.project()?;
            if state.workspace_name.as_deref() != Some(name)
                || state.workspace_id.as_deref() != Some(workspace_id.as_str())
                || state.default_cwd.as_deref() != default_cwd.as_deref()
                || state.focus_preset.as_deref() != projected_focus_preset
                || state.archived
            {
                return Err(WorkspaceError::IncompleteWorkspace { root: root.clone() });
            }
            let persisted_count = store.event_count()?;
            if persisted_count
                != u64::try_from(events.len())
                    .map_err(|_| WorkspaceError::IncompleteWorkspace { root: root.clone() })?
            {
                return Err(WorkspaceError::IncompleteWorkspace { root: root.clone() });
            }
            Ok(())
        })();
        if let Err(error) = prepared {
            return Err(cleanup_failed_creation(
                &root,
                &staging,
                error,
                &mut cleanup,
            ));
        }
        if let Err(error) = std::fs::rename(&staging, &events_dir) {
            return Err(cleanup_failed_creation(
                &root,
                &staging,
                error.into(),
                &mut cleanup,
            ));
        }
        #[cfg(unix)]
        if let Err(error) = std::fs::File::open(&lina_dir).and_then(|dir| dir.sync_all()) {
            tracing::error!(
                path = %lina_dir.display(),
                %error,
                "Espaço publicado; fsync do diretório falhou sem invalidar a criação confirmada"
            );
        }
        reopen(&root).map_err(|error| WorkspaceError::PublishedButUnavailable {
            root,
            detail: error.to_string(),
        })
    }

    /// Abre um Espaço existente (sem recuperação — ver [`Workspace::open_or_recover`]).
    pub fn open(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = root.as_ref().to_path_buf();
        let store = EventStore::open(Self::events_dir(&root))?;
        Ok(Self { root, store })
    }

    /// Abre o Espaço apontado por uma entrada do registry e VERIFICA que o store
    /// re-deriva a MESMA identidade do ponteiro (red-team b da revisão F1-4-1: um
    /// registry adulterado com JSON válido não redireciona para o Espaço errado em
    /// silêncio). O ponteiro NUNCA é autoridade — a identidade vem do log.
    pub fn open_verified(entry: &WorkspaceEntry) -> Result<Self, WorkspaceError> {
        let ws = Self::open(&entry.path)?;
        let derived = ws.registry_entry()?;
        if derived.id != entry.id {
            return Err(WorkspaceError::PointerMismatch {
                expected: entry.id.clone(),
                found: derived.id,
            });
        }
        Ok(ws)
    }

    /// Abertura RESILIENTE (W0-6): se o `.db` do Espaço corrompeu, preserva e re-deriva
    /// tudo do espelho `log.jsonl` — visível via `UiHost`, nunca silencioso. A falha de
    /// UM Espaço não toca os demais (um store por Espaço — mini-ADR).
    pub fn open_or_recover(
        root: impl AsRef<Path>,
        ui: &mut dyn UiHost,
    ) -> Result<Self, WorkspaceError> {
        let root = root.as_ref().to_path_buf();
        let (store, _projected) =
            EventStore::open_projected_or_recover(Self::events_dir(&root), ui)?;
        Ok(Self { root, store })
    }

    /// Raiz do projeto deste Espaço.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Acesso ao event store do Espaço (appends de domínio, inspeção do log).
    pub fn store_mut(&mut self) -> &mut EventStore {
        &mut self.store
    }

    /// Estado projetado do Espaço — replay determinístico do SEU log, e só dele.
    pub fn project(&self) -> Result<ProjectedState, WorkspaceError> {
        Ok(self.store.project()?)
    }

    /// Define/troca o Diretório de Trabalho do Espaço (`None` — ou path VAZIO, a
    /// sentinela `""` do evento — limpa → fallback gerenciado). É EVENTO no log:
    /// persiste e re-deriva no replay; afeta só nós FUTUROS (nós vivos não migram —
    /// F1-2-4). Path não-UTF8 é recusado visivelmente, sem apendar nada.
    pub fn set_default_cwd(&mut self, cwd: Option<&Path>) -> Result<u64, WorkspaceError> {
        let cwd = match cwd {
            Some(p) => utf8_path(p)?.to_string(),
            None => String::new(),
        };
        Ok(self
            .store
            .append(&DomainEvent::WorkspaceDefaultCwdSet { cwd })?)
    }

    /// Renomeia o Espaço (evento; último vence).
    pub fn rename(&mut self, name: &str) -> Result<u64, WorkspaceError> {
        Ok(self.store.append(&DomainEvent::WorkspaceRenamed {
            name: name.to_string(),
        })?)
    }

    /// Arquiva o Espaço (evento; sai da contagem ativa do gating — a vaga libera).
    pub fn archive(&mut self) -> Result<u64, WorkspaceError> {
        Ok(self.store.append(&DomainEvent::WorkspaceArchived)?)
    }

    /// A entrada de registry deste Espaço, RE-DERIVADA do log (o registry é ponteiro:
    /// id/nome/arquivado vêm daqui; `last_focus` é conveniência do ponteiro e nasce 0).
    /// Ausência de `WorkspaceIdAssigned` significa criação incompleta; callers de migração
    /// precisam atribuir a identidade explicitamente antes de publicar a entrada.
    pub fn registry_entry(&self) -> Result<WorkspaceEntry, WorkspaceError> {
        let state = self.project()?;
        let name = state
            .workspace_name
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| WorkspaceError::NotAWorkspace {
                root: self.root.clone(),
            })?;
        let id = state
            .workspace_id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| WorkspaceError::IncompleteWorkspace {
                root: self.root.clone(),
            })?;
        Ok(WorkspaceEntry {
            id,
            name,
            path: self.root.clone(),
            last_focus: 0,
            archived: state.archived,
        })
    }
}

// ───────────────────────────── registry global (ponteiro) ─────────────────────────────

/// Uma entrada do registry: PONTEIRO para um Espaço conhecido. Todo campo de fato
/// (id, nome, arquivado) é re-derivável do store do Espaço; `last_focus` é estado de
/// conveniência do ponteiro (qual Espaço o app abre primeiro), não um fato do domínio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub last_focus: u64,
    #[serde(default)]
    pub archived: bool,
}

/// Intenção durável da troca de foco. O evento `WorkspaceFocusSet` com o mesmo
/// `focus_id` é o ponto de commit; o registry só é atualizado depois dele.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingWorkspaceFocus {
    pub focus_id: String,
    pub target: WorkspaceEntry,
    /// Semente de relógio fornecida pelo caller. Ao persistir o journal, o registry
    /// substitui por uma ordem estritamente maior que todo `last_focus` conhecido.
    pub focused_at_ms: u64,
}

/// Guarda interprocesso da transação de foco. A conexão mantém `BEGIN IMMEDIATE`
/// aberto desde antes de ler/escrever o journal até commit ou abort; o mesmo lock
/// serializa todo save do registry. Queda do processo fecha a conexão e libera o lock.
pub struct WorkspaceFocusTransaction {
    _connection: Connection,
    registry_path: PathBuf,
    pending: PendingWorkspaceFocus,
}

impl WorkspaceFocusTransaction {
    /// Intenção exatamente como foi persistida, incluindo a ordem monotônica.
    #[must_use]
    pub fn pending(&self) -> &PendingWorkspaceFocus {
        &self.pending
    }
}

/// Resultado de completar uma troca cujo evento já foi confirmado. Mesmo quando
/// o registry/journal ficam temporariamente indisponíveis, o commit não volta a
/// ser apresentado como falha pré-commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceFocusCommit {
    Committed,
    CommittedPending { detail: String },
}

/// Efeito observado ao reconciliar o journal durante o startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceFocusRecovery {
    NoPending,
    Aborted { focus_id: String },
    Committed(WorkspaceFocusCommit),
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingWorkspaceFocusFile {
    #[serde(default)]
    version: u32,
    pending: PendingWorkspaceFocus,
}

/// Forma persistida do registry — versionada como todo artefato durável do projeto
/// (campos futuros entram aditivos com `serde(default)`).
#[derive(Debug, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    workspaces: Vec<WorkspaceEntry>,
}

/// Registry global mínimo dos Espaços conhecidos da máquina (decisão 3 do despacho:
/// `~/.lina/workspaces.json` — ver [`default_registry_path`]). **Ponteiro re-derivável,
/// NUNCA autoridade**: perder/corromper este arquivo não perde Espaço algum.
pub struct WorkspaceRegistry {
    path: PathBuf,
    entries: Vec<WorkspaceEntry>,
}

impl WorkspaceRegistry {
    /// Carrega o registry de `path`. Arquivo ausente → registry VAZIO (ponteiro novo);
    /// arquivo ilegível → `Err` VISÍVEL (nunca engolido) — o chamador re-deriva as
    /// entradas dos stores ([`WorkspaceRegistry::rederive_entry`]) e salva por cima.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let path = path.as_ref().to_path_buf();
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    entries: Vec::new(),
                });
            }
            Err(error) => return Err(error.into()),
        };
        let file: RegistryFile = serde_json::from_str(&body)?;
        Ok(Self {
            path,
            entries: file.workspaces,
        })
    }

    /// Funde no snapshot em memória todas as entradas que outro processo já publicou
    /// no mesmo arquivo. O chamador usa isto **dentro** do lock da operação antes de
    /// tomar decisões de idempotência/nome: uma janela antiga não pode transformar um
    /// retry da mesma intenção em um novo workspace sufixado.
    pub fn merge_from_disk(&mut self) -> Result<(), WorkspaceError> {
        let on_disk = Self::load(&self.path)?;
        for entry in on_disk.entries {
            self.upsert(entry);
        }
        Ok(())
    }

    /// Persiste ATÔMICO (tmp + `sync_all` + rename no mesmo dir — padrão do
    /// `plan.md`, ADR 0001): crash de processo nunca deixa o ponteiro meio-escrito,
    /// e o fsync do tmp fecha a janela rename-antes-dos-dados em perda de energia
    /// (o ponteiro segue re-derivável de qualquer forma).
    ///
    /// Antes de escrever, FUNDE com o arquivo corrente sob o lock. A foto do disco é
    /// a base: `last_focus` usa máximo, `archived` é monotônico (não existe unarchive)
    /// e nome/path só mudam quando o store autoritativo pode ser re-derivado. Assim um
    /// writer stale não desfaz archive/rename nem apaga entrada concorrente. Registry
    /// corrente ilegível é `Err` visível e byte-idêntico; reconstrução usa a API CAS.
    pub fn save(&mut self) -> Result<(), WorkspaceError> {
        let _connection = self.acquire_focus_lock()?;
        self.save_under_registry_lock()
    }

    /// O caller já possui o lock SQLite compartilhado por saves e transações de
    /// foco. Separar este miolo evita tentar readquirir o mesmo lock durante
    /// `prepare_focus → append → commit`.
    fn save_under_registry_lock(&mut self) -> Result<(), WorkspaceError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut merged = Self::load(&self.path)?.entries;
        for memory in &self.entries {
            match merged.iter_mut().find(|entry| entry.id == memory.id) {
                Some(current) => {
                    let last_focus = current.last_focus.max(memory.last_focus);
                    let archived = current.archived || memory.archived;
                    match Self::rederive_entry(&memory.path) {
                        Ok(mut canonical) if canonical.id == memory.id => {
                            canonical.last_focus = last_focus;
                            canonical.archived |= archived;
                            *current = canonical;
                        }
                        Ok(canonical) => {
                            tracing::error!(
                                path = %memory.path.display(),
                                expected = %memory.id,
                                found = %canonical.id,
                                "registry não adotou fatos de um store com identidade divergente"
                            );
                            current.last_focus = last_focus;
                            current.archived = archived;
                        }
                        Err(error) => {
                            tracing::warn!(
                                path = %memory.path.display(),
                                %error,
                                "store indisponível ao salvar registry; fatos do disco foram preservados"
                            );
                            current.last_focus = last_focus;
                            current.archived = archived;
                        }
                    }
                }
                None => {
                    let mut candidate = memory.clone();
                    match Self::rederive_entry(&memory.path) {
                        Ok(mut canonical) if canonical.id == memory.id => {
                            canonical.last_focus = canonical.last_focus.max(memory.last_focus);
                            canonical.archived |= memory.archived;
                            candidate = canonical;
                        }
                        Ok(canonical) => tracing::error!(
                            path = %memory.path.display(),
                            expected = %memory.id,
                            found = %canonical.id,
                            "ponteiro novo diverge do store; mantido para quarentena visível"
                        ),
                        Err(error) => tracing::warn!(
                            path = %memory.path.display(),
                            %error,
                            "ponteiro novo não pôde ser re-derivado; mantido para diagnóstico visível"
                        ),
                    }
                    merged.push(candidate);
                }
            }
        }
        self.entries = merged;
        self.write_atomically()
    }

    /// Caminho do journal transacional de foco, ao lado do registry global.
    #[must_use]
    pub fn pending_focus_path(&self) -> PathBuf {
        self.path.with_file_name("workspaces.focus-pending.json")
    }

    /// SQLite dedicado à exclusão interprocesso dos saves e da transação de foco.
    /// Não contém estado de domínio; o arquivo pode sobreviver, o lock não sobrevive
    /// ao processo.
    #[must_use]
    pub fn focus_lock_path(&self) -> PathBuf {
        self.path.with_file_name("workspaces.focus-lock.sqlite3")
    }

    /// Persiste a intenção antes de o caller apendar `WorkspaceFocusSet`. Repetir
    /// a mesma intenção é idempotente; outra intenção precisa primeiro reconciliar
    /// a pendente, evitando dois commits concorrentes sem ordem durável. A guarda
    /// retornada mantém exclusão interprocesso até commit/abort.
    pub fn prepare_focus(
        &self,
        pending: &PendingWorkspaceFocus,
    ) -> Result<(WorkspaceFocusTransaction, PendingWorkspaceFocus), WorkspaceError> {
        validate_pending_focus(pending)?;
        let connection = self.acquire_focus_lock()?;
        if let Some(existing) = self.read_pending_focus()? {
            if same_focus_intent(&existing, pending) {
                return Ok((
                    WorkspaceFocusTransaction {
                        _connection: connection,
                        registry_path: self.path.clone(),
                        pending: existing.clone(),
                    },
                    existing,
                ));
            }
            return Err(WorkspaceError::InvalidFocusIntent {
                reason: format!(
                    "já existe foco pendente {} para {}",
                    existing.focus_id,
                    existing.target.path.display()
                ),
            });
        }
        let mut durable = pending.clone();
        durable.focused_at_ms = self.next_focus_order(pending.focused_at_ms)?;
        let body = serde_json::to_vec_pretty(&PendingWorkspaceFocusFile {
            version: 1,
            pending: durable.clone(),
        })?;
        write_focus_journal_atomically(&self.pending_focus_path(), &body)?;
        Ok((
            WorkspaceFocusTransaction {
                _connection: connection,
                registry_path: self.path.clone(),
                pending: durable.clone(),
            },
            durable,
        ))
    }

    fn acquire_focus_lock(&self) -> Result<Connection, WorkspaceError> {
        if let Some(parent) = self.focus_lock_path().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = self.focus_lock_path();
        let connection = Connection::open(&path).map_err(|error| focus_lock_error(&path, error))?;
        connection
            .busy_timeout(Duration::ZERO)
            .map_err(|error| focus_lock_error(&path, error))?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| focus_lock_error(&path, error))?;
        Ok(connection)
    }

    fn next_focus_order(&self, requested: u64) -> Result<u64, WorkspaceError> {
        let memory_max = self
            .entries
            .iter()
            .map(|entry| entry.last_focus)
            .max()
            .unwrap_or_default();
        let disk_max = Self::load(&self.path)?
            .entries
            .into_iter()
            .map(|entry| entry.last_focus)
            .max()
            .unwrap_or_default();
        requested
            .max(memory_max)
            .max(disk_max)
            .checked_add(1)
            .ok_or_else(|| WorkspaceError::InvalidFocusIntent {
                reason: "a ordem de foco atingiu o limite u64".to_owned(),
            })
    }

    /// Completa um foco somente depois de observar no store alvo o evento com o
    /// mesmo `focus_id`. Falha de save/remoção depois desse ponto vira
    /// `CommittedPending`, nunca um falso erro que induziria outra troca.
    pub fn commit_prepared_focus(
        &mut self,
        transaction: WorkspaceFocusTransaction,
        committed_seq: u64,
    ) -> Result<WorkspaceFocusCommit, WorkspaceError> {
        self.validate_focus_transaction(&transaction)?;
        let journal =
            self.read_pending_focus()?
                .ok_or_else(|| WorkspaceError::InvalidFocusIntent {
                    reason: format!(
                        "journal do foco {} não existe",
                        transaction.pending.focus_id
                    ),
                })?;
        if journal != transaction.pending {
            return Err(WorkspaceError::InvalidFocusIntent {
                reason: format!(
                    "journal do foco {} mudou durante a transação",
                    transaction.pending.focus_id
                ),
            });
        }
        let canonical = observed_focus_target_at(&transaction.pending, committed_seq)?;
        self.finish_prepared_focus(&transaction.pending, canonical)
    }

    /// Aborta uma intenção cujo append falhou, ainda sob a mesma exclusão que a preparou.
    pub fn abort_prepared_focus(
        &self,
        transaction: WorkspaceFocusTransaction,
    ) -> Result<(), WorkspaceError> {
        self.validate_focus_transaction(&transaction)?;
        let journal =
            self.read_pending_focus()?
                .ok_or_else(|| WorkspaceError::InvalidFocusIntent {
                    reason: format!(
                        "journal do foco {} não existe",
                        transaction.pending.focus_id
                    ),
                })?;
        if journal != transaction.pending {
            return Err(WorkspaceError::InvalidFocusIntent {
                reason: format!(
                    "journal do foco {} mudou durante o abort",
                    transaction.pending.focus_id
                ),
            });
        }
        remove_focus_journal(&self.pending_focus_path())
    }

    fn validate_focus_transaction(
        &self,
        transaction: &WorkspaceFocusTransaction,
    ) -> Result<(), WorkspaceError> {
        if transaction.registry_path == self.path {
            Ok(())
        } else {
            Err(WorkspaceError::InvalidFocusIntent {
                reason: "a transação pertence a outro registry de Espaços".to_owned(),
            })
        }
    }

    fn finish_prepared_focus(
        &mut self,
        pending: &PendingWorkspaceFocus,
        canonical: WorkspaceEntry,
    ) -> Result<WorkspaceFocusCommit, WorkspaceError> {
        let target_id = canonical.id.clone();
        self.upsert(canonical);
        if !self.set_focus(&target_id, pending.focused_at_ms) {
            return Err(WorkspaceError::InvalidFocusIntent {
                reason: format!("o alvo confirmado {target_id} não entrou no registry"),
            });
        }
        if let Err(error) = self.save_under_registry_lock() {
            return Ok(WorkspaceFocusCommit::CommittedPending {
                detail: format!(
                    "o evento de foco foi confirmado, mas o registry aguarda reconciliação: {error}"
                ),
            });
        }
        if let Err(error) = remove_focus_journal(&self.pending_focus_path()) {
            return Ok(WorkspaceFocusCommit::CommittedPending {
                detail: format!(
                    "o registry foi atualizado, mas o journal de foco aguarda limpeza: {error}"
                ),
            });
        }
        Ok(WorkspaceFocusCommit::Committed)
    }

    /// Reconcilia uma troca interrompida no startup. Sem evento correlacionado, o
    /// crash ocorreu antes do commit e a intenção é descartada. Com evento, conclui
    /// registry + limpeza de modo idempotente.
    pub fn recover_pending_focus(&mut self) -> Result<WorkspaceFocusRecovery, WorkspaceError> {
        let _connection = self.acquire_focus_lock()?;
        let Some(pending) = self.read_pending_focus()? else {
            return Ok(WorkspaceFocusRecovery::NoPending);
        };
        let Some(canonical) = observed_focus_target_by_scan(&pending)? else {
            remove_focus_journal(&self.pending_focus_path())?;
            return Ok(WorkspaceFocusRecovery::Aborted {
                focus_id: pending.focus_id,
            });
        };
        self.finish_prepared_focus(&pending, canonical)
            .map(WorkspaceFocusRecovery::Committed)
    }

    fn read_pending_focus(&self) -> Result<Option<PendingWorkspaceFocus>, WorkspaceError> {
        let path = self.pending_focus_path();
        let body = match std::fs::read(&path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let file: PendingWorkspaceFocusFile = serde_json::from_slice(&body)?;
        if file.version != 1 {
            return Err(WorkspaceError::InvalidFocusIntent {
                reason: format!("versão {} do journal não é suportada", file.version),
            });
        }
        validate_pending_focus(&file.pending)?;
        Ok(Some(file.pending))
    }

    /// Substitui um ponteiro ausente/corrompido exclusivamente por entradas que o
    /// chamador já re-derivou dos stores. Diferente de [`Self::save`], não tenta fundir
    /// o arquivo atual: ele é justamente o artefato que está sendo reconstruído.
    #[cfg(test)]
    pub fn rebuild_from_verified_entries(
        path: impl AsRef<Path>,
        entries: Vec<WorkspaceEntry>,
    ) -> Result<Self, WorkspaceError> {
        let registry = Self::validated_rebuild(path.as_ref(), entries)?;
        let _connection = registry.acquire_focus_lock()?;
        registry.write_atomically()?;
        Ok(registry)
    }

    /// Publica uma reconstrução somente se o arquivo ainda for byte-idêntico à
    /// foto que foi varrida. A comparação ocorre sob o mesmo lock de saves/foco, fechando
    /// o intervalo scan→write sem manter o lock durante a leitura de todos os stores.
    pub fn rebuild_from_verified_entries_if_unchanged(
        path: impl AsRef<Path>,
        entries: Vec<WorkspaceEntry>,
        expected_file: Option<&[u8]>,
    ) -> Result<Self, WorkspaceError> {
        let registry = Self::validated_rebuild(path.as_ref(), entries)?;
        let _connection = registry.acquire_focus_lock()?;
        let current_file = match std::fs::read(&registry.path) {
            Ok(body) => Some(body),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if current_file.as_deref() != expected_file {
            return Err(WorkspaceError::RegistryChangedDuringRebuild {
                path: registry.path.clone(),
            });
        }
        registry.write_atomically()?;
        Ok(registry)
    }

    fn validated_rebuild(
        path: &Path,
        entries: Vec<WorkspaceEntry>,
    ) -> Result<Self, WorkspaceError> {
        let mut ids = std::collections::BTreeSet::new();
        let mut paths = std::collections::BTreeSet::new();
        for entry in &entries {
            if entry.id.trim().is_empty() {
                return Err(WorkspaceError::InvalidRegistryRebuild {
                    reason: format!("{} não tem workspace_id válido", entry.path.display()),
                });
            }
            if entry.name.trim().is_empty() {
                return Err(WorkspaceError::InvalidRegistryRebuild {
                    reason: format!("{} não tem nome válido", entry.path.display()),
                });
            }
            if !ids.insert(entry.id.clone()) {
                return Err(WorkspaceError::InvalidRegistryRebuild {
                    reason: format!("workspace_id duplicado: {}", entry.id),
                });
            }
            if !paths.insert(entry.path.clone()) {
                return Err(WorkspaceError::InvalidRegistryRebuild {
                    reason: format!("caminho duplicado: {}", entry.path.display()),
                });
            }
        }

        Ok(Self {
            path: path.to_path_buf(),
            entries,
        })
    }

    fn write_atomically(&self) -> Result<(), WorkspaceError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&RegistryFile {
            version: 1,
            workspaces: self.entries.clone(),
        })?;
        let tmp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        // O rename torna o conteúdo atômico; sincronizar o diretório torna o novo
        // nome durável após perda de energia (em plataformas que expõem fsync de dir).
        #[cfg(unix)]
        if let Some(parent) = self.path.parent() {
            if let Err(error) =
                std::fs::File::open(parent).and_then(|directory| directory.sync_all())
            {
                eprintln!(
                    "lina-core: registry publicado em {}, mas o fsync do diretório falhou ({error})",
                    self.path.display()
                );
            }
        }
        Ok(())
    }

    /// As entradas conhecidas, na ordem de inserção.
    #[must_use]
    pub fn entries(&self) -> &[WorkspaceEntry] {
        &self.entries
    }

    /// Insere/atualiza por `id`. Os FATOS (nome/path/arquivado) vêm da entrada nova
    /// (re-derivada do store); o `last_focus` preserva o maior já visto — re-derivar
    /// uma entrada nunca apaga o histórico de foco do ponteiro.
    pub fn upsert(&mut self, entry: WorkspaceEntry) {
        match self.entries.iter_mut().find(|e| e.id == entry.id) {
            Some(existing) => {
                let last_focus = existing.last_focus.max(entry.last_focus);
                *existing = entry;
                existing.last_focus = last_focus;
            }
            None => self.entries.push(entry),
        }
    }

    /// Marca o Espaço `id` como focado em `ts` (millis — o relógio é do chamador,
    /// como em todo o core). `false` se o id não é conhecido.
    pub fn set_focus(&mut self, id: &str, ts: u64) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.last_focus = ts;
                true
            }
            None => false,
        }
    }

    /// O Espaço ATIVO: o não-arquivado focado por último (`last_focus` máximo, > 0).
    /// `None` = nenhum foco registrado ainda (1º boot) — o app decide o onboarding.
    #[must_use]
    pub fn focused(&self) -> Option<&WorkspaceEntry> {
        self.entries
            .iter()
            .filter(|e| !e.archived && e.last_focus > 0)
            .max_by_key(|e| e.last_focus)
    }

    /// Nº de Espaços ATIVOS (não-arquivados) — a base do gating free/PRO.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.archived).count()
    }

    /// Gating free/PRO sobre a contagem ativa deste registry.
    pub fn can_create(&self, tier: LicenseTier) -> Result<(), WorkspaceError> {
        can_create_workspace(tier, self.active_count())
    }

    /// RE-DERIVA a entrada de um Espaço COMPLETO a partir do store dele (a autoridade) —
    /// o caminho de recuperação quando o ponteiro se perde/corrompe. Exige SQLite
    /// existente ou um espelho certificado antes de abrir (a validação nunca cria um
    /// store num ponteiro morto), `WorkspaceCreated` e `WorkspaceIdAssigned`.
    pub fn rederive_entry(root: impl AsRef<Path>) -> Result<WorkspaceEntry, WorkspaceError> {
        let root = root.as_ref().to_path_buf();
        let events_dir = Workspace::events_dir(&root);
        let database_exists = events_dir.join("lina.db").try_exists()?;
        let recoverable_mirror_exists = events_dir.join("log.jsonl").try_exists()?
            && events_dir.join("log.recovery.json").try_exists()?;
        if !database_exists && !recoverable_mirror_exists {
            return Err(WorkspaceError::NotAWorkspace { root });
        }
        let mut host = lina_host::HeadlessUiHost::new();
        let (_store, state) = EventStore::open_projected_or_recover(&events_dir, &mut host)?;
        let name = state
            .workspace_name
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| WorkspaceError::NotAWorkspace { root: root.clone() })?;
        let id = state
            .workspace_id
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| WorkspaceError::IncompleteWorkspace { root: root.clone() })?;
        Ok(WorkspaceEntry {
            id,
            name,
            path: root,
            last_focus: 0,
            archived: state.archived,
        })
    }
}

fn remove_creation_staging(staging: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(staging) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn cleanup_failed_creation(
    root: &Path,
    staging: &Path,
    operation: WorkspaceError,
    cleanup: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> WorkspaceError {
    match cleanup(staging) {
        Ok(()) => operation,
        Err(cleanup_error) => WorkspaceError::CreationCleanupFailed {
            root: root.to_path_buf(),
            staging: staging.to_path_buf(),
            operation: operation.to_string(),
            cleanup: cleanup_error.to_string(),
        },
    }
}

fn focus_lock_error(path: &Path, error: rusqlite::Error) -> WorkspaceError {
    match &error {
        rusqlite::Error::SqliteFailure(sqlite, _)
            if matches!(
                sqlite.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            WorkspaceError::FocusTransactionBusy {
                lock: path.to_path_buf(),
            }
        }
        _ => WorkspaceError::FocusTransactionLock {
            lock: path.to_path_buf(),
            detail: error.to_string(),
        },
    }
}

fn validate_pending_focus(pending: &PendingWorkspaceFocus) -> Result<(), WorkspaceError> {
    let invalid_reason = if pending.focus_id.trim().is_empty() {
        Some("focus_id vazio")
    } else if pending.target.id.trim().is_empty() {
        Some("workspace_id alvo vazio")
    } else if pending.target.name.trim().is_empty() {
        Some("nome alvo vazio")
    } else if pending.target.path.as_os_str().is_empty() {
        Some("caminho alvo vazio")
    } else if pending.target.archived {
        Some("o Espaço alvo está arquivado")
    } else {
        None
    };
    match invalid_reason {
        Some(reason) => Err(WorkspaceError::InvalidFocusIntent {
            reason: reason.to_owned(),
        }),
        None => Ok(()),
    }
}

fn same_focus_intent(left: &PendingWorkspaceFocus, right: &PendingWorkspaceFocus) -> bool {
    left.focus_id == right.focus_id && left.target == right.target
}

fn write_focus_journal_atomically(path: &Path, body: &[u8]) -> Result<(), WorkspaceError> {
    let parent = path
        .parent()
        .ok_or_else(|| WorkspaceError::InvalidFocusIntent {
            reason: format!("{} não tem diretório pai", path.display()),
        })?;
    std::fs::create_dir_all(parent)?;
    let staging = path.with_extension(format!("tmp-{}-{}", std::process::id(), Uuid::now_v7()));
    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&staging)?;
        file.write_all(body)?;
        file.sync_all()?;
        std::fs::rename(&staging, path)?;
        sync_directory(parent)
    })();
    if let Err(operation) = write_result {
        return match std::fs::remove_file(&staging) {
            Ok(()) => Err(operation.into()),
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => Err(operation.into()),
            Err(cleanup) => Err(WorkspaceError::InvalidFocusIntent {
                reason: format!(
                    "não consegui persistir o journal ({operation}) nem limpar {} ({cleanup})",
                    staging.display()
                ),
            }),
        };
    }
    Ok(())
}

fn remove_focus_journal(path: &Path) -> Result<(), WorkspaceError> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            // Se o fsync falhar, um crash pode ressuscitar o journal; isso é seguro:
            // o recovery reencontra o mesmo focus_id e repete apenas o upsert idempotente.
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                if let Err(error) = sync_directory(parent) {
                    tracing::error!(
                        path = %path.display(),
                        %error,
                        "journal de foco removido; fsync do diretório falhou e o recovery poderá repetir a reconciliação"
                    );
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn observed_focus_target_at(
    pending: &PendingWorkspaceFocus,
    committed_seq: u64,
) -> Result<WorkspaceEntry, WorkspaceError> {
    validate_pending_focus(pending)?;
    let events_dir = Workspace::events_dir(&pending.target.path);
    let store = EventStore::open(&events_dir)?;
    let record =
        store
            .event_at(committed_seq)?
            .ok_or_else(|| WorkspaceError::FocusNotCommitted {
                focus_id: pending.focus_id.clone(),
            })?;
    if record.kind != "WorkspaceFocusSet" {
        return Err(WorkspaceError::FocusNotCommitted {
            focus_id: pending.focus_id.clone(),
        });
    }
    let event: DomainEvent = serde_json::from_value(record.payload).map_err(|error| {
        WorkspaceError::InvalidFocusIntent {
            reason: format!("WorkspaceFocusSet inválido no alvo: {error}"),
        }
    })?;
    match event {
        DomainEvent::WorkspaceFocusSet {
            workspace,
            focus_id: Some(focus_id),
        } if focus_id == pending.focus_id && workspace == pending.target.name => {
            Ok(pending.target.clone())
        }
        _ => Err(WorkspaceError::FocusNotCommitted {
            focus_id: pending.focus_id.clone(),
        }),
    }
}

fn observed_focus_target_by_scan(
    pending: &PendingWorkspaceFocus,
) -> Result<Option<WorkspaceEntry>, WorkspaceError> {
    validate_pending_focus(pending)?;
    let events_dir = Workspace::events_dir(&pending.target.path);
    let database_exists = events_dir.join("lina.db").try_exists()?;
    let recoverable_mirror_exists = events_dir.join("log.jsonl").try_exists()?
        && events_dir.join("log.recovery.json").try_exists()?;
    if !database_exists && !recoverable_mirror_exists {
        return Err(WorkspaceError::NotAWorkspace {
            root: pending.target.path.clone(),
        });
    }
    let mut host = lina_host::HeadlessUiHost::new();
    let (store, state) = EventStore::open_projected_or_recover(&events_dir, &mut host)?;
    let name = state
        .workspace_name
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| WorkspaceError::NotAWorkspace {
            root: pending.target.path.clone(),
        })?;
    let id = state
        .workspace_id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| WorkspaceError::IncompleteWorkspace {
            root: pending.target.path.clone(),
        })?;
    if id != pending.target.id {
        return Err(WorkspaceError::InvalidFocusIntent {
            reason: format!(
                "a identidade de {} mudou (esperado {}, encontrado {id})",
                pending.target.path.display(),
                pending.target.id
            ),
        });
    }
    if state.archived {
        return Err(WorkspaceError::InvalidFocusIntent {
            reason: format!("{} foi arquivado", pending.target.path.display()),
        });
    }

    let mut observed = false;
    for record in store.events()? {
        if record.kind != "WorkspaceFocusSet" {
            continue;
        }
        let event: DomainEvent = serde_json::from_value(record.payload).map_err(|error| {
            WorkspaceError::InvalidFocusIntent {
                reason: format!("WorkspaceFocusSet inválido no alvo: {error}"),
            }
        })?;
        if let DomainEvent::WorkspaceFocusSet {
            workspace,
            focus_id: Some(focus_id),
        } = event
        {
            if focus_id == pending.focus_id {
                if workspace != pending.target.name {
                    return Err(WorkspaceError::InvalidFocusIntent {
                        reason: format!(
                            "o foco {} foi gravado para '{workspace}', não para '{}'",
                            pending.focus_id, pending.target.name
                        ),
                    });
                }
                observed = true;
            }
        }
    }
    Ok(observed.then_some(WorkspaceEntry {
        id,
        name,
        path: pending.target.path.clone(),
        last_focus: 0,
        archived: false,
    }))
}

/// Caminho padrão do registry global: `~/.lina/workspaces.json` (decisão 3 do
/// despacho), no idioma do repo `HOME → USERPROFILE` (`bridge.rs:user_home_dir`,
/// `main.rs`) — no Windows típico HOME não existe e USERPROFILE é o padrão. `None`
/// só quando NENHUMA das duas existe — aí o app cai na sua base persistente
/// por-usuário e passa o caminho explícito a [`WorkspaceRegistry::load`].
#[must_use]
pub fn default_registry_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".lina").join("workspaces.json"))
}

#[cfg(test)]
mod reliability_tests {
    use super::*;

    #[test]
    fn workspace_reliability_partial_store_is_not_a_catalog_workspace() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-partial-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("interrompido");
        let mut store = EventStore::open(Workspace::events_dir(&root)).expect("store");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Interrompido".into(),
                focus_preset: "dev_app".into(),
            })
            .expect("primeiro evento confirmado");
        drop(store);

        assert!(matches!(
            WorkspaceRegistry::rederive_entry(&root),
            Err(WorkspaceError::IncompleteWorkspace { .. })
        ));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_dead_pointer_validation_does_not_create_store() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-dead-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("ponteiro-morto");

        assert!(WorkspaceRegistry::rederive_entry(&root).is_err());
        assert!(
            !Workspace::events_dir(&root).exists(),
            "validar um alvo ausente não materializa .lina/events"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_core_creation_publishes_only_complete_store() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-core-atomic-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("workspace");
        let crashed_staging = root.join(".lina").join(".events-create-crashed");
        EventStore::open(&crashed_staging)
            .expect("staging")
            .append(&DomainEvent::WorkspaceCreated {
                name: "Parcial escondido".into(),
                focus_preset: "blank".into(),
            })
            .expect("corte após primeiro evento");

        assert!(matches!(
            WorkspaceRegistry::rederive_entry(&root),
            Err(WorkspaceError::NotAWorkspace { .. })
        ));
        assert!(
            !Workspace::events_dir(&root).exists(),
            "staging interrompido nunca ocupa o caminho publicado"
        );

        let workspace =
            Workspace::create(&root, "Completo", "blank", None).expect("retry independente");
        let state = workspace.project().expect("projeção final");
        assert_eq!(state.workspace_name.as_deref(), Some("Completo"));
        assert!(state
            .workspace_id
            .as_deref()
            .is_some_and(|id| !id.is_empty()));
        assert!(WorkspaceRegistry::rederive_entry(&root).is_ok());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_post_rename_reopen_failure_is_typed_and_adoptable() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-post-rename-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("workspace");

        let error = match Workspace::create_seeded_with_seams(
            root.clone(),
            "Completo",
            "blank",
            None,
            &[],
            |store, events| {
                store.append_batch(events)?;
                Ok(())
            },
            remove_creation_staging,
            |_| Err(std::io::Error::from_raw_os_error(24).into()),
        ) {
            Ok(_) => panic!("falha injetada acontece depois da publicação"),
            Err(error) => error,
        };

        assert_eq!(error.published_root(), Some(root.as_path()));
        assert!(matches!(
            error,
            WorkspaceError::PublishedButUnavailable { .. }
        ));
        let adopted = Workspace::open(&root).expect("retry adota a raiz já publicada");
        let state = adopted.project().expect("projeção completa");
        assert_eq!(state.workspace_name.as_deref(), Some("Completo"));
        assert!(state.workspace_id.is_some());
        assert!(matches!(
            Workspace::create(&root, "Outro", "blank", None),
            Err(WorkspaceError::AlreadyExists { .. })
        ));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_cleanup_failure_preserves_both_causes_and_no_catalog_root() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-cleanup-error-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("workspace");

        let error = match Workspace::create_seeded_with_seams(
            root.clone(),
            "Completo",
            "blank",
            None,
            &[],
            |_store, _events| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "falha de persistência injetada",
                )
                .into())
            },
            |_staging| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "falha de limpeza injetada",
                ))
            },
            |_| panic!("falha pré-publicação não reabre raiz"),
        ) {
            Ok(_) => panic!("persistência injetada deveria falhar"),
            Err(error) => error,
        };

        match error {
            WorkspaceError::CreationCleanupFailed {
                operation,
                cleanup,
                staging,
                ..
            } => {
                assert!(operation.contains("falha de persistência injetada"));
                assert!(cleanup.contains("falha de limpeza injetada"));
                assert!(
                    staging.exists(),
                    "o seam simulou o lixo que não pôde ser removido"
                );
            }
            other => panic!("erro deveria conservar ambas as causas: {other}"),
        }
        assert!(
            !Workspace::events_dir(&root).exists(),
            "staging oculto nunca ocupa a raiz catalogável"
        );
        assert!(matches!(
            WorkspaceRegistry::rederive_entry(&root),
            Err(WorkspaceError::NotAWorkspace { .. })
        ));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_registry_rebuild_replaces_corrupt_pointer_atomically() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-registry-rebuild-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let first_root = base.join("first");
        let second_root = base.join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        let mut second = Workspace::create(&second_root, "Segundo", "blank", None).expect("second");
        second.archive().expect("archive second");
        drop(second);
        let registry_path = base.join("workspaces.json");
        std::fs::write(&registry_path, b"{ registry corrompido").expect("corrupt fixture");

        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("derive first");
        let second = WorkspaceRegistry::rederive_entry(&second_root).expect("derive second");
        WorkspaceRegistry::rebuild_from_verified_entries(
            &registry_path,
            vec![first.clone(), second.clone()],
        )
        .expect("rebuild");

        let loaded = WorkspaceRegistry::load(&registry_path).expect("registry legível");
        assert_eq!(loaded.entries(), &[first, second]);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_registry_save_never_overwrites_corruption_seen_after_load() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-registry-save-corrupt-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("workspace");
        drop(Workspace::create(&root, "Íntegro", "blank", None).expect("workspace"));
        let registry_path = base.join("workspaces.json");
        let mut seed = WorkspaceRegistry::load(&registry_path).expect("registry novo");
        let mut entry = WorkspaceRegistry::rederive_entry(&root).expect("entrada");
        entry.last_focus = 1;
        let id = entry.id.clone();
        seed.upsert(entry);
        seed.save().expect("seed válido");

        let mut stale = WorkspaceRegistry::load(&registry_path).expect("snapshot em memória");
        let corrupt = b"{ registry corrompido depois do load";
        std::fs::write(&registry_path, corrupt).expect("corrupção externa");
        assert!(stale.set_focus(&id, 2));
        assert!(
            stale.save().is_err(),
            "save não recebe autorização para substituir bytes inconclusivos"
        );
        assert_eq!(
            std::fs::read(&registry_path).expect("registry preservado"),
            corrupt,
            "a corrupção observada sob o lock permanece byte-idêntica para o rebuild CAS"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_stale_registry_writer_cannot_resurrect_archived_facts() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-registry-stale-archive-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("workspace");
        drop(Workspace::create(&root, "Antes", "blank", None).expect("workspace"));
        let registry_path = base.join("workspaces.json");
        let mut seed = WorkspaceRegistry::load(&registry_path).expect("registry novo");
        let mut entry = WorkspaceRegistry::rederive_entry(&root).expect("entrada");
        entry.last_focus = 1;
        let id = entry.id.clone();
        seed.upsert(entry);
        seed.save().expect("seed válido");

        let mut stale_focus = WorkspaceRegistry::load(&registry_path).expect("writer A");
        let mut archiver = WorkspaceRegistry::load(&registry_path).expect("writer B");
        let mut store = EventStore::open(Workspace::events_dir(&root)).expect("store");
        store
            .append_batch(&[
                DomainEvent::WorkspaceRenamed {
                    name: "Depois".to_owned(),
                },
                DomainEvent::WorkspaceArchived,
            ])
            .expect("fatos novos");
        drop(store);
        archiver.upsert(WorkspaceRegistry::rederive_entry(&root).expect("entrada arquivada"));
        archiver.save().expect("writer B publica archive");

        assert!(stale_focus.set_focus(&id, 9));
        stale_focus.save().expect("writer A salva foco stale");
        let final_entry = WorkspaceRegistry::load(&registry_path)
            .expect("registry final")
            .entries()
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .expect("entrada final");
        assert!(final_entry.archived, "archive é monotônico");
        assert_eq!(final_entry.name, "Depois", "nome vem do store autoritativo");
        assert_eq!(final_entry.path, root);
        assert_eq!(
            final_entry.last_focus, 9,
            "o foco concorrente também sobrevive"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_registry_rebuild_rejects_stale_snapshot_under_lock() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-registry-cas-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let scanned_root = base.join("scanned");
        let concurrent_root = base.join("concurrent");
        drop(Workspace::create(&scanned_root, "Varrido", "blank", None).expect("scanned"));
        drop(
            Workspace::create(&concurrent_root, "Concorrente", "blank", None).expect("concurrent"),
        );
        let registry_path = base.join("workspaces.json");
        let stale_bytes = b"{ registry corrompido".to_vec();
        std::fs::write(&registry_path, &stale_bytes).expect("snapshot inicial");
        let scanned = WorkspaceRegistry::rederive_entry(&scanned_root).expect("scanned entry");

        // Simula uma criação que termina depois do scan, mas antes de o rebuild
        // adquirir o lock. A publicação concorrente é a versão que deve sobreviver.
        let concurrent =
            WorkspaceRegistry::rederive_entry(&concurrent_root).expect("concurrent entry");
        WorkspaceRegistry::rebuild_from_verified_entries_if_unchanged(
            &registry_path,
            vec![concurrent.clone()],
            Some(&stale_bytes),
        )
        .expect("rebuild concorrente publica sobre o snapshot que verificou");
        let concurrent_bytes = std::fs::read(&registry_path).expect("foto concorrente");

        assert!(matches!(
            WorkspaceRegistry::rebuild_from_verified_entries_if_unchanged(
                &registry_path,
                vec![scanned],
                Some(&stale_bytes),
            ),
            Err(WorkspaceError::RegistryChangedDuringRebuild { .. })
        ));
        assert_eq!(
            std::fs::read(&registry_path).expect("registry final"),
            concurrent_bytes,
            "CAS rejeita o scan velho sem apagar a publicação concorrente"
        );
        assert_eq!(
            WorkspaceRegistry::load(&registry_path)
                .expect("registry final legível")
                .entries(),
            &[concurrent]
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_registry_rebuild_rejects_duplicate_ids_before_write() {
        let base = std::env::temp_dir().join(format!(
            "lina-workspace-registry-duplicate-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let first_root = base.join("first");
        let second_root = base.join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("derive first");
        let mut second = WorkspaceRegistry::rederive_entry(&second_root).expect("derive second");
        second.id.clone_from(&first.id);
        let registry_path = base.join("workspaces.json");

        let error = match WorkspaceRegistry::rebuild_from_verified_entries(
            &registry_path,
            vec![first, second],
        ) {
            Ok(_) => panic!("id duplicado nunca é publicado"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            WorkspaceError::InvalidRegistryRebuild { .. }
        ));
        assert!(!registry_path.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    fn assert_focus_order_survives_restart(tag: &str, previous: u64, requested: u64) {
        let base = std::env::temp_dir().join(format!(
            "lina-focus-order-{tag}-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let previous_root = base.join("previous");
        let target_root = base.join("target");
        drop(Workspace::create(&previous_root, "Anterior", "blank", None).expect("previous"));
        drop(Workspace::create(&target_root, "Alvo", "blank", None).expect("target"));
        let registry_path = base.join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let mut previous_entry =
            WorkspaceRegistry::rederive_entry(&previous_root).expect("previous entry");
        previous_entry.last_focus = previous;
        let target = WorkspaceRegistry::rederive_entry(&target_root).expect("target entry");
        registry.upsert(previous_entry);
        registry.upsert(target.clone());
        registry.save().expect("registry inicial");

        let pending = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target,
            focused_at_ms: requested,
        };
        let (transaction, durable) = registry.prepare_focus(&pending).expect("prepare ordenado");
        assert!(durable.focused_at_ms > previous);
        assert!(durable.focused_at_ms > requested);
        let committed_seq = EventStore::open(Workspace::events_dir(&target_root))
            .expect("target store")
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: pending.target.name.clone(),
                focus_id: Some(pending.focus_id.clone()),
            })
            .expect("commit point");
        registry
            .commit_prepared_focus(transaction, committed_seq)
            .expect("commit do journal normalizado");

        let restarted = WorkspaceRegistry::load(&registry_path).expect("restart");
        let focused = restarted.focused().expect("foco após restart");
        assert_eq!(focused.id, pending.target.id);
        assert_eq!(focused.last_focus, durable.focused_at_ms);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_focus_tie_in_same_millisecond_is_strictly_ordered() {
        assert_focus_order_survives_restart("tie", 500, 500);
    }

    #[test]
    fn workspace_reliability_regressing_clock_cannot_restore_the_previous_focus() {
        assert_focus_order_survives_restart("clock-regressed", 1_000, 900);
    }

    #[test]
    fn workspace_reliability_future_timestamp_cannot_pin_an_old_workspace() {
        assert_focus_order_survives_restart("future-entry", 9_000_000_000_000, 1_700_000_000_000);
    }

    #[test]
    fn workspace_reliability_focus_order_overflow_is_visible() {
        let base = std::env::temp_dir().join(format!(
            "lina-focus-order-overflow-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("target");
        drop(Workspace::create(&root, "Alvo", "blank", None).expect("target"));
        let mut registry = WorkspaceRegistry::load(base.join("workspaces.json")).expect("registry");
        let target = WorkspaceRegistry::rederive_entry(&root).expect("entry");
        registry.upsert(target.clone());
        let pending = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target,
            focused_at_ms: u64::MAX,
        };
        assert!(matches!(
            registry.prepare_focus(&pending),
            Err(WorkspaceError::InvalidFocusIntent { .. })
        ));
        assert!(!registry.pending_focus_path().exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_focus_prepare_refuses_newly_corrupt_registry() {
        let base = std::env::temp_dir().join(format!(
            "lina-focus-order-corrupt-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let root = base.join("target");
        drop(Workspace::create(&root, "Alvo", "blank", None).expect("target"));
        let registry_path = base.join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let target = WorkspaceRegistry::rederive_entry(&root).expect("entry");
        registry.upsert(target.clone());
        registry.save().expect("ponteiro inicial");
        let event_count_before = EventStore::open(Workspace::events_dir(&root))
            .expect("target store")
            .event_count()
            .expect("head inicial");
        std::fs::write(&registry_path, b"{ registry quebrado")
            .expect("corromper entre catálogo e prepare");
        let pending = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target,
            focused_at_ms: 100,
        };

        assert!(matches!(
            registry.prepare_focus(&pending),
            Err(WorkspaceError::Json(_))
        ));
        assert!(!registry.pending_focus_path().exists());
        let target_store = EventStore::open(Workspace::events_dir(&root)).expect("reopen target");
        assert_eq!(
            target_store.event_count().expect("head final"),
            event_count_before
        );
        assert!(target_store
            .events()
            .expect("eventos")
            .iter()
            .all(|record| record.kind != "WorkspaceFocusSet"));
        let _ = std::fs::remove_dir_all(base);
    }

    fn focus_transaction_fixture(tag: &str) -> (PathBuf, PathBuf, WorkspaceEntry, WorkspaceEntry) {
        let base = std::env::temp_dir().join(format!(
            "lina-focus-transaction-{tag}-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let first_root = base.join("first");
        let second_root = base.join("second");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&second_root, "Segundo", "blank", None).expect("second"));
        let registry_path = base.join("workspaces.json");
        let first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        let second = WorkspaceRegistry::rederive_entry(&second_root).expect("second entry");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        registry.upsert(first.clone());
        registry.upsert(second.clone());
        registry.save().expect("registry inicial");
        (base, registry_path, first, second)
    }

    #[test]
    fn workspace_reliability_focus_transaction_blocks_prepare_and_recovery() {
        let (base, registry_path, first, second) = focus_transaction_fixture("exclusive");
        let registry_a = WorkspaceRegistry::load(&registry_path).expect("registry A");
        let mut registry_b = WorkspaceRegistry::load(&registry_path).expect("registry B");
        let requested_a = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target: first,
            focused_at_ms: 100,
        };
        let (transaction_a, durable_a) = registry_a
            .prepare_focus(&requested_a)
            .expect("A prepara e segura o lock");
        let requested_b = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target: second,
            focused_at_ms: 100,
        };

        assert!(matches!(
            registry_b.prepare_focus(&requested_b),
            Err(WorkspaceError::FocusTransactionBusy { .. })
        ));
        assert!(matches!(
            registry_b.recover_pending_focus(),
            Err(WorkspaceError::FocusTransactionBusy { .. })
        ));
        assert!(matches!(
            registry_b.save(),
            Err(WorkspaceError::FocusTransactionBusy { .. })
        ));
        assert_eq!(
            registry_a.read_pending_focus().expect("journal de A"),
            Some(durable_a),
            "B ocupado não lê, remove nem substitui o journal de A"
        );
        registry_a
            .abort_prepared_focus(transaction_a)
            .expect("A aborta sob o próprio lock");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_focus_transactions_order_two_targets_without_overwrite() {
        let (base, registry_path, first, second) = focus_transaction_fixture("ordered");
        let mut registry_a = WorkspaceRegistry::load(&registry_path).expect("registry A");
        // B nasce antes do commit de A: seu snapshot em memória está deliberadamente stale.
        let mut registry_b = WorkspaceRegistry::load(&registry_path).expect("registry B stale");
        let requested_a = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target: first.clone(),
            focused_at_ms: 500,
        };
        let (transaction_a, durable_a) = registry_a.prepare_focus(&requested_a).expect("prepare A");
        let seq_a = EventStore::open(Workspace::events_dir(&first.path))
            .expect("store A")
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: durable_a.target.name.clone(),
                focus_id: Some(durable_a.focus_id.clone()),
            })
            .expect("append A");
        assert_eq!(
            registry_a
                .commit_prepared_focus(transaction_a, seq_a)
                .expect("commit A"),
            WorkspaceFocusCommit::Committed
        );

        let requested_b = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target: second.clone(),
            focused_at_ms: 500,
        };
        let (transaction_b, durable_b) = registry_b
            .prepare_focus(&requested_b)
            .expect("B prepara depois de A liberar");
        assert!(
            durable_b.focused_at_ms > durable_a.focused_at_ms,
            "a leitura fresca sob lock vence o snapshot stale de B"
        );
        assert_eq!(
            registry_b.read_pending_focus().expect("journal B"),
            Some(durable_b.clone()),
            "o segundo target substitui somente depois do commit completo de A"
        );
        let seq_b = EventStore::open(Workspace::events_dir(&second.path))
            .expect("store B")
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: durable_b.target.name.clone(),
                focus_id: Some(durable_b.focus_id.clone()),
            })
            .expect("append B");
        assert_eq!(
            registry_b
                .commit_prepared_focus(transaction_b, seq_b)
                .expect("commit B"),
            WorkspaceFocusCommit::Committed
        );

        let restarted = WorkspaceRegistry::load(&registry_path).expect("restart");
        let persisted_a = restarted
            .entries()
            .iter()
            .find(|entry| entry.id == first.id)
            .expect("A persistido");
        let persisted_b = restarted
            .entries()
            .iter()
            .find(|entry| entry.id == second.id)
            .expect("B persistido");
        assert_eq!(persisted_a.last_focus, durable_a.focused_at_ms);
        assert_eq!(persisted_b.last_focus, durable_b.focused_at_ms);
        assert_ne!(persisted_a.last_focus, persisted_b.last_focus);
        assert_eq!(restarted.focused().map(|entry| &entry.id), Some(&second.id));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_focus_lock_child_process_holds_transaction() {
        let Some(registry_path) = std::env::var_os("LINA_FOCUS_LOCK_CHILD_REGISTRY") else {
            return;
        };
        let target_root = PathBuf::from(
            std::env::var_os("LINA_FOCUS_LOCK_CHILD_TARGET").expect("target do child"),
        );
        let registry =
            WorkspaceRegistry::load(PathBuf::from(registry_path)).expect("registry child");
        let target = WorkspaceRegistry::rederive_entry(target_root).expect("target child");
        let requested = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target,
            focused_at_ms: 700,
        };
        let (_transaction, _durable) = registry.prepare_focus(&requested).expect("lock child");
        println!("LINA_FOCUS_LOCK_READY");
        use std::io::Write as _;
        std::io::stdout().flush().expect("flush ready");
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn workspace_reliability_focus_lock_is_released_when_child_dies() {
        struct KillOnDrop(std::process::Child);
        impl Drop for KillOnDrop {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let (base, registry_path, first, _second) = focus_transaction_fixture("child-crash");
        let mut child = KillOnDrop(
            std::process::Command::new(std::env::current_exe().expect("test executable"))
                .arg("--exact")
                .arg(
                    "workspace::reliability_tests::workspace_reliability_focus_lock_child_process_holds_transaction",
                )
                .arg("--nocapture")
                .env("LINA_FOCUS_LOCK_CHILD_REGISTRY", &registry_path)
                .env("LINA_FOCUS_LOCK_CHILD_TARGET", &first.path)
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("spawn child"),
        );
        let stdout = child.0.stdout.take().expect("stdout child");
        let mut reader = std::io::BufReader::new(stdout);
        use std::io::BufRead as _;
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("ler barreira child");
            assert!(read > 0, "child encerrou antes de adquirir o lock");
            if line.contains("LINA_FOCUS_LOCK_READY") {
                break;
            }
        }

        let mut parent_registry = WorkspaceRegistry::load(&registry_path).expect("registry parent");
        assert!(matches!(
            parent_registry.recover_pending_focus(),
            Err(WorkspaceError::FocusTransactionBusy { .. })
        ));
        child.0.kill().expect("kill child");
        child.0.wait().expect("wait child");

        assert!(matches!(
            parent_registry
                .recover_pending_focus()
                .expect("SO liberou lock após morte"),
            WorkspaceFocusRecovery::Aborted { .. }
        ));
        assert!(!parent_registry.pending_focus_path().exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_focus_recovery_aborts_precommit_without_changing_registry() {
        let base = std::env::temp_dir().join(format!(
            "lina-focus-precommit-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let first_root = base.join("first");
        let target_root = base.join("target");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&target_root, "Alvo", "blank", None).expect("target"));
        let registry_path = base.join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let mut first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        first.last_focus = 10;
        let first_id = first.id.clone();
        let target = WorkspaceRegistry::rederive_entry(&target_root).expect("target entry");
        registry.upsert(first);
        registry.upsert(target.clone());
        registry.save().expect("last-good registry");
        let pending = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target,
            focused_at_ms: 20,
        };
        let (transaction, _durable) = registry.prepare_focus(&pending).expect("prepare durable");
        assert!(registry.pending_focus_path().is_file());
        drop(transaction); // simula a queda do processo antes do append

        let mut restarted = WorkspaceRegistry::load(&registry_path).expect("restart registry");
        assert_eq!(
            restarted.recover_pending_focus().expect("abort precommit"),
            WorkspaceFocusRecovery::Aborted {
                focus_id: pending.focus_id
            }
        );
        assert_eq!(
            restarted.focused().map(|entry| entry.id.as_str()),
            Some(first_id.as_str())
        );
        assert!(!restarted.pending_focus_path().exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_reliability_focus_commit_pending_recovers_exactly_once() {
        let base = std::env::temp_dir().join(format!(
            "lina-focus-postcommit-{}-{}",
            std::process::id(),
            Uuid::now_v7()
        ));
        let first_root = base.join("first");
        let target_root = base.join("target");
        drop(Workspace::create(&first_root, "Primeiro", "blank", None).expect("first"));
        drop(Workspace::create(&target_root, "Alvo", "blank", None).expect("target"));
        let registry_path = base.join("workspaces.json");
        let mut registry = WorkspaceRegistry::load(&registry_path).expect("registry");
        let mut first = WorkspaceRegistry::rederive_entry(&first_root).expect("first entry");
        first.last_focus = 10;
        let target = WorkspaceRegistry::rederive_entry(&target_root).expect("target entry");
        registry.upsert(first);
        registry.upsert(target.clone());
        registry.save().expect("last-good registry");
        let pending = PendingWorkspaceFocus {
            focus_id: Uuid::now_v7().to_string(),
            target,
            focused_at_ms: 20,
        };
        let (transaction, _durable) = registry.prepare_focus(&pending).expect("prepare durable");
        let committed_seq = EventStore::open(Workspace::events_dir(&target_root))
            .expect("target store")
            .append(&DomainEvent::WorkspaceFocusSet {
                workspace: pending.target.name.clone(),
                focus_id: Some(pending.focus_id.clone()),
            })
            .expect("commit point");

        let blocked_tmp = registry_path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::create_dir(&blocked_tmp).expect("bloquear save do registry");
        let outcome = registry
            .commit_prepared_focus(transaction, committed_seq)
            .expect("append confirmado nunca vira erro falso por falha de save");
        assert!(matches!(
            outcome,
            WorkspaceFocusCommit::CommittedPending { .. }
        ));
        assert_eq!(
            registry.focused().map(|entry| entry.id.as_str()),
            Some(pending.target.id.as_str()),
            "estado em memória acompanha o commit mesmo antes da reconciliação durável"
        );
        assert!(registry.pending_focus_path().is_file());
        std::fs::remove_dir(&blocked_tmp).expect("desbloquear save");

        let mut restarted = WorkspaceRegistry::load(&registry_path).expect("restart registry");
        assert_eq!(
            restarted.recover_pending_focus().expect("reconcile"),
            WorkspaceFocusRecovery::Committed(WorkspaceFocusCommit::Committed)
        );
        assert_eq!(
            restarted.focused().map(|entry| entry.id.as_str()),
            Some(pending.target.id.as_str())
        );
        assert_eq!(
            restarted.recover_pending_focus().expect("segunda recovery"),
            WorkspaceFocusRecovery::NoPending,
            "recovery repetido não reaplica nem duplica"
        );
        let focus_events = EventStore::open(Workspace::events_dir(&target_root))
            .expect("reopen target")
            .events()
            .expect("events")
            .into_iter()
            .filter(|record| record.kind == "WorkspaceFocusSet")
            .count();
        assert_eq!(focus_events, 1, "recovery não apenda segundo commit");
        let _ = std::fs::remove_dir_all(base);
    }
}

#[cfg(test)]
mod worktree_tests {
    use super::worktree_branch_name;

    #[test]
    fn branch_name_slugs_and_prefixes() {
        assert_eq!(worktree_branch_name("Terminal A"), "lina/terminal-a");
        assert_eq!(
            worktree_branch_name("Carga 01 (burst)"),
            "lina/carga-01-burst"
        );
    }

    /// Base do aceite F3-4-1: dois nós no MESMO projeto recebem branches DIFERENTES.
    #[test]
    fn distinct_names_yield_distinct_branches() {
        assert_ne!(
            worktree_branch_name("Terminal A"),
            worktree_branch_name("Terminal B")
        );
    }

    /// `git check-ref-format` proíbe espaço, `~ ^ : ? * [ \`, `..`, e fim em `.`/`/`. O slug
    /// nunca pode produzir um refname que o `git worktree add -b` recusaria.
    #[test]
    fn branch_name_is_a_valid_git_refname() {
        for raw in [
            "Terminal A",
            "Carga 01 (burst)",
            "feat/x..y",
            "  ~weird^stuff~  ",
            "UPPER",
            "dot.",
        ] {
            let b = worktree_branch_name(raw);
            assert!(b.starts_with("lina/"), "{b} sem prefixo");
            assert!(!b.contains(".."), "{b} contém ..");
            assert!(
                !b.ends_with('.') && !b.ends_with('/') && !b.ends_with('-'),
                "{b} termina mal"
            );
            for c in [' ', '~', '^', ':', '?', '*', '[', '\\', '\t'] {
                assert!(!b.contains(c), "{b} contém char proibido {c:?}");
            }
        }
    }

    /// Nome vazio / só-símbolos → fallback não-vazio e válido (nunca `lina/` cru).
    #[test]
    fn branch_name_falls_back_on_empty_slug() {
        assert_eq!(worktree_branch_name("***"), "lina/agente");
        assert_eq!(worktree_branch_name(""), "lina/agente");
    }
}
