//! `sidebar` — **M8 (F1-4-4): o Switcher de Espaços como rail esquerdo expansível.**
//!
//! Duas camadas, no padrão do app (`attention_ui`/`CreateSpaceModal`):
//! - **Estado gpui-free** ([`SidebarState`] + [`build_rows`] + células puras): 100%
//!   testável headless — lista canônica do `WorkspaceRegistry` com a varredura como
//!   fallback, filtro de busca, roving de seleção, índice `⌘{n}` ESTÁVEL e a
//!   apresentação honesta do mini-status (M8-T2, `dashboard.rs`).
//! - **Casca de render gpui** ([`Sidebar`]): rail colapsado (ícones ▣) ↔ expandido
//!   (busca + linhas da gramática da spec §1). Cores 100% em tokens de
//!   [`crate::theme`] (o lint anti-hardcode varre este arquivo).
//!
//! **Fronteiras desta rodada (despacho m8-r2-t4):** as interações chegam por
//! CALLBACKS injetados (`on_switch`/`on_create`/`on_rename`/`on_archive`) — a troca
//! real, o mount no shell e os atalhos ⌘O/⌘1..9 são fiação do Maestro na integração.
//! O 🔔 por Espaço segue BLOQUEADO pelo gap §6-C1 (spec): nenhuma linha exibe sino nem
//! afirma "em dia" — ausência de dado ≠ zero pendência (estado transitório declarado).
//!
//! **Moeda (PENDENTE-MAESTRO, spec §6-A1):** variante (a) — a célula de custo exibe a
//! forma curta REAL do produto (`~US$ {valor}`, vinda do mini-status) ou «—» sem dado.

// Headless ANTES da fiação, como `gallery.rs`/`dashboard.rs` (a fiação no shell é do
// Maestro nesta rodada — `switch_to_workspace`/mount/atalhos). Remover ao wirar o render.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{div, prelude::*, px, rgb, text, AnyElement, ClickEvent, Context, Role, Window};
use lina_core::WorkspaceEntry as RegistryEntry;

use crate::dashboard::{self, MiniStatusCache, UiState, WorkspaceMiniStatus};
use crate::persistence_ui::WorkspaceEntry as ScanEntry;
use crate::theme::Theme;
use crate::WorkspaceView;

// ─────────────── copy auditável (citações: copy-f1-4.md §5/§1a · ux-flows · spec §1/§4) ───────────────

/// Espaço sem nenhum terminal (copy-f1-4.md §5 — congelada).
pub const COPY_M8_NO_AGENTS: &str = "sem Agentes ainda";
/// Todos os vivos dormindo — caixa baixa INTENCIONAL em posição de mini-status
/// (copy-f1-4.md §5, seguindo o wireframe M8).
pub const COPY_M8_ALL_SLEEPING: &str = "dormindo";
/// Tooltip da célula de custo (copy-f1-4.md §5 — congelada).
pub const COPY_M8_COST_TOOLTIP: &str = "gasto estimado de hoje neste Espaço";
/// Item de criação no rodapé do M8 (copy-f1-4.md §1a — congelada).
pub const COPY_M8_NEW_SPACE: &str = "+ Novo Espaço…";
/// Selo do gating free=1 nas vitrines (copy-f1-4.md §1a — pequeno, sem tooltip de venda).
pub const COPY_M8_PRO_BADGE: &str = "PRO";
/// Link do rodapé para a vista de arquivados (copy-f1-4.md §5 — congelada).
pub const COPY_M8_ARCHIVED_LINK: &str = "Espaços arquivados ▸";
/// Ação de linha (copy-f1-4.md §5 — congelada).
pub const COPY_M8_RENAME: &str = "Renomear";
/// Ação de linha (copy-f1-4.md §5 — congelada).
pub const COPY_M8_ARCHIVE: &str = "Arquivar";
/// Linha-⚠ de Espaço cuja pasta sumiu (fragmento congelado de ux-flows:772; spec §1
/// "Erro: pasta do Espaço sumiu" — a linha NUNCA some, anti-padrão 6).
pub const COPY_M8_FOLDER_MISSING: &str = "pasta não encontrada";
/// Moldura do campo de busca (ux-flows:659 — literal do wireframe).
pub const COPY_M8_SEARCH_PLACEHOLDER: &str = "( buscar… )";
/// Busca sem resultado (copy-f1-4.md §5 — congelada).
pub const COPY_M8_SEARCH_EMPTY: &str = "Nenhum Espaço com esse nome.";

// F2-2-4 — strings da PORTA VISÍVEL da paleta de comandos. Não há fonte única de strings (R9) neste
// crate: a convenção é `COPY_*` por módulo (ver acima). Rótulo LEIGO ("Buscar comandos") + o atalho
// (⌘K) exibido no botão. O botão abre a MESMA paleta do ⌘K, materializando o invariante de fase
// "nada existe só atrás de atalho" (paleta escondida = paleta morta — D4-A1).
/// Rótulo visível do botão de busca de comandos (≠ a busca de Espaços, que é ⌘F).
pub const COPY_PALETTE_BUTTON: &str = "Buscar comandos";
/// Atalho exibido junto ao rótulo.
pub const COPY_PALETTE_SHORTCUT: &str = "⌘K";
/// Aria (a palavra viaja para o leitor de tela mesmo quando o rail está colapsado em ícone).
pub const COPY_PALETTE_ARIA: &str = "Buscar comandos — atalho Command-K";

/// Botão de criar a partir da busca vazia (copy-f1-4.md §5: `[ + Criar “{texto}” ]`).
#[must_use]
pub fn create_from_search_label(texto: &str) -> String {
    format!("+ Criar “{texto}”")
}

// ═══════════════ toast de arquivamento com Desfazer (spec-m8-m9 §M8, a11y) ═══════════════

/// Janela do Desfazer (ms). O `WorkspaceArchived` só é GRAVADO no log quando o toast expira
/// (commit adiado): desfazer dentro da janela cancela a gravação — sem precisar de um evento
/// `WorkspaceUnarchived` que o core ainda não tem (spec §6-C2). 8s cobre leitura + Tab+Enter.
pub const ARCHIVE_TOAST_TIMEOUT_MS: u64 = 8_000;

/// Rótulo do botão do toast (leigo, padrão de Desfazer).
pub const COPY_ARCHIVE_UNDO: &str = "Desfazer";

/// Anúncio do toast na live-region (a11y — spec §M8: o arquivamento é AUDÍVEL, não só cor).
#[must_use]
pub fn copy_archive_announce(name: &str) -> String {
    format!("Espaço “{name}” arquivado — nada se perde. Desfazer disponível por alguns segundos.")
}

/// Anúncio do Desfazer efetivado (live-region).
#[must_use]
pub fn copy_archive_undone(name: &str) -> String {
    format!("Arquivamento de “{name}” desfeito — o Espaço voltou à lista.")
}

// ═══════════════ r5 BUG 4 — roteador de teclado do rail (puro, testável) ═══════════════

/// Para onde uma tecla vai com o rail aberto. O princípio (tela do fundador, r5):
/// **digitação NUNCA é roubada** — o rail só consome teclado quando é o dono (`kbd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKey {
    /// `⌘O` — alterna o rail (abre/recolhe), de qualquer lugar.
    ToggleRail,
    /// `⌘F` com o rail aberto e sem foco — a busca assume o teclado.
    FocusSearch,
    /// O rail é o dono (`kbd`) — a tecla dirige busca/roving/rename/Esc-fecha.
    ToRail,
    /// Tudo o mais FLUI ao terminal focado (inclusive Esc com `kbd=false` — o rail
    /// aberto nunca rouba o Esc de um terminal).
    PassThrough,
}

/// Decide o destino de `(key, ⌘?)` dado `(expanded, kbd)`. Não-vacuoso por construção:
/// remover o conceito de `kbd` faz a matriz do BUG 4 falhar.
#[must_use]
pub fn rail_key_route(expanded: bool, kbd: bool, key: &str, platform: bool) -> RailKey {
    if platform && key == "o" {
        return RailKey::ToggleRail;
    }
    if !expanded {
        return RailKey::PassThrough;
    }
    if kbd {
        return RailKey::ToRail;
    }
    if platform && key == "f" {
        return RailKey::FocusSearch;
    }
    RailKey::PassThrough
}

/// Subtexto leigo do `[ Arquivar ]` (r5 BUG 2) — honesto sobre o que acontece.
pub const COPY_M8_ARCHIVE_HINT: &str = "some da lista; nada é apagado do disco";

// ═══════════════ r5 (unload-ui) — Descarregar Espaço de fundo ═══════════════

/// Rótulo da ação de linha (curto; a explicação viaja no hint/aria).
pub const COPY_M8_UNLOAD: &str = "Descarregar";
/// Explicação leiga (aria/tooltip) — honesta: só os OCIOSOS desligam, e religam.
pub const COPY_M8_UNLOAD_HINT: &str =
    "desliga os terminais ociosos deste Espaço — religam quando você voltar";

/// A ação Descarregar aparece nesta linha? Regra (despacho r5): só Espaço de FUNDO
/// (`row.focused` = ativo desta janela ⇒ nunca) e só com o seam do executor PLUGADO
/// (sem mecanismo, sem botão — tela nunca mente).
#[must_use]
pub fn row_can_unload(row_is_active: bool, seam_wired: bool) -> bool {
    seam_wired && !row_is_active
}

/// Narração pós-Descarregar (live-region + visual), dos números REAIS do
/// `UnloadPlan` do core: `parked` desligados, `kept` preservados porque trabalhavam.
#[must_use]
pub fn copy_unload_narration(parked: usize, kept: usize) -> String {
    let off = match parked {
        0 => "nenhum terminal precisou desligar".to_string(),
        1 => "1 terminal ocioso desligado — religa quando você voltar".to_string(),
        n => format!("{n} terminais ociosos desligados — religam quando você voltar"),
    };
    match kept {
        0 => format!("Espaço descarregado: {off}."),
        1 => format!("Espaço descarregado: 1 agente segue trabalhando e ficou ligado; {off}."),
        n => {
            format!("Espaço descarregado: {n} agentes seguem trabalhando e ficaram ligados; {off}.")
        }
    }
}

/// Estado headless do toast «Espaço arquivado · [ Desfazer ] (Ns)» — quem grava o evento
/// no timeout e remonta as linhas é a fiação do shell (main.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveToast {
    /// Raiz do Espaço com arquivamento PENDENTE (a linha some da lista, mas o log só
    /// recebe `WorkspaceArchived` quando a janela fecha).
    pub root: PathBuf,
    /// Nome exibido/anunciado.
    pub name: String,
    /// Instante (epoch ms) em que o commit acontece.
    pub deadline_ms: u64,
    /// `[ Desfazer ]` focado pelo teclado (Tab) — Enter o ativa.
    pub undo_focused: bool,
}

impl ArchiveToast {
    #[must_use]
    pub fn new(root: PathBuf, name: String, now_ms: u64) -> Self {
        Self {
            root,
            name,
            deadline_ms: now_ms.saturating_add(ARCHIVE_TOAST_TIMEOUT_MS),
            undo_focused: false,
        }
    }

    /// Janela fechou → a fiação grava o `WorkspaceArchived` e derruba o toast.
    #[must_use]
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms >= self.deadline_ms
    }

    /// Segundos restantes (teto), para o countdown do render.
    #[must_use]
    pub fn remaining_secs(&self, now_ms: u64) -> u64 {
        self.deadline_ms.saturating_sub(now_ms).div_ceil(1000)
    }
}

/// Efeito de uma tecla sobre o toast vivo (puro — a fiação do shell aplica).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveToastKey {
    /// `Tab` → move o foco do teclado para o `[ Desfazer ]`.
    FocusUndo,
    /// `⌘Z` (de qualquer lugar) ou `Enter` com o botão focado → desfaz.
    Undo,
    /// A tecla não é do toast — segue o fluxo normal.
    None,
}

/// Decide o efeito de `(key, ⌘?, foco)` sobre o toast — é o caminho de TECLADO exigido pela
/// spec (§M8: `[ Desfazer ]` alcançável por teclado antes do timeout): `Tab` foca, `Enter`
/// ativa; `⌘Z` desfaz direto (gesto universal), sem depender do foco.
#[must_use]
pub fn archive_toast_key(key: &str, platform: bool, undo_focused: bool) -> ArchiveToastKey {
    match key {
        "z" if platform => ArchiveToastKey::Undo,
        "tab" if !platform => ArchiveToastKey::FocusUndo,
        "enter" | "return" if !platform && undo_focused => ArchiveToastKey::Undo,
        _ => ArchiveToastKey::None,
    }
}

// ════════════════════════════ camada 1 · estado (gpui-free) ════════════════════════════

/// Uma linha do Switcher: a gramática da spec §1 já RESOLVIDA em dados
/// (`▣ {nome} {N} Agentes ● {custo} ⌘{n}`), com o mini-status honesto do M8-T2.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarRow {
    pub name: String,
    /// Raiz do Espaço (o que `on_switch` recebe; a troca real é fiação do Maestro).
    pub ws_root: PathBuf,
    /// É o Espaço ATIVO desta janela.
    pub focused: bool,
    /// Mini-status derivado do log daquele Espaço (M8-T2). `unreachable=true` ⇒ a
    /// linha vira ⚠ «pasta não encontrada» — nunca some (anti-padrão 6).
    pub status: WorkspaceMiniStatus,
    /// Índice do atalho `⌘{n}` — posição de CRIAÇÃO no registry (1-based, ≤9), NUNCA
    /// a posição visual: ⌘1..9 funciona com o M8 fechado e reordenar a lista não pode
    /// re-apontar atalho (spec §1). Fallback de varredura (fora do registry) → `None`.
    pub shortcut_index: Option<usize>,
}

/// O `events_dir` canônico de um Espaço (`<ws>/.lina/events`) — a MESMA derivação que
/// o caller usa para popular o [`MiniStatusCache`] (chaves têm de bater byte a byte).
#[must_use]
pub fn events_dir_of(ws_root: &Path) -> PathBuf {
    ws_root.join(".lina").join("events")
}

/// Mini-status de um `events_dir` SEGUNDO O CACHE. Entrada ausente (o caller não fez
/// `refresh` com este dir — violação do contrato de integração) degrada VISÍVEL para
/// linha-⚠: afirmar "sem Agentes ainda" sem dado mentiria pior que o ⚠ honesto.
fn status_of(cache: &MiniStatusCache, events_dir: &Path) -> WorkspaceMiniStatus {
    cache.get(events_dir).map_or_else(
        || WorkspaceMiniStatus {
            agents_alive: 0,
            agents_total: 0,
            dominant: None,
            cost_short: None,
            cost_tooltip: "sem dados: o mini-status deste Espaço não foi computado".to_owned(),
            unreachable: true,
        },
        |entry| entry.status.clone(),
    )
}

/// Monta as linhas do M8. **Fonte canônica = `WorkspaceRegistry`** (ordem de criação);
/// a varredura T6 entra como FALLBACK de recuperação (Espaço no disco que o ponteiro
/// perdeu) — deduplicada por raiz, sem `⌘{n}` (índice estável é privilégio do registry).
/// Arquivados saem da lista padrão (vivem sob «Espaços arquivados ▸»), mas SEGURAM a
/// posição deles no índice `⌘{n}` — arquivar o 1º não re-aponta ⌘2 em silêncio.
#[must_use]
pub fn build_rows(
    registry_entries: &[RegistryEntry],
    varredura_fallback: &[ScanEntry],
    active_root: &Path,
    cache: &MiniStatusCache,
) -> Vec<SidebarRow> {
    let mut rows: Vec<SidebarRow> = Vec::new();
    for (pos, entry) in registry_entries.iter().enumerate() {
        if entry.archived {
            continue;
        }
        let shortcut = pos + 1;
        rows.push(SidebarRow {
            name: entry.name.clone(),
            ws_root: entry.path.clone(),
            focused: entry.path == active_root,
            status: status_of(cache, &events_dir_of(&entry.path)),
            shortcut_index: (shortcut <= 9).then_some(shortcut),
        });
    }
    for scan in varredura_fallback {
        let root = dashboard::ws_root_of(&scan.events_dir);
        let known = rows.iter().any(|r| r.ws_root == root)
            || registry_entries.iter().any(|e| e.path == root);
        if known {
            continue;
        }
        rows.push(SidebarRow {
            name: scan.name.clone(),
            ws_root: root.clone(),
            focused: scan.focused || root == active_root,
            status: status_of(cache, &scan.events_dir),
            shortcut_index: None,
        });
    }
    rows
}

/// Dobra para busca: minúsculas + acentos comuns do PT removidos. Cobre o alfabeto
/// latino acentuado de nomes reais de Espaço; um acento fora da tabela degrada para o
/// próprio char minúsculo (busca segue funcionando com o acento digitado igual).
#[must_use]
pub fn search_fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// Filtro da busca do M8: substring case/acento-insensível (spec §1, client-side).
#[must_use]
pub fn matches_query(name: &str, query: &str) -> bool {
    let q = search_fold(query);
    q.is_empty() || search_fold(name).contains(&q)
}

/// **Estado headless do rail** — quem roteia teclado/cliques é o shell (contrato
/// declarado na entrega M8-T4): `⌘O`/clique → [`Self::toggle`]; imprimível/Backspace →
/// [`Self::type_char`]/[`Self::backspace`]; `↑↓` → [`Self::select_next`]/
/// [`Self::select_prev`] (roving sobre a lista FILTRADA); `Enter` → `on_switch` com
/// [`Self::selected_row`]; `Esc` → [`Self::close`] SEM trocar (spec §4).
#[derive(Debug, Default)]
pub struct SidebarState {
    /// Rail expandido (~280px) ou colapsado em ícones (~52px).
    pub expanded: bool,
    /// r5 BUG 4: o rail é o DONO do teclado? Abrir o rail NÃO move o foco (o terminal
    /// segue digitável); só clique na busca / `⌘F` / rename ligam isto — e `Esc` com o
    /// foco aqui fecha o rail, mas NUNCA rouba o Esc do terminal quando está desligado.
    pub kbd: bool,
    /// Texto corrente da busca (filtro substring, [`matches_query`]).
    pub query: String,
    /// Seleção do roving — índice na lista FILTRADA (padrão `PaletteState`).
    pub selected: usize,
    /// Linhas correntes (saída de [`build_rows`], injetada pelo shell a cada abertura).
    pub rows: Vec<SidebarRow>,
    /// Buffer do rename inline da linha selecionada (campo ADITIVO ao despacho —
    /// `on_rename(ws_root, novo)` exige coletar o `novo` em algum lugar, e o padrão
    /// do app para input inline é estado headless, como o `creating` de M3/M4).
    pub renaming: Option<String>,
}

impl SidebarState {
    /// Substitui as linhas (re-`build_rows` na abertura/refresh), preservando a
    /// seleção quando ainda válida — sem saltos de foco gratuitos.
    pub fn set_rows(&mut self, rows: Vec<SidebarRow>) {
        self.rows = rows;
        let len = self.filtered().len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    /// Abre expandido. **r5 BUG 4(e) — divergência REGISTRADA da spec §4:** a spec
    /// mandava focar a busca ao abrir (ux-flows:670); a tela do fundador provou que isso
    /// sequestra a digitação do terminal. Abrir agora NÃO move o foco (`kbd=false`);
    /// a busca foca por clique ou `⌘F`.
    pub fn open(&mut self) {
        self.expanded = true;
        self.kbd = false;
    }

    /// Fecha SEM trocar (Esc, spec §4): zera busca/seleção/rename/foco — reabrir nasce limpo.
    pub fn close(&mut self) {
        self.expanded = false;
        self.kbd = false;
        self.query.clear();
        self.selected = 0;
        self.renaming = None;
    }

    /// Clique na busca / `⌘F` com o rail aberto: o rail assume o teclado (BUG 4(b)).
    pub fn focus_search(&mut self) {
        self.kbd = true;
    }

    /// `⌘O`/clique no rail: expande ↔ colapsa.
    pub fn toggle(&mut self) {
        if self.expanded {
            self.close();
        } else {
            self.open();
        }
    }

    /// Imprimível → busca (ou buffer de rename, quando ativo). Mudar a busca reposiciona
    /// o roving no 1º resultado (padrão paleta).
    pub fn type_char(&mut self, s: &str) {
        match &mut self.renaming {
            Some(buf) => buf.push_str(s),
            None => {
                self.query.push_str(s);
                self.selected = 0;
            }
        }
    }

    /// Backspace na busca (ou no buffer de rename, quando ativo).
    pub fn backspace(&mut self) {
        match &mut self.renaming {
            Some(buf) => {
                buf.pop();
            }
            None => {
                self.query.pop();
                self.selected = 0;
            }
        }
    }

    /// Índices (em `rows`) das linhas que passam no filtro corrente, na ordem.
    #[must_use]
    pub fn filtered(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches_query(&r.name, &self.query))
            .map(|(i, _)| i)
            .collect()
    }

    /// Roving ↓ — clampado no fim da lista filtrada (sem wrap; spec §4 só pede navegar).
    pub fn select_next(&mut self) {
        let len = self.filtered().len();
        if len > 0 && self.selected + 1 < len {
            self.selected += 1;
        }
    }

    /// Roving ↑ — clampado no início.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// A linha sob o roving (o alvo do Enter → `on_switch`).
    #[must_use]
    pub fn selected_row(&self) -> Option<&SidebarRow> {
        let filtered = self.filtered();
        filtered.get(self.selected).map(|&i| &self.rows[i])
    }

    /// Entra no rename inline da linha selecionada, semeado com o nome atual
    /// (`[ Renomear ]`/atalho). Sem linha sob o roving → no-op honesto. Renomear é
    /// digitação → o rail assume o teclado (BUG 4: senão as letras iriam ao terminal).
    pub fn start_rename(&mut self) {
        if let Some(row) = self.selected_row() {
            self.renaming = Some(row.name.clone());
            self.kbd = true;
        }
    }

    /// Move o roving para a linha de `root` (clique numa ação de linha NÃO-selecionada —
    /// r5 BUG 2: com as ações visíveis em TODA linha, a ação age na linha CLICADA).
    pub fn select_root(&mut self, root: &Path) {
        if let Some(pos) = self
            .filtered()
            .iter()
            .position(|&i| self.rows[i].ws_root == root)
        {
            self.selected = pos;
        }
    }

    /// `[ Renomear ]` clicado numa linha específica: seleciona-a e abre o rename inline.
    pub fn start_rename_for(&mut self, root: &Path) {
        self.select_root(root);
        self.start_rename();
    }

    /// Confirma o rename: devolve `(ws_root, novo)` para o shell despachar em
    /// `on_rename` — o evento (`WorkspaceRenamed`) é fiação do Maestro. Nome vazio
    /// (só espaços) não confirma nada (`None`), e o modo de edição encerra.
    pub fn commit_rename(&mut self) -> Option<(PathBuf, String)> {
        let buf = self.renaming.take()?;
        let novo = buf.trim();
        if novo.is_empty() {
            return None;
        }
        let row = self.selected_row()?;
        Some((row.ws_root.clone(), novo.to_owned()))
    }

    /// Cancela o rename (Esc durante a edição): descarta o buffer, nada muda.
    pub fn cancel_rename(&mut self) {
        self.renaming = None;
    }
}

// ─────────────── células puras (a gramática da spec §1 em dados, sem gpui) ───────────────

/// A célula «{N} Agentes ●» resolvida: texto + estado do dot + tooltip da dupla.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentsCell {
    /// «{N} Agentes» / «1 Agente» / «sem Agentes ainda» / «dormindo» (copy §5).
    pub text: String,
    /// Estado do ● (cor via `color_token()` no render). `None` = célula sem dot
    /// («sem Agentes ainda» / linha-⚠).
    pub dot: Option<UiState>,
    /// Tooltip/aria da dupla contagem+dot: «{N} Agentes — {Estado}» (copy §5) —
    /// a palavra viaja JUNTO da cor, nunca só a cor. `None` = sem terminais.
    pub tooltip: Option<String>,
}

/// «{N} Agentes» com o singular congelado «1 Agente» (copy §5).
fn agents_count_label(n: usize) -> String {
    if n == 1 {
        "1 Agente".to_owned()
    } else {
        format!("{n} Agentes")
    }
}

/// Resolve a célula de Agentes a partir do mini-status (M8-T2) — TODAS as variantes
/// da copy §5: vivos+dominante, todos dormindo, sem Agentes, todos-Encerrado (a 6ª
/// palavra, emenda §6-A3) e a linha-⚠.
#[must_use]
pub fn agents_cell(status: &WorkspaceMiniStatus) -> AgentsCell {
    if status.unreachable {
        // Linha-⚠: a célula não afirma contagem nenhuma (anti-padrão 6).
        return AgentsCell {
            text: COPY_M8_FOLDER_MISSING.to_owned(),
            dot: None,
            tooltip: None,
        };
    }
    if status.agents_total == 0 {
        return AgentsCell {
            text: COPY_M8_NO_AGENTS.to_owned(),
            dot: None,
            tooltip: None,
        };
    }
    if status.agents_alive == 0 {
        // Todos-Encerrado: «{N} Agentes» com N = TOTAL + ● cinza-escuro — "sem
        // Agentes ainda" mentiria (houve time). Spec §1 + emenda §6-A3.
        let n = status.agents_total;
        return AgentsCell {
            text: agents_count_label(n),
            dot: Some(UiState::Encerrado),
            tooltip: Some(format!(
                "{} — {}",
                agents_count_label(n),
                UiState::Encerrado.label()
            )),
        };
    }
    let n = status.agents_alive;
    let dominant = status.dominant.unwrap_or(UiState::Ocioso);
    // `dominant == Dormindo` com vivos ⇒ TODOS os vivos dormem (Dormindo só perde de
    // Encerrado no ranking pior-primeiro) — a dupla vira a palavra «dormindo» (copy §5).
    let text = if dominant == UiState::Dormindo {
        COPY_M8_ALL_SLEEPING.to_owned()
    } else {
        agents_count_label(n)
    };
    AgentsCell {
        text,
        dot: Some(dominant),
        tooltip: Some(format!("{} — {}", agents_count_label(n), dominant.label())),
    }
}

/// A célula de custo resolvida: «—» sem dado (NUNCA «0,00» — honestidade do medidor)
/// ou a forma curta real do produto (variante (a) da pendência de moeda, §6-A1).
#[derive(Debug, Clone, PartialEq)]
pub struct CostCell {
    /// «~US$ 0,01» ou «—».
    pub text: String,
    /// Tooltip honesto: com dado, o congelado «gasto estimado de hoje neste Espaço» +
    /// o texto integral da `CostLine` (inclui «(estimado)»/parcela de fora); sem dado,
    /// SÓ o fallback honesto do mini-status (sem prometer "gasto" inexistente).
    pub tooltip: String,
}

/// Resolve a célula de custo a partir do mini-status.
#[must_use]
pub fn cost_cell(status: &WorkspaceMiniStatus) -> CostCell {
    match &status.cost_short {
        Some(short) => CostCell {
            text: short.clone(),
            tooltip: format!("{COPY_M8_COST_TOOLTIP} — {}", status.cost_tooltip),
        },
        None => CostCell {
            text: "—".to_owned(),
            tooltip: status.cost_tooltip.clone(),
        },
    }
}

/// aria-label da linha (checklist §4): concatenação dos fragmentos CONGELADOS com
/// separador tipográfico — «{nome} · {N} Agentes — {Estado} · gasto estimado de hoje
/// neste Espaço: {custo}». Sem custo, o fragmento é OMITIDO (não lido como "zero");
/// o fragmento de Pedidos fica fora ATÉ a costura §6-C1 (sem dado por Espaço, o leitor
/// não ouve nem sino nem "em dia"). Linha-⚠ lê «{nome} · pasta não encontrada».
#[must_use]
pub fn row_aria_label(row: &SidebarRow) -> String {
    if row.status.unreachable {
        return format!("{} · {}", row.name, COPY_M8_FOLDER_MISSING);
    }
    let cell = agents_cell(&row.status);
    let mut parts = vec![row.name.clone(), cell.tooltip.unwrap_or(cell.text)];
    if let Some(short) = &row.status.cost_short {
        parts.push(format!("{COPY_M8_COST_TOOLTIP}: {short}"));
    }
    parts.join(" · ")
}

/// aria-label do item de criação: bloqueado (free no limite) lê «Novo Espaço — PRO»
/// (checklist §4 — concatenação dos dois rótulos visíveis, nada novo).
#[must_use]
pub fn create_item_aria(create_blocked: bool) -> String {
    if create_blocked {
        "Novo Espaço — PRO".to_owned()
    } else {
        COPY_M8_NEW_SPACE.to_owned()
    }
}

// ════════════════════════════ camada 2 · render gpui (casca fina) ════════════════════════════

/// Largura do rail colapsado (ícones ▣ por Espaço).
pub const SIDEBAR_COLLAPSED_W: f32 = 52.0;
/// Largura do rail expandido (busca + linhas da gramática §1).
pub const SIDEBAR_EXPANDED_W: f32 = 280.0;

/// Callback de linha: recebe a raiz do Espaço alvo.
pub type RowCallback =
    Rc<dyn Fn(&mut WorkspaceView, &Path, &mut Window, &mut Context<WorkspaceView>)>;
/// Callback sem alvo (criar / toggle / abrir arquivados).
pub type PlainCallback = Rc<dyn Fn(&mut WorkspaceView, &mut Window, &mut Context<WorkspaceView>)>;
/// Callback do rename confirmado: raiz + nome novo.
pub type RenameCallback =
    Rc<dyn Fn(&mut WorkspaceView, &Path, &str, &mut Window, &mut Context<WorkspaceView>)>;

/// **O componente do rail** — estado headless + callbacks injetados. A fiação real
/// (trocar Espaço, criar, renomear, arquivar, montar no shell) é do Maestro na
/// integração: aqui os cliques SÓ despacham os closures.
pub struct Sidebar {
    pub state: SidebarState,
    /// Vitrine free=1 (copy §1a): selo «PRO» nas affordances de criação — o item
    /// continua clicável (vitrine, não beco); quem decide é o caller (seam §3).
    pub create_blocked: bool,
    /// Enter/clique numa linha → trocar para o Espaço (raiz). NÃO implementado aqui.
    pub on_switch: RowCallback,
    /// `+ Novo Espaço…` / `[ + Criar “{texto}” ]` → abrir o M9.
    pub on_create: PlainCallback,
    /// Rename CONFIRMADO (raiz, nome novo) → `WorkspaceRenamed` é fiação do Maestro.
    pub on_rename: RenameCallback,
    /// `[ Arquivar ]` na linha → `WorkspaceArchived` + toast são fiação do Maestro.
    pub on_archive: RowCallback,
    /// Clique no cabeçalho do rail (expande ↔ colapsa) — ADITIVO ao despacho: sem
    /// ele o mouse não abre o rail (o ⌘O do teclado é fiação do shell).
    pub on_toggle: PlainCallback,
    /// «Espaços arquivados ▸» — a vista de arquivados é de outra fatia; o link
    /// despacha pro shell decidir (ADITIVO, mesmo racional do `on_toggle`).
    pub on_archived: PlainCallback,
    /// `[ Renomear ]` por mouse — o shell normalmente chama `state.start_rename()`
    /// (ADITIVO: clique precisa de UM despacho, e o componente não se auto-muta).
    pub on_rename_begin: RowCallback,
    /// r5 (unload-ui): `[ Descarregar ]` numa linha de Espaço de FUNDO — o MECANISMO
    /// (estacionar PTYs ociosos via `lina_core::suspend::unload_plan`) é do seam do
    /// Core A2A; aqui só o render. **`None` = seam ainda não plugado ⇒ a ação NÃO
    /// aparece** (botão que não faz nada é tela que mente). Setado via [`Self::set_on_unload`].
    pub on_unload: Option<RowCallback>,
    /// F2-2-4 — clique na PORTA VISÍVEL da paleta de comandos (abre o que o ⌘K abre). Mesmo racional
    /// `Option` do `on_unload`: **`None` ⇒ o botão NÃO aparece** (sem fiação, não há porta). Setado
    /// via [`Self::set_on_open_palette`].
    pub on_open_palette: Option<PlainCallback>,
}

/// Cor do ● a partir do token SEMÂNTICO do estado (mesmo mapa do dashboard no shell —
/// `cinza`/`cinza-escuro` caem no muted; nenhum rgb literal aqui, lint F1-2-1).
fn dot_color(state: UiState, th: &Theme) -> u32 {
    match state.color_token() {
        "verde" => th.state.success,
        "ambar" => th.state.warning,
        "vermelho-suave" => th.state.danger,
        "azul-escuro" => th.accent.primary,
        _ => th.text.muted,
    }
}

impl Sidebar {
    /// Monta o componente com os callbacks da integração (a ordem espelha o despacho).
    #[must_use]
    #[allow(clippy::too_many_arguments)] // o contrato do despacho são 4 callbacks nomeados + 3 estruturais — agrupar em struct esconderia o contrato da integração
    pub fn new(
        on_switch: RowCallback,
        on_create: PlainCallback,
        on_rename: RenameCallback,
        on_archive: RowCallback,
        on_toggle: PlainCallback,
        on_archived: PlainCallback,
        on_rename_begin: RowCallback,
    ) -> Self {
        Self {
            state: SidebarState::default(),
            create_blocked: false,
            on_switch,
            on_create,
            on_rename,
            on_archive,
            on_toggle,
            on_archived,
            on_rename_begin,
            on_unload: None,
            on_open_palette: None,
        }
    }

    /// Pluga o seam do Descarregar (Core A2A) — a partir daqui a ação aparece nas
    /// linhas de Espaço de fundo. Separado do `new` de propósito: a UI nasceu antes
    /// do mecanismo e o botão só existe quando há executor de verdade.
    pub fn set_on_unload(&mut self, cb: RowCallback) {
        self.on_unload = Some(cb);
    }

    /// F2-2-4 — pluga a PORTA VISÍVEL da paleta. A partir daqui o botão "Buscar comandos · ⌘K"
    /// aparece no rail (colapsado e expandido) e abre a paleta ao clique. Mesmo padrão do
    /// `set_on_unload`: a UI só mostra a porta quando há quem a abra.
    pub fn set_on_open_palette(&mut self, cb: PlainCallback) {
        self.on_open_palette = Some(cb);
    }

    /// O botão da porta da paleta, ou nada se o seam não está plugado. `compact` = forma de ÍCONE
    /// (rail colapsado, 52px — só a lupa, com a palavra no aria); senão a PÍLULA rotulada
    /// ("🔍 Buscar comandos · ⌘K"). Os dois disparam o mesmo `on_open_palette`.
    fn palette_button(
        &self,
        th: &Theme,
        cx: &mut Context<WorkspaceView>,
        compact: bool,
    ) -> Option<AnyElement> {
        let open = self.on_open_palette.clone()?;
        let base = div()
            .id("sb-palette-open")
            .rounded_md()
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(COPY_PALETTE_ARIA)
            .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| open(v, w, cx)));
        let btn = if compact {
            base.flex()
                .flex_col()
                .items_center()
                .gap_1()
                .px_1()
                .py_1()
                .text_color(rgb(th.text.secondary))
                .child(
                    div()
                        .text_size(px(f32::from(th.typography.size.subtitle)))
                        .child(text!("🔍")),
                )
                .child(
                    // A palavra viaja mesmo no rail estreito — porta ROTULADA, não só ícone.
                    div()
                        .text_size(px(f32::from(th.typography.size.caption)))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_PALETTE_BUTTON
                            .split(' ')
                            .next()
                            .unwrap_or("Buscar"))),
                )
        } else {
            base.flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .bg(rgb(th.surface.raised))
                .text_color(rgb(th.text.primary))
                .text_size(px(f32::from(th.typography.size.body)))
                .child(text!(format!("🔍  {COPY_PALETTE_BUTTON}")))
                .child(
                    // Atalho à direita, esmaecido (Notion/Zed: a porta ENSINA o atalho).
                    div()
                        .flex_1()
                        .flex()
                        .justify_end()
                        .text_size(px(f32::from(th.typography.size.small)))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_PALETTE_SHORTCUT)),
                )
        };
        Some(btn.into_any_element())
    }

    /// Render do rail — colapsado (ícones) ou expandido (busca + linhas + rodapé).
    pub fn render(&self, th: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
        if self.state.expanded {
            self.render_expanded(th, cx)
        } else {
            self.render_collapsed(th, cx)
        }
    }

    /// Rail colapsado: cabeçalho ▣ (expande) + um ícone por Espaço (clique troca) +
    /// `+` no pé (criar). Cada ícone carrega o aria com o NOME (cor nunca sozinha).
    fn render_collapsed(&self, th: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
        let toggle = self.on_toggle.clone();
        let mut rail = div()
            .id("sb-rail")
            .w(px(SIDEBAR_COLLAPSED_W))
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .p_2()
            .bg(rgb(th.surface.chrome))
            .border_color(rgb(th.surface.border))
            .child(
                div()
                    .id("sb-toggle")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label("Espaços — abrir a lista")
                    .text_color(rgb(th.text.primary))
                    .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| toggle(v, w, cx)))
                    .child(text!("▣")),
            );
        // F2-2-4: a porta da paleta vive no TOPO do rail — sempre visível, mesmo colapsado.
        if let Some(btn) = self.palette_button(th, cx, true) {
            rail = rail.child(btn);
        }
        for (idx, row) in self.state.rows.iter().enumerate() {
            let switch = self.on_switch.clone();
            let root = row.ws_root.clone();
            let warn = row.status.unreachable;
            let mut icon = div()
                .id(("sb-rail-row", idx))
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(row_aria_label(row))
                .text_color(rgb(if warn {
                    th.state.warning
                } else {
                    th.text.secondary
                }))
                .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| switch(v, &root, w, cx)))
                .child(text!(if warn { "⚠" } else { "▣" }));
            if row.focused {
                icon = icon.border_2().border_color(rgb(th.focus.ring));
            }
            rail = rail.child(icon);
        }
        let create = self.on_create.clone();
        rail = rail.child(
            div()
                .id("sb-rail-create")
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(create_item_aria(self.create_blocked))
                .text_color(rgb(th.text.muted))
                .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| create(v, w, cx)))
                .child(text!("+")),
        );
        rail.into_any_element()
    }

    /// Rail expandido: busca no topo, linhas (gramática §1), rodapé com arquivados +
    /// criar. Linha FOCADA (Espaço ativo) leva `focus.ring`; a linha sob o roving leva
    /// fundo `raised` — dois sinais distintos, nunca só cor (a palavra viaja no aria).
    fn render_expanded(&self, th: &Theme, cx: &mut Context<WorkspaceView>) -> AnyElement {
        let toggle = self.on_toggle.clone();
        let query_view = if self.state.query.is_empty() {
            COPY_M8_SEARCH_PLACEHOLDER.to_owned()
        } else {
            self.state.query.clone()
        };
        let query_color = if self.state.query.is_empty() {
            th.text.muted
        } else {
            th.text.primary
        };
        // r5 BUG 4: caret visível SÓ quando a busca é dona do teclado — o estado de foco
        // nunca é só cor (anel + caret).
        let search_focused = self.state.kbd && self.state.renaming.is_none();
        let query_view = if search_focused && !self.state.query.is_empty() {
            format!("{query_view}▏")
        } else {
            query_view
        };
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                // r5 BUG 1: affordance de FECHAR por mouse — chevron explícito (o ▣
                // antigo não comunicava ação; o fundador só fechava com Esc).
                div()
                    .id("sb-collapse")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label("Recolher a lista de Espaços (⌘O)")
                    .text_color(rgb(th.text.primary))
                    .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| toggle(v, w, cx)))
                    .child(text!("◀")),
            )
            .child(
                // Campo de busca: o TEXTO é estado headless (`query`); o roteamento de
                // teclado é do shell. r5 BUG 4(b): a busca só captura teclado quando
                // explicitamente focada — clique aqui ou ⌘F.
                {
                    let mut search = div()
                        .id("sb-search")
                        .flex_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(rgb(th.surface.raised))
                        .aria_label("buscar Espaços (⌘F)")
                        .text_color(rgb(query_color))
                        .text_size(px(12.0))
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.sidebar.state.focus_search();
                            cx.notify();
                        }))
                        .child(text!(query_view));
                    if search_focused {
                        search = search.border_2().border_color(rgb(th.focus.ring));
                    }
                    search
                },
            );

        let filtered = self.state.filtered();
        let mut list = div().id("sb-list").flex_1().overflow_y_scroll().child(
            div().flex().flex_col().gap_1().children(
                filtered
                    .iter()
                    .enumerate()
                    .map(|(visible_idx, &row_idx)| self.render_row(visible_idx, row_idx, th, cx)),
            ),
        );
        if filtered.is_empty() {
            // Busca sem resultado: o vazio explica e oferece a criação (copy §5) —
            // a vitrine «PRO» aparece ANTES do clique (item 1a).
            let create = self.on_create.clone();
            let label = create_from_search_label(&self.state.query);
            let mut create_btn = div()
                .id("sb-create-search")
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .role(Role::Button)
                .aria_label(if self.create_blocked {
                    format!("{label} — {COPY_M8_PRO_BADGE}")
                } else {
                    label.clone()
                })
                .text_color(rgb(th.accent.primary))
                .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| create(v, w, cx)))
                .child(text!(label));
            if self.create_blocked {
                create_btn = create_btn.child(
                    div()
                        .text_size(px(9.0))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_M8_PRO_BADGE)),
                );
            }
            list = div()
                .id("sb-list")
                .flex_1()
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_M8_SEARCH_EMPTY)),
                )
                .child(create_btn);
        }

        let archived = self.on_archived.clone();
        let create = self.on_create.clone();
        let mut create_item = div()
            .id("sb-create")
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(create_item_aria(self.create_blocked))
            .text_color(rgb(th.text.primary))
            .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| create(v, w, cx)))
            .child(text!(COPY_M8_NEW_SPACE));
        if self.create_blocked {
            create_item = create_item.child(
                div()
                    .text_size(px(9.0))
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_M8_PRO_BADGE)),
            );
        }
        let footer = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("sb-archived")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .role(Role::Button)
                    .aria_label(COPY_M8_ARCHIVED_LINK)
                    .text_color(rgb(th.text.muted))
                    .text_size(px(11.0))
                    .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| archived(v, w, cx)))
                    .child(text!(COPY_M8_ARCHIVED_LINK)),
            )
            .child(create_item);

        div()
            .id("sb-panel")
            .w(px(SIDEBAR_EXPANDED_W))
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(rgb(th.surface.panel))
            .border_color(rgb(th.surface.border))
            .text_size(px(12.0))
            .line_height(px(18.0))
            .child(header)
            // F2-2-4: porta ROTULADA da paleta de comandos (≠ busca de Espaços acima). Logo no topo,
            // modelo Notion search-first: o leigo clica e descobre o que o Lina faz.
            .children(self.palette_button(th, cx, false))
            .child(list)
            .child(footer)
            .into_any_element()
    }

    /// Uma linha expandida: `▣ {nome}  {célula Agentes}●  {custo}  ⌘{n}` + ações
    /// `[ Renomear ]`/`[ Arquivar ]` na linha sob o roving. Linha-⚠ troca a gramática
    /// pelos fragmentos honestos (nome + «pasta não encontrada») — nunca some.
    fn render_row(
        &self,
        visible_idx: usize,
        row_idx: usize,
        th: &Theme,
        cx: &mut Context<WorkspaceView>,
    ) -> AnyElement {
        let row = &self.state.rows[row_idx];
        let is_roving = visible_idx == self.state.selected;
        let renaming = is_roving.then(|| self.state.renaming.clone()).flatten();
        let cell = agents_cell(&row.status);
        let cost = cost_cell(&row.status);
        let switch = self.on_switch.clone();
        let root = row.ws_root.clone();

        // O nome — ou o buffer do rename inline com um caret honesto (padrão M3/M4).
        let name_view = renaming
            .as_ref()
            .map_or_else(|| row.name.clone(), |buf| format!("{buf}▏"));

        let can_unload = row_can_unload(row.focused, self.on_unload.is_some());

        // ── LINHA PRIMÁRIA: ícone · NOME (cede) · meta (agentes/custo/⌘n) ──
        // Doutrina B3 (espelhada de `agent_modal`): só o NOME cede — `flex_1 + min_w(0) +
        // truncate` (o `…` é a affordance VISÍVEL de "há mais"; o nome inteiro viaja no
        // tooltip de hover, descoberto pelo leigo de mouse). Ícone e meta são `flex_shrink_0`
        // e NUNCA somem. Antes (bf-sidebar): `flex_1 + overflow_hidden` sem `min_w(0)` no nome
        // e sem `flex_shrink_0` nas células → o nome herdava min-content gigante e EXPULSAVA as
        // ações para fora dos 280px (Arquivar sumia; nome cortado mudo).
        let icon = div()
            .flex_shrink_0()
            .text_color(rgb(if row.status.unreachable {
                th.state.warning
            } else {
                th.text.secondary
            }))
            .child(text!(if row.status.unreachable { "⚠" } else { "▣" }));

        let name_cell = if renaming.is_some() {
            // Editando: o caret ▏ vive no FIM — trunca pelo INÍCIO p/ o cursor ficar visível.
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis_start()
                .text_color(rgb(th.text.primary))
                .child(text!(name_view))
                .into_any_element()
        } else {
            let full = row.name.clone();
            div()
                .id(("sb-name", row_idx))
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_color(rgb(th.text.primary))
                .tooltip(move |_w, cx| cx.new(|_| NameTooltip(full.clone())).into())
                .child(text!(name_view))
                .into_any_element()
        };

        let mut primary = div()
            .flex()
            .items_center()
            .gap_2()
            .child(icon)
            .child(name_cell);

        // Célula Agentes + ● (tooltip honesto: a palavra viaja no aria da célula).
        let mut agents = div()
            .id(("sb-agents", row_idx))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .text_size(px(11.0))
            .text_color(rgb(th.text.secondary))
            .child(text!(cell.text.clone()));
        if let Some(tip) = &cell.tooltip {
            agents = agents.aria_label(tip.clone());
        }
        if let Some(state) = cell.dot {
            agents = agents.child(
                div()
                    .text_color(rgb(dot_color(state, th)))
                    .child(text!("●")),
            );
        }
        primary = primary.child(agents);

        // Célula de custo: «—» sem dado (nunca «0,00»); tooltip honesto no aria.
        if !row.status.unreachable {
            let has_cost = row.status.cost_short.is_some();
            primary = primary.child(
                div()
                    .id(("sb-cost", row_idx))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(rgb(if has_cost {
                        th.text.primary
                    } else {
                        th.text.muted
                    }))
                    .aria_label(cost.tooltip.clone())
                    .child(text!(cost.text.clone())),
            );
        }

        // ⌘{n}: índice ESTÁVEL do registry — só existe para Espaços registrados.
        if let Some(n) = row.shortcut_index {
            primary = primary.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(rgb(th.text.muted))
                    .child(text!(format!("⌘{n}"))),
            );
        }

        // ── A LINHA é uma COLUNA: primária + faixa de ações própria ──
        // Em 280px, nome+meta+3 ações NÃO cabem numa linha só (a causa do bug). As ações
        // ganham faixa própria SEMPRE-VISÍVEL (decisão r5 mantida; descobríveis, rotuladas,
        // nunca cortadas). `flex_wrap` é a rede: rótulo futuro maior quebra a linha, não clipa.
        let mut row_el = div()
            .id(("sb-row", row_idx))
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .role(Role::Button)
            .aria_label(row_aria_label(row))
            .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| switch(v, &root, w, cx)))
            .child(primary);

        // r5 BUG 2 + bf-sidebar: ações VISÍVEIS em TODA linha, agora na própria faixa (antes
        // vazavam para fora e Arquivar sumia). `row_actions` é a fonte ÚNICA da presença (teste
        // guardião abaixo); o aria do Arquivar carrega o subtexto honesto; o destino é a linha
        // CLICADA. Cada botão é `flex_shrink_0` — nenhum encolhe ao ponto de virar ilegível.
        if renaming.is_none() {
            let mut actions = div().flex().flex_wrap().items_center().gap_3();
            for action in row_actions(false, can_unload) {
                let btn = match action {
                    RowAction::Rename => {
                        let begin = self.on_rename_begin.clone();
                        let root_rn = row.ws_root.clone();
                        Some(
                            div()
                                .id(("sb-rename", row_idx))
                                .flex_shrink_0()
                                .px_1()
                                .rounded_md()
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_label(COPY_M8_RENAME)
                                .text_size(px(10.0))
                                .text_color(rgb(th.text.muted))
                                .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| {
                                    begin(v, &root_rn, w, cx)
                                }))
                                .child(text!(COPY_M8_RENAME)),
                        )
                    }
                    RowAction::Archive => {
                        let archive = self.on_archive.clone();
                        let root_ar = row.ws_root.clone();
                        Some(
                            div()
                                .id(("sb-archive", row_idx))
                                .flex_shrink_0()
                                .px_1()
                                .rounded_md()
                                .cursor_pointer()
                                .role(Role::Button)
                                .aria_label(format!("{COPY_M8_ARCHIVE} — {COPY_M8_ARCHIVE_HINT}"))
                                .text_size(px(10.0))
                                .text_color(rgb(th.text.muted))
                                .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| {
                                    archive(v, &root_ar, w, cx)
                                }))
                                .child(text!(COPY_M8_ARCHIVE)),
                        )
                    }
                    // `row_actions` só devolve Unload quando `can_unload` (⇒ o seam está
                    // plugado e `on_unload` é Some); o `map` honra isso sem unwrap.
                    RowAction::Unload => self.on_unload.clone().map(|unload| {
                        let root_un = row.ws_root.clone();
                        div()
                            .id(("sb-unload", row_idx))
                            .flex_shrink_0()
                            .px_1()
                            .rounded_md()
                            .cursor_pointer()
                            .role(Role::Button)
                            .aria_label(format!("{COPY_M8_UNLOAD} — {COPY_M8_UNLOAD_HINT}"))
                            .text_size(px(10.0))
                            .text_color(rgb(th.text.muted))
                            .on_click(cx.listener(move |v, _ev: &ClickEvent, w, cx| {
                                unload(v, &root_un, w, cx)
                            }))
                            .child(text!(COPY_M8_UNLOAD))
                    }),
                };
                if let Some(btn) = btn {
                    actions = actions.child(btn);
                }
            }
            row_el = row_el.child(actions);

            // Subtexto leigo na linha sob o roving (onde o olho está ao navegar por teclado)
            // — nas demais o aria/tooltip carrega a explicação. Faixa própria: nunca compete.
            if is_roving {
                row_el = row_el.child(
                    div()
                        .text_size(px(9.0))
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_M8_ARCHIVE_HINT)),
                );
            }
        }

        if is_roving {
            row_el = row_el.bg(rgb(th.surface.raised));
        }
        if row.focused {
            row_el = row_el.border_2().border_color(rgb(th.focus.ring));
        }
        row_el.into_any_element()
    }
}

// ─────────────── ações de linha: presença pura (testável) + tooltip de nome ───────────────

/// Uma ação de linha do Espaço. A ENUM é a fonte única da presença — o render itera sobre
/// [`row_actions`] e despacha o callback certo; o teste guardião abaixo prova o invariante do
/// bug `bf-sidebar`: **Arquivar NUNCA some** fora do rename.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowAction {
    Rename,
    Archive,
    Unload,
}

/// Quais ações a linha mostra. Renomeando → nenhuma (o campo vira editor). Caso contrário,
/// Renomear+Arquivar SEMPRE; Descarregar só quando o seam permite (`can_unload`, regra r5).
/// Fonte única consumida pelo render — remover `Archive` daqui some-o da UI E quebra o teste.
#[must_use]
pub fn row_actions(renaming: bool, can_unload: bool) -> Vec<RowAction> {
    if renaming {
        return Vec::new();
    }
    let mut actions = vec![RowAction::Rename, RowAction::Archive];
    if can_unload {
        actions.push(RowAction::Unload);
    }
    actions
}

/// Tooltip de hover do nome do Espaço: entrega o nome COMPLETO quando a linha o trunca. O
/// `…` (de `truncate()`) já sinaliza "há mais"; este card revela o texto inteiro sem cortar —
/// a affordance que o leigo de mouse precisa para não perder informação em silêncio.
pub struct NameTooltip(pub String);

impl Render for NameTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let th = crate::theme::active();
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(th.surface.raised))
            .border_1()
            .border_color(rgb(th.surface.border))
            .text_size(px(f32::from(th.typography.size.body)))
            .text_color(rgb(th.text.primary))
            .child(text!(self.0.clone()))
    }
}

// ════════════════════════════ testes (camada headless, sem gpui) ════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use lina_core::DomainEvent;
    use uuid::Uuid;

    // ── fixtures: Espaço REAL em temp dir (mesmo padrão do M8-T2 em dashboard.rs) ──

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!(
                "lina-sb-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&p).expect("tempdir");
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

    /// Espaço real em disco (store + N terminais com status); devolve o `events_dir`.
    fn seed_space(root: &Path, name: &str, statuses: &[&str]) -> PathBuf {
        let events = events_dir_of(root);
        let mut store = lina_core::EventStore::open(&events).expect("open");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: name.into(),
                focus_preset: String::new(),
            })
            .expect("ws");
        for status in statuses {
            let node = Uuid::now_v7();
            store
                .append(&DomainEvent::NodeAdded {
                    node,
                    kind: "Terminal".into(),
                    x: 0.0,
                    y: 0.0,
                    requested_by: None,
                })
                .expect("node");
            store
                .append(&DomainEvent::NodeStatusChanged {
                    node,
                    status: (*status).into(),
                    from: String::new(),
                    reason: String::new(),
                })
                .expect("status");
        }
        events
    }

    fn registry_entry(name: &str, path: &Path, archived: bool) -> RegistryEntry {
        RegistryEntry {
            id: format!("id-{name}"),
            name: name.to_owned(),
            path: path.to_path_buf(),
            last_focus: 0,
            archived,
        }
    }

    /// Mini-status sintético (campos públicos) para as células puras.
    fn ms(alive: usize, total: usize, dominant: Option<UiState>) -> WorkspaceMiniStatus {
        WorkspaceMiniStatus {
            agents_alive: alive,
            agents_total: total,
            dominant,
            cost_short: None,
            cost_tooltip: "sem sessão hoje".to_owned(),
            unreachable: false,
        }
    }

    fn row_with(status: WorkspaceMiniStatus) -> SidebarRow {
        SidebarRow {
            name: "Projeto X".to_owned(),
            ws_root: PathBuf::from("/tmp/x"),
            focused: false,
            status,
            shortcut_index: Some(1),
        }
    }

    const TODAY: &str = "2026-06-11";

    // ── build_rows: canônico + fallback + ⚠ ──

    /// Registry é a fonte canônica (ordem de criação); a varredura entra como fallback
    /// deduplicado; Espaço sem store vira linha-⚠ e NUNCA some (anti-padrão 6).
    #[test]
    fn build_rows_canonico_fallback_e_linha_warn() {
        let a = TempDir::new("a");
        let c = TempDir::new("c");
        let d = TempDir::new("d");
        let ev_a = seed_space(a.path(), "Alfa", &["Busy"]);
        let ev_c = seed_space(c.path(), "Gama", &[]);
        // D existe só na varredura E sem store legível (pasta vazia) → ⚠.
        let ev_d = events_dir_of(d.path());

        let registry = vec![
            registry_entry("Alfa", a.path(), false),
            registry_entry("Beta (arquivado)", Path::new("/tmp/lina-sb-beta"), true),
            registry_entry("Gama", c.path(), false),
        ];
        let scan = vec![
            // Duplicata do canônico (a varredura também enxerga o Alfa) → deduplicada.
            ScanEntry {
                name: "Alfa".into(),
                events_dir: ev_a.clone(),
                focused: false,
                nodes: 1,
            },
            ScanEntry {
                name: "Delta (fora do registry)".into(),
                events_dir: ev_d.clone(),
                focused: false,
                nodes: 0,
            },
        ];
        let mut cache = MiniStatusCache::default();
        cache.refresh(&[ev_a, ev_c, ev_d], &[], TODAY, 1);

        let rows = build_rows(&registry, &scan, a.path(), &cache);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Alfa", "Gama", "Delta (fora do registry)"],
            "canônico na ordem do registry + fallback no fim; arquivado fora da lista padrão; sem duplicata"
        );
        assert!(rows[0].focused, "active_root marca o focado");
        assert!(!rows[0].status.unreachable);
        assert_eq!(rows[0].status.agents_alive, 1, "mini-status real do store");
        assert!(
            rows[2].status.unreachable,
            "Espaço que não projeta vira linha-⚠, nunca some"
        );
        assert_eq!(rows[2].shortcut_index, None, "fallback não ganha ⌘n");
    }

    /// Contrato de integração violado (cache sem a entrada) degrada VISÍVEL: linha-⚠,
    /// nunca um "sem Agentes ainda" inventado.
    #[test]
    fn build_rows_cache_sem_entrada_degrada_para_warn() {
        let registry = vec![registry_entry(
            "Solto",
            Path::new("/tmp/lina-sb-solto"),
            false,
        )];
        let rows = build_rows(
            &registry,
            &[],
            Path::new("/tmp/outro"),
            &MiniStatusCache::default(),
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].status.unreachable);
        assert!(!rows[0].focused);
    }

    // ── índice ⌘n estável ──

    /// O `⌘{n}` segue a posição de CRIAÇÃO no registry: arquivado SEGURA a posição
    /// (⌘2 não re-aponta), reordenar a lista visual não troca os rótulos, e >9 → None.
    #[test]
    fn indice_cmd_n_estavel_com_lista_reordenada_e_arquivado_segura_posicao() {
        let mut registry = vec![
            registry_entry("Primeiro", Path::new("/tmp/sb-1"), false),
            registry_entry("Segundo (arquivado)", Path::new("/tmp/sb-2"), true),
            registry_entry("Terceiro", Path::new("/tmp/sb-3"), false),
        ];
        for i in 4..=11 {
            registry.push(registry_entry(
                &format!("Espaço {i}"),
                Path::new(&format!("/tmp/sb-{i}")),
                false,
            ));
        }
        let cache = MiniStatusCache::default();
        let mut rows = build_rows(&registry, &[], Path::new("/tmp/sb-3"), &cache);

        let by_name = |rows: &[SidebarRow], n: &str| {
            rows.iter()
                .find(|r| r.name == n)
                .expect("linha")
                .shortcut_index
        };
        assert_eq!(by_name(&rows, "Primeiro"), Some(1));
        assert_eq!(
            by_name(&rows, "Terceiro"),
            Some(3),
            "o arquivado segura a posição 2 — ⌘3 nunca re-aponta em silêncio"
        );
        assert_eq!(by_name(&rows, "Espaço 9"), Some(9));
        assert_eq!(by_name(&rows, "Espaço 10"), None, "atalhos param no ⌘9");

        // Reordenação VISUAL (focado-primeiro): os rótulos viajam com as linhas.
        rows.sort_by_key(|r| !r.focused);
        assert_eq!(rows[0].name, "Terceiro");
        assert_eq!(
            rows[0].shortcut_index,
            Some(3),
            "índice não vira posição visual"
        );
    }

    // ── filtro de busca ──

    /// Substring case/acento-insensível (spec §1) — função pura.
    #[test]
    fn filtro_substring_case_e_acento_insensivel() {
        assert!(matches_query("São Paulo", "sao"));
        assert!(matches_query("São Paulo", "SAO PA"));
        assert!(matches_query("Café", "CAFE"));
        assert!(matches_query("Estúdio", "tud"));
        assert!(matches_query("Atelier", "ateliê".trim_end_matches('ê')));
        assert!(!matches_query("Loja", "lojas"));
        assert!(matches_query("Qualquer", ""), "busca vazia não filtra nada");

        let mut st = SidebarState::default();
        st.set_rows(vec![
            row_with(ms(0, 0, None)),
            SidebarRow {
                name: "São Paulo".into(),
                ..row_with(ms(0, 0, None))
            },
        ]);
        st.selected = 1;
        st.type_char("sao");
        assert_eq!(st.filtered().len(), 1);
        assert_eq!(
            st.selected, 0,
            "mudar a busca reposiciona o roving no 1º resultado"
        );
        assert_eq!(st.selected_row().expect("linha").name, "São Paulo");
    }

    // ── dominância exibida (célula Agentes) ──

    /// A célula exibe a dominância VINDA do mini-status (M8-T2): todas as variantes da
    /// copy §5 — plural/singular, todos-Encerrado, «sem Agentes ainda» e «dormindo».
    #[test]
    fn dominancia_exibida_vem_do_mini_status() {
        // Vivos mistos: dominante pior-primeiro → palavra + cor juntas no tooltip.
        let cell = agents_cell(&ms(2, 3, Some(UiState::EsperandoVoce)));
        assert_eq!(cell.text, "2 Agentes");
        assert_eq!(cell.dot, Some(UiState::EsperandoVoce));
        assert_eq!(cell.tooltip.as_deref(), Some("2 Agentes — Esperando você"));

        // Singular congelado.
        let cell = agents_cell(&ms(1, 1, Some(UiState::Trabalhando)));
        assert_eq!(cell.text, "1 Agente");

        // Todos-Encerrado: N = TOTAL + ● cinza-escuro («sem Agentes ainda» mentiria).
        let cell = agents_cell(&ms(0, 2, Some(UiState::Encerrado)));
        assert_eq!(cell.text, "2 Agentes");
        assert_eq!(cell.dot, Some(UiState::Encerrado));
        assert_eq!(cell.tooltip.as_deref(), Some("2 Agentes — Encerrado"));

        // Sem terminais: «sem Agentes ainda», sem dot (nada a colorir).
        let cell = agents_cell(&ms(0, 0, None));
        assert_eq!(cell.text, COPY_M8_NO_AGENTS);
        assert_eq!(cell.dot, None);

        // Todos os vivos dormindo: a dupla vira a palavra «dormindo» (caixa baixa).
        let cell = agents_cell(&ms(2, 2, Some(UiState::Dormindo)));
        assert_eq!(cell.text, COPY_M8_ALL_SLEEPING);
        assert_eq!(cell.dot, Some(UiState::Dormindo));

        // Linha-⚠: nenhuma contagem afirmada.
        let mut un = ms(0, 0, None);
        un.unreachable = true;
        let cell = agents_cell(&un);
        assert_eq!(cell.text, COPY_M8_FOLDER_MISSING);
        assert_eq!(cell.dot, None);
    }

    // ── custo honesto ──

    /// Sem dado → «—» + fallback honesto no tooltip; NUNCA «0,00» (honestidade do medidor).
    #[test]
    fn custo_sem_dado_mostra_travessao_nunca_zero() {
        let cell = cost_cell(&ms(1, 1, Some(UiState::Trabalhando)));
        assert_eq!(cell.text, "—");
        assert_eq!(
            cell.tooltip, "sem sessão hoje",
            "tooltip = fallback da CostLine"
        );
        assert!(!cell.tooltip.contains("0,00"));

        let mut com = ms(1, 1, Some(UiState::Trabalhando));
        com.cost_short = Some("~US$ 0,01".into());
        com.cost_tooltip = "hoje: ~US$ 0,01 (estimado)".into();
        let cell = cost_cell(&com);
        assert_eq!(cell.text, "~US$ 0,01");
        assert!(cell.tooltip.starts_with(COPY_M8_COST_TOOLTIP));
        assert!(
            cell.tooltip.contains("(estimado)"),
            "o «estimado» viaja no tooltip"
        );
    }

    // ── aria-label (checklist §4) ──

    /// Concatenação dos fragmentos congelados; sem dado o fragmento é OMITIDO (não lido
    /// como zero); o fragmento de Pedidos fica fora até a costura §6-C1; ⚠ lê o fragmento
    /// congelado de pasta sumida.
    #[test]
    fn aria_label_concatena_fragmentos_e_omite_sem_dado() {
        let mut row = row_with(ms(2, 3, Some(UiState::Trabalhando)));
        row.status.cost_short = Some("~US$ 0,45".into());
        let aria = row_aria_label(&row);
        assert_eq!(
            aria,
            "Projeto X · 2 Agentes — Trabalhando · gasto estimado de hoje neste Espaço: ~US$ 0,45"
        );

        let row = row_with(ms(0, 0, None));
        let aria = row_aria_label(&row);
        assert_eq!(aria, "Projeto X · sem Agentes ainda");
        assert!(!aria.contains("gasto"), "sem custo o fragmento é omitido");
        assert!(
            !aria.contains("Pedido"),
            "🔔 por Espaço bloqueado (§6-C1): nem sino, nem 'em dia'"
        );

        let mut un = ms(0, 0, None);
        un.unreachable = true;
        let aria = row_aria_label(&row_with(un));
        assert_eq!(aria, format!("Projeto X · {COPY_M8_FOLDER_MISSING}"));

        assert_eq!(create_item_aria(true), "Novo Espaço — PRO");
        assert_eq!(create_item_aria(false), COPY_M8_NEW_SPACE);
    }

    // ── roving + abrir/fechar ──

    /// ↑↓ clampado sobre a lista FILTRADA; Esc fecha sem trocar e zera busca/rename.
    #[test]
    fn roving_clampado_e_esc_fecha_limpo() {
        let mut st = SidebarState::default();
        st.set_rows(vec![
            SidebarRow {
                name: "A".into(),
                ..row_with(ms(0, 0, None))
            },
            SidebarRow {
                name: "B".into(),
                ..row_with(ms(0, 0, None))
            },
        ]);
        st.open();
        st.select_prev();
        assert_eq!(st.selected, 0, "clamp no início");
        st.select_next();
        st.select_next();
        assert_eq!(st.selected, 1, "clamp no fim");
        assert_eq!(st.selected_row().expect("linha").name, "B");

        st.type_char("zzz");
        assert!(st.filtered().is_empty());
        assert_eq!(st.selected_row(), None, "sem resultado, Enter não tem alvo");

        st.close();
        assert!(!st.expanded);
        assert!(st.query.is_empty(), "reabrir nasce limpo");
        assert_eq!(st.renaming, None);
    }

    // ── rename inline (headless) ──

    /// `start` semeia o nome atual, digitar/backspace editam o buffer, `commit` devolve
    /// `(raiz, novo)` p/ o shell despachar em `on_rename`; vazio não confirma; `cancel`
    /// descarta sem efeito.
    #[test]
    fn rename_inline_headless() {
        let mut st = SidebarState::default();
        st.set_rows(vec![SidebarRow {
            name: "Velho".into(),
            ws_root: PathBuf::from("/tmp/sb-rn"),
            ..row_with(ms(0, 0, None))
        }]);
        st.start_rename();
        assert_eq!(
            st.renaming.as_deref(),
            Some("Velho"),
            "semeado com o nome atual"
        );

        st.backspace();
        st.backspace();
        st.backspace();
        st.backspace();
        st.backspace();
        st.type_char("Novo Nome");
        assert_eq!(
            st.query, "",
            "com rename ativo, digitação vai pro buffer, não pra busca"
        );
        let (root, novo) = st.commit_rename().expect("commit");
        assert_eq!(root, PathBuf::from("/tmp/sb-rn"));
        assert_eq!(novo, "Novo Nome");
        assert_eq!(st.renaming, None, "commit encerra o modo");

        st.start_rename();
        st.renaming = Some("   ".into());
        assert_eq!(st.commit_rename(), None, "nome vazio não confirma");

        st.start_rename();
        st.type_char("X");
        st.cancel_rename();
        assert_eq!(st.renaming, None, "cancel descarta sem efeito");
    }

    /// r5 BUG 4 — a MATRIZ do teclado: rail aberto sem foco deixa TUDO fluir ao
    /// terminal (imprimível, Esc, setas); a busca só captura quando focada (⌘F/clique);
    /// Esc fecha o rail SÓ com o foco nele; ⌘O alterna de qualquer estado. Não-vacuoso:
    /// remover o conceito de `kbd` (voltar a rotear por `expanded`) quebra as asserções
    /// de PassThrough.
    #[test]
    fn rail_never_steals_keyboard_matrix() {
        use RailKey::{FocusSearch, PassThrough, ToRail, ToggleRail};
        // Rail FECHADO: nada é do rail (⌘O abre; resto flui).
        assert_eq!(rail_key_route(false, false, "o", true), ToggleRail);
        assert_eq!(rail_key_route(false, false, "a", false), PassThrough);
        assert_eq!(rail_key_route(false, false, "escape", false), PassThrough);
        // Rail ABERTO sem foco (o estado do bug do fundador): digitação vai ao TERMINAL.
        assert_eq!(rail_key_route(true, false, "a", false), PassThrough);
        assert_eq!(rail_key_route(true, false, "down", false), PassThrough);
        assert_eq!(
            rail_key_route(true, false, "escape", false),
            PassThrough,
            "Esc com foco no terminal vai ao terminal — NUNCA fecha o rail"
        );
        assert_eq!(
            rail_key_route(true, false, "o", true),
            ToggleRail,
            "⌘O fecha"
        );
        assert_eq!(
            rail_key_route(true, false, "f", true),
            FocusSearch,
            "⌘F foca a busca"
        );
        // Rail aberto COM foco: o rail dirige (busca/roving/Esc-fecha).
        assert_eq!(rail_key_route(true, true, "a", false), ToRail);
        assert_eq!(rail_key_route(true, true, "escape", false), ToRail);
        assert_eq!(rail_key_route(true, true, "down", false), ToRail);
        assert_eq!(
            rail_key_route(true, true, "o", true),
            ToggleRail,
            "⌘O fecha mesmo focado"
        );
    }

    /// r5 BUG 4(e) + BUG 1: abrir NÃO assume o teclado (divergência registrada da spec
    /// §4/ux-flows:670 — decisão da tela do fundador); clique na busca/⌘F assume;
    /// renomear assume (é digitação); fechar limpa tudo.
    #[test]
    fn opening_rail_does_not_take_keyboard_focus() {
        let mut st = SidebarState::default();
        st.open();
        assert!(
            st.expanded && !st.kbd,
            "aberto, mas o terminal segue dono do teclado"
        );
        st.focus_search();
        assert!(st.kbd, "⌘F/clique na busca: agora o rail é o dono");
        st.close();
        assert!(!st.expanded && !st.kbd, "fechar devolve e limpa");

        // Renomear implica digitação → assume o teclado.
        st.open();
        st.rows = vec![SidebarRow {
            name: "Loja".into(),
            ws_root: PathBuf::from("/tmp/sb-kbd"),
            focused: false,
            status: WorkspaceMiniStatus {
                agents_alive: 0,
                agents_total: 0,
                dominant: None,
                cost_short: None,
                cost_tooltip: String::new(),
                unreachable: false,
            },
            shortcut_index: Some(1),
        }];
        st.start_rename();
        assert!(st.kbd, "rename inline captura a digitação para o buffer");
    }

    /// r5 BUG 2: a ação age na linha CLICADA — `start_rename_for(root)` move o roving
    /// para a linha certa antes de abrir o rename (com as ações visíveis em toda linha,
    /// renomear pelo roving renomearia a errada).
    #[test]
    fn row_action_targets_the_clicked_row() {
        let status = WorkspaceMiniStatus {
            agents_alive: 0,
            agents_total: 0,
            dominant: None,
            cost_short: None,
            cost_tooltip: String::new(),
            unreachable: false,
        };
        let mut st = SidebarState::default();
        st.open();
        st.rows = (1..=3)
            .map(|i| SidebarRow {
                name: format!("Espaço {i}"),
                ws_root: PathBuf::from(format!("/tmp/sb-act-{i}")),
                focused: false,
                status: status.clone(),
                shortcut_index: Some(i),
            })
            .collect();
        assert_eq!(st.selected, 0, "roving nasce na 1ª linha");
        st.start_rename_for(Path::new("/tmp/sb-act-3"));
        assert_eq!(st.selected, 2, "o roving foi à linha clicada");
        assert_eq!(
            st.renaming.as_deref(),
            Some("Espaço 3"),
            "renomeia a CLICADA"
        );
    }

    /// r5 (unload-ui): a ação Descarregar SÓ existe em Espaço de FUNDO e SÓ com o seam
    /// do executor plugado — Espaço ativo nunca a mostra; seam ausente = botão ausente
    /// (tela nunca mente). Não-vacuoso: remover qualquer um dos dois guardas falha aqui.
    #[test]
    fn unload_action_only_on_background_rows_with_seam_wired() {
        assert!(row_can_unload(false, true), "fundo + seam = aparece");
        assert!(!row_can_unload(true, true), "Espaço ATIVO nunca mostra");
        assert!(!row_can_unload(false, false), "sem executor, sem botão");
        assert!(!row_can_unload(true, false));
    }

    /// **Guardião do bug `bf-sidebar` (a regressão que sumia com "Arquivar").** A faixa de
    /// ações é montada a partir de [`row_actions`] — a fonte ÚNICA da presença. O invariante:
    /// fora do rename, Renomear E Arquivar SEMPRE estão presentes (era o que "vazava" para
    /// fora dos 280px e desaparecia); Descarregar entra só sob o seam (regra r5). Não-vacuoso:
    /// remover `Archive` de `row_actions` some-a da UI E falha este teste.
    #[test]
    fn row_actions_always_keep_rename_and_archive_outside_editing() {
        use RowAction::{Archive, Rename, Unload};
        // Linha comum (sem seam de descarregar): Renomear + Arquivar, nesta ordem, sempre.
        assert_eq!(row_actions(false, false), vec![Rename, Archive]);
        // Espaço de fundo com seam plugado: ganha Descarregar ao FIM (nunca expulsa as outras).
        assert_eq!(row_actions(false, true), vec![Rename, Archive, Unload]);
        // Arquivar é o coração do bug — afirme-o explicitamente em ambos os casos.
        assert!(
            row_actions(false, false).contains(&Archive)
                && row_actions(false, true).contains(&Archive),
            "Arquivar NUNCA pode sumir fora do rename"
        );
        // Renomeando: o campo vira editor inline — nenhuma ação compete por espaço.
        assert!(
            row_actions(true, false).is_empty(),
            "rename esconde as ações"
        );
        assert!(
            row_actions(true, true).is_empty(),
            "rename ignora até o seam"
        );
    }

    /// r5 (unload-ui): a narração vem dos números REAIS do plano — quem segue
    /// trabalhando fica LIGADO e é dito; ociosos desligam "e religam quando você
    /// voltar"; plural/singular honestos.
    #[test]
    fn unload_narration_tells_kept_and_parked_counts() {
        let s = copy_unload_narration(3, 2);
        assert!(
            s.contains("2 agentes seguem trabalhando e ficaram ligados"),
            "{s}"
        );
        assert!(s.contains("3 terminais ociosos desligados"), "{s}");
        assert!(s.contains("religam quando você voltar"), "{s}");

        let s1 = copy_unload_narration(1, 1);
        assert!(
            s1.contains("1 agente segue trabalhando e ficou ligado"),
            "{s1}"
        );
        assert!(s1.contains("1 terminal ocioso desligado"), "{s1}");

        let s0 = copy_unload_narration(0, 3);
        assert!(s0.contains("3 agentes seguem trabalhando"), "{s0}");
        assert!(s0.contains("nenhum terminal precisou desligar"), "{s0}");

        let all_idle = copy_unload_narration(4, 0);
        assert!(
            !all_idle.contains("seguem trabalhando"),
            "sem kept, não inventa agente trabalhando: {all_idle}"
        );
    }

    /// Spec §M8 (a11y): o toast de arquivar tem janela REAL de Desfazer — não expira antes
    /// do timeout, expira depois, e o countdown desce de N até 0 (teto, nunca "0s" com
    /// tempo sobrando). Tudo com relógio INJETADO (determinístico).
    #[test]
    fn archive_toast_window_and_countdown() {
        let t0 = 1_000_000_u64;
        let t = ArchiveToast::new(PathBuf::from("/tmp/sb-arch"), "Loja".into(), t0);
        assert!(!t.expired(t0), "nasce dentro da janela");
        assert!(!t.expired(t0 + ARCHIVE_TOAST_TIMEOUT_MS - 1));
        assert!(t.expired(t0 + ARCHIVE_TOAST_TIMEOUT_MS), "fecha no prazo");
        assert_eq!(t.remaining_secs(t0), ARCHIVE_TOAST_TIMEOUT_MS / 1000);
        assert_eq!(
            t.remaining_secs(t0 + 7_100),
            1,
            "teto: 900ms restantes → 1s"
        );
        assert_eq!(t.remaining_secs(t0 + ARCHIVE_TOAST_TIMEOUT_MS), 0);
        // O anúncio da live-region carrega o NOME (o leigo sabe O QUE foi arquivado).
        assert!(copy_archive_announce("Loja").contains("“Loja”"));
        assert!(copy_archive_undone("Loja").contains("“Loja”"));
    }

    /// Spec §M8: `[ Desfazer ]` é alcançável por TECLADO antes do timeout — `Tab` foca,
    /// `Enter` ativa; `⌘Z` desfaz de qualquer lugar; `Enter` SEM foco no botão não desfaz
    /// (não rouba o Enter de quem está digitando). Não-vacuoso: remova `archive_toast_key`
    /// e este teste falha.
    #[test]
    fn archive_toast_undo_reachable_by_keyboard() {
        use ArchiveToastKey::{FocusUndo, None as KNone, Undo};
        // caminho Tab → Enter (foco primeiro, ativação depois).
        assert_eq!(archive_toast_key("tab", false, false), FocusUndo);
        assert_eq!(archive_toast_key("enter", false, true), Undo);
        assert_eq!(archive_toast_key("return", false, true), Undo);
        // ⌘Z desfaz direto, com ou sem foco no botão.
        assert_eq!(archive_toast_key("z", true, false), Undo);
        assert_eq!(archive_toast_key("z", true, true), Undo);
        // Enter sem foco no botão NÃO desfaz; demais teclas seguem o fluxo normal.
        assert_eq!(archive_toast_key("enter", false, false), KNone);
        assert_eq!(archive_toast_key("z", false, false), KNone);
        assert_eq!(archive_toast_key("escape", false, false), KNone);
    }
}
