//! F1-5-8 — API de consulta **paginada** do histórico (fatia CORE).
//!
//! Camada de leitura sobre o [`ScrollbackStore`]: `tail`/`search`/`export`, **toda chamada
//! com limite duro** (default + máximo) — NENHUM caminho não-paginado é exposto a agente.
//! A saída são as linhas linearizadas da W0-2 (sem ANSI por construção; o store só guarda
//! texto já linearizado).
//!
//! **Janela expirada responde "expirado", nunca erro** (consome `expired_before` da F1-5-9):
//! toda resposta carrega `expired_before`, e uma janela inteiramente abaixo dele volta vazia
//! com a flag [`HistoryPage::expired`] acesa — a UI/verbo traduz para "histórico expirado".
//!
//! **Policy de acesso (ADR 0006):** leitura CROSS-terminal passa pela MESMA fronteira de
//! pertencimento da entrega A2A — membros do mesmo Espaço leem; fora dele, default-deny.
//! Toda leitura cross emite [`DomainEvent::HistoryReadCross`] no log (auditável, inv. #4);
//! falha ao auditar NEGA a leitura (nunca existe leitura cross não-registrada).
//!
//! O verbo `lina history` (bin) é COSTURA do time externo — o contrato (flags/saída) vive em
//! `tasks/epico-f1/contrato-lina-history.md`.

use crate::events::{DomainEvent, EventStore};
use crate::scrollback::{ScrollbackError, ScrollbackStore};
use crate::NodeId;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Limites duros de paginação. `default_page` quando o chamador não pede tamanho;
/// `max_page` é o TETO — pedido acima (até 10^9) é silenciosamente clampado e a
/// continuação vai no `next_cursor` (adversarial-safe: nenhum pedido devolve mais).
#[derive(Debug, Clone, Copy)]
pub struct HistoryLimits {
    pub default_page: usize,
    pub max_page: usize,
    /// Teto de linhas VARRIDAS por chamada de `search` (a busca é bounded mesmo
    /// quando nada casa — o cursor avança pela região varrida).
    pub max_scan: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            default_page: 200,
            max_page: 1_000,
            max_scan: 10_000,
        }
    }
}

/// Erros da API de histórico. Janela expirada NÃO é erro (ver [`HistoryPage::expired`]).
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// O `pattern` do `search` não compila.
    #[error("regex inválido: {0}")]
    BadRegex(String),
    /// Leitura cross negada: par fora da fronteira de pertencimento (default-deny).
    #[error("leitura cross negada: {reader} não pertence ao mesmo Espaço que o dono de {panel}")]
    CrossDenied { reader: NodeId, panel: String },
    /// A trilha de auditoria falhou — a leitura cross é NEGADA (nunca cross sem registro).
    #[error("auditoria da leitura cross falhou (leitura negada): {0}")]
    AuditFailed(String),
    #[error(transparent)]
    Store(#[from] ScrollbackError),
}

/// Uma página de linhas do histórico. `start` é o índice GLOBAL da 1ª linha devolvida;
/// `next_cursor` é o argumento da próxima chamada (`None` = não há mais).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryPage {
    pub panel: String,
    pub start: u64,
    pub lines: Vec<String>,
    pub next_cursor: Option<u64>,
    /// Piso de expiração do painel (F1-5-9): linhas `[0, expired_before)` não existem mais.
    pub expired_before: u64,
    /// A janela pedida caiu (no todo ou em parte) abaixo de `expired_before`.
    pub expired: bool,
}

/// Um hit do `search`: o índice global + a linha (sem ANSI).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchHit {
    pub idx: u64,
    pub line: String,
}

/// Resposta do `search`: hits até o `limit`, e `next_cursor` apontando o primeiro índice
/// ainda NÃO varrido (a busca é bounded por `max_scan` mesmo sem hits).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchPage {
    pub panel: String,
    pub hits: Vec<SearchHit>,
    pub next_cursor: Option<u64>,
    pub expired_before: u64,
}

/// Formato do `export`. JSON é o [`HistoryPage`] serializado (round-trip íntegro);
/// TXT são as linhas cruas separadas por `\n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Txt,
}

/// Últimas `n` linhas do painel, com `offset` a partir do fim (offset 0 = a cauda).
/// `n` é clampado a `max_page`; a continuação (linhas mais antigas) vai no `next_cursor`
/// (que é o PRÓXIMO `offset`). Janela abaixo do piso de expiração → página vazia/parcial
/// com `expired: true`, nunca erro.
pub fn tail(
    store: &ScrollbackStore,
    panel: &str,
    n: Option<usize>,
    offset: u64,
    limits: &HistoryLimits,
) -> Result<HistoryPage, HistoryError> {
    let total = store.total_lines(panel);
    let expired_before = store.expired_before(panel);
    let page = n.unwrap_or(limits.default_page).clamp(1, limits.max_page) as u64;

    let hi = total.saturating_sub(offset);
    if hi <= expired_before {
        // Janela inteiramente expirada (ou além do início): responde "expirado", não erro.
        return Ok(HistoryPage {
            panel: panel.to_string(),
            start: expired_before,
            lines: Vec::new(),
            next_cursor: None,
            expired_before,
            expired: expired_before > 0,
        });
    }
    let lo_pedido = hi.saturating_sub(page);
    let lo = lo_pedido.max(expired_before);
    let lines = store.range(panel, lo, hi)?;
    let next_cursor = (lo > expired_before).then(|| offset + (hi - lo));
    Ok(HistoryPage {
        panel: panel.to_string(),
        start: lo,
        lines,
        next_cursor,
        expired_before,
        // Parcialmente expirada: o chamador pediu mais para trás do que ainda existe.
        expired: lo_pedido < expired_before && expired_before > 0,
    })
}

/// Busca `pattern` (regex) a partir de `cursor` (índice global; `None` = início vivo).
/// Bounded DUPLO: devolve no máximo `limit` hits (clampado a `max_page`) E varre no
/// máximo `max_scan` linhas por chamada — `next_cursor` continua de onde parou.
pub fn search(
    store: &ScrollbackStore,
    panel: &str,
    pattern: &str,
    limit: Option<usize>,
    cursor: Option<u64>,
    limits: &HistoryLimits,
) -> Result<SearchPage, HistoryError> {
    let re = Regex::new(pattern).map_err(|e| HistoryError::BadRegex(e.to_string()))?;
    let total = store.total_lines(panel);
    let expired_before = store.expired_before(panel);
    let limit = limit
        .unwrap_or(limits.default_page)
        .clamp(1, limits.max_page);

    // Cursor abaixo do piso é CONSUMIDO pelo piso (região expirada não é varrível).
    let start = cursor.unwrap_or(expired_before).max(expired_before);
    let scan_end = start.saturating_add(limits.max_scan as u64).min(total);

    let mut hits = Vec::new();
    let mut idx = start;
    // Varre em blocos (não hidrata a região inteira de uma vez — teto de RAM).
    const BLOCK: u64 = 512;
    while idx < scan_end && hits.len() < limit {
        let hi = (idx + BLOCK).min(scan_end);
        for (off, line) in store.range(panel, idx, hi)?.into_iter().enumerate() {
            if hits.len() >= limit {
                break;
            }
            if re.is_match(&line) {
                hits.push(SearchHit {
                    idx: idx + off as u64,
                    line,
                });
            }
        }
        idx = hi;
    }
    // Próximo índice ainda não varrido. Hits cheios antes do fim do bloco: retoma do
    // último hit + 1 (nada varrido se perde, nada é re-devolvido).
    let resume = if hits.len() >= limit {
        hits.last().map_or(idx, |h| h.idx + 1)
    } else {
        idx
    };
    let next_cursor = (resume < total).then_some(resume);
    Ok(SearchPage {
        panel: panel.to_string(),
        hits,
        next_cursor,
        expired_before,
    })
}

/// Exporta a janela `[lo, hi)` no formato pedido. A janela é clampada ao piso de
/// expiração e ao teto `max_page` por chamada (continuação via cursor devolvido).
/// JSON = [`HistoryPage`] serializado (round-trip por `serde_json::from_str`).
pub fn export(
    store: &ScrollbackStore,
    panel: &str,
    format: ExportFormat,
    lo: u64,
    hi: u64,
    limits: &HistoryLimits,
) -> Result<(String, Option<u64>), HistoryError> {
    let total = store.total_lines(panel);
    let expired_before = store.expired_before(panel);
    let lo_efetivo = lo.max(expired_before);
    let hi_efetivo = hi
        .min(total)
        .min(lo_efetivo.saturating_add(limits.max_page as u64));
    let (lines, expired) = if lo_efetivo >= hi_efetivo {
        (Vec::new(), expired_before > 0 && lo < expired_before)
    } else {
        (
            store.range(panel, lo_efetivo, hi_efetivo)?,
            lo < expired_before,
        )
    };
    let next_cursor = (hi_efetivo < hi.min(total) && !lines.is_empty()).then_some(hi_efetivo);
    let page = HistoryPage {
        panel: panel.to_string(),
        start: lo_efetivo,
        lines,
        next_cursor,
        expired_before,
        expired,
    };
    let payload = match format {
        ExportFormat::Json => serde_json::to_string(&page)
            .map_err(|e| HistoryError::AuditFailed(format!("serializar export: {e}")))?,
        ExportFormat::Txt => page.lines.join("\n"),
    };
    Ok((payload, next_cursor))
}

/// Fronteira de pertencimento da leitura cross (mesma semântica da `WorkspaceTrust` do
/// A2A): `reader` lê o painel de `owner` sse AMBOS são membros vivos do Espaço.
/// Same-owner (ler o próprio painel) é sempre permitido e NÃO é auditado (alto
/// volume/baixo sinal — só o cross é sinal de auditoria).
#[must_use]
pub fn cross_allowed(reader: NodeId, owner: NodeId, members: &[NodeId]) -> bool {
    reader == owner || (members.contains(&reader) && members.contains(&owner))
}

/// `tail` CROSS-terminal: aplica a fronteira de pertencimento e **audita ANTES de ler**
/// ([`DomainEvent::HistoryReadCross`] no log). Auditoria falhou → leitura NEGADA
/// (nunca existe leitura cross sem rastro). Same-owner delega ao [`tail`] puro.
#[allow(clippy::too_many_arguments)]
pub fn tail_cross(
    events: &mut EventStore,
    members: &[NodeId],
    reader: NodeId,
    owner: NodeId,
    store: &ScrollbackStore,
    panel: &str,
    n: Option<usize>,
    offset: u64,
    limits: &HistoryLimits,
) -> Result<HistoryPage, HistoryError> {
    audit_cross(events, members, reader, owner, panel, "tail")?;
    tail(store, panel, n, offset, limits)
}

/// `search` CROSS-terminal — mesma fronteira/auditoria do [`tail_cross`].
#[allow(clippy::too_many_arguments)]
pub fn search_cross(
    events: &mut EventStore,
    members: &[NodeId],
    reader: NodeId,
    owner: NodeId,
    store: &ScrollbackStore,
    panel: &str,
    pattern: &str,
    limit: Option<usize>,
    cursor: Option<u64>,
    limits: &HistoryLimits,
) -> Result<SearchPage, HistoryError> {
    audit_cross(events, members, reader, owner, panel, "search")?;
    search(store, panel, pattern, limit, cursor, limits)
}

/// `export` CROSS-terminal — mesma fronteira/auditoria do [`tail_cross`]. Exportar o
/// scrollback ALHEIO é a leitura de maior volume (um bloco inteiro de cada vez), logo a
/// que MAIS precisa de rastro: nenhuma das 3 ops escapa da fronteira de pertencimento.
#[allow(clippy::too_many_arguments)]
pub fn export_cross(
    events: &mut EventStore,
    members: &[NodeId],
    reader: NodeId,
    owner: NodeId,
    store: &ScrollbackStore,
    panel: &str,
    format: ExportFormat,
    lo: u64,
    hi: u64,
    limits: &HistoryLimits,
) -> Result<(String, Option<u64>), HistoryError> {
    audit_cross(events, members, reader, owner, panel, "export")?;
    export(store, panel, format, lo, hi, limits)
}

/// Gate comum do caminho cross: pertencimento (default-deny) + evento auditável.
/// Same-owner passa direto (sem evento).
fn audit_cross(
    events: &mut EventStore,
    members: &[NodeId],
    reader: NodeId,
    owner: NodeId,
    panel: &str,
    query: &str,
) -> Result<(), HistoryError> {
    if reader == owner {
        return Ok(());
    }
    if !cross_allowed(reader, owner, members) {
        return Err(HistoryError::CrossDenied {
            reader,
            panel: panel.to_string(),
        });
    }
    events
        .append(&DomainEvent::HistoryReadCross {
            reader,
            panel: panel.to_string(),
            query: query.to_string(),
        })
        .map_err(|e| HistoryError::AuditFailed(e.to_string()))?;
    Ok(())
}
