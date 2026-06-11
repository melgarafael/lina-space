//! **W4-5 · Galeria de Focos (T3) — presets que montam o Espaço com time + papéis, 100% local.**
//!
//! Módulo **gpui-free, puro e testável** (mesmo padrão de [`crate::wiring`]/[`crate::canvas`]): aqui
//! mora a LÓGICA durável de W4-5; o render gpui da galeria (grid de cards) a CONSOME no shell.
//!
//! Dois passos, na ordem da realidade do core:
//! 1. [`preset_team`] — **puro** — deriva o time `{nome, papel}` pelo **role-discovery** (W3-1). NÃO
//!    cunha `NodeId`: o id é alocado pelo `Supervisor` no spawn-vivo (fonte ÚNICA de id), e a galeria
//!    NÃO o duplica (senão a projeção e o roster `agents.json` divergiriam).
//! 2. [`apply_preset`] — **event-sourcing** (inv #4) — recebe os agentes JÁ com o `NodeId` que o
//!    shell alocou ([`PlacedAgent`]) e loga `WorkspaceCreated{focus_preset}` + por agente
//!    `NodeAdded`(Terminal) + `NodeRoleAssigned`. **NADA sai da máquina** (inv #2): só I/O do event
//!    log local — zero rede, zero PTY aqui (o PTY é cabeado pelo shell via `wire_terminal`, W3-2).
//!
//! Fluxo do shell (a cabear no render T3 — validação NA TELA):
//! ```ignore
//! let team = gallery::preset_team(preset, &registry);                  // {nome, papel} (puro)
//! let placed: Vec<_> = team.iter().map(|a| {
//!     let (node, _grid) = wire_terminal(&mut pty, &sup, …, &a.name, …); // Supervisor aloca o NodeId
//!     gallery::PlacedAgent { node, name: a.name.clone(), role: a.role.clone() }
//! }).collect();
//! gallery::apply_preset(preset, ws_name, &placed, &mut store)?;          // loga o MESMO id
//! ```
//!
//! ⚠️ **[A CONFIRMAR] taxonomia:** uso a do épico W4-5 (`dev_app|research_content|blank`); a
//! Arquitetura §9.2 lista `dev_app|landing|automation|design` — DIVERGEM (decidir o canônico do MVP).
//! A **composição de cada time** também não é fixada no épico (só os nomes dos presets) → defaults
//! sensatos, igualmente **[A CONFIRMAR]**.

// Módulo-biblioteca aguardando o WIRING do render gpui da galeria T3 no shell: a API pública é
// CONSUMIDA pelo render (a cabear) e, por ora, exercida pelos testes — mesmo padrão de `canvas.rs`/
// `onboarding.rs` (`allow(dead_code)` para o que ainda só os testes usam). Remover ao wirar o render.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use lina_core::{
    can_create_workspace, DomainEvent, EventStore, LicenseTier, StoreError, WorkspaceRegistry,
};
use lina_host::NodeId;
use lina_role_discovery::RoleRegistry;

use crate::workspace_boot::{self, WorkdirError};

/// Presets de Foco do MVP (épico W4-5). `Blank` é o caminho de fuga (Espaço vazio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPreset {
    /// "App" — time de desenvolvimento de produto.
    DevApp,
    /// "Pesquisa & Conteúdo" — curadoria + redação.
    ResearchContent,
    /// "Em Branco" — Espaço vazio (caminho de fuga).
    Blank,
}

impl FocusPreset {
    /// Todos os presets, na ordem de exibição da galeria (App → Pesquisa → Em Branco).
    #[must_use]
    pub fn all() -> [FocusPreset; 3] {
        [
            FocusPreset::DevApp,
            FocusPreset::ResearchContent,
            FocusPreset::Blank,
        ]
    }

    /// Id canônico gravado em `WorkspaceCreated.focus_preset` (projeção SQLite). Estável (contrato).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            FocusPreset::DevApp => "dev_app",
            FocusPreset::ResearchContent => "research_content",
            FocusPreset::Blank => "blank",
        }
    }

    /// Rótulo do card (PT-BR, zero jargão — inv #6, não-técnico-first).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            FocusPreset::DevApp => "App",
            FocusPreset::ResearchContent => "Pesquisa & Conteúdo",
            FocusPreset::Blank => "Em Branco",
        }
    }

    /// Descrição curta do card (o que o leigo ganha ao escolher).
    #[must_use]
    pub fn blurb(self) -> &'static str {
        match self {
            FocusPreset::DevApp => {
                "Um time de produto pronto: arquiteto, back-end, front-end e qualidade."
            }
            FocusPreset::ResearchContent => {
                "Curadoria e redação para pesquisa e conteúdo, já com papéis."
            }
            FocusPreset::Blank => "Comece do zero — você adiciona agentes quando quiser.",
        }
    }

    /// Nomes-semente do time; o **papel** é DERIVADO por nome via role-discovery (W3-1). `Blank` = vazio.
    /// **[A CONFIRMAR]** — composição não fixada no épico.
    #[must_use]
    fn team_seed(self) -> &'static [&'static str] {
        match self {
            FocusPreset::DevApp => &["@Arquiteto", "@Dev Backend", "@Dev Frontend", "@QA"],
            FocusPreset::ResearchContent => &["@Curador", "@Redator"],
            FocusPreset::Blank => &[],
        }
    }
}

/// Um membro do time a montar: nome-semente + papel derivado (role-discovery). **Sem `NodeId`** — o
/// id é do `Supervisor`, alocado no spawn-vivo do shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetAgent {
    pub name: String,
    pub role: String,
}

/// Um agente JÁ posicionado: o [`PresetAgent`] + o `NodeId` que o `Supervisor` alocou. É o que
/// [`apply_preset`] loga (o MESMO id que o roster vivo/`agents.json` usa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedAgent {
    pub node: NodeId,
    pub name: String,
    pub role: String,
}

/// Deriva o time de um preset — nome-semente → papel via role-discovery (W3-1). **Puro** (sem I/O,
/// sem rede, sem cunhar id).
#[must_use]
pub fn preset_team(preset: FocusPreset, registry: &RoleRegistry) -> Vec<PresetAgent> {
    preset
        .team_seed()
        .iter()
        .map(|name| PresetAgent {
            name: (*name).to_string(),
            role: registry.infer_role(name).role,
        })
        .collect()
}

/// **W4-5 — aplica um preset (camada durável, local-first).** Loga no `store`, em ordem:
/// `WorkspaceCreated{focus_preset}` e, por agente JÁ posicionado, `NodeAdded`(kind `Terminal`) +
/// `NodeRoleAssigned`. **Não abre rede** (inv #2) e **não spawna PTY** (runtime do shell). `team` vem
/// do shell, que alocou o `NodeId` de cada agente via `Supervisor` (id ÚNICO, igual ao roster vivo).
///
/// `Blank` (team vazio) loga só `WorkspaceCreated{focus_preset:"blank"}` — Espaço vazio (fuga).
///
/// # Errors
/// [`StoreError`] na 1ª falha de persistência do event log (aborta — sem meio-termo silencioso).
pub fn apply_preset(
    preset: FocusPreset,
    workspace_name: &str,
    team: &[PlacedAgent],
    store: &mut EventStore,
) -> Result<(), StoreError> {
    store.append(&DomainEvent::WorkspaceCreated {
        name: workspace_name.to_string(),
        focus_preset: preset.id().to_string(),
    })?;

    // Coordenadas-semente em linha; o canvas (W4-2) reposiciona — aqui só não nascem sobrepostos.
    for (i, agent) in team.iter().enumerate() {
        let x = 30.0 + (i as f64) * 360.0;
        store.append(&DomainEvent::NodeAdded {
            node: agent.node,
            kind: "Terminal".to_string(),
            x,
            y: 96.0,
            requested_by: None,
        })?;
        store.append(&DomainEvent::NodeRoleAssigned {
            node: agent.node,
            role: agent.role.clone(),
        })?;
    }
    Ok(())
}

// ═══════════════════════ M9 — modal Criar Espaço (M8-T5) ═══════════════════════
//
// Galeria de Focos COM seletor de Diretório de Trabalho embutido (decisão do fundador,
// 2026-06-09 — NÃO o modal simples do Maestri). Mesmo split do M6 (`agent_modal`):
// [`CreateSpaceModal`] é o estado PURO (gpui-free, testável headless); a casca de render
// e o roteamento de teclado/cliques são do shell (contrato declarado na entrega M8-T5).

// ── copy auditável (citações: copy-f1-4.md §1a · spec-m8-m9 §2/§4 · ux-flows mapa #6/#8) ──

/// Rótulo LITERAL do fundador (F1-4-1 critério 6; spec §2.2).
pub const COPY_M9_WORKDIR_LABEL: &str = "Diretório de Trabalho";
/// Label programático do campo de nome (spec §4 M9).
pub const COPY_M9_NAME_LABEL: &str = "Nome";
/// Botão do seletor nativo de pasta (ux-flows mapa de cliques #8; spec §2.2).
pub const COPY_M9_BROWSE: &str = "Procurar…";
/// Botão primário (ux-flows mapa #6 — `[[ Criar Espaço ]]`).
pub const COPY_M9_CREATE: &str = "Criar Espaço";
/// Botão de saída sem criar (spec §4 M9 — ordem de tab termina nele).
pub const COPY_M9_CANCEL: &str = "Cancelar";
/// Vitrine 4 do gating free=1 (copy-f1-4.md §1a, congelada — citação literal).
pub const COPY_M9_BLOCKED_FREE: &str = "Você já usa o Espaço do plano Free (1 de 1)";
/// CTA da vitrine (copy-f1-4.md §1a).
pub const COPY_M9_BLOCKED_CTA: &str = "Conhecer o PRO";
/// Saída do erro «não acessível» na CRIAÇÃO (spec §2 tabela de erros, coluna Saída).
pub const COPY_M9_ERR_EXIT_BROWSE: &str = "Procurar…";
/// Saída do erro «sem permissão de escrita» (reuso literal do M6 — ux-flows:207).
pub const COPY_M9_ERR_EXIT_OTHER: &str = "Escolher outra";
/// Saída do erro «falha ao criar» (reuso literal do M9 — ux-flows:774).
pub const COPY_M9_ERR_EXIT_OTHER_PLACE: &str = "Escolher outro lugar";

/// Sugestão de Diretório de Trabalho quando `Settings.default_cwd` não existe
/// (ux-flows:681 — `~/EspaçosDeTrabalho/{nome}`). Exposta p/ o render exibir o template.
#[must_use]
pub fn workdir_template(name: &str) -> String {
    let base = name.trim();
    let base = if base.is_empty() { "Meu Espaço" } else { base };
    format!("~/EspaçosDeTrabalho/{base}")
}

/// Erro do Diretório de Trabalho PRONTO para a superfície: a string leiga congelada
/// (spec §2) + o rótulo da saída honesta daquela situação (cada erro tem a SUA saída —
/// a tabela da spec difere por variante).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkdirErrorView {
    /// Mensagem leiga (== `WorkdirError::to_string()`, congelada na spec §2).
    pub message: String,
    /// Rótulo do botão de saída (Procurar… / Escolher outra / Escolher outro lugar).
    pub exit: &'static str,
}

impl From<WorkdirError> for WorkdirErrorView {
    fn from(e: WorkdirError) -> Self {
        let exit = match e {
            WorkdirError::NotAccessible => COPY_M9_ERR_EXIT_BROWSE,
            WorkdirError::NoWritePermission => COPY_M9_ERR_EXIT_OTHER,
            WorkdirError::CreateFailed => COPY_M9_ERR_EXIT_OTHER_PLACE,
        };
        Self {
            message: e.to_string(),
            exit,
        }
    }
}

/// Alvos de foco do M9 na ORDEM de tab da spec §4: cards → Nome → Diretório →
/// Procurar → Criar → Cancelar (ordem de leitura = ordem visual). No estado
/// bloqueado (free=1) o ciclo é Upsell → Cancelar (o card de upsell é focável e
/// lê a string congelada inteira).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M9Focus {
    Cards,
    Name,
    Workdir,
    Browse,
    Create,
    Cancel,
    /// Card de upsell free=1 (vitrine 4) — só existe com `blocked`.
    Upsell,
}

/// **O modal M9 — estado puro (nenhum gpui; nenhum side effect até o Criar).**
///
/// Contrato de teclado (roteado pelo shell, precedência de modal — `stop_propagation`):
/// - foco inicial: 1º card de Foco (ou o card de upsell, se `blocked`);
/// - `Tab`/`Shift-Tab` → [`Self::focus_next`]/[`Self::focus_prev`];
/// - `←→`/`↑↓` com foco nos cards → [`Self::select_prev_preset`]/[`Self::select_next_preset`];
/// - imprimível/Backspace → [`Self::type_char`]/[`Self::backspace`] (campo focado);
/// - `Enter` SUBMETE SEMPRE → [`Self::submit`] (a precedência da Fila de Atenção vale
///   só para `⌘⏎` — spec §4, correção da banca);
/// - `Esc` fecha sem criar (o shell dropa o modal; nada a desfazer — estado é RAM).
pub struct CreateSpaceModal {
    /// Card de Foco selecionado (índice em [`FocusPreset::all`]).
    pub preset_idx: usize,
    /// Nome do Espaço (pré-preenchido pelo Foco até o usuário tocar).
    pub name: String,
    /// `true` → o usuário editou o Nome; trocar de card NÃO sobrescreve mais.
    pub name_touched: bool,
    /// Diretório de Trabalho como exibido (pode conter `~`; expandido no submit).
    pub workdir: String,
    /// `true` → o usuário tocou o campo; o pré-preenchimento NUNCA sobrescreve mais.
    pub workdir_touched: bool,
    /// Erro corrente do Diretório (validação CEDO: na seleção E na criação).
    pub error: Option<WorkdirErrorView>,
    /// Vitrine free=1 (copy §1a) — `Some` ⇒ criação indisponível, upsell no lugar.
    pub blocked: Option<String>,
    /// Alvo de foco corrente (ordem de tab da spec §4).
    pub focus: M9Focus,
    /// `Settings.default_cwd` (T7) — base do pré-preenchimento quando definido.
    default_cwd: Option<String>,
    /// Nomes de Espaço já tomados (registry) — aviso leve de duplicado (padrão M6).
    existing: Vec<String>,
}

impl CreateSpaceModal {
    /// Abre o modal. O gating roda AQUI (antes de qualquer esforço — spec §3): com
    /// `tier`/`active_count` no limite, o modal nasce na vitrine de upsell (§1a).
    /// `default_cwd` = `Settings.default_cwd` (T7) se definido; `existing` = nomes
    /// já inscritos no registry (aviso de duplicado).
    #[must_use]
    pub fn new(
        tier: LicenseTier,
        active_count: usize,
        default_cwd: Option<String>,
        existing: Vec<String>,
    ) -> Self {
        let blocked = can_create_workspace(tier, active_count)
            .map_or(Some(COPY_M9_BLOCKED_FREE.into()), |()| None);
        let focus = if blocked.is_some() {
            M9Focus::Upsell
        } else {
            M9Focus::Cards
        };
        let mut m = Self {
            preset_idx: 0,
            name: String::new(),
            name_touched: false,
            workdir: String::new(),
            workdir_touched: false,
            error: None,
            blocked,
            focus,
            default_cwd,
            existing,
        };
        m.refresh_prefills();
        m
    }

    /// O preset do card selecionado.
    #[must_use]
    pub fn preset(&self) -> FocusPreset {
        FocusPreset::all()[self.preset_idx]
    }

    /// Seleciona um card de Foco. Enquanto o usuário não tocou o Nome, o nome
    /// acompanha o rótulo do Foco (e o Diretório acompanha o nome, mesma regra).
    pub fn select_preset(&mut self, idx: usize) {
        self.preset_idx = idx.min(FocusPreset::all().len() - 1);
        self.refresh_prefills();
    }

    /// Próximo card (→/↓ com foco nos cards).
    pub fn select_next_preset(&mut self) {
        self.select_preset((self.preset_idx + 1) % FocusPreset::all().len());
    }

    /// Card anterior (←/↑ com foco nos cards).
    pub fn select_prev_preset(&mut self) {
        let n = FocusPreset::all().len();
        self.select_preset((self.preset_idx + n - 1) % n);
    }

    /// Re-aplica os pré-preenchimentos SEM atropelar o que o usuário já editou
    /// (flags `*_touched` — spec §2.2: "depois de editado manualmente, nunca sobrescrever").
    fn refresh_prefills(&mut self) {
        if !self.name_touched {
            self.name = self.preset().label().to_string();
        }
        if !self.workdir_touched {
            self.workdir = self
                .default_cwd
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| workdir_template(&self.name));
        }
    }

    /// Caractere imprimível no campo focado. Editar o Nome mantém o Diretório
    /// acompanhando (enquanto não tocado); editar o Diretório o destaca para sempre
    /// e limpa o erro corrente (re-valida na seleção/criação, não a cada tecla).
    pub fn type_char(&mut self, s: &str) {
        match self.focus {
            M9Focus::Name => {
                self.name.push_str(s);
                self.name_touched = true;
                if !self.workdir_touched {
                    self.workdir = self
                        .default_cwd
                        .clone()
                        .filter(|d| !d.trim().is_empty())
                        .unwrap_or_else(|| workdir_template(&self.name));
                }
            }
            M9Focus::Workdir => {
                self.workdir.push_str(s);
                self.workdir_touched = true;
                self.error = None;
            }
            _ => {}
        }
    }

    /// Backspace no campo focado (mesmas regras de acompanhamento do [`Self::type_char`]).
    pub fn backspace(&mut self) {
        match self.focus {
            M9Focus::Name => {
                self.name.pop();
                self.name_touched = true;
                if !self.workdir_touched {
                    self.workdir = self
                        .default_cwd
                        .clone()
                        .filter(|d| !d.trim().is_empty())
                        .unwrap_or_else(|| workdir_template(&self.name));
                }
            }
            M9Focus::Workdir => {
                self.workdir.pop();
                self.workdir_touched = true;
                self.error = None;
            }
            _ => {}
        }
    }

    /// A ordem de tab da spec §4 (cards → Nome → Diretório → Procurar → Criar →
    /// Cancelar, circular). Bloqueado: Upsell → Cancelar.
    fn tab_ring(&self) -> &'static [M9Focus] {
        if self.blocked.is_some() {
            &[M9Focus::Upsell, M9Focus::Cancel]
        } else {
            &[
                M9Focus::Cards,
                M9Focus::Name,
                M9Focus::Workdir,
                M9Focus::Browse,
                M9Focus::Create,
                M9Focus::Cancel,
            ]
        }
    }

    /// `Tab` — avança o foco na ordem da spec §4.
    pub fn focus_next(&mut self) {
        let ring = self.tab_ring();
        let i = ring.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = ring[(i + 1) % ring.len()];
    }

    /// `Shift-Tab` — volta o foco.
    pub fn focus_prev(&mut self) {
        let ring = self.tab_ring();
        let i = ring.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = ring[(i + ring.len() - 1) % ring.len()];
    }

    /// O que a live-region (AccessKit) anuncia para o foco CORRENTE (spec §4 M9):
    /// erro do Diretório ao focar o campo; vitrine inteira ao focar o card de upsell.
    #[must_use]
    pub fn announcement(&self) -> Option<String> {
        match self.focus {
            M9Focus::Workdir => self.error.as_ref().map(|e| e.message.clone()),
            M9Focus::Upsell => self
                .blocked
                .as_ref()
                .map(|b| format!("{b} — {COPY_M9_BLOCKED_CTA}")),
            _ => None,
        }
    }

    /// Pasta devolvida pelo seletor NATIVO (`[ Procurar… ]` — o shell chama
    /// `prompt_for_paths`, mesmo mecanismo do M6, e entrega o resultado aqui).
    /// Valida na SELEÇÃO (spec §2.2 — cedo, nunca só no fim).
    pub fn set_workdir_picked(&mut self, path: &Path) {
        self.workdir = path.display().to_string();
        self.workdir_touched = true;
        self.error = workspace_boot::validate_workdir(path).err().map(Into::into);
    }

    /// O Diretório como caminho REAL (`~` expandido via `$HOME`) — o que valida e cria.
    #[must_use]
    pub fn expanded_workdir(&self) -> PathBuf {
        let raw = self.workdir.trim();
        if let Some(home) = std::env::var_os("HOME") {
            if raw == "~" {
                return PathBuf::from(home);
            }
            if let Some(rest) = raw.strip_prefix("~/") {
                return PathBuf::from(home).join(rest);
            }
        }
        PathBuf::from(raw)
    }

    /// Valida o Diretório corrente (mesma régua da criação — `validate_workdir`, T3).
    /// Pasta inexistente NÃO é erro (o caminho feliz a cria). Devolve `true` se ok.
    pub fn validate(&mut self) -> bool {
        self.error = workspace_boot::validate_workdir(&self.expanded_workdir())
            .err()
            .map(Into::into);
        self.error.is_none()
    }

    /// Aviso leve de nome duplicado (padrão M6: sufixo automático, NÃO bloqueia).
    /// `Some` quando o nome atual já existe — o texto é o `copy_dup_note` do M6
    /// (reuso literal) com o nome que o Espaço de fato ganhará.
    #[must_use]
    pub fn dup_note(&self) -> Option<String> {
        let base = self.name.trim();
        let base = if base.is_empty() { "Meu Espaço" } else { base };
        let taken = |n: &str| self.existing.iter().any(|e| e.trim() == n);
        if !taken(base) {
            return None;
        }
        let unique = (2..100)
            .map(|i| format!("{base} ({i})"))
            .find(|c| !taken(c))
            .unwrap_or_else(|| format!("{base} (∞)"));
        Some(crate::agent_modal::copy_dup_note(&unique))
    }

    /// **`[[ Criar Espaço ]]` / Enter.** Valida CEDO de novo (a pasta pode ter mudado
    /// desde a seleção), cria via [`workspace_boot::create_workspace`] (T3 — gating,
    /// dedup de nome/pasta, eventos canônicos, registry sem foco) e devolve o
    /// `ws_root` ao caller via `on_created` — o FOCO/troca pós-criação é da
    /// integração (shell), não do modal. Devolve `true` quando criou (o shell fecha
    /// o modal); `false` mantém o modal aberto com o erro/vitrine na tela.
    pub fn submit(
        &mut self,
        parent: &Path,
        registry: &mut WorkspaceRegistry,
        now_ms: u64,
        tier: LicenseTier,
        on_created: impl FnOnce(PathBuf),
    ) -> bool {
        if self.blocked.is_some() {
            return false; // vitrine §1a no lugar do form — nada a submeter.
        }
        if !self.validate() {
            self.focus = M9Focus::Workdir; // erro anunciado ao focar o campo (spec §4).
            return false;
        }
        let workdir = self.expanded_workdir();
        match workspace_boot::create_workspace(
            parent,
            &self.name,
            &self.preset(),
            Some(&workdir),
            registry,
            now_ms,
            tier,
        ) {
            Ok(ws_root) => {
                on_created(ws_root);
                true
            }
            Err(msg) => {
                // Gating estourou ENTRE abrir e criar (outro processo criou um Espaço):
                // vira a mesma vitrine §1a — nunca um erro técnico cru.
                if can_create_workspace(tier, registry.active_count()).is_err() {
                    self.blocked = Some(COPY_M9_BLOCKED_FREE.into());
                    self.focus = M9Focus::Upsell;
                } else {
                    // Falha de disco/pasta (a string leiga já vem do T3 quando é o caso).
                    let exit = if msg == WorkdirError::CreateFailed.to_string() {
                        COPY_M9_ERR_EXIT_OTHER_PLACE
                    } else {
                        COPY_M9_ERR_EXIT_BROWSE
                    };
                    self.error = Some(WorkdirErrorView { message: msg, exit });
                    self.focus = M9Focus::Workdir;
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid; // `uuid` é dev-dependency — fabrica `NodeId` no teste (o que o Supervisor faria).

    fn registry() -> RoleRegistry {
        RoleRegistry::with_defaults().expect("default-roles.yaml embutido compila")
    }

    fn temp_store(tag: &str) -> (EventStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lina-gallery-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = EventStore::open(&dir).expect("abrir EventStore temp");
        (store, dir)
    }

    /// Simula o shell: deriva o time e "aloca" um `NodeId` por agente (o que `Supervisor::register`
    /// faria no spawn-vivo), devolvendo os [`PlacedAgent`] que `apply_preset` loga.
    fn place(preset: FocusPreset, reg: &RoleRegistry) -> Vec<PlacedAgent> {
        preset_team(preset, reg)
            .into_iter()
            .map(|a| PlacedAgent {
                node: Uuid::now_v7(),
                name: a.name,
                role: a.role,
            })
            .collect()
    }

    /// Aceite W4-5: preset "App" cria um Espaço com `focus_preset='dev_app'` na projeção + N nós, cada
    /// um com o papel DERIVADO pelo role-discovery (W3-1) — não um papel hardcoded na galeria.
    #[test]
    fn apply_dev_app_logs_focus_preset_and_team_with_roles() {
        let reg = registry();
        let (mut store, dir) = temp_store("devapp");
        let placed = place(FocusPreset::DevApp, &reg);
        assert_eq!(placed.len(), 4, "time de 4 agentes");

        apply_preset(FocusPreset::DevApp, "App de Teste", &placed, &mut store).expect("apply");

        let state = store.project().expect("projetar");
        assert_eq!(
            state.focus_preset.as_deref(),
            Some("dev_app"),
            "focus_preset na projecao SQLite"
        );
        assert_eq!(state.nodes.len(), 4, "4 nos na projecao");

        // Cada nó projetado tem o papel inferido pelo role-discovery p/ o seu nome (prova a W3-1).
        for agent in &placed {
            let expected = reg.infer_role(&agent.name).role;
            assert_eq!(agent.role, expected, "papel via role-discovery");
            let node = state.nodes.get(&agent.node).expect("no na projecao");
            assert_eq!(
                node.role.as_deref(),
                Some(expected.as_str()),
                "papel projetado"
            );
            assert_eq!(node.kind, "Terminal");
        }

        // O time de "App" cobre os papéis de produto esperados (arquitetura + back + front + QA).
        let roles: std::collections::BTreeSet<&str> =
            placed.iter().map(|a| a.role.as_str()).collect();
        for must in ["ARQUITETO", "BACKEND", "FRONTEND", "QA"] {
            assert!(
                roles.contains(must),
                "preset App deve ter o papel {must}; veio {roles:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Aceite W4-5: "Em Branco" cria Espaço VAZIO (caminho de fuga) — só o `WorkspaceCreated`, 0 nós.
    #[test]
    fn apply_blank_is_empty_escape_hatch() {
        let reg = registry();
        let (mut store, dir) = temp_store("blank");
        let placed = place(FocusPreset::Blank, &reg);
        assert!(placed.is_empty(), "Em Branco nao spawna ninguem");

        apply_preset(FocusPreset::Blank, "Vazio", &placed, &mut store).expect("apply blank");

        let state = store.project().expect("projetar");
        assert_eq!(state.focus_preset.as_deref(), Some("blank"));
        assert!(state.nodes.is_empty(), "Espaco vazio");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Local-first (inv #2): o ÚNICO efeito de `apply_preset` é o event log — exatamente
    /// `1 WorkspaceCreated + N*(NodeAdded + NodeRoleAssigned)`, nada mais (sem rede, sem PTY, sem
    /// evento surpresa). Prova estrutural de que a criação não dispara nada além do log local.
    #[test]
    fn apply_preset_side_effects_are_only_local_events() {
        let reg = registry();
        let (mut store, dir) = temp_store("localfirst");
        let placed = place(FocusPreset::DevApp, &reg);

        apply_preset(FocusPreset::DevApp, "App", &placed, &mut store).expect("apply");

        let kinds: Vec<String> = store
            .events()
            .expect("events")
            .into_iter()
            .map(|r| r.kind)
            .collect();
        // 1 WorkspaceCreated + 4*(NodeAdded + NodeRoleAssigned) = 9 eventos, todos locais.
        assert_eq!(kinds.len(), 9, "exatamente 9 eventos; veio {kinds:?}");
        assert_eq!(kinds[0], "WorkspaceCreated");
        assert_eq!(
            kinds.iter().filter(|k| *k == "NodeAdded").count(),
            4,
            "4 NodeAdded"
        );
        assert_eq!(
            kinds.iter().filter(|k| *k == "NodeRoleAssigned").count(),
            4,
            "4 NodeRoleAssigned"
        );
        // Nenhum evento de rede/entrega/discovery foi disparado na criação.
        for forbidden in ["MessageRouted", "MessageDelivered", "DiscoveryIndexed"] {
            assert!(
                !kinds.iter().any(|k| k == forbidden),
                "criacao nao deve disparar {forbidden}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `preset_team` deriva papéis pelo role-discovery (W3-1), sem cunhar id (isso é do Supervisor).
    #[test]
    fn preset_team_uses_role_discovery_without_minting_ids() {
        let reg = registry();
        let team = preset_team(FocusPreset::ResearchContent, &reg);
        assert_eq!(team.len(), 2);
        for agent in &team {
            assert_eq!(agent.role, reg.infer_role(&agent.name).role);
        }
    }

    // ═════════════════════ M9 — CreateSpaceModal (headless) ═════════════════════

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lina-m9-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("criar temp dir");
        dir
    }

    fn open_modal(tier: LicenseTier, active: usize) -> CreateSpaceModal {
        CreateSpaceModal::new(tier, active, None, Vec::new())
    }

    /// Spec §2.2: o Diretório acompanha o nome (template `~/EspaçosDeTrabalho/{nome}`)
    /// ENQUANTO o usuário não tocou o campo; depois de tocado, NUNCA é sobrescrito.
    /// O Nome, por sua vez, acompanha o Foco até ser tocado.
    #[test]
    fn m9_prefill_segue_o_nome_ate_o_usuario_tocar() {
        let mut m = open_modal(LicenseTier::Pro, 0);
        assert_eq!(m.name, "App", "nome pré-preenchido pelo Foco");
        assert_eq!(m.workdir, "~/EspaçosDeTrabalho/App");

        // Trocar o card acompanha nome E diretório (nada tocado ainda).
        m.select_preset(1);
        assert_eq!(m.name, "Pesquisa & Conteúdo");
        assert_eq!(m.workdir, "~/EspaçosDeTrabalho/Pesquisa & Conteúdo");

        // Editar o NOME: diretório continua acompanhando (campo dele não foi tocado)…
        m.focus = M9Focus::Name;
        m.type_char("!");
        assert!(m.name_touched);
        assert_eq!(m.workdir, "~/EspaçosDeTrabalho/Pesquisa & Conteúdo!");
        // …e trocar o card NÃO atropela mais o nome editado.
        m.select_preset(0);
        assert_eq!(m.name, "Pesquisa & Conteúdo!", "nome tocado é do usuário");

        // Tocar o DIRETÓRIO: a partir daqui nada o sobrescreve.
        m.focus = M9Focus::Workdir;
        m.type_char("x");
        assert!(m.workdir_touched);
        let frozen = m.workdir.clone();
        m.focus = M9Focus::Name;
        m.type_char("?");
        m.select_preset(2);
        assert_eq!(m.workdir, frozen, "diretório tocado nunca é sobrescrito");
    }

    /// `Settings.default_cwd` (T7) definido vence o template como base do pré-preenchimento.
    #[test]
    fn m9_default_cwd_do_settings_e_a_base_quando_definido() {
        let m = CreateSpaceModal::new(
            LicenseTier::Pro,
            0,
            Some("/tmp/projetos".into()),
            Vec::new(),
        );
        assert_eq!(m.workdir, "/tmp/projetos");
    }

    /// Padrão M6 (spec §2.1): nome duplicado ganha sufixo « (2)» com AVISO leve —
    /// não bloqueia; o texto é o `copy_dup_note` do M6 (reuso literal).
    #[test]
    fn m9_nome_duplicado_avisa_com_o_sufixo_m6() {
        let mut m = CreateSpaceModal::new(
            LicenseTier::Pro,
            0,
            None,
            vec!["App".into(), "App (2)".into()],
        );
        let note = m.dup_note().expect("aviso leve presente");
        assert_eq!(note, crate::agent_modal::copy_dup_note("App (3)"));
        // Nome livre → sem aviso.
        m.name_touched = true;
        m.name = "Cliente X".into();
        assert!(m.dup_note().is_none());
    }

    /// Vitrine 4 do gating (§1a): Free com 1 ativo abre o modal BLOQUEADO — string
    /// congelada, foco no card de upsell (focável, anuncia a frase inteira), tab
    /// alcança só Cancelar, e submit não cria NADA.
    #[test]
    fn m9_blocked_free_renderiza_vitrine_sem_criar_nada() {
        let mut m = open_modal(LicenseTier::Free, 1);
        assert_eq!(m.blocked.as_deref(), Some(COPY_M9_BLOCKED_FREE));
        assert_eq!(m.focus, M9Focus::Upsell);
        assert_eq!(
            m.announcement().as_deref(),
            Some("Você já usa o Espaço do plano Free (1 de 1) — Conhecer o PRO")
        );
        m.focus_next();
        assert_eq!(m.focus, M9Focus::Cancel, "Esc/Cancelar — nunca beco");

        let parent = temp_dir("blocked");
        let mut registry =
            WorkspaceRegistry::load(parent.join("registry.json")).expect("registry vazio");
        let mut created = None;
        let ok = m.submit(&parent, &mut registry, 1, LicenseTier::Free, |r| {
            created = Some(r);
        });
        assert!(!ok && created.is_none(), "Blocked não submete");
        assert_eq!(
            std::fs::read_dir(&parent).unwrap().count(),
            0,
            "zero side effects no disco"
        );
        // Com tier Pro (stub desta fatia), nenhuma vitrine aparece.
        assert!(open_modal(LicenseTier::Pro, 1).blocked.is_none());
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// Spec §2 (tabela de erros): cada situação do Diretório mapeia para a SUA string
    /// leiga congelada + saída; pasta INEXISTENTE não é erro (o caminho feliz cria).
    #[test]
    fn m9_erro_de_workdir_mapeia_para_a_string_certa() {
        let dir = temp_dir("errs");
        let mut m = open_modal(LicenseTier::Pro, 0);

        // É arquivo → «não está acessível» + Procurar…
        let file = dir.join("arquivo.txt");
        std::fs::write(&file, b"x").unwrap();
        m.set_workdir_picked(&file);
        let e = m.error.clone().expect("erro presente");
        assert_eq!(
            e.message,
            "Essa pasta não está acessível — escolha outra ou crie uma nova."
        );
        assert_eq!(e.exit, COPY_M9_ERR_EXIT_BROWSE);
        // Erro anunciado ao focar o campo (spec §4).
        m.focus = M9Focus::Workdir;
        assert_eq!(m.announcement().as_deref(), Some(e.message.as_str()));

        // Sem escrita (chmod 000) → «não consigo trabalhar» + Escolher outra.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let sealed = dir.join("selada");
            std::fs::create_dir(&sealed).unwrap();
            std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000)).unwrap();
            if std::fs::write(sealed.join("probe"), b"x").is_err() {
                m.set_workdir_picked(&sealed);
                let e = m.error.clone().expect("erro presente");
                assert_eq!(e.message, "Não consigo trabalhar nessa pasta.");
                assert_eq!(e.exit, COPY_M9_ERR_EXIT_OTHER);
            }
            std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // Pasta que NÃO existe → ok (validação limpa o erro).
        m.set_workdir_picked(&dir.join("ainda-nao-existe"));
        assert!(m.error.is_none(), "inexistente não é erro");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Spec §4 M9: foco inicial no 1º card; tab percorre cards → Nome → Diretório →
    /// Procurar → Criar → Cancelar (circular, Shift-Tab inverte); ←→ navega os cards.
    #[test]
    fn m9_teclado_ordem_de_tab_e_navegacao_de_cards() {
        let mut m = open_modal(LicenseTier::Pro, 0);
        assert_eq!(m.focus, M9Focus::Cards, "foco inicial no 1º card");
        let order = [
            M9Focus::Name,
            M9Focus::Workdir,
            M9Focus::Browse,
            M9Focus::Create,
            M9Focus::Cancel,
            M9Focus::Cards, // circular
        ];
        for expected in order {
            m.focus_next();
            assert_eq!(m.focus, expected);
        }
        m.focus_prev();
        assert_eq!(m.focus, M9Focus::Cancel, "Shift-Tab inverte");

        // ←→ nos cards (com wrap), sem sair do foco.
        m.focus = M9Focus::Cards;
        m.select_next_preset();
        assert_eq!(m.preset(), FocusPreset::ResearchContent);
        m.select_prev_preset();
        m.select_prev_preset();
        assert_eq!(m.preset(), FocusPreset::Blank, "wrap para trás");
        assert_eq!(m.focus, M9Focus::Cards);
    }

    /// Caminho feliz fim-a-fim do estado: Enter/Criar valida de novo, cria via T3 e
    /// devolve o `ws_root` pelo callback `on_created` — com `WorkspaceDefaultCwdSet`
    /// apontando o Diretório expandido. O foco/troca é do caller (não testado aqui —
    /// não é do modal).
    #[test]
    fn m9_submit_cria_o_espaco_e_devolve_ws_root_ao_caller() {
        let parent = temp_dir("submit");
        let workdir = parent.join("area-de-trabalho");
        let mut registry =
            WorkspaceRegistry::load(parent.join("registry.json")).expect("registry vazio");

        let mut m = open_modal(LicenseTier::Pro, 0);
        m.select_preset(2); // Em Branco — sem time, criação enxuta p/ o teste.
        m.set_workdir_picked(&workdir); // inexistente: não é erro, o submit cria.
        assert!(m.error.is_none());

        let mut created: Option<std::path::PathBuf> = None;
        let ok = m.submit(&parent, &mut registry, 7, LicenseTier::Pro, |r| {
            created = Some(r);
        });
        assert!(ok, "submit criou; erro: {:?}", m.error);
        let ws_root = created.expect("ws_root devolvido ao caller");
        assert!(ws_root.starts_with(&parent) && ws_root.exists());

        // A fonte da verdade nasceu certa: preset + Diretório no log do Espaço NOVO.
        let store = EventStore::open(lina_core::Workspace::events_dir(&ws_root)).expect("abrir");
        let state = store.project().expect("projetar");
        assert_eq!(state.focus_preset.as_deref(), Some("blank"));
        assert_eq!(
            state.default_cwd.as_deref(),
            Some(workdir.display().to_string().as_str()),
            "WorkspaceDefaultCwdSet com o caminho expandido"
        );
        assert_eq!(registry.entries().len(), 1, "inscrito no registry global");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
