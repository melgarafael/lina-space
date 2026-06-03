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

use lina_bootstrap::{autonomy_from_env, pretooluse_output, BootstrapInput, Bootstrapper};
use lina_core::{
    check_action, lookup_action, parse_autonomy, DomainEvent, EventStore, MailMessage, Mailbox,
    CLASS_GATED_HARD_EXTERNAL,
};

/// Arquivo de estado, relativo ao cwd do terminal (o app o escreve antes de spawnar o shell).
const INPUT_PATH: &str = ".lina/bootstrap.json";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("whoami") => run_whoami(args.iter().any(|a| a == "--bootstrap")),
        Some("ask") => run_ask(&args[1..]),
        Some("broadcast") => run_broadcast(&args[1..]),
        Some("handshake") => run_handshake(),
        Some("plan") => run_plan(&args[1..]),
        Some("guard") => run_guard(&args[1..]),
        Some("resume") => run_resume(&args[1..]),
        Some("do") => run_do(&args[1..]),
        Some("list") => run_list(args.iter().any(|a| a == "--json")),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "uso:\n  lina whoami [--bootstrap]\n  lina ask @<alvo> \"<msg>\" [--await] [--intent ask|handoff|broadcast|...] [--role PAPEL] [--reply-to <id>]\n  lina broadcast \"*\" \"<msg>\"   (avisa TODOS os terminais vivos; --role PAPEL p/ um papel. ADR0007:\n   o fan-out INICIAL pedido pelo humano entrega a todos SEM gate; a CASCATA (re-espalhar) pede ok.)\n  lina handshake\n  lina plan read | claim <id> | check <id>\n  lina guard --check-action --cmd \"<comando>\" --autonomy <manual|assistido|autonomo>\n  lina guard --pretooluse   (hook PreToolUse do Claude Code: le JSON no stdin, emite a decisao em JSON no stdout)\n  lina resume   (W3-7c: PEDE retomada do teto de custo; o agente NAO des-pausa — gate humano na janela)\n  lina do <deploy|pay|send> [args]   (W3-6c: acao custodiada; o agente REGISTRA, NAO executa)\n  lina list [--json]   (W4-2: lista os agentes do workspace — nome/papel/status do agents.json)\n\n  (--reply-to <id>: responde a uma pergunta --await; fecha o await do colega)\n  (resume: registra resume.request na fila de broker por-no; o supervisor apenda CostCeilingResumed SO\n   apos confirmacao HUMANA na janela (Cmd+Enter). O agente, sozinho, NUNCA tira do estado Paused.)\n  (guard --check-action: imprime allow|ask|deny; apenda ActionGated ao log quando NAO for allow)\n  (guard --pretooluse: autonomia via LINA_AUTONOMY (default assistido); fail-safe ask em erro)\n  (do: gated-hard-external; o segredo vive so no SecretVault do Lina. O agente nao tem o token nem\n   confirmacao -> registra o pedido + apenda ActionGated{{ask}}+BrokerDenied{{unconfirmed}}; quem executa\n   COM o segredo, apos gate humano, e o supervisor/broker. Custodia = camada inquebravel, ADR 0004.)"
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
    let mailbox = Mailbox::new(mailbox_root());
    // W3-6c A3: outbox POR-NÓ — `from` é autenticado pela origem (dir-dono), não pelo campo forjável.
    match enqueue_per_node(&mailbox, &from, &msg) {
        Ok(()) => {
            // Feedback HONESTO (#22): a entrega é ASSÍNCRONA — quem decide o destino final é o
            // supervisor (guardrails anti-loop/teto de custo podem barrar DEPOIS). Não prometer
            // "entregue"; dizer que foi enviada e que, se um limite barrar, fica no log do Espaço.
            // (Sem o jargão "enfileirada", que confundiu o fundador não-técnico.)
            println!(
                "ok: enviada a {} (id {}). A entrega é automática e pode levar um instante; se um \
                 limite de segurança (anti-loop ou teto de custo) barrar, fica registrado no log do Espaço.",
                msg.to, msg.id
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
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
