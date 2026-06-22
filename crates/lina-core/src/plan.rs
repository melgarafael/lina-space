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
    /// F3-4-3 (spec 36 §2, ADR 0041): arquivos/globs que o item RESERVA, relativos ao repo. Cruzados
    /// com `CodeChanged.paths` de um commit do time: interseção com o claim de OUTRO nó → o dono PARA
    /// e abre um `CodeConflict` (a "trava cooperativa" do pertencimento). DADO declarado, JAMAIS
    /// autoridade (família ADR 0007): só abre alerta, nunca concede posse nem autoriza ação.
    /// `#[serde(default)]` → plano/log anterior reconstrói com `paths: []`, round-trip byte-a-byte
    /// exato (inv #4).
    #[serde(default)]
    pub paths: Vec<String>,
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
    /// F3-1 (spec 52 §2): `claim`/despacho de um item cujos `parents` ainda não estão todos
    /// `Done` (anti-race estrutural). O roteador já mapeia `Err` → `RouteOutcome::PlanRejected`,
    /// então este erro vira rejeição com a lista de pais pendentes, sem novo ramo no router.
    #[error("item '{id}' depende de {pending:?} ainda nao concluidos")]
    ParentsNotDone { id: String, pending: Vec<String> },
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
    /// Idempotente para o MESMO owner; rejeita se outro já é dono ou se há pai não-`Done`.
    ///
    /// # Errors
    /// [`PlanError::NoSuchItem`] se o item não existe; [`PlanError::AlreadyOwned`] se outro owner;
    /// [`PlanError::ParentsNotDone`] se algum `parent` ainda não está `Done` (spec 52 §2).
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
        // F3-1 (spec 52 §2): guarda de ordem — recusa o claim enquanto algum `parent` não está
        // `Done` (pai inexistente conta como pendente: fail-safe, nunca libera despacho prematuro).
        // É o que torna `parents:` DADO que bloqueia o despacho, não prosa. Lê só estado projetado
        // (determinístico, zero LLM); roda antes da mutação, igual à guarda de `AlreadyOwned`.
        let pending: Vec<String> = item
            .parents
            .iter()
            .filter(|p| self.find(p).is_none_or(|it| it.status != ItemState::Done))
            .cloned()
            .collect();
        if !pending.is_empty() {
            return Err(PlanError::ParentsNotDone {
                id: id.to_string(),
                pending,
            });
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
                paths: Vec::new(),
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
        paths: Vec<String>,
    ) {
        if let Some(it) = self.find_mut(item) {
            it.goal_id = goal_id;
            it.parents = parents;
            it.acceptance = acceptance;
            it.budget_tokens = budget_tokens;
            it.paths = paths;
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
            // F3-1 (spec 52 §2): atributos da Goal como sufixos OPCIONAIS, em ordem fixa (= ordem
            // dos campos). Emitidos só quando não-default, de modo que o item legado (todos default)
            // produz a MESMA linha de antes — round-trip byte-a-byte com o log gravado pré-spec.
            if let Some(goal) = &item.goal_id {
                out.push_str(FIELD_SEP);
                out.push_str("@goal:");
                out.push_str(goal);
            }
            if !item.parents.is_empty() {
                out.push_str(FIELD_SEP);
                out.push_str("@parents:");
                out.push_str(&item.parents.join(","));
            }
            // F3-4-3 (ADR 0041): `@paths:` ao lado de `@parents:` — emitido só quando não-vazio, então
            // o item legado (paths default) produz a MESMA linha de antes (round-trip byte-a-byte).
            if !item.paths.is_empty() {
                out.push_str(FIELD_SEP);
                out.push_str("@paths:");
                out.push_str(&item.paths.join(","));
            }
            if !item.acceptance.is_empty() {
                out.push_str(FIELD_SEP);
                out.push_str("@accept:");
                out.push_str(&render_acceptance(&item.acceptance));
            }
            if item.budget_tokens != 0 {
                out.push_str(FIELD_SEP);
                out.push_str("@budget:");
                out.push_str(&item.budget_tokens.to_string());
            }
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

    // Campos por ` :: `. desc pode conter `::`, então id=primeiro; os campos de PONTA (owner,
    // status) e os sufixos opcionais (spec 52 §2) são lidos do fim; desc = miolo re-unido.
    let mut parts: Vec<&str> = rest.split(FIELD_SEP).collect();

    // F3-1 (spec 52 §2): peça os sufixos OPCIONAIS a partir do FIM. A varredura PARA no primeiro
    // campo que não casa um prefixo de sufixo — que é sempre o `status:` obrigatório. Assim um
    // segmento da `desc` parecido com sufixo (ele vive ANTES de @owner/status) nunca é confundido.
    // O piso `> 4` garante que jamais comemos o núcleo (id :: desc :: @owner :: status).
    let mut goal_id: Option<String> = None;
    let mut parents: Vec<String> = Vec::new();
    let mut acceptance: Vec<AcceptanceCriterion> = Vec::new();
    let mut budget_tokens: u64 = 0;
    let mut paths: Vec<String> = Vec::new();
    while parts.len() > 4 {
        let last = parts[parts.len() - 1];
        if let Some(g) = last.strip_prefix("@goal:") {
            goal_id = Some(g.to_string());
        } else if let Some(ps) = last.strip_prefix("@parents:") {
            parents = if ps.is_empty() {
                Vec::new()
            } else {
                ps.split(',').map(str::to_string).collect()
            };
        } else if let Some(ps) = last.strip_prefix("@paths:") {
            // F3-4-3 (ADR 0041): mesma gramática de `@parents:` (lista separada por vírgula).
            paths = if ps.is_empty() {
                Vec::new()
            } else {
                ps.split(',').map(str::to_string).collect()
            };
        } else if let Some(ac) = last.strip_prefix("@accept:") {
            acceptance = parse_acceptance(ac, line)?;
        } else if let Some(b) = last.strip_prefix("@budget:") {
            budget_tokens = b.parse::<u64>().map_err(|_| {
                PlanError::Malformed(format!("@budget nao numerico: {last:?} ({line:?})"))
            })?;
        } else {
            break; // chegou no campo obrigatório `status:` — fim dos sufixos
        }
        parts.pop();
    }

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
        // F3-1 (spec 52 §2): preenchidos pelo peeling acima; ausentes → defaults (linha legada
        // round-trippa byte-a-byte).
        goal_id,
        parents,
        acceptance,
        budget_tokens,
        paths,
    })
}

/// Serializa os critérios de aceite como JSON compacto (convenção da casa — `serde_json`, igual a
/// `goal.rs`), com o separador de campos do plano escapado por [`escape_sep`] para que `desc`/
/// `check_arg` de texto livre nunca quebrem a gramática de linha. Round-trip exato com
/// [`parse_acceptance`].
fn render_acceptance(criteria: &[AcceptanceCriterion]) -> String {
    // Tipos POD (String/Option/enum) → serialização infalível; o `expect` documenta o invariante,
    // mesmo precedente de `goal.rs` (`serde_json::to_value(ev).expect(...)`).
    let json = serde_json::to_string(criteria).expect("AcceptanceCriterion e sempre serializavel");
    escape_sep(&json)
}

/// Inverso de [`render_acceptance`]: desescapa o separador e relê o JSON. JSON malformado vira
/// [`PlanError::Malformed`] (parser rígido — degrada sem corromper).
fn parse_acceptance(raw: &str, line: &str) -> Result<Vec<AcceptanceCriterion>, PlanError> {
    serde_json::from_str(&unescape_sep(raw))
        .map_err(|e| PlanError::Malformed(format!("@accept invalido ({e}): {line:?}")))
}

/// Escapa o separador de campos (`FIELD_SEP`) e a barra-invertida de forma REVERSÍVEL, para embutir
/// texto livre numa linha cuja gramática usa ` :: ` como delimitador. `\` → `\\` PRIMEIRO (toda
/// barra vira um par), depois ` :: ` → `\s` (marcador que, por isso, nunca colide com uma barra do
/// conteúdo). [`unescape_sep`] é o inverso exato.
fn escape_sep(s: &str) -> String {
    s.replace('\\', "\\\\").replace(FIELD_SEP, "\\s")
}

/// Inverso de [`escape_sep`]: varre da esquerda consumindo 2 chars por escape (`\\` → `\`,
/// `\s` → ` :: `). Entrada não gerada por nós degrada preservando o literal (sem corromper).
fn unescape_sep(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('s') => out.push_str(FIELD_SEP),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
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
            paths: Vec::new(),
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
            paths: Vec::new(),
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
            paths: Vec::new(),
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
            paths: Vec::new(),
        });
        let back = Plan::parse(&p.render()).unwrap();
        assert_eq!(back, p);
    }

    /// F3-4-3 (ADR 0041): um item COM `@paths:` round-trippa byte-a-byte (parse → render → texto
    /// idêntico) e a lista é lida correta. O `@paths:` é emitido ao lado de `@parents:`.
    #[test]
    fn item_with_paths_roundtrips_exactly() {
        let canonical = "# Plano — W\n\
            <!-- lina/plan@1 · escritor unico: supervisor · NAO editar a mao -->\n\
            ## Decisoes\n\
            ## Itens\n\
            - [~] T1 :: tela de leads :: @owner:@Front :: status:doing :: @paths:src/ui/leads.rs,src/ui/mod.rs\n";
        let plan = Plan::parse(canonical).expect("parse com @paths");
        let item = plan.itens.iter().find(|i| i.id == "T1").expect("T1");
        assert_eq!(
            item.paths,
            vec!["src/ui/leads.rs".to_string(), "src/ui/mod.rs".to_string()]
        );
        // Round-trip byte-a-byte: o render do parseado é IDÊNTICO ao texto canônico.
        assert_eq!(plan.render(), canonical, "@paths round-trip byte-a-byte");
        // E parse(render(plan)) == plan (identidade estrutural).
        assert_eq!(Plan::parse(&plan.render()).expect("re-parse"), plan);
    }

    /// F3-4-3 (inv #4): item SEM paths (o legado, todos os sufixos default) NÃO emite `@paths:` — a
    /// linha é IDÊNTICA à de antes da F3-4 (replay de plano antigo reserializa byte-a-byte igual).
    #[test]
    fn legacy_item_without_paths_renders_without_paths_suffix() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.try_claim("T1", "@A").unwrap();
        let text = p.render();
        assert!(!text.contains("@paths:"), "sem paths → sem sufixo @paths");
        assert!(text.ends_with("- [~] T1 :: x :: @owner:@A :: status:doing\n"));
        assert!(p.find("T1").expect("T1").paths.is_empty());
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

    // ─────────────────── F3-1 (spec 52 §2): parents · sufixos · round-trip ───────────────────

    use crate::events::CheckKind;

    /// Gate F3-1 (f): `parents:["T1"]` bloqueia `claim T2` até `T1` virar `Done`.
    #[test]
    fn parents_block_claim_until_all_done() {
        let mut p = Plan::new("W");
        p.add_item("T1", "schema").unwrap();
        p.add_item("T2", "api").unwrap();
        p.apply_item_attributed("T2", None, vec!["T1".into()], Vec::new(), 0, Vec::new());

        // T1 ainda Todo → claim T2 recusado, sem mutar o item.
        let err = p.try_claim("T2", "@W").unwrap_err();
        assert!(
            matches!(&err, PlanError::ParentsNotDone { pending, .. } if pending == &vec!["T1".to_string()]),
            "esperava ParentsNotDone[T1], veio {err:?}"
        );
        assert_eq!(
            p.find("T2").unwrap().status,
            ItemState::Todo,
            "rejeicao nao muta"
        );
        assert!(p.find("T2").unwrap().owner.is_none());

        // Conclui T1 → T2 fica reivindicável.
        p.try_claim("T1", "@A").unwrap();
        p.try_check("T1", "@A").unwrap();
        p.try_claim("T2", "@W").unwrap();
        assert_eq!(p.find("T2").unwrap().status, ItemState::Doing);
        assert_eq!(p.find("T2").unwrap().owner.as_deref(), Some("@W"));
    }

    /// `pending` lista SÓ os pais ainda não-`Done` (ordem preservada).
    #[test]
    fn parents_not_done_reports_only_the_pending() {
        let mut p = Plan::new("W");
        for id in ["T1", "T2", "T3"] {
            p.add_item(id, "x").unwrap();
        }
        p.apply_item_attributed(
            "T3",
            None,
            vec!["T1".into(), "T2".into()],
            Vec::new(),
            0,
            Vec::new(),
        );
        p.try_claim("T1", "@A").unwrap();
        p.try_check("T1", "@A").unwrap(); // só T1 Done

        match p.try_claim("T3", "@W").unwrap_err() {
            PlanError::ParentsNotDone { id, pending } => {
                assert_eq!(id, "T3");
                assert_eq!(pending, vec!["T2".to_string()], "T1 ja Done sai da lista");
            }
            other => panic!("esperava ParentsNotDone, veio {other:?}"),
        }
    }

    /// Pai inexistente conta como NÃO-`Done` (fail-safe: não libera despacho prematuro).
    #[test]
    fn missing_parent_counts_as_not_done() {
        let mut p = Plan::new("W");
        p.add_item("T2", "x").unwrap();
        p.apply_item_attributed(
            "T2",
            None,
            vec!["NAO_EXISTE".into()],
            Vec::new(),
            0,
            Vec::new(),
        );
        assert!(matches!(
            p.try_claim("T2", "@W"),
            Err(PlanError::ParentsNotDone { .. })
        ));
    }

    /// A linha LEGADA (todos os campos novos no default) é byte-idêntica à do schema congelado —
    /// nenhum sufixo `@goal`/`@parents`/`@accept`/`@budget` vaza para itens sem esses dados.
    #[test]
    fn legacy_item_line_has_no_suffixes() {
        let mut p = Plan::new("W");
        p.add_item("T1", "faz").unwrap();
        p.try_claim("T1", "@A").unwrap();
        let expected = "# Plano — W\n\
            <!-- lina/plan@1 · escritor unico: supervisor · NAO editar a mao -->\n\
            ## Decisoes\n\
            ## Itens\n\
            - [~] T1 :: faz :: @owner:@A :: status:doing\n";
        assert_eq!(p.render(), expected);
    }

    /// Só os campos PRESENTES são emitidos (parents sem goal/accept/budget) — e round-trippam.
    #[test]
    fn only_present_fields_are_emitted() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.apply_item_attributed(
            "T1",
            None,
            vec!["P1".into(), "P2".into()],
            Vec::new(),
            0,
            Vec::new(),
        );
        let text = p.render();
        let line = text.lines().last().unwrap();
        assert_eq!(
            line,
            "- [ ] T1 :: x :: @owner:? :: status:todo :: @parents:P1,P2"
        );
        assert_eq!(Plan::parse(&text).unwrap(), p);
    }

    /// Round-trip EXATO com os quatro campos preenchidos (modelo + texto byte-a-byte).
    #[test]
    fn goal_parents_accept_budget_roundtrip() {
        let mut p = Plan::new("W");
        p.add_item("T1", "raiz").unwrap();
        p.add_item("T2", "trabalho").unwrap();
        p.apply_item_attributed(
            "T2",
            Some("G1".into()),
            vec!["T1".into()],
            vec![AcceptanceCriterion {
                desc: "compila sem warnings".into(),
                check_kind: CheckKind::Command,
                check_arg: Some("cargo clippy".into()),
            }],
            4096,
            Vec::new(),
        );
        let text = p.render();
        let back = Plan::parse(&text).expect("parse do proprio render");
        assert_eq!(back, p, "modelo round-trip");
        assert_eq!(back.render(), text, "texto round-trip byte-a-byte");

        let t2 = back.find("T2").unwrap();
        assert_eq!(t2.goal_id.as_deref(), Some("G1"));
        assert_eq!(t2.parents, vec!["T1".to_string()]);
        assert_eq!(t2.budget_tokens, 4096);
        assert_eq!(t2.acceptance[0].desc, "compila sem warnings");
        assert_eq!(t2.acceptance[0].check_kind, CheckKind::Command);
    }

    /// F3-4-3 (ADR 0041): a ATRIBUIÇÃO (ponto de entrada do event-sourcing — `PlanItemAttributed`)
    /// seta os `paths`, e eles sobrevivem ao render→parse. Prova que os paths chegam à projeção pelo
    /// log (não só pelo parse de um plano.md à mão).
    #[test]
    fn apply_item_attributed_sets_paths_and_roundtrips() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.apply_item_attributed(
            "T1",
            None,
            Vec::new(),
            Vec::new(),
            0,
            vec!["src/a.rs".into(), "src/b.rs".into()],
        );
        let it = p.find("T1").expect("T1");
        assert_eq!(
            it.paths,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
        let back = Plan::parse(&p.render()).expect("re-parse");
        assert_eq!(back.find("T1").expect("T1").paths, it.paths);
    }

    /// Texto livre do `acceptance` com o PRÓPRIO separador (` :: `) e barra-invertida round-trippa —
    /// prova o escape reversível do `FIELD_SEP`.
    #[test]
    fn accept_with_field_sep_and_backslash_roundtrips() {
        let mut p = Plan::new("W");
        p.add_item("T1", "x").unwrap();
        p.apply_item_attributed(
            "T1",
            None,
            Vec::new(),
            vec![AcceptanceCriterion {
                desc: "passo A :: passo B".into(),
                check_kind: CheckKind::TestPass,
                check_arg: Some(r"glob\com\barra :: e separador".into()),
            }],
            0,
            Vec::new(),
        );
        let text = p.render();
        let back = Plan::parse(&text).expect("parse");
        assert_eq!(back, p, "modelo round-trip");
        assert_eq!(back.render(), text, "texto round-trip byte-a-byte");

        let c = &back.find("T1").unwrap().acceptance[0];
        assert_eq!(c.desc, "passo A :: passo B");
        assert_eq!(
            c.check_arg.as_deref(),
            Some(r"glob\com\barra :: e separador")
        );
    }

    /// Um segmento da `desc` que PARECE um sufixo (`@budget:99`) fica protegido no miolo: a varredura
    /// de sufixos para no `status:`, antes de alcançá-lo.
    #[test]
    fn desc_resembling_suffix_is_not_parsed_as_one() {
        let mut p = Plan::new("W");
        p.add_item("T1", "ver :: @budget:99 no doc").unwrap();
        p.try_claim("T1", "@A").unwrap();
        let text = p.render();
        let back = Plan::parse(&text).unwrap();
        let t1 = back.find("T1").unwrap();
        assert_eq!(t1.desc, "ver :: @budget:99 no doc");
        assert_eq!(t1.budget_tokens, 0, "o @budget:99 da desc nao virou campo");
        assert_eq!(back.render(), text, "round-trip byte-a-byte");
    }

    /// `@accept` com JSON malformado é REJEITADO pelo parser rígido (não corrompe silenciosamente).
    #[test]
    fn parse_rejects_malformed_accept() {
        let bad = "# Plano — W\n\
            <!-- lina/plan@1 -->\n\
            ## Decisoes\n\
            ## Itens\n\
            - [ ] T1 :: x :: @owner:? :: status:todo :: @accept:{isto-nao-e-json}\n";
        assert!(matches!(Plan::parse(bad), Err(PlanError::Malformed(_))));
    }

    /// `@budget` não-numérico é rejeitado (rigidez do parser).
    #[test]
    fn parse_rejects_non_numeric_budget() {
        let bad = "# Plano — W\n\
            <!-- lina/plan@1 -->\n\
            ## Decisoes\n\
            ## Itens\n\
            - [ ] T1 :: x :: @owner:? :: status:todo :: @budget:abc\n";
        assert!(matches!(Plan::parse(bad), Err(PlanError::Malformed(_))));
    }
}
