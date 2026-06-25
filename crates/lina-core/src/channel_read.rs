//! F4-1-3 + F4-1-5 · Leitura de conversa como contexto + pré-config de grupos.
//!
//! **STUB DE LARGADA (Maestro 01).** Território do **Terminal K** na onda F4-1.
//!
//! ## O que esta frente constrói (TAG [6] Fluxo de informação)
//! A assimetria-doutrina de F4-1: **ler é fluxo** (entra como contexto, sem efeito externo → sem gate),
//! **mas auditado e escopado**. Duas peças acopladas (por isso o mesmo dono):
//! - **F4-1-5 — scope de grupos (default-deny):** reusa `tool_scope.rs` (`ToolScopeDeclared`/`Revoked`)
//!   para WhatsApp. Só grupos DECLARADOS pelo leigo (`grupo:Vendas`) são visíveis à IA; um não-declarado
//!   é **invisível** — ler um deles é recusado e auditado, não retorna conteúdo.
//! - **F4-1-3 — leitura auditada:** quando a IA lê um grupo declarado, o transporte (`channel_waha`)
//!   puxa as mensagens, o conteúdo volta ao chamador como contexto, e a leitura deixa rastro
//!   `ChannelMessageRead { channel, conversation_ref, count, read_by_node }` — **só metadados**
//!   (conteúdo NUNCA vai cru ao log; espelha `HistoryReadCross`). Liga-se à camada de briefing para
//!   a IA ver "você me deu acesso ao grupo X".
//!
//! ## Fronteira
//! Função PURA de decisão (`pode_ler(scope_projetado, conversation_ref) -> bool`) testável headless,
//! separada do wrapper que faz I/O (lê `tool_scope` projetado do log; chama `channel_waha` p/ puxar).
//! NÃO toca `channel.rs` (projeção de status = Terminal I) nem `channel_waha.rs` (transporte = Terminal B):
//! consome o contrato público de ambos. Costura de briefing: mapear o diff e propor ao Maestro (camada 2).

use crate::events::{DomainEvent, EventStore, StoreError};
use crate::tool_scope::ToolScopeSet;

/// O que pode dar errado ao ler uma conversa declarada. Genérico no erro do transporte (`E`) porque
/// `channel_read` **não conhece** o cliente concreto (`channel_waha` é território do Terminal B, atrás
/// de uma borda injetável) — ele só sabe que o transporte pode falhar. Sem `PartialEq`/`Eq`/`Clone`:
/// a variante `Auditoria` embrulha `StoreError` (I/O), que não é comparável — os testes usam `matches!`.
#[derive(Debug)]
pub enum LeituraError<E> {
    /// **default-deny (F4-1-5):** a conversa/grupo NÃO está em `ToolScopeDeclared` para este projeto —
    /// é INVISÍVEL à IA. A recusa é o fato auditável; **nenhum** `ChannelMessageRead` (evento de
    /// sucesso) é emitido e o transporte **nunca** é tocado (zero exfiltração). O chamador narra a
    /// recusa ao leigo; espelha `HistoryError::CrossDenied`.
    NaoDeclarado {
        channel: String,
        conversation_ref: String,
    },
    /// O transporte (`channel_waha`) falhou ao puxar as mensagens. Encapsula o erro de I/O do cliente
    /// sem que `channel_read` precise conhecê-lo.
    Transporte(E),
    /// A leitura ocorreu, mas persistir o rastro `ChannelMessageRead` falhou. Espelha
    /// `HistoryError::AuditFailed`: leitura sem rastro é um erro, não um sucesso silencioso.
    Auditoria(StoreError),
}

/// **Coração da decisão (PURO · F4-1-5).** `true` sse a conversa/grupo `conversation_ref` está
/// DECLARADO para `project`/`channel` no escopo projetado — a forma da regra "ler exige declaração".
/// **default-deny:** nada declarado ⇒ `false` (invisível). Reusa [`ToolScopeSet::is_declared`] (a
/// álgebra de scope é do Terminal M); aqui só damos nome à leitura como gate, testável headless e sem
/// I/O. O `conversation_ref` (ex.: `"grupo:Vendas"`) casa com o `scope` da declaração.
#[must_use]
pub fn pode_ler(
    scope: &ToolScopeSet,
    project: &str,
    channel: &str,
    conversation_ref: &str,
) -> bool {
    scope.is_declared(project, channel, conversation_ref)
}

/// **Leitura auditada (F4-1-3).** Lê uma conversa declarada como CONTEXTO de entrada e deixa rastro —
/// fluxo, não ação externa (sem gate), mas escopado e auditado.
///
/// Ordem (espelha `history::audit_cross`): **gate ANTES de qualquer I/O** → só o caminho permitido puxa
/// o transporte → emite o rastro. Conversa não declarada ⇒ `Err(NaoDeclarado)` **sem** tocar o
/// transporte e **sem** evento (a função `puxar` nem é chamada — prova anti-exfiltração).
///
/// O conteúdo das mensagens (`Vec<M>`, opaco) volta ao chamador como contexto e **NUNCA** vai ao log:
/// só os metadados `ChannelMessageRead { channel, conversation_ref, count, read_by_node }` (espelha
/// `HistoryReadCross`, ADR 0035 §5). `M`/`E`/`puxar` são a borda injetável: o transporte real
/// (`channel_waha::read`) entra como adaptador de 1 linha; o teste injeta um mock sem rede.
///
/// # Errors
/// [`LeituraError::NaoDeclarado`] se o escopo não declara a conversa; [`LeituraError::Transporte`] se
/// `puxar` falha; [`LeituraError::Auditoria`] se persistir o rastro falha.
pub fn ler_conversa<M, E, F>(
    store: &mut EventStore,
    scope: &ToolScopeSet,
    project: &str,
    channel: &str,
    conversation_ref: &str,
    read_by_node: &str,
    puxar: F,
) -> Result<Vec<M>, LeituraError<E>>
where
    F: FnOnce() -> Result<Vec<M>, E>,
{
    if !pode_ler(scope, project, channel, conversation_ref) {
        return Err(LeituraError::NaoDeclarado {
            channel: channel.to_string(),
            conversation_ref: conversation_ref.to_string(),
        });
    }

    // Autorizado: só agora tocamos o transporte. O conteúdo fica opaco — nunca inspecionado, nunca logado.
    let mensagens = puxar().map_err(LeituraError::Transporte)?;
    // `count` é metadado; precisa do resultado, por isso o rastro vem APÓS a leitura (≠ history, que não
    // carrega count). >u32::MAX mensagens satura em vez de panicar (sem unwrap em produção).
    let count = u32::try_from(mensagens.len()).unwrap_or(u32::MAX);

    store
        .append(&DomainEvent::ChannelMessageRead {
            channel: channel.to_string(),
            conversation_ref: conversation_ref.to_string(),
            count,
            read_by_node: read_by_node.to_string(),
        })
        .map_err(LeituraError::Auditoria)?;

    Ok(mensagens)
}

// ───────────────────────────────────────── testes ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_scope::declare_tool_scope;
    use std::cell::Cell;

    /// `EventStore` temporário, isolado por processo+thread (mesmo padrão de `tool_scope.rs`).
    fn temp_store(tag: &str) -> (EventStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "lina-channel-read-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = EventStore::open(&dir).expect("abre store temporário");
        (store, dir)
    }

    fn channel_message_reads(store: &EventStore) -> Vec<DomainEvent> {
        store
            .events()
            .expect("lê o log")
            .iter()
            .filter_map(|r| {
                DomainEvent::from_record(&r.kind, r.version, r.payload.clone())
                    .ok()
                    .filter(|e| matches!(e, DomainEvent::ChannelMessageRead { .. }))
            })
            .collect()
    }

    /// F4-1-5: grupo declarado → `pode_ler` é `true`; não declarado (escopo/canal/projeto) → `false`.
    #[test]
    fn pode_ler_segue_default_deny() {
        let mut store = EventStore::open(temp_store("puro").1.as_path()).expect("store");
        declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
        let scope = ToolScopeSet::replay(&store).expect("replay");

        assert!(pode_ler(&scope, "loja", "whatsapp", "grupo:Vendas"));
        assert!(!pode_ler(&scope, "loja", "whatsapp", "grupo:Suporte"), "escopo não declarado");
        assert!(!pode_ler(&scope, "loja", "instagram", "grupo:Vendas"), "canal não declarado");
        assert!(!pode_ler(&scope, "outra", "whatsapp", "grupo:Vendas"), "projeto não declarado");
        assert!(!pode_ler(&ToolScopeSet::default(), "loja", "whatsapp", "grupo:Vendas"), "vazio = deny");
    }

    /// F4-1-3 caminho feliz: grupo declarado → conteúdo volta ao chamador + 1 `ChannelMessageRead` com
    /// `count` correto e SEM o texto das mensagens no log.
    #[test]
    fn le_grupo_declarado_audita_metadados_sem_conteudo() {
        let (mut store, dir) = temp_store("happy");
        declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
        let scope = ToolScopeSet::replay(&store).expect("replay");

        // sentinela única: se vazar para o log, o teste pega.
        const SEGREDO: &str = "TEXTO-CONFIDENCIAL-DO-CLIENTE-zzz999";
        let conteudo = ler_conversa::<String, std::io::Error, _>(
            &mut store,
            &scope,
            "loja",
            "whatsapp",
            "grupo:Vendas",
            "n-abc",
            || Ok(vec![SEGREDO.to_string(), "outra".to_string()]),
        )
        .expect("leitura permitida");

        assert_eq!(conteudo.len(), 2, "conteúdo volta ao chamador como contexto");

        let lidos = channel_message_reads(&store);
        assert_eq!(lidos.len(), 1, "exatamente 1 rastro de leitura");
        match &lidos[0] {
            DomainEvent::ChannelMessageRead {
                channel,
                conversation_ref,
                count,
                read_by_node,
            } => {
                assert_eq!(channel, "whatsapp");
                assert_eq!(conversation_ref, "grupo:Vendas");
                assert_eq!(*count, 2, "metadado = quantas mensagens, nunca o texto");
                assert_eq!(read_by_node, "n-abc");
            }
            other => panic!("evento errado: {other:?}"),
        }

        // D2 (conteúdo fora do log): varre o LOG INTEIRO serializado — o texto não pode aparecer.
        let log = serde_json::to_string(&store.events().expect("log")).expect("serializa");
        assert!(!log.contains(SEGREDO), "conteúdo da mensagem NUNCA vai cru ao log");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F4-1-5 (anti-exfiltração, o critério de segurança): grupo NÃO declarado → recusa, ZERO eventos e
    /// o transporte NEM É CHAMADO (a IA não vira canal de exfiltração de um grupo invisível).
    #[test]
    fn grupo_nao_declarado_recusa_sem_tocar_transporte() {
        let (mut store, dir) = temp_store("deny");
        declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
        let scope = ToolScopeSet::replay(&store).expect("replay");

        let puxou = Cell::new(false);
        let r = ler_conversa::<String, std::io::Error, _>(
            &mut store,
            &scope,
            "loja",
            "whatsapp",
            "grupo:Suporte", // não declarado → invisível
            "n-abc",
            || {
                puxou.set(true);
                Ok(vec!["não deveria chegar aqui".to_string()])
            },
        );

        assert!(
            matches!(r, Err(LeituraError::NaoDeclarado { .. })),
            "não declarado = recusa, não leitura"
        );
        assert!(!puxou.get(), "transporte NÃO tocado: gate roda ANTES de qualquer I/O");
        assert!(
            channel_message_reads(&store).is_empty(),
            "recusa não emite ChannelMessageRead (sem evento de sucesso)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Falha do transporte (após autorizado) → `Transporte`, sem rastro de sucesso.
    #[test]
    fn falha_de_transporte_nao_audita_sucesso() {
        let (mut store, dir) = temp_store("io-err");
        declare_tool_scope(&mut store, "loja", "whatsapp", "grupo:Vendas").expect("declara");
        let scope = ToolScopeSet::replay(&store).expect("replay");

        let r = ler_conversa::<String, _, _>(
            &mut store,
            &scope,
            "loja",
            "whatsapp",
            "grupo:Vendas",
            "n-abc",
            || Err(std::io::Error::other("waha caiu")),
        );

        assert!(matches!(r, Err(LeituraError::Transporte(_))));
        assert!(
            channel_message_reads(&store).is_empty(),
            "transporte falhou → nenhum ChannelMessageRead"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
