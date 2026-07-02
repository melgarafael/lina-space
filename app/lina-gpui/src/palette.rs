//! **W4-2 · M1 — PALETA DE COMANDOS (Cmd-K)** sobre o canvas T4. O MODELO (estado + fuzzy-match +
//! navegação por teclado) é PURO e testável (sem gpui); só o `render` consome gpui. As AÇÕES são
//! genéricas ([`PaletteAction`]) — quem as executa é o `WorkspaceView` (toca `naming`/`focus`/`brake`),
//! para manter a paleta desacoplada do canvas.
//!
//! UX: **Cmd-K abre** (com a lista de comandos do momento) · **digitar filtra** (subsequência) · **↑↓
//! navega** · **Enter executa** a seleção · **Esc fecha**.
//!
//! **F2-2-4 — porta visível + ranking previsível.** A paleta deixou de ser só ⌘K: o `sidebar.rs`
//! ganhou um botão ROTULADO ("Buscar comandos · ⌘K") que abre esta mesma paleta — materializando o
//! invariante de fase **"nada existe só atrás de atalho"** (paleta escondida = paleta morta, caso
//! GitHub auditado em D4-A1). E o filtro virou um **ranking determinístico em tiers** (D4-A2, modelo
//! Zed), nesta ordem fixa — para quem digita NUNCA se surpreender:
//!
//! 1. **alias estrito por prefixo** — sinônimos técnicos como keywords invisíveis ("webhook" acha "Gatilho"); um prefixo do rótulo ou de um alias sobe ao topo;
//! 2. **MRU curto e estável** — o que você usou por último (persistido pelo shell);
//! 3. **fuzzy smart-case** — a subsequência de sempre (case-sensitive só se você digitar maiúscula);
//! 4. **hit-count** — desempate por popularidade, **projeção do event log** (costura `events.rs`; injetado via [`PaletteState::set_hits`] — o modelo NUNCA fabrica contagem).
//!
//! Tudo PURO e testável sem gpui (ver `tests`). **Filtro por contexto** (padrão Zed
//! `CommandPaletteFilter`): ações inaplicáveis ao estado atual saem da lista (não acinzentadas) —
//! o shell marca [`Command::when`] e o ranking as descarta.

use std::collections::HashMap;

use gpui::{div, prelude::*, px, rgb, text, AnyElement};
use lina_host::{NodeId, NodeKind, NodeStatus};

use crate::ui::RadiusExt;

/// O que a paleta dispara. Genérico: o `WorkspaceView` executa (abre o modal M6, foca um
/// nó, alterna o freio, ou — placeholder até `creators.rs` — cria nota/pasta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    /// F1-2-2 (M6): abre o modal "Novo Agente" (evoluiu o modo-nomeação do M2).
    NewAgent,
    /// F4-0-2 (UI): abre o modal "Conectar um canal" — upload de credencial p/ o cofre.
    ConnectChannel,
    /// F4-1-2-UI: abre o modal "Conectar seu WhatsApp" — QR + pareamento (canal concreto).
    ConnectWhatsApp,
    /// F4-WA-2c (UI): abre o modal "Receber um aviso de fora" — liga um webhook a um terminal vivo.
    ConfigureWebhook,
    /// F1-2-2 (M6-E): abre o modal em modo EDITAR para um nó vivo.
    EditAgent(NodeId),
    /// Foca (e revela) um nó do canvas.
    FocusNode(NodeId),
    /// F2-2-5 (aditivo): leva 1 clique até a DECISÃO na fila de atenção (foca o nó + abre a
    /// superfície 🔔 com Aprovar/Recusar). **NAVEGA, nunca decide** — a autorização tem autoridade
    /// única na fila (doutrina de segurança); a toolbar só encurta o caminho até o gate.
    GotoAtencao(NodeId),
    /// F2-2-5 (aditivo): encerra o nó (mata o processo do CLI) — reusa o caminho do ✕ do header, não
    /// cria um novo. Vira ação de registry para ganhar paridade mouse(toolbar)/teclado(paleta).
    CloseNode(NodeId),
    /// W4-3: alterna o freio (pausa/retoma a auto-orquestração).
    ToggleBrake,
    /// F1-2-1 (fix de tela): abre a janela de Ajustes (T7 — Aparência: tema/acento, Espaços, T8).
    OpenSettings,
    /// M3 (placeholder até `creators.rs`): nova nota.
    NewNote,
    /// M4 (placeholder até `creators.rs`): nova pasta.
    NewFolder,
    /// F1-1-5 (P6/fluxo c): abre/fecha o painel "Atividade e custos" do Espaço.
    ToggleDashboard,
}

/// Comandos BASE da paleta (independentes do roster) — puro p/ o guardião de entry points.
/// O rótulo de Ajustes carrega os termos que o T7§A manda a paleta achar: "tema", "cor",
/// "aparência", "claro/escuro" (fix de tela: o tema era INALCANÇÁVEL sem env de dev — inv#6).
#[must_use]
pub fn base_commands() -> Vec<Command> {
    vec![
        Command::new("✦ Novo agente", PaletteAction::NewAgent)
            .with_aliases(&["terminal", "criar", "novo", "time", "colega", "ia"]),
        // F4-1-2-UI: o canal concreto — "Conectar seu WhatsApp" (QR + pareamento). Vem ANTES do canal
        // genérico para que "whatsapp"/"zap" caiam aqui (a tela com QR), não no upload de credencial cru.
        Command::new("📱 Conectar seu WhatsApp", PaletteAction::ConnectWhatsApp).with_aliases(&[
            "whatsapp", "zap", "wpp", "qr", "celular", "conectar", "mensagem",
        ]),
        Command::new("🔌 Conectar um canal", PaletteAction::ConnectChannel).with_aliases(&[
            "canal",
            "credencial",
            "chave",
            "senha",
            "email",
            "integração",
        ]),
        // F4-WA-2c: o leigo digita "webhook"/"aviso"/"gatilho" e acha esta tela (alias = keyword
        // invisível). "Receber um aviso de fora" = ligar um evento externo a um terminal vivo.
        Command::new(
            "📩 Receber um aviso de fora",
            PaletteAction::ConfigureWebhook,
        )
        .with_aliases(&[
            "webhook",
            "aviso",
            "gatilho",
            "trigger",
            "evento",
            "notificação",
            "automação",
            "receber",
        ]),
        Command::new(
            "🎨 Aparência: tema e cores (claro/escuro) — Ajustes",
            PaletteAction::OpenSettings,
        )
        .with_aliases(&[
            "configuração",
            "preferências",
            "settings",
            "acento",
            "fonte",
        ]),
        Command::new(
            "⏸ Pausar / retomar orquestração (freio)",
            PaletteAction::ToggleBrake,
        )
        .with_aliases(&["parar", "pausar", "retomar", "brake", "congelar"]),
        Command::new("📝 Nova nota", PaletteAction::NewNote).with_aliases(&[
            "documento",
            "texto",
            "anotação",
            "markdown",
        ]),
        Command::new("📁 Nova pasta", PaletteAction::NewFolder).with_aliases(&[
            "diretório",
            "folder",
            "organizar",
        ]),
        // F1-1-5 (entry point descobrível do P6 — fluxo c): "dashboard", "atividade",
        // "custo" são os termos que um leigo digita.
        Command::new(
            "📊 Dashboard: atividade e custos do time",
            PaletteAction::ToggleDashboard,
        )
        .with_aliases(&[
            "gasto",
            "dinheiro",
            "preço",
            "consumo",
            "painel",
            "métricas",
        ]),
    ]
}

/// **Contexto de UM nó focado** — a entrada PURA de [`node_commands`]. O shell (costura `main.rs`)
/// preenche a partir do estado vivo; o seletor decide as ações SEM tocar gpui (testável). `needs_human`
/// é a MESMA fonte da fila de atenção (a toolbar LÊ o sinal, não o computa — spec §2.1).
#[derive(Debug, Clone, Copy)]
pub struct NodeCtx {
    pub id: NodeId,
    pub kind: NodeKind,
    pub status: NodeStatus,
    /// Gate de custódia/permissão pendente para ESTE nó (`needs_human` da projeção de atenção).
    pub needs_human: bool,
    /// Quantos nós há no Espaço — Encerrar exige `> 1` (1 clique nunca esvazia o Espaço).
    pub roster_len: usize,
}

impl NodeCtx {
    /// Vivo = processo de pé: qualquer status MENOS `Dead` (`Crashed` = painel quebrou, processo vive).
    #[must_use]
    fn is_alive(&self) -> bool {
        !matches!(self.status, NodeStatus::Dead)
    }

    /// "Agente/terminal" no vocabulário da spec = o nó-CLI (`NodeKind::Terminal`). Nota/Pasta/etc não.
    #[must_use]
    fn is_agent(&self) -> bool {
        matches!(self.kind, NodeKind::Terminal)
    }
}

/// **O SELETOR ÚNICO de ações-de-nó (F2-2-5).** Uma fonte, DUAS superfícies: a paleta dinâmica
/// (teclado) e a toolbar contextual (mouse) consomem ESTE registry — paridade por construção (regra
/// anti-Zed: nenhuma ação só atrás de atalho). Ordem fixa (spec §2.2): primária → reversíveis →
/// destrutiva por último. Cada ação carrega seu [`Command::when`]; o seletor JÁ descarta as
/// inaplicáveis — então "pede-aprovação" devolve 4, os demais estados 3. Rótulos placeholder
/// (R9/@Redator reconcilia as `COPY_TB_*`); todo `Command` tem label NÃO-vazio (anti icon-only, D4-A3).
#[must_use]
pub fn node_commands(ctx: &NodeCtx) -> Vec<Command> {
    use PaletteAction as A;
    [
        // Primária (cor=significado): só quando o nó PEDE você — e NAVEGA até a decisão, não decide.
        Command::new("⚑ Atender", A::GotoAtencao(ctx.id)).when(ctx.needs_human),
        // Reversíveis: Editar só em nó-agente vivo; Centralizar sempre.
        Command::new("✎ Editar", A::EditAgent(ctx.id)).when(ctx.is_agent() && ctx.is_alive()),
        Command::new("⤢ Centralizar", A::FocusNode(ctx.id)),
        // Destrutiva por último (padrão de rodapé) — nunca deixa o Espaço sem nenhum nó.
        Command::new("✕ Encerrar", A::CloseNode(ctx.id)).when(ctx.roster_len > 1),
    ]
    .into_iter()
    .filter(|c| c.enabled) // o `when` vira filtro AQUI: a superfície recebe só o aplicável
    .collect()
}

/// Um comando listável: rótulo (o que o humano LÊ) + `aliases` (keywords INVISÍVEIS que o humano
/// pode DIGITAR — sinônimos técnicos: "webhook"→"Gatilho") + ação + `enabled` (filtro por contexto).
#[derive(Debug, Clone)]
pub struct Command {
    pub label: String,
    pub action: PaletteAction,
    /// Termos extras que casam a query mas NÃO aparecem no rótulo (D4-A2: o leigo digita o sinônimo
    /// que conhece; o alias estrito por prefixo é o tier 1 do ranking).
    pub aliases: Vec<String>,
    /// Filtro por contexto (padrão Zed `CommandPaletteFilter`): `false` = inaplicável ao estado
    /// atual → SAI da lista (não acinzentado). Default `true`; o shell rebaixa com [`Self::when`].
    pub enabled: bool,
}

impl Command {
    pub fn new(label: impl Into<String>, action: PaletteAction) -> Self {
        Self {
            label: label.into(),
            action,
            aliases: Vec::new(),
            enabled: true,
        }
    }

    /// Anexa keywords invisíveis (sinônimos). Builder — encadeia após `new`.
    #[must_use]
    pub fn with_aliases(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// Filtro por contexto: `when(false)` tira o comando da lista enquanto a condição não vale
    /// (ex.: "Editar Agente" só com um Agente vivo selecionado). Builder. **Consumidor real:
    /// [`node_commands`] (F2-2-5)** — o `allow(dead_code)` prometido em r4 cai aqui.
    #[must_use]
    pub fn when(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// **Identidade ESTÁVEL** do comando para MRU e hit-count — derivada da AÇÃO (não do rótulo, que
    /// muda com i18n/emoji). Ações com nó carregam o id, para o MRU distinguir "Focar: A" de
    /// "Focar: B". É a chave que o event log projeta em contagem de uso (costura `events.rs`).
    #[must_use]
    pub fn key(&self) -> String {
        action_key(&self.action)
    }
}

/// Chave estável por variante de [`PaletteAction`] (ver [`Command::key`]).
#[must_use]
fn action_key(action: &PaletteAction) -> String {
    use PaletteAction as A;
    match action {
        A::NewAgent => "new_agent".to_owned(),
        A::ConnectChannel => "connect_channel".to_owned(),
        A::ConnectWhatsApp => "connect_whatsapp".to_owned(),
        A::ConfigureWebhook => "configure_webhook".to_owned(),
        A::EditAgent(id) => format!("edit_agent:{id}"),
        A::FocusNode(id) => format!("focus_node:{id}"),
        A::GotoAtencao(id) => format!("goto_atencao:{id}"),
        A::CloseNode(id) => format!("close_node:{id}"),
        A::ToggleBrake => "toggle_brake".to_owned(),
        A::OpenSettings => "open_settings".to_owned(),
        A::NewNote => "new_note".to_owned(),
        A::NewFolder => "new_folder".to_owned(),
        A::ToggleDashboard => "toggle_dashboard".to_owned(),
    }
}

/// Quantos comandos o MRU lembra (curto e estável — D4-A2: uma lista longa deixa de ser previsível).
const MRU_CAP: usize = 8;

/// **A classe de casamento** de um comando contra a query — o tier 1 do ranking. `Prefix` (a query é
/// prefixo do rótulo OU de um alias) vem ANTES de `Fuzzy` (subsequência espalhada). Ordem do derive
/// = ordem de prioridade (menor = melhor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchClass {
    Prefix,
    Fuzzy,
}

/// **Smart-case:** se a query tem QUALQUER maiúscula, casa case-SENSITIVE (o humano foi específico);
/// senão, case-insensitive. Norma comum a prefixo e subsequência.
fn fold(query: &str, haystack: &str) -> (String, String) {
    if query.chars().any(char::is_uppercase) {
        (query.to_owned(), haystack.to_owned())
    } else {
        (query.to_lowercase(), haystack.to_lowercase())
    }
}

/// A query (smart-case) é PREFIXO do haystack?
fn is_prefix(query: &str, haystack: &str) -> bool {
    let (q, h) = fold(query, haystack);
    h.starts_with(&q)
}

/// A query (smart-case) é SUBSEQUÊNCIA do haystack? Mesma norma de caixa do prefixo (via [`fold`]),
/// para o smart-case valer inteiro — prefixo e fuzzy nunca discordam sobre maiúsculas.
fn is_subsequence(query: &str, haystack: &str) -> bool {
    let (q, h) = fold(query, haystack);
    if q.is_empty() {
        return true;
    }
    let mut needle = q.chars().peekable();
    for hc in h.chars() {
        match needle.peek() {
            Some(&qc) if qc == hc => {
                needle.next();
            }
            Some(_) => {}
            None => break,
        }
    }
    needle.peek().is_none()
}

/// A melhor [`MatchClass`] do comando contra a query (rótulo + aliases), ou `None` se não casa.
/// Query vazia → `Fuzzy` (casa tudo; o MRU/hit-count é que ordena a lista default).
fn match_class(query: &str, cmd: &Command) -> Option<MatchClass> {
    if query.is_empty() {
        return Some(MatchClass::Fuzzy);
    }
    let haystacks =
        std::iter::once(cmd.label.as_str()).chain(cmd.aliases.iter().map(String::as_str));
    let mut best: Option<MatchClass> = None;
    for h in haystacks {
        if is_prefix(query, h) {
            return Some(MatchClass::Prefix); // melhor possível — corta cedo
        }
        if is_subsequence(query, h) {
            best = Some(MatchClass::Fuzzy);
        }
    }
    best
}

/// **Estado da paleta** (vive no `WorkspaceView`). Fechada por padrão.
#[derive(Debug, Default)]
pub struct PaletteState {
    open: bool,
    query: String,
    /// Índice da seleção DENTRO da lista FILTRADA.
    selected: usize,
    commands: Vec<Command>,
    /// MRU — chaves dos comandos usados, MAIS RECENTE PRIMEIRO (tier 2 do ranking). SOBREVIVE a
    /// abrir/fechar (não é limpo em `close`/`open`); o shell o carrega 1× no boot ([`Self::set_mru`])
    /// e o persiste após cada escolha ([`Self::mru`]).
    mru: Vec<String>,
    /// Hit-count por chave — **projeção do event log** (costura `events.rs`), injetada a cada
    /// abertura ([`Self::set_hits`]). Desempate final (tier 4). Vazio = sem dados ⇒ não desempata.
    hits: HashMap<String, u32>,
}

impl PaletteState {
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Abre com a lista de comandos do MOMENTO (rebuild a cada abertura — o roster muda).
    pub fn open(&mut self, commands: Vec<Command>) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.commands = commands;
    }

    /// Fecha e limpa o estado (não retém os comandos).
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.commands.clear();
    }

    /// Carrega o MRU persistido (shell, 1× no boot). Trunca ao [`MRU_CAP`].
    pub fn set_mru(&mut self, mru: Vec<String>) {
        self.mru = mru;
        self.mru.truncate(MRU_CAP);
    }

    /// O MRU atual (mais recente primeiro) — o shell persiste isto após cada escolha.
    #[must_use]
    pub fn mru(&self) -> &[String] {
        &self.mru
    }

    /// Injeta o hit-count (projeção do event log) usado como desempate. Chamado a cada abertura.
    pub fn set_hits(&mut self, hits: HashMap<String, u32>) {
        self.hits = hits;
    }

    /// Comandos APLICÁVEIS que casam a query, **ranqueados** (alias-prefixo > MRU > fuzzy >
    /// hit-count). Query vazia → todos os aplicáveis, ordenados por MRU/hit-count. Ver [`rank`].
    #[must_use]
    pub fn filtered(&self) -> Vec<&Command> {
        rank(&self.commands, &self.query, &self.mru, &self.hits)
    }

    /// Registra uma escolha no MRU: a chave vai para a FRENTE (dedup), truncando ao [`MRU_CAP`].
    fn record_use(&mut self, key: &str) {
        self.mru.retain(|k| k != key);
        self.mru.insert(0, key.to_owned());
        self.mru.truncate(MRU_CAP);
    }

    /// **Trata UMA tecla** com a paleta aberta. `Some(action)` no Enter (e fecha); `None` caso
    /// contrário. `ch` é o caractere imprimível da tecla (gpui `key_char`), se houver. gpui-free.
    pub fn handle_key(&mut self, key: &str, ch: Option<&str>) -> Option<PaletteAction> {
        match key {
            "escape" => {
                self.close();
                None
            }
            "enter" | "return" => {
                // Extrai (ação + chave) da seleção ANTES de mutar — a escolha sobe no MRU (tier 2).
                let chosen = self
                    .filtered()
                    .get(self.selected)
                    .map(|c| (c.action.clone(), c.key()));
                if let Some((_, key)) = &chosen {
                    self.record_use(key);
                }
                self.close();
                chosen.map(|(action, _)| action)
            }
            "up" => {
                self.move_sel(-1);
                None
            }
            "down" => {
                self.move_sel(1);
                None
            }
            "backspace" => {
                self.query.pop();
                self.clamp_sel();
                None
            }
            _ => {
                // Caractere imprimível → entra na query (ignora teclas-controle e chords).
                if let Some(c) = ch.filter(|c| !c.is_empty() && !c.chars().any(char::is_control)) {
                    self.query.push_str(c);
                    self.clamp_sel();
                }
                None
            }
        }
    }

    fn move_sel(&mut self, delta: i64) {
        let n = self.filtered().len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        // Navegação CIRCULAR (↓ no fim volta ao topo; ↑ no topo vai ao fim).
        let cur = self.selected.min(n - 1) as i64;
        self.selected = (cur + delta).rem_euclid(n as i64) as usize;
    }

    fn clamp_sel(&mut self) {
        let n = self.filtered().len();
        self.selected = if n == 0 { 0 } else { self.selected.min(n - 1) };
    }

    /// **Render gpui** do overlay (caixa centrada no topo): linha da query + comandos filtrados, com a
    /// seleção destacada. Sem listeners (a paleta é dirigida por teclado).
    #[must_use]
    pub fn render(&self) -> AnyElement {
        // F1-2-1: tokens vivos do tema (dark/light + acento aplicam-se à paleta também).
        let th = crate::theme::active();
        let filtered = self.filtered();
        let sel = self.selected;

        let mut list = div().flex().flex_col();
        if filtered.is_empty() {
            list = list.child(
                div()
                    .px_4()
                    .py_2()
                    .text_color(rgb(th.text.muted))
                    .child(text!("(nenhum comando casa)")),
            );
        } else {
            for (i, c) in filtered.iter().enumerate() {
                let selected = i == sel;
                list = list.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(rgb(if selected {
                            th.surface.raised_alt
                        } else {
                            th.surface.panel
                        }))
                        .text_color(rgb(if selected {
                            th.text.bright
                        } else {
                            th.text.primary
                        }))
                        .child(text!(c.label.clone())),
                );
            }
        }

        // Backdrop full-screen (escurece o canvas) + caixa centrada no topo.
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .mt(px(90.0))
                    .w(px(560.0))
                    .flex()
                    .flex_col()
                    .bg(rgb(th.surface.chrome))
                    .rounded_chrome()
                    .border_1()
                    .border_color(rgb(th.surface.raised_alt))
                    .overflow_hidden()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .bg(rgb(th.surface.panel))
                            .text_color(rgb(th.text.bright))
                            .child(text!(format!("⌘K  {}▌", self.query))),
                    )
                    .child(list),
            )
            .into_any_element()
    }
}

/// **O ranking determinístico (F2-2-4 / D4-A2).** Função PURA — a fonte da previsibilidade. Recebe os
/// comandos, a query e o contexto (MRU + hit-count) e devolve os APLICÁVEIS que casam, ordenados em
/// tiers FIXOS (estável: empates caem na ordem original). Tiers, do mais forte ao mais fraco:
///   1. **classe de casamento** — `Prefix` (alias/rótulo começa com a query) antes de `Fuzzy`;
///   2. **MRU** — usado mais recentemente primeiro (chave em [`PaletteState::mru`]);
///   3. **hit-count** — mais popular primeiro (projeção do event log);
///   4. **ordem original** — desempate final, para o resultado nunca "dançar" entre teclas.
///
/// O **filtro por contexto** (Zed `CommandPaletteFilter`) é o primeiro passo: `!c.enabled` sai fora.
#[must_use]
pub fn rank<'a>(
    commands: &'a [Command],
    query: &str,
    mru: &[String],
    hits: &HashMap<String, u32>,
) -> Vec<&'a Command> {
    // Rank do MRU: índice na lista (0 = mais recente); ausente = pior (fim).
    let mru_rank = |key: &str| mru.iter().position(|k| k == key).unwrap_or(usize::MAX);
    let hit = |key: &str| hits.get(key).copied().unwrap_or(0);

    let mut scored: Vec<(usize, &Command, MatchClass)> = commands
        .iter()
        .enumerate()
        .filter(|(_, c)| c.enabled) // filtro por contexto: inaplicável NÃO entra na lista
        .filter_map(|(i, c)| match_class(query, c).map(|class| (i, c, class)))
        .collect();

    scored.sort_by(|a, b| {
        let (ka, kb) = (a.1.key(), b.1.key());
        a.2.cmp(&b.2) // 1. Prefix < Fuzzy
            .then_with(|| mru_rank(&ka).cmp(&mru_rank(&kb))) // 2. MRU (menor índice = melhor)
            .then_with(|| hit(&kb).cmp(&hit(&ka))) // 3. hit-count (maior primeiro)
            .then_with(|| a.0.cmp(&b.0)) // 4. ordem original (estabilidade)
    });

    scored.into_iter().map(|(_, c, _)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **GUARDIÃO (fix de tela F1-2-1/inv#6)**: o tema NUNCA volta a ficar inalcançável — os
    /// comandos base da paleta SEMPRE incluem a entrada de Ajustes/Aparência, encontrável pelos
    /// termos do T7§A ("tema", "cor", "aparência", "claro/escuro"). Quem remover a entrada
    /// quebra este teste — e o fundador volta a não achar o botão.
    #[test]
    fn settings_entry_is_always_reachable_from_palette() {
        let cmds = base_commands();
        let settings = cmds
            .iter()
            .find(|c| matches!(c.action, PaletteAction::OpenSettings))
            .expect("entrada de Ajustes nos comandos base");
        let label = settings.label.to_lowercase();
        for term in ["tema", "cor", "aparência", "claro", "escuro", "ajustes"] {
            assert!(
                label.contains(term),
                "o rótulo precisa ser encontrável por {term:?} (T7§A): {label}"
            );
        }
    }
    use uuid::Uuid;

    fn cmds() -> Vec<Command> {
        vec![
            Command::new("Novo agente", PaletteAction::NewAgent),
            Command::new("Pausar orquestração", PaletteAction::ToggleBrake),
            Command::new("Nova nota", PaletteAction::NewNote),
        ]
    }

    /// O matcher do ranking: subsequência smart-case + prefixo. Query vazia casa tudo; minúscula é
    /// case-insensitive; um char a mais não casa; maiúscula explícita torna case-sensitive.
    #[test]
    fn subsequence_and_prefix_engine() {
        assert!(is_subsequence("", "qualquer"));
        assert!(is_subsequence("na", "Novo agente"), "n…a subsequência");
        assert!(
            is_subsequence("nota", "Nova nota"),
            "minúscula ignora caixa"
        );
        assert!(!is_subsequence("xyz", "Novo agente"));
        assert!(
            !is_subsequence("agentee", "Novo agente"),
            "char a mais não casa"
        );
        // Smart-case: maiúscula explícita NÃO casa rótulo minúsculo.
        assert!(!is_subsequence("NOTA", "nova nota"));
        // Prefixo (smart-case).
        assert!(is_prefix("nov", "Novo agente"));
        assert!(
            !is_prefix("agente", "Novo agente"),
            "não começa com a query"
        );
    }

    /// Digitar filtra; ↓↑ navegam circular; Enter executa a seleção e fecha; Esc fecha sem ação.
    #[test]
    fn typing_filters_and_enter_executes_selection() {
        let mut p = PaletteState::default();
        assert!(!p.is_open());
        p.open(cmds());
        assert!(p.is_open());
        assert_eq!(p.filtered().len(), 3, "query vazia → todos");

        // Filtra "no" → "Novo agente" + "Nova nota".
        p.handle_key("n", Some("n"));
        p.handle_key("o", Some("o"));
        assert_eq!(p.filtered().len(), 2);

        // ↓ seleciona o 2º; Enter executa a ação dele e fecha.
        p.handle_key("down", None);
        let action = p.handle_key("enter", None);
        assert_eq!(action, Some(PaletteAction::NewNote));
        assert!(!p.is_open(), "Enter fecha a paleta");
    }

    /// Esc fecha sem ação. Backspace edita a query e re-clampa a seleção.
    #[test]
    fn esc_closes_and_backspace_edits() {
        let mut p = PaletteState::default();
        p.open(cmds());
        p.handle_key("p", Some("p")); // só "Pausar..."
        assert_eq!(p.filtered().len(), 1);
        p.handle_key("backspace", None); // query vazia → todos
        assert_eq!(p.filtered().len(), 3);
        assert_eq!(p.handle_key("escape", None), None);
        assert!(!p.is_open());
    }

    /// Enter sem nada casando devolve `None` (não executa ação fantasma) e fecha.
    #[test]
    fn enter_with_no_match_yields_none() {
        let mut p = PaletteState::default();
        p.open(cmds());
        for c in "zzz".chars() {
            p.handle_key(&c.to_string(), Some(&c.to_string()));
        }
        assert!(p.filtered().is_empty());
        assert_eq!(p.handle_key("enter", None), None);
        assert!(!p.is_open());
    }

    /// FocusNode carrega o NodeId real (a ação genérica que o WorkspaceView resolve).
    #[test]
    fn focus_node_action_carries_id() {
        let id = Uuid::now_v7();
        let mut p = PaletteState::default();
        p.open(vec![Command::new(
            "Focar: Terminal A",
            PaletteAction::FocusNode(id),
        )]);
        assert_eq!(
            p.handle_key("enter", None),
            Some(PaletteAction::FocusNode(id))
        );
    }

    // ───────────────────────── F2-2-4 · ranking previsível ─────────────────────────

    /// Atalho de teste: rótulos ranqueados, na ordem.
    fn ranked(cmds: &[Command], q: &str, mru: &[&str], hits: &[(&str, u32)]) -> Vec<String> {
        let mru: Vec<String> = mru.iter().map(|s| (*s).to_owned()).collect();
        let hits: HashMap<String, u32> = hits.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
        rank(cmds, q, &mru, &hits)
            .into_iter()
            .map(|c| c.label.clone())
            .collect()
    }

    /// **Alias = keyword INVISÍVEL:** o leigo digita o sinônimo técnico ("webhook") e acha o comando
    /// cujo RÓTULO não tem a palavra ("Gatilho"). É o caso-âncora da D4-A2.
    #[test]
    fn alias_finds_command_by_invisible_keyword() {
        let cmds = vec![
            Command::new("⚡ Gatilho de automação", PaletteAction::NewFolder)
                .with_aliases(&["webhook", "trigger"]),
            Command::new("📝 Nova nota", PaletteAction::NewNote),
        ];
        let out = ranked(&cmds, "webhook", &[], &[]);
        assert_eq!(
            out,
            vec!["⚡ Gatilho de automação"],
            "alias acha o que o rótulo esconde"
        );
    }

    /// **Tier 1 — prefixo > fuzzy:** quem COMEÇA com a query sobe acima de quem só a contém espalhada.
    #[test]
    fn prefix_outranks_scattered_fuzzy() {
        let cmds = vec![
            Command::new("Nova nota", PaletteAction::NewNote), // fuzzy: n-o-t-a espalhado
            Command::new("Notas do dia", PaletteAction::NewFolder), // prefixo "nota"
        ];
        let out = ranked(&cmds, "nota", &[], &[]);
        assert_eq!(
            out,
            vec!["Notas do dia", "Nova nota"],
            "prefixo vence subsequência"
        );
    }

    /// **Tier 2 — MRO:** com a MESMA classe de casamento, o usado por último sobe.
    #[test]
    fn mru_lifts_recently_used() {
        let cmds = vec![
            Command::new("Pausar", PaletteAction::ToggleBrake), // key: toggle_brake
            Command::new("Painel", PaletteAction::ToggleDashboard), // key: toggle_dashboard
        ];
        // Ambos casam "pa" por prefixo; MRU coloca o Painel na frente.
        let out = ranked(&cmds, "pa", &["toggle_dashboard"], &[]);
        assert_eq!(
            out,
            vec!["Painel", "Pausar"],
            "MRU recente sobe dentro do mesmo tier"
        );
    }

    /// **Tier 1 ainda vence o tier 2:** um PREFIXO não-MRU fica acima de um FUZZY que está no MRU
    /// (a ordem dos tiers é fixa — alias-prefixo antes de MRU).
    #[test]
    fn prefix_beats_mru_fuzzy() {
        let cmds = vec![
            Command::new("Abrir nota", PaletteAction::NewNote), // fuzzy "no": n…o espalhado
            Command::new("Nota azul", PaletteAction::NewFolder), // prefixo "no" (No-ta)
        ];
        // "no": "Nota azul" é PREFIXO (tier 1); "Abrir nota" é só fuzzy. Mesmo com o fuzzy no MRU,
        // o prefixo manda — a ordem dos tiers é fixa.
        let out = ranked(&cmds, "no", &["new_note"], &[]);
        assert_eq!(
            out,
            vec!["Nota azul", "Abrir nota"],
            "tier de classe > tier de MRU"
        );
    }

    /// **Tier 4 — hit-count desempata:** mesma classe, sem MRU → o mais popular primeiro.
    #[test]
    fn hit_count_breaks_remaining_ties() {
        let cmds = vec![
            Command::new("Pausar", PaletteAction::ToggleBrake),
            Command::new("Painel", PaletteAction::ToggleDashboard),
        ];
        let out = ranked(
            &cmds,
            "pa",
            &[],
            &[("toggle_dashboard", 9), ("toggle_brake", 2)],
        );
        assert_eq!(
            out,
            vec!["Painel", "Pausar"],
            "mais usado historicamente desempata"
        );
    }

    /// **Filtro por contexto (Zed `CommandPaletteFilter`):** `when(false)` TIRA o comando da lista —
    /// não acinzentado, FORA. Um botão que não faz nada é tela que mente.
    #[test]
    fn context_filter_drops_inapplicable() {
        let cmds = vec![
            Command::new("Editar Agente", PaletteAction::EditAgent(Uuid::now_v7())).when(false),
            Command::new("Novo agente", PaletteAction::NewAgent),
        ];
        let out = ranked(&cmds, "", &[], &[]);
        assert_eq!(out, vec!["Novo agente"], "inaplicável não aparece");
    }

    /// **Smart-case:** query toda minúscula casa qualquer caixa; uma maiúscula torna o match
    /// case-sensitive (o humano foi específico).
    #[test]
    fn smart_case_matching() {
        let cmds = vec![Command::new("novo agente", PaletteAction::NewAgent)];
        assert_eq!(ranked(&cmds, "novo", &[], &[]).len(), 1, "minúscula casa");
        assert!(
            ranked(&cmds, "Novo", &[], &[]).is_empty(),
            "maiúscula explícita NÃO casa rótulo minúsculo"
        );
    }

    /// **MRU grava no Enter e SOBREVIVE a reabrir:** escolher um comando o joga ao topo na próxima
    /// abertura (previsibilidade entre sessões; o shell persiste via `mru()`).
    #[test]
    fn enter_records_mru_and_persists_across_reopen() {
        let mut p = PaletteState::default();
        p.open(vec![
            Command::new("Pausar", PaletteAction::ToggleBrake),
            Command::new("Painel", PaletteAction::ToggleDashboard),
        ]);
        // Seleciona o 2º (Painel) e executa.
        p.handle_key("down", None);
        assert_eq!(
            p.handle_key("enter", None),
            Some(PaletteAction::ToggleDashboard)
        );
        assert_eq!(
            p.mru().first().map(String::as_str),
            Some("toggle_dashboard")
        );
        // Reabre: Painel agora lidera a lista (query vazia ordena por MRU).
        p.open(vec![
            Command::new("Pausar", PaletteAction::ToggleBrake),
            Command::new("Painel", PaletteAction::ToggleDashboard),
        ]);
        let labels: Vec<_> = p.filtered().iter().map(|c| c.label.clone()).collect();
        assert_eq!(
            labels,
            vec!["Painel", "Pausar"],
            "o último usado abre no topo"
        );
    }

    /// MRU dedup + teto: re-escolher não duplica; o teto [`MRU_CAP`] segura a lista curta.
    #[test]
    fn mru_dedups_and_caps() {
        let mut p = PaletteState::default();
        p.set_mru(vec!["toggle_brake".into(), "new_note".into()]);
        p.record_use("new_note"); // já existia → vai pra frente, sem duplicar
        assert_eq!(p.mru(), &["new_note".to_owned(), "toggle_brake".to_owned()]);
        // Enche além do teto.
        for i in 0..MRU_CAP + 3 {
            p.record_use(&format!("k{i}"));
        }
        assert_eq!(p.mru().len(), MRU_CAP, "MRU fica curto (teto)");
    }

    // ───────────────────────── F2-2-5 · registry node_commands ─────────────────────────

    fn ctx(kind: NodeKind, status: NodeStatus, needs_human: bool, roster_len: usize) -> NodeCtx {
        NodeCtx {
            id: Uuid::now_v7(),
            kind,
            status,
            needs_human,
            roster_len,
        }
    }

    fn has(cmds: &[Command], pred: impl Fn(&PaletteAction) -> bool) -> bool {
        cmds.iter().any(|c| pred(&c.action))
    }

    /// Spec §7.1: `needs_human` liga "Atender" (4 ações); todo Command tem label não-vazio (anti
    /// icon-only); Centralizar está SEMPRE; a ordem é primária→reversíveis→destrutiva.
    #[test]
    fn node_commands_needs_human_shows_atender_first() {
        let c = node_commands(&ctx(NodeKind::Terminal, NodeStatus::Idle, true, 3));
        assert_eq!(c.len(), 4, "pede-aprovação → 4 ações");
        assert!(
            matches!(c[0].action, PaletteAction::GotoAtencao(_)),
            "primária no topo"
        );
        assert!(
            matches!(c[3].action, PaletteAction::CloseNode(_)),
            "destrutiva por último"
        );
        assert!(
            c.iter().all(|cmd| !cmd.label.trim().is_empty()),
            "nenhum botão icon-only"
        );
        assert!(
            has(&c, |a| matches!(a, PaletteAction::FocusNode(_))),
            "Centralizar sempre"
        );
    }

    /// Sem pedido de aprovação → 3 ações (sem a primária colorida). Editar presente em nó-agente vivo.
    #[test]
    fn node_commands_working_state_has_three_no_primary() {
        let c = node_commands(&ctx(NodeKind::Terminal, NodeStatus::Busy, false, 2));
        assert_eq!(c.len(), 3, "trabalhando/pronto/novo → 3 ações");
        assert!(
            !has(&c, |a| matches!(a, PaletteAction::GotoAtencao(_))),
            "sem Atender sem needs_human"
        );
        assert!(
            has(&c, |a| matches!(a, PaletteAction::EditAgent(_))),
            "Editar em agente vivo"
        );
    }

    /// Filtro por contexto (§2.2): nó NÃO-agente esconde Editar; nó MORTO esconde Editar.
    #[test]
    fn node_commands_hides_editar_when_not_agent_or_dead() {
        let nota = node_commands(&ctx(NodeKind::Note, NodeStatus::Idle, false, 2));
        assert!(
            !has(&nota, |a| matches!(a, PaletteAction::EditAgent(_))),
            "Nota não edita Agente"
        );
        let morto = node_commands(&ctx(NodeKind::Terminal, NodeStatus::Dead, false, 2));
        assert!(
            !has(&morto, |a| matches!(a, PaletteAction::EditAgent(_))),
            "nó morto não edita"
        );
        // Centralizar permanece mesmo morto/Nota (sempre).
        assert!(has(&nota, |a| matches!(a, PaletteAction::FocusNode(_))));
    }

    /// Encerrar exige roster > 1: 1 clique nunca esvazia o Espaço.
    #[test]
    fn node_commands_close_needs_more_than_one_node() {
        let solo = node_commands(&ctx(NodeKind::Terminal, NodeStatus::Idle, false, 1));
        assert!(
            !has(&solo, |a| matches!(a, PaletteAction::CloseNode(_))),
            "último nó não fecha por 1 clique"
        );
        let multi = node_commands(&ctx(NodeKind::Terminal, NodeStatus::Idle, false, 2));
        assert!(has(&multi, |a| matches!(a, PaletteAction::CloseNode(_))));
    }

    /// As 2 ações aditivas têm key ESTÁVEL (telemetria/ranking — não o rótulo).
    #[test]
    fn additive_actions_have_stable_keys() {
        let id = Uuid::now_v7();
        assert_eq!(
            action_key(&PaletteAction::GotoAtencao(id)),
            format!("goto_atencao:{id}")
        );
        assert_eq!(
            action_key(&PaletteAction::CloseNode(id)),
            format!("close_node:{id}")
        );
    }
}
