//! F4-0-4 · frente CONTEXTO (dono: Terminal M) — **pré-config de ferramentas/grupos por projeto**.
//!
//! STUB DA LARGADA F4-0 (preencher nesta frente). Prima concreta das "pistas" (`clue.rs`, F3-5-6):
//! o leigo DECLARA, por projeto, quais ferramentas/grupos a IA pode enxergar (doc-fonte 63) — vira
//! **contexto declarado** (Meadows ponto [6] Fluxo de informação), que uma camada do briefing lê.
//!
//! ## O que esta frente entrega (critérios F4-0-4, épico 41 §III)
//! - **projeção `ToolScopeSet`** — por replay de `ToolScopeDeclared`/`ToolScopeRevoked` (padrão
//!   `ClueSet`: o último vence; `Revoked` RETRAI a chave, sem variante de remoção extra).
//! - **`declare_tool_scope(...)` / `revoke_tool_scope(...)`** — emitem os eventos (congelados na
//!   largada). Remover a declaração → o acesso some **no próximo turno** (push de evento, sem
//!   restart — doc 40 §9).
//! - **camada de briefing** que injeta "você me deu acesso a <ferramenta/grupo X>" no contexto do
//!   terminal daquele projeto (coordenar com `briefing.rs` — costura de I; ver despacho).
//!
//! ## Invariante-mãe (red-team do gate re-deriva no código)
//! - **Declarar ≠ AUTORIZAR.** Declarar uma ferramenta a EXPÕE à leitura da IA (fluxo de contexto);
//!   **não** autoriza ação externa — essa continua passando pelo broker (`broker.rs`, F4-0-3) +
//!   gate humano + custódia (ADR 0004). Nenhum campo de `ToolScopeDeclared` entra no caminho de
//!   identidade/permissão (ponto [6] regra de fronteira: mostrar ≠ autorizar).
//! - **default-deny:** ferramenta/grupo NÃO declarado é INVISÍVEL à IA (espelha ADR 0006).
//! - **Projeção do log (inv #4):** `from_records` PURA; replay reconstrói byte-a-byte; eventos
//!   indecodificáveis são pulados (derivada, não validadora — espelha `Clue`/`Mentality`).
//!
//! Eventos: `DomainEvent::ToolScopeDeclared`/`ToolScopeRevoked { project, channel, scope }`
//! (congelados na largada — não re-editar `events.rs`).

use std::collections::{BTreeMap, BTreeSet};

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};

/// Uma ferramenta/grupo que um projeto declarou enxergar: o par `(canal, escopo)`. DADO de contexto —
/// informa o que a IA OLHA, **nunca** concede autoridade de ação (a externa segue pelo broker + gate).
/// Ordenável por `(channel, scope)` → conjunto e briefing determinísticos.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeclaredTool {
    /// Canal a que a ferramenta/grupo pertence (ex.: `"whatsapp"`).
    pub channel: String,
    /// A ferramenta/grupo declarado (ex.: `"grupo:Vendas"`).
    pub scope: String,
}

/// Projeção das ferramentas/grupos declarados por PROJETO.
///
/// Prima das pistas ([`crate::clue::ClueSet`]), mas com álgebra distinta: lá a chave (`scope`) tem
/// valor rico e o último-vence; aqui a unidade é a EXISTÊNCIA da tripla `(project, channel, scope)`.
/// `ToolScopeDeclared` adiciona o par `(channel, scope)` ao projeto; `ToolScopeRevoked` o retira — o
/// último evento que toca a tripla vence. `BTreeMap`/`BTreeSet` dão ordem estável → fingerprint
/// determinístico para o replay e o briefing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolScopeSet {
    by_project: BTreeMap<String, BTreeSet<DeclaredTool>>,
}

impl ToolScopeSet {
    /// Reconstrói a projeção do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log: cada `ToolScopeDeclared`
    /// adiciona o par ao projeto, cada `ToolScopeRevoked` o retira. Sem relógio nem I/O — testável com
    /// log sintético.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut by_project: BTreeMap<String, BTreeSet<DeclaredTool>> = BTreeMap::new();
        for record in records {
            // Registro indecodificável (versão futura): a projeção é DERIVADA, não validadora do log
            // (espelha `Clue`/`Mentality`) — pula sem panicar.
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            match event {
                DomainEvent::ToolScopeDeclared {
                    project,
                    channel,
                    scope,
                } => {
                    by_project
                        .entry(project)
                        .or_default()
                        .insert(DeclaredTool { channel, scope });
                }
                DomainEvent::ToolScopeRevoked {
                    project,
                    channel,
                    scope,
                } => {
                    // Retira o par; se o projeto ficou sem nenhuma declaração, retrai a chave — declarar
                    // e depois revogar tem de ser idêntico a nunca declarar (projeção não guarda projeto
                    // vazio), senão `is_empty`/`projects` mentiriam.
                    let mut emptied = false;
                    if let Some(tools) = by_project.get_mut(&project) {
                        tools.remove(&DeclaredTool { channel, scope });
                        emptied = tools.is_empty();
                    }
                    if emptied {
                        by_project.remove(&project);
                    }
                }
                _ => {}
            }
        }
        Self { by_project }
    }

    /// As ferramentas/grupos declarados de `project`, em ordem estável (vazio se nenhum). Gating por
    /// projeto: o terminal só enxerga as declarações do SEU projeto (o caller passa o projeto do nó).
    #[must_use]
    pub fn tools_for(&self, project: &str) -> Vec<&DeclaredTool> {
        self.by_project
            .get(project)
            .map(|tools| tools.iter().collect())
            .unwrap_or_default()
    }

    /// `true` se a tripla `(project, channel, scope)` está declarada AGORA — a forma direta da
    /// invariante **default-deny**: não declarado ⇒ `false` (invisível à IA). Compara sem alocar.
    #[must_use]
    pub fn is_declared(&self, project: &str, channel: &str, scope: &str) -> bool {
        self.by_project.get(project).is_some_and(|tools| {
            tools
                .iter()
                .any(|t| t.channel == channel && t.scope == scope)
        })
    }

    /// `true` se nenhum projeto tem ferramenta declarada.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_project.is_empty()
    }

    /// Os projetos com ao menos uma declaração ativa, em ordem estável.
    pub fn projects(&self) -> impl Iterator<Item = &str> {
        self.by_project.keys().map(String::as_str)
    }
}

// ─────────────────── handlers do verbo (Maestro fia o dispatch) ───────────────────

/// Declara uma ferramenta/grupo para um projeto: emite `ToolScopeDeclared`. A declaração EXPÕE a
/// ferramenta à leitura da IA (fluxo de contexto) — **não autoriza** ação externa (essa continua
/// passando pelo broker + gate humano + custódia). Os campos são DADO; nenhum entra no caminho de
/// identidade/permissão. `trim` normaliza o input do leigo para casar com o `revoke` correspondente.
///
/// # Errors
/// Falha ao persistir o evento.
pub fn declare_tool_scope(
    store: &mut EventStore,
    project: &str,
    channel: &str,
    scope: &str,
) -> Result<u64, StoreError> {
    store.append(&DomainEvent::ToolScopeDeclared {
        project: project.trim().to_string(),
        channel: channel.trim().to_string(),
        scope: scope.trim().to_string(),
    })
}

/// Revoga a declaração de uma ferramenta/grupo de um projeto: emite `ToolScopeRevoked`. No próximo
/// replay a tripla some da projeção → o acesso desaparece do briefing **sem restart** (push de evento).
///
/// # Errors
/// Falha ao persistir o evento.
pub fn revoke_tool_scope(
    store: &mut EventStore,
    project: &str,
    channel: &str,
    scope: &str,
) -> Result<u64, StoreError> {
    store.append(&DomainEvent::ToolScopeRevoked {
        project: project.trim().to_string(),
        channel: channel.trim().to_string(),
        scope: scope.trim().to_string(),
    })
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Constrói o `EventRecord` serializado COMO no log: a tag interna `event` (enum
    /// `#[serde(tag = "event")]`) precisa estar no payload, senão `from_record` não decodifica — por
    /// isso serializamos o `DomainEvent` REAL via `kind`/`current_version`/`serde_json`, nunca à mão.
    fn record_of(seq: u64, event: &DomainEvent) -> EventRecord {
        EventRecord {
            seq,
            ts: seq, // irrelevante para a projeção (a ordem de log decide).
            kind: event.kind().to_string(),
            version: event.current_version(),
            payload: serde_json::to_value(event).expect("serializa o evento"),
        }
    }

    fn declared(seq: u64, project: &str, channel: &str, scope: &str) -> EventRecord {
        record_of(
            seq,
            &DomainEvent::ToolScopeDeclared {
                project: project.into(),
                channel: channel.into(),
                scope: scope.into(),
            },
        )
    }

    fn revoked(seq: u64, project: &str, channel: &str, scope: &str) -> EventRecord {
        record_of(
            seq,
            &DomainEvent::ToolScopeRevoked {
                project: project.into(),
                channel: channel.into(),
                scope: scope.into(),
            },
        )
    }

    /// Critério (c): ferramenta declarada → presente na projeção do seu projeto.
    #[test]
    fn declared_tool_is_visible_for_its_project() {
        let scopes = ToolScopeSet::from_records(&[declared(1, "loja", "whatsapp", "grupo:Vendas")]);
        assert!(scopes.is_declared("loja", "whatsapp", "grupo:Vendas"));
        let tools = scopes.tools_for("loja");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].channel, "whatsapp");
        assert_eq!(tools[0].scope, "grupo:Vendas");
    }

    /// default-deny: ferramenta/canal/projeto NÃO declarado é invisível (espelha ADR 0006).
    #[test]
    fn undeclared_is_invisible_default_deny() {
        let scopes = ToolScopeSet::from_records(&[declared(1, "loja", "whatsapp", "grupo:Vendas")]);
        assert!(
            !scopes.is_declared("loja", "whatsapp", "grupo:Suporte"),
            "escopo não declarado é invisível"
        );
        assert!(
            !scopes.is_declared("loja", "instagram", "grupo:Vendas"),
            "canal não declarado é invisível"
        );
        assert!(
            !scopes.is_declared("outro", "whatsapp", "grupo:Vendas"),
            "projeto não declarado é invisível"
        );
        assert!(
            scopes.tools_for("outro").is_empty(),
            "projeto sem declaração → lista vazia"
        );
    }

    /// Gating por projeto: um projeto só vê as próprias declarações (zero vazamento entre projetos).
    #[test]
    fn declarations_are_scoped_by_project_no_leak() {
        let scopes = ToolScopeSet::from_records(&[
            declared(1, "loja", "whatsapp", "grupo:Vendas"),
            declared(2, "blog", "instagram", "perfil:Editorial"),
        ]);
        assert!(scopes.is_declared("loja", "whatsapp", "grupo:Vendas"));
        assert!(scopes.is_declared("blog", "instagram", "perfil:Editorial"));
        assert!(
            !scopes.is_declared("loja", "instagram", "perfil:Editorial"),
            "'loja' não enxerga a declaração de 'blog'"
        );
        assert_eq!(scopes.tools_for("loja").len(), 1);
        assert_eq!(scopes.tools_for("blog").len(), 1);
    }

    /// Critério (c): revogar → some no próximo replay; declarar+revogar ≡ nunca declarar (chave retraída).
    #[test]
    fn revoked_tool_disappears_on_next_replay() {
        let scopes = ToolScopeSet::from_records(&[
            declared(1, "loja", "whatsapp", "grupo:Vendas"),
            revoked(2, "loja", "whatsapp", "grupo:Vendas"),
        ]);
        assert!(
            !scopes.is_declared("loja", "whatsapp", "grupo:Vendas"),
            "revogada → invisível"
        );
        assert!(scopes.tools_for("loja").is_empty());
        assert!(
            scopes.is_empty(),
            "projeto sem declaração ativa não permanece na projeção"
        );
    }

    /// Revogar UMA de duas declarações do projeto → a outra permanece intacta.
    #[test]
    fn revoking_one_keeps_the_others() {
        let scopes = ToolScopeSet::from_records(&[
            declared(1, "loja", "whatsapp", "grupo:Vendas"),
            declared(2, "loja", "whatsapp", "grupo:Suporte"),
            revoked(3, "loja", "whatsapp", "grupo:Vendas"),
        ]);
        assert!(!scopes.is_declared("loja", "whatsapp", "grupo:Vendas"));
        assert!(
            scopes.is_declared("loja", "whatsapp", "grupo:Suporte"),
            "a outra declaração permanece"
        );
        let tools = scopes.tools_for("loja");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].scope, "grupo:Suporte");
    }

    /// O último evento sobre a tripla vence: re-declarar após revogar reativa, sem duplicar.
    #[test]
    fn redeclare_after_revoke_reactivates() {
        let scopes = ToolScopeSet::from_records(&[
            declared(1, "loja", "whatsapp", "grupo:Vendas"),
            revoked(2, "loja", "whatsapp", "grupo:Vendas"),
            declared(3, "loja", "whatsapp", "grupo:Vendas"),
        ]);
        assert!(scopes.is_declared("loja", "whatsapp", "grupo:Vendas"));
        assert_eq!(scopes.tools_for("loja").len(), 1, "conjunto, não duplica");
    }

    /// Declarar a MESMA tripla duas vezes é idempotente (conjunto, não lista).
    #[test]
    fn duplicate_declaration_is_idempotent() {
        let scopes = ToolScopeSet::from_records(&[
            declared(1, "loja", "whatsapp", "grupo:Vendas"),
            declared(2, "loja", "whatsapp", "grupo:Vendas"),
        ]);
        assert_eq!(scopes.tools_for("loja").len(), 1);
    }

    /// A projeção é SELETIVA: eventos de outro tipo (ex.: uma pista) não afetam o tool-scope.
    #[test]
    fn unrelated_events_are_ignored() {
        let scopes = ToolScopeSet::from_records(&[
            record_of(
                1,
                &DomainEvent::ClueSetDefined {
                    scope: "loja".into(),
                    paths: vec!["src/".into()],
                    label: None,
                },
            ),
            declared(2, "loja", "whatsapp", "grupo:Vendas"),
        ]);
        assert_eq!(
            scopes.tools_for("loja").len(),
            1,
            "só o tool-scope conta para esta projeção"
        );
        assert!(scopes.is_declared("loja", "whatsapp", "grupo:Vendas"));
    }

    /// Replay determinístico (inv #4): mesma sequência de eventos → projeção idêntica.
    #[test]
    fn replay_is_deterministic() {
        let log = [
            declared(1, "a", "c1", "s1"),
            declared(2, "b", "c2", "s2"),
            revoked(3, "a", "c1", "s1"),
        ];
        assert_eq!(
            ToolScopeSet::from_records(&log),
            ToolScopeSet::from_records(&log)
        );
    }

    /// `projects()` lista os projetos com declaração ativa, em ordem estável (`BTreeMap`).
    #[test]
    fn projects_lists_active_in_stable_order() {
        let scopes = ToolScopeSet::from_records(&[
            declared(1, "zeta", "c", "s"),
            declared(2, "alfa", "c", "s"),
        ]);
        let projects: Vec<&str> = scopes.projects().collect();
        assert_eq!(projects, ["alfa", "zeta"]);
    }

    /// End-to-end pelos handlers: declarar via [`declare_tool_scope`] → replay do store real → presente;
    /// revogar via [`revoke_tool_scope`] → replay → ausente. Prova a costura emit→projeção (não só o
    /// `from_records` puro), espelhando o teste de log real do `Mentality`.
    #[test]
    fn handlers_emit_and_projection_reflects_them() {
        let dir = std::env::temp_dir().join(format!(
            "lina-tool-scope-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(&dir).expect("abre store temporário");

        declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
        let after_declare = ToolScopeSet::replay(&store).expect("replay pós-declaração");
        assert!(
            after_declare.is_declared("loja", "whatsapp", "grupo:Vendas"),
            "declaração aparece na projeção do log real"
        );

        revoke_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("revoga");
        let after_revoke = ToolScopeSet::replay(&store).expect("replay pós-revogação");
        assert!(
            !after_revoke.is_declared("loja", "whatsapp", "grupo:Vendas"),
            "revogação some da projeção sem restart"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `trim` normaliza o input: declarar com espaços e revogar sem casam a mesma tripla.
    #[test]
    fn declare_and_revoke_normalize_whitespace() {
        let dir = std::env::temp_dir().join(format!(
            "lina-tool-scope-trim-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(&dir).expect("abre store temporário");

        declare_tool_scope(&mut store, " loja ", " whatsapp ", " grupo:Vendas ").expect("declara");
        assert!(
            ToolScopeSet::replay(&store).expect("replay").is_declared(
                "loja",
                "whatsapp",
                "grupo:Vendas"
            ),
            "espaços do leigo são normalizados na declaração"
        );

        revoke_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("revoga");
        assert!(
            !ToolScopeSet::replay(&store).expect("replay").is_declared(
                "loja",
                "whatsapp",
                "grupo:Vendas"
            ),
            "revoke sem espaços casa a declaração com espaços (trim simétrico)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
