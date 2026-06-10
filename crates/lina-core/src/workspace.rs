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
        let root = root.as_ref().to_path_buf();
        // Valida ANTES de qualquer append: recusa de input não deixa Espaço meio-criado.
        let default_cwd = default_cwd.map(utf8_path).transpose()?.map(str::to_string);
        let mut store = EventStore::open(Self::events_dir(&root))?;
        if store.project()?.workspace_name.is_some() {
            return Err(WorkspaceError::AlreadyExists { root });
        }
        store.append(&DomainEvent::WorkspaceCreated {
            name: name.to_string(),
            focus_preset: focus_preset.to_string(),
        })?;
        store.append(&DomainEvent::WorkspaceIdAssigned {
            workspace_id: Uuid::now_v7().to_string(),
        })?;
        if let Some(cwd) = default_cwd {
            store.append(&DomainEvent::WorkspaceDefaultCwdSet { cwd })?;
        }
        Ok(Self { root, store })
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
        let store = EventStore::open_or_recover(Self::events_dir(&root), ui)?;
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
    /// Log legado sem `WorkspaceIdAssigned` → id degrada para o path (honesto).
    pub fn registry_entry(&self) -> Result<WorkspaceEntry, WorkspaceError> {
        let state = self.project()?;
        let name = state
            .workspace_name
            .ok_or_else(|| WorkspaceError::NotAWorkspace {
                root: self.root.clone(),
            })?;
        let id = state
            .workspace_id
            .unwrap_or_else(|| self.root.display().to_string());
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
        if !path.exists() {
            return Ok(Self {
                path,
                entries: Vec::new(),
            });
        }
        let body = std::fs::read_to_string(&path)?;
        let file: RegistryFile = serde_json::from_str(&body)?;
        Ok(Self {
            path,
            entries: file.workspaces,
        })
    }

    /// Persiste ATÔMICO (tmp + `sync_all` + rename no mesmo dir — padrão do
    /// `plan.md`, ADR 0001): crash de processo nunca deixa o ponteiro meio-escrito,
    /// e o fsync do tmp fecha a janela rename-antes-dos-dados em perda de energia
    /// (o ponteiro segue re-derivável de qualquer forma).
    ///
    /// Antes de escrever, FUNDE com o arquivo corrente (união por id; `last_focus`
    /// máximo; fatos — nome/path/arquivado — do estado em memória vencem, pois vêm
    /// de re-derivação mais fresca do store): dois escritores intercalados (app +
    /// CLI) não apagam o Espaço um do outro — last-writer-wins é proibido num
    /// ponteiro que o leigo lê como "a lista dos meus Espaços". Arquivo corrente
    /// ilegível → segue só com o estado em memória (recuperação de ponteiro
    /// corrompido, por cima).
    pub fn save(&mut self) -> Result<(), WorkspaceError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Ok(current) = Self::load(&self.path) {
            for theirs in current.entries {
                match self.entries.iter_mut().find(|e| e.id == theirs.id) {
                    Some(mine) => mine.last_focus = mine.last_focus.max(theirs.last_focus),
                    None => self.entries.push(theirs),
                }
            }
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

    /// RE-DERIVA a entrada de um Espaço a partir do store dele (a autoridade) — o
    /// caminho de recuperação quando o ponteiro se perde/corrompe. Diretório sem store
    /// ou log sem `WorkspaceCreated` → `NotAWorkspace` (nunca inventa entrada).
    pub fn rederive_entry(root: impl AsRef<Path>) -> Result<WorkspaceEntry, WorkspaceError> {
        let root = root.as_ref().to_path_buf();
        if !Workspace::events_dir(&root).exists() {
            return Err(WorkspaceError::NotAWorkspace { root });
        }
        Workspace::open(&root)?.registry_entry()
    }
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
