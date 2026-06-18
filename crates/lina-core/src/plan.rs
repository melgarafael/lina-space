//! **W3-5 · Plano compartilhado (`.lina/plan.md`).**
//!
//! **Invariante do `CLAUDE.md` (#4):** o event log é a fonte da verdade; o `plan.md` é uma
//! **projeção reconstruível**. Este módulo é a parte PURA — modelo + parser/serializer rígido
//! (round-trip exato) + a álgebra de comandos/projeção. O I/O (escrita atômica do arquivo) e o
//! event-sourcing (log + reprojeção) vivem no [`crate::Router`] (o **escritor único** de `.lina/`).
//!
//! Há dois caminhos sobre o MESMO modelo, na distinção comando-vs-evento do event-sourcing:
//! - **Comandos** ([`Plan::try_claim`]/[`Plan::try_check`]/[`Plan::add_item`]) — VALIDAM contra o
//!   estado atual e podem REJEITAR; o supervisor os roda ANTES de logar o evento.
//! - **Aplicadores** ([`Plan::apply_claimed`]/…) — PROJETAM um evento JÁ validado, infalíveis;
//!   rodam no replay ([`crate::apply`]). Comando e aplicador partilham a mutação-núcleo, então o
//!   `plan.md` ao-vivo e o reconstruído convergem byte-a-byte.

use crate::events::AcceptanceCriterion;
use serde::{Deserialize, Serialize};

/// Versão do formato canônico de `.lina/plan.md` (parser rígido).
pub const PLAN_SCHEMA_V1: &str = "lina/plan@1";

/// Cabeçalho da 1ª linha (o `<workspace>` vem depois). Em-dash canônico.
const HEADER_PREFIX: &str = "# Plano — ";
/// Comentário da 2ª linha — sentinela de versão + aviso de escritor único.
const SCHEMA_COMMENT: &str = "<!-- lina/plan@1 · escritor unico: supervisor · NAO editar a mao -->";
const SECTION_DECISOES: &str = "## Decisoes";
const SECTION_ITENS: &str = "## Itens";
/// Separador de campos de um item (espaços inclusos — desc pode conter `::` interno).
const FIELD_SEP: &str = " :: ";

/// Estado de um item do plano. O checkbox e o `status:` do formato são DUAS faces do mesmo estado;
/// o parser exige que batam (rigidez) e o serializer emite ambos a partir desta enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemState {
    /// `[ ]` · `status:todo`
    Todo,
    /// `[~]` · `status:doing`
    Doing,
    /// `[x]` · `status:done`
    Done,
    /// `[!]` · `status:blocked`
    Blocked,
}

impl ItemState {
    /// O checkbox canônico (`[ ]`/`[~]`/`[x]`/`[!]`).
    #[must_use]
    pub fn checkbox(self) -> &'static str {
        match self {
            ItemState::Todo => "[ ]",
            ItemState::Doing => "[~]",
            ItemState::Done => "[x]",
            ItemState::Blocked => "[!]",
        }
    }

    /// A palavra de status canônica (`todo`/`doing`/`done`/`blocked`).
    #[must_use]
    pub fn status_word(self) -> &'static str {
        match self {
            ItemState::Todo => "todo",
            ItemState::Doing => "doing",
            ItemState::Done => "done",
            ItemState::Blocked => "blocked",
        }
    }

    fn from_checkbox(s: &str) -> Option<Self> {
        match s {
            "[ ]" => Some(ItemState::Todo),
            "[~]" => Some(ItemState::Doing),
            "[x]" => Some(ItemState::Done),
            "[!]" => Some(ItemState::Blocked),
            _ => None,
        }
    }

    fn from_status_word(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(ItemState::Todo),
            "doing" => Some(ItemState::Doing),
            "done" => Some(ItemState::Done),
            "blocked" => Some(ItemState::Blocked),
            _ => None,
        }
    }
}

/// Um item do plano. `owner == None` ↔ `@owner:?` (sem dono).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    /// Id curto e estável (ex.: `T1`).
    pub id: String,
    /// Descrição livre (pode conter `::` — o parser preserva).
    pub desc: String,
    /// Dono atual (`@Nome`) ou `None` para `@owner:?`.
    pub owner: Option<String>,
    pub status: ItemState,
    /// F3-1 (spec 52 §2): a qual Goal este item serve (`None` = item avulso, comportamento legado).
    #[serde(default)]
    pub goal_id: Option<String>,
    /// F3-1 (spec 52 §2): dependências ESTRUTURADAS (ids de outros `PlanItem`) — corrige a
    /// incoerência skill↔código (`lina-orchestration` já fala `parents:`; o código passa a tê-lo).
    /// A guarda que bloqueia despacho até os parents estarem `Done` é da fatia CORE-Plan.
    #[serde(default)]
    pub parents: Vec<String>,
    /// F3-1 (spec 52 §2): DoD por item — critérios verificados para emitir o `ReviewVerdict`.
    #[serde(default)]
    pub acceptance: Vec<AcceptanceCriterion>,
    /// F3-1 (spec 52 §2): orçamento de tokens deste item (`0` = sem teto próprio; herda a Goal).
    #[serde(default)]
    pub budget_tokens: u64,
}

/// O plano compartilhado — projeção do event log, serializável em `.lina/plan.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Nome do workspace (cabeçalho). Vem de `WorkspaceCreated` na projeção.
    pub workspace: String,
    /// Decisões vivas — precedência sobre o vault, abaixo da instrução corrente do usuário.
    pub decisoes: Vec<String>,
    pub itens: Vec<PlanItem>,
}

/// Erros do plano: malformação de parse e rejeições de comando (`claim`/`check`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// Texto fora do formato canônico `lina/plan@1` (linha citada).
    #[error("plan.md malformado: {0}")]
    Malformed(String),
    /// `claim`/`check` referenciou um item inexistente.
    #[error("item '{0}' nao existe no plano")]
    NoSuchItem(String),
    /// `claim` de item já reivindicado por OUTRO owner (idempotente p/ o mesmo).
    #[error("item '{id}' ja pertence a {current} (nao a {wanted})")]
    AlreadyOwned {
        id: String,
        current: String,
        wanted: String,
    },
    /// `check` por quem NÃO é o owner do item.
    #[error("item '{id}' so pode ser concluido pelo owner {current}, nao por {wanted}")]
    NotOwner {
        id: String,
        current: String,
        wanted: String,
    },
    /// Item duplicado ao semear (`add_item` de um id já existente).
    #[error("item '{0}' ja existe no plano")]
    DuplicateItem(String),
}

impl Plan {
    /// Plano vazio para um workspace.
    #[must_use]
    pub fn new(workspace: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            decisoes: Vec::new(),
            itens: Vec::new(),
        }
    }

    fn find(&self, id: &str) -> Option<&PlanItem> {
        self.itens.iter().find(|i| i.id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut PlanItem> {
        self.itens.iter_mut().find(|i| i.id == id)
    }

    // ───────────────────────── comandos (validam, podem rejeitar) ─────────────────────────

    /// Semeia um item (`status:todo`, `@owner:?`). Rejeita id duplicado.
    ///
    /// # Errors
    /// [`PlanError::DuplicateItem`] se o `id` já existe.
    pub fn add_item(
        &mut self,
        id: impl Into<String>,
        desc: impl Into<String>,
    ) -> Result<(), PlanError> {
        let id = id.into();
        if self.find(&id).is_some() {
            return Err(PlanError::DuplicateItem(id));
        }
        self.apply_item_added(id, desc.into());
        Ok(())
    }

    /// Adiciona uma decisão viva (sempre aceita).
    pub fn add_decision(&mut self, text: impl Into<String>) {
        self.apply_decision_added(text.into());
    }

    /// **`claim`**: `owner` reivindica o item → `@owner:owner` + `status:doing`.
    /// Idempotente para o MESMO owner; rejeita se outro já é dono.
    ///
    /// # Errors
    /// [`PlanError::NoSuchItem`] se o item não existe; [`PlanError::AlreadyOwned`] se outro owner.
    pub fn try_claim(&mut self, id: &str, owner: &str) -> Result<(), PlanError> {
        let item = self
            .find(id)
            .ok_or_else(|| PlanError::NoSuchItem(id.to_string()))?;
        if let Some(cur) = &item.owner {
            if cur != owner {
                return Err(PlanError::AlreadyOwned {
                    id: id.to_string(),
                    current: cur.clone(),
                    wanted: owner.to_string(),
                });
            }
        }
        self.apply_claimed(id, owner);
        Ok(())
    }

    /// **`check`**: o owner conclui o item → `status:done`. Exige que `who` seja o owner.
    ///
    /// # Errors
    /// [`PlanError::NoSuchItem`] se o item não existe; [`PlanError::NotOwner`] se `who` não é o owner
    /// (inclui item sem dono).
    pub fn try_check(&mut self, id: &str, who: &str) -> Result<(), PlanError> {
        let item = self
            .find(id)
            .ok_or_else(|| PlanError::NoSuchItem(id.to_string()))?;
        match &item.owner {
            Some(cur) if cur == who => {}
            other => {
                return Err(PlanError::NotOwner {
                    id: id.to_string(),
                    current: other.clone().unwrap_or_else(|| "?".to_string()),
                    wanted: who.to_string(),
                })
            }
        }
        self.apply_checked(id, who);
        Ok(())
    }

    // ───────────────────────── aplicadores (projeção, infalíveis) ─────────────────────────

    /// Projeta `PlanItemAdded` (replay). Idempotente por id — ignora duplicata.
    pub(crate) fn apply_item_added(&mut self, id: String, desc: String) {
        if self.find(&id).is_none() {
            self.itens.push(PlanItem {
                id,
                desc,
                owner: None,
                status: ItemState::Todo,
                goal_id: None,
                parents: Vec::new(),
                acceptance: Vec::new(),
                budget_tokens: 0,
            });
        }
    }

    /// F3-1 (spec 52 §2): projeta `PlanItemAttributed` (replay) — atribui Goal + dependências + DoD a
    /// um item já semeado. Idempotente por item (último vence); item inexistente → no-op (espelha a
    /// robustez de [`Self::apply_claimed`]).
    pub(crate) fn apply_item_attributed(
        &mut self,
        item: &str,
        goal_id: Option<String>,
        parents: Vec<String>,
        acceptance: Vec<AcceptanceCriterion>,
        budget_tokens: u64,
    ) {
        if let Some(it) = self.find_mut(item) {
            it.goal_id = goal_id;
            it.parents = parents;
            it.acceptance = acceptance;
            it.budget_tokens = budget_tokens;
        }
    }

    /// Projeta `PlanDecisionAdded` (replay).
    pub(crate) fn apply_decision_added(&mut self, text: String) {
        self.decisoes.push(text);
    }

    /// Projeta `PlanClaimed` (replay): fato já validado → seta owner + doing.
    pub(crate) fn apply_claimed(&mut self, id: &str, owner: &str) {
        if let Some(item) = self.find_mut(id) {
            item.owner = Some(owner.to_string());
            item.status = ItemState::Doing;
        }
    }

    /// Projeta `PlanChecked` (replay): fato já validado → done (guarda de owner por robustez).
    pub(crate) fn apply_checked(&mut self, id: &str, who: &str) {
        if let Some(item) = self.find_mut(id) {
            if item.owner.as_deref() == Some(who) {
                item.status = ItemState::Done;
            }
        }
    }

    // ───────────────────────── serializer / parser (round-trip exato) ─────────────────────────

    /// Serializa para o formato canônico `lina/plan@1`. Determinístico; round-trip com [`Plan::parse`].
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(HEADER_PREFIX);
        out.push_str(&self.workspace);
        out.push('\n');
        out.push_str(SCHEMA_COMMENT);
        out.push('\n');
        out.push_str(SECTION_DECISOES);
        out.push('\n');
        for d in &self.decisoes {
            out.push_str("- ");
            out.push_str(d);
            out.push('\n');
        }
        out.push_str(SECTION_ITENS);
        out.push('\n');
        for item in &self.itens {
            let owner = item.owner.as_deref().unwrap_or("?");
            out.push_str("- ");
            out.push_str(item.status.checkbox());
            out.push(' ');
            out.push_str(&item.id);
            out.push_str(FIELD_SEP);
            out.push_str(&item.desc);
            out.push_str(FIELD_SEP);
            out.push_str("@owner:");
            out.push_str(owner);
            out.push_str(FIELD_SEP);
            out.push_str("status:");
            out.push_str(item.status.status_word());
            out.push('\n');
        }
        out
    }

    /// Parser RÍGIDO do formato canônico. Falha (sem corromper) em qualquer desvio: cabeçalho/versão
    /// errados, checkbox≠status, owner/status sem prefixo. Round-trip exato com [`Plan::render`].
    ///
    /// # Errors
    /// [`PlanError::Malformed`] com a linha ofensora.
    pub fn parse(text: &str) -> Result<Self, PlanError> {
        let mut lines = text.lines();

        let header = lines
            .next()
            .ok_or_else(|| PlanError::Malformed("vazio (sem cabecalho)".into()))?;
        let workspace = header
            .strip_prefix(HEADER_PREFIX)
            .ok_or_else(|| PlanError::Malformed(format!("cabecalho invalido: {header:?}")))?
            .to_string();

        let comment = lines
            .next()
            .ok_or_else(|| PlanError::Malformed("falta o comentario de versao".into()))?;
        if !comment.contains(PLAN_SCHEMA_V1) {
            return Err(PlanError::Malformed(format!(
                "versao ausente/errada (esperado {PLAN_SCHEMA_V1}): {comment:?}"
            )));
        }

        let mut plan = Plan::new(workspace);
        let mut section = Section::None;
        for line in lines {
            if line == SECTION_DECISOES {
                section = Section::Decisoes;
            } else if line == SECTION_ITENS {
                section = Section::Itens;
            } else if line.is_empty() {
                // tolera linha em branco entre seções (não emitimos, mas não rejeitamos).
            } else {
                match section {
                    Section::Decisoes => {
                        let d = line.strip_prefix("- ").ok_or_else(|| {
                            PlanError::Malformed(format!("decisao sem marcador '- ': {line:?}"))
                        })?;
                        plan.decisoes.push(d.to_string());
                    }
                    Section::Itens => plan.itens.push(parse_item(line)?),
                    Section::None => {
                        return Err(PlanError::Malformed(format!(
                            "conteudo fora de uma secao: {line:?}"
                        )))
                    }
                }
            }
        }
        Ok(plan)
    }
}

#[derive(Clone, Copy)]
enum Section {
    None,
    Decisoes,
    Itens,
}

/// Parseia UMA linha de item: `- [x] T3 :: desc :: @owner:@QA :: status:done`.
fn parse_item(line: &str) -> Result<PlanItem, PlanError> {
    let body = line
        .strip_prefix("- ")
        .ok_or_else(|| PlanError::Malformed(format!("item sem marcador '- ': {line:?}")))?;
    // Checkbox = 3 bytes ASCII (`[ ]`/`[~]`/`[x]`/`[!]` — o de `todo` TEM espaço interno, então não
    // se pode dividir no 1º espaço) + 1 espaço separador.
    let checkbox = body
        .get(0..3)
        .ok_or_else(|| PlanError::Malformed(format!("item sem checkbox: {line:?}")))?;
    let state = ItemState::from_checkbox(checkbox)
        .ok_or_else(|| PlanError::Malformed(format!("checkbox invalido {checkbox:?}: {line:?}")))?;
    let rest = body
        .get(3..)
        .and_then(|s| s.strip_prefix(' '))
        .ok_or_else(|| {
            PlanError::Malformed(format!("item sem espaco apos o checkbox: {line:?}"))
        })?;

    // Campos por ` :: `. desc pode conter `::`, então id=primeiro, status=último, owner=penúltimo,
    // desc = tudo no meio re-unido pelo separador.
    let parts: Vec<&str> = rest.split(FIELD_SEP).collect();
    if parts.len() < 4 {
        return Err(PlanError::Malformed(format!(
            "item precisa de id :: desc :: @owner:.. :: status:..: {line:?}"
        )));
    }
    let id = parts[0].to_string();
    let status_part = parts[parts.len() - 1];
    let owner_part = parts[parts.len() - 2];
    let desc = parts[1..parts.len() - 2].join(FIELD_SEP);

    let owner_raw = owner_part.strip_prefix("@owner:").ok_or_else(|| {
        PlanError::Malformed(format!(
            "campo owner sem '@owner:': {owner_part:?} ({line:?})"
        ))
    })?;
    let owner = if owner_raw == "?" {
        None
    } else {
        Some(owner_raw.to_string())
    };

    let status_word = status_part.strip_prefix("status:").ok_or_else(|| {
        PlanError::Malformed(format!(
            "campo status sem 'status:': {status_part:?} ({line:?})"
        ))
    })?;
    let status_state = ItemState::from_status_word(status_word).ok_or_else(|| {
        PlanError::Malformed(format!("status invalido {status_word:?}: {line:?}"))
    })?;

    // Rigidez: o checkbox e o `status:` têm de bater (duas faces do mesmo estado).
    if status_state != state {
        return Err(PlanError::Malformed(format!(
            "checkbox {checkbox:?} contradiz status:{status_word} em: {line:?}"
        )));
    }

    Ok(PlanItem {
        id,
        desc,
        owner,
        status: state,
        // F3-1: os atributos da Goal não vivem na gramática rígida desta fatia (a serialização dos
        // sufixos `@goal:`/`@parents:`/`@accept:` é da fatia CORE-Plan); a linha legada parseia com
        // os defaults, preservando o round-trip byte-a-byte.
        goal_id: None,
        parents: Vec::new(),
        acceptance: Vec::new(),
        budget_tokens: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> Plan {
        let mut p = Plan::new("App X");
        p.add_decision("Backend usa Postgres");
        p.add_decision("Sem libs de UI no core");
        p.add_item("T1", "Desenhar schema").unwrap();
        p.add_item("T2", "Implementar API :: com paginacao")
            .unwrap(); // desc com `::` interno
        p.add_item("T3", "Escrever testes").unwrap();
        p.try_claim("T2", "@Dev Backend").unwrap();
        p.try_claim("T3", "@QA").unwrap();
        p.try_check("T3", "@QA").unwrap();
        p
    }

    /// Aceite (round-trip): texto canônico → modelo → texto IDÊNTICO.
    #[test]
    fn render_then_parse_then_render_is_identical() {
        let plan = sample_plan();
        let text = plan.render();
        let reparsed = Plan::parse(&text).expect("parse do proprio render");
        assert_eq!(reparsed, plan, "modelo round-trip");
        assert_eq!(reparsed.render(), text, "texto round-trip byte-a-byte");
    }

    /// O formato canônico bate com a especificação congelada (cabeçalho, comentário, seções).
    #[test]
    fn rendered_format_matches_frozen_spec() {
        let mut p = Plan::new("App X");
        p.add_item("T1", "fazer algo").unwrap();
        p.try_claim("T1", "@A").unwrap();
        let text = p.render();
        let expected = "# Plano — App X\n\
            <!-- lina/plan@1 · escritor unico: supervisor · NAO editar a mao -->\n\
            ## Decisoes\n\
            ## Itens\n\
            - [~] T1 :: fazer algo :: @owner:@A :: status:doing\n";
        assert_eq!(text, expected);
    }

    /// desc com `::` interno sobrevive ao round-trip (owner/status são lidos das pontas).
    #[test]
    fn desc_with_double_colon_roundtrips() {
        let text = sample_plan().render();
        let back = Plan::parse(&text).unwrap();
        let t2 = back.itens.iter().find(|i| i.id == "T2").unwrap();
        assert_eq!(t2.desc, "Implementar API :: com paginacao");
        assert_eq!(t2.owner.as_deref(), Some("@Dev Backend"));
        assert_eq!(t2.status, ItemState::Doing);
    }

    /// Os quatro estados (incl. blocked) round-trippam (checkbox e status:).
    #[test]
    fn all_four_states_roundtrip() {
        let mut p = Plan::new("W");
        p.itens.push(PlanItem {
            id: "A".into(),
            desc: "a".into(),
            owner: None,
            status: ItemState::Todo,
            goal_id: None,
            parents: Vec::new(),
            acceptance: Vec::new(),
            budget_tokens: 0,
        });
        p.itens.push(PlanItem {
            id: "B".into(),
            desc: "b".into(),
            owner: Some("@x".into()),
            status: ItemState::Doing,
            goal_id: None,
            parents: Vec::new(),
            acceptance: Vec::new(),
            budget_tokens: 0,
        });
        p.itens.push(PlanItem {
            id: "C".into(),
            desc: "c".into(),
            owner: Some("@y".into()),
            status: ItemState::Done,
            goal_id: None,
            parents: Vec::new(),
            acceptance: Vec::new(),
            budget_tokens: 0,
        });
        p.itens.push(PlanItem {
            id: "D".into(),
            desc: "d".into(),
            owner: None,
            status: ItemState::Blocked,
            goal_id: None,
            parents: Vec::new(),
            acceptance: Vec::new(),
            budget_tokens: 0,
        });
        let back = Plan::parse(&p.render()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn parse_rejects_wrong_version() {
        let bad = "# Plano — W\n<!-- lina/plan@99 -->\n## Decisoes\n## Itens\n";
        assert!(matches!(Plan::parse(bad), Err(PlanError::Malformed(_))));
    }

    #[test]
    fn parse_rejects_checkbox_status_mismatch() {
        let bad = "# Plano — W\n<!-- lina/plan@1 -->\n## Decisoes\n## Itens\n\
            - [ ] T1 :: x :: @owner:? :: status:done\n";
        assert!(matches!(Plan::parse(bad), Err(PlanError::Malformed(_))));
    }

    #[test]
    fn parse_rejects_owner_without_prefix() {
        let bad = "# Plano — W\n<!-- lina/plan@1 -->\n## Decisoes\n## Itens\n\
            - [ ] T1 :: x :: @A :: status:todo\n";
        assert!(matches!(Plan::parse(bad), Err(PlanError::Malformed(_))));
    }

    /// claim de item livre → doing + owner; claim do MESMO owner é idempotente.
    #[test]
    fn claim_sets_owner_and_is_idempotent_for_same_owner() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.try_claim("T1", "@A").unwrap();
        let i = p.find("T1").unwrap();
        assert_eq!(i.owner.as_deref(), Some("@A"));
        assert_eq!(i.status, ItemState::Doing);
        // de novo pelo mesmo owner: OK, sem erro.
        p.try_claim("T1", "@A").unwrap();
        assert_eq!(p.find("T1").unwrap().status, ItemState::Doing);
    }

    /// claim de item já ownereado por OUTRO → rejeitado, sem mutar o plano.
    #[test]
    fn claim_of_owned_item_by_another_is_rejected_without_corruption() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.try_claim("T1", "@A").unwrap();
        let before = p.clone();
        let err = p.try_claim("T1", "@B").unwrap_err();
        assert!(matches!(err, PlanError::AlreadyOwned { .. }));
        assert_eq!(p, before, "rejeicao nao pode corromper o plano");
    }

    /// check pelo owner → done; check por NÃO-owner → rejeitado.
    #[test]
    fn check_requires_owner() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.try_claim("T1", "@A").unwrap();
        // não-owner não conclui.
        assert!(matches!(
            p.try_check("T1", "@B"),
            Err(PlanError::NotOwner { .. })
        ));
        assert_eq!(
            p.find("T1").unwrap().status,
            ItemState::Doing,
            "rejeicao nao muda status"
        );
        // owner conclui.
        p.try_check("T1", "@A").unwrap();
        assert_eq!(p.find("T1").unwrap().status, ItemState::Done);
    }

    #[test]
    fn check_of_unclaimed_item_is_rejected() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        assert!(matches!(
            p.try_check("T1", "@A"),
            Err(PlanError::NotOwner { .. })
        ));
    }

    #[test]
    fn claim_or_check_missing_item_errors() {
        let mut p = Plan::new("W");
        assert!(matches!(
            p.try_claim("Nope", "@A"),
            Err(PlanError::NoSuchItem(_))
        ));
        assert!(matches!(
            p.try_check("Nope", "@A"),
            Err(PlanError::NoSuchItem(_))
        ));
    }

    #[test]
    fn add_duplicate_item_is_rejected() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        assert!(matches!(
            p.add_item("T1", "y"),
            Err(PlanError::DuplicateItem(_))
        ));
    }
}
