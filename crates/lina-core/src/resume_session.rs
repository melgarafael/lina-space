//! F3-5-1/-2 · frente SESSÕES (dono: Terminal B / Ultra Code) — `ResumeSessionStore`.
//!
//! O buffer de sessões `--resume` (spec 53 §11.B): fecha "salvar sessões do Claude" **E** "Retome o
//! que estava fazendo" (restore conversacional pós-crash). Projeção event-sourced PURA (molde
//! [`crate::router::CostLedger`]/[`crate::code::CodeIntegration`], inv #4/#1 — ZERO LLM): a sessão
//! "ativa" de um nó = um `SessionPersisted` sem `SessionRetired` posterior para o mesmo `session_id`.
//!
//! Três responsabilidades, todas determinísticas:
//!   1. [`ResumeSessionStore::replay`] — reconstrói as sessões salvas ativas varrendo o event log.
//!   2. [`ResumeSessionStore::plan_persist`] — a REGRA DE ADMISSÃO/PODA (PURA): dado o estado atual e
//!      uma sessão a salvar (1º prompt observado), decide quais eventos emitir — o `SessionPersisted`
//!      e eventuais `SessionRetired` (`cli_invalidated` na rotação `/resume`, `lru` ao estourar o cap).
//!   3. [`resume_command`] — monta o comando de religação COM o `session_id` exato (F3-5-2), o que
//!      torna "Retome o que estava fazendo" preciso (a sessão certa, não "a última do diretório").
//!
//! `session_id` é PROJEÇÃO (ponteiro de `--resume` validado pelo funil `admit_node`, ADR 0022),
//! JAMAIS identidade — forjá-lo no log não confere nada (a identidade segue `LINA_NODE_ID`, ADR 0026).
//! NÃO confundir com o `SessionStore`-CÂMERA (pan/zoom, `session.rs:68`, ADR 0029): domínio distinto,
//! nome distinto, não tocado.

use std::collections::BTreeMap;

use crate::events::{DomainEvent, EventRecord, EventStore, StoreError};
use crate::CliProfile;

/// Cap default de sessões salvas ativas por workspace (spec 53 §11.B, faixa 0..=500; `0` = ilimitado).
/// Provisório como tunável (futuro `RouterConfig::session_store_max_entries`); o caller o injeta na
/// admissão para a poda LRU ser reprodutível por replay (não depende de constante embutida no log).
pub const DEFAULT_SESSION_STORE_MAX_ENTRIES: usize = 50;

/// Uma sessão `--resume` salva e ATIVA (projeção). Espelha o schema da spec 53 §11.B — o que o modal
/// "Sessões salvas" lista e o que o restore consome para montar o comando de religação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSession {
    /// `node_id` do nó cuja sessão foi salva (binding server-side, nunca do payload).
    pub node: String,
    /// `id` do CLI Profile de origem (os `resume_args` vivem lá — `lina-cli-profiles`).
    pub cli: String,
    /// Id capturado do JSONL pelo `lina-session-watch` — DADO, jamais autoridade.
    pub session_id: String,
    /// Nome amigável p/ o leigo escolher no modal (quando o caller o forneceu).
    pub label: Option<String>,
    /// Carimbo da persistência — chave da ordenação LRU.
    pub persisted_at_ms: u64,
}

/// Projeção das sessões `--resume` salvas e ativas. Igualdade estrutural prova o replay idêntico
/// (gate j da onda). Ativas = `SessionPersisted` − `SessionRetired` posterior, por `session_id`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeSessionStore {
    /// Sessões ativas, ordenadas por `persisted_at_ms` ASC (mais antiga primeiro → candidata natural
    /// do LRU); empate desfeito por `session_id` (determinístico). O modal pode reverter para exibir.
    pub sessions: Vec<SavedSession>,
}

impl ResumeSessionStore {
    /// Reconstrói as sessões salvas ativas do `EventStore` por replay.
    ///
    /// # Errors
    /// Falha ao ler o event log.
    pub fn replay(store: &EventStore) -> Result<Self, StoreError> {
        Ok(Self::from_records(&store.events()?))
    }

    /// **Coração determinístico (PURO).** Varre os registros em ordem de log: cada `SessionPersisted`
    /// (re)ativa a sessão do seu `session_id`; cada `SessionRetired` a desativa. Testável com log
    /// sintético. Versão futura/indecodificável é PULADA (a projeção é derivada, não validadora do
    /// log — espelha `CodeIntegration::from_records`), nunca panica.
    #[must_use]
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut active: BTreeMap<String, SavedSession> = BTreeMap::new();
        for record in records {
            let Ok(event) =
                DomainEvent::from_record(&record.kind, record.version, record.payload.clone())
            else {
                continue;
            };
            match event {
                DomainEvent::SessionPersisted {
                    node,
                    cli,
                    session_id,
                    label,
                    persisted_at_ms,
                    ..
                } => {
                    active.insert(
                        session_id.clone(),
                        SavedSession {
                            node,
                            cli,
                            session_id,
                            label,
                            persisted_at_ms,
                        },
                    );
                }
                DomainEvent::SessionRetired { session_id, .. } => {
                    active.remove(&session_id);
                }
                _ => {}
            }
        }
        let mut sessions: Vec<SavedSession> = active.into_values().collect();
        sessions.sort_by(|a, b| {
            a.persisted_at_ms
                .cmp(&b.persisted_at_ms)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        Self { sessions }
    }

    /// A sessão ativa MAIS RECENTE de um nó — a relevante para o restore (após a rotação
    /// `cli_invalidated` há no máximo uma; `max_by_key` é a desambiguação defensiva). `None` ⇒ o nó
    /// renasce do zero (badge "novo começo").
    #[must_use]
    pub fn active_for(&self, node: &str) -> Option<&SavedSession> {
        self.sessions
            .iter()
            .filter(|s| s.node == node)
            .max_by_key(|s| s.persisted_at_ms)
    }

    /// **A REGRA DE ADMISSÃO/PODA (PURA, spec 53 §11.B).** Chamada quando o shell observa o 1º prompt
    /// de um nó (via `lina-session-watch`): decide os eventos a apendar para registrar a sessão
    /// `session_id` do nó `node` (CLI `cli`), aplicando as duas podas declaradas:
    /// - **`cli_invalidated`**: re-persistir o MESMO nó com um `session_id` NOVO (rotação `/resume`)
    ///   aposenta a(s) sessão(ões) anterior(es) daquele nó — só uma fica ativa por nó.
    /// - **`lru`**: ao estourar `max_entries` ativas no workspace, as MAIS ANTIGAS por
    ///   `persisted_at_ms` saem. A recém-persistida JAMAIS é podada no próprio ato (protegida do
    ///   conjunto podável) — senão um relógio não-monotônico podaria a sessão que acabou de nascer.
    ///
    /// Idempotente: se `session_id` já está ativo, devolve `[]` (o 1º prompt já foi registrado — não
    /// re-emite a cada poll do watch). `max_entries == 0` ⇒ cap desligado (sem poda LRU).
    /// NÃO apenda nada — o caller persiste a sequência (mantém a função testável sem I/O).
    #[must_use]
    pub fn plan_persist(
        &self,
        node: &str,
        cli: &str,
        session_id: &str,
        label: Option<String>,
        now_ms: u64,
        max_entries: usize,
    ) -> Vec<DomainEvent> {
        // Idempotência: o 1º prompt desta sessão já foi registrado → nada a fazer.
        if self.sessions.iter().any(|s| s.session_id == session_id) {
            return Vec::new();
        }

        let mut events = Vec::new();

        // Rotação `/resume` (cli_invalidated): o nó troca de sessão → as anteriores DELE saem.
        for superseded in self
            .sessions
            .iter()
            .filter(|s| s.node == node && s.session_id != session_id)
        {
            events.push(DomainEvent::SessionRetired {
                node: superseded.node.clone(),
                session_id: superseded.session_id.clone(),
                reason: "cli_invalidated".to_string(),
                retired_at_ms: now_ms,
            });
        }

        // O fato central: a sessão ficou retomável (a invariante "só após ≥1 prompt" é o contrato
        // de quem chama — `after_first_prompt` fica `true`, legível e auditável no log).
        events.push(DomainEvent::SessionPersisted {
            node: node.to_string(),
            cli: cli.to_string(),
            session_id: session_id.to_string(),
            after_first_prompt: true,
            first_prompt_at_ms: now_ms,
            label,
            persisted_at_ms: now_ms,
        });

        // Poda LRU: candidatas = ativas PRÉ-existentes que sobrevivem ao passo de rotação (a nova NÃO
        // entra no conjunto podável). Estoura o cap ⇒ as mais antigas saem.
        if max_entries > 0 {
            let mut prunable: Vec<&SavedSession> = self
                .sessions
                .iter()
                .filter(|s| !(s.node == node && s.session_id != session_id))
                .collect();
            let total_after = prunable.len() + 1; // +1 = a sessão recém-persistida
            if total_after > max_entries {
                prunable.sort_by(|a, b| {
                    a.persisted_at_ms
                        .cmp(&b.persisted_at_ms)
                        .then_with(|| a.session_id.cmp(&b.session_id))
                });
                let overflow = total_after - max_entries;
                for victim in prunable.into_iter().take(overflow) {
                    events.push(DomainEvent::SessionRetired {
                        node: victim.node.clone(),
                        session_id: victim.session_id.clone(),
                        reason: "lru".to_string(),
                        retired_at_ms: now_ms,
                    });
                }
            }
        }

        events
    }

    /// Conveniência sobre [`Self::plan_persist`]: projeta o store, planeja a admissão e apenda os
    /// eventos resultantes. É o ponto que o caller (1º prompt observado) usa em produção; devolve os
    /// eventos apendados para a narração/observabilidade.
    ///
    /// # Errors
    /// Falha ao ler ou apendar no event log.
    pub fn persist_after_first_prompt(
        store: &mut EventStore,
        node: &str,
        cli: &str,
        session_id: &str,
        label: Option<String>,
        now_ms: u64,
        max_entries: usize,
    ) -> Result<Vec<DomainEvent>, StoreError> {
        let events =
            Self::replay(store)?.plan_persist(node, cli, session_id, label, now_ms, max_entries);
        for event in &events {
            store.append(event)?;
        }
        Ok(events)
    }
}

/// **F3-5-2: o comando de religação COM a sessão exata.** `[program, args…, resume_args…, session_id]`
/// quando o CLI sabe retomar (`can_resume()`) E há um `session_id` salvo; senão "novo começo"
/// (`[program, args…]`). O verbo de resume vem do TOML (inv #3 — trocá-lo muda o comando sem
/// recompilar); o `session_id` é DADO capturado, anexado para retomar a conversa CERTA (não "a última
/// do diretório", que é o que `claude --resume` sem id faria). Espelha e estende
/// `resume_verb_comes_from_toml_not_code` (`lina-cli-profiles`).
#[must_use]
pub fn resume_command(profile: &CliProfile, session_id: Option<&str>) -> Vec<String> {
    let mut cmd = vec![profile.program.clone()];
    cmd.extend(profile.args.iter().cloned());
    // Declarar resume é NECESSÁRIO, nunca SUFICIENTE (copy-f1-4 §4): só retoma se há sessão de fato.
    if let Some(sid) = session_id {
        if profile.can_resume() {
            cmd.extend(profile.resume_args.iter().cloned());
            cmd.push(sid.to_string());
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventStore;

    /// Store temporário (event log em disco) — descartado no Drop. `thread::id` no nome mata o flaky
    /// sistêmico de colisão de tmpdir no gate paralelo (achado F3-CONF-2).
    struct TmpStore {
        dir: std::path::PathBuf,
        store: EventStore,
    }
    impl TmpStore {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lina-resume-session-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let store = EventStore::open(&dir).expect("abrir store");
            Self { dir, store }
        }
    }
    impl Drop for TmpStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Conta eventos de um `kind` (com filtro opcional por `reason` de `SessionRetired`) no log.
    fn count_kind(store: &EventStore, kind: &str, reason: Option<&str>) -> usize {
        store
            .events()
            .expect("ler log")
            .iter()
            .filter(|r| r.kind == kind)
            .filter(|r| match reason {
                Some(want) => r.payload.get("reason").and_then(|v| v.as_str()) == Some(want),
                None => true,
            })
            .count()
    }

    /// Profile claude-code mínimo que SABE retomar (resume_args não-vazio) — fixture do comando.
    fn claude_profile() -> CliProfile {
        CliProfile::from_toml_str(
            r#"
                id = "claude-code"
                program = "claude"
                args = ["--output-format", "stream-json"]
                resume_args = ["--resume"]
                delivery = "session_resume"
                prompt_ready_regex = "> "
                session_dir_pattern = "~/.claude/projects/*/*.jsonl"
                [end_signal]
                kind = "idle"
            "#,
            "<claude>",
        )
        .expect("claude-code deve parsear")
    }

    /// Profile shell puro que NÃO retoma (resume_args vazio) — prova "novo começo".
    fn shell_profile() -> CliProfile {
        CliProfile::from_toml_str(
            r#"
                id = "shell"
                program = "bash"
                delivery = "pty_inject"
                prompt_ready_regex = '\$\s$'
                [end_signal]
                kind = "idle"
            "#,
            "<shell>",
        )
        .expect("shell deve parsear")
    }

    /// **Gate (a) — 0 prompts ⇒ 0 SessionPersisted.** Um terminal que nunca recebeu prompt não tem
    /// sessão salva: o caller simplesmente não chama a admissão. A projeção é vazia.
    #[test]
    fn zero_prompts_zero_sessions() {
        let ts = TmpStore::new("zero");
        let store = ResumeSessionStore::replay(&ts.store).expect("replay");
        assert!(store.sessions.is_empty(), "sem prompt ⇒ sem sessão salva");
        assert_eq!(count_kind(&ts.store, "SessionPersisted", None), 0);
    }

    /// **Gate (a) — 1 prompt ⇒ exatamente 1 SessionPersisted.** A admissão do 1º prompt emite um
    /// único evento; re-chamar com o MESMO `session_id` é idempotente (não polui o log a cada poll).
    #[test]
    fn one_prompt_one_session_idempotent() {
        let mut ts = TmpStore::new("one");
        let emitted = ResumeSessionStore::persist_after_first_prompt(
            &mut ts.store,
            "n-1",
            "claude-code",
            "sess-1",
            Some("Página de vendas".to_string()),
            1_000,
            DEFAULT_SESSION_STORE_MAX_ENTRIES,
        )
        .expect("persistir");
        assert_eq!(emitted.len(), 1, "1º prompt ⇒ 1 evento");
        assert_eq!(count_kind(&ts.store, "SessionPersisted", None), 1);

        // Idempotência: o mesmo session_id não re-emite.
        let again = ResumeSessionStore::persist_after_first_prompt(
            &mut ts.store,
            "n-1",
            "claude-code",
            "sess-1",
            None,
            2_000,
            DEFAULT_SESSION_STORE_MAX_ENTRIES,
        )
        .expect("persistir de novo");
        assert!(again.is_empty(), "mesmo session_id ⇒ no-op");
        assert_eq!(count_kind(&ts.store, "SessionPersisted", None), 1);

        let store = ResumeSessionStore::replay(&ts.store).expect("replay");
        assert_eq!(store.sessions.len(), 1);
        let saved = store.active_for("n-1").expect("sessão ativa do nó");
        assert_eq!(saved.session_id, "sess-1");
        assert_eq!(saved.label.as_deref(), Some("Página de vendas"));
    }

    /// **Gate (a) — cap LRU: 3 de teto, 4 sessões ⇒ exatamente 1 SessionRetired{lru} e 3 ativas.** O
    /// critério integral do épico. Quatro nós distintos, persistidos em `now` crescente; ao admitir o
    /// 4º, a sessão mais antiga sai por LRU.
    #[test]
    fn lru_cap_three_retires_oldest() {
        let mut ts = TmpStore::new("lru");
        for (i, (node, sid)) in [
            ("n-1", "s-1"),
            ("n-2", "s-2"),
            ("n-3", "s-3"),
            ("n-4", "s-4"),
        ]
        .iter()
        .enumerate()
        {
            ResumeSessionStore::persist_after_first_prompt(
                &mut ts.store,
                node,
                "claude-code",
                sid,
                None,
                1_000 + i as u64, // now crescente ⇒ s-1 é a mais antiga
                3,
            )
            .expect("persistir");
        }
        assert_eq!(count_kind(&ts.store, "SessionPersisted", None), 4);
        assert_eq!(
            count_kind(&ts.store, "SessionRetired", Some("lru")),
            1,
            "exatamente 1 poda LRU"
        );

        let store = ResumeSessionStore::replay(&ts.store).expect("replay");
        assert_eq!(store.sessions.len(), 3, "3 ativas após a poda");
        let ids: Vec<&str> = store
            .sessions
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        assert!(!ids.contains(&"s-1"), "a mais antiga (s-1) saiu");
        assert!(ids.contains(&"s-2") && ids.contains(&"s-3") && ids.contains(&"s-4"));
    }

    /// Rotação `/resume`: re-persistir o MESMO nó com um `session_id` novo aposenta o anterior com
    /// `cli_invalidated` — só uma sessão fica ativa por nó (spec 53 §11.B).
    #[test]
    fn re_persist_same_node_invalidates_previous() {
        let mut ts = TmpStore::new("rotate");
        ResumeSessionStore::persist_after_first_prompt(
            &mut ts.store,
            "n-1",
            "claude-code",
            "old",
            None,
            1_000,
            50,
        )
        .expect("1ª sessão");
        let emitted = ResumeSessionStore::persist_after_first_prompt(
            &mut ts.store,
            "n-1",
            "claude-code",
            "new",
            None,
            2_000,
            50,
        )
        .expect("rotação");
        assert_eq!(
            count_kind(&ts.store, "SessionRetired", Some("cli_invalidated")),
            1,
            "a sessão antiga do nó foi invalidada"
        );
        assert!(emitted.iter().any(|e| matches!(
            e,
            DomainEvent::SessionRetired { reason, .. } if reason == "cli_invalidated"
        )));
        let store = ResumeSessionStore::replay(&ts.store).expect("replay");
        assert_eq!(store.sessions.len(), 1, "uma ativa por nó");
        assert_eq!(store.active_for("n-1").expect("ativa").session_id, "new");
    }

    /// **Gate (a) — comando de restore montado ASSERTÁVEL.** `can_resume()==true` + session salva ⇒
    /// `[program, args…, resume_args…, session_id]`. É o "Retome o que estava fazendo" preciso.
    #[test]
    fn resume_command_includes_session_id() {
        let claude = claude_profile();
        assert_eq!(
            resume_command(&claude, Some("sess-abc")),
            vec![
                "claude",
                "--output-format",
                "stream-json",
                "--resume",
                "sess-abc"
            ]
        );
        // Sem sessão salva ⇒ novo começo (sem resume_args nem id).
        assert_eq!(
            resume_command(&claude, None),
            vec!["claude", "--output-format", "stream-json"]
        );
        // CLI que não retoma ⇒ novo começo mesmo com session_id (declarar é necessário, não suficiente).
        assert_eq!(
            resume_command(&shell_profile(), Some("sess-abc")),
            vec!["bash"]
        );
    }

    /// **Gate (j) — replay idêntico.** Reprojetar do log reconstrói a mesma projeção byte-a-byte
    /// (igualdade estrutural). Prova que o estado vive no log, não em campo efêmero.
    #[test]
    fn replay_is_deterministic() {
        let mut ts = TmpStore::new("replay");
        for (i, (node, sid)) in [("n-1", "s-1"), ("n-2", "s-2"), ("n-1", "s-3")]
            .iter()
            .enumerate()
        {
            ResumeSessionStore::persist_after_first_prompt(
                &mut ts.store,
                node,
                "claude-code",
                sid,
                None,
                1_000 + i as u64,
                50,
            )
            .expect("persistir");
        }
        let a = ResumeSessionStore::replay(&ts.store).expect("replay 1");
        let b = ResumeSessionStore::replay(&ts.store).expect("replay 2");
        assert_eq!(a, b, "duas projeções do mesmo log são idênticas");
        // n-1 rotacionou (s-1 → s-3); n-2 intacto.
        assert_eq!(a.active_for("n-1").expect("n-1").session_id, "s-3");
        assert_eq!(a.active_for("n-2").expect("n-2").session_id, "s-2");
    }
}
