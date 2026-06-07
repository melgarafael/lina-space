//! **W4-2 · M1 — PALETA DE COMANDOS (Cmd-K)** sobre o canvas T4. O MODELO (estado + fuzzy-match +
//! navegação por teclado) é PURO e testável (sem gpui); só o `render` consome gpui. As AÇÕES são
//! genéricas ([`PaletteAction`]) — quem as executa é o `WorkspaceView` (toca `naming`/`focus`/`brake`),
//! para manter a paleta desacoplada do canvas.
//!
//! UX: **Cmd-K abre** (com a lista de comandos do momento) · **digitar filtra** (subsequência) · **↑↓
//! navega** · **Enter executa** a seleção · **Esc fecha**.

use gpui::{div, prelude::*, px, rgb, text, AnyElement};
use lina_host::NodeId;

/// O que a paleta dispara. Genérico: o `WorkspaceView` executa (abre o modal M6, foca um
/// nó, alterna o freio, ou — placeholder até `creators.rs` — cria nota/pasta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    /// F1-2-2 (M6): abre o modal "Novo Agente" (evoluiu o modo-nomeação do M2).
    NewAgent,
    /// F1-2-2 (M6-E): abre o modal em modo EDITAR para um nó vivo.
    EditAgent(NodeId),
    /// Foca (e revela) um nó do canvas.
    FocusNode(NodeId),
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
        Command::new("✦ Novo agente", PaletteAction::NewAgent),
        Command::new(
            "🎨 Aparência: tema e cores (claro/escuro) — Ajustes",
            PaletteAction::OpenSettings,
        ),
        Command::new(
            "⏸ Pausar / retomar orquestração (freio)",
            PaletteAction::ToggleBrake,
        ),
        Command::new("📝 Nova nota", PaletteAction::NewNote),
        Command::new("📁 Nova pasta", PaletteAction::NewFolder),
        // F1-1-5 (entry point descobrível do P6 — fluxo c): "dashboard", "atividade",
        // "custo" são os termos que um leigo digita.
        Command::new(
            "📊 Dashboard: atividade e custos do time",
            PaletteAction::ToggleDashboard,
        ),
    ]
}

/// Um comando listável: rótulo (o que o humano lê/filtra) + ação.
#[derive(Debug, Clone)]
pub struct Command {
    pub label: String,
    pub action: PaletteAction,
}

impl Command {
    pub fn new(label: impl Into<String>, action: PaletteAction) -> Self {
        Self {
            label: label.into(),
            action,
        }
    }
}

/// **Estado da paleta** (vive no `WorkspaceView`). Fechada por padrão.
#[derive(Debug, Default)]
pub struct PaletteState {
    open: bool,
    query: String,
    /// Índice da seleção DENTRO da lista FILTRADA.
    selected: usize,
    commands: Vec<Command>,
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

    /// Comandos que casam a query (fuzzy, ordem original). Query vazia → todos.
    #[must_use]
    pub fn filtered(&self) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|c| fuzzy_match(&self.query, &c.label))
            .collect()
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
                let action = self.filtered().get(self.selected).map(|c| c.action.clone());
                self.close();
                action
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
                    .rounded_md()
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

/// **Fuzzy-match SIMPLES:** a `query` (case-insensitive) é SUBSEQUÊNCIA do `label`. Query vazia casa
/// tudo. Ex.: "na" casa "Novo **a**gente" (n…a) e "**N**ova not**a**".
#[must_use]
pub fn fuzzy_match(query: &str, label: &str) -> bool {
    let q = query.to_lowercase();
    if q.is_empty() {
        return true;
    }
    let mut needle = q.chars().peekable();
    for lc in label.to_lowercase().chars() {
        match needle.peek() {
            Some(&qc) if qc == lc => {
                needle.next();
            }
            Some(_) => {}
            None => break,
        }
    }
    needle.peek().is_none()
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

    /// Subsequência case-insensitive; query vazia casa tudo.
    #[test]
    fn fuzzy_match_is_case_insensitive_subsequence() {
        assert!(fuzzy_match("", "qualquer"));
        assert!(fuzzy_match("na", "Novo agente"), "n…a subsequência");
        assert!(fuzzy_match("NOTA", "Nova nota"));
        assert!(fuzzy_match("paus", "Pausar orquestração"));
        assert!(!fuzzy_match("xyz", "Novo agente"));
        assert!(
            !fuzzy_match("agentee", "Novo agente"),
            "char a mais não casa"
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
}
