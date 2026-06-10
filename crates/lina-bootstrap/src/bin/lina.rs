//! Binário **`lina`** — os verbos do agente dentro do Lina Space.
//!
//! - `lina whoami [--bootstrap]` (W3-2): imprime o **ESTADO VIVO** do terminal corrente — papel,
//!   skills, vault, **colegas reais**, plano, autonomia — lendo `.lina/bootstrap.json` (escrito
//!   pelo app no cwd do terminal). Com `--bootstrap`, emite o JSON do hook `SessionStart`.
//! - `lina ask @<alvo> "<msg>" [--await] [--intent X] [--role PAPEL]` (W3-4): monta a mensagem
//!   canônica `lina/msg@1` (`from` = este terminal) e a **deposita na mailbox** (`.lina/outbox/`),
//!   que o supervisor (no app) observa, roteia e injeta no PTY do alvo.
//! - `lina handshake` (W3-4): registra presença (ping na mailbox, **0 broadcast**) e imprime os
//!   colegas do workspace (de `.lina/agents.json` ou do roster do bootstrap).

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use lina_bootstrap::{
    autonomy_from_env, classify_retro_args, pretooluse_output, project_retro, render_report,
    Autonomy, BootstrapInput, Bootstrapper, RetroInvocation,
};
use lina_core::{
    check_action, lookup_action, parse_autonomy, DomainEvent, EventStore, HandoffContract,
    MailMessage, Mailbox, CLASS_GATED_HARD_EXTERNAL,
};

/// Arquivo de estado, relativo ao cwd do terminal (o app o escreve antes de spawnar o shell).
const INPUT_PATH: &str = ".lina/bootstrap.json";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("whoami") => run_whoami(args.iter().any(|a| a == "--bootstrap")),
        Some("ask") => run_ask(&args[1..]),
        Some("handoff") => run_handoff(&args[1..]),
        Some("check") => run_check(&args[1..]),
        Some("broadcast") => run_broadcast(&args[1..]),
        Some("handshake") => run_handshake(),
        Some("plan") => run_plan(&args[1..]),
        Some("guard") => run_guard(&args[1..]),
        Some("resume") => run_resume(&args[1..]),
        Some("do") => run_do(&args[1..]),
        Some("list") => run_list(args.iter().any(|a| a == "--json")),
        Some("vault") => run_vault(&args[1..]),
        Some("spawn") => run_spawn(&args[1..]),
        Some("retro") => run_retro(&args[1..]),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "uso:\n  lina whoami [--bootstrap]\n  lina ask @<alvo> \"<msg>\" [--await] [--intent ask|handoff|broadcast|...] [--role PAPEL] [--reply-to <id>]\n  lina handoff @<alvo> \"<tarefa>\" [--context <arquivo>] [--ref plan:<id>] [--timeout-sec N] [--await]\n   (F1-0-6: delega COM contrato estruturado lina/msg@2 — schema de entrada/saida, timeout, retry;\n    --context ANEXA o conteudo do arquivo ao payload. Fire-and-forget por padrao; acompanhe com\n    `lina check`. Em autonomia manual o proprio comando recusa — delegacao bloqueada localmente.)\n  lina check @<alvo>   (F1-0-6: estado VIVO do colega — Ready/Busy/Idle/Blocked/Dead + motivo da\n   ultima transicao + travamento (ADR 0019) + ultima atividade A2A. LEITURA PURA de agents.json +\n   log.jsonl: nao injeta NADA no terminal do colega.)\n  lina broadcast \"*\" \"<msg>\"   (avisa TODOS os terminais vivos; --role PAPEL p/ um papel. ADR0007:\n   o fan-out INICIAL pedido pelo humano entrega a todos SEM gate; a CASCATA (re-espalhar) pede ok.)\n  lina handshake\n  lina plan read | claim <id> | check <id>\n  lina guard --check-action --cmd \"<comando>\" --autonomy <manual|assistido|autonomo>\n  lina guard --pretooluse   (hook PreToolUse do Claude Code: le JSON no stdin, emite a decisao em JSON no stdout)\n  lina resume   (W3-7c: PEDE retomada do teto de custo; o agente NAO des-pausa — gate humano na janela)\n  lina do <deploy|pay|send> [args]   (W3-6c: acao custodiada; o agente REGISTRA, NAO executa)\n  lina list [--json]   (W4-2: lista os agentes do workspace — nome/papel/status do agents.json)\n  lina vault path | index | read <nota> | search <termo>   (segundo cerebro: le os vault(s) Obsidian\n   linkados no onboarding em .lina/vault.json; `index` mostra o mapa estrutural PageIndex; `read`/`search`\n   acessam as notas. Comece por `index` para NAVEGAR antes de abrir notas.)\n  lina spawn @<Nome> --role <papel> [--prompt \"<1o prompt>\"]   (F1-3-6: PEDE criar um terminal novo\n   quando falta um papel. Gate inforjavel: ORIGEM ok; CASCATA/cap/custo pedem aval humano; manual\n   recusa. A criacao fisica e do Espaco — voce NAO cunha o terminal.)\n  lina retro [--json] [--now-ms <ms>]   (F1-3-7: auto-aprimoramento v0. Le o event log (SO-LEITURA) e\n   emite um RELATORIO deterministico de projecoes: skills (uso/stale>30d/archive>90d), coordenacao\n   (bloqueios/spawns gated/re-delegacoes/breaker), custos por terminal+outliers, pedidos de origem e\n   lacunas de papel. ZERO LLM: quem PROPOE melhorias e o agente (skill lina-retro), com gate humano.\n   So OBSERVA e SUGERE — nao existe `lina retro apply`; arquivar/fixar/mudar passa pelo humano.)\n\n  (--reply-to <id>: responde a uma pergunta --await; fecha o await do colega)\n  (resume: registra resume.request na fila de broker por-no; o supervisor apenda CostCeilingResumed SO\n   apos confirmacao HUMANA na janela (Cmd+Enter). O agente, sozinho, NUNCA tira do estado Paused.)\n  (guard --check-action: imprime allow|ask|deny; apenda ActionGated ao log quando NAO for allow)\n  (guard --pretooluse: autonomia via LINA_AUTONOMY (default assistido); fail-safe ask em erro)\n  (do: gated-hard-external; o segredo vive so no SecretVault do Lina. O agente nao tem o token nem\n   confirmacao -> registra o pedido + apenda ActionGated{{ask}}+BrokerDenied{{unconfirmed}}; quem executa\n   COM o segredo, apos gate humano, e o supervisor/broker. Custodia = camada inquebravel, ADR 0004.)"
    );
}

/// Raiz da mailbox: `LINA_HOME` (o app aponta para o `.lina/` compartilhado do workspace) ou, como
/// fallback standalone, o `.lina/` do cwd.
fn mailbox_root() -> PathBuf {
    std::env::var_os("LINA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".lina"))
}

/// Raiz da **fila de broker** (`<LINA_HOME>/broker/`) — irmã do outbox A2A, drenada SÓ pela
/// `BrokerPump` do app (o `Router` A2A nunca a varre). É onde `lina do` deposita o pedido custodiado
/// para o supervisor rodar o gate humano + `run_custody` (ADR 0004). Separá-la do outbox A2A evita
/// que o pedido seja tratado como entrega A2A (e consumido como `NoTarget`).
fn broker_mailbox_root() -> PathBuf {
    mailbox_root().join("broker")
}

/// **W3-6c A3 — enfileira no outbox POR-NÓ** (`enqueue_as`) para o A2A (`lina ask`): cada PTY escreve
/// no SEU subdir e o supervisor atribui `from` = dir-dono (origem inforjável). **ESTRITO — SEM fallback
/// ao outbox flat** (BUG 2 / regressão do Round 6): desde a anonimização A3 do drain flat (o `from` do
/// flat vira VAZIO, anti-impersonação), um fallback flat depositaria a msg num caminho cujo `from=""` o
/// router RECUSA como `UnknownSender` → a mensagem SOME em silêncio (o que o fundador viu: "envio nada
/// acontece"). Falhar VISÍVEL é melhor: um nome de nó inseguro/vazio retorna `Err` (o `lina ask` imprime
/// e sai 1), em vez de degradar para um caminho que será descartado. Mesma doutrina ESTRITA já usada por
/// `lina do`/`lina resume` (hole 3) — o `from` autenticado por origem é a ÚNICA via.
fn enqueue_per_node(mailbox: &Mailbox, node: &str, msg: &MailMessage) -> std::io::Result<()> {
    mailbox.enqueue_as(node, msg)
}

fn load_input() -> Result<BootstrapInput, String> {
    let data = std::fs::read_to_string(INPUT_PATH).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

/// `hook = true` → JSON do `SessionStart`; `false` → bloco legível.
fn run_whoami(hook: bool) -> ExitCode {
    let input = match load_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH}: {e}");
            return ExitCode::from(1);
        }
    };
    let bs = match Bootstrapper::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("lina: registry de papeis invalido: {e}");
            return ExitCode::from(1);
        }
    };
    if hook {
        println!("{}", bs.whoami_hook_json(&input));
    } else {
        println!("{}", bs.whoami(&input));
    }
    ExitCode::SUCCESS
}

/// `lina ask` — monta a `MailMessage` e a deposita na mailbox.
fn run_ask(args: &[String]) -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut intent = String::from("ask");
    let mut await_reply = false;
    let mut role: Option<String> = None;
    let mut reply_to: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--await" => await_reply = true,
            "--intent" => {
                i += 1;
                match args.get(i) {
                    Some(v) => intent = v.clone(),
                    None => {
                        eprintln!("lina: --intent exige um valor");
                        return ExitCode::from(2);
                    }
                }
            }
            "--reply-to" => {
                i += 1;
                match args.get(i) {
                    Some(v) => reply_to = Some(v.clone()),
                    None => {
                        eprintln!("lina: --reply-to exige o id da pergunta");
                        return ExitCode::from(2);
                    }
                }
            }
            "--role" => {
                i += 1;
                match args.get(i) {
                    Some(v) => role = Some(v.clone()),
                    None => {
                        eprintln!("lina: --role exige um valor (papel)");
                        return ExitCode::from(2);
                    }
                }
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    // Alvo + mensagem: com --role, o 1º positional é a mensagem; senão é alvo + mensagem.
    let (to, payload) = if let Some(r) = role {
        match positional.into_iter().next() {
            Some(msg) => (format!("role:{r}"), msg),
            None => {
                usage();
                return ExitCode::from(2);
            }
        }
    } else {
        let mut it = positional.into_iter();
        match (it.next(), it.next()) {
            (Some(target), Some(msg)) => (target, msg),
            _ => {
                usage();
                return ExitCode::from(2);
            }
        }
    };

    let from = match load_input() {
        Ok(i) => i.terminal_name,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'): {e}");
            return ExitCode::from(1);
        }
    };

    let mut msg = MailMessage::new(from.clone(), to, intent, payload);
    if await_reply {
        msg = msg.awaiting();
    }
    if let Some(rt) = reply_to {
        msg = msg.replying_to(rt);
    }
    enqueue_and_report(&from, msg)
}

/// Enfileira no outbox POR-NÓ (W3-6c A3: `from` autenticado pela origem, dir-dono) e
/// reporta o desfecho REAL do roteamento — caminho compartilhado por `ask` e `handoff`
/// (F1-0-6: o handoff é açúcar estruturado sobre a MESMA fila, nunca um canal novo).
///
/// CONFIRMAÇÃO REAL (fix do "envio nada acontece"): a entrega é assíncrona (o supervisor
/// roteia DEPOIS), mas o resultado é registrado no event log. Antes, imprimíamos um
/// "ok: enviada" CEGO mesmo quando o roteador BLOQUEAVA a msg (unknown_sender/no_target) —
/// o agente não sabia que falhou e concluía que o colega era um "stub mudo". Aguardamos
/// (poll bounded no espelho `log.jsonl`) o desfecho REAL e o reportamos.
fn enqueue_and_report(from: &str, msg: MailMessage) -> ExitCode {
    let mailbox = Mailbox::new(mailbox_root());
    match enqueue_per_node(&mailbox, from, &msg) {
        Ok(()) => match poll_route_outcome(&msg.id) {
            RouteConfirm::Delivered { to_node } => {
                let dst = if to_node.is_empty() {
                    msg.to.clone()
                } else {
                    to_node
                };
                println!("ok: {dst} recebeu a mensagem (id {}).", msg.id);
                ExitCode::SUCCESS
            }
            RouteConfirm::Blocked { reason } => {
                eprintln!(
                    "lina: a mensagem NAO chegou a {} — o Espaco a bloqueou ({}).\n{}",
                    msg.to,
                    explain_block(&reason),
                    block_hint(&reason)
                );
                ExitCode::from(1)
            }
            RouteConfirm::Pending => {
                println!(
                    "ok: enviada a {} (id {}); ainda SEM confirmacao de entrega apos a espera (o \
                     Espaco pode estar ocupado). Confirme com `lina list` se o destino esta vivo e \
                     tente de novo — NAO conclua que o colega e um stub.",
                    msg.to, msg.id
                );
                ExitCode::SUCCESS
            }
        },
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
    }
}

/// **F1-0-6 — `lina handoff @<alvo> "<tarefa>" [--context arq] [--ref plan:ID]
/// [--timeout-sec N] [--await]`**: delegação ESTRUTURADA no contrato `lina/msg@2`
/// (F1-0-5) — açúcar sobre a mesma fila do `ask`, com `intent=handoff` canônico e o
/// [`HandoffContract`] completo (o router valida campo a campo; nada implícito "que o
/// outro agente deve adivinhar"). Capacidade sensível = VERBO estruturado (doutrina
/// InsForge do épico), nunca o contorno `ask --intent handoff` sem contrato.
fn run_handoff(args: &[String]) -> ExitCode {
    let mut positional: Vec<String> = Vec::new();
    let mut context: Option<String> = None;
    let mut ref_id: Option<String> = None;
    let mut timeout_sec: u64 = 600;
    let mut await_reply = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--await" => await_reply = true,
            "--context" => {
                i += 1;
                match args.get(i) {
                    Some(v) => context = Some(v.clone()),
                    None => {
                        eprintln!("lina: --context exige um arquivo");
                        return ExitCode::from(2);
                    }
                }
            }
            "--ref" => {
                i += 1;
                match args.get(i) {
                    Some(v) => ref_id = Some(v.clone()),
                    None => {
                        eprintln!("lina: --ref exige um id (ex.: plan:T4)");
                        return ExitCode::from(2);
                    }
                }
            }
            "--timeout-sec" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u64>().ok()) {
                    Some(v) if v >= 1 => timeout_sec = v,
                    _ => {
                        eprintln!("lina: --timeout-sec exige um inteiro >= 1");
                        return ExitCode::from(2);
                    }
                }
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    let mut it = positional.into_iter();
    let (Some(to), Some(task)) = (it.next(), it.next()) else {
        usage();
        return ExitCode::from(2);
    };

    let input = match load_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'): {e}");
            return ExitCode::from(1);
        }
    };
    // Bloqueio LOCAL em `manual` (doutrina bloco 5: "garantido pelo proprio comando, nao
    // por hook") — handoff é DELEGAÇÃO; em manual o agente só PROPÕE ao usuário. O router
    // segue como backstop (defesa em profundidade), mas a recusa nasce aqui, visível.
    if input.autonomy == Autonomy::Manual {
        eprintln!(
            "lina: handoff bloqueado — a autonomia do workspace esta em MANUAL e delegar e \
             acao de delegacao. PROPONHA a tarefa ao usuario em portugues simples e execute \
             so depois do sim dele (ou peca para mudar a autonomia)."
        );
        return ExitCode::from(1);
    }
    let from = input.terminal_name;

    // --context: ANEXA o conteúdo (falha VISÍVEL se ilegível — nunca enfileirar um handoff
    // prometendo um contexto que não foi anexado; fidelidade > contorno).
    let mut payload = task;
    let mut input_schema =
        String::from("tarefa em texto no payload; responda no formato do output_schema");
    if let Some(path) = context {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                payload.push_str(&format!("\n\n--- contexto anexado ({path}) ---\n{content}"));
                input_schema =
                    format!("tarefa em texto + contexto anexado ({path}) no proprio payload");
            }
            Err(e) => {
                eprintln!("lina: nao consegui ler o --context {path}: {e} — handoff NAO enviado");
                return ExitCode::from(1);
            }
        }
    }

    let mut constraints = std::collections::BTreeMap::new();
    constraints.insert("autonomy".to_string(), input.autonomy.label().to_string());
    let contract = HandoffContract {
        input_schema,
        output_schema:
            "resultado da tarefa no formato pedido em [EXPECTED]; termine com PRONTO: <resumo> \
             ou BLOCKED: <motivo>"
                .to_string(),
        error_codes: vec!["E_TIMEOUT".to_string(), "E_BLOCKED".to_string()],
        timeout_sec,
        retry_policy: "manual".to_string(),
        constraints_metadata: constraints,
    };

    let mut msg = MailMessage::new_v2(from.clone(), to, "handoff", payload).with_contract(contract);
    if await_reply {
        msg = msg.awaiting();
    }
    if let Some(r) = ref_id {
        msg.ref_id = Some(r);
    }
    enqueue_and_report(&from, msg)
}

/// **F1-0-6 — `lina check @<alvo>`**: estado VIVO do colega pela PROJEÇÃO do lifecycle
/// (F1-0-3/ADR 0019 §5: o veredito vem do log, nunca de view cacheada) + última
/// atividade A2A. **Leitura PURA** de `agents.json` + `log.jsonl` — não injeta NADA no
/// terminal do colega (espiar ≠ interromper; é o anti-"cutucar pra ver se está vivo").
fn run_check(args: &[String]) -> ExitCode {
    let Some(target_raw) = args.first() else {
        usage();
        return ExitCode::from(2);
    };
    let target = target_raw.trim_start_matches('@');

    // Roster (papel + status do app) — mesmo leitor do `lina list`.
    let mailbox = Mailbox::new(mailbox_root());
    let roster = mailbox.read_agents().unwrap_or_default();
    let roster_entry = roster
        .iter()
        .find(|a| a.name == target || a.name.trim_start_matches('@') == target);

    // Projeção do lifecycle + última atividade, varrendo o espelho `log.jsonl` em ordem
    // (tolerante a linha parcial — arquivo sob append, mesma postura do poll de `ask`).
    let mut node_id: Option<String> = None;
    let mut state: Option<(String, String)> = None; // (status, reason)
    let mut stalled = false;
    let mut last_a2a: Option<(String, String, String, u64)> = None; // intent, from, to, ts
    if let Ok(content) = std::fs::read_to_string(event_log_path()) {
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let p = &v["payload"];
            match v["kind"].as_str().unwrap_or_default() {
                "NodeRenamed" if p["name"].as_str() == Some(target) => {
                    node_id = p["node"].as_str().map(str::to_string);
                }
                "NodeStatusChanged"
                    if node_id.is_some() && node_id.as_deref() == p["node"].as_str() =>
                {
                    state = Some((
                        p["status"].as_str().unwrap_or("?").to_string(),
                        p["reason"].as_str().unwrap_or("").to_string(),
                    ));
                    stalled = false; // transição limpa o WARN (regra da projeção F1-0-3)
                }
                "NodeStalled" if node_id.is_some() && node_id.as_deref() == p["node"].as_str() => {
                    stalled = true;
                }
                "MessageRouted" => {
                    let from = p["from"].as_str().unwrap_or_default();
                    let to = p["to"].as_str().unwrap_or_default();
                    let to_node = p["to_node"].as_str().unwrap_or_default();
                    let touches = from == target
                        || to.trim_start_matches('@') == target
                        || (!to_node.is_empty() && node_id.as_deref() == Some(to_node));
                    if touches {
                        last_a2a = Some((
                            p["intent"].as_str().unwrap_or("?").to_string(),
                            from.to_string(),
                            to.to_string(),
                            v["ts"].as_u64().unwrap_or(0),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    if state.is_none() && roster_entry.is_none() {
        eprintln!(
            "lina: nao encontrei '{target_raw}' nem no lifecycle do log nem no roster — \
             confira quem esta no Espaco com `lina list`."
        );
        return ExitCode::from(1);
    }

    match (&state, roster_entry) {
        (Some((status, reason)), entry) => {
            let stall_txt = if stalled {
                " · TRAVADO (Busy sem progresso — ADR 0019; veja se precisa de ajuda ou re-direcao)"
            } else {
                ""
            };
            println!("@{target} — estado: {status} (motivo: {reason}){stall_txt}");
            if let Some(a) = entry {
                println!("papel: {}", a.role.as_deref().unwrap_or("—"));
            }
        }
        (None, Some(a)) => {
            println!(
                "@{target} — estado: {} (do roster do app; sem lifecycle deste no no log)",
                a.status
            );
            println!("papel: {}", a.role.as_deref().unwrap_or("—"));
        }
        (None, None) => unreachable!("guard acima"),
    }
    match last_a2a {
        Some((intent, from, to, ts)) => {
            println!("ultima atividade A2A: {intent} de {from} para {to} (ts {ts})");
        }
        None => println!("ultima atividade A2A: nenhuma registrada no log"),
    }
    ExitCode::SUCCESS
}

/// Desfecho REAL do roteamento de uma `lina ask`, lido do espelho `log.jsonl`.
#[derive(Debug, PartialEq, Eq)]
enum RouteConfirm {
    /// O Espaço entregou/roteou a mensagem ao destino.
    Delivered { to_node: String },
    /// O roteador bloqueou a mensagem (`reason` = `unknown_sender`/`no_target`/`hop_limit`/…).
    Blocked { reason: String },
    /// Sem evento de desfecho no tempo de espera (o app pode estar ocupado/lento).
    Pending,
}

/// Caminho do espelho append-only do event log (`<LINA_HOME>/events/log.jsonl`). Lemos ESTE (não o
/// SQLite) p/ não abrir uma conexão concorrente ao banco do app (evita lock na troca de WAL).
fn event_log_path() -> PathBuf {
    mailbox_root().join("events").join("log.jsonl")
}

/// **PURO** (testável, sem I/O/timing): varre o conteúdo do `log.jsonl` pelo desfecho de `msg_id`.
/// `Delivered` vence `Blocked` (se a msg foi entregue em alguma tentativa, chegou); `None` se ainda não
/// há desfecho. Tolera linhas parciais/inválidas (arquivo sob append).
fn scan_log_outcome(content: &str, msg_id: &str) -> Option<RouteConfirm> {
    let mut last_block: Option<String> = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let p = &v["payload"];
        if p.get("id").and_then(serde_json::Value::as_str) != Some(msg_id) {
            continue;
        }
        match v.get("kind").and_then(serde_json::Value::as_str) {
            Some("MessageDelivered" | "MessageRouted") => {
                let to = p
                    .get("to_node")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| p.get("to").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                return Some(RouteConfirm::Delivered { to_node: to }); // sucesso vence bloqueio
            }
            Some("RouteBlocked") => {
                last_block = p
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
            }
            _ => {}
        }
    }
    last_block.map(|reason| RouteConfirm::Blocked { reason })
}

/// Aguarda (poll bounded ~3s) o desfecho do roteamento de `msg_id` no `log.jsonl`. Retorna `Delivered`
/// assim que a msg é roteada/entregue; no fim do prazo, `Blocked` (se houve) ou `Pending`. Tolerante a
/// log ausente. A lógica de parse vive em [`scan_log_outcome`] (pura/testada).
fn poll_route_outcome(msg_id: &str) -> RouteConfirm {
    use std::time::{Duration, Instant};
    let path = event_log_path();
    let deadline = Instant::now() + Duration::from_millis(3000);
    loop {
        let outcome = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| scan_log_outcome(&c, msg_id));
        match outcome {
            // Entregue → conclui já. Bloqueado → só conclui no prazo (pode ser re-tentado e entregar).
            Some(o @ RouteConfirm::Delivered { .. }) => return o,
            Some(o @ RouteConfirm::Blocked { .. }) if Instant::now() >= deadline => return o,
            _ if Instant::now() >= deadline => return RouteConfirm::Pending,
            _ => std::thread::sleep(Duration::from_millis(150)),
        }
    }
}

/// Tradução acionável do motivo de bloqueio (o leitor é um agente de IA — texto claro, sem jargão de log).
fn explain_block(reason: &str) -> String {
    match reason {
        "unknown_sender" => {
            "o Espaco nao reconheceu voce como remetente vivo (seu terminal pode nao \
             estar no roster ainda, ou houve uma corrida de registro)"
                .to_string()
        }
        "no_target" => "o destino nao foi encontrado entre os agentes vivos do Espaco".to_string(),
        "hop_limit" => "o limite de encaminhamento (anti-loop) foi atingido".to_string(),
        "self_message" => "voce tentou enviar para si mesmo".to_string(),
        other => format!("motivo tecnico: {other}"),
    }
}

/// Dica de recuperação por motivo. Para `unknown_sender`/`no_target` instrui EXPLICITAMENTE a NÃO
/// concluir que o colega é um "stub" (foi o que enganou o agente: o roteamento falhou, o colega vive).
fn block_hint(reason: &str) -> &'static str {
    match reason {
        "unknown_sender" | "no_target" => {
            "→ Tente de novo em alguns segundos (o roster pode estar sincronizando) e confira os nomes \
             vivos com `lina list`. NAO conclua que o colega e um \"stub\"/mudo: ele pode estar vivo — \
             foi o ROTEAMENTO que falhou, nao o colega."
        }
        _ => "→ Veja o estado do Espaco com `lina list`.",
    }
}

/// **F1-3-6 — `lina spawn @<Nome> --role <papel> [--prompt "<1º prompt>"]`**: o agente PEDE criar um
/// terminal novo quando percebe que falta um papel (capacidade SENSÍVEL = VERBO estruturado, doutrina
/// InsForge — nunca um contorno). `from` é autenticado pelo dir-dono do outbox (não por flag). O
/// router DECIDE pelo gate INFORJÁVEL (`handle_spawn`): ORIGEM permitida; CASCATA/cap/custo pedem aval
/// humano; `manual` recusa. A criação física (PTY + register + bootstrap) é do app (seam da tela).
///
/// **Manual local (doutrina bloco 5):** em `autonomy=manual` recusa AQUI, antes de enfileirar (UX
/// imediata); o router é o backstop durável. O nível vem do `bootstrap.json` (`load_input`).
fn run_spawn(args: &[String]) -> ExitCode {
    let mut role: Option<String> = None;
    let mut prompt = String::new();
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--role" => {
                i += 1;
                match args.get(i) {
                    Some(v) => role = Some(v.clone()),
                    None => {
                        eprintln!("lina: --role exige um valor (papel do novo terminal)");
                        return ExitCode::from(2);
                    }
                }
            }
            "--prompt" => {
                i += 1;
                match args.get(i) {
                    Some(v) => prompt = v.clone(),
                    None => {
                        eprintln!("lina: --prompt exige o texto do 1o prompt");
                        return ExitCode::from(2);
                    }
                }
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    let (Some(raw_name), Some(role)) = (positional.into_iter().next(), role) else {
        eprintln!("lina: uso: lina spawn @<Nome> --role <papel> [--prompt \"<1o prompt>\"]");
        return ExitCode::from(2);
    };
    // Normaliza o `@` (aceita "QA" e "@QA") — o nome do nó no roster inclui o `@`.
    let name = if raw_name.starts_with('@') {
        raw_name
    } else {
        format!("@{raw_name}")
    };

    let input = match load_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!(
                "lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'/autonomia): {e}"
            );
            return ExitCode::from(1);
        }
    };
    let from = input.terminal_name.clone();

    // Manual local: recusa antes de enfileirar (o router é o backstop durável — ver gate).
    if matches!(input.autonomy, Autonomy::Manual) {
        eprintln!(
            "lina: no modo MANUAL voce NAO cria terminais sozinho — sugira a criacao de {name} \
             ({role}) ao usuario/Maestro (ou peca para subir a autonomia)."
        );
        return ExitCode::from(1);
    }

    // `role` viaja no campo `ref` do envelope (`role:<papel>`) — sem inventar campo novo; o `name`
    // é o `to`, o `prompt` é o payload. O router (handle_spawn) lê os três.
    let msg = MailMessage::new(from.clone(), name.clone(), "spawn", prompt)
        .with_ref(format!("role:{role}"));
    enqueue_and_report_spawn(&from, &name, &role, msg)
}

/// Desfecho de um `lina spawn`, lido do espelho `log.jsonl` (eventos do gate, não a PTY).
#[derive(Debug, PartialEq, Eq)]
enum SpawnConfirm {
    /// Gate APROVOU (origem, sob cap, custo ok): `SpawnRequested` presente SEM `SpawnGated`. O app
    /// cria o terminal (seam da tela).
    Approved,
    /// Gate barrou/adiou: `SpawnGated{reason}` (`cascade`/`over_cap`/`cost`/`manual`).
    Gated { reason: String },
    /// Sem desfecho no log no tempo de espera.
    Pending,
}

/// **PURO** (testável, sem I/O): varre o `log.jsonl` pelo desfecho do spawn `msg_id`. `SpawnGated`
/// vence (decisão definitiva do gate); senão `SpawnRequested` ⇒ aprovado; senão `Pending`. O gate
/// loga `SpawnRequested` SEMPRE e `SpawnGated` SÓ quando barra — então "requested sem gated" = aprovado.
fn scan_spawn_outcome(content: &str, msg_id: &str) -> SpawnConfirm {
    let mut requested = false;
    let mut gated: Option<String> = None;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let p = &v["payload"];
        if p.get("id").and_then(serde_json::Value::as_str) != Some(msg_id) {
            continue;
        }
        match v.get("kind").and_then(serde_json::Value::as_str) {
            Some("SpawnGated") => {
                gated = p
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
            }
            Some("SpawnRequested") => requested = true,
            _ => {}
        }
    }
    match gated {
        Some(reason) => SpawnConfirm::Gated { reason },
        None if requested => SpawnConfirm::Approved,
        None => SpawnConfirm::Pending,
    }
}

/// Aguarda (poll bounded ~3s) o desfecho do spawn no `log.jsonl`. `Gated` é definitivo (retorna já);
/// `Approved` só conclui no prazo (evita ler entre o append de `SpawnRequested` e o de `SpawnGated`,
/// que são sequenciais no MESMO roteamento). Lógica de parse em [`scan_spawn_outcome`] (pura/testada).
fn poll_spawn_outcome(msg_id: &str) -> SpawnConfirm {
    use std::time::{Duration, Instant};
    let path = event_log_path();
    let deadline = Instant::now() + Duration::from_millis(3000);
    loop {
        let outcome = std::fs::read_to_string(&path)
            .ok()
            .map(|c| scan_spawn_outcome(&c, msg_id))
            .unwrap_or(SpawnConfirm::Pending);
        match outcome {
            SpawnConfirm::Gated { .. } => return outcome, // decisão definitiva do gate
            SpawnConfirm::Approved if Instant::now() >= deadline => return outcome,
            _ if Instant::now() >= deadline => return SpawnConfirm::Pending,
            _ => std::thread::sleep(Duration::from_millis(150)),
        }
    }
}

/// Enfileira o pedido de spawn (`from` autenticado por dir-dono) e reporta o desfecho REAL do gate.
fn enqueue_and_report_spawn(from: &str, name: &str, role: &str, msg: MailMessage) -> ExitCode {
    let mailbox = Mailbox::new(mailbox_root());
    if let Err(e) = enqueue_per_node(&mailbox, from, &msg) {
        eprintln!("lina: falha ao enfileirar o pedido de spawn na mailbox: {e}");
        return ExitCode::from(1);
    }
    match poll_spawn_outcome(&msg.id) {
        SpawnConfirm::Approved => {
            println!(
                "ok: pedido de criar {name} ({role}) APROVADO e enviado ao Espaco (id {}). O \
                 canvas traz o especialista; o 1o prompt ja vai na fila dele.",
                msg.id
            );
            ExitCode::SUCCESS
        }
        SpawnConfirm::Gated { reason } => {
            eprintln!(
                "lina: a criacao de {name} ({role}) NAO foi automatica — {}.\n{}",
                explain_spawn_gate(&reason),
                spawn_gate_hint(&reason)
            );
            // `manual` é recusa terminal (exit 1); os demais são GATE humano legítimo (pedido válido,
            // aguardando aval) — exit 0 (não é erro do agente).
            if reason == "manual" {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        SpawnConfirm::Pending => {
            println!(
                "ok: pedido de criar {name} ({role}) enviado (id {}); ainda SEM desfecho apos a \
                 espera (o Espaco pode estar ocupado). Confirme com `lina list`.",
                msg.id
            );
            ExitCode::SUCCESS
        }
    }
}

/// Tradução acionável do motivo do gate de spawn (o leitor é um agente — texto claro, sem jargão).
fn explain_spawn_gate(reason: &str) -> String {
    match reason {
        "cascade" => "voce recebeu uma tarefa de outro agente e quer criar um terminal; \
             criar-a-partir-de-uma-cadeia exige o aval do usuario (defesa anti-fork-bomb)"
            .to_string(),
        "over_cap" => {
            "o limite de criacoes por turno foi atingido; a proxima precisa do aval do usuario"
                .to_string()
        }
        "cost" => "o teto de custo do Espaco foi atingido; retome o custo antes de criar mais \
             terminais"
            .to_string(),
        "manual" => "o Espaco esta em modo manual — voce nao cria terminais sozinho".to_string(),
        other => format!("motivo: {other}"),
    }
}

/// Dica de recuperação por motivo do gate de spawn.
fn spawn_gate_hint(reason: &str) -> &'static str {
    match reason {
        "cost" => "→ peca ao usuario para retomar o custo (`lina resume --confirm`); nao insista.",
        "manual" => "→ sugira a criacao ao usuario/Maestro (ou peca para subir a autonomia).",
        _ => "→ explique ao usuario POR QUE falta o papel e aguarde o aval; nao recrie em loop.",
    }
}

/// `lina broadcast "*" "<msg>"` (ou `--role PAPEL`) — açúcar de `lina ask` para o **fan-out**: avisa
/// TODOS os nós vivos (alvo `*`) ou um papel inteiro. É um adaptador FINO sobre [`run_ask`] (reusa
/// parsing, autenticação de origem por dir-dono e escrita no outbox); só carimba `intent=broadcast`
/// quando não há `--intent` explícito. O destino `*` precede com naturalidade o gate de fan-out do
/// router (ADR 0007): a 1ª onda de ORIGEM (o agente pedido pelo humano, `hops==0`) entrega a todos SEM
/// confirmação; a CASCATA (`hops>=1`, re-espalhar) segue gateada. O alvo NÃO é default-implícito: o
/// chamador passa `"*"` (ou `--role`) — sem alvo, recai no `usage()` do `run_ask` (sem ambiguidade).
fn run_broadcast(args: &[String]) -> ExitCode {
    let mut forwarded: Vec<String> = args.to_vec();
    if !args.iter().any(|a| a == "--intent") {
        forwarded.push("--intent".to_string());
        forwarded.push("broadcast".to_string());
    }
    run_ask(&forwarded)
}

/// `lina plan ...` (W3-5) — `read` (read-only, lê o `.lina/plan.md`) | `claim`/`check <id>`
/// (depositam um intent na mailbox; o supervisor, escritor único, aplica ao plano e loga).
fn run_plan(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("read") => run_plan_read(),
        Some("claim") => run_plan_intent("plan.claim", args.get(1)),
        Some("check") => run_plan_intent("plan.check", args.get(1)),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

/// `lina plan read` — imprime o `.lina/plan.md` cru (qualquer processo pode ler; só o supervisor escreve).
fn run_plan_read() -> ExitCode {
    let mailbox = Mailbox::new(mailbox_root());
    match mailbox.read_plan() {
        Ok(Some(text)) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("(sem plano ainda — nenhum item no workspace)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao ler o plano: {e}");
            ExitCode::from(1)
        }
    }
}

/// `lina plan claim|check <id>` — monta o envelope de plano (intent=`plan.claim`/`plan.check`,
/// `ref=plan:<id>`, `from`=este terminal) e o deposita na mailbox. O bin é processo SEPARADO e NÃO
/// escreve no `plan.md` — quem aplica é o supervisor.
fn run_plan_intent(intent: &str, id: Option<&String>) -> ExitCode {
    let Some(id) = id else {
        eprintln!("lina: '{intent}' exige o id do item (ex.: lina plan claim T1)");
        usage();
        return ExitCode::from(2);
    };
    let from = match load_input() {
        Ok(i) => i.terminal_name,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'): {e}");
            return ExitCode::from(1);
        }
    };
    // Alvo sentinela "plan": o supervisor intercepta por INTENT, não por alvo. `ref=plan:<id>` liga
    // ao item do plano (envelope §3.4). payload vazio.
    let msg = MailMessage::new(from.clone(), "plan", intent, "").with_ref(format!("plan:{id}"));
    let mailbox = Mailbox::new(mailbox_root());
    // Round 6 (F1): plano via outbox POR-NÓ (como `lina ask`) — o supervisor carimba `from`=origem
    // (dir-dono) e o guard de origem do router valida o remetente; fecha a forja do `from` que o outbox
    // FLAT (não-autenticado) permitia (claim/check impersonando um colega no plano compartilhado).
    match enqueue_per_node(&mailbox, &from, &msg) {
        Ok(()) => {
            println!("ok: {intent} do item {id} enfileirado (msg {})", msg.id);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
    }
}

/// Diretório do event store do workspace (`<.lina>/events/`). O gate apenda o `ActionGated`
/// aqui — mesmo log que o supervisor projeta (invariante #4: log = fonte da verdade).
fn events_dir() -> PathBuf {
    mailbox_root().join("events")
}

/// Wall-clock em millis para o `now_ms` de produção do `lina retro` (`--now-ms` o sobrescreve).
/// O wall-clock vive SÓ aqui (na casca); a projeção (`retro.rs`) é pura sobre `now_ms` injetado
/// (lição `feature-filtra-ts-wallclock`). Espelha `events::now_millis` (privado ao core).
fn retro_now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// **F1-3-7 — `lina retro [--json] [--now-ms <ms>]`**: auto-aprimoramento v0. Lê o event log
/// (SÓ-LEITURA via `EventStore::events()`) e emite um RELATÓRIO de projeções determinísticas
/// (skills/coordenação/custos/pedidos/lacunas), ZERO LLM — quem pensa é o agente (inv#1).
///
/// **Gate inviolável (critério 4):** [`classify_retro_args`] é a porta única; este verbo NÃO tem
/// subcomando de mutação — qualquer `archive`/`apply`/`pin`/… é recusado ANTES de tocar o log. Não
/// há, neste verbo, nenhum `append`: toda manutenção (arquivar/fixar skill, papel, preset) passa
/// por gate humano, fora daqui. O agente lê o relatório (skill `lina-retro`) e PROPÕE; o humano decide.
fn run_retro(args: &[String]) -> ExitCode {
    match classify_retro_args(args) {
        RetroInvocation::Refused {
            offending,
            mutation,
        } => {
            if mutation {
                eprintln!(
                    "lina retro SO observa e sugere -- '{offending}' tentaria APLICAR uma mudanca, e \
                     isso NAO existe aqui (nao ha `lina retro apply`). Arquivar/fixar skill, mudar papel \
                     ou preset passa SEMPRE por GATE HUMANO. Rode `lina retro` (sem argumentos) para ver \
                     o relatorio e PROPONHA ao humano citando a evidencia (numero/evento)."
                );
            } else {
                eprintln!(
                    "lina retro: argumento desconhecido '{offending}'. Uso: lina retro [--json] [--now-ms <ms>]"
                );
            }
            ExitCode::from(2)
        }
        RetroInvocation::Report { json, now_ms } => {
            let now = now_ms.unwrap_or_else(retro_now_ms);
            // SÓ-LEITURA: lê o espelho `log.jsonl` (mesma escolha de `check`/`spawn` — NUNCA abre o
            // SQLite do app, p/ não disputar lock na troca de WAL). Sem log ⇒ string vazia ⇒
            // first-run honesto. Zero `append`: nenhuma escrita parte deste verbo.
            let content = std::fs::read_to_string(event_log_path()).unwrap_or_default();
            let records = lina_bootstrap::parse_log_records(&content);
            let report = project_retro(&records, now);
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("lina retro: falha ao serializar o relatorio: {e}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                print!("{}", render_report(&report));
            }
            ExitCode::SUCCESS
        }
    }
}

/// `lina guard --check-action --cmd "<comando>" --autonomy <manual|assistido|autonomo>` (W3-6,
/// AC-6.2): classifica o comando (pattern-match determinístico, ZERO LLM), aplica a matriz
/// nível×classe e imprime a decisão (`allow`/`ask`/`deny`). Quando a decisão NÃO é `allow`, apenda
/// `ActionGated{cmd, class, decision}` ao event log. `routine` (allow) não toca o log.
fn run_guard(args: &[String]) -> ExitCode {
    // Modo hook `PreToolUse` (Claude Code, tier 1): lê o JSON do stdin e emite a decisão do gate
    // em JSON no stdout. É um caminho SEPARADO do verbo `--check-action` (W3-6a).
    if args.iter().any(|a| a == "--pretooluse") {
        return run_pretooluse();
    }

    let mut check_action_flag = false;
    let mut cmd: Option<String> = None;
    let mut autonomy: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check-action" => check_action_flag = true,
            "--cmd" => {
                i += 1;
                match args.get(i) {
                    Some(v) => cmd = Some(v.clone()),
                    None => {
                        eprintln!("lina: --cmd exige o comando a checar");
                        return ExitCode::from(2);
                    }
                }
            }
            "--autonomy" => {
                i += 1;
                match args.get(i) {
                    Some(v) => autonomy = Some(v.clone()),
                    None => {
                        eprintln!("lina: --autonomy exige um valor (manual|assistido|autonomo)");
                        return ExitCode::from(2);
                    }
                }
            }
            other => {
                eprintln!("lina: argumento desconhecido para guard: {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    if !check_action_flag {
        eprintln!("lina: guard exige --check-action");
        usage();
        return ExitCode::from(2);
    }
    let (Some(cmd), Some(autonomy)) = (cmd, autonomy) else {
        eprintln!("lina: guard exige --cmd \"<comando>\" e --autonomy <nivel>");
        usage();
        return ExitCode::from(2);
    };
    let level = match parse_autonomy(&autonomy) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(2);
        }
    };

    let verdict = check_action(&cmd, level);
    // Sempre imprime a decisão (o hook/shim lê esta linha).
    println!("{}", verdict.decision.as_str());

    // Livro-razão das recusas: só apenda quando NÃO é allow (ação routine não polui o log).
    if verdict.decision != lina_core::Decision::Allow {
        let mut store = match EventStore::open(events_dir()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("lina: falha ao abrir o event store: {e}");
                return ExitCode::from(1);
            }
        };
        let event = DomainEvent::ActionGated {
            cmd,
            class: verdict.class.as_str().to_string(),
            decision: verdict.decision.as_str().to_string(),
        };
        if let Err(e) = store.append(&event) {
            eprintln!("lina: falha ao apendar ActionGated: {e}");
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}

/// `lina guard --pretooluse` (W3-6, AC-6.3) — **hook `PreToolUse` do Claude Code (tier 1)**.
/// Lê no stdin o JSON do `PreToolUse`, extrai o comando (`tool_input.command` p/ Bash), reusa o
/// núcleo determinístico do gate (ZERO LLM) e emite no stdout APENAS o JSON
/// `{"hookSpecificOutput":{...}}`. A autonomia vem de `LINA_AUTONOMY` (default `assistido`).
///
/// **Robustez (regra do `SessionStart`, W3-2):** NUNCA imprime texto cru no stdout. Em qualquer
/// falha (stdin ilegível, JSON inválido) emite um JSON fail-safe `ask` e loga em stderr. Sempre
/// sai com `SUCCESS` — o gate fala pelo conteúdo do JSON, não pelo exit code (o harness lê o JSON).
fn run_pretooluse() -> ExitCode {
    let mut raw = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        // stdin ilegível → fail-safe `ask` (decisão volta ao humano), diagnóstico em stderr.
        eprintln!("lina: falha ao ler stdin do PreToolUse: {e}");
        println!("{}", pretooluse_output("", &autonomy_from_env()));
        return ExitCode::SUCCESS;
    }
    println!("{}", pretooluse_output(&raw, &autonomy_from_env()));
    ExitCode::SUCCESS
}

/// `lina resume` (W3-7c · ROUND 5 hole 1) — **pedido de retomada do teto; o AGENTE NÃO des-pausa.**
///
/// O furo anterior: o agente rodava `lina resume --confirm` e apendava `CostCeilingResumed` DIRETO no
/// store — anulava o teto como gate (auto-retomada). FIX (igual à custódia, ADR 0004): o agente só
/// **REGISTRA** um intent `resume.request` na FILA DE BROKER por-nó (origem autenticada); o SUPERVISOR
/// no app aplica `CostCeilingResumed` **SÓ após confirmação no canal humano** (⌘⏎ na janela). Este bin
/// **nunca** apenda `CostCeilingResumed`. `--confirm` é aceito por retrocompat, mas é no-op (a
/// confirmação real é a tecla na janela — inforjável pelo PTY do agente).
fn run_resume(_args: &[String]) -> ExitCode {
    // Origem autenticada pela fila por-nó (não o campo `from`): o requester vem do dir-dono no drain.
    let from = load_input()
        .map(|i| i.terminal_name)
        .unwrap_or_else(|_| "agente-desconhecido".to_string());

    let msg = MailMessage::new(&from, "broker", "resume.request", "").with_ref("resume:ceiling");
    let mailbox = Mailbox::new(broker_mailbox_root());
    // ESTRITO (sem fallback flat — hole 3): pedido privilegiado só entra por origem autenticada.
    match mailbox.enqueue_as(&from, &msg) {
        Ok(()) => {
            println!(
                "pedido de retomada do teto registrado (msg {}). O agente NAO des-pausa: requer",
                msg.id
            );
            println!("confirmacao HUMANA na janela do Lina (Cmd+Enter). So entao o supervisor retoma o teto.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "lina: falha ao registrar o pedido de retomada (origem '{from}' precisa ser um no valido): {e}"
            );
            ExitCode::from(1)
        }
    }
}

/// `lina do <action> [args]` (W3-6c, ADR 0004) — **verbo brokerado de ação custodiada** do lado
/// do AGENTE. O agente NÃO executa a ação: ele não tem o segredo (deploy key/API key/token — vive
/// só no SecretVault do Lina) nem caminho de confirmação humana. Este verbo:
///   1. valida que `<action>` é custodiada (registry do broker);
///   2. apenda `ActionGated{class:"gated-hard-external", decision:"ask"}` (o gate disparou — piso);
///   3. apenda `BrokerDenied{reason:"unconfirmed"}` (sem gate humano, a tentativa do agente é
///      bloqueada — prova de custódia: o agente, sozinho, NUNCA executa);
///   4. registra o pedido na mailbox (intent `broker.do`) para o supervisor rodar o gate humano +
///      `run_custody` (que obtém o segredo do cofre e executa — fora deste binário).
///
/// A ação real NÃO roda aqui (este crate nem linka `lina-secrets`: o agente não tem acesso ao cofre).
fn run_do(args: &[String]) -> ExitCode {
    let Some(action) = args.first() else {
        eprintln!("lina: 'do' exige uma acao custodiada (ex.: lina do deploy --env prod)");
        usage();
        return ExitCode::from(2);
    };
    let Some(custody) = lookup_action(action) else {
        eprintln!("lina: acao '{action}' nao e custodiada. Acoes suportadas: deploy | pay | send");
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    let display = if rest.is_empty() {
        format!("lina do {action}")
    } else {
        format!("lina do {action} {}", rest.join(" "))
    };

    // Identidade do requisitante: auto-declarada (A3 pendente — autoria NÃO-autenticada, ADR 0004 §4).
    let requester = load_input()
        .map(|i| i.terminal_name)
        .unwrap_or_else(|_| "agente-desconhecido".to_string());

    let mut store = match EventStore::open(events_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lina: falha ao abrir o event store: {e}");
            return ExitCode::from(1);
        }
    };

    // (2) O gate disparou: ação externa custodiada → ask em qualquer nível (piso de custódia).
    let gated = DomainEvent::ActionGated {
        cmd: display.clone(),
        class: CLASS_GATED_HARD_EXTERNAL.to_string(),
        decision: "ask".to_string(),
    };
    if let Err(e) = store.append(&gated) {
        eprintln!("lina: falha ao apendar ActionGated: {e}");
        return ExitCode::from(1);
    }
    // (3) O agente não tem confirmação humana → a tentativa é bloqueada (custódia).
    let denied = DomainEvent::BrokerDenied {
        action: action.clone(),
        reason: "unconfirmed".to_string(),
    };
    if let Err(e) = store.append(&denied) {
        eprintln!("lina: falha ao apendar BrokerDenied: {e}");
        return ExitCode::from(1);
    }

    // (4) Registra o pedido na FILA DE BROKER dedicada (`<LINA_HOME>/broker/`), NÃO no outbox A2A: é
    //     uma mensagem de controle para o supervisor (gate humano + `run_custody` com o segredo do
    //     cofre), não uma entrega a um colega. **`enqueue_as` ESTRITO (sem fallback flat — hole 3):** a
    //     origem (dir-dono) autentica o `requester`; um fallback flat reabriria a forja do campo `from`
    //     que o humano lê no gate. Nome de nó inválido → erro duro (o pedido privilegiado NÃO entra por
    //     canal forjável).
    let msg = MailMessage::new(&requester, "broker", "broker.do", rest.join(" "))
        .with_ref(format!("do:{action}"));
    let mailbox = Mailbox::new(broker_mailbox_root());
    if let Err(e) = mailbox.enqueue_as(&requester, &msg) {
        eprintln!(
            "lina: falha ao registrar o pedido custodiado na fila de broker (origem '{requester}' precisa ser um no valido): {e}"
        );
        return ExitCode::from(1);
    }

    println!(
        "gated: acao custodiada '{action}' ({}) requer confirmacao humana.",
        custody.desc
    );
    println!("registrado para o supervisor (msg {}). O agente NAO executa: o segredo vive so no cofre do Lina.", msg.id);
    ExitCode::SUCCESS
}

/// `lina handshake` — registra presença (0 broadcast) e lista os colegas do workspace.
fn run_handshake() -> ExitCode {
    let input = match load_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {INPUT_PATH}: {e}");
            return ExitCode::from(1);
        }
    };
    let from = input.terminal_name.clone();
    let mailbox = Mailbox::new(mailbox_root());

    // Ping de presença na mailbox: intent=handshake, to=* — o supervisor REGISTRA a entrada
    // (`agent.joined`) mas NÃO injeta em ninguém (0 broadcast no boot, design §2/§4).
    let ping = MailMessage::new(
        &from,
        "*",
        "handshake",
        format!("{from} entrou no workspace"),
    );
    if let Err(e) = mailbox.enqueue(&ping) {
        eprintln!("lina: falha ao registrar presenca: {e}");
        return ExitCode::from(1);
    }

    // Colegas: do agents.json (escrito pelo supervisor) se houver; senão do roster do bootstrap.
    let colegas: Vec<String> = match mailbox.read_agents() {
        Ok(agents) if !agents.is_empty() => agents
            .into_iter()
            .filter(|a| a.name != from)
            .map(|a| format!("{} ({})", a.name, a.role.unwrap_or_else(|| "—".into())))
            .collect(),
        _ => input
            .roster
            .iter()
            .filter(|n| **n != from)
            .cloned()
            .collect(),
    };

    println!("=== Lina Space HANDSHAKE ===");
    println!("Voce: {from}");
    if colegas.is_empty() {
        println!("COLEGAS: (nenhum colega no workspace ainda)");
    } else {
        println!("COLEGAS: {}", colegas.join("; "));
    }
    println!("(NAO responda — handshake e informativo, nao interrogativo)");
    println!("=== FIM HANDSHAKE ===");
    ExitCode::SUCCESS
}

/// `lina list [--json]` (W4-2) — lista os agentes do workspace (NOME · PAPEL · STATUS), lido do
/// `agents.json` que o supervisor (app) escreve a cada mudança de roster. `--json` emite JSON — é o
/// que VERIFICA o M2: o agente criado pelo nome aparece aqui com o PAPEL derivado (ex.: reviewer).
/// Read-only (qualquer processo pode); roster vazio → lista vazia, nunca erro.
fn run_list(json: bool) -> ExitCode {
    let mailbox = Mailbox::new(mailbox_root());
    let agents = match mailbox.read_agents() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lina: falha ao ler o roster (agents.json): {e}");
            return ExitCode::from(1);
        }
    };
    if json {
        match serde_json::to_string_pretty(&agents) {
            Ok(s) => {
                println!("{s}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("lina: falha ao serializar o roster: {e}");
                ExitCode::from(1)
            }
        }
    } else {
        if agents.is_empty() {
            println!("(nenhum agente no workspace ainda)");
        } else {
            for a in &agents {
                println!(
                    "{} · {} · {}",
                    a.name,
                    a.role.as_deref().unwrap_or("—"),
                    a.status
                );
            }
        }
        ExitCode::SUCCESS
    }
}

// ════════════════════════ `lina vault` — acesso ao "segundo cérebro" (Obsidian) ════════════════════════
//
// A doutrina (BLOCO 3) promete ao agente os verbos `lina vault path|read|search` para acionar os vaults
// linkados no onboarding SEM explorar o filesystem na mão — mas o comando não existia (o agente batia em
// "vault não implementado" e caía pro `cat`/`grep` direto, frágil). Aqui implementamos sobre o contrato
// JÁ escrito pelo app: `<LINA_HOME>/vault.json` (vaults linkados) + `<LINA_HOME>/vault-index/*.md` (o
// mapa estrutural PageIndex, determinístico). O bin NÃO importa o app: parseia o JSON simples localmente.

/// Um vault linkado, lido de `vault.json` (formato escrito por `obsidian.rs::write_vault_config`).
#[derive(serde::Deserialize)]
struct VaultLinkJson {
    #[serde(default)]
    name: String,
    path: String,
}

/// Conteúdo de `<LINA_HOME>/vault.json`. Campos extras (ex.: `writable`) são ignorados.
#[derive(serde::Deserialize)]
struct VaultConfigJson {
    #[serde(default)]
    primary: String,
    #[serde(default)]
    vaults: Vec<VaultLinkJson>,
}

/// Lê os vaults linkados de `<LINA_HOME>/vault.json`. `Err` acionável se não houver vault linkado (o
/// agente é orientado a pedir ao usuário que rode o passo "Segundo cérebro" do onboarding).
fn load_vault_config() -> Result<VaultConfigJson, String> {
    let path = mailbox_root().join("vault.json");
    let data = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "nenhum vault Obsidian linkado ainda ({}): {e}\n\
             → peça ao usuário para concluir o passo \"Segundo cérebro\" do onboarding do Lina.",
            path.display()
        )
    })?;
    serde_json::from_str(&data).map_err(|e| format!("vault.json inválido: {e}"))
}

/// `lina vault <sub>` — roteia os subcomandos.
fn run_vault(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("path") => vault_path(),
        Some("index") => vault_index(),
        Some("read") => vault_read(args.get(1).map(String::as_str)),
        Some("search") => vault_search(args.get(1).map(String::as_str)),
        _ => {
            eprintln!(
                "uso: lina vault path | index | read <nota> | search <termo>\n  \
                 (index = mapa estrutural PageIndex; comece por ele para navegar antes de abrir notas)"
            );
            ExitCode::from(2)
        }
    }
}

/// `lina vault path` — raiz do vault primário + lista de todos os vaults linkados.
fn vault_path() -> ExitCode {
    let cfg = match load_vault_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    println!("{}", cfg.primary);
    if cfg.vaults.len() > 1 {
        eprintln!("(vaults linkados:)");
        for v in &cfg.vaults {
            eprintln!("  - {} · {}", v.name, v.path);
        }
    }
    ExitCode::SUCCESS
}

/// `lina vault index` — imprime o(s) mapa(s) estrutural(is) PageIndex (`<LINA_HOME>/vault-index/*.md`),
/// determinísticos e LOCAIS (não re-baixa o vault). É a "porta de entrada": o agente navega o grafo de
/// pastas/headings/links/hubs aqui e SÓ ENTÃO abre as notas certas com `read`.
fn vault_index() -> ExitCode {
    let dir = mailbox_root().join("vault-index");
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => {
            eprintln!(
                "lina: índice ainda não gerado ({}). O Lina o cria em segundo plano ao linkar o vault; \
                 tente de novo em instantes, ou use `lina vault search`.",
                dir.display()
            );
            return ExitCode::from(1);
        }
    };
    let mut mds: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    mds.sort();
    if mds.is_empty() {
        eprintln!("lina: nenhum índice .md em {}", dir.display());
        return ExitCode::from(1);
    }
    for p in mds {
        match std::fs::read_to_string(&p) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("lina: não li {}: {e}", p.display()),
        }
    }
    ExitCode::SUCCESS
}

/// Resolve uma `nota` (caminho relativo, com ou sem `.md`) contra os vaults linkados. 1º match vence.
fn resolve_note(cfg: &VaultConfigJson, nota: &str) -> Option<PathBuf> {
    let rel = nota.trim_start_matches('/');
    for v in &cfg.vaults {
        for cand in [
            PathBuf::from(&v.path).join(rel),
            PathBuf::from(&v.path).join(format!("{rel}.md")),
        ] {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// `lina vault read <nota>` — lê uma nota inteira (resolve em qualquer vault linkado).
fn vault_read(nota: Option<&str>) -> ExitCode {
    let Some(nota) = nota else {
        eprintln!("uso: lina vault read <caminho/da/nota.md>");
        return ExitCode::from(2);
    };
    let cfg = match load_vault_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    match resolve_note(&cfg, nota) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(s) => {
                println!("{s}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("lina: não consegui ler '{nota}': {e}");
                ExitCode::from(1)
            }
        },
        None => {
            eprintln!("lina: nota '{nota}' não encontrada nos vaults linkados (use `lina vault index` para achar o caminho).");
            ExitCode::from(1)
        }
    }
}

/// Teto de resultados do `search` — evita varredura sem fim e flood na saída do agente.
const VAULT_SEARCH_MAX_HITS: usize = 40;

/// `lina vault search <termo>` — busca case-insensitive de uma substring no CONTEÚDO das notas `.md`
/// (read-only, ignora `.obsidian/`/`.trash/`/ocultos). Para com teto de resultados. Para NAVEGAR a
/// estrutura (mais barato que ler tudo) o agente deve preferir `lina vault index`.
fn vault_search(termo: Option<&str>) -> ExitCode {
    let Some(termo) = termo.map(str::trim).filter(|t| !t.is_empty()) else {
        eprintln!("uso: lina vault search <termo>");
        return ExitCode::from(2);
    };
    let cfg = match load_vault_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    let needle = termo.to_lowercase();
    let mut hits = 0usize;
    for v in &cfg.vaults {
        let root = PathBuf::from(&v.path);
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                if hits >= VAULT_SEARCH_MAX_HITS {
                    println!("… (parei em {VAULT_SEARCH_MAX_HITS} resultados — refine o termo)");
                    return ExitCode::SUCCESS;
                }
                let p = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if p.is_dir() {
                    if !name.starts_with('.') && name != ".trash" {
                        stack.push(p);
                    }
                } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                    let Ok(content) = std::fs::read_to_string(&p) else {
                        continue;
                    };
                    let rel = p
                        .strip_prefix(&root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    for (n, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(&needle) {
                            println!("{rel}:{}: {}", n + 1, line.trim());
                            hits += 1;
                            if hits >= VAULT_SEARCH_MAX_HITS {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    if hits == 0 {
        println!(
            "(sem resultados para \"{termo}\" — tente `lina vault index` para ver a estrutura)"
        );
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod vault_tests {
    use super::*;

    /// O `vault.json` que o app escreve (formato de `obsidian.rs::write_vault_config`) desserializa,
    /// expondo o `primary` e os caminhos — campos extras (`writable`) são ignorados sem quebrar.
    #[test]
    fn parses_app_vault_json() {
        let json = r#"{
          "primary": "/Users/x/Documents/Vault A",
          "vaults": [
            { "name": "Vault A", "path": "/Users/x/Documents/Vault A", "writable": "/Users/x/Documents/Vault A/Lina" },
            { "name": "Vault B", "path": "/Users/x/Documents/Vault B", "writable": "/Users/x/Documents/Vault B/Lina" }
          ]
        }"#;
        let cfg: VaultConfigJson = serde_json::from_str(json).expect("parseia o vault.json do app");
        assert_eq!(cfg.primary, "/Users/x/Documents/Vault A");
        assert_eq!(cfg.vaults.len(), 2);
        assert_eq!(cfg.vaults[1].name, "Vault B");
        assert_eq!(cfg.vaults[1].path, "/Users/x/Documents/Vault B");
    }

    /// `resolve_note` acha a nota em qualquer vault linkado, com ou sem `.md`, e devolve `None` p/ ausente.
    #[test]
    fn resolve_note_finds_with_and_without_md() {
        let tmp = std::env::temp_dir().join(format!("lina-vault-test-{}", std::process::id()));
        let vault = tmp.join("Vault");
        std::fs::create_dir_all(vault.join("Area")).expect("mkdir");
        std::fs::write(vault.join("Area").join("nota.md"), "# Oi\nconteúdo").expect("nota");
        let cfg = VaultConfigJson {
            primary: vault.display().to_string(),
            vaults: vec![VaultLinkJson {
                name: "Vault".into(),
                path: vault.display().to_string(),
            }],
        };
        // com .md explícito e sem (o resolver tenta `<rel>` e `<rel>.md`).
        assert!(resolve_note(&cfg, "Area/nota.md").is_some());
        assert!(resolve_note(&cfg, "Area/nota").is_some());
        // barra inicial é tolerada.
        assert!(resolve_note(&cfg, "/Area/nota.md").is_some());
        // inexistente → None.
        assert!(resolve_note(&cfg, "Area/fantasma.md").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `scan_log_outcome` (núcleo da confirmação de `lina ask`): lê o desfecho real do `log.jsonl`.
    /// Formato verbatim do espelho (`kind` + `payload.id`/`reason`/`to_node`).
    #[test]
    fn scan_log_outcome_reads_real_route_events() {
        let blocked = r#"{"seq":3,"ts":1,"kind":"RouteBlocked","version":1,"payload":{"event":"RouteBlocked","id":"msg_X","reason":"unknown_sender","from":"Terminal B","to":"@Terminal C"}}"#;
        let delivered = r#"{"seq":4,"ts":2,"kind":"MessageRouted","version":1,"payload":{"event":"MessageRouted","id":"msg_Y","from":"Terminal B","to":"@Terminal C","to_node":"019e-uuid","intent":"ask","hops":0,"root_cause_id":"msg_Y"}}"#;

        // Bloqueada → reporta o motivo.
        assert_eq!(
            scan_log_outcome(blocked, "msg_X"),
            Some(RouteConfirm::Blocked {
                reason: "unknown_sender".into()
            })
        );
        // Entregue → reporta o nó destino.
        assert_eq!(
            scan_log_outcome(delivered, "msg_Y"),
            Some(RouteConfirm::Delivered {
                to_node: "019e-uuid".into()
            })
        );
        // Entrega VENCE bloqueio quando a mesma msg tem os dois (re-tentada e entregue).
        let both = format!(
            "{}\n{}",
            blocked.replace("msg_X", "msg_Z"),
            delivered.replace("msg_Y", "msg_Z")
        );
        assert_eq!(
            scan_log_outcome(&both, "msg_Z"),
            Some(RouteConfirm::Delivered {
                to_node: "019e-uuid".into()
            })
        );
        // id ausente → None (ainda sem desfecho); linha parcial/lixo é tolerada.
        assert_eq!(scan_log_outcome(blocked, "msg_INEXISTENTE"), None);
        assert_eq!(scan_log_outcome("{lixo parcial\n", "msg_X"), None);
    }

    /// **F1-3-6: `scan_spawn_outcome` (puro).** `SpawnRequested` sem `SpawnGated` ⇒ aprovado;
    /// `SpawnGated{reason}` ⇒ gated (vence o requested); nenhum dos dois ⇒ pending. Tolera lixo.
    #[test]
    fn scan_spawn_outcome_maps_gate_events() {
        let approved = concat!(
            r#"{"kind":"SpawnRequested","payload":{"id":"msg_A","name":"@QA","role":"qa"}}"#,
            "\n"
        );
        assert_eq!(
            scan_spawn_outcome(approved, "msg_A"),
            SpawnConfirm::Approved
        );

        let gated = concat!(
            r#"{"kind":"SpawnRequested","payload":{"id":"msg_B","name":"@H","role":"h"}}"#,
            "\n",
            r#"{"kind":"SpawnGated","payload":{"id":"msg_B","reason":"cascade"}}"#,
            "\n"
        );
        assert_eq!(
            scan_spawn_outcome(gated, "msg_B"),
            SpawnConfirm::Gated {
                reason: "cascade".into()
            },
            "SpawnGated vence o SpawnRequested do mesmo id"
        );

        // id sem desfecho ⇒ Pending; linha-lixo tolerada.
        assert_eq!(scan_spawn_outcome(approved, "msg_X"), SpawnConfirm::Pending);
        assert_eq!(
            scan_spawn_outcome("{lixo\n", "msg_A"),
            SpawnConfirm::Pending
        );
    }
}
