//! `agent_modal` — **F1-2-2 (M6/M6-E): o modal de criar/editar Agente** (evolui o M2 da Fase 0).
//!
//! A tese da story: superar o Maestri por **menos** — 4 controles visíveis (motor·nome·papel·
//! pasta), zero YAML/jargão na superfície, co-piloto de papel pelo nome (via o contrato
//! [`crate::role_suggester::RoleSuggester`] — fronteira F1-2-2↔F1-2-3) e quick-start **somente
//! dos motores detectados** (W4-1), nunca lista fixa.
//!
//! Split sedimentado do shell: [`AgentModal`] é **gpui-free e testável** (estados, debounce por
//! tick, validação, dedup de nome, plano de criação); [`render`] é a casca fina gpui. Toda
//! mutação vira evento **só no Criar/Salvar** (kill -9 no meio do preenchimento não deixa nó
//! fantasma — estado do modal é RAM). Wireframe/copy: `ux-flows.md` fluxo (a), autoritativo.
//!
//! **Critério 7 (neutralidade)**: o comando do motor vem do **CLI Profile TOML lido do disco em
//! runtime** ([`load_profiles`] — semeado uma vez de `profiles/claude-code.toml` e depois SEMPRE
//! lido do arquivo: trocar o TOML muda o comando sem recompilar). Descoberta de motores roda
//! **fora da thread de UI** e entra por [`AgentModal::set_engines`] (lição W4-1: injetável).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, text, transparent_black, AnyElement, ClickEvent, Context, FontWeight,
    Pixels, ScrollHandle, Size,
};
use lina_cli_profiles::ProfileRegistry;
use lina_core::DiscoveredCli;
use lina_host::NodeId;

use crate::bridge::AgentEngine;
use crate::role_suggester::{humanize, role_registry, RoleSuggester, RoleSuggestion};
use crate::theme;
use crate::WorkspaceView;

// ═══════════════════════════ copy (auditável — critério 2: zero jargão) ═══════════════════════════
// Toda string visível do modal mora AQUI e passa pelo teste `copy_has_no_jargon` (proíbe
// YAML/TOML/profile/terminal em QUALQUER estado — anti-padrões 1/2/5 do fluxo (a)).

pub const COPY_TITLE_CREATE: &str = "Novo Agente";
pub const COPY_TITLE_EDIT: &str = "Editar Agente";
pub const COPY_QUICKSTART: &str = "Início rápido — motores prontos no seu computador:";
pub const COPY_SCANNING: &str = "Procurando motores no seu computador…";
pub const COPY_EMPTY_TITLE: &str = "Vamos preparar tudo para você";
pub const COPY_EMPTY_CTA: &str = "Instalar o motor recomendado";
pub const COPY_INSTALL_OTHER: &str = "+ Instalar outro motor…";
pub const COPY_NAME_LABEL: &str = "Nome";
pub const COPY_NAME_PLACEHOLDER: &str = "Ex.: Revisor, Designer de Landing…";
pub const COPY_ROLE_LABEL: &str = "Papel";
pub const COPY_ROLE_PLACEHOLDER: &str =
    "O papel nasce do nome: digite o nome acima que eu sugiro aqui. Para escolher você mesmo, use o Trocar.";
/// Fix de tela: nome sem padrão conhecido (ex.: "Teles") mostra fallback HONESTO no campo —
/// nunca silêncio (o fundador digitou e "nada aconteceu").
pub const COPY_ROLE_NO_MATCH: &str =
    "Não reconheci um papel nesse nome — escolha um com o Trocar, ou crie assim mesmo (dá para definir depois).";
/// Hint do preview da sugestão DENTRO do campo Papel (aceita por Tab ou clique).
pub const COPY_ROLE_SUGGESTED_HINT: &str = "sugestão — aceite com Tab ou clique";
// ── seletor de papéis (F1-2-3 + requisito do fundador: dropdown, não ciclar) ──
pub const COPY_GALLERY_TITLE: &str = "Escolha o papel";
pub const COPY_GALLERY_SEARCH_PLACEHOLDER: &str = "busque por nome ou pelo que ele faz…";
pub const COPY_GALLERY_EMPTY: &str =
    "Nenhum papel casa com a busca — apague para ver todos, ou comece pelo Nome que eu sugiro.";
pub const COPY_GALLERY_HINT: &str = "↑↓ navega · Enter escolhe · Esc fecha";
pub const COPY_ROLE_SWAP: &str = "Trocar";
pub const COPY_CWD_LABEL: &str = "Pasta de trabalho";
pub const COPY_CWD_DEFAULT: &str = "a pasta deste Espaço";
pub const COPY_CWD_PICK: &str = "Trocar…";
pub const COPY_ADVANCED: &str = "▸ Avançado";
pub const COPY_ADVANCED_OPEN: &str = "▾ Avançado";
pub const COPY_CMD_LABEL: &str = "Comando do motor (modo técnico)";
pub const COPY_CREATE: &str = "Criar Agente";
pub const COPY_SAVE: &str = "Salvar";
pub const COPY_CANCEL: &str = "Cancelar";
pub const COPY_ERR_EMPTY_NAME: &str = "Dê um nome ao agente antes de criar.";
pub const COPY_ERR_SCANNING: &str = "Um instante — ainda estou procurando os motores.";
pub const COPY_ERR_NO_ENGINE: &str =
    "Nenhum motor instalado ainda — use o botão acima e eu instalo para você.";
pub const COPY_DISCARD_ARMED: &str = "Descartar este Agente? (Esc de novo descarta)";
/// Falha ao criar/salvar (ex.: motor sumiu entre a detecção e o clique) — leiga, sem stack trace;
/// o detalhe técnico vai ao stderr.
pub const COPY_ERR_COMMIT: &str =
    "Não consegui criar agora — o motor pode ter mudado. Tente de novo ou escolha outro.";
pub const COPY_EDIT_ENGINE_LOCKED: &str = "⚠ trocar o motor reinicia o Agente — em breve";
pub const COPY_EDIT_APPLY_NOTE: &str = "Mudanças de nome e papel valem agora.";
pub const COPY_WHY_PREFIX: &str = "ⓘ por quê?";

/// Linha-fantasma do co-piloto (M6-E2): nunca popup, nunca rouba foco.
#[must_use]
pub fn copy_ghost(label: &str) -> String {
    format!("Parece um {label} — usar este papel? (Tab)")
}

/// Aviso leve de nome duplicado (tabela de erros do fluxo (a): sufixo automático, não bloqueia).
#[must_use]
pub fn copy_dup_note(unique: &str) -> String {
    format!("Já existia um com esse nome — este será \u{201c}{unique}\u{201d}.")
}

// ═══════════════════════════ motores (W4-1 → quick-start) ═══════════════════════════

/// Um motor pronto para o quick-start: rótulo leigo + comando (do CLI Profile TOML quando há;
/// senão o binário descoberto, cru).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    /// id da descoberta (`"claude"`, `"codex"`, …).
    pub id: String,
    /// Rótulo leigo do chip ("Claude Code").
    pub label: String,
    pub version: Option<String>,
    pub program: String,
    pub args: Vec<String>,
    /// id do CLI Profile TOML quando existe (`"claude-code"`) → vira `CliProfileSet` no log.
    pub profile_id: Option<String>,
}

/// Rótulo leigo de um CLI descoberto (ids do `KNOWN_CLIS` do core).
#[must_use]
pub fn engine_label(id: &str) -> String {
    match id {
        "claude" => "Claude Code".to_string(),
        "codex" => "Codex".to_string(),
        "gemini" => "Gemini CLI".to_string(),
        "opencode" => "OpenCode".to_string(),
        "copilot" => "Copilot CLI".to_string(),
        "agy" => "Antigravity".to_string(),
        other => {
            let mut s = other.to_string();
            if let Some(f) = s.get_mut(0..1) {
                f.make_ascii_uppercase();
            }
            s
        }
    }
}

/// Monta a lista de motores do quick-start a partir da DESCOBERTA (W4-1) + os profiles TOML do
/// disco. **Recomendado primeiro** (claude), demais na ordem da descoberta. Puro e testável.
#[must_use]
pub fn engines_from(found: &[DiscoveredCli], profiles: &ProfileRegistry) -> Vec<Engine> {
    let mut out: Vec<Engine> = found
        .iter()
        .map(|d| {
            // O profile cujo `program` é o binário descoberto dá comando/args/identidade.
            let profile = profiles.ids().find_map(|id| {
                let p = profiles.get(id)?;
                (p.program == d.id).then_some(p)
            });
            Engine {
                id: d.id.clone(),
                label: engine_label(&d.id),
                version: d.version.clone(),
                program: profile.map_or_else(|| d.id.clone(), |p| p.program.clone()),
                args: profile.map(|p| p.args.clone()).unwrap_or_default(),
                profile_id: profile.map(|p| p.id.clone()),
            }
        })
        .collect();
    // Recomendado (claude) primeiro — estabilidade no resto (sem reordenar surpresa).
    out.sort_by_key(|e| if e.id == "claude" { 0 } else { 1 });
    out
}

/// Estado da varredura de motores (a descoberta roda FORA da thread de UI e entra por
/// [`AgentModal::set_engines`] — lição W4-1: off-thread + injetável p/ teste determinístico).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineScan {
    Scanning,
    Ready(Vec<Engine>),
}

// ═══════════════════════════ CLI Profiles em RUNTIME (critério 7) ═══════════════════════════

/// Seed do profile recomendado — escrito UMA vez no dir de profiles do usuário; depois disso o
/// arquivo do DISCO manda (editar/trocar o TOML muda o comando sem recompilar).
const CLAUDE_PROFILE_SEED: &str = include_str!("../../../profiles/claude-code.toml");

/// Onde os CLI Profiles do usuário moram: `LINA_PROFILES` (override de dev) ou
/// `<ws_root>/.lina/profiles` (local-first, por Espaço).
#[must_use]
pub fn profiles_dir(mailbox_dir: &Path) -> PathBuf {
    std::env::var_os("LINA_PROFILES")
        .map(PathBuf::from)
        .unwrap_or_else(|| mailbox_dir.join("profiles"))
}

/// Carrega os CLI Profiles TOML **do disco** (semeando `claude-code.toml` na primeira vez).
/// Best-effort: erro de I/O/parse → registry vazio + stderr (o modal degrada para o binário
/// descoberto; criação nunca bloqueia).
#[must_use]
pub fn load_profiles(dir: &Path) -> ProfileRegistry {
    let seed = dir.join("claude-code.toml");
    if !seed.exists() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("lina-gpui: M6 — não criei {} ({e})", dir.display());
        } else if let Err(e) = std::fs::write(&seed, CLAUDE_PROFILE_SEED) {
            eprintln!("lina-gpui: M6 — não semeei {} ({e})", seed.display());
        }
    }
    match ProfileRegistry::load_dir(dir) {
        Ok(reg) => reg,
        Err(e) => {
            eprintln!(
                "lina-gpui: M6 — profiles de {} inválidos ({e})",
                dir.display()
            );
            ProfileRegistry::new()
        }
    }
}

// ═══════════════════════════ o modelo (gpui-free) ═══════════════════════════

/// Ticks do loop do modal (~50ms cada) até o co-piloto disparar após a pausa de digitação
/// (M6-E2: ~600ms). 12 × 50ms = 600ms.
pub const SUGGEST_DEBOUNCE_TICKS: u32 = 12;

// ═══════════ contenção do seletor (fix de tela bugB2 — matemática PURA, testável) ═══════════
// O dropdown da galeria ESTOURAVA o modal e a janela (lista inteira inline, sem viewport; caixa
// de 640px fixa em janela estreita). gpui não roda headless → o clamp vive AQUI, em f32 puro,
// com teste unitário; o render só APLICA os números. As constantes são FONTE ÚNICA: a mesma
// tabela alimenta a conta e os `.h(px(..))`/`.line_height(px(..))` do render — se divergirem, a
// matemática mente (teste `gallery_row_math_matches_render_constants` guarda a amarração).

/// Largura de projeto da caixa do modal (a de antes do fix — preservada quando há espaço).
pub const MODAL_W: f32 = 640.0;
/// Piso da caixa em janela minúscula (abaixo disso o conteúdo vira ruído).
pub const MODAL_W_MIN: f32 = 320.0;
/// Respiro mínimo entre a caixa e CADA borda da janela (requisito 3: nunca colar/estourar).
pub const WINDOW_MARGIN: f32 = 16.0;
/// Padding horizontal da caixa (px_6) — a largura útil do modal é `w - 2*MODAL_PAD_X`.
pub const MODAL_PAD_X: f32 = 24.0;
/// Padding interno do dropdown (px_3/py_3).
pub const GALLERY_PAD: f32 = 12.0;
/// gap_2 entre cabeçalho · busca · lista.
pub const GALLERY_GAP: f32 = 8.0;
/// border_1 do container do dropdown — conta no chrome (taffy é border-box: a borda come o
/// espaço interno; sem ela na soma, a lista clipa 2px).
pub const GALLERY_BORDER: f32 = 1.0;
/// line_height EXPLÍCITA de cada linha de texto (lição do projeto: a natural da fonte clipa).
pub const GALLERY_LINE_H: f32 = 20.0;
/// Cabeçalho do dropdown (1 linha: título + hint).
pub const GALLERY_HEADER_H: f32 = 20.0;
/// Caixa de busca (py_2 + 1 linha) — FIXA no topo (requisito 4); só a lista rola.
pub const GALLERY_SEARCH_H: f32 = 36.0;
/// Linha da lista: 2 linhas de texto + py_2 + border_2 dos dois lados (geometria UNIFORME — a
/// inativa usa borda transparente, então navegar nunca reflui o layout).
pub const GALLERY_ROW_H: f32 = 60.0;
/// Espaço entre linhas da lista (gap_1, via mb por linha — o viewport de scroll é bloco).
pub const GALLERY_ROW_GAP: f32 = 4.0;
/// Teto de conforto da viewport (~6,4 linhas): em tela grande a lista NÃO vira um totem — o
/// corte não-inteiro de linha sinaliza visualmente que rola.
pub const GALLERY_VIEWPORT_MAX: f32 = 384.0;
/// Orçamento vertical do modal SEM a lista (título + chips + nome + papel + pasta + avançado +
/// rodapé + gaps + paddings), estimado por cima de propósito: sobrar é seguro, faltar estoura.
pub const MODAL_BASE_RESERVED: f32 = 500.0;

/// Tamanho lógico em px — `f32` cru (gpui-free) para o teste rodar headless.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WinSize {
    pub w: f32,
    pub h: f32,
}

/// O quadro da caixa do modal DENTRO da janela: largura clampada + teto de altura.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModalFrame {
    /// Largura final da caixa (`MODAL_W` quando cabe; encolhe em janela estreita).
    pub w: f32,
    /// Teto de altura da caixa (janela − 2×margem) — o render aplica como `max_h`.
    pub max_h: f32,
}

/// Quadro do modal para uma janela: NUNCA mais largo/alto que a janela menos as margens
/// (era o estouro horizontal: `w(px(640))` fixo em janela estreita).
#[must_use]
pub fn modal_frame(window: WinSize) -> ModalFrame {
    ModalFrame {
        w: MODAL_W.min(window.w - 2.0 * WINDOW_MARGIN).max(MODAL_W_MIN),
        max_h: (window.h - 2.0 * WINDOW_MARGIN).max(160.0),
    }
}

/// Altura do "chrome" do dropdown (tudo menos a viewport da lista): bordas + paddings + gaps +
/// cabeçalho + busca.
#[must_use]
pub fn gallery_chrome_h() -> f32 {
    2.0 * GALLERY_BORDER
        + 2.0 * GALLERY_PAD
        + 2.0 * GALLERY_GAP
        + GALLERY_HEADER_H
        + GALLERY_SEARCH_H
}

/// Bounds finais do dropdown + viewport da lista (o contrato do fix).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GalleryLayout {
    /// Largura do dropdown == largura ÚTIL do modal (requisito 1: nunca mais largo).
    pub w: f32,
    /// Altura total do dropdown (chrome + viewport).
    pub h: f32,
    /// Altura da VIEWPORT da lista — o conteúdo rola POR DENTRO quando excede (requisito 2).
    pub list_viewport_h: f32,
    /// O conteúdo excede a viewport? (o render usa a viewport igual; a flag é dos testes/UX.)
    pub scrolls: bool,
}

/// **A conta do fix**: dado o quadro do modal + a janela + nº de itens filtrados → bounds do
/// dropdown e viewport da lista. Clamp em 3 camadas: conteúdo justo (sem buraco morto) ≤ espaço
/// vertical disponível (janela − orçamento base − chrome) ≤ teto de conforto — com PISO de 1
/// linha (janela degenerada mantém o seletor usável; o residual é absorvido pelo scroll do
/// corpo do modal, fallback declarativo do render — nunca pintado por cima do canvas).
#[must_use]
pub fn gallery_layout(modal: ModalFrame, window: WinSize, n_items: usize) -> GalleryLayout {
    let w = modal.w - 2.0 * MODAL_PAD_X;
    let content = if n_items == 0 {
        GALLERY_ROW_H // a linha única da mensagem de "nenhum papel casa".
    } else {
        n_items as f32 * GALLERY_ROW_H + (n_items as f32 - 1.0) * GALLERY_ROW_GAP
    };
    // Teto REAL: o menor entre o quadro recebido e a própria janela (robusto a um frame que
    // não veio de `modal_frame`).
    let ceiling = (window.h - 2.0 * WINDOW_MARGIN).min(modal.max_h);
    let space = ceiling - MODAL_BASE_RESERVED - gallery_chrome_h();
    // clamp seguro: ROW_H (60) ≤ VIEWPORT_MAX (384) por construção — nunca panica.
    let cap = space.clamp(GALLERY_ROW_H, GALLERY_VIEWPORT_MAX);
    let list_viewport_h = content.min(cap);
    GalleryLayout {
        w,
        h: gallery_chrome_h() + list_viewport_h,
        list_viewport_h,
        scrolls: content > list_viewport_h + 0.5,
    }
}

/// **Seletor de papéis** ([Trocar] — F1-2-3 + feedback de tela do fundador: "tinha que ser um
/// seletor ou dropdown", não ciclar de um em um). TODOS os papéis da biblioteca visíveis de uma
/// vez (a biblioteca É o registry W3-1 humanizado — fonte única, offline, zero LLM), com busca
/// por texto (nome leigo OU descrição) e navegação por teclado (↑↓ + Enter; Esc fecha).
/// gpui-free e testável — espelha o idioma da paleta M1. (Exceção consciente: o `scroll` é um
/// `gpui::ScrollHandle` — um Rc-cell INERTE, sem janela/plataforma — para o ↑↓ manter a linha
/// destacada visível dentro da viewport; construí-lo em teste headless é seguro.)
pub struct RoleGallery {
    items: Vec<RoleSuggestion>,
    query: String,
    /// Índice selecionado DENTRO da lista filtrada (teclado ↑↓; clamp ao filtrar).
    selected: usize,
    /// Scroll da viewport da lista (one-shot `scroll_to_item` ao navegar por teclado — não
    /// briga com o wheel: fora da navegação o offset é do usuário).
    scroll: ScrollHandle,
}

impl RoleGallery {
    /// Monta a biblioteca a partir do registry W3-1 (humanizado): nome leigo + 1 frase por papel.
    #[must_use]
    pub fn from_library() -> Self {
        let items = role_registry()
            .map(|reg| {
                reg.role_names()
                    .map(|canon| {
                        let (label, blurb) = humanize(canon);
                        RoleSuggestion {
                            role: canon.to_string(),
                            label,
                            blurb,
                            why: "Você escolheu este papel na biblioteca.".to_string(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            items,
            query: String::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
        }
    }

    /// Handle do scroll da lista — o render amarra na viewport via `track_scroll`.
    #[must_use]
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// Os papéis que casam com a busca (case-insensitive, nome leigo OU descrição). Sem busca →
    /// TODOS de uma vez (o requisito do fundador).
    #[must_use]
    pub fn filtered(&self) -> Vec<&RoleSuggestion> {
        let q = self.query.trim().to_lowercase();
        self.items
            .iter()
            .filter(|r| {
                q.is_empty()
                    || r.label.to_lowercase().contains(&q)
                    || r.blurb.to_lowercase().contains(&q)
            })
            .collect()
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Índice selecionado na lista FILTRADA (p/ o render destacar com o focus ring).
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected.min(self.filtered().len().saturating_sub(1))
    }

    pub fn type_char(&mut self, s: &str) {
        self.query.push_str(s);
        self.selected = 0;
        self.scroll.scroll_to_item(0); // filtro novo → lista volta ao topo.
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
        self.scroll.scroll_to_item(0);
    }

    /// ↑↓ com clamp nas bordas (sem wrap — previsível pro leitor de tela). A viewport SEGUE a
    /// seleção (scroll mínimo one-shot) — o destaque nunca some abaixo/acima da dobra.
    pub fn move_selection(&mut self, delta: i32) {
        let len = self.filtered().len();
        if len == 0 {
            return;
        }
        let cur = self.selected() as i32;
        self.selected = (cur + delta).clamp(0, len as i32 - 1) as usize;
        self.scroll.scroll_to_item(self.selected);
    }

    /// O papel sob o cursor (Enter) — `None` com a busca vazia de resultados.
    #[must_use]
    pub fn pick(&self) -> Option<RoleSuggestion> {
        self.filtered().get(self.selected()).map(|r| (*r).clone())
    }

    /// O papel na posição `idx` da lista FILTRADA (clique do mouse).
    #[must_use]
    pub fn pick_at(&self, idx: usize) -> Option<RoleSuggestion> {
        self.filtered().get(idx).map(|r| (*r).clone())
    }
}

/// Criar do zero ou editar um nó vivo (M6-E).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalMode {
    Create,
    Edit { node: NodeId },
}

/// Qual buffer de texto recebe o teclado (Nome é o foco inicial; Comando só no Avançado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusField {
    Name,
    Command,
}

/// O plano de CRIAÇÃO commitado pelo `[[ Criar Agente ]]` — só aqui nascem eventos (critério 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    /// Nome final (dedup com sufixo "(2)" aplicado).
    pub name: String,
    /// Aviso leve quando o dedup agiu (tabela de erros: não bloqueia).
    pub dup_note: Option<String>,
    pub engine: AgentEngine,
    /// Pasta escolhida (None = pasta gerenciada do Espaço).
    pub cwd: Option<PathBuf>,
    /// Papel CANÔNICO escolhido/aceito (None = inferir do nome, comportamento M2).
    pub role: Option<String>,
}

/// O plano de SALVAR do modo edição (M6-E): aplica nome/papel agora; motor fica p/ F1-2-4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    pub node: NodeId,
    pub name: String,
    /// Papel canônico a gravar (None = não mudou).
    pub role: Option<String>,
}

/// O modal M6/M6-E — estado puro (nenhum gpui; nenhum evento até Criar/Salvar).
pub struct AgentModal {
    pub mode: ModalMode,
    name: String,
    /// Ticks desde a última tecla no Nome (None = sem digitação pendente de sugestão).
    typing_ticks: Option<u32>,
    /// Linha-fantasma corrente (não aplicada — some se ignorada).
    suggestion: Option<RoleSuggestion>,
    /// Cartão de Papel APLICADO (Tab/clique/Trocar). Nunca aplicado silenciosamente.
    role: Option<RoleSuggestion>,
    /// `true` → o papel veio de escolha manual ([Trocar]) e NÃO deve ser re-sugerido por cima.
    role_pinned: bool,
    engines: EngineScan,
    selected_engine: usize,
    /// Pasta escolhida (None = default leigo "a pasta deste Espaço").
    cwd: Option<PathBuf>,
    advanced: bool,
    /// Override do comando (Avançado). None = derivado do motor selecionado.
    command: Option<String>,
    focus: FocusField,
    error: Option<String>,
    /// Esc com conteúdo arma a confirmação de descarte (2º Esc fecha).
    discard_armed: bool,
    /// Tooltip "por quê" aberto (transparência da sugestão).
    why_open: bool,
    /// O co-piloto RODOU (3+ chars, pausa) e não reconheceu papel — o campo mostra o fallback
    /// honesto em vez de silêncio (fix de tela). Limpa ao sugerir/aceitar/trocar.
    no_match: bool,
    /// Nomes já existentes no Espaço (dedup "(2)").
    existing: Vec<String>,
    /// [Trocar] aberto: o seletor de papéis (dropdown — F1-2-3; fundador: "não ciclar").
    gallery: Option<RoleGallery>,
    suggester: Arc<dyn RoleSuggester>,
    /// (M6-E) nome/papel originais — p/ detectar mudança real no Salvar.
    original: Option<(String, Option<String>)>,
}

impl AgentModal {
    /// Modal em modo CRIAR. `existing` = nomes vivos do Espaço (dedup).
    #[must_use]
    pub fn new_create(suggester: Arc<dyn RoleSuggester>, existing: Vec<String>) -> Self {
        Self {
            mode: ModalMode::Create,
            name: String::new(),
            typing_ticks: None,
            suggestion: None,
            role: None,
            role_pinned: false,
            engines: EngineScan::Scanning,
            selected_engine: 0,
            cwd: None,
            advanced: false,
            command: None,
            focus: FocusField::Name,
            error: None,
            discard_armed: false,
            why_open: false,
            no_match: false,
            existing,
            gallery: None,
            suggester,
            original: None,
        }
    }

    /// Modal em modo EDITAR (M6-E): pré-preenchido com o estado atual do nó.
    #[must_use]
    pub fn new_edit(
        suggester: Arc<dyn RoleSuggester>,
        node: NodeId,
        name: &str,
        role_canon: Option<&str>,
        existing: Vec<String>,
    ) -> Self {
        let mut m = Self::new_create(suggester, existing);
        m.mode = ModalMode::Edit { node };
        m.name = name.to_string();
        m.role = role_canon.map(|r| {
            let (label, blurb) = humanize(r);
            RoleSuggestion {
                role: r.to_string(),
                label,
                blurb,
                why: "Papel atual deste Agente.".to_string(),
            }
        });
        // editar nunca re-sugere por cima do papel vigente sem novo nome (M6-E: pergunta antes).
        m.role_pinned = m.role.is_some();
        m.original = Some((name.to_string(), role_canon.map(str::to_string)));
        m
    }

    // ── leitura (render/testes) ──

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn suggestion(&self) -> Option<&RoleSuggestion> {
        self.suggestion.as_ref()
    }
    #[must_use]
    pub fn role(&self) -> Option<&RoleSuggestion> {
        self.role.as_ref()
    }
    #[must_use]
    pub fn engines(&self) -> &EngineScan {
        &self.engines
    }
    #[must_use]
    pub fn selected_engine(&self) -> Option<&Engine> {
        match &self.engines {
            EngineScan::Ready(v) => v.get(self.selected_engine),
            EngineScan::Scanning => None,
        }
    }
    #[must_use]
    pub fn selected_engine_idx(&self) -> usize {
        self.selected_engine
    }
    #[must_use]
    pub fn cwd_display(&self) -> String {
        self.cwd
            .as_ref()
            .map_or_else(|| COPY_CWD_DEFAULT.to_string(), |p| p.display().to_string())
    }
    #[must_use]
    pub fn advanced_open(&self) -> bool {
        self.advanced
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[must_use]
    pub fn discard_armed(&self) -> bool {
        self.discard_armed
    }
    #[must_use]
    pub fn why_open(&self) -> bool {
        self.why_open
    }
    #[must_use]
    pub fn focus_field(&self) -> FocusField {
        self.focus
    }

    /// O comando EXIBIDO no Avançado: override do usuário, senão o do motor (profile TOML).
    #[must_use]
    pub fn effective_command(&self) -> String {
        if let Some(c) = &self.command {
            return c.clone();
        }
        match self.selected_engine() {
            Some(e) => {
                let mut s = e.program.clone();
                for a in &e.args {
                    s.push(' ');
                    s.push_str(a);
                }
                s
            }
            None => String::new(),
        }
    }

    // ── mutação (teclado/cliques) ──

    /// Caractere imprimível no campo focado. Digitar no Nome re-arma o debounce do co-piloto e
    /// desarma o descarte; digitar dispensa a linha-fantasma corrente sem fricção (M6-E2).
    pub fn type_char(&mut self, s: &str) {
        self.discard_armed = false;
        self.error = None;
        match self.focus {
            FocusField::Name => {
                self.name.push_str(s);
                self.typing_ticks = Some(0);
                self.suggestion = None;
            }
            FocusField::Command => {
                let mut c = self.effective_command();
                c.push_str(s);
                self.command = Some(c);
            }
        }
    }

    pub fn backspace(&mut self) {
        self.discard_armed = false;
        self.error = None;
        match self.focus {
            FocusField::Name => {
                self.name.pop();
                self.typing_ticks = Some(0);
                self.suggestion = None;
            }
            FocusField::Command => {
                let mut c = self.effective_command();
                c.pop();
                self.command = Some(c);
            }
        }
    }

    pub fn set_focus(&mut self, f: FocusField) {
        self.focus = f;
    }

    /// Um tick do loop do modal (~50ms). Devolve `true` se algo visível mudou (→ `cx.notify`).
    /// O co-piloto dispara na PAUSA (SUGGEST_DEBOUNCE_TICKS), com 3+ chars, e **re-sugere só em
    /// mudança substancial**: papel diferente do já aplicado/sugerido (não a cada tecla).
    pub fn tick(&mut self) -> bool {
        let Some(t) = self.typing_ticks else {
            return false;
        };
        if t + 1 < SUGGEST_DEBOUNCE_TICKS {
            self.typing_ticks = Some(t + 1);
            return false;
        }
        self.typing_ticks = None;
        let fresh = self.suggester.suggest(&self.name);
        // Fix de tela: rodou e NÃO reconheceu (nome com 3+ chars) → fallback honesto no campo.
        self.no_match = fresh.is_none() && self.name.trim().len() >= 3;
        match fresh {
            Some(s) => {
                // papel manual ([Trocar]) não é atropelado; igual ao aplicado = sem ghost (ruído).
                let same_as_applied = self.role.as_ref().is_some_and(|r| r.role == s.role);
                if same_as_applied || (self.role_pinned && self.role.is_some()) {
                    // M6-E: renomear COM papel pinado → a pergunta explícita fica visível na
                    // linha-fantasma mesmo assim quando o papel sugerido difere.
                    if self.role_pinned && self.role.as_ref().is_some_and(|r| r.role != s.role) {
                        self.suggestion = Some(s);
                        return true;
                    }
                    self.suggestion = None;
                    return true;
                }
                self.suggestion = Some(s);
                true
            }
            None => self.suggestion.take().is_some(),
        }
    }

    /// Aceita a sugestão (Tab/clique) — ÚNICO caminho que aplica papel sugerido (nunca silencioso).
    pub fn accept_suggestion(&mut self) -> bool {
        match self.suggestion.take() {
            Some(s) => {
                self.role = Some(s);
                self.role_pinned = false;
                self.why_open = false;
                self.no_match = false;
                true
            }
            None => false,
        }
    }

    /// O co-piloto rodou e não reconheceu papel no nome atual (fallback honesto no campo).
    #[must_use]
    pub fn copilot_no_match(&self) -> bool {
        self.no_match
    }

    /// [Trocar]: abre o SELETOR de papéis (F1-2-3 + feedback do fundador — todos os papéis de
    /// uma vez, com busca; nada de ciclar de um em um).
    pub fn open_gallery(&mut self) {
        self.gallery = Some(RoleGallery::from_library());
    }

    #[must_use]
    pub fn gallery(&self) -> Option<&RoleGallery> {
        self.gallery.as_ref()
    }

    #[must_use]
    pub fn gallery_open(&self) -> bool {
        self.gallery.is_some()
    }

    pub fn gallery_close(&mut self) {
        self.gallery = None;
    }

    pub fn gallery_type(&mut self, s: &str) {
        if let Some(g) = self.gallery.as_mut() {
            g.type_char(s);
        }
    }

    pub fn gallery_backspace(&mut self) {
        if let Some(g) = self.gallery.as_mut() {
            g.backspace();
        }
    }

    pub fn gallery_move(&mut self, delta: i32) {
        if let Some(g) = self.gallery.as_mut() {
            g.move_selection(delta);
        }
    }

    /// Aplica o papel escolhido (Enter / clique em `idx`) — PINA (escolha manual vence a
    /// re-sugestão, mesma regra do antigo Trocar) e fecha o seletor. `false` = nada escolhido.
    pub fn gallery_pick(&mut self, idx: Option<usize>) -> bool {
        let Some(g) = self.gallery.as_ref() else {
            return false;
        };
        let picked = match idx {
            Some(i) => g.pick_at(i),
            None => g.pick(),
        };
        match picked {
            Some(role) => {
                self.role = Some(role);
                self.role_pinned = true;
                self.suggestion = None;
                self.no_match = false;
                self.gallery = None;
                true
            }
            None => false,
        }
    }

    pub fn toggle_why(&mut self) {
        self.why_open = !self.why_open;
    }

    pub fn toggle_advanced(&mut self) {
        self.advanced = !self.advanced;
        if !self.advanced {
            self.focus = FocusField::Name;
        }
    }

    pub fn select_engine(&mut self, idx: usize) {
        if let EngineScan::Ready(v) = &self.engines {
            if idx < v.len() {
                self.selected_engine = idx;
                self.command = None; // o comando derivado segue o motor novo
            }
        }
    }

    /// Resultado da descoberta off-thread (ou snapshot injetado em teste).
    pub fn set_engines(&mut self, engines: Vec<Engine>) {
        self.engines = EngineScan::Ready(engines);
        self.selected_engine = 0;
    }

    pub fn set_cwd(&mut self, p: PathBuf) {
        self.cwd = Some(p);
    }

    /// Esc: com conteúdo digitado, ARMA a confirmação («Descartar este Agente?»); sem conteúdo
    /// (ou já armado), devolve `true` = fechar agora. (Mapa de cliques #11.)
    pub fn escape(&mut self) -> bool {
        if self.discard_armed || self.name.trim().is_empty() {
            return true;
        }
        self.discard_armed = true;
        false
    }

    /// Valida e monta o plano de CRIAÇÃO (critérios 1/3/4/5): nome leigo obrigatório, motor
    /// detectado obrigatório (estado vazio aponta o caminho de instalar — nunca beco), dedup de
    /// nome com sufixo. **Nenhum evento aqui** — quem commita é o caller com o plano.
    pub fn create_plan(&mut self) -> Result<CreatePlan, String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(COPY_ERR_EMPTY_NAME.to_string());
        }
        let engine = match &self.engines {
            EngineScan::Scanning => return Err(COPY_ERR_SCANNING.to_string()),
            EngineScan::Ready(v) if v.is_empty() => return Err(COPY_ERR_NO_ENGINE.to_string()),
            EngineScan::Ready(v) => v[self.selected_engine.min(v.len() - 1)].clone(),
        };
        // Comando final: override do Avançado (split simples; modo técnico) ou o do profile.
        let (program, args) = match &self.command {
            Some(c) if !c.trim().is_empty() => {
                let mut it = c.split_whitespace().map(str::to_string);
                let program = it.next().unwrap_or_else(|| engine.program.clone());
                (program, it.collect())
            }
            _ => (engine.program.clone(), engine.args.clone()),
        };
        let unique = unique_name(&name, &self.existing);
        let dup_note = (unique != name).then(|| copy_dup_note(&unique));
        Ok(CreatePlan {
            name: unique,
            dup_note,
            engine: AgentEngine {
                program,
                args,
                profile_id: engine.profile_id.clone(),
                label: engine.label.clone(),
            },
            cwd: self.cwd.clone(),
            role: self.role.as_ref().map(|r| r.role.clone()),
        })
    }

    /// Valida e monta o plano de SALVAR (M6-E): nome/papel valem agora; motor é F1-2-4.
    pub fn save_plan(&mut self) -> Result<SavePlan, String> {
        let ModalMode::Edit { node } = self.mode else {
            return Err("não está em modo edição".to_string());
        };
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(COPY_ERR_EMPTY_NAME.to_string());
        }
        let (orig_name, orig_role) = self.original.clone().unwrap_or_default();
        let role_now = self.role.as_ref().map(|r| r.role.clone());
        // dedup ignora o PRÓPRIO nome original (renomear para si mesmo não ganha sufixo).
        let others: Vec<String> = self
            .existing
            .iter()
            .filter(|n| **n != orig_name)
            .cloned()
            .collect();
        Ok(SavePlan {
            node,
            name: unique_name(&name, &others),
            role: (role_now != orig_role).then_some(role_now).flatten(),
        })
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
    }
}

/// Dedup leigo: nome já existe → sufixo " (2)"/" (3)"… (tabela de erros: avisa, não bloqueia).
#[must_use]
fn unique_name(name: &str, existing: &[String]) -> String {
    let taken = |n: &str| existing.iter().any(|e| e.trim() == n);
    if !taken(name) {
        return name.to_string();
    }
    for i in 2..100 {
        let candidate = format!("{name} ({i})");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{name} (∞)")
}

// ═══════════════════════════ a casca gpui (render fina) ═══════════════════════════

/// Renderiza o modal sobre o canvas. Cliques roteiam por métodos do [`WorkspaceView`]
/// (`modal_*`) — o modelo continua gpui-free.
pub fn render(
    modal: &AgentModal,
    viewport: Size<Pixels>,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let th = theme::active();
    let is_edit = matches!(modal.mode, ModalMode::Edit { .. });
    // Contenção (fix bugB2): TODA dimensão vem da matemática pura — o render só aplica.
    let win = WinSize {
        w: f32::from(viewport.width),
        h: f32::from(viewport.height),
    };
    let frame = modal_frame(win);

    let mut col = div().flex().flex_col().gap_3();

    // ── título ──
    let title = if is_edit {
        format!("{COPY_TITLE_EDIT} — {}", modal.name())
    } else {
        COPY_TITLE_CREATE.to_string()
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(th.text.bright))
                    .child(text!(title)),
            )
            .child(
                div()
                    .id("m6-close")
                    .px_2()
                    .rounded_md()
                    .text_color(rgb(th.text.muted))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_cancel(cx);
                    }))
                    .child(text!("✕")),
            ),
    );

    // ── quick-start de motores (M6-E1) ──
    match modal.engines() {
        EngineScan::Scanning => {
            col = col.child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_SCANNING)),
            );
        }
        EngineScan::Ready(v) if v.is_empty() => {
            // Estado vazio (critério 4): caminho ÚNICO de instalar — nunca quick-start vazio.
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(th.surface.panel))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(th.text.primary))
                            .child(text!(COPY_EMPTY_TITLE)),
                    )
                    .child(
                        div()
                            .id("m6-install-reco")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(th.accent.action))
                            .text_color(rgb(th.text.on_accent))
                            .cursor_pointer()
                            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                                v.modal_install_engine(cx);
                            }))
                            .child(text!(COPY_EMPTY_CTA)),
                    ),
            );
        }
        EngineScan::Ready(v) => {
            col = col.child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_QUICKSTART)),
            );
            let mut chips = div().flex().flex_row().flex_wrap().items_center().gap_2();
            for (i, e) in v.iter().enumerate() {
                let active = i == modal.selected_engine_idx();
                let mark = if active { "◉" } else { "○" };
                let reco = if e.id == "claude" {
                    " «recomendado»"
                } else {
                    ""
                };
                let mut chip = div()
                    .id(("m6-engine", i))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(if active {
                        th.surface.raised_alt
                    } else {
                        th.surface.raised
                    }))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _ev: &ClickEvent, _w, cx| {
                        view.modal_select_engine(i, cx);
                    }))
                    .child(text!(format!("{mark} {}{reco}", e.label)));
                if active {
                    chip = chip.border_2().border_color(rgb(th.focus.ring));
                }
                chips = chips.child(chip);
            }
            chips = chips.child(
                div()
                    .id("m6-install-other")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.primary))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_install_engine(cx);
                    }))
                    .child(text!(COPY_INSTALL_OTHER)),
            );
            col = col.child(chips);
        }
    }

    // (M6-E) motor travado nesta story: troca exige reinício — F1-2-4 (nunca silencioso).
    if is_edit {
        col = col.child(
            div()
                .text_color(rgb(th.state.warning))
                .child(text!(COPY_EDIT_ENGINE_LOCKED)),
        );
    }

    // ── Nome (foco do teclado; dispara o co-piloto) ──
    let (typed, typed_fg) = if modal.name().is_empty() {
        (COPY_NAME_PLACEHOLDER.to_string(), th.text.muted)
    } else {
        (format!("{}▌", modal.name()), th.text.bright)
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(120.0))
                    .text_color(rgb(th.text.primary))
                    .child(text!(COPY_NAME_LABEL)),
            )
            .child(
                div()
                    .id("m6-name")
                    .flex_1()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(th.surface.chrome))
                    .border_1()
                    .border_color(rgb(if modal.focus_field() == FocusField::Name {
                        th.focus.ring
                    } else {
                        th.surface.border
                    }))
                    .text_color(rgb(typed_fg))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_focus(FocusField::Name, cx);
                    }))
                    .child(text!(typed)),
            ),
    );

    // ── linha-fantasma do co-piloto (M6-E2) + por quê ──
    if let Some(s) = modal.suggestion() {
        let mut ghost = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("m6-ghost")
                    .text_color(rgb(th.accent.primary))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_accept_suggestion(cx);
                    }))
                    .child(text!(copy_ghost(&s.label))),
            )
            .child(
                div()
                    .id("m6-why")
                    .text_color(rgb(th.text.muted))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_toggle_why(cx);
                    }))
                    .child(text!(COPY_WHY_PREFIX)),
            );
        if modal.why_open() {
            ghost = ghost.child(
                div()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(s.why.clone())),
            );
        }
        col = col.child(ghost);
    }

    // ── cartão de Papel — M6 manda CARTÃO humano (ícone + nome + 1 frase), nunca textarea/
    //    YAML; a edição de instruções é o "Ver e ajustar" da F1-2-3. Fix de tela: 4 estados
    //    explícitos, todos com cara do que são — (a) APLICADO: read-only deliberado (sem cursor
    //    de clique); (b) PREVIEW da sugestão VIVA no campo enquanto digita (clicável: aceita);
    //    (c) co-piloto rodou e não reconheceu → fallback HONESTO (nunca silêncio); (d) vazio:
    //    copy que diz onde digitar (o nome, acima) e como escolher (Trocar).
    let role_card: AnyElement = match (modal.role(), modal.suggestion()) {
        (Some(r), _) => div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(th.surface.panel))
            .border_1()
            .border_color(rgb(th.surface.border))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(th.text.primary))
                    .child(text!(format!("✎ {}", r.label))),
            )
            .child(
                div()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(r.blurb.clone())),
            )
            .into_any_element(),
        (None, Some(s)) => div()
            .id("m6-role-preview")
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(th.surface.panel))
            .border_1()
            .border_color(rgb(th.accent.primary))
            .cursor_pointer()
            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                v.modal_accept_suggestion(cx);
            }))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(th.accent.primary))
                    .child(text!(format!("✨ {}", s.label))),
            )
            .child(
                div()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(s.blurb.clone())),
            )
            .child(
                div()
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_ROLE_SUGGESTED_HINT)),
            )
            .into_any_element(),
        (None, None) if modal.copilot_no_match() => div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(th.surface.panel))
            .text_color(rgb(th.text.secondary))
            .child(text!(COPY_ROLE_NO_MATCH))
            .into_any_element(),
        _ => div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(th.surface.panel))
            .text_color(rgb(th.text.muted))
            .child(text!(COPY_ROLE_PLACEHOLDER))
            .into_any_element(),
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_3()
            .child(
                div()
                    .w(px(120.0))
                    .text_color(rgb(th.text.primary))
                    .child(text!(COPY_ROLE_LABEL)),
            )
            .child(div().flex_1().child(role_card))
            .child(
                div()
                    .id("m6-role-swap")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_open_gallery(cx);
                    }))
                    .child(text!(COPY_ROLE_SWAP)),
            ),
    );

    // ── seletor de papéis aberto (F1-2-3): TODOS os papéis + busca + teclado ──
    // Fix bugB2: o dropdown CONTIDO — largura = útil do modal, busca FIXA no topo e lista numa
    // VIEWPORT de altura calculada (matemática pura `gallery_layout`) que rola por dentro.
    if let Some(g) = modal.gallery() {
        let filtered = g.filtered();
        let sel = g.selected();
        let lay = gallery_layout(frame, win, filtered.len());
        let (qtxt, qfg) = if g.query().is_empty() {
            (COPY_GALLERY_SEARCH_PLACEHOLDER.to_string(), th.text.muted)
        } else {
            (format!("{}▌", g.query()), th.text.bright)
        };
        // Viewport da lista: raiz de scroll em BLOCO (lição W6: flex no root de scroll não
        // engata) com altura TRAVADA pela conta; as linhas são filhas DIRETAS — é o que o
        // `scroll_to_item` do teclado indexa (child_bounds do container trackeado).
        let mut list = div()
            .id("m6-gallery-list")
            // a11y: anuncia o tamanho real da lista e, quando a viewport corta, que rola.
            .aria_label(if lay.scrolls {
                format!("lista de papéis, {} resultados — rola", filtered.len())
            } else {
                format!("lista de papéis, {} resultados", filtered.len())
            })
            .track_scroll(g.scroll_handle())
            .overflow_y_scroll()
            .h(px(lay.list_viewport_h))
            .w_full();
        if filtered.is_empty() {
            list = list.child(
                div()
                    .h(px(GALLERY_ROW_H))
                    .px_3()
                    .py_2()
                    .overflow_hidden()
                    .line_height(px(GALLERY_LINE_H))
                    .text_color(rgb(th.text.muted))
                    .child(text!(COPY_GALLERY_EMPTY)),
            );
        }
        for (i, r) in filtered.iter().enumerate() {
            let active = i == sel;
            // Geometria UNIFORME por linha: altura fixa (== GALLERY_ROW_H da conta), borda
            // sempre presente (transparente na inativa) — navegar não reflui o layout, e a
            // matemática do viewport nunca mente. a11y: ElementId ÚNICO por linha (lição:
            // text! em loop colide nó AccessKit) + rótulo anunciável completo.
            let row = div()
                .id(("m6-role-item", i))
                .aria_label(format!("papel {} — {}", r.label, r.blurb))
                .h(px(GALLERY_ROW_H))
                .mb(px(GALLERY_ROW_GAP))
                .flex()
                .flex_col()
                .px_3()
                .py_2()
                .rounded_md()
                .overflow_hidden()
                .bg(rgb(if active {
                    th.surface.raised_alt
                } else {
                    th.surface.panel
                }))
                .border_2()
                .border_color(if active {
                    // foco visível: o MESMO token do design system (≥3:1 nos 2 temas — F1-2-1).
                    rgb(th.focus.ring).into()
                } else {
                    transparent_black()
                })
                .cursor_pointer()
                .on_click(cx.listener(move |v, _ev: &ClickEvent, _w, cx| {
                    v.modal_gallery_pick(Some(i), cx);
                }))
                .child(
                    div()
                        .line_height(px(GALLERY_LINE_H))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(if active {
                            th.text.bright
                        } else {
                            th.text.primary
                        }))
                        .child(text!(r.label.clone())),
                )
                .child(
                    div()
                        .line_height(px(GALLERY_LINE_H))
                        .text_color(rgb(th.text.secondary))
                        .child(text!(r.blurb.clone())),
                );
            list = list.child(row);
        }
        col = col.child(
            div()
                // bounds FINAIS da conta: largura útil do modal × chrome+viewport — pinados.
                .w(px(lay.w))
                .h(px(lay.h))
                .flex()
                .flex_col()
                .gap_2()
                .px_3()
                .py_3()
                .rounded_md()
                .bg(rgb(th.surface.chrome))
                .border_1()
                .border_color(rgb(th.surface.raised_alt))
                .child(
                    div()
                        .h(px(GALLERY_HEADER_H))
                        .flex()
                        .flex_row()
                        .items_center()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex_1()
                                .line_height(px(GALLERY_HEADER_H))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(th.text.primary))
                                .child(text!(COPY_GALLERY_TITLE)),
                        )
                        .child(
                            div()
                                .line_height(px(GALLERY_HEADER_H))
                                .text_color(rgb(th.text.muted))
                                .child(text!(COPY_GALLERY_HINT)),
                        ),
                )
                .child(
                    div()
                        .id("m6-gallery-search")
                        .h(px(GALLERY_SEARCH_H))
                        .px_3()
                        .py_2()
                        .overflow_hidden()
                        .line_height(px(GALLERY_LINE_H))
                        .rounded_md()
                        .bg(rgb(th.surface.panel))
                        .text_color(rgb(qfg))
                        .child(text!(format!("🔎 {qtxt}"))),
                )
                .child(list),
        );
    }

    // ── Pasta de trabalho ──
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(120.0))
                    .text_color(rgb(th.text.primary))
                    .child(text!(COPY_CWD_LABEL)),
            )
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(th.text.secondary))
                    .child(text!(modal.cwd_display())),
            )
            .child(
                div()
                    .id("m6-cwd-pick")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_pick_folder(cx);
                    }))
                    .child(text!(COPY_CWD_PICK)),
            ),
    );

    // ── ▸ Avançado (divulgação progressiva — o comando NUNCA é controle primário) ──
    col = col.child(
        div()
            .id("m6-advanced")
            .text_color(rgb(th.text.muted))
            .cursor_pointer()
            .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                v.modal_toggle_advanced(cx);
            }))
            .child(text!(if modal.advanced_open() {
                COPY_ADVANCED_OPEN
            } else {
                COPY_ADVANCED
            })),
    );
    if modal.advanced_open() {
        let cmd = modal.effective_command();
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_color(rgb(th.text.muted))
                        .child(text!(COPY_CMD_LABEL)),
                )
                .child(
                    div()
                        .id("m6-cmd")
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(th.surface.chrome))
                        .border_1()
                        .border_color(rgb(if modal.focus_field() == FocusField::Command {
                            th.focus.ring
                        } else {
                            th.surface.border
                        }))
                        .font_family("Menlo")
                        .text_color(rgb(th.text.primary))
                        .cursor_pointer()
                        .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                            v.modal_focus(FocusField::Command, cx);
                        }))
                        .child(text!(format!("{cmd}▌"))),
                ),
        );
    }

    // ── erro inline (leigo) / descarte armado ──
    if let Some(e) = modal.error() {
        col = col.child(
            div()
                .text_color(rgb(th.state.danger))
                .child(text!(e.to_string())),
        );
    }
    if modal.discard_armed() {
        col = col.child(
            div()
                .text_color(rgb(th.state.warning))
                .child(text!(COPY_DISCARD_ARMED)),
        );
    }
    if is_edit {
        col = col.child(
            div()
                .text_color(rgb(th.text.muted))
                .child(text!(COPY_EDIT_APPLY_NOTE)),
        );
    }

    // ── rodapé: Cancelar · Criar/Salvar ──
    col = col.child(
        div()
            .flex()
            .flex_row()
            .justify_end()
            .gap_2()
            .child(
                div()
                    .id("m6-cancel")
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(th.surface.raised))
                    .text_color(rgb(th.text.bright))
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, _w, cx| {
                        v.modal_cancel(cx);
                    }))
                    .child(text!(COPY_CANCEL)),
            )
            .child(
                div()
                    .id("m6-create")
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(th.accent.confirm))
                    .text_color(rgb(th.text.on_accent))
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .on_click(cx.listener(|v, _ev: &ClickEvent, window, cx| {
                        v.modal_commit(window, cx);
                    }))
                    .child(text!(if is_edit { COPY_SAVE } else { COPY_CREATE })),
            ),
    );

    // backdrop + caixa central (irmão do overlay de nomeação do M2 que este modal substitui).
    // Fix bugB2: a caixa vem do `modal_frame` (clamp à janela nos 2 eixos — era `w(640)` fixo
    // sem teto). O corpo é raiz de scroll em BLOCO (lição W6): se algum estado raro exceder o
    // orçamento (`MODAL_BASE_RESERVED`), o conteúdo ROLA dentro da caixa — nunca pinta por
    // cima do canvas nem passa da borda da janela.
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(frame.w))
                .max_h(px(frame.max_h))
                .flex()
                .flex_col()
                .px_6()
                .py_5()
                .gap_2()
                .rounded_lg()
                .bg(rgb(th.surface.card))
                .border_2()
                .border_color(rgb(th.accent.create))
                .child(
                    div()
                        .id("m6-body")
                        // min_h(0): filho de flex-col tem min-height:auto — sem isto o corpo
                        // nunca encolhe e o scroll-fallback não engata.
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .w_full()
                        .child(col),
                ),
        )
        .into_any_element()
}

// ═══════════════════════════ testes (gpui-free) ═══════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::role_suggester::W31Suggester;

    fn modal() -> AgentModal {
        AgentModal::new_create(Arc::new(W31Suggester), vec![])
    }

    fn type_str(m: &mut AgentModal, s: &str) {
        for c in s.chars() {
            m.type_char(&c.to_string());
        }
    }

    fn engines_one() -> Vec<Engine> {
        vec![Engine {
            id: "claude".into(),
            label: engine_label("claude"),
            version: Some("2.1".into()),
            program: "claude".into(),
            args: vec!["--flag".into()],
            profile_id: Some("claude-code".into()),
        }]
    }

    /// M6-E2: o co-piloto dispara na PAUSA (~600ms = SUGGEST_DEBOUNCE_TICKS), nunca a cada tecla.
    #[test]
    fn suggestion_fires_after_debounce_pause() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        assert!(m.suggestion().is_none(), "sem sugestão antes da pausa");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS - 1 {
            m.tick();
        }
        assert!(m.suggestion().is_none(), "ainda dentro da pausa");
        assert!(m.tick(), "o tick da pausa produz mudança visível");
        let s = m.suggestion().expect("linha-fantasma");
        assert_eq!(s.role, "reviewer");
        assert!(!s.why.is_empty(), "transparência: o porquê vem junto");
    }

    /// Aceitar (Tab) preenche o cartão de Papel; a sugestão NUNCA se aplica sozinha (governança).
    #[test]
    fn accept_fills_role_card_never_silently() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.role().is_none(), "antes do Tab o papel está vazio");
        assert!(m.accept_suggestion());
        assert_eq!(m.role().expect("cartão").role, "reviewer");
        assert!(m.suggestion().is_none(), "ghost some após aceitar");
        // continuar digitando NÃO muda o papel aplicado sem novo aceite.
        type_str(&mut m, " Senior");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert_eq!(m.role().expect("cartão").role, "reviewer");
    }

    /// Re-sugestão só em mudança SUBSTANCIAL (papel diferente) — não a cada tecla (M6-E2).
    #[test]
    fn resuggests_only_on_substantial_change() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        m.accept_suggestion();
        // mesmo papel re-derivado → sem ghost novo (sem ruído).
        type_str(&mut m, " de PRs");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.suggestion().is_none(), "mesmo papel não re-sugere");
        // nome que muda o papel → ghost novo aparece.
        for _ in 0.."Revisor de PRs".len() {
            m.backspace();
        }
        type_str(&mut m, "Designer de Landing");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert_eq!(
            m.suggestion().expect("re-sugere").role,
            "UIUX_DESIGNER",
            "mudança substancial re-sugere"
        );
    }

    /// Critério 5: nome vazio/só-espaços → erro INLINE leigo; NENHUM plano (nenhum evento).
    #[test]
    fn empty_name_is_rejected_inline_before_commit() {
        let mut m = modal();
        m.set_engines(engines_one());
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_EMPTY_NAME);
        type_str(&mut m, "   ");
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_EMPTY_NAME);
    }

    /// Critérios 3/4: varrendo → espera leiga; zero motores → caminho de instalar (nunca beco);
    /// com motor → plano com comando do CLI Profile (programa + args do TOML).
    #[test]
    fn engine_states_guard_creation() {
        let mut m = modal();
        type_str(&mut m, "Revisor");
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_SCANNING);
        m.set_engines(vec![]);
        assert_eq!(m.create_plan().unwrap_err(), COPY_ERR_NO_ENGINE);
        m.set_engines(engines_one());
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.engine.program, "claude");
        assert_eq!(plan.engine.args, vec!["--flag".to_string()]);
        assert_eq!(plan.engine.profile_id.as_deref(), Some("claude-code"));
    }

    /// Dedup do nome (tabela de erros): sufixo automático "(2)" + aviso leve, sem bloquear.
    #[test]
    fn duplicate_name_gets_suffix_and_note() {
        let mut m = AgentModal::new_create(
            Arc::new(W31Suggester),
            vec!["Revisor".into(), "Revisor (2)".into()],
        );
        m.set_engines(engines_one());
        type_str(&mut m, "Revisor");
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.name, "Revisor (3)");
        assert!(plan.dup_note.is_some(), "aviso leve presente");
    }

    /// Critério 7 (neutralidade): os profiles são lidos do DISCO em runtime — trocar o TOML muda
    /// o comando do quick-start SEM recompilar (seed na 1ª vez; depois o arquivo manda).
    #[test]
    fn command_follows_profile_toml_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "lina-m6-profiles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // 1ª carga: semeia o claude-code.toml default e o lê do disco.
        let reg = load_profiles(&dir);
        let found = vec![DiscoveredCli {
            id: "claude".into(),
            version: None,
            path: "/usr/local/bin/claude".into(),
        }];
        let engines = engines_from(&found, &reg);
        assert_eq!(engines[0].profile_id.as_deref(), Some("claude-code"));
        assert_eq!(engines[0].program, "claude");
        let default_args = engines[0].args.clone();
        assert!(!default_args.is_empty(), "args do TOML default presentes");

        // Usuário TROCA o TOML no disco (sem recompilar) → o comando muda.
        std::fs::write(
            dir.join("claude-code.toml"),
            r#"
id = "claude-code"
program = "claude"
args = ["--minha-flag-custom"]
delivery = "pty_inject"
prompt_ready_regex = '.'
[end_signal]
kind = "idle"
"#,
        )
        .expect("escrever TOML custom");
        let reg2 = load_profiles(&dir);
        let engines2 = engines_from(&found, &reg2);
        assert_eq!(
            engines2[0].args,
            vec!["--minha-flag-custom".to_string()],
            "trocar o TOML mudou o comando sem recompilar"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Esc com texto ARMA a confirmação de descarte; o 2º Esc fecha; sem texto fecha direto.
    #[test]
    fn escape_arms_discard_confirmation() {
        let mut m = modal();
        assert!(m.escape(), "vazio fecha direto");
        let mut m = modal();
        type_str(&mut m, "Revisor");
        assert!(!m.escape(), "1º Esc arma, não fecha");
        assert!(m.discard_armed());
        assert!(m.escape(), "2º Esc descarta");
        // digitar desarma (o usuário desistiu de descartar).
        let mut m = modal();
        type_str(&mut m, "Rev");
        m.escape();
        m.type_char("i");
        assert!(!m.discard_armed(), "digitar desarma o descarte");
    }

    /// **GUARDIÃO DO SELETOR (F1-2-3 + feedback do fundador: "dropdown, não ciclar")**:
    /// [Trocar] abre a biblioteca com TODOS os papéis visíveis de uma vez (nome leigo + 1 frase,
    /// nenhum jargão); a busca filtra por nome OU descrição; escolher aplica NA HORA, pina (a
    /// re-sugestão não atropela) e fecha o seletor.
    #[test]
    fn gallery_shows_all_roles_at_once_search_and_pick() {
        let mut m = modal();
        m.open_gallery();
        let total = crate::role_suggester::role_registry()
            .expect("registry")
            .role_names()
            .count();
        {
            let g = m.gallery().expect("seletor aberto");
            let all = g.filtered();
            assert_eq!(all.len(), total, "TODOS os papéis da biblioteca de uma vez");
            assert!(
                all.len() >= 10,
                "biblioteca real, não amostra: {}",
                all.len()
            );
            for r in &all {
                assert!(!r.label.contains('_'), "rótulo leigo: {}", r.label);
                assert!(!r.blurb.is_empty(), "1 frase por papel: {}", r.label);
            }
        }
        // busca por TEXTO (nome leigo)…
        m.gallery_type("revis");
        assert_eq!(m.gallery().unwrap().filtered().len(), 1, "busca estreita");
        // …e escolher aplica na hora + pina + fecha.
        assert!(m.gallery_pick(None));
        assert!(!m.gallery_open(), "escolher fecha o seletor");
        assert_eq!(m.role().expect("papel aplicado").role, "reviewer");
        type_str(&mut m, "Designer de Landing");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert_eq!(
            m.role().unwrap().role,
            "reviewer",
            "pinado não troca sozinho"
        );
        // busca por DESCRIÇÃO também acha (ex.: "telas" → Especialista em telas).
        m.open_gallery();
        m.gallery_type("telas");
        assert!(
            m.gallery()
                .unwrap()
                .filtered()
                .iter()
                .any(|r| r.role == "FRONTEND"),
            "busca pela descrição/função acha o papel"
        );
    }

    /// Teclado do seletor (a11y do gate): ↑↓ navega com clamp nas bordas, Enter escolhe o
    /// destacado, Esc fecha SEM aplicar; busca sem resultado → Enter não aplica nada.
    #[test]
    fn gallery_keyboard_navigation_and_escape() {
        let mut m = modal();
        m.open_gallery();
        m.gallery_move(-1);
        assert_eq!(m.gallery().unwrap().selected(), 0, "clamp no topo");
        m.gallery_move(1);
        m.gallery_move(1);
        assert_eq!(m.gallery().unwrap().selected(), 2);
        let expected = m.gallery().unwrap().filtered()[2].role.clone();
        assert!(m.gallery_pick(None), "Enter escolhe o destacado");
        assert_eq!(m.role().unwrap().role, expected);
        // Esc fecha sem aplicar.
        let mut m2 = modal();
        m2.open_gallery();
        m2.gallery_close();
        assert!(!m2.gallery_open());
        assert!(m2.role().is_none(), "Esc não aplica nada");
        // busca sem resultados → Enter é no-op (não aplica fantasma).
        let mut m3 = modal();
        m3.open_gallery();
        m3.gallery_type("zzz-nada");
        assert!(m3.gallery().unwrap().filtered().is_empty());
        assert!(!m3.gallery_pick(None), "sem resultado, nada a escolher");
        assert!(
            m3.gallery_open(),
            "seletor segue aberto p/ corrigir a busca"
        );
    }

    /// **GUARDIÃO (fix de tela bugB2 — dropdown estourava modal e janela)**: a caixa do modal
    /// SEMPRE cabe na janela — largura de projeto (640) só quando há espaço; janela estreita
    /// (o screenshot do fundador, ~736 lógicos) clampa com folga de respiro nas duas bordas.
    #[test]
    fn modal_frame_clamps_to_window() {
        // janela folgada → largura de projeto + teto de altura com margem dos dois lados.
        let f = modal_frame(WinSize {
            w: 1440.0,
            h: 900.0,
        });
        assert!((f.w - MODAL_W).abs() < 0.01, "largura de projeto: {}", f.w);
        assert!(
            (f.max_h - (900.0 - 2.0 * WINDOW_MARGIN)).abs() < 0.01,
            "teto = janela - 2*margem: {}",
            f.max_h
        );
        // janela estreita → a caixa encolhe (era o estouro horizontal da tela do fundador).
        let f = modal_frame(WinSize { w: 700.0, h: 800.0 });
        assert!(
            f.w <= 700.0 - 2.0 * WINDOW_MARGIN,
            "caixa dentro da janela estreita: {}",
            f.w
        );
        assert!(f.w >= MODAL_W_MIN, "nunca degenera abaixo do mínimo");
    }

    /// O dropdown NUNCA é mais largo que a largura útil do modal (requisito 1 do fundador) —
    /// em qualquer janela, a largura vem do frame, não do conteúdo.
    #[test]
    fn gallery_never_wider_than_modal_usable_width() {
        for win in [
            WinSize {
                w: 1440.0,
                h: 900.0,
            },
            WinSize { w: 700.0, h: 800.0 },
            WinSize { w: 420.0, h: 600.0 },
        ] {
            let frame = modal_frame(win);
            let lay = gallery_layout(frame, win, 14);
            assert!(
                (lay.w - (frame.w - 2.0 * MODAL_PAD_X)).abs() < 0.01,
                "largura útil exata do modal em {win:?}: {} vs frame {}",
                lay.w,
                frame.w
            );
        }
    }

    /// Requisito 2: lista longa (8+ papéis) → viewport LIMITADA com scroll interno; o dropdown
    /// inteiro (busca + viewport) cabe no orçamento vertical do modal dentro da janela.
    #[test]
    fn gallery_long_list_scrolls_inside_window_budget() {
        let win = WinSize {
            w: 1440.0,
            h: 900.0,
        };
        let frame = modal_frame(win);
        let lay = gallery_layout(frame, win, 14);
        let content = 14.0 * GALLERY_ROW_H + 13.0 * GALLERY_ROW_GAP;
        assert!(
            lay.list_viewport_h < content,
            "viewport menor que o conteúdo: {} < {content}",
            lay.list_viewport_h
        );
        assert!(lay.scrolls, "lista longa rola por dentro");
        assert!(
            MODAL_BASE_RESERVED + lay.h <= frame.max_h + 0.01,
            "modal + dropdown cabem no teto: {} + {} > {}",
            MODAL_BASE_RESERVED,
            lay.h,
            frame.max_h
        );
    }

    /// Poucos itens → viewport JUSTA (sem buraco morto) e sem scroll.
    #[test]
    fn gallery_few_items_fit_exactly_without_scroll() {
        let win = WinSize {
            w: 1440.0,
            h: 900.0,
        };
        let lay = gallery_layout(modal_frame(win), win, 3);
        let content = 3.0 * GALLERY_ROW_H + 2.0 * GALLERY_ROW_GAP;
        assert!(
            (lay.list_viewport_h - content).abs() < 0.01,
            "viewport == conteúdo: {} vs {content}",
            lay.list_viewport_h
        );
        assert!(!lay.scrolls);
    }

    /// Busca sem resultado (n=0) → viewport de UMA linha (a mensagem de vazio), sem scroll.
    #[test]
    fn gallery_empty_search_keeps_single_row_viewport() {
        let win = WinSize {
            w: 1440.0,
            h: 900.0,
        };
        let lay = gallery_layout(modal_frame(win), win, 0);
        assert!(
            (lay.list_viewport_h - GALLERY_ROW_H).abs() < 0.01,
            "1 linha p/ a mensagem de vazio: {}",
            lay.list_viewport_h
        );
        assert!(!lay.scrolls);
    }

    /// Janela DEGENERADA (menor que o orçamento base do modal): a viewport mantém o piso de
    /// 1 linha (usabilidade) — o transbordo residual é absorvido pelo scroll do corpo do modal
    /// (fallback declarativo no render), nunca pintado por cima do canvas.
    #[test]
    fn gallery_degenerate_window_keeps_one_row_floor() {
        let win = WinSize { w: 400.0, h: 300.0 };
        let lay = gallery_layout(modal_frame(win), win, 14);
        assert!(
            lay.list_viewport_h >= GALLERY_ROW_H - 0.01,
            "piso de 1 linha: {}",
            lay.list_viewport_h
        );
        assert!(lay.scrolls, "14 itens numa linha só → rola");
    }

    /// **Varredura de invariantes** (janelas × nº de itens): largura sempre a útil do modal;
    /// viewport sempre ≥ piso e ≤ máx de conforto; `scrolls` ⟺ conteúdo > viewport; altura do
    /// dropdown = chrome + viewport; e em janela não-degenerada (altura ≥ 700 = orçamento base
    /// + chrome + piso de 1 linha + margens) o modal INTEIRO respeita o teto — o bug não volta.
    #[test]
    fn gallery_containment_invariants_sweep() {
        for w in [360.0_f32, 640.0, 736.0, 1024.0, 1440.0, 2560.0] {
            for h in [300.0_f32, 600.0, 640.0, 700.0, 768.0, 900.0, 1200.0, 1600.0] {
                for n in [0_usize, 1, 3, 8, 14, 40] {
                    let win = WinSize { w, h };
                    let frame = modal_frame(win);
                    let lay = gallery_layout(frame, win, n);
                    let content = if n == 0 {
                        GALLERY_ROW_H
                    } else {
                        n as f32 * GALLERY_ROW_H + (n as f32 - 1.0) * GALLERY_ROW_GAP
                    };
                    assert!(
                        (lay.w - (frame.w - 2.0 * MODAL_PAD_X)).abs() < 0.01,
                        "largura útil em {w}x{h}/n={n}"
                    );
                    assert!(
                        lay.list_viewport_h >= GALLERY_ROW_H.min(content) - 0.01,
                        "piso da viewport em {w}x{h}/n={n}: {}",
                        lay.list_viewport_h
                    );
                    assert!(
                        lay.list_viewport_h <= GALLERY_VIEWPORT_MAX.max(GALLERY_ROW_H) + 0.01,
                        "máx de conforto em {w}x{h}/n={n}: {}",
                        lay.list_viewport_h
                    );
                    assert_eq!(
                        lay.scrolls,
                        content > lay.list_viewport_h + 0.5,
                        "scrolls ⟺ conteúdo > viewport em {w}x{h}/n={n}"
                    );
                    assert!(
                        (lay.h - (gallery_chrome_h() + lay.list_viewport_h)).abs() < 0.01,
                        "altura = chrome + viewport em {w}x{h}/n={n}"
                    );
                    if h >= 700.0 {
                        assert!(
                            MODAL_BASE_RESERVED + lay.h <= frame.max_h + 0.01,
                            "contenção estrita em {w}x{h}/n={n}: {} + {} > {}",
                            MODAL_BASE_RESERVED,
                            lay.h,
                            frame.max_h
                        );
                    }
                }
            }
        }
    }

    /// As constantes da matemática SÃO as do render (fonte única): linha da lista = 2 linhas de
    /// texto com line_height explícita (lição: a natural da fonte clipa) + py_2 + borda_2 dos
    /// dois lados (geometria uniforme — a linha inativa usa borda transparente).
    #[test]
    fn gallery_row_math_matches_render_constants() {
        assert!(
            (GALLERY_ROW_H - (2.0 * GALLERY_LINE_H + 2.0 * 8.0 + 2.0 * 2.0)).abs() < 0.01,
            "linha = 2*line_height + py_2 + border_2: {GALLERY_ROW_H}"
        );
        assert!(
            (gallery_chrome_h()
                - (2.0 * GALLERY_BORDER
                    + 2.0 * GALLERY_PAD
                    + 2.0 * GALLERY_GAP
                    + GALLERY_HEADER_H
                    + GALLERY_SEARCH_H))
                .abs()
                < 0.01,
            "chrome = bordas + paddings + gaps + cabeçalho + busca"
        );
    }

    /// **Critério 3 da F1-2-3 (auditoria de deps)**: NENHUM client de LLM/rede no app — a
    /// sugestão é heurística local (W3-1), offline por construção (invariantes #1/#2).
    #[test]
    fn no_llm_or_network_client_deps() {
        let manifest = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        )
        .expect("Cargo.toml");
        for forbidden in [
            "openai",
            "anthropic",
            "genai",
            "llm",
            "reqwest",
            "hyper",
            "ureq",
            "curl",
        ] {
            assert!(
                !manifest.lines().any(|l| {
                    let l = l.trim();
                    !l.starts_with('#') && l.starts_with(forbidden)
                }),
                "dep proibida no app (LLM/rede): {forbidden}"
            );
        }
    }

    /// M6-E (modo edição): pré-preenchido; Salvar detecta mudança de nome e papel; renomear p/
    /// o PRÓPRIO nome não ganha sufixo de dedup.
    #[test]
    fn edit_mode_save_plan_detects_changes() {
        let node = uuid::Uuid::now_v7();
        let mut m = AgentModal::new_edit(
            Arc::new(W31Suggester),
            node,
            "Ajudante",
            Some("WRITER"),
            vec!["Ajudante".into(), "Outro".into()],
        );
        assert_eq!(m.role().unwrap().role, "WRITER");
        // sem mudanças: plano preserva nome, papel None (não mudou).
        let p = m.save_plan().expect("plano");
        assert_eq!(p.name, "Ajudante");
        assert_eq!(p.role, None);
        // muda papel via SELETOR → plano carrega papel novo.
        m.open_gallery();
        m.gallery_move(1); // um papel diferente do atual
        assert!(m.gallery_pick(None));
        let role_now = m.role().unwrap().role.clone();
        let p = m.save_plan().expect("plano");
        assert_eq!(p.role.as_deref(), Some(role_now.as_str()));
    }

    /// **GUARDIÃO (fix de tela)**: nome SEM padrão conhecido ("Teles", o caso do fundador) →
    /// o co-piloto roda e o campo mostra o fallback HONESTO (`copilot_no_match`), nunca
    /// silêncio. Nome conhecido depois → o fallback dá lugar à sugestão viva no campo.
    #[test]
    fn unknown_name_shows_honest_fallback_never_silence() {
        let mut m = modal();
        type_str(&mut m, "Teles");
        assert!(!m.copilot_no_match(), "antes da pausa, sem veredito");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.suggestion().is_none(), "'Teles' não tem papel conhecido");
        assert!(
            m.copilot_no_match(),
            "rodou e não reconheceu → fallback honesto NO CAMPO (nunca silêncio)"
        );
        // nome que sugere → o fallback sai e a sugestão viva entra no campo.
        for _ in 0.."Teles".len() {
            m.backspace();
        }
        type_str(&mut m, "Revisor");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m.tick();
        }
        assert!(m.suggestion().is_some(), "sugestão viva no campo");
        assert!(!m.copilot_no_match(), "fallback deu lugar à sugestão");
        // aceitar (clique no preview/Tab) limpa tudo e aplica.
        assert!(m.accept_suggestion());
        assert!(!m.copilot_no_match());
        assert_eq!(m.role().unwrap().role, "reviewer");
        // [Trocar] também limpa o no_match (escolha manual é resposta válida ao fallback).
        let mut m2 = modal();
        type_str(&mut m2, "Teles");
        for _ in 0..SUGGEST_DEBOUNCE_TICKS {
            m2.tick();
        }
        assert!(m2.copilot_no_match());
        m2.open_gallery();
        assert!(m2.gallery_pick(None));
        assert!(!m2.copilot_no_match(), "Trocar resolve o fallback");
        assert!(m2.role().is_some());
    }

    /// Critério 2 (auditoria de copy): NENHUM estado do modal mostra YAML/TOML/"profile"/
    /// "terminal" — o leigo nunca vê jargão (anti-padrões 1/2/5 do fluxo (a)).
    #[test]
    fn copy_has_no_jargon() {
        let dynamic = [
            copy_ghost("Revisor"),
            copy_dup_note("Revisor (2)"),
            engine_label("claude"),
            engine_label("agy"),
        ];
        let all: Vec<&str> = [
            COPY_TITLE_CREATE,
            COPY_TITLE_EDIT,
            COPY_QUICKSTART,
            COPY_SCANNING,
            COPY_EMPTY_TITLE,
            COPY_EMPTY_CTA,
            COPY_INSTALL_OTHER,
            COPY_NAME_LABEL,
            COPY_NAME_PLACEHOLDER,
            COPY_ROLE_LABEL,
            COPY_ROLE_PLACEHOLDER,
            COPY_ROLE_NO_MATCH,
            COPY_ROLE_SUGGESTED_HINT,
            COPY_ROLE_SWAP,
            COPY_CWD_LABEL,
            COPY_CWD_DEFAULT,
            COPY_CWD_PICK,
            COPY_ADVANCED,
            COPY_ADVANCED_OPEN,
            COPY_CMD_LABEL,
            COPY_CREATE,
            COPY_SAVE,
            COPY_CANCEL,
            COPY_ERR_EMPTY_NAME,
            COPY_ERR_SCANNING,
            COPY_ERR_NO_ENGINE,
            COPY_DISCARD_ARMED,
            COPY_EDIT_ENGINE_LOCKED,
            COPY_EDIT_APPLY_NOTE,
            COPY_WHY_PREFIX,
        ]
        .into_iter()
        .chain(dynamic.iter().map(String::as_str))
        .collect();
        for s in all {
            let low = s.to_lowercase();
            for jargon in ["yaml", "toml", "profile", "terminal", "frontmatter"] {
                assert!(
                    !low.contains(jargon),
                    "jargão {jargon:?} na copy do modal: {s:?}"
                );
            }
        }
    }

    /// Comando custom no Avançado: o override entra no plano (programa + args re-parseados);
    /// trocar o motor selecionado descarta o override (o comando segue o motor).
    #[test]
    fn advanced_command_override_enters_plan() {
        let mut m = modal();
        m.set_engines(engines_one());
        type_str(&mut m, "Revisor");
        m.toggle_advanced();
        m.set_focus(FocusField::Command);
        // limpa o derivado e digita um comando custom.
        for _ in 0..m.effective_command().len() {
            m.backspace();
        }
        type_str(&mut m, "meu-cli --x");
        let plan = m.create_plan().expect("plano");
        assert_eq!(plan.engine.program, "meu-cli");
        assert_eq!(plan.engine.args, vec!["--x".to_string()]);
        // trocar o motor reseta o override.
        m.select_engine(0);
        assert_eq!(m.effective_command(), "claude --flag");
    }
}
