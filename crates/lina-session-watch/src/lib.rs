//! `lina-session-watch` — watch de session-files JSONL + schema único de sessão
//! (story **F1-1-2**, camada 3 do padrão de detecção/observabilidade).
//!
//! ## O que é (e o que NUNCA é)
//! CLIs de IA gravam session-files JSONL em disco (ex.: Claude Code em
//! `~/.claude/projects/<slug>/*.jsonl` — o padrão vem do `session_dir_pattern`
//! do `CliProfile`, F1-1-1, invariante #3: nada de caminho hardcoded). Este crate
//! observa esses arquivos e os normaliza num **schema único** ([`Session`]) para o
//! dashboard (F1-1-5) e para AGREGAR confiança à detecção de CLI (camada 3 — nunca
//! decisória). Tudo aqui é **PROJEÇÃO reconstruível** (invariante #4): apagar a
//! projeção e re-escanear produz o mesmo resultado; nada disto é autoridade nem
//! entra no event log de domínio.
//!
//! ## Honestidade contábil (pesquisa 13.5, achado 2)
//! O JSONL tem **subcontagem documentada** (até 100–174× no input). Todo custo
//! derivado daqui é ESTIMATIVA: [`Session::cost_estimated`] é sempre `true` quando
//! a fonte é JSONL — consumidores exibem "~" e nunca tratam como verdade contábil.
//!
//! ## Privacidade (invariante #2 + fronteira da story)
//! O parser extrai SÓ os metadados do schema (tokens/modelo/nomes de ferramentas/
//! ids); o CONTEÚDO das conversas não é retido nem persistido. Nada sai da máquina.
//!
//! ## Incremental por design
//! Cursor por arquivo `(offset, mtime, size)`: re-parse lê **só o delta** desde o
//! último poll (provado por contador de bytes lidos). Arquivos são lidos em
//! streaming (linha a linha, nunca o arquivo inteiro em RAM) e o que fica retido
//! são AGREGADOS por sessão — medível por [`SessionScanner::ram_bytes`]
//! (determinístico; RSS é só corroboração).
//!
//! ## Watch sem dependência de plataforma (decisão registrada)
//! A descoberta de mudança é por **poll de `(mtime, size)`** via [`SessionWatch::poll_once`]
//! — determinístico em teste e suficiente para a meta de frescor da story (<2s).
//! Um backend `notify` (FSEvents/inotify) pode embrulhar `poll_once` depois sem
//! mudar a API (porta aberta; dep nova fica fora desta story).

#![forbid(unsafe_code)]

pub mod pricing;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Teto de bytes de UMA linha JSONL. Acima disto a linha é pulada e CONTADA
/// ([`SessionScanner::skipped_lines`]) — uma linha patológica não pode reter
/// RAM nem travar o scan (streaming por linha, nunca o arquivo inteiro).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// **Fonte do custo/tokens de uma sessão** (F1-1-4 — ponto de merge do schema).
/// `Jsonl` = derivado dos session-files (subcontado, `cost_estimated=true`); `Otel` =
/// recebido pelo collector local (preferido quando presente — 13.5 item 9 corrige o
/// subconto). O dashboard (F1-1-5) exibe esta origem (critério 1 da story).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CostSource {
    /// Session-file JSONL (camada 3, F1-1-2) — sempre estimativa.
    #[default]
    Jsonl,
    /// OTel local (camada opcional, F1-1-4) — fonte preferida de custo quando ligada.
    Otel,
}

impl CostSource {
    /// Rótulo estável (persistido na projeção e exibido).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CostSource::Jsonl => "jsonl",
            CostSource::Otel => "otel",
        }
    }
    /// Parse tolerante (projeção antiga/desconhecida → `Jsonl`, o default seguro).
    #[must_use]
    pub fn from_str_or_jsonl(s: &str) -> Self {
        if s == "otel" {
            CostSource::Otel
        } else {
            CostSource::Jsonl
        }
    }
}

/// Schema ÚNICO de sessão (13.5 item 4) — a projeção normalizada que o dashboard
/// (F1-1-5) e a camada 3 de detecção consomem, independente do CLI de origem.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// `id` do `CliProfile` de origem (ex.: `"claude-code"`).
    pub cli: String,
    /// Id da sessão no CLI (campo `sessionId`; fallback: stem do arquivo).
    pub session_id: String,
    /// Tokens de input somados na sessão (SUBCONTADOS na fonte — ver doc do crate).
    pub tokens_in: u64,
    /// Tokens de output somados.
    pub tokens_out: u64,
    /// Tokens de cache (criação + leitura somados).
    pub tokens_cache: u64,
    /// Tokens de thinking, quando a fonte os expõe (0 quando não).
    pub tokens_thinking: u64,
    /// Custo somado — `costUSD` da linha quando presente (formato antigo); senão
    /// DERIVADO de `usage` × preço do modelo ([`pricing`]; o formato atual não grava
    /// `costUSD`). SEMPRE estimativa; modelo fora da tabela não soma (sem chute).
    pub cost_usd: f64,
    /// `true` quando a fonte é JSONL (subcontagem documentada, 13.5 achado 2).
    pub cost_estimated: bool,
    /// Último modelo visto na sessão.
    pub model: Option<String>,
    /// `cwd` da sessão (chave da correlação sessão↔nó).
    pub cwd: Option<String>,
    /// Timestamp ISO da última linha vista (atividade; opaco — o dashboard formata).
    pub last_ts: Option<String>,
    /// Subagentes distintos vistos (linhas `isSidechain` com `agentId`).
    pub subagents: Vec<String>,
    /// Ferramentas distintas usadas (`content[].type == "tool_use"` → `name`).
    pub tools: Vec<String>,
    /// **F1-1-4:** fonte do custo/tokens — `Jsonl` por default; vira `Otel` quando o
    /// collector local enriquece a sessão ([`Session::merge_otel_cost`]).
    pub source: CostSource,
}

/// **F1-1-4 — custo/tokens normalizados vindos do OTel** para uma sessão. Tipo
/// PRIMITIVO (o crate `lina-otel` o produz e chama [`Session::merge_otel_cost`]); fica
/// AQUI para que o ponto de merge não dependa do receiver — sem ciclo de crates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OtelCost {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache: u64,
    pub tokens_thinking: u64,
    /// Custo em USD **medido** (não estimado) quando o CLI o emite; `None` = só tokens
    /// (custo derivado tokens×pricing fica a cargo do consumidor — 13.5 por-CLI honesto).
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
}

/// **F1-1-4 — divergência relativa entre o custo JSONL e o OTel** (13.5 item 1: o
/// sanity check que LOGA quando as fontes discordam acima de um threshold). `|otel −
/// jsonl| / max(otel, ε)`; `0.0` quando ambos ~zero. Função pura.
#[must_use]
pub fn cost_divergence(jsonl_cost: f64, otel_cost: f64) -> f64 {
    let denom = otel_cost.abs().max(1e-9);
    (otel_cost - jsonl_cost).abs() / denom
}

/// Threshold default do sanity check (20%) — acima disso, o merge sinaliza divergência.
pub const COST_DIVERGENCE_THRESHOLD: f64 = 0.20;

impl Session {
    /// **F1-1-4 — ponto de merge: OTel é a fonte PREFERIDA de custo** (13.5 item 9).
    /// Devolve a sessão com tokens/custo do OTel, `source = Otel` e
    /// `cost_estimated = false` quando o OTel trouxe custo medido. Os campos NÃO-custo
    /// (cwd, tools, subagents, last_ts) permanecem do JSONL (o OTel não os carrega).
    /// Retorna também a **divergência** de custo (para o sanity check / log do chamador).
    #[must_use]
    pub fn merge_otel_cost(&self, otel: &OtelCost) -> (Session, f64) {
        let divergence = cost_divergence(self.cost_usd, otel.cost_usd.unwrap_or(self.cost_usd));
        let merged = Session {
            tokens_in: otel.tokens_in,
            tokens_out: otel.tokens_out,
            tokens_cache: otel.tokens_cache,
            tokens_thinking: otel.tokens_thinking,
            cost_usd: otel.cost_usd.unwrap_or(self.cost_usd),
            // Custo OTEL medido → não é estimativa; só-tokens (custo derivado) segue estimado.
            cost_estimated: otel.cost_usd.is_none(),
            model: otel.model.clone().or_else(|| self.model.clone()),
            source: CostSource::Otel,
            cli: self.cli.clone(),
            session_id: self.session_id.clone(),
            cwd: self.cwd.clone(),
            last_ts: self.last_ts.clone(),
            subagents: self.subagents.clone(),
            tools: self.tools.clone(),
        };
        (merged, divergence)
    }
}

/// Agregado interno por sessão (sets para distintos; vira [`Session`] na leitura).
#[derive(Debug, Default)]
struct SessionAgg {
    tokens_in: u64,
    tokens_out: u64,
    tokens_cache: u64,
    tokens_thinking: u64,
    cost_usd: f64,
    model: Option<String>,
    cwd: Option<String>,
    last_ts: Option<String>,
    subagents: BTreeSet<String>,
    tools: BTreeSet<String>,
    /// `requestId`s (fallback: `message.id`) cujo `usage` JÁ foi contado. O formato
    /// real grava VÁRIAS linhas `assistant` por request repetindo o MESMO `usage`
    /// (medido: 56/62 requests com 2-4 linhas idênticas num arquivo real) — sem este
    /// dedup, tokens e custo estimado inflam ~3×.
    counted_requests: BTreeSet<String>,
}

impl SessionAgg {
    fn to_session(&self, cli: &str, session_id: &str) -> Session {
        Session {
            cli: cli.to_owned(),
            session_id: session_id.to_owned(),
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            tokens_cache: self.tokens_cache,
            tokens_thinking: self.tokens_thinking,
            cost_usd: self.cost_usd,
            cost_estimated: true, // fonte JSONL → sempre estimativa (13.5)
            model: self.model.clone(),
            cwd: self.cwd.clone(),
            last_ts: self.last_ts.clone(),
            subagents: self.subagents.iter().cloned().collect(),
            tools: self.tools.iter().cloned().collect(),
            source: CostSource::Jsonl, // origem deste agregado é o session-file
        }
    }

    /// Absorve uma linha JSONL já parseada (extrai SÓ metadados do schema —
    /// o conteúdo da conversa nunca é retido).
    fn absorb(&mut self, line: &serde_json::Value) {
        if let Some(ts) = line.get("timestamp").and_then(|v| v.as_str()) {
            self.last_ts = Some(ts.to_owned());
        }
        if let Some(cwd) = line.get("cwd").and_then(|v| v.as_str()) {
            self.cwd = Some(cwd.to_owned());
        }
        if let Some(cost) = line.get("costUSD").and_then(serde_json::Value::as_f64) {
            self.cost_usd += cost;
        }
        if line.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
            if let Some(agent) = line.get("agentId").and_then(|v| v.as_str()) {
                self.subagents.insert(agent.to_owned());
            }
        }
        let Some(message) = line.get("message") else {
            return;
        };
        if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
            self.model = Some(model.to_owned());
        }
        // Dedup de usage POR REQUEST: o formato real repete o MESMO `usage` em 2-4
        // linhas do mesmo `requestId` (ver doc de `counted_requests`). Linha sem chave
        // (formato antigo) conta sempre — comportamento anterior preservado.
        let request_key = line
            .get("requestId")
            .and_then(|v| v.as_str())
            .or_else(|| message.get("id").and_then(|v| v.as_str()));
        let first_of_request = match request_key {
            Some(key) => self.counted_requests.insert(key.to_owned()),
            None => true,
        };
        if let Some(usage) = message.get("usage") {
            if first_of_request {
                let n = |k: &str| {
                    usage
                        .get(k)
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
                };
                self.tokens_in += n("input_tokens");
                self.tokens_out += n("output_tokens");
                self.tokens_cache +=
                    n("cache_creation_input_tokens") + n("cache_read_input_tokens");
                self.tokens_thinking += n("thinking_tokens");

                // Custo: `costUSD` da linha vence (formato antigo, já somado acima).
                // SEM ele — o formato atual NUNCA o grava — deriva a ESTIMATIVA de
                // usage × preço do modelo (o da linha; fallback: último da sessão).
                // Modelo fora da tabela → nada somado (fallback honesto do card).
                if line.get("costUSD").is_none() {
                    let model = message
                        .get("model")
                        .and_then(|v| v.as_str())
                        .or(self.model.as_deref());
                    if let Some(est) = model.and_then(|m| pricing::estimate_usage_cost(m, usage)) {
                        self.cost_usd += est;
                    }
                }
            }
        }
        if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                    if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                        self.tools.insert(name.to_owned());
                    }
                }
            }
        }
    }
}

/// Cursor incremental de UM session-file: o re-parse continua do `offset` (fim da
/// última linha COMPLETA consumida) e `sids` lembra as sessões derivadas DESTE
/// arquivo (para re-derivar sem dupla contagem quando ele é truncado/reescrito).
#[derive(Debug, Default)]
struct FileCursor {
    offset: u64,
    sids: BTreeSet<String>,
}

/// Scanner/agregador de session-files JSONL — streaming (linha a linha), retém
/// SÓ os agregados por sessão + um cursor pequeno por arquivo.
#[derive(Debug, Default)]
pub struct SessionScanner {
    sessions: BTreeMap<(String, String), SessionAgg>,
    files: BTreeMap<PathBuf, FileCursor>,
    bytes_read_total: u64,
    skipped_lines: u64,
}

impl SessionScanner {
    /// Scanner vazio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Total de bytes CONSUMIDOS (linhas completas) desde a criação — é a prova
    /// observável do incremental: re-scan sem mudança soma 0; append soma só o delta.
    /// (Uma linha ainda PARCIAL — sem `\n`, escritor no meio do write — é re-lida no
    /// próximo poll e só conta quando completa.)
    #[must_use]
    pub fn bytes_read_total(&self) -> u64 {
        self.bytes_read_total
    }

    /// Linhas puladas (JSON inválido ou linha > [`MAX_LINE_BYTES`]) — contadas,
    /// nunca engolidas em silêncio.
    #[must_use]
    pub fn skipped_lines(&self) -> u64 {
        self.skipped_lines
    }

    /// Bytes REALMENTE retidos no heap por este scanner (agregados por sessão +
    /// cursores por arquivo) — medição DETERMINÍSTICA do critério 5 da story
    /// (gate em bytes retidos; RSS de processo é só corroboração, poluído por
    /// allocator/cache). Não inclui buffers transitórios de leitura.
    #[must_use]
    pub fn ram_bytes(&self) -> usize {
        let s_str = std::mem::size_of::<String>();
        let mut total = 0usize;
        for ((cli, sid), agg) in &self.sessions {
            total += std::mem::size_of::<SessionAgg>() + 2 * s_str;
            total += cli.len() + sid.len();
            total += agg.model.as_ref().map_or(0, |s| s.len() + s_str);
            total += agg.cwd.as_ref().map_or(0, |s| s.len() + s_str);
            total += agg.last_ts.as_ref().map_or(0, |s| s.len() + s_str);
            total += agg.tools.iter().map(|t| t.len() + s_str).sum::<usize>();
            total += agg.subagents.iter().map(|t| t.len() + s_str).sum::<usize>();
            total += agg
                .counted_requests
                .iter()
                .map(|r| r.len() + s_str)
                .sum::<usize>();
        }
        for (path, cursor) in &self.files {
            total += std::mem::size_of::<FileCursor>() + std::mem::size_of::<PathBuf>();
            total += path.as_os_str().len();
            total += cursor.sids.iter().map(|s| s.len() + s_str).sum::<usize>();
        }
        total
    }

    /// Lê um session-file do cursor em diante e agrega as linhas novas no schema
    /// único. Truncation/rotação (arquivo menor que o offset) re-deriva as sessões
    /// originadas deste arquivo a partir do conteúdo novo. Retorna o delta consumido
    /// (bytes + sessões tocadas) — é o que o [`SessionWatch::poll_once`] agrega.
    ///
    /// Limitação documentada: a detecção de mudança é por TAMANHO (`len` vs cursor).
    /// Reescrita in-place com o MESMO tamanho não é detectada — session-files reais
    /// são append-only, esse padrão não ocorre.
    pub fn scan_file(
        &mut self,
        cli: &str,
        path: impl AsRef<Path>,
    ) -> Result<ScanDelta, WatchError> {
        let path = path.as_ref();
        let label = path.display().to_string();
        let io_err = |source| WatchError::Io {
            path: label.clone(),
            source,
        };

        let len = std::fs::metadata(path).map_err(&io_err)?.len();
        let cursor = self.files.entry(path.to_path_buf()).or_default();

        // Truncation/rotação: o conteúdo antigo não existe mais — descarta os
        // agregados derivados DESTE arquivo (renascem do conteúdo novo) e re-lê do 0.
        if len < cursor.offset {
            for sid in std::mem::take(&mut cursor.sids) {
                self.sessions.remove(&(cli.to_owned(), sid));
            }
            cursor.offset = 0;
        }
        if len == cursor.offset {
            return Ok(ScanDelta::default()); // nada novo — sem I/O de conteúdo
        }
        let start = cursor.offset;

        let mut file = std::fs::File::open(path).map_err(&io_err)?;
        file.seek(SeekFrom::Start(start)).map_err(&io_err)?;

        // Fallback de identidade quando a linha não traz `sessionId`.
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("<sem-nome>")
            .to_owned();

        let mut reader = BufReader::new(file);
        let mut buf = String::new();
        let mut consumed = start;
        let mut touched_sids: BTreeSet<String> = BTreeSet::new();
        loop {
            buf.clear();
            let read = reader.read_line(&mut buf).map_err(&io_err)?;
            if read == 0 {
                break; // EOF
            }
            if !buf.ends_with('\n') {
                break; // linha parcial (escritor no meio do write) — fica p/ o próximo poll
            }
            consumed += read as u64;
            self.bytes_read_total += read as u64;

            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() > MAX_LINE_BYTES {
                self.skipped_lines += 1;
                buf = String::new(); // solta a alocação patológica
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
                self.skipped_lines += 1;
                continue;
            };
            let sid = value
                .get("sessionId")
                .and_then(|v| v.as_str())
                .unwrap_or(&stem)
                .to_owned();
            touched_sids.insert(sid.clone());
            self.sessions
                .entry((cli.to_owned(), sid))
                .or_default()
                .absorb(&value);
        }
        // Grava o cursor de uma vez (offset + sessões derivadas deste arquivo).
        if let Some(cursor) = self.files.get_mut(path) {
            cursor.offset = consumed;
            cursor.sids.extend(touched_sids.iter().cloned());
        }
        Ok(ScanDelta {
            bytes_consumed: consumed - start,
            sessions: touched_sids,
        })
    }

    /// Sessão agregada (snapshot do schema único), se conhecida.
    #[must_use]
    pub fn session(&self, cli: &str, session_id: &str) -> Option<Session> {
        self.sessions
            .get(&(cli.to_owned(), session_id.to_owned()))
            .map(|agg| agg.to_session(cli, session_id))
    }
}

/// Delta consumido por um [`SessionScanner::scan_file`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanDelta {
    /// Bytes de linhas completas consumidos neste scan (0 = nada novo).
    pub bytes_consumed: u64,
    /// `session_id`s tocadas por este scan.
    pub sessions: BTreeSet<String>,
}

/// Resultado de um [`SessionWatch::poll_once`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PollOutcome {
    /// Arquivos cujo scan consumiu bytes novos neste poll.
    pub files_scanned: usize,
    /// Pares `(cli, session_id)` atualizados neste poll — o que o dashboard
    /// (F1-1-5) e a agregação de confiança da camada 3 consomem.
    pub sessions_updated: Vec<(String, String)>,
}

/// Watch de session-files por CLI: fontes vêm do `session_dir_pattern` do
/// `CliProfile` (F1-1-1; invariante #3 — nada de caminho compilado).
///
/// Descoberta de mudança por **poll** de `(len vs cursor)` — determinístico e
/// suficiente para a meta de frescor (<2s) com intervalo de ~500ms-1s no caller;
/// um backend `notify` pode embrulhar [`Self::poll_once`] depois sem mudar a API.
#[derive(Debug)]
pub struct SessionWatch {
    scanner: SessionScanner,
    home: PathBuf,
    sources: Vec<(String, String)>,
}

impl SessionWatch {
    /// Watch com o `~` resolvido para `home` — **injetável** (testes usam tempdir;
    /// produção passa o home real do usuário, decidido pelo caller).
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        Self {
            scanner: SessionScanner::new(),
            home: home.into(),
            sources: Vec::new(),
        }
    }

    /// Registra uma fonte: `cli` + `session_dir_pattern` do seu `CliProfile`
    /// (glob com `*` por componente; `~/` expande para o home injetado).
    pub fn add_source(&mut self, cli: impl Into<String>, pattern: impl Into<String>) {
        self.sources.push((cli.into(), pattern.into()));
    }

    /// Acesso de leitura ao agregador (sessões + contadores observáveis).
    #[must_use]
    pub fn scanner(&self) -> &SessionScanner {
        &self.scanner
    }

    /// Um passo de watch: resolve os patterns, escaneia SÓ o que mudou (cursor
    /// por arquivo) e devolve o que foi atualizado. Pattern sem matches (CLI que
    /// nunca rodou) NÃO é erro — é um CLI sem sessões ainda.
    pub fn poll_once(&mut self) -> Result<PollOutcome, WatchError> {
        let mut outcome = PollOutcome::default();
        for (cli, pattern) in &self.sources {
            for path in expand_pattern(&self.home, pattern) {
                let delta = self.scanner.scan_file(cli, &path)?;
                if delta.bytes_consumed > 0 {
                    outcome.files_scanned += 1;
                    outcome
                        .sessions_updated
                        .extend(delta.sessions.into_iter().map(|sid| (cli.clone(), sid)));
                }
            }
        }
        Ok(outcome)
    }
}

/// Expande um pattern com `~/` + `*` por componente (mini-glob local — evita
/// dependência nova; só o necessário para `session_dir_pattern`). Diretório
/// inexistente → sem matches (não é erro: o CLI pode nunca ter rodado).
fn expand_pattern(home: &Path, pattern: &str) -> Vec<PathBuf> {
    let expanded = if let Some(rest) = pattern.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(pattern)
    };

    let mut candidates = vec![PathBuf::new()];
    for comp in expanded.iter() {
        let comp = comp.to_string_lossy();
        let mut next = Vec::new();
        if comp.contains('*') {
            for base in &candidates {
                let Ok(read) = std::fs::read_dir(base) else {
                    continue; // base não existe/ilegível → sem matches por aqui
                };
                let mut names: Vec<PathBuf> = read
                    .flatten()
                    .filter(|e| wildcard_match(&e.file_name().to_string_lossy(), &comp))
                    .map(|e| e.path())
                    .collect();
                names.sort();
                next.extend(names);
            }
        } else {
            for base in &candidates {
                next.push(base.join(comp.as_ref()));
            }
        }
        candidates = next;
    }
    candidates.retain(|p| p.is_file());
    candidates
}

/// Match de wildcard `*` (qualquer sequência) dentro de UM componente de caminho.
fn wildcard_match(name: &str, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return name == pattern; // sem '*': literal
    }
    let mut rest = name;
    // Prefixo fixo.
    let Some(stripped) = rest.strip_prefix(parts[0]) else {
        return false;
    };
    rest = stripped;
    // Partes do meio: greedy, em ordem.
    for part in &parts[1..parts.len() - 1] {
        if part.is_empty() {
            continue;
        }
        let Some(pos) = rest.find(part) else {
            return false;
        };
        rest = &rest[pos + part.len()..];
    }
    // Sufixo fixo.
    let last = parts[parts.len() - 1];
    last.is_empty() || rest.ends_with(last)
}

// ═══════════════ correlação sessão↔nó — camada 3 (13.9): confirma, nunca decide ═══════════════

/// Pista de correlação de UM nó vivo/encerrado, fornecida pelo caller (o core
/// conhece os nós; este crate não conhece `NodeId` — desacoplamento de fronteira).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeHint {
    /// Id do nó (opaco aqui; o core cunha — autoridade única).
    pub node_id: String,
    /// `cwd` em que o nó spawnou seu CLI.
    pub cwd: String,
    /// Início da janela viva do nó (epoch ms).
    pub alive_from_ms: u64,
    /// Fim da janela viva (`None` = ainda vivo).
    pub alive_to_ms: Option<u64>,
}

/// Veredito da correlação — `Ambiguous` é primeira-classe: a camada 3 AGREGA
/// confiança, nunca chuta identidade (13.9; doutrina A2A do projeto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Correlation {
    /// Exatamente um nó casa → confiança pode ser agregada a ele.
    Unique(String),
    /// Mais de um nó casa (mesmo cwd, janelas sobrepostas) — não decidir.
    Ambiguous(Vec<String>),
    /// Nenhum nó casa (cwd desconhecido ou sessão fora de qualquer janela viva).
    None,
}

/// Correlaciona uma sessão a um nó por `(cwd, janela de mtime)` — **função pura**
/// (timestamps injetados; nada de relógio interno): `session_mtime_ms` é o mtime
/// do session-file, e casa se cai dentro da janela viva do nó no MESMO cwd.
/// PID nunca participa (componente refutado da camada 2 — 13.9).
#[must_use]
pub fn correlate(session_cwd: &str, session_mtime_ms: u64, hints: &[NodeHint]) -> Correlation {
    let mut matches: Vec<&NodeHint> = hints
        .iter()
        .filter(|h| {
            h.cwd == session_cwd
                && session_mtime_ms >= h.alive_from_ms
                && h.alive_to_ms.is_none_or(|to| session_mtime_ms <= to)
        })
        .collect();
    match matches.len() {
        0 => Correlation::None,
        1 => Correlation::Unique(matches.remove(0).node_id.clone()),
        _ => Correlation::Ambiguous(matches.iter().map(|h| h.node_id.clone()).collect()),
    }
}

// ═══════════════ projeção SQLite — reconstruível, NUNCA autoridade (inv#4) ═══════════════

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    cli             TEXT NOT NULL,
    session_id      TEXT NOT NULL,
    tokens_in       INTEGER NOT NULL,
    tokens_out      INTEGER NOT NULL,
    tokens_cache    INTEGER NOT NULL,
    tokens_thinking INTEGER NOT NULL,
    cost_usd        REAL NOT NULL,
    cost_estimated  INTEGER NOT NULL,
    model           TEXT,
    cwd             TEXT,
    last_ts         TEXT,
    subagents       TEXT NOT NULL,
    tools           TEXT NOT NULL,
    source          TEXT NOT NULL DEFAULT 'jsonl',
    PRIMARY KEY (cli, session_id)
);
";

/// Projeção local das sessões em SQLite (`sessions.db`) para o dashboard (F1-1-5).
///
/// **É projeção, não autoridade** (invariante #4): apagar o arquivo e re-derivar
/// dos session-files produz o mesmo conteúdo — nenhuma decisão pode depender só
/// daqui. Mesmo padrão de concorrência do EventStore/ScrollbackStore do repo:
/// `busy_timeout` ANTES de qualquer escrita + `journal_mode=WAL` com retry bounded
/// (a troca de journal não honra `busy_timeout` de forma confiável).
pub struct SessionProjection {
    conn: rusqlite::Connection,
}

impl SessionProjection {
    /// Abre (ou cria) `sessions.db` em `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, WatchError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(|source| WatchError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let conn = rusqlite::Connection::open(dir.join("sessions.db"))?;
        conn.busy_timeout(std::time::Duration::from_millis(3000))?;
        enable_wal(&conn)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Insere/atualiza a sessão (chave `(cli, session_id)` — upsert, nunca duplica).
    pub fn upsert(&mut self, s: &Session) -> Result<(), WatchError> {
        let subagents = serde_json::to_string(&s.subagents).map_err(|source| WatchError::Json {
            path: "<subagents>".to_owned(),
            source: Box::new(source),
        })?;
        let tools = serde_json::to_string(&s.tools).map_err(|source| WatchError::Json {
            path: "<tools>".to_owned(),
            source: Box::new(source),
        })?;
        self.conn.execute(
            "INSERT OR REPLACE INTO sessions
             (cli, session_id, tokens_in, tokens_out, tokens_cache, tokens_thinking,
              cost_usd, cost_estimated, model, cwd, last_ts, subagents, tools, source)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            rusqlite::params![
                s.cli,
                s.session_id,
                s.tokens_in as i64,
                s.tokens_out as i64,
                s.tokens_cache as i64,
                s.tokens_thinking as i64,
                s.cost_usd,
                s.cost_estimated,
                s.model,
                s.cwd,
                s.last_ts,
                subagents,
                tools,
                s.source.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Sessão persistida, se existir.
    pub fn get(&self, cli: &str, session_id: &str) -> Result<Option<Session>, WatchError> {
        use rusqlite::OptionalExtension;
        let row = self
            .conn
            .query_row(
                "SELECT tokens_in, tokens_out, tokens_cache, tokens_thinking, cost_usd,
                        cost_estimated, model, cwd, last_ts, subagents, tools, source
                 FROM sessions WHERE cli = ?1 AND session_id = ?2",
                rusqlite::params![cli, session_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, f64>(4)?,
                        r.get::<_, bool>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, String>(9)?,
                        r.get::<_, String>(10)?,
                        r.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((t_in, t_out, t_cache, t_think, cost, est, model, cwd, last_ts, sub, tools, src)) =
            row
        else {
            return Ok(None);
        };
        let parse_vec = |src: &str| -> Result<Vec<String>, WatchError> {
            serde_json::from_str(src).map_err(|source| WatchError::Json {
                path: "<projeção sessions.db>".to_owned(),
                source: Box::new(source),
            })
        };
        Ok(Some(Session {
            cli: cli.to_owned(),
            session_id: session_id.to_owned(),
            tokens_in: t_in.max(0) as u64,
            tokens_out: t_out.max(0) as u64,
            tokens_cache: t_cache.max(0) as u64,
            tokens_thinking: t_think.max(0) as u64,
            cost_usd: cost,
            cost_estimated: est,
            model,
            cwd,
            last_ts,
            subagents: parse_vec(&sub)?,
            tools: parse_vec(&tools)?,
            source: CostSource::from_str_or_jsonl(&src),
        }))
    }

    /// Nº de sessões na projeção.
    pub fn count(&self) -> Result<u64, WatchError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    }
}

/// `true` se o erro do rusqlite é `SQLITE_BUSY` (disputa transitória de lock).
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _) if err.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

/// Liga `journal_mode=WAL` tolerando a corrida de setup entre conexões (mesmo
/// footgun do EventStore/ScrollbackStore: a troca de journal não honra
/// `busy_timeout` de forma confiável). Retry BOUNDED.
fn enable_wal(conn: &rusqlite::Connection) -> Result<(), WatchError> {
    const TRIES: u32 = 50; // ~50 × 20ms = 1s
    let mut left = TRIES;
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) && left > 1 => {
                left -= 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Erros do watch/scan — acionáveis (carregam o caminho), nunca panic.
#[derive(Debug, Error)]
pub enum WatchError {
    /// Falha de I/O ao abrir/ler um session-file.
    #[error("falha de I/O em '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// Linha JSONL malformada.
    #[error("JSONL inválido em '{path}': {source}")]
    Json {
        path: String,
        #[source]
        source: Box<serde_json::Error>,
    },
    /// Falha na projeção SQLite.
    #[error("sqlite (projeção): {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Tempdir manual (sem dev-dep): nome único por processo + tag.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("lina-sw-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).expect("criar tempdir");
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

    /// Fixture SINTÉTICA fiel ao formato de session-file do Claude Code
    /// (campos/estrutura reais; conteúdo inventado — nunca sessão privada).
    const FIXTURE: &str = concat!(
        r#"{"type":"user","sessionId":"sess-aaa","cwd":"/work/projeto-a","timestamp":"2026-06-06T12:00:00.000Z","message":{"role":"user","content":"oi"}}"#,
        "\n",
        r#"{"type":"assistant","sessionId":"sess-aaa","cwd":"/work/projeto-a","timestamp":"2026-06-06T12:00:05.000Z","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":40,"cache_creation_input_tokens":10,"cache_read_input_tokens":25},"content":[{"type":"tool_use","name":"Bash","input":{}}]},"costUSD":0.0123}"#,
        "\n",
        r#"{"type":"assistant","sessionId":"sess-aaa","cwd":"/work/projeto-a","timestamp":"2026-06-06T12:00:09.000Z","message":{"model":"claude-opus-4-8","usage":{"input_tokens":50,"output_tokens":60,"cache_read_input_tokens":5,"thinking_tokens":8},"content":[{"type":"tool_use","name":"Read"},{"type":"text","text":"ok"}]}}"#,
        "\n",
        r#"{"type":"assistant","sessionId":"sess-aaa","cwd":"/work/projeto-a","timestamp":"2026-06-06T12:00:12.000Z","isSidechain":true,"agentId":"explorer-1","message":{"usage":{"input_tokens":5,"output_tokens":5},"content":[{"type":"tool_use","name":"Grep"}]}}"#,
        "\n",
    );

    /// Fixture no formato ATUAL do Claude Code (verificado em session-files reais de
    /// 2026-06): NÃO existe `costUSD` em linha alguma — só `message.usage` — e um
    /// MESMO request gera 2-4 linhas `assistant` repetindo o MESMO `usage` (medido:
    /// 56/62 requests multi-linha num arquivo real). Conteúdo inventado; estrutura fiel.
    const FIXTURE_REAL: &str = concat!(
        // req_A: 2 linhas com o MESMO requestId + MESMO usage (duplicação real do formato).
        r#"{"type":"assistant","sessionId":"sess-real","requestId":"req_A","cwd":"/Users/test/Library/Application Support/Lina/walking-skeleton/t0","timestamp":"2026-06-07T14:10:12.010Z","message":{"id":"msg_A","model":"claude-opus-4-8","usage":{"input_tokens":16456,"output_tokens":2896,"cache_creation_input_tokens":41804,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_1h_input_tokens":41804,"ephemeral_5m_input_tokens":0},"service_tier":"standard"},"content":[{"type":"text","text":"oi"}]}}"#,
        "\n",
        r#"{"type":"assistant","sessionId":"sess-real","requestId":"req_A","cwd":"/Users/test/Library/Application Support/Lina/walking-skeleton/t0","timestamp":"2026-06-07T14:10:13.000Z","message":{"id":"msg_A","model":"claude-opus-4-8","usage":{"input_tokens":16456,"output_tokens":2896,"cache_creation_input_tokens":41804,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_1h_input_tokens":41804,"ephemeral_5m_input_tokens":0},"service_tier":"standard"},"content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#,
        "\n",
        // req_B: 1 linha, cache write 5m + cache read.
        r#"{"type":"assistant","sessionId":"sess-real","requestId":"req_B","cwd":"/Users/test/Library/Application Support/Lina/walking-skeleton/t0","timestamp":"2026-06-07T14:10:49.056Z","message":{"id":"msg_B","model":"claude-opus-4-8","usage":{"input_tokens":2,"output_tokens":378,"cache_creation_input_tokens":1245,"cache_read_input_tokens":64702,"cache_creation":{"ephemeral_5m_input_tokens":1245,"ephemeral_1h_input_tokens":0}},"content":[{"type":"text","text":"ok"}]}}"#,
        "\n",
    );

    /// Preço oficial (USD/Mtok) usado nas contas esperadas dos testes — Opus 4.5+.
    const OPUS_IN: f64 = 5.0 / 1e6;
    const OPUS_OUT: f64 = 25.0 / 1e6;

    /// Custo total esperado da [`FIXTURE_REAL`], request a request (dedup por requestId):
    /// cache write 1h = 2× input; write 5m = 1.25×; read = 0.1× (multiplicadores da API).
    fn fixture_real_expected_cost() -> f64 {
        let req_a = 16456.0 * OPUS_IN + 2896.0 * OPUS_OUT + 41804.0 * OPUS_IN * 2.0;
        let req_b =
            2.0 * OPUS_IN + 378.0 * OPUS_OUT + 1245.0 * OPUS_IN * 1.25 + 64702.0 * OPUS_IN * 0.1;
        req_a + req_b
    }

    // ── BUG custo-zero no .app (F1-1-5): formato real SEM costUSD → custo derivado ──

    /// O bug do fundador: session-file no formato ATUAL (sem `costUSD`) deixava
    /// `cost_usd = 0` → card "sem estimativa de custo ainda" para sempre. O custo deve
    /// ser DERIVADO de `usage` × preço do modelo (estimativa honesta), com dedup por
    /// `requestId` (senão tokens e custo inflam ~3× — duplicação medida no formato real).
    #[test]
    fn real_format_without_costusd_derives_estimated_cost_deduped() {
        let tmp = TempDir::new("real-cost");
        let file = tmp.path().join("sess-real.jsonl");
        std::fs::write(&file, FIXTURE_REAL).expect("escrever fixture");

        let mut scanner = SessionScanner::new();
        scanner.scan_file("claude-code", &file).expect("scan");
        let s = scanner
            .session("claude-code", "sess-real")
            .expect("sessão agregada");

        // Dedup: req_A tem 2 linhas com o MESMO usage → conta UMA vez.
        assert_eq!(s.tokens_in, 16456 + 2, "usage de req_A não pode dobrar");
        assert_eq!(s.tokens_out, 2896 + 378);
        assert_eq!(s.tokens_cache, 41804 + 1245 + 64702);

        // O coração do bug: custo > 0 derivado de usage × preço, mesmo sem costUSD.
        assert!(
            s.cost_usd > 0.0,
            "formato real sem costUSD precisa render custo estimado (era o bug do .app)"
        );
        assert!(
            (s.cost_usd - fixture_real_expected_cost()).abs() < 1e-9,
            "custo {} != esperado {}",
            s.cost_usd,
            fixture_real_expected_cost()
        );
        assert!(s.cost_estimated, "derivado de usage → SEMPRE estimativa");
        // cwd REAL com espaços (Application Support) preservado p/ correlação sessão↔nó.
        assert_eq!(
            s.cwd.as_deref(),
            Some("/Users/test/Library/Application Support/Lina/walking-skeleton/t0")
        );
    }

    /// Honestidade: modelo fora da tabela de preço → custo NÃO é chutado (fica 0 →
    /// o dashboard exibe "sem estimativa ainda", nunca um número inventado).
    #[test]
    fn unknown_model_without_costusd_stays_without_estimate() {
        let tmp = TempDir::new("unknown-model");
        let file = tmp.path().join("sess-x.jsonl");
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"assistant","sessionId":"sess-x","requestId":"req_X","message":{"model":"futuro-llm-99","usage":{"input_tokens":100,"output_tokens":50}}}"#,
                "\n"
            ),
        )
        .expect("escrever");
        let mut scanner = SessionScanner::new();
        scanner.scan_file("claude-code", &file).expect("scan");
        let s = scanner.session("claude-code", "sess-x").expect("sessão");
        assert_eq!(s.tokens_in, 100, "tokens contam mesmo sem preço");
        assert!(
            s.cost_usd == 0.0,
            "modelo desconhecido nunca chuta preço (got {})",
            s.cost_usd
        );
    }

    /// REPRO headless do ambiente do .app (Finder): home injetado, layout REAL de
    /// `~/.claude/projects/<cwd-codificado>/<uuid>.jsonl`, pattern do TOML de produção
    /// e JSONL no formato atual. Antes do fix: custo 0 ("sem estimativa" eterno).
    /// O processo de teste pode rodar sob `env -i HOME=… PATH=/usr/bin:/bin` — nada
    /// aqui depende de env além do home INJETADO (mesma resolução do boot).
    #[test]
    fn headless_app_bundle_repro_yields_estimated_cost() {
        let tmp = TempDir::new("app-repro");
        // Layout real: nome de pasta codificado como o claude grava p/ cwd com espaços.
        let proj = tmp.path().join(
            ".claude/projects/-Users-test-Library-Application-Support-Lina-walking-skeleton-t0",
        );
        std::fs::create_dir_all(&proj).expect("árvore");
        std::fs::write(
            proj.join("0eecfc25-a46c-4358-b4ed-cb0bc29fce70.jsonl"),
            FIXTURE_REAL,
        )
        .expect("fixture");

        let mut watch = SessionWatch::with_home(tmp.path());
        // Pattern EXATO do profiles/claude-code.toml de produção (F1-1-1).
        watch.add_source("claude-code", "~/.claude/projects/*/*.jsonl");
        let out = watch.poll_once().expect("poll");
        assert_eq!(out.files_scanned, 1, "descobriu o session-file do .app");

        let s = watch
            .scanner()
            .session("claude-code", "sess-real")
            .expect("sessão do terminal do .app");
        assert!(
            s.cost_usd > 0.0,
            "ambiente do .app precisa produzir custo estimado (era o bug)"
        );
        assert!(s.cost_estimated);
        assert_eq!(
            s.cwd.as_deref(),
            Some("/Users/test/Library/Application Support/Lina/walking-skeleton/t0"),
            "cwd com espaços preservado → correlação sessão↔nó casa com o hint do spawn"
        );
    }

    // ── Ciclo A (F1-1-2 critério 1): agregação da fixture no schema único ──

    #[test]
    fn aggregates_fixture_into_unified_session_schema() {
        let tmp = TempDir::new("agg");
        let file = tmp.path().join("sess-aaa.jsonl");
        std::fs::write(&file, FIXTURE).expect("escrever fixture");

        let mut scanner = SessionScanner::new();
        scanner.scan_file("claude-code", &file).expect("scan");

        let s = scanner
            .session("claude-code", "sess-aaa")
            .expect("sessão agregada existe");

        // Tokens in/out/cache/thinking (cache = creation + read).
        assert_eq!(s.tokens_in, 155);
        assert_eq!(s.tokens_out, 105);
        assert_eq!(s.tokens_cache, 40);
        assert_eq!(s.tokens_thinking, 8);

        // Custo: linha COM costUSD usa o valor gravado (0.0123); linhas SÓ com usage
        // derivam a estimativa tokens×preço (fix do custo-zero — o formato atual nunca
        // grava costUSD). Linha 3 (modelo opus-4-8): 50×in + 60×out + 5×read(0,1×in).
        // Linha 4 (sidechain, sem model → último da sessão): 5×in + 5×out.
        let est_line3 = 50.0 * OPUS_IN + 60.0 * OPUS_OUT + 5.0 * OPUS_IN * 0.1;
        let est_line4 = 5.0 * OPUS_IN + 5.0 * OPUS_OUT;
        let expected = 0.0123 + est_line3 + est_line4;
        assert!(
            (s.cost_usd - expected).abs() < 1e-9,
            "custo {} != {expected}",
            s.cost_usd
        );
        assert!(s.cost_estimated, "fonte JSONL → cost_estimated = true");

        // Identidade/atividade.
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(s.cwd.as_deref(), Some("/work/projeto-a"));
        assert_eq!(s.last_ts.as_deref(), Some("2026-06-06T12:00:12.000Z"));

        // Ferramentas e subagentes distintos (ordem estável).
        assert_eq!(s.tools, vec!["Bash", "Grep", "Read"]);
        assert_eq!(s.subagents, vec!["explorer-1"]);
    }

    // ── Ciclo B (F1-1-2 critério 1): re-parse lê SÓ o delta, provado por bytes lidos ──

    #[test]
    fn incremental_rescan_reads_only_the_delta() {
        let tmp = TempDir::new("delta");
        let file = tmp.path().join("sess-aaa.jsonl");
        std::fs::write(&file, FIXTURE).expect("escrever fixture");

        let mut scanner = SessionScanner::new();
        scanner.scan_file("claude-code", &file).expect("scan 1");
        let after_first = scanner.bytes_read_total();
        assert_eq!(
            after_first,
            FIXTURE.len() as u64,
            "1º scan lê o arquivo inteiro"
        );

        // Re-scan SEM mudança: zero bytes novos.
        scanner.scan_file("claude-code", &file).expect("scan 2");
        assert_eq!(
            scanner.bytes_read_total(),
            after_first,
            "sem delta → 0 bytes"
        );

        // Append de 1 linha completa → o re-scan lê SÓ o delta.
        let delta = concat!(
            r#"{"type":"assistant","sessionId":"sess-aaa","timestamp":"2026-06-06T12:00:20.000Z","message":{"usage":{"input_tokens":1,"output_tokens":2}}}"#,
            "\n"
        );
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .expect("abrir p/ append");
            f.write_all(delta.as_bytes()).expect("append");
        }
        scanner.scan_file("claude-code", &file).expect("scan 3");
        assert_eq!(
            scanner.bytes_read_total(),
            after_first + delta.len() as u64,
            "re-parse leu só o delta"
        );
        // E o agregado absorveu o delta (155+1 / 105+2).
        let s = scanner.session("claude-code", "sess-aaa").expect("sessão");
        assert_eq!(s.tokens_in, 156);
        assert_eq!(s.tokens_out, 107);
        assert_eq!(s.last_ts.as_deref(), Some("2026-06-06T12:00:20.000Z"));
    }

    /// Arquivo TRUNCADO/reescrito (rotação): cursor reseta e a sessão é re-derivada
    /// do conteúdo novo — sem dupla contagem do conteúdo antigo.
    #[test]
    fn truncated_file_resets_cursor_and_rederives_session() {
        let tmp = TempDir::new("trunc");
        let file = tmp.path().join("sess-aaa.jsonl");
        std::fs::write(&file, FIXTURE).expect("escrever fixture");

        let mut scanner = SessionScanner::new();
        scanner.scan_file("claude-code", &file).expect("scan 1");
        assert_eq!(
            scanner
                .session("claude-code", "sess-aaa")
                .expect("sessão")
                .tokens_in,
            155
        );

        // Reescreve MENOR (truncation): só 1 linha de 7 tokens de input.
        let rewritten = concat!(
            r#"{"type":"assistant","sessionId":"sess-aaa","timestamp":"2026-06-06T13:00:00.000Z","message":{"usage":{"input_tokens":7}}}"#,
            "\n"
        );
        std::fs::write(&file, rewritten).expect("reescrever menor");
        scanner.scan_file("claude-code", &file).expect("scan 2");

        let s = scanner.session("claude-code", "sess-aaa").expect("sessão");
        assert_eq!(
            s.tokens_in, 7,
            "truncation re-deriva (não soma sobre o antigo)"
        );
        assert_eq!(s.last_ts.as_deref(), Some("2026-06-06T13:00:00.000Z"));
    }

    /// Linha não-JSON e linha gigante NÃO derrubam o scan: são puladas e CONTADAS
    /// (doutrina "nunca engula erro" — o contador é o sinal observável).
    #[test]
    fn invalid_and_oversized_lines_are_skipped_and_counted() {
        let tmp = TempDir::new("skip");
        let file = tmp.path().join("sess-bbb.jsonl");
        let giant = format!(
            "{{\"sessionId\":\"sess-bbb\",\"pad\":\"{}\"}}\n",
            "x".repeat(MAX_LINE_BYTES)
        );
        let mut content = String::new();
        content.push_str(
            r#"{"type":"assistant","sessionId":"sess-bbb","message":{"usage":{"input_tokens":3}}}"#,
        );
        content.push('\n');
        content.push_str("isto nao é json\n");
        content.push_str(&giant);
        content.push_str(
            r#"{"type":"assistant","sessionId":"sess-bbb","message":{"usage":{"input_tokens":4}}}"#,
        );
        content.push('\n');
        std::fs::write(&file, &content).expect("escrever");

        let mut scanner = SessionScanner::new();
        scanner
            .scan_file("claude-code", &file)
            .expect("scan não derruba");

        let s = scanner.session("claude-code", "sess-bbb").expect("sessão");
        assert_eq!(s.tokens_in, 7, "linhas válidas em volta foram agregadas");
        assert_eq!(scanner.skipped_lines(), 2, "inválida + gigante contadas");
    }

    // ── Ciclo C: descoberta via session_dir_pattern (F1-1-1) + poll incremental ──

    #[test]
    fn watch_discovers_files_from_pattern_and_polls_only_changes() {
        let tmp = TempDir::new("watch");
        // "Home" fake com a estrutura real do Claude Code (inv#3: o padrão vem
        // do TOML, o teste injeta o home — nada de caminho real do usuário).
        let proj = tmp.path().join(".claude/projects/proj-x");
        std::fs::create_dir_all(&proj).expect("criar árvore");
        std::fs::write(proj.join("sess-aaa.jsonl"), FIXTURE).expect("fixture");

        let mut watch = SessionWatch::with_home(tmp.path());
        watch.add_source("claude-code", "~/.claude/projects/*/*.jsonl");

        // 1º poll: descobre e escaneia o arquivo.
        let out = watch.poll_once().expect("poll 1");
        assert_eq!(out.files_scanned, 1);
        assert!(out
            .sessions_updated
            .contains(&("claude-code".to_owned(), "sess-aaa".to_owned())));
        assert!(watch.scanner().session("claude-code", "sess-aaa").is_some());

        // 2º poll sem mudança: nada re-escaneado (incremental de verdade).
        let out = watch.poll_once().expect("poll 2");
        assert_eq!(out.files_scanned, 0);
        assert!(out.sessions_updated.is_empty());

        // Append → só o delta; arquivo NOVO em outro projeto → descoberto.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(proj.join("sess-aaa.jsonl"))
                .expect("append");
            f.write_all(
                concat!(
                    r#"{"type":"assistant","sessionId":"sess-aaa","message":{"usage":{"output_tokens":9}}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .expect("write");
        }
        let proj2 = tmp.path().join(".claude/projects/proj-y");
        std::fs::create_dir_all(&proj2).expect("proj2");
        std::fs::write(
            proj2.join("sess-zzz.jsonl"),
            concat!(
                r#"{"type":"assistant","sessionId":"sess-zzz","cwd":"/work/projeto-z","message":{"usage":{"input_tokens":2}}}"#,
                "\n"
            ),
        )
        .expect("fixture 2");

        let out = watch.poll_once().expect("poll 3");
        assert_eq!(out.files_scanned, 2, "delta no antigo + arquivo novo");
        assert_eq!(
            watch
                .scanner()
                .session("claude-code", "sess-aaa")
                .expect("aaa")
                .tokens_out,
            105 + 9
        );
        assert!(watch.scanner().session("claude-code", "sess-zzz").is_some());

        // Diretório de um CLI que nunca rodou: pattern sem matches NÃO é erro.
        watch.add_source("codex", "~/.codex/sessions/*.jsonl");
        let out = watch.poll_once().expect("poll 4");
        assert_eq!(out.files_scanned, 0);
    }

    // ── Ciclo D (F1-1-2 critério 2): correlação sessão↔nó por (cwd, janela de mtime) ──

    #[test]
    fn correlates_sessions_to_nodes_by_cwd_and_mtime_window() {
        let hints = [
            NodeHint {
                node_id: "node-a".to_owned(),
                cwd: "/work/projeto-a".to_owned(),
                alive_from_ms: 1_000,
                alive_to_ms: None, // vivo
            },
            NodeHint {
                node_id: "node-b".to_owned(),
                cwd: "/work/projeto-b".to_owned(),
                alive_from_ms: 1_000,
                alive_to_ms: None,
            },
        ];

        // 2 sessões em cwds distintos → cada uma no seu nó, SEM ambiguidade.
        assert_eq!(
            correlate("/work/projeto-a", 5_000, &hints),
            Correlation::Unique("node-a".to_owned())
        );
        assert_eq!(
            correlate("/work/projeto-b", 5_000, &hints),
            Correlation::Unique("node-b".to_owned())
        );

        // cwd sem nó → None (não chuta).
        assert_eq!(correlate("/outro/lugar", 5_000, &hints), Correlation::None);

        // mtime FORA da janela viva do nó → None (sessão antiga de outra vida).
        let finished = [NodeHint {
            node_id: "node-velho".to_owned(),
            cwd: "/work/projeto-a".to_owned(),
            alive_from_ms: 1_000,
            alive_to_ms: Some(2_000),
        }];
        assert_eq!(
            correlate("/work/projeto-a", 5_000, &finished),
            Correlation::None
        );

        // 2 nós no MESMO cwd com janelas sobrepostas → Ambiguous (confiança é
        // agregada, nunca decisória — camada 3 não chuta identidade).
        let twins = [
            NodeHint {
                node_id: "node-1".to_owned(),
                cwd: "/work/x".to_owned(),
                alive_from_ms: 0,
                alive_to_ms: None,
            },
            NodeHint {
                node_id: "node-2".to_owned(),
                cwd: "/work/x".to_owned(),
                alive_from_ms: 0,
                alive_to_ms: None,
            },
        ];
        assert_eq!(
            correlate("/work/x", 5_000, &twins),
            Correlation::Ambiguous(vec!["node-1".to_owned(), "node-2".to_owned()])
        );
    }

    // ── Ciclo E: projeção SQLite — reconstruível do zero (inv#4: NUNCA autoridade) ──

    #[test]
    fn sqlite_projection_upserts_and_is_rebuildable_from_scratch() {
        let tmp = TempDir::new("proj");
        let file = tmp.path().join("sess-aaa.jsonl");
        std::fs::write(&file, FIXTURE).expect("fixture");
        let mut scanner = SessionScanner::new();
        scanner.scan_file("claude-code", &file).expect("scan");
        let session = scanner.session("claude-code", "sess-aaa").expect("sessão");

        let db_dir = tmp.path().join("db");
        {
            let mut proj = SessionProjection::open(&db_dir).expect("open");
            proj.upsert(&session).expect("upsert");
            assert_eq!(proj.count().expect("count"), 1);

            // Upsert do MESMO par (cli, session_id) sobrescreve — não duplica.
            let mut updated = session.clone();
            updated.tokens_out += 9;
            proj.upsert(&updated).expect("re-upsert");
            assert_eq!(proj.count().expect("count"), 1);

            let read = proj
                .get("claude-code", "sess-aaa")
                .expect("get")
                .expect("existe");
            assert_eq!(read.tokens_out, session.tokens_out + 9);
            assert_eq!(read.tokens_in, 155);
            assert!(read.cost_estimated, "estimativa sobrevive ao round-trip");
            assert_eq!(read.tools, vec!["Bash", "Grep", "Read"]);
            assert_eq!(read.subagents, vec!["explorer-1"]);
            assert_eq!(read.cwd.as_deref(), Some("/work/projeto-a"));
        } // fecha a conexão

        // PROJEÇÃO: apagar o .db e re-derivar da fonte produz o mesmo conteúdo.
        std::fs::remove_dir_all(&db_dir).expect("apagar projeção");
        let mut proj2 = SessionProjection::open(&db_dir).expect("reopen do zero");
        proj2.upsert(&session).expect("re-derivar");
        let read = proj2
            .get("claude-code", "sess-aaa")
            .expect("get")
            .expect("existe de novo");
        assert_eq!(read, session, "reconstruída byte-a-byte do agregado fonte");
    }

    // ── Ciclo F (critério 5): medição REAL — targets do 13.5 são a medir, não premissas ──

    /// Gera uma fixture sintética grande (formato fiel) e MEDE: full-scan, refresh
    /// incremental e RAM retida (determinística — bytes da estrutura, não RSS).
    /// Os números saem no stderr (`--nocapture`) e vão pro relatório da story;
    /// os asserts duros são ESTRUTURAIS (agregação ≪ arquivo; delta exato), não
    /// de tempo (tempo é registro, não promessa — e varia por máquina/build).
    #[test]
    fn measurement_large_fixture_streaming_and_retained_ram() {
        let tmp = TempDir::new("measure");
        let file = tmp.path().join("sess-big.jsonl");

        const LINES: usize = 50_000;
        let mut content = String::with_capacity(LINES * 260);
        for i in 0..LINES {
            content.push_str(&format!(
                r#"{{"type":"assistant","sessionId":"sess-big","cwd":"/work/grande","timestamp":"2026-06-06T12:{:02}:{:02}.000Z","message":{{"model":"claude-opus-4-8","usage":{{"input_tokens":3,"output_tokens":7,"cache_read_input_tokens":2}},"content":[{{"type":"tool_use","name":"Tool{}"}}]}}}}"#,
                (i / 60) % 60,
                i % 60,
                i % 10
            ));
            content.push('\n');
        }
        std::fs::write(&file, &content).expect("escrever fixture grande");
        let file_bytes = content.len() as u64;

        // Full-scan inicial (streaming linha a linha).
        let mut scanner = SessionScanner::new();
        let t0 = std::time::Instant::now();
        scanner.scan_file("claude-code", &file).expect("full scan");
        let full_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let s = scanner.session("claude-code", "sess-big").expect("sessão");
        assert_eq!(s.tokens_in, 3 * LINES as u64, "agregação íntegra");
        assert_eq!(s.tools.len(), 10, "tools distintas (não 50k)");

        // RAM retida DETERMINÍSTICA: agregados + cursores ≪ arquivo (streaming
        // de verdade — reter ~o arquivo seria a classe de leak).
        let retained = scanner.ram_bytes() as u64;
        assert!(
            retained < file_bytes / 100,
            "retido {retained}B não é ≪ arquivo {file_bytes}B — scanner está acumulando linhas?"
        );

        // REFRESH incremental: +200 linhas → poll lê SÓ o delta.
        let mut delta = String::new();
        for _ in 0..200 {
            delta.push_str(
                r#"{"type":"assistant","sessionId":"sess-big","message":{"usage":{"input_tokens":1,"output_tokens":1}}}"#,
            );
            delta.push('\n');
        }
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .expect("append");
            f.write_all(delta.as_bytes()).expect("write delta");
        }
        let before = scanner.bytes_read_total();
        let t1 = std::time::Instant::now();
        scanner.scan_file("claude-code", &file).expect("refresh");
        let refresh_ms = t1.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(
            scanner.bytes_read_total() - before,
            delta.len() as u64,
            "refresh consumiu exatamente o delta"
        );

        // Números REAIS para o relatório (targets 13.5: <50ms refresh / <20MB RAM).
        eprintln!(
            "[F1-1-2 medição] arquivo={:.1}MB linhas={LINES} | full_scan={full_ms:.1}ms \
             refresh(+200 linhas)={refresh_ms:.2}ms | ram_retida={retained}B (~{:.2}KB) \
             | build={}",
            file_bytes as f64 / 1e6,
            retained as f64 / 1e3,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
    }
}
