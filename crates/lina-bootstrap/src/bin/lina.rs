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
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lina_bootstrap::{
    autonomy_from_env, canonical_role, classify_retro_args, parse_log_records, pretooluse_result,
    project_retro, render_report, Autonomy, BootstrapInput, Bootstrapper, GatedAsk,
    RetroInvocation, AUTONOMY_ENV,
};
use lina_core::history::{self, ExportFormat, HistoryLimits, HistoryPage, SearchPage};
use lina_core::scrollback::ScrollbackStore;
use lina_core::{
    check_action, lookup_action, parse_autonomy, project_goals, AcceptanceCriterion, CheckKind,
    DomainEvent, EventStore, Goal, GoalPhase, HandoffContract, MailMessage, Mailbox, NodeId,
    ParamsLedger, SystemParams, CLASS_GATED_HARD_EXTERNAL,
};

/// Arquivo de estado, relativo ao cwd do terminal (o app o escreve antes de spawnar o shell).
const INPUT_PATH: &str = ".lina/bootstrap.json";

/// **ADR 0026 (BUG-1 dogfood r1) — identidade por ENV DE SPAWN.** O app injeta
/// `LINA_NODE_NAME` (e `LINA_NODE_ID`) no env do PTY ao admitir o nó — autoridade do APP,
/// inforjável por conteúdo de mensagem e POR-PROCESSO (não por-diretório). Com N terminais
/// no MESMO cwd (default da F1-4-1), a ficha `.lina/bootstrap.json` é último-escritor-vence;
/// o env não colide. Ordem de resolução do nome: (a) env, se presente; (b) ficha (compat
/// terminal puro/standalone). Os DEMAIS campos (roster/vault/autonomia/plano) continuam
/// vindo da ficha — só `terminal_name` prefere o env.
const NODE_NAME_ENV: &str = "LINA_NODE_NAME";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("whoami") => run_whoami(args.iter().any(|a| a == "--bootstrap")),
        Some("ask") => run_ask(&args[1..]),
        Some("handoff") => run_handoff(&args[1..]),
        Some("check") => run_check(&args[1..]),
        Some("history") => run_history(&args[1..]),
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
        Some("params") => run_params(&args[1..]),
        Some("effort") => run_effort(&args[1..]),
        Some("goal") => run_goal(&args[1..]),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!(
        "uso:\n  lina whoami [--bootstrap]\n  lina ask @<alvo> \"<msg>\" [--await] [--intent ask|handoff|broadcast|...] [--role PAPEL] [--reply-to <id>]\n  lina handoff @<alvo> \"<tarefa>\" [--context <arquivo>] [--ref plan:<id>] [--timeout-sec N] [--await]\n   (F1-0-6: delega COM contrato estruturado lina/msg@2 — schema de entrada/saida, timeout, retry;\n    --context ANEXA o conteudo do arquivo ao payload. Fire-and-forget por padrao; acompanhe com\n    `lina check`. Em autonomia manual o proprio comando recusa — delegacao bloqueada localmente.)\n  lina check @<alvo>   (F1-0-6: estado VIVO do colega — Ready/Busy/Idle/Blocked/Dead + motivo da\n   ultima transicao + travamento (ADR 0019) + ultima atividade A2A. LEITURA PURA de agents.json +\n   log.jsonl: nao injeta NADA no terminal do colega.)\n  lina history @<colega> [--tail N] [--offset K] [--search \"<regex>\" [--limit N] [--cursor I]]\n   [--export json|txt --from A --to B] [--json]   (#15: o Maestro VE a tela do colega — leitura PURA\n   do scrollback pela fronteira de pertencimento (ADR 0006): membro do mesmo Espaco le, fora dela e\n   barrado + auditado. Default imprime as ultimas linhas; --json devolve o formato do contrato F1.\n   NAO injeta nada — espiar != cutucar, igual `lina check`.)\n  lina broadcast \"*\" \"<msg>\"   (avisa TODOS os terminais vivos; --role PAPEL p/ um papel. ADR0007:\n   o fan-out INICIAL pedido pelo humano entrega a todos SEM gate; a CASCATA (re-espalhar) pede ok.)\n  lina handshake\n  lina plan read | claim <id> | check <id> | add <id> \"<desc>\" [--goal G] [--parents T1,T2] [--accept \"<>\"] [--budget N] | seed <goal_id>\n  lina guard --check-action --cmd \"<comando>\" --autonomy <manual|assistido|autonomo>\n  lina guard --pretooluse   (hook PreToolUse do Claude Code: le JSON no stdin, emite a decisao em JSON no stdout)\n  lina resume   (W3-7c: PEDE retomada do teto de custo; o agente NAO des-pausa — gate humano na janela)\n  lina do <deploy|pay|send> [args]   (W3-6c: acao custodiada; o agente REGISTRA, NAO executa)\n  lina list [--json]   (W4-2: lista os agentes do workspace — nome/papel/status do agents.json)\n  lina vault path | index | read <nota> | search <termo>   (segundo cerebro: le os vault(s) Obsidian\n   linkados no onboarding em .lina/vault.json; `index` mostra o mapa estrutural PageIndex; `read`/`search`\n   acessam as notas. Comece por `index` para NAVEGAR antes de abrir notas.)\n  lina spawn @<Nome> --role <papel> [--prompt \"<1o prompt>\"]   (F1-3-6: PEDE criar um terminal novo\n   quando falta um papel. Gate inforjavel: ORIGEM ok; CASCATA/cap/custo pedem aval humano; manual\n   recusa. A criacao fisica e do Espaco — voce NAO cunha o terminal.)\n  lina retro [--json] [--now-ms <ms>]   (F1-3-7: auto-aprimoramento v0. Le o event log (SO-LEITURA) e\n   emite um RELATORIO deterministico de projecoes: skills (uso/stale>30d/archive>90d), coordenacao\n   (bloqueios/spawns gated/re-delegacoes/breaker), custos por terminal+outliers, pedidos de origem e\n   lacunas de papel. ZERO LLM: quem PROPOE melhorias e o agente (skill lina-retro), com gate humano.\n   So OBSERVA e SUGERE — nao existe `lina retro apply`; arquivar/fixar/mudar passa pelo humano.)\n  lina params show | set <chave> <valor> --scope <escopo> [--target <alvo>] | reset <chave> --scope <escopo>\n   (F3-0-5: parametros de orquestracao versionados. show projeta o log (SO-LEITURA); set/reset enfileiram\n    p/ o supervisor validar a faixa, carimbar a origem e aplicar. escopos: global|workspace|preset|terminal;\n    em autonomia manual o proprio comando recusa.)\n  lina effort @<Nome> <low|medium|high>   (F3-0-5: define o nivel de raciocinio (cognicao) de um terminal;\n   enfileira p/ o supervisor resolver o alvo, validar e aplicar. manual recusa; auto-atribuicao e barrada server-side.)\n  lina goal define \"<meta>\" [--budget N] [--accept \"<criterio>\"]... | interpret <goal_id> --understanding \"<>\" --strategy \"<>\" [--team A,B] [--accept ...] | confirm <goal_id> | status <goal_id> [--json]\n   (F3-1: a Meta como primitiva. define/interpret/confirm ENFILEIRAM o intent (o supervisor cunha o goal_id,\n    valida o ciclo e emite os eventos); status le a projecao da Goal (SO-LEITURA). manual recusa as mutacoes.)\n\n  (--reply-to <id>: responde a uma pergunta --await; fecha o await do colega)\n  (resume: registra resume.request na fila de broker por-no; o supervisor apenda CostCeilingResumed SO\n   apos confirmacao HUMANA na janela (Cmd+Enter). O agente, sozinho, NUNCA tira do estado Paused.)\n  (guard --check-action: imprime allow|ask|deny; apenda ActionGated ao log quando NAO for allow)\n  (guard --pretooluse: autonomia via LINA_AUTONOMY (default assistido); fail-safe ask em erro)\n  (do: gated-hard-external; o segredo vive so no SecretVault do Lina. O agente nao tem o token nem\n   confirmacao -> registra o pedido + apenda ActionGated{{ask}}+BrokerDenied{{unconfirmed}}; quem executa\n   COM o segredo, apos gate humano, e o supervisor/broker. Custodia = camada inquebravel, ADR 0004.)"
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
    load_input_at(Path::new(INPUT_PATH))
}

/// Lê a ficha em `path`. Separada de [`load_input`] para os testes provarem o compat:
/// ficha AUSENTE continua erro com orientação (jamais identidade inventada).
fn load_input_at(path: &Path) -> Result<BootstrapInput, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

/// Valor de [`NODE_NAME_ENV`] (trim; vazio = ausente — env vazio não apaga a ficha).
fn env_node_name() -> Option<String> {
    let v = std::env::var(NODE_NAME_ENV).ok()?;
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// **Resolução de nome (ADR 0026, função PURA):** env do spawn vence a ficha por-cwd.
/// Env ausente/vazio → ficha (compat terminal puro).
fn resolved_name(env_name: Option<&str>, ficha_name: &str) -> String {
    match env_name.map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => ficha_name.to_string(),
    }
}

/// Valor de [`AUTONOMY_ENV`] (`LINA_AUTONOMY`, trim; vazio = ausente — env vazio não apaga a ficha).
fn env_autonomy() -> Option<String> {
    let v = std::env::var(AUTONOMY_ENV).ok()?;
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Rótulo de autonomia → enum. Aceita o pt-br do `Autonomy::label()` (o que o app injeta no env) e
/// as formas serde en. Desconhecido → `None` (cai na ficha; NUNCA inventa um nível).
fn parse_autonomy_label(s: &str) -> Option<Autonomy> {
    match s.trim().to_ascii_lowercase().as_str() {
        "manual" => Some(Autonomy::Manual),
        "assistido" | "assisted" => Some(Autonomy::Assisted),
        "autonomo" | "autônomo" | "autonomous" => Some(Autonomy::Autonomous),
        _ => None,
    }
}

/// **#17 residual (ADR 0026 / FIX-3) — autonomia env-first, função PURA:** o app injeta
/// `LINA_AUTONOMY` POR-NÓ no PTY (`bridge.rs::node_identity_env`; autoridade do APP, por-processo).
/// O GATE local de handoff/spawn/params/goal precisa honrar o nível DESTE nó — não o do
/// `bootstrap.json` do cwd COMPARTILHADO, que é último-escritor-vence: com N terminais no mesmo cwd,
/// um colega sobrescreve a ficha e o gate leria a autonomia ERRADA (liberar onde devia recusar).
/// Env presente/legível vence; ausente/desconhecido → a ficha (compat standalone). Espelha
/// [`resolved_name`] — mesma doutrina de identidade-no-env do ADR 0026.
fn resolved_autonomy(env: Option<&str>, ficha: Autonomy) -> Autonomy {
    env.and_then(parse_autonomy_label).unwrap_or(ficha)
}

/// Ficha com a IDENTIDADE RESOLVIDA (ADR 0026): carrega `.lina/bootstrap.json` e aplica a
/// preferência de env em `terminal_name` E `autonomy` (#17) — whoami/handshake/`enqueue_as`
/// (outbox por-nó) e os GATES de delegação passam a usar o nome E o nível certos mesmo com a ficha
/// sobrescrita por um colega de cwd. Demais campos (roster/vault/plano) seguem da ficha.
fn load_identity() -> Result<BootstrapInput, String> {
    let mut input = load_input()?;
    input.terminal_name = resolved_name(env_node_name().as_deref(), &input.terminal_name);
    input.autonomy = resolved_autonomy(env_autonomy().as_deref(), input.autonomy);
    Ok(input)
}

/// `hook = true` → JSON do `SessionStart`; `false` → bloco legível.
fn run_whoami(hook: bool) -> ExitCode {
    let input = match load_identity() {
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
    // BUG-3 (dogfood r1): papéis REAIS do roster (`agents.json`, autoridade do supervisor) —
    // paridade whoami × handshake × list. Ausente (terminal puro) → inferência por nome.
    let roles = roster_roles();
    if hook {
        println!("{}", bs.whoami_hook_json_with_roles(&input, &roles));
    } else {
        println!("{}", bs.whoami_with_roles(&input, &roles));
        // FIX-4: o ramo HUMANO ganha a linha de estado global; o JSON do hook NÃO (não corromper o
        // contrato de contexto do SessionStart). O agente que roda `lina whoami` enxerga o freio/teto.
        println!("{}", space_state_line(space_state()));
    }
    ExitCode::SUCCESS
}

/// Papéis REAIS do roster, lidos do `agents.json` (escrito pelo supervisor): pares
/// `(nome, papel cru)`. Arquivo ausente/ilegível → vazio (o whoami cai na inferência).
fn roster_roles() -> Vec<(String, String)> {
    Mailbox::new(mailbox_root())
        .read_agents()
        .map(|agents| {
            agents
                .into_iter()
                .filter_map(|a| a.role.map(|r| (a.name, r)))
                .collect()
        })
        .unwrap_or_default()
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

    let from = match load_identity() {
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
    if let Err(e) = enqueue_per_node(&mailbox, from, &msg) {
        eprintln!("lina: falha ao enfileirar na mailbox: {e}");
        return ExitCode::from(1);
    }
    // FIX-4: o freio de orquestração e o teto de custo são estados GLOBAIS que SÓ o humano vê no
    // canvas. Sob pausa, a msg JÁ ENFILEIROU acima (durável — nada se perde) mas NÃO será roteada
    // até o humano retomar; o poll só devolveria `Pending` → o velho "tente de novo" fazia o agente
    // pedalar 20 min contra a fila congelada. Contamos a verdade INTEIRA (o agente NARRA e PARA);
    // a fila é a MESMA — só a narração muda.
    if let Some(notice) = dispatch_pause_notice(space_state()) {
        println!("{notice}");
        return ExitCode::SUCCESS;
    }
    match poll_route_outcome(&msg.id) {
        RouteConfirm::Delivered { to_node } => {
            let dst = if to_node.is_empty() {
                msg.to.clone()
            } else {
                to_node
            };
            println!("ok: {dst} recebeu a mensagem (id {}).", msg.id);
            ExitCode::SUCCESS
        }
        RouteConfirm::Routed { to_node } => {
            // #22c: o Espaço ACEITOU e roteou a msg, mas a injeção física no terminal do destino
            // ainda NÃO foi confirmada (sem `MessageDelivered`). NÃO dizemos "recebeu" — seria o
            // falso-entregue que cegava o orquestrador. Honesto: roteada, entrega ainda pendente.
            let dst = if to_node.is_empty() {
                msg.to.clone()
            } else {
                to_node
            };
            println!(
                "ok: a mensagem foi ROTEADA para {dst} (id {}), mas a entrega no terminal dele \
                 ainda NAO foi confirmada. NAO conclua que ja virou trabalho — confirme o 1o \
                 progresso com `lina check` antes de marcar como entregue.",
                msg.id
            );
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

    let input = match load_identity() {
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
/// **PURO (r4 achado #13; r-confiab #4/#14/#23c) — resolve um NOME para o node-id CERTO do log.**
/// Nome reusado entre sessões gera homônimos: o `check` apontava o lifecycle MORTO da sessão antiga.
/// Semântica: o nome ATUAL de um nó é o seu ÚLTIMO `NodeRenamed`; entre os nós cujo nome atual casa
/// com `target` (tolerante a `@`/caixa — espelho do `normalize_name` do roster vivo), vence o
/// **vivo** mais recentemente batizado; sem vivo, o último batizado (exibe o morto, honesto).
/// MORTE = `NodeStatusChanged(Dead)` OU `TerminalExited` OU `NodeRemoved` (não só o 1º — senão um nó
/// que saiu com último status "Idle" parecia vivo). Tolera linhas parciais/inválidas (arquivo sob
/// append).
fn resolve_check_node(content: &str, target: &str) -> Option<String> {
    fn norm(s: &str) -> String {
        s.trim().trim_start_matches('@').trim().to_ascii_lowercase()
    }
    let want = norm(target);
    if want.is_empty() {
        return None;
    }
    // (node → nome atual), mantido em ordem de RECÊNCIA do último batismo.
    let mut names: Vec<(String, String)> = Vec::new();
    // Último SINAL DE VIDA por nó (último vence), espelhando a projeção do core (`events.rs`):
    // `NodeStatusChanged` e `TerminalSpawned`("Running") dão status; `TerminalExited`("Dead") e
    // `NodeRemoved` são MORTE. #4/#14/#23c: antes só `NodeStatusChanged` era lido — um nó que SAIU
    // por `TerminalExited`/`NodeRemoved` com último status "Idle" parecia VIVO, e o `check` apontava
    // o lifecycle da sessão ANTIGA. Agora a morte conta por qualquer um dos três sinais.
    let mut last_status: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut removed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let p = &v["payload"];
        let node = p["node"].as_str();
        match v["kind"].as_str().unwrap_or_default() {
            "NodeRenamed" => {
                if let (Some(node), Some(name)) = (node, p["name"].as_str()) {
                    names.retain(|(n, _)| n != node);
                    names.push((node.to_string(), name.to_string()));
                }
            }
            "NodeStatusChanged" => {
                if let (Some(node), Some(st)) = (node, p["status"].as_str()) {
                    last_status.insert(node.to_string(), st.to_string());
                }
            }
            "TerminalSpawned" => {
                if let Some(node) = node {
                    last_status.insert(node.to_string(), "Running".to_string());
                }
            }
            "TerminalExited" => {
                if let Some(node) = node {
                    last_status.insert(node.to_string(), "Dead".to_string());
                }
            }
            "NodeRemoved" => {
                if let Some(node) = node {
                    removed.insert(node.to_string());
                }
            }
            _ => {}
        }
    }
    let candidates: Vec<&str> = names
        .iter()
        .filter(|(_, name)| norm(name) == want)
        .map(|(node, _)| node.as_str())
        .collect();
    // Prioridade de seleção (maior vence; empate → batismo mais recente, i.e. maior índice):
    //   2 = VIVO com status conhecido não-morto;  1 = sem sinal (desconhecido — não vence um vivo
    //   provado, mas vence um morto);  0 = MORTO (status Dead, `TerminalExited` ou `NodeRemoved`).
    // "status ausente não é vivo por default quando há homônimo com status" (#4/#14): tier 1 < 2.
    // Todos mortos → o de maior índice (último batizado) — exibe o morto, honesto.
    let priority = |node: &str| -> u8 {
        if removed.contains(node) || last_status.get(node).map(String::as_str) == Some("Dead") {
            0
        } else if last_status.contains_key(node) {
            2
        } else {
            1
        }
    };
    candidates
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|&(idx, node)| (priority(node), idx))
        .map(|(_, node)| node.to_string())
}

// ───────────────── #15 (achado dogfooding): `lina history` — o Maestro vê a tela do colega ─────────────────

/// Env do NodeId do terminal CORRENTE (o LEITOR), injetado pelo app ao admitir o nó (ADR 0026).
const NODE_ID_ENV: &str = "LINA_NODE_ID";

/// Valor de uma flag `--nome <valor>` em `args` (1ª ocorrência). `None` se ausente ou sem valor.
fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Identidade do LEITOR, do env injetado pelo app (ADR 0026 — autoridade do app, JAMAIS de flag/
/// arquivo de agente: campo de agente não decide autorização). Sem ele não há como provar quem lê →
/// a leitura cross é negada (fail-safe — nunca espia às cegas).
fn reader_node_id() -> Result<NodeId, String> {
    let raw = std::env::var(NODE_ID_ENV).map_err(|_| {
        format!("{NODE_ID_ENV} ausente — nao sei quem e este terminal (rode dentro do Espaco).")
    })?;
    raw.trim()
        .parse::<NodeId>()
        .map_err(|_| format!("{NODE_ID_ENV} invalido: {raw:?}"))
}

/// NodeIds VIVOS do Espaço — os MEMBROS da fronteira de pertencimento (ADR 0006). Projeção do log
/// (inv #4): um nó é membro se apareceu e seu último sinal NÃO é morte (`NodeStatusChanged(Dead)`,
/// `TerminalExited` ou `NodeRemoved`) — MESMA semântica de morte do `resolve_check_node` (#4/#14/#23c).
/// A ordem não importa (a fronteira é um teste de `contains`), então um `HashSet` basta.
fn live_member_ids(content: &str) -> Vec<NodeId> {
    use std::collections::{HashMap, HashSet};
    let mut seen: HashSet<String> = HashSet::new();
    let mut last_status: HashMap<String, String> = HashMap::new();
    let mut removed: HashSet<String> = HashSet::new();
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let p = &v["payload"];
        let Some(node) = p["node"].as_str() else {
            continue;
        };
        match v["kind"].as_str().unwrap_or_default() {
            "NodeAdded" | "NodeRenamed" => {
                seen.insert(node.to_string());
            }
            "TerminalSpawned" => {
                seen.insert(node.to_string());
                last_status.insert(node.to_string(), "Running".to_string());
            }
            "NodeStatusChanged" => {
                seen.insert(node.to_string());
                if let Some(st) = p["status"].as_str() {
                    last_status.insert(node.to_string(), st.to_string());
                }
            }
            "TerminalExited" => {
                seen.insert(node.to_string());
                last_status.insert(node.to_string(), "Dead".to_string());
            }
            "NodeRemoved" => {
                removed.insert(node.to_string());
            }
            _ => {}
        }
    }
    seen.into_iter()
        .filter(|n| !removed.contains(n) && last_status.get(n).map(String::as_str) != Some("Dead"))
        .filter_map(|n| n.parse::<NodeId>().ok())
        .collect()
}

/// Resolve o ALVO (`@worker`) ao seu NodeId VIVO no log (homônimo vivo vence o morto — `resolve_check_node`,
/// #4/#14/#23c). O painel do scrollback é chaveado pelo NodeId (igual ao app — `bridge.rs`:
/// `panel = node.to_string()`), então o NodeId resolvido É o painel.
fn resolve_owner_node(content: &str, target: &str) -> Result<NodeId, String> {
    let id = resolve_check_node(content, target).ok_or_else(|| {
        format!("nao encontrei '{target}' no Espaco — confira quem esta vivo com `lina list`.")
    })?;
    id.parse::<NodeId>()
        .map_err(|_| format!("node-id invalido no log para '{target}': {id:?}"))
}

/// Render LEGÍVEL do `tail` (a "tela do colega" — o objetivo do #15): as LINHAS, com cabeçalho e
/// rodapé de paginação. `--json` → a [`HistoryPage`] serializada (contrato F1, uso programático).
/// Função PURA (testável). Janela expirada/vazia degrada com texto honesto, nunca erro.
fn render_tail(page: &HistoryPage, json: bool, target: &str) -> String {
    if json {
        return serde_json::to_string(page)
            .map(|s| format!("{s}\n"))
            .unwrap_or_default();
    }
    if page.expired && page.lines.is_empty() {
        return format!("@{target} — historico expirado (retencao excedida).\n");
    }
    if page.lines.is_empty() {
        return format!("@{target} — sem historico ainda.\n");
    }
    let mut out = format!(
        "@{target} — ultimas {} linha(s) (a partir da #{}):\n",
        page.lines.len(),
        page.start
    );
    for line in &page.lines {
        out.push_str(line);
        out.push('\n');
    }
    if let Some(next) = page.next_cursor {
        out.push_str(&format!(
            "  … mais antigas: `lina history @{target} --offset {next}`\n"
        ));
    }
    out
}

/// Render legível do `search`: os hits (índice global + linha). `--json` → a [`SearchPage`] do contrato.
fn render_search(page: &SearchPage, json: bool, target: &str) -> String {
    if json {
        return serde_json::to_string(page)
            .map(|s| format!("{s}\n"))
            .unwrap_or_default();
    }
    if page.hits.is_empty() {
        let mut out = format!("@{target} — nenhuma linha casou.\n");
        if let Some(next) = page.next_cursor {
            out.push_str(&format!("  … continue a varredura: `--cursor {next}`\n"));
        }
        return out;
    }
    let mut out = format!("@{target} — {} ocorrencia(s):\n", page.hits.len());
    for hit in &page.hits {
        out.push_str(&format!("  #{}: {}\n", hit.idx, hit.line));
    }
    if let Some(next) = page.next_cursor {
        out.push_str(&format!("  … continue: `--cursor {next}`\n"));
    }
    out
}

/// **#15 — observabilidade do Maestro: ver a tela do colega.** LEITURA PURA do scrollback de um
/// terminal (não injeta NADA — espiar ≠ cutucar, igual `lina check`), pela fronteira de pertencimento
/// (ADR 0006): membros do mesmo Espaço leem; fora dela, default-deny + auditoria (`HistoryReadCross`).
/// Identidade do leitor vem do env do app (ADR 0026), nunca de flag. Default imprime as LINHAS (a
/// "tela"); `--json` devolve o `HistoryPage`/`SearchPage` do contrato F1. SEMPRE via `*_cross` (que
/// audita+gate; same-owner passa sem evento) — o caminho cross nunca chama a variante pura.
fn run_history(args: &[String]) -> ExitCode {
    let Some(target_raw) = args.first() else {
        usage();
        return ExitCode::from(2);
    };
    let target = target_raw.trim_start_matches('@');
    let json = args.iter().any(|a| a == "--json");

    let reader = match reader_node_id() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    let content = std::fs::read_to_string(event_log_path()).unwrap_or_default();
    let owner = match resolve_owner_node(&content, target) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    let members = live_member_ids(&content);
    let panel = owner.to_string();

    let store = match ScrollbackStore::open_default(mailbox_root()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lina: falha ao abrir o scrollback: {e}");
            return ExitCode::from(1);
        }
    };
    let mut events = match EventStore::open(events_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lina: falha ao abrir o event store (auditoria da leitura): {e}");
            return ExitCode::from(1);
        }
    };
    let limits = HistoryLimits::default();

    // Modo: search > export > tail (default = "ultimo estado/saida", o que o #15 pede).
    if let Some(pattern) = flag_value(args, "--search") {
        let limit = flag_value(args, "--limit").and_then(|s| s.parse::<usize>().ok());
        let cursor = flag_value(args, "--cursor").and_then(|s| s.parse::<u64>().ok());
        match history::search_cross(
            &mut events,
            &members,
            reader,
            owner,
            &store,
            &panel,
            &pattern,
            limit,
            cursor,
            &limits,
        ) {
            Ok(page) => {
                print!("{}", render_search(&page, json, target));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("lina: {e}");
                ExitCode::from(1)
            }
        }
    } else if let Some(fmt_raw) = flag_value(args, "--export") {
        let fmt = match fmt_raw.as_str() {
            "json" => ExportFormat::Json,
            "txt" => ExportFormat::Txt,
            other => {
                eprintln!("lina: formato de export invalido: {other:?} (use json|txt)");
                return ExitCode::from(2);
            }
        };
        let lo = flag_value(args, "--from")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let hi = flag_value(args, "--to")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::MAX);
        match history::export_cross(
            &mut events,
            &members,
            reader,
            owner,
            &store,
            &panel,
            fmt,
            lo,
            hi,
            &limits,
        ) {
            Ok((payload, _next)) => {
                print!("{payload}");
                if !payload.ends_with('\n') {
                    println!();
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("lina: {e}");
                ExitCode::from(1)
            }
        }
    } else {
        let n = flag_value(args, "--tail").and_then(|s| s.parse::<usize>().ok());
        let offset = flag_value(args, "--offset")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        match history::tail_cross(
            &mut events,
            &members,
            reader,
            owner,
            &store,
            &panel,
            n,
            offset,
            &limits,
        ) {
            Ok(page) => {
                print!("{}", render_tail(&page, json, target));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("lina: {e}");
                ExitCode::from(1)
            }
        }
    }
}

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
    // r4 achado #13: a resolução nome→nó vem PRONTA de `resolve_check_node` (homônimo vivo
    // vence o morto de sessão antiga; sigil/caixa tolerados) — antes, um match exato inline
    // grudava no primeiro nó renomeado e mostrava o lifecycle errado em nome reusado.
    let content = std::fs::read_to_string(event_log_path()).unwrap_or_default();
    let node_id = resolve_check_node(&content, target);
    let mut state: Option<(String, String)> = None; // (status, reason)
    let mut stalled = false;
    let mut last_a2a: Option<(String, String, String, u64)> = None; // intent, from, to, ts
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let p = &v["payload"];
        match v["kind"].as_str().unwrap_or_default() {
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
    // FIX-4: a mesma dor — o agente checava um colega "Idle" sem saber que o ESPAÇO estava pausado e
    // re-tentava sem fim. A linha de estado global (leitura pura do log) torna visível o freio/teto
    // que só apareciam no canvas.
    println!("{}", space_state_line(space_state()));
    ExitCode::SUCCESS
}

/// Desfecho REAL do roteamento de uma `lina ask`, lido do espelho `log.jsonl`.
#[derive(Debug, PartialEq, Eq)]
enum RouteConfirm {
    /// **Entrega REAL**: `MessageDelivered` no log — a injeção física no PTY do destino ocorreu
    /// (ready:true + submit). É a ÚNICA confirmação que autoriza dizer "recebeu".
    Delivered { to_node: String },
    /// **Roteada, aguardando entrega**: o roteador aceitou e emitiu `MessageRouted` (gravado ANTES
    /// da injeção física — `router.rs`), mas NÃO há `MessageDelivered`. O remetente NÃO pode
    /// concluir "entregue" daqui (#22c: era o falso-entregue que cegava o orquestrador).
    Routed { to_node: String },
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

/// **PURO** (testável, sem I/O/timing): varre o `log.jsonl` pelo desfecho de `msg_id`.
///
/// **"Entregue" exige `MessageDelivered`** (injeção física real). `MessageRouted` (gravado ANTES da
/// injeção no `router.rs`) vira no MÁXIMO `Routed` ("roteada, aguardando entrega"). Antes os dois
/// viravam `Delivered` e o remetente via "recebeu" mesmo quando NADA foi injetado (#22c: o report
/// mentia e o orquestrador seguia cego, marcando como feito o que nunca começou). Precedência:
/// `Delivered` (injeção confirmada, vence tudo) > `Routed` (roteada, sem entrega) > `Blocked`.
/// `None` se ainda não há desfecho. Tolera linhas parciais/inválidas (arquivo sob append).
fn scan_log_outcome(content: &str, msg_id: &str) -> Option<RouteConfirm> {
    let dest = |p: &serde_json::Value| {
        p.get("to_node")
            .and_then(serde_json::Value::as_str)
            .or_else(|| p.get("to").and_then(serde_json::Value::as_str))
            .unwrap_or("")
            .to_string()
    };
    let mut routed: Option<String> = None;
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
            // Injeção física confirmada → entregue de fato; vence qualquer outro desfecho.
            Some("MessageDelivered") => return Some(RouteConfirm::Delivered { to_node: dest(p) }),
            // Roteada (pré-injeção): guarda, mas SEGUE varrendo — um Delivered posterior vence.
            Some("MessageRouted") => routed = Some(dest(p)),
            Some("RouteBlocked") => {
                last_block = p
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
            }
            _ => {}
        }
    }
    // Sem Delivered: roteada (aguardando entrega) vence bloqueio; senão, o bloqueio.
    routed
        .map(|to_node| RouteConfirm::Routed { to_node })
        .or_else(|| last_block.map(|reason| RouteConfirm::Blocked { reason }))
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
            // Entregue (injeção real) → conclui já. Roteada/Bloqueada NÃO concluem cedo: a injeção
            // física emite `MessageDelivered` logo após `MessageRouted` — seguimos no poll até o
            // prazo para CAPTURAR esse upgrade (senão reportaríamos "roteada" no caminho feliz).
            Some(o @ RouteConfirm::Delivered { .. }) => return o,
            Some(o @ (RouteConfirm::Routed { .. } | RouteConfirm::Blocked { .. }))
                if Instant::now() >= deadline =>
            {
                return o
            }
            _ if Instant::now() >= deadline => return RouteConfirm::Pending,
            _ => std::thread::sleep(Duration::from_millis(150)),
        }
    }
}

/// **FIX-4 — estado GLOBAL do Espaço projetado do event log** (livro-razão; invariante #4). São
/// estados que SÓ o humano vê (rodapé do canvas) e que, invisíveis ao agente, o faziam pedalar
/// contra uma fila congelada ("estados globais do Espaço são invisíveis aos agentes" — mesma
/// família do teto de custo). Cada flag segue o ÚLTIMO evento de transição do log — o mesmo replay
/// de `Router::restore_orchestration_state` e do `CostLedger` (W3-7c §2.2). `Default` = nada
/// pausado: a AUSÊNCIA de freio no log É "ativo" (nunca inventamos uma pausa que o humano não pôs).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SpaceState {
    /// Freio de orquestração (W4-3): `true` = PAUSADO — delegações novas ENFILEIRAM (duráveis), não
    /// são roteadas, até o humano clicar ▶ Retomar cooperação no canvas.
    orchestration_paused: bool,
    /// Teto de custo do dia (W3-7c): `true` = ATINGIDO — workspace pausado até a confirmação humana
    /// (`lina resume` → o supervisor apenda `CostCeilingResumed`).
    cost_ceiling_hit: bool,
}

/// **PURO** (testável, sem I/O): projeta o [`SpaceState`] varrendo o `log.jsonl`. Cada flag segue o
/// ÚLTIMO evento de transição (último vence — idêntico ao replay do core). Só o `kind` decide (o
/// freio nem tem payload; o teto tem campos que aqui são irrelevantes). Tolera linhas parciais/
/// inválidas (arquivo sob append, mesma postura de [`scan_log_outcome`]).
fn scan_space_state(content: &str) -> SpaceState {
    let mut state = SpaceState::default();
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("kind").and_then(serde_json::Value::as_str) {
            Some("OrchestrationPaused") => state.orchestration_paused = true,
            Some("OrchestrationResumed") => state.orchestration_paused = false,
            Some("CostCeilingHit") => state.cost_ceiling_hit = true,
            Some("CostCeilingResumed") => state.cost_ceiling_hit = false,
            _ => {}
        }
    }
    state
}

/// Lê o espelho `log.jsonl` (NÃO o SQLite — evita conexão concorrente/lock na troca de WAL, como
/// [`poll_route_outcome`]) e projeta o [`SpaceState`]. Log ausente/ilegível → `Default` (nada
/// pausado): sem evento de freio, o Espaço está ativo.
fn space_state() -> SpaceState {
    std::fs::read_to_string(event_log_path())
        .ok()
        .map(|c| scan_space_state(&c))
        .unwrap_or_default()
}

/// Copy congelada (fundador, FIX-4) do freio de orquestração — narração leiga que o agente LÊ e
/// repassa ao humano (anti-eco). Os símbolos ⏸/▶ casam os botões do rodapé do canvas (o humano
/// clica em ▶ Retomar cooperação); são semânticos, não decoração.
const ORCHESTRATION_PAUSED_NOTICE: &str = "⏸ o Espaço está PAUSADO — sua mensagem ficou guardada na fila (nada se perde). Diga ao usuário: clique em ▶ Retomar cooperação para o time voltar a se falar.";

/// Copy do teto de custo (FIX-4, mesma família do freio): o teto do dia foi atingido; a retomada
/// exige confirmação HUMANA na janela do Lina (espelha o que [`run_resume`] já registra).
const COST_CEILING_NOTICE: &str = "⏸ o teto de custo do dia foi atingido — sua mensagem ficou guardada na fila (nada se perde). Diga ao usuário que é preciso confirmar a retomada do teto na janela do Lina para o time voltar a trabalhar.";

/// **PURO** — narração que RETÉM a delegação quando um estado global está pausado, ou `None` quando
/// o Espaço está ativo (segue o fluxo normal de confirmação). É o coração do FIX-4: trocar o "ok,
/// tente de novo" (meia-verdade que faz o agente pedalar) pela verdade INTEIRA, para o agente NARRAR
/// e PARAR. Freio e teto são gates independentes; com os dois ativos, conta os dois.
fn dispatch_pause_notice(state: SpaceState) -> Option<String> {
    match (state.orchestration_paused, state.cost_ceiling_hit) {
        (false, false) => None,
        (true, false) => Some(ORCHESTRATION_PAUSED_NOTICE.to_string()),
        (false, true) => Some(COST_CEILING_NOTICE.to_string()),
        (true, true) => Some(format!(
            "{ORCHESTRATION_PAUSED_NOTICE}\n{COST_CEILING_NOTICE}"
        )),
    }
}

/// **PURO** — a linha de ESTADO GLOBAL que `lina check`/`lina whoami` exibem nos DOIS casos (ativo
/// e pausado): torna visível, em vocabulário leigo, o que antes só aparecia no canvas. O agente que
/// sempre lê "cooperação automática: ativa" reconhece de imediato quando vira "PAUSADA".
fn space_state_line(state: SpaceState) -> String {
    let cooperacao = if state.orchestration_paused {
        "⏸ PAUSADA (delegações ficam guardadas na fila; peça ao usuário ▶ Retomar cooperação)"
    } else {
        "ativa"
    };
    let teto = if state.cost_ceiling_hit {
        "⏸ ATINGIDO (delegações guardadas; precisa de confirmação humana na janela do Lina)"
    } else {
        "ok"
    };
    format!("Estado do Espaço · cooperação automática: {cooperacao} · teto de custo: {teto}")
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

    let input = match load_identity() {
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
    // r4 (costura do fix #12.1): com o freio cobrindo o spawn, um pedido sob pausa enfileira
    // DURÁVEL mas só acontece no resume — mesma verdade-inteira do ask/handoff (FIX-4), senão
    // o agente leria o `Pending` como falha e re-tentaria contra a fila congelada.
    if let Some(notice) = dispatch_pause_notice(space_state()) {
        println!("{notice}");
        return ExitCode::SUCCESS;
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
        Some("add") => run_plan_add(&args[1..]),
        Some("seed") => run_plan_seed(args.get(1)),
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
    let from = match load_identity() {
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

// ── F3-0-5: verbos `lina params set|reset` — parse + gate de autonomia + montagem do envelope ──
// O bin enfileira o contrato {key,scope,value,target?}; o supervisor (`handle_params`) valida a
// faixa (`validate_range`), carimba `by` server-side (ADR 0007) e emite `SystemParamsChanged`.

/// Camadas válidas de um parâmetro (espelha `ParamScope` do core, serializado em `lowercase`).
const PARAM_SCOPES: [&str; 4] = ["global", "workspace", "preset", "terminal"];

/// A mutação parseada de `lina params set|reset` (PURA). `reset` é `set` com `value` vazio: no replay
/// o core cai em `None` → default (`set_from_event`). `target` nomeia o NodeId (scope=terminal) ou o
/// slug (scope=preset); `None` para workspace/global.
#[derive(Debug, PartialEq, Eq)]
struct ParamsMutation {
    key: String,
    scope: String,
    value: String,
    target: Option<String>,
}

/// Parseia `set <key> <value> --scope <s> [--target <t>]` ou `reset <key> --scope <s> [--target <t>]`.
/// Valida só a FORMA (escopo no enum; `target` obrigatório p/ terminal/preset). A faixa do VALOR é do
/// core (`validate_range`, F3-0-5 (iii)) — JAMAIS duplicada aqui (COD-5). `Err` NOMEIA o que faltou.
fn parse_params_mutation(verb: &str, args: &[String]) -> Result<ParamsMutation, String> {
    let mut scope: Option<String> = None;
    let mut target: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scope" => {
                i += 1;
                scope = Some(
                    args.get(i)
                        .ok_or("--scope exige um valor (global|workspace|preset|terminal)")?
                        .clone(),
                );
            }
            "--target" => {
                i += 1;
                target = Some(
                    args.get(i)
                        .ok_or("--target exige um valor (NodeId p/ terminal, slug p/ preset)")?
                        .clone(),
                );
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    let mut positional = positional.into_iter();
    let key = positional.next().ok_or_else(|| {
        format!("lina params {verb} exige a chave do parametro (ex.: fanout_gate)")
    })?;
    let value = if verb == "reset" {
        String::new()
    } else {
        positional.next().ok_or_else(|| {
            format!(
                "lina params {verb} exige o valor (ex.: lina params set {key} 8 --scope workspace)"
            )
        })?
    };

    let scope =
        scope.ok_or("lina params set/reset exige --scope <global|workspace|preset|terminal>")?;
    if !PARAM_SCOPES.contains(&scope.as_str()) {
        return Err(format!(
            "escopo desconhecido: {scope:?} — use um de: {}",
            PARAM_SCOPES.join("|")
        ));
    }
    if matches!(scope.as_str(), "terminal" | "preset") && target.is_none() {
        return Err(format!(
            "scope={scope} exige --target <alvo> (NodeId p/ terminal, slug p/ preset)"
        ));
    }

    Ok(ParamsMutation {
        key,
        scope,
        value,
        target,
    })
}

/// Monta o envelope do contrato (b): intent `params.set`/`params.reset`, payload JSON
/// `{key,scope,value,target?}`, alvo sentinela `"params"` (o supervisor intercepta por INTENT, como em
/// `plan`). O bin NÃO emite o evento nem carimba `by` — só enfileira; `handle_params` (escritor único)
/// valida, carimba `by` server-side (ADR 0007) e emite `SystemParamsChanged`.
fn build_params_envelope(from: &str, intent: &str, m: &ParamsMutation) -> MailMessage {
    let payload = serde_json::json!({
        "key": m.key,
        "scope": m.scope,
        "value": m.value,
        "target": m.target,
    })
    .to_string();
    MailMessage::new(from, "params", intent, payload)
}

/// Gate de autonomia da mutação (espelha `run_spawn`): `manual` recusa AQUI com orientação (o agente
/// PROPÕE ao humano, não altera sozinho); `assistido`/`autonomo` seguem — o propõe->confirma do
/// assistido é a narração do agente, não um passo do CLI. `Some(msg)` = recusa; `None` = pode seguir.
fn params_mutation_gate(autonomy: Autonomy) -> Option<String> {
    matches!(autonomy, Autonomy::Manual).then(|| {
        "no modo MANUAL voce NAO altera parametros do Espaco sozinho — proponha a mudanca ao \
         usuario/Maestro (ou peca para subir a autonomia)."
            .to_string()
    })
}

/// `lina params show | set <chave> <valor> --scope <s> [--target <t>] | reset <chave> --scope <s>`
/// (F3-0-5). `show` projeta o log (SÓ-LEITURA); `set`/`reset` enfileiram o contrato {key,scope,value,
/// target?} — o supervisor (`handle_params`) valida a faixa, carimba `by` server-side e emite o evento.
fn run_params(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("show") => run_params_show(),
        Some(verb @ ("set" | "reset")) => run_params_mutation(verb, &args[1..]),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

/// `lina params set <chave> <valor> --scope <s> [--target <t>]` / `reset <chave> --scope <s> [...]`.
/// Parseia o contrato, aplica o gate de autonomia (`manual` recusa AQUI — o agente PROPÕE ao humano;
/// o router é o backstop durável) e ENFILEIRA o envelope. O bin NÃO valida a faixa nem aplica: quem
/// valida (`validate_range`), carimba `by` server-side e emite `SystemParamsChanged` é o supervisor.
fn run_params_mutation(verb: &str, args: &[String]) -> ExitCode {
    let m = match parse_params_mutation(verb, args) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(2);
        }
    };
    let input = match load_identity() {
        Ok(i) => i,
        Err(e) => {
            eprintln!(
                "lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'/autonomia): {e}"
            );
            return ExitCode::from(1);
        }
    };
    if let Some(refusal) = params_mutation_gate(input.autonomy) {
        eprintln!("lina: {refusal}");
        return ExitCode::from(1);
    }
    let intent = format!("params.{verb}");
    let msg = build_params_envelope(&input.terminal_name, &intent, &m);
    let mailbox = Mailbox::new(mailbox_root());
    match enqueue_per_node(&mailbox, &input.terminal_name, &msg) {
        Ok(()) => {
            let shown = if m.value.is_empty() {
                "(reset)"
            } else {
                m.value.as_str()
            };
            println!(
                "ok: params {verb} {}={shown} (escopo {}) enfileirado (msg {}) — o supervisor valida e aplica",
                m.key, m.scope, msg.id
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
    }
}

/// `lina params show` — SÓ-LEITURA: lê o espelho `log.jsonl` (NUNCA o SQLite — doutrina do verbo
/// read-only, lina.rs:1139), reconstrói as 4 camadas por replay (`ParamsLedger::from_records`,
/// invariante #4) e lista os parâmetros com override + a camada de origem. Sem log ⇒ tudo default.
fn run_params_show() -> ExitCode {
    let content = std::fs::read_to_string(event_log_path()).unwrap_or_default();
    let records = parse_log_records(&content);
    let ledger = ParamsLedger::from_records(&records);
    print!("{}", render_params_show(&ledger));
    ExitCode::SUCCESS
}

/// Renderiza os parâmetros com override + a camada de ORIGEM. Camadas em precedência DECRESCENTE
/// (`terminal` vence): a primeira que opina um parâmetro define o efetivo. Usa serde (genérico) para
/// não duplicar o mapa de 18 chaves do core (COD-5) — campo `None` serializa como `null`.
fn render_params_show(ledger: &ParamsLedger) -> String {
    let layers: [(&str, &SystemParams); 4] = [
        ("terminal", &ledger.terminal),
        ("preset", &ledger.preset),
        ("workspace", &ledger.workspace),
        ("global", &ledger.global),
    ];
    let serialized: Vec<(&str, serde_json::Map<String, serde_json::Value>)> = layers
        .iter()
        .map(|(scope, params)| {
            let map = serde_json::to_value(params)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            (*scope, map)
        })
        .collect();

    // Conjunto canônico de chaves = as de uma camada serializada (todas presentes via serde).
    let keys: Vec<&String> = serialized
        .first()
        .map(|(_, m)| m.keys().collect())
        .unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    for key in keys {
        let origin = serialized
            .iter()
            .find_map(|(scope, map)| map.get(key).filter(|v| !v.is_null()).map(|v| (*scope, v)));
        if let Some((scope, value)) = origin {
            lines.push(format!(
                "  {key} = {}  ·  {scope}",
                render_param_value(value)
            ));
        }
    }

    if lines.is_empty() {
        "Parametros de orquestracao: tudo no default (nenhum override no Espaco).\n".to_string()
    } else {
        let mut out = String::from("Parametros de orquestracao (efetivo · origem):\n");
        out.push_str(&lines.join("\n"));
        out.push('\n');
        out
    }
}

/// Valor do parâmetro sem as aspas JSON de string (`8`, `high`, …).
fn render_param_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ── F3-0-5 Parte 2: verbo `lina effort @T low|medium|high` (contrato aprovado pelo Maestro) ──
// O bin enfileira {target, effort} com intent `effort.assign`; o supervisor (`handle_effort`) resolve
// o alvo, valida o nível, carimba `by`/`origin` server-side, RECUSA auto-atribuição e emite `EffortAssigned`.

/// Níveis de raciocínio do contrato NEUTRO (o mapeamento concreto é do CLI Profile — invariante #3).
const EFFORT_LEVELS: [&str; 3] = ["low", "medium", "high"];

/// O pedido parseado de `lina effort @<Nome> <nível>`.
#[derive(Debug, PartialEq, Eq)]
struct EffortAssignment {
    target: String,
    effort: String,
}

/// Parseia `lina effort @<Nome> <low|medium|high>`. Normaliza o `@` (aceita "QA"/"@QA") e valida o
/// nível no contrato neutro. `Err` NOMEIA o que faltou.
fn parse_effort_args(args: &[String]) -> Result<EffortAssignment, String> {
    let mut positional = args.iter().filter(|a| !a.starts_with("--"));
    let raw_target = positional
        .next()
        .ok_or("lina effort exige o terminal alvo (ex.: lina effort @QA high)")?;
    let effort = positional
        .next()
        .ok_or("lina effort exige o nivel (low|medium|high)")?
        .to_string();
    if !EFFORT_LEVELS.contains(&effort.as_str()) {
        return Err(format!(
            "nivel de effort desconhecido: {effort:?} — use um de: {}",
            EFFORT_LEVELS.join("|")
        ));
    }
    let target = if raw_target.starts_with('@') {
        raw_target.clone()
    } else {
        format!("@{raw_target}")
    };
    Ok(EffortAssignment { target, effort })
}

/// Monta o envelope do contrato aprovado: intent `effort.assign`, payload {target, effort}, alvo
/// sentinela `"effort"` (o supervisor intercepta por INTENT, como em params/plan). O bin NÃO carimba
/// `by`/`origin` — `handle_effort` emite `EffortAssigned{origin:"assigned", by server-side}` (ADR 0007).
fn build_effort_envelope(from: &str, a: &EffortAssignment) -> MailMessage {
    let payload = serde_json::json!({ "target": a.target, "effort": a.effort }).to_string();
    MailMessage::new(from, "effort", "effort.assign", payload)
}

/// `lina effort @<Nome> <low|medium|high>` (F3-0-5 Parte 2). Parseia o pedido, aplica o gate de
/// autonomia (`manual` recusa — o agente PROPÕE ao humano; reusa o gate das mutações de parâmetro,
/// pois `effort` É um parâmetro) e ENFILEIRA o envelope `effort.assign`. O supervisor (`handle_effort`)
/// resolve o alvo, valida, carimba `by`/`origin` server-side, RECUSA auto-atribuição e emite o evento.
fn run_effort(args: &[String]) -> ExitCode {
    let a = match parse_effort_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(2);
        }
    };
    let input = match load_identity() {
        Ok(i) => i,
        Err(e) => {
            eprintln!(
                "lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'/autonomia): {e}"
            );
            return ExitCode::from(1);
        }
    };
    if let Some(refusal) = params_mutation_gate(input.autonomy) {
        eprintln!("lina: {refusal}");
        return ExitCode::from(1);
    }
    let msg = build_effort_envelope(&input.terminal_name, &a);
    let mailbox = Mailbox::new(mailbox_root());
    match enqueue_per_node(&mailbox, &input.terminal_name, &msg) {
        Ok(()) => {
            println!(
                "ok: effort {} de {} enfileirado (msg {}) — o supervisor valida e aplica",
                a.effort, a.target, msg.id
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
    }
}

// ── F3-1-6: superfície CLI da Goal — `lina goal define|interpret|status` + `lina plan add|seed` ──
// O loop da Goal (épico 39): o Lina deixa de executar pedidos e passa a perseguir metas. Estes verbos
// são a BOCA da Goal: `define`/`interpret` e `plan add`/`seed` ENFILEIRAM o intent (o supervisor cunha
// os ids e emite os eventos — `handle_goal`/`handle_plan`, fatia CORE-Goal); `goal status` é LEITURA
// PURA da projeção (estilo `params show`/`retro`). Alvo sentinela "goal"/"plan": o supervisor intercepta
// por INTENT, não por alvo (como params/effort/plan). NENHUM evento nasce aqui — o bin é processo à parte.

/// Critérios de aceite (`--accept`) → `Vec<AcceptanceCriterion>` do contrato: a CLI só dá a `desc`
/// (legível ao leigo); `check_kind` cai no default CONSERVADOR `HumanReview` e `check_arg` em `None`
/// (a CLI não decide COMO verificar — degrada honesto, exige gate). Constrói o tipo REAL do core (acopla
/// em tempo de compilação ao schema que o handler desserializa; sem duplicar o mapa de verificação, COD-5).
fn acceptance_from_descs(descs: &[String]) -> Vec<AcceptanceCriterion> {
    descs
        .iter()
        .map(|desc| AcceptanceCriterion {
            desc: desc.clone(),
            check_kind: CheckKind::default(),
            check_arg: None,
        })
        .collect()
}

/// Quebra um valor `--team A,B,C` / `--parents T1,T2` em itens (vírgula), aparando espaços e
/// descartando vazios (`"A,,B"` → `["A","B"]`). Repetir a flag ESTENDE a lista (acumulável).
fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// `--budget <N>` → `u64`. Erro LEGÍVEL (nunca `unwrap`/`expect`, COD-4) num valor não-numérico.
fn parse_budget(raw: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|_| format!("--budget espera um numero inteiro de tokens, recebi {raw:?}"))
}

/// Gate de autonomia dos WRITERS da Goal (`goal define|interpret`, `plan add|seed`): em `manual` o
/// agente NÃO cria nem altera metas/itens sozinho — PROPÕE ao humano (espelha `params_mutation_gate`/
/// `run_spawn`; o router é o backstop durável). `Some(msg)` = recusa legível; `None` = pode seguir.
fn goal_write_gate(autonomy: Autonomy) -> Option<String> {
    matches!(autonomy, Autonomy::Manual).then(|| {
        "no modo MANUAL voce NAO cria nem altera metas/itens do plano sozinho — proponha a mudanca \
         ao usuario/Maestro (ou peca para subir a autonomia)."
            .to_string()
    })
}

/// Tail comum dos WRITERS da Goal (`goal define|interpret`, `plan add|seed`): carrega a identidade
/// (origem do `from`), aplica o [`goal_write_gate`] (`manual` recusa — o agente PROPÕE) e ENFILEIRA o
/// envelope que `build` monta. Espelha o tail de `run_params_mutation`/`run_effort`; centralizado para
/// não duplicar o I/O (COD-5). `label` nomeia a ação na confirmação ao agente.
fn enqueue_goal_write(label: &str, build: impl FnOnce(&str) -> MailMessage) -> ExitCode {
    let input = match load_identity() {
        Ok(i) => i,
        Err(e) => {
            eprintln!(
                "lina: nao foi possivel ler {INPUT_PATH} (de onde vem o 'from'/autonomia): {e}"
            );
            return ExitCode::from(1);
        }
    };
    if let Some(refusal) = goal_write_gate(input.autonomy) {
        eprintln!("lina: {refusal}");
        return ExitCode::from(1);
    }
    let msg = build(&input.terminal_name);
    let mailbox = Mailbox::new(mailbox_root());
    match enqueue_per_node(&mailbox, &input.terminal_name, &msg) {
        Ok(()) => {
            println!(
                "ok: {label} enfileirado (msg {}) — o supervisor cunha os ids, valida o ciclo e emite os eventos",
                msg.id
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar na mailbox: {e}");
            ExitCode::from(1)
        }
    }
}

/// `lina goal define|interpret|status` (F3-1-6). `define`/`interpret` MUTAM (enfileiram o intent, gate
/// de autonomia); `status` é LEITURA PURA da projeção.
fn run_goal(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("define") => run_goal_define(&args[1..]),
        Some("interpret") => run_goal_interpret(&args[1..]),
        Some("confirm") => run_goal_confirm(args.get(1)),
        Some("status") => run_goal_status(&args[1..]),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

/// O pedido parseado de `lina goal define "<statement>" [--budget N] [--accept "<>"]...` (PURO).
#[derive(Debug, PartialEq, Eq)]
struct GoalDefinition {
    statement: String,
    budget_tokens: Option<u64>,
    acceptance: Vec<String>,
}

/// Parseia `goal define`. `statement` é o único posicional (o pedido bruto da meta); `--budget` é
/// numérico; `--accept` é REPETÍVEL (um critério por ocorrência). `Err` NOMEIA o que faltou/falhou.
fn parse_goal_define(args: &[String]) -> Result<GoalDefinition, String> {
    let mut budget_tokens: Option<u64> = None;
    let mut acceptance: Vec<String> = Vec::new();
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--budget" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or("--budget exige um numero de tokens (ex.: --budget 50000)")?;
                budget_tokens = Some(parse_budget(v)?);
            }
            "--accept" => {
                i += 1;
                acceptance.push(
                    args.get(i)
                        .ok_or(
                            "--accept exige o criterio (ex.: --accept \"a landing abre em <2s\")",
                        )?
                        .clone(),
                );
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    let statement = positional.into_iter().next().ok_or(
        "lina goal define exige o enunciado da meta (ex.: lina goal define \"dobrar os leads em 30 dias\")",
    )?;
    Ok(GoalDefinition {
        statement,
        budget_tokens,
        acceptance,
    })
}

/// Monta o envelope `goal.define`: payload `{statement, budget_tokens?, acceptance:[AcceptanceCriterion]}`,
/// alvo sentinela "goal". O bin NÃO cunha `goal_id` nem decompõe — `handle_goal` (escritor único) cunha o
/// id, carimba `origin`/`root_cause_id` server-side (ADR 0007) e emite `GoalDefined`, que espera a interpretação.
fn build_goal_define_envelope(from: &str, d: &GoalDefinition) -> MailMessage {
    let payload = serde_json::json!({
        "statement": d.statement,
        "budget_tokens": d.budget_tokens,
        "acceptance": acceptance_from_descs(&d.acceptance),
    })
    .to_string();
    MailMessage::new(from, "goal", "goal.define", payload)
}

/// `lina goal define` — parseia e enfileira; erro de forma sai com código 2 (uso).
fn run_goal_define(args: &[String]) -> ExitCode {
    match parse_goal_define(args) {
        Ok(d) => enqueue_goal_write("goal define", |from| build_goal_define_envelope(from, &d)),
        Err(e) => {
            eprintln!("lina: {e}");
            ExitCode::from(2)
        }
    }
}

/// O pedido parseado de `lina goal interpret <goal_id> --understanding "<>" --strategy "<>" [--team ..] [--accept ..]`.
#[derive(Debug, PartialEq, Eq)]
struct GoalInterpretation {
    goal_id: String,
    interpretation: String,
    strategy: String,
    proposed_team: Vec<String>,
    acceptance: Vec<String>,
}

/// Parseia `goal interpret`. `goal_id` é posicional; `--understanding` (vira `interpretation` no evento)
/// e `--strategy` são OBRIGATÓRIOS — o Maestro devolve o entendimento ANTES de executar (doc-fonte 11).
/// `--team` é CSV acumulável; `--accept` é repetível. `Err` NOMEIA a flag faltante.
fn parse_goal_interpret(args: &[String]) -> Result<GoalInterpretation, String> {
    let mut understanding: Option<String> = None;
    let mut strategy: Option<String> = None;
    let mut proposed_team: Vec<String> = Vec::new();
    let mut acceptance: Vec<String> = Vec::new();
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--understanding" => {
                i += 1;
                understanding = Some(
                    args.get(i)
                        .ok_or("--understanding exige o texto do que o Maestro entendeu da meta")?
                        .clone(),
                );
            }
            "--strategy" => {
                i += 1;
                strategy = Some(
                    args.get(i)
                        .ok_or("--strategy exige o texto da estrategia de ataque")?
                        .clone(),
                );
            }
            "--team" => {
                i += 1;
                proposed_team
                    .extend(split_csv(args.get(i).ok_or(
                        "--team exige a lista de papeis/terminais (ex.: --team A,B,C)",
                    )?));
            }
            "--accept" => {
                i += 1;
                acceptance.push(
                    args.get(i)
                        .ok_or("--accept exige o criterio de aceite")?
                        .clone(),
                );
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    let goal_id = positional.into_iter().next().ok_or(
        "lina goal interpret exige o goal_id (ex.: lina goal interpret g-7 --understanding \"...\" --strategy \"...\")",
    )?;
    let interpretation = understanding
        .ok_or("lina goal interpret exige --understanding \"<o que voce entendeu>\"")?;
    let strategy = strategy.ok_or("lina goal interpret exige --strategy \"<como vai atacar>\"")?;
    Ok(GoalInterpretation {
        goal_id,
        interpretation,
        strategy,
        proposed_team,
        acceptance,
    })
}

/// Monta o envelope `goal.interpret`: payload `{goal_id, interpretation, strategy, proposed_team,
/// acceptance:[AcceptanceCriterion]}`, alvo sentinela "goal". `handle_goal` valida o ciclo (só interpreta
/// meta `Defined`), carimba `by` server-side e emite `GoalInterpreted` (que SUGERE — confirmação é gate humano).
fn build_goal_interpret_envelope(from: &str, g: &GoalInterpretation) -> MailMessage {
    let payload = serde_json::json!({
        "goal_id": g.goal_id,
        "interpretation": g.interpretation,
        "strategy": g.strategy,
        "proposed_team": g.proposed_team,
        "acceptance": acceptance_from_descs(&g.acceptance),
    })
    .to_string();
    MailMessage::new(from, "goal", "goal.interpret", payload)
}

/// `lina goal interpret` — parseia e enfileira; erro de forma sai com código 2 (uso).
fn run_goal_interpret(args: &[String]) -> ExitCode {
    match parse_goal_interpret(args) {
        Ok(g) => {
            let label = format!("goal interpret {}", g.goal_id);
            enqueue_goal_write(&label, |from| build_goal_interpret_envelope(from, &g))
        }
        Err(e) => {
            eprintln!("lina: {e}");
            ExitCode::from(2)
        }
    }
}

/// Monta o envelope `goal.confirm`: o GATE HUMANO passou — payload `{goal_id}`, alvo sentinela "goal".
/// `handle_goal` valida o ciclo (só confirma meta `Interpreted`), carimba `by` server-side (ADR 0007 —
/// quem confirma é autoridade, JAMAIS o payload) e emite `GoalConfirmed`, que habilita a decomposição (`plan seed`).
fn build_goal_confirm_envelope(from: &str, goal_id: &str) -> MailMessage {
    let payload = serde_json::json!({ "goal_id": goal_id }).to_string();
    MailMessage::new(from, "goal", "goal.confirm", payload)
}

/// `lina goal confirm <goal_id>` — enfileira a confirmação (gate humano) que libera a decomposição.
/// Reusa o gate/tail dos writers da Goal: em `manual` o agente PROPÕE, não confirma a meta sozinho.
/// Sem `goal_id`, sai com código 2 (uso).
fn run_goal_confirm(goal_id: Option<&String>) -> ExitCode {
    let Some(goal_id) = goal_id else {
        eprintln!("lina: 'goal confirm' exige o goal_id (ex.: lina goal confirm g-7)");
        usage();
        return ExitCode::from(2);
    };
    let label = format!("goal confirm {goal_id}");
    enqueue_goal_write(&label, |from| build_goal_confirm_envelope(from, goal_id))
}

/// `lina goal status <goal_id> [--json]` — LEITURA PURA da projeção `Goal` (replay do log, SÓ-LEITURA,
/// como `params show`/`retro`). `project_goals` (CORE-Goal) varre o log por `goal_id` e reconstrói o
/// ciclo da meta; este verbo renderiza a projeção sem mutar.
fn run_goal_status(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let Some(goal_id) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("lina: 'goal status' exige o goal_id (ex.: lina goal status g-7 [--json])");
        usage();
        return ExitCode::from(2);
    };
    let content = std::fs::read_to_string(event_log_path()).unwrap_or_default();
    let records = parse_log_records(&content);
    let goals = project_goals(&records);
    let found = goals.iter().find(|g| &g.goal_id == goal_id);
    if json {
        print!("{}", render_goal_status_json(goal_id, found));
    } else {
        print!("{}", render_goal_status(goal_id, found));
    }
    ExitCode::SUCCESS
}

/// Rótulo pt-br da fase do ciclo (narra ao leigo, sem o jargão do enum).
fn goal_phase_label(phase: GoalPhase) -> &'static str {
    match phase {
        GoalPhase::Defined => "definida (aguardando interpretacao)",
        GoalPhase::Interpreted => "interpretada (aguardando confirmacao)",
        GoalPhase::Confirmed => "confirmada",
        GoalPhase::Decomposed => "decomposta em itens",
        GoalPhase::InLoop => "em execucao",
        GoalPhase::Achieved => "concluida",
        GoalPhase::Escalated => "escalada ao humano",
    }
}

/// Render legível (leigo) do status. `None` = nenhuma meta com esse id no log. Mostra os campos da
/// projeção (fase/enunciado/entendimento/iteracoes/aceite/itens); budget e vereditos por item entram
/// quando a projeção os reconstruir.
fn render_goal_status(goal_id: &str, goal: Option<&Goal>) -> String {
    let Some(g) = goal else {
        return format!("Meta {goal_id}: nenhuma com esse id no log do Espaco ainda.\n");
    };
    let mut out = format!("Meta {} · {}\n", g.goal_id, goal_phase_label(g.phase));
    out.push_str(&format!("  enunciado: {}\n", g.statement));
    if let Some(interp) = &g.interpretation {
        out.push_str(&format!("  entendimento: {interp}\n"));
    }
    out.push_str(&format!("  iteracoes: {}\n", g.iterations));
    if g.acceptance.is_empty() {
        out.push_str("  criterios de aceite: (nenhum)\n");
    } else {
        out.push_str("  criterios de aceite:\n");
        for c in &g.acceptance {
            out.push_str(&format!("    - {}\n", c.desc));
        }
    }
    if g.items.is_empty() {
        out.push_str("  itens do plano: (nenhum)\n");
    } else {
        out.push_str(&format!("  itens do plano: {}\n", g.items.join(", ")));
    }
    out
}

/// Render `--json` do status (para scripts/UI). Reusa a projeção `Goal` (deriva `Serialize`); `goal`
/// é `null` quando a meta não existe. Forma estável: `{"goal_id":..,"found":bool,"goal":Goal|null}`.
fn render_goal_status_json(goal_id: &str, goal: Option<&Goal>) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "goal_id": goal_id,
            "found": goal.is_some(),
            "goal": goal,
        })
    )
}

/// O pedido parseado de `lina plan add <id> "<desc>" [--goal G] [--parents T1,T2] [--accept ..] [--budget N]`.
#[derive(Debug, PartialEq, Eq)]
struct PlanAddition {
    item: String,
    desc: String,
    goal_id: Option<String>,
    parents: Vec<String>,
    acceptance: Vec<String>,
    budget_tokens: Option<u64>,
}

/// Parseia `plan add`. `item` e `desc` são posicionais (id + descrição); `--goal` liga à meta,
/// `--parents` é CSV acumulável, `--accept` repetível, `--budget` numérico. `Err` NOMEIA o que faltou.
fn parse_plan_add(args: &[String]) -> Result<PlanAddition, String> {
    let mut goal_id: Option<String> = None;
    let mut parents: Vec<String> = Vec::new();
    let mut acceptance: Vec<String> = Vec::new();
    let mut budget_tokens: Option<u64> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--goal" => {
                i += 1;
                goal_id = Some(
                    args.get(i)
                        .ok_or("--goal exige o goal_id que o item serve")?
                        .clone(),
                );
            }
            "--parents" => {
                i += 1;
                parents
                    .extend(split_csv(args.get(i).ok_or(
                        "--parents exige a lista de ids (ex.: --parents T1,T2)",
                    )?));
            }
            "--accept" => {
                i += 1;
                acceptance.push(
                    args.get(i)
                        .ok_or("--accept exige o criterio de aceite")?
                        .clone(),
                );
            }
            "--budget" => {
                i += 1;
                let v = args.get(i).ok_or("--budget exige um numero de tokens")?;
                budget_tokens = Some(parse_budget(v)?);
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }

    let mut positional = positional.into_iter();
    let item = positional
        .next()
        .ok_or("lina plan add exige o id do item (ex.: lina plan add T4 \"montar a API\")")?;
    let desc = positional.next().ok_or(
        "lina plan add exige a descricao do item (ex.: lina plan add T4 \"montar a API de leads\")",
    )?;
    Ok(PlanAddition {
        item,
        desc,
        goal_id,
        parents,
        acceptance,
        budget_tokens,
    })
}

/// Monta o envelope `plan.add`: promove `seed_plan_item` (router, hoje só-teste) a verbo real — payload
/// `{item, desc, goal_id?, parents, acceptance:[AcceptanceCriterion], budget_tokens?}`. `handle_plan`
/// emite `PlanItemAdded` + `PlanItemAttributed` (a atribuição à Goal/parents/aceite, spec 52 §2).
fn build_plan_add_envelope(from: &str, a: &PlanAddition) -> MailMessage {
    let payload = serde_json::json!({
        "item": a.item,
        "desc": a.desc,
        "goal_id": a.goal_id,
        "parents": a.parents,
        "acceptance": acceptance_from_descs(&a.acceptance),
        "budget_tokens": a.budget_tokens,
    })
    .to_string();
    MailMessage::new(from, "plan", "plan.add", payload)
}

/// `lina plan add` — parseia e enfileira (reusa o gate/tail dos writers da Goal); erro de forma → código 2.
fn run_plan_add(args: &[String]) -> ExitCode {
    match parse_plan_add(args) {
        Ok(a) => {
            let label = format!("plan add {}", a.item);
            enqueue_goal_write(&label, |from| build_plan_add_envelope(from, &a))
        }
        Err(e) => {
            eprintln!("lina: {e}");
            ExitCode::from(2)
        }
    }
}

/// Monta o envelope `plan.seed`: decompõe o `GoalInterpreted` CONFIRMADO em itens — payload `{goal_id}`,
/// alvo sentinela "plan". `handle_plan` emite `GoalDecomposed` + N `PlanItemAttributed` (spec 52). Recusar
/// uma meta não-confirmada é do supervisor (validação de ciclo), não do bin.
fn build_plan_seed_envelope(from: &str, goal_id: &str) -> MailMessage {
    let payload = serde_json::json!({ "goal_id": goal_id }).to_string();
    MailMessage::new(from, "plan", "plan.seed", payload)
}

/// `lina plan seed <goal_id>` — enfileira a decomposição (reusa o gate/tail dos writers da Goal). Sem
/// `goal_id`, sai com código 2 (uso).
fn run_plan_seed(goal_id: Option<&String>) -> ExitCode {
    let Some(goal_id) = goal_id else {
        eprintln!("lina: 'plan seed' exige o goal_id (ex.: lina plan seed g-7)");
        usage();
        return ExitCode::from(2);
    };
    let label = format!("plan seed {goal_id}");
    enqueue_goal_write(&label, |from| build_plan_seed_envelope(from, goal_id))
}

#[cfg(test)]
mod f31_goal_surface_tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_string()).collect()
    }

    // ── goal define ──
    #[test]
    fn parse_goal_define_pega_statement_budget_e_aceites_repetidos() {
        let d = parse_goal_define(&argv(&[
            "dobrar os leads",
            "--budget",
            "50000",
            "--accept",
            "abre em <2s",
            "--accept",
            "form envia",
        ]))
        .expect("valido");
        assert_eq!(d.statement, "dobrar os leads");
        assert_eq!(d.budget_tokens, Some(50000));
        assert_eq!(
            d.acceptance,
            vec!["abre em <2s".to_string(), "form envia".to_string()],
            "--accept repetido acumula"
        );
    }

    #[test]
    fn parse_goal_define_sem_statement_falha() {
        let err = parse_goal_define(&argv(&["--budget", "10"])).expect_err("sem enunciado falha");
        assert!(err.contains("enunciado"), "nomeia o que faltou: {err}");
    }

    #[test]
    fn parse_goal_define_budget_nao_numerico_falha() {
        let err = parse_goal_define(&argv(&["meta", "--budget", "muito"]))
            .expect_err("budget invalido falha");
        assert!(err.contains("numero"), "explica o erro de budget: {err}");
    }

    #[test]
    fn goal_define_envelope_enfileira_intent_e_aceites_estruturados() {
        let d = GoalDefinition {
            statement: "meta".into(),
            budget_tokens: Some(7),
            acceptance: vec!["c1".into()],
        };
        let env = build_goal_define_envelope("Terminal I", &d);
        assert_eq!(env.intent, "goal.define");
        assert_eq!(
            env.to, "goal",
            "alvo sentinela 'goal' (intercept por intent)"
        );
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["statement"], "meta");
        assert_eq!(p["budget_tokens"], 7);
        // acceptance vira AcceptanceCriterion: desc + check_kind default HumanReview (degrada honesto).
        assert_eq!(p["acceptance"][0]["desc"], "c1");
        assert_eq!(p["acceptance"][0]["check_kind"], "HumanReview");
        // o bin NUNCA cunha goal_id nem carimba `by` (autoridade do supervisor, ADR 0007).
        assert!(
            !env.payload.contains("goal_id") && !env.payload.contains("\"by\""),
            "o bin nao cunha id nem carimba autoridade: {}",
            env.payload
        );
    }

    // ── goal interpret ──
    #[test]
    fn parse_goal_interpret_exige_goal_id_understanding_strategy_e_aceita_time_csv() {
        let g = parse_goal_interpret(&argv(&[
            "g-7",
            "--understanding",
            "entendi X",
            "--strategy",
            "ataco Y",
            "--team",
            "A,B , C",
            "--accept",
            "criterio",
        ]))
        .expect("valido");
        assert_eq!(g.goal_id, "g-7");
        assert_eq!(
            g.interpretation, "entendi X",
            "--understanding vira interpretation"
        );
        assert_eq!(g.strategy, "ataco Y");
        assert_eq!(
            g.proposed_team,
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            "csv apara espacos e descarta vazios"
        );
        assert_eq!(g.acceptance, vec!["criterio".to_string()]);
    }

    #[test]
    fn parse_goal_interpret_sem_strategy_falha() {
        let err = parse_goal_interpret(&argv(&["g-1", "--understanding", "x"]))
            .expect_err("sem strategy falha");
        assert!(err.contains("strategy"), "nomeia a flag faltante: {err}");
    }

    #[test]
    fn goal_interpret_envelope_carrega_entendimento_e_time() {
        let g = GoalInterpretation {
            goal_id: "g-7".into(),
            interpretation: "u".into(),
            strategy: "s".into(),
            proposed_team: vec!["A".into()],
            acceptance: vec![],
        };
        let env = build_goal_interpret_envelope("Terminal I", &g);
        assert_eq!(env.intent, "goal.interpret");
        assert_eq!(env.to, "goal");
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["goal_id"], "g-7");
        assert_eq!(p["interpretation"], "u");
        assert_eq!(p["strategy"], "s");
        assert_eq!(p["proposed_team"][0], "A");
    }

    // ── goal confirm ──
    #[test]
    fn goal_confirm_envelope_enfileira_intent_so_com_goal_id() {
        let env = build_goal_confirm_envelope("Terminal I", "g-7");
        assert_eq!(env.intent, "goal.confirm");
        assert_eq!(env.to, "goal", "alvo sentinela 'goal'");
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["goal_id"], "g-7");
        // o bin NUNCA carimba `by` — quem confirma é autoridade server-side (ADR 0007).
        assert!(
            !env.payload.contains("\"by\""),
            "o bin nao carimba a autoridade da confirmacao: {}",
            env.payload
        );
    }

    // ── plan add ──
    #[test]
    fn parse_plan_add_pega_id_desc_e_atribuicao() {
        let a = parse_plan_add(&argv(&[
            "T4",
            "montar a API",
            "--goal",
            "g-7",
            "--parents",
            "T1,T2",
            "--budget",
            "1000",
            "--accept",
            "responde 200",
        ]))
        .expect("valido");
        assert_eq!(a.item, "T4");
        assert_eq!(a.desc, "montar a API");
        assert_eq!(a.goal_id, Some("g-7".to_string()));
        assert_eq!(a.parents, vec!["T1".to_string(), "T2".to_string()]);
        assert_eq!(a.budget_tokens, Some(1000));
        assert_eq!(a.acceptance, vec!["responde 200".to_string()]);
    }

    #[test]
    fn parse_plan_add_sem_desc_falha() {
        let err = parse_plan_add(&argv(&["T4"])).expect_err("sem descricao falha");
        assert!(err.contains("descricao"), "nomeia o que faltou: {err}");
    }

    #[test]
    fn plan_add_envelope_enfileira_plan_add_com_atribuicao() {
        let a = PlanAddition {
            item: "T4".into(),
            desc: "d".into(),
            goal_id: Some("g-7".into()),
            parents: vec!["T1".into()],
            acceptance: vec!["c".into()],
            budget_tokens: Some(9),
        };
        let env = build_plan_add_envelope("Terminal I", &a);
        assert_eq!(env.intent, "plan.add");
        assert_eq!(env.to, "plan");
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["item"], "T4");
        assert_eq!(p["desc"], "d");
        assert_eq!(p["goal_id"], "g-7");
        assert_eq!(p["parents"][0], "T1");
        assert_eq!(p["acceptance"][0]["desc"], "c");
        assert_eq!(p["budget_tokens"], 9);
    }

    // ── plan seed ──
    #[test]
    fn plan_seed_envelope_enfileira_plan_seed_do_goal() {
        let env = build_plan_seed_envelope("Terminal I", "g-7");
        assert_eq!(env.intent, "plan.seed");
        assert_eq!(env.to, "plan");
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["goal_id"], "g-7");
    }

    // ── gate de autonomia (writers) ──
    #[test]
    fn goal_write_gate_recusa_em_manual_e_libera_no_resto() {
        assert!(
            goal_write_gate(Autonomy::Manual).is_some(),
            "manual recusa o write — o agente PROPOE"
        );
        assert!(
            goal_write_gate(Autonomy::Assisted).is_none(),
            "assistido segue (propoe->confirma e a narracao do agente)"
        );
        assert!(
            goal_write_gate(Autonomy::Autonomous).is_none(),
            "autonomo segue"
        );
    }

    // ── goal status (leitura pura) ──
    #[test]
    fn goal_status_de_meta_inexistente_e_legivel_e_json() {
        assert!(
            render_goal_status("g-x", None).contains("nenhuma"),
            "legivel: meta ausente"
        );
        let j: serde_json::Value =
            serde_json::from_str(render_goal_status_json("g-x", None).trim()).expect("JSON valido");
        assert_eq!(j["found"], false);
        assert_eq!(j["goal_id"], "g-x");
        assert!(j["goal"].is_null());
    }

    #[test]
    fn goal_status_renderiza_fase_aceite_e_itens() {
        let g = Goal {
            goal_id: "g-7".into(),
            statement: "dobrar leads".into(),
            phase: GoalPhase::Decomposed,
            interpretation: Some("entendi".into()),
            acceptance: vec![AcceptanceCriterion {
                desc: "abre <2s".into(),
                check_kind: CheckKind::default(),
                check_arg: None,
            }],
            items: vec!["T1".into(), "T2".into()],
            iterations: 2,
        };
        let out = render_goal_status("g-7", Some(&g));
        assert!(out.contains("dobrar leads"), "mostra o enunciado: {out}");
        assert!(out.contains("decomposta"), "rotula a fase em pt-br: {out}");
        assert!(out.contains("abre <2s"), "lista os criterios: {out}");
        assert!(out.contains("T1, T2"), "lista os itens: {out}");
        let j: serde_json::Value =
            serde_json::from_str(render_goal_status_json("g-7", Some(&g)).trim())
                .expect("JSON valido");
        assert_eq!(j["found"], true);
        assert_eq!(j["goal"]["iterations"], 2);
    }
}

#[cfg(test)]
mod f305_scaffold_tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn parse_effort_normaliza_arroba_e_valida_nivel() {
        let a = parse_effort_args(&argv(&["QA", "high"])).expect("valido");
        assert_eq!(a.target, "@QA", "normaliza o @ ausente");
        assert_eq!(a.effort, "high");
        let a2 = parse_effort_args(&argv(&["@Dev", "low"])).expect("valido com @");
        assert_eq!(a2.target, "@Dev");
    }

    #[test]
    fn parse_effort_recusa_nivel_invalido() {
        let err =
            parse_effort_args(&argv(&["@QA", "turbo"])).expect_err("nivel fora do contrato falha");
        assert!(err.contains("turbo"), "nomeia o nivel invalido: {err}");
    }

    #[test]
    fn parse_effort_sem_nivel_falha() {
        let err = parse_effort_args(&argv(&["@QA"])).expect_err("sem nivel falha");
        assert!(err.contains("nivel"), "pede o nivel: {err}");
    }

    #[test]
    fn effort_envelope_do_contrato_aprovado() {
        let a = EffortAssignment {
            target: "@QA".to_string(),
            effort: "high".to_string(),
        };
        let env = build_effort_envelope("Terminal I", &a);
        assert_eq!(env.intent, "effort.assign");
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["target"], "@QA");
        assert_eq!(p["effort"], "high");
        assert!(
            env.payload.find("by").is_none(),
            "o bin nunca carimba `by`: {}",
            env.payload
        );
    }

    #[test]
    fn render_show_efetivo_com_origem_e_precedencia() {
        let mut led = ParamsLedger::default();
        led.workspace.fanout_gate = Some(8);
        led.terminal.fanout_gate = Some(12); // terminal vence (precedência mais alta)
        let out = render_params_show(&led);
        assert!(
            out.contains("fanout_gate = 12"),
            "efetivo é o de terminal: {out}"
        );
        assert!(out.contains("terminal"), "origem é terminal: {out}");
        assert!(
            !out.contains("= 8"),
            "o 8 do workspace foi sobreposto, não é efetivo: {out}"
        );
    }

    #[test]
    fn render_show_sem_override_e_tudo_default() {
        let out = render_params_show(&ParamsLedger::default());
        assert!(
            out.to_lowercase().contains("default"),
            "sem override = tudo default: {out}"
        );
    }

    #[test]
    fn parse_set_extrai_key_value_scope() {
        let m = parse_params_mutation("set", &argv(&["fanout_gate", "8", "--scope", "workspace"]))
            .expect("set valido");
        assert_eq!(m.key, "fanout_gate");
        assert_eq!(m.value, "8");
        assert_eq!(m.scope, "workspace");
        assert_eq!(m.target, None);
    }

    #[test]
    fn parse_reset_tem_value_vazio() {
        // reset = set com valor vazio (no replay vira None -> default, via set_from_event do core).
        let m = parse_params_mutation("reset", &argv(&["fanout_gate", "--scope", "workspace"]))
            .expect("reset valido");
        assert_eq!(m.key, "fanout_gate");
        assert_eq!(
            m.value, "",
            "reset limpa o override desta camada (valor vazio)"
        );
    }

    #[test]
    fn parse_recusa_scope_desconhecido() {
        let err = parse_params_mutation("set", &argv(&["fanout_gate", "8", "--scope", "galaxy"]))
            .expect_err("scope fora do enum deve falhar");
        assert!(
            err.contains("galaxy"),
            "o erro nomeia o escopo invalido: {err}"
        );
    }

    #[test]
    fn parse_terminal_exige_target() {
        let err = parse_params_mutation("set", &argv(&["fanout_gate", "8", "--scope", "terminal"]))
            .expect_err("scope=terminal sem --target deve falhar");
        assert!(err.contains("target"), "o erro pede o alvo: {err}");
    }

    #[test]
    fn parse_set_sem_scope_falha() {
        let err = parse_params_mutation("set", &argv(&["fanout_gate", "8"]))
            .expect_err("set sem --scope deve falhar");
        assert!(err.contains("scope"), "o erro pede o escopo: {err}");
    }

    #[test]
    fn envelope_carrega_intent_e_payload_do_contrato_b() {
        let m = ParamsMutation {
            key: "fanout_gate".to_string(),
            scope: "workspace".to_string(),
            value: "8".to_string(),
            target: None,
        };
        let env = build_params_envelope("Terminal I", "params.set", &m);
        assert_eq!(env.intent, "params.set");
        // Contrato (b): payload JSON {key,scope,value,target?}. `by` é carimbo server-side do
        // supervisor (handle_params), NUNCA do bin — não viaja aqui.
        let p: serde_json::Value = serde_json::from_str(&env.payload).expect("payload é JSON");
        assert_eq!(p["key"], "fanout_gate");
        assert_eq!(p["scope"], "workspace");
        assert_eq!(p["value"], "8");
        assert!(
            env.payload.find("by").is_none(),
            "o bin nunca carimba `by`: {}",
            env.payload
        );
    }

    #[test]
    fn gate_manual_recusa_demais_seguem() {
        assert!(
            params_mutation_gate(Autonomy::Manual).is_some(),
            "manual recusa localmente (o agente PROPÕE ao humano, não altera sozinho)"
        );
        assert!(
            params_mutation_gate(Autonomy::Assisted).is_none(),
            "assistido segue no CLI (o propõe->confirma é a narração do agente, como no spawn)"
        );
        assert!(
            params_mutation_gate(Autonomy::Autonomous).is_none(),
            "autônomo segue"
        );
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
            // FIX-2: guard genérico rodando no PTY do agente → carimba o NOME do nó (env do spawn).
            // Fora de um terminal admitido (ex.: testes) o env é ausente → `None` (sem item na fila).
            node: env_node_name(),
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
    let autonomy = autonomy_from_env();
    let mut raw = String::new();
    let result = if let Err(e) = std::io::stdin().read_to_string(&mut raw) {
        // stdin ilegível → fail-safe `ask` (decisão volta ao humano), diagnóstico em stderr.
        eprintln!("lina: falha ao ler stdin do PreToolUse: {e}");
        pretooluse_result("", &autonomy)
    } else {
        pretooluse_result(&raw, &autonomy)
    };
    // O JSON da decisão SEMPRE sai no stdout — o hook fala pelo conteúdo do JSON, não pelo exit code
    // (fail-safe do guard intacto, mesmo se o append abaixo falhar).
    println!("{}", result.json);
    // FIX-2 (dogfood): um `ask` sobre Bash BLOQUEIA o agente, mas o detector F1-1-6 não pegava o
    // dialog de hook-ask (formato ≠ nativo; em bypass o nativo nem existe). Apenda
    // `ActionGated{decision:"ask", node:LINA_NODE_NAME}` para a fila de atenção alertar+focar. O
    // append NUNCA pode quebrar o hook: o JSON já foi emitido; erro de store vai só ao stderr.
    if let Some(gated) = result.gated_ask {
        append_guard_ask(&gated);
    }
    ExitCode::SUCCESS
}

/// FIX-2: apenda `ActionGated{decision:"ask"}` carimbando o NOME do nó (env do spawn) para a fila de
/// atenção. Reusa o `events_dir`/`EventStore::open` do `run_check_action`. Falha SÓ no stderr — o
/// hook já emitiu a decisão no stdout, então o guard nunca trava o agente por um problema de log.
fn append_guard_ask(gated: &GatedAsk) {
    let mut store = match EventStore::open(events_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lina: ActionGated(ask) nao logado (store indisponivel): {e}");
            return;
        }
    };
    let event = DomainEvent::ActionGated {
        cmd: gated.cmd.clone(),
        class: gated.class.clone(),
        decision: "ask".to_string(),
        node: env_node_name(),
    };
    if let Err(e) = store.append(&event) {
        eprintln!("lina: ActionGated(ask) nao logado: {e}");
    }
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
    // Red-team r1 do ADR 0026: em verbo CUSTODIADO, ficha ausente NÃO cai no env (que o processo
    // do agente pode exportar) — degrada para "agente-desconhecido", como sempre. O env só entra
    // via `load_identity()` (ficha presente), onde a autoridade final segue sendo o dir-dono.
    let from = load_identity()
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
    // Red-team r1 do ADR 0026: idem `run_resume` — verbo custodiado não toma identidade do env
    // com ficha ausente; degrada para "agente-desconhecido" (falha visível > identidade exportável).
    let requester = load_identity()
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
        // Custódia (`lina do`): já vira item `Custody` na fila — não carimba o nó (evita GuardAsk
        // duplicado, FIX-2).
        node: None,
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
    let input = match load_identity() {
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
            .map(|a| handshake_colleague_entry(&a.name, a.role.as_deref()))
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

/// **BUG-3 (dogfood r1) — paridade de papel nas superfícies.** A linha de colega do
/// `handshake` exibe o papel CANÔNICO ([`canonical_role`]) — a MESMA forma do `whoami`
/// (colegas) e do `lina list`. Papel ausente → "—" (como antes).
fn handshake_colleague_entry(name: &str, role: Option<&str>) -> String {
    format!(
        "{name} ({})",
        role.map(canonical_role).unwrap_or_else(|| "—".into())
    )
}

/// **BUG-3 — a linha do `lina list` (modo texto)**: papel CANÔNICO, igual às demais
/// superfícies. O `--json` segue CRU (espelho fiel do `agents.json` — rastreabilidade).
fn list_agent_entry(name: &str, role: Option<&str>, status: &str) -> String {
    format!(
        "{name} · {} · {status}",
        role.map(canonical_role).unwrap_or_else(|| "—".into())
    )
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
                    "{}",
                    list_agent_entry(&a.name, a.role.as_deref(), &a.status)
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
mod identity_tests {
    use super::*;

    /// **ADR 0026 (BUG-1) — resolução de nome:** env do spawn VENCE a ficha; env
    /// ausente/vazio/whitespace cai na ficha (controle). Remover a preferência de env
    /// faz o primeiro assert falhar (não-vacuoso).
    #[test]
    fn env_name_wins_over_ficha_and_empty_env_falls_back() {
        assert_eq!(
            resolved_name(Some("Bug Finder"), "Automatizador"),
            "Bug Finder",
            "env do spawn (autoridade do app) vence a ficha por-cwd"
        );
        assert_eq!(
            resolved_name(None, "Automatizador"),
            "Automatizador",
            "controle: sem env, a ficha continua mandando (terminal puro)"
        );
        assert_eq!(
            resolved_name(Some(""), "Automatizador"),
            "Automatizador",
            "env vazio não apaga a identidade da ficha"
        );
        assert_eq!(
            resolved_name(Some("   "), "Automatizador"),
            "Automatizador",
            "env whitespace idem"
        );
        assert_eq!(
            resolved_name(Some("  Bug Finder  "), "Automatizador"),
            "Bug Finder",
            "o nome do env chega aparado"
        );
    }

    /// **#17 residual (ADR 0026 / FIX-3) — autonomia env-first:** o env `LINA_AUTONOMY` (injetado
    /// POR-NÓ pelo app) VENCE a ficha do cwd compartilhado; ausente/desconhecido cai na ficha.
    /// Remover a preferência de env faz o 1º assert falhar (não-vacuoso). Aceita o rótulo pt-br do
    /// `Autonomy::label()` (o que o app injeta) e as formas serde en.
    #[test]
    fn env_autonomy_wins_over_ficha_and_unknown_falls_back() {
        assert_eq!(
            resolved_autonomy(Some("manual"), Autonomy::Autonomous),
            Autonomy::Manual,
            "o env por-nó (autoridade do app) vence a ficha sobrescrita por um colega de cwd"
        );
        assert_eq!(
            resolved_autonomy(Some("autonomo"), Autonomy::Manual),
            Autonomy::Autonomous,
            "rótulo pt-br do label() que o app injeta"
        );
        assert_eq!(
            resolved_autonomy(Some("  assisted  "), Autonomy::Manual),
            Autonomy::Assisted,
            "forma serde en, aparada"
        );
        assert_eq!(
            resolved_autonomy(None, Autonomy::Manual),
            Autonomy::Manual,
            "sem env → a ficha manda (compat standalone)"
        );
        assert_eq!(
            resolved_autonomy(Some("xyz"), Autonomy::Assisted),
            Autonomy::Assisted,
            "rótulo desconhecido NÃO inventa nível — cai na ficha"
        );
        assert_eq!(
            resolved_autonomy(Some(""), Autonomy::Autonomous),
            Autonomy::Autonomous,
            "env vazio não apaga a autonomia da ficha"
        );
    }

    /// **ADR 0026 — outbox POR-NÓ usa o nome RESOLVIDO:** com env divergente da ficha, a
    /// mensagem deposita no subdir do NOME DO ENV (e nada no da ficha); controle sem env →
    /// subdir da ficha. É o caminho real do `lina ask` (`resolved_name` → `enqueue_per_node`).
    #[test]
    fn outbox_per_node_uses_resolved_name() {
        let root = std::env::temp_dir().join(format!(
            "lina-adr0026-outbox-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mailbox = Mailbox::new(&root);

        // Env setado e ficha divergente → subdir do env.
        let from = resolved_name(Some("Bug Finder"), "Automatizador");
        let msg = MailMessage::new(&from, "@QA", "ask", "oi");
        enqueue_per_node(&mailbox, &from, &msg).expect("enqueue com nome do env");
        let env_dir = root.join("outbox").join("Bug Finder");
        let ficha_dir = root.join("outbox").join("Automatizador");
        assert_eq!(
            std::fs::read_dir(&env_dir).map(Iterator::count).ok(),
            Some(1),
            "a mensagem mora no subdir do NOME DO ENV"
        );
        assert!(
            !ficha_dir.exists(),
            "nada vaza para o subdir do nome da ficha sobrescrita"
        );

        // Controle: sem env → subdir da ficha (compat).
        let from = resolved_name(None, "Automatizador");
        let msg = MailMessage::new(&from, "@QA", "ask", "oi de novo");
        enqueue_per_node(&mailbox, &from, &msg).expect("enqueue compat");
        assert_eq!(
            std::fs::read_dir(&ficha_dir).map(Iterator::count).ok(),
            Some(1),
            "sem env, o subdir da ficha continua sendo o canal"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **ADR 0026 — compat preservado:** ficha AUSENTE continua `Err` com a mensagem de
    /// orientação (nenhuma identidade inventada); ficha presente lê o nome dela.
    #[test]
    fn missing_ficha_still_errors_with_orientation() {
        let dir = std::env::temp_dir().join(format!(
            "lina-adr0026-ficha-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let missing = load_input_at(&dir.join("bootstrap.json"));
        assert!(
            missing.is_err(),
            "ficha ausente → erro/orientação (jamais default silencioso)"
        );

        let path = dir.join("ok.json");
        std::fs::write(
            &path,
            r#"{"terminal_name":"Automatizador","roster":["Automatizador"],"vault_path":"/v","autonomy":"assisted"}"#,
        )
        .expect("ficha");
        let input = load_input_at(&path).expect("ficha válida parseia");
        assert_eq!(input.terminal_name, "Automatizador");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **BUG-3 (dogfood r1) — PARIDADE:** para cada papel do roster de teste, whoami
    /// (colegas) × handshake × list exibem o MESMO papel canônico — inclusive o `reviewer`
    /// minúsculo do `agents.json` (→ REVIEWER) e um papel desconhecido (fica CRU, jamais
    /// vira DEVELOPER só numa superfície).
    #[test]
    fn role_parity_across_whoami_handshake_and_list() {
        let roster_roles: Vec<(String, String)> = vec![
            ("Automatizador".into(), "AUTOMATOR".into()),
            ("Revisor".into(), "reviewer".into()),
            ("Especialista em Telas".into(), "FRONTEND".into()),
            ("Visionária".into(), "PAPEL_NOVO".into()),
        ];
        let mut roster: Vec<String> = roster_roles.iter().map(|(n, _)| n.clone()).collect();
        roster.push("Bug Finder".into());
        let input = BootstrapInput::new("Bug Finder", roster, "/tmp/v", Autonomy::Assisted);
        let bs = Bootstrapper::new().expect("registry");
        let who = bs.whoami_with_roles(&input, &roster_roles);

        for (name, raw) in &roster_roles {
            let canon = canonical_role(raw);
            let token = format!("{name} ({canon})");
            assert!(
                who.contains(&token),
                "whoami deve exibir {token:?} (papel real canônico): {who}"
            );
            assert_eq!(
                handshake_colleague_entry(name, Some(raw)),
                token,
                "handshake exibe o MESMO papel canônico"
            );
            assert!(
                list_agent_entry(name, Some(raw), "Ready")
                    .starts_with(&format!("{name} · {canon} ·")),
                "list exibe o MESMO papel canônico"
            );
        }
        // O caso da tela: "Automatizador" jamais aparece DEVELOPER em superfície alguma.
        assert!(!who.contains("Automatizador (DEVELOPER)"), "{who}");
    }
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

    /// `scan_log_outcome` (núcleo da confirmação de `lina ask`/`handoff`): lê o desfecho real do
    /// `log.jsonl`. **#22c — "entregue" exige `MessageDelivered`** (injeção física); `MessageRouted`
    /// sozinho (pré-injeção) vira no máximo `Routed`. Formato verbatim do espelho.
    #[test]
    fn scan_log_outcome_requires_delivered_for_entregue() {
        let blocked = r#"{"seq":3,"ts":1,"kind":"RouteBlocked","version":1,"payload":{"event":"RouteBlocked","id":"msg_X","reason":"unknown_sender","from":"Terminal B","to":"@Terminal C"}}"#;
        let routed = r#"{"seq":4,"ts":2,"kind":"MessageRouted","version":1,"payload":{"event":"MessageRouted","id":"msg_Y","from":"Terminal B","to":"@Terminal C","to_node":"019e-uuid","intent":"ask","hops":0,"root_cause_id":"msg_Y"}}"#;
        let delivered = r#"{"seq":5,"ts":3,"kind":"MessageDelivered","version":1,"payload":{"event":"MessageDelivered","id":"msg_D","from":"Terminal B","to":"@Terminal C","to_node":"019e-uuid"}}"#;

        // Bloqueada → reporta o motivo.
        assert_eq!(
            scan_log_outcome(blocked, "msg_X"),
            Some(RouteConfirm::Blocked {
                reason: "unknown_sender".into()
            })
        );
        // SÓ `MessageRouted` (pré-injeção) → ROTEADA, jamais entregue (mata o falso-entregue #22c).
        assert_eq!(
            scan_log_outcome(routed, "msg_Y"),
            Some(RouteConfirm::Routed {
                to_node: "019e-uuid".into()
            }),
            "MessageRouted sozinho nunca vira Delivered — a injeção física ainda não ocorreu"
        );
        // `MessageDelivered` → ENTREGUE de fato (reporta o nó destino).
        assert_eq!(
            scan_log_outcome(delivered, "msg_D"),
            Some(RouteConfirm::Delivered {
                to_node: "019e-uuid".into()
            })
        );
        // Roteada + entregue (mesmo id) → Delivered vence (a injeção foi confirmada depois).
        let routed_then_delivered = format!(
            "{}\n{}",
            routed.replace("msg_Y", "msg_Z"),
            delivered.replace("msg_D", "msg_Z")
        );
        assert_eq!(
            scan_log_outcome(&routed_then_delivered, "msg_Z"),
            Some(RouteConfirm::Delivered {
                to_node: "019e-uuid".into()
            })
        );
        // Bloqueada + roteada (mesmo id, re-tentada) → Routed vence o bloqueio, mas NÃO é Delivered.
        let blocked_then_routed = format!(
            "{}\n{}",
            blocked.replace("msg_X", "msg_W"),
            routed.replace("msg_Y", "msg_W")
        );
        assert_eq!(
            scan_log_outcome(&blocked_then_routed, "msg_W"),
            Some(RouteConfirm::Routed {
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

#[cfg(test)]
mod check_resolution_tests {
    use super::*;

    fn rename(node: &str, name: &str) -> String {
        format!(
            r#"{{"seq":1,"ts":1,"kind":"NodeRenamed","version":1,"payload":{{"event":"NodeRenamed","node":"{node}","name":"{name}"}}}}"#
        )
    }
    fn status(node: &str, st: &str) -> String {
        format!(
            r#"{{"seq":1,"ts":1,"kind":"NodeStatusChanged","version":1,"payload":{{"event":"NodeStatusChanged","node":"{node}","status":"{st}","from":"Running","reason":"t"}}}}"#
        )
    }
    fn exited(node: &str) -> String {
        format!(
            r#"{{"seq":1,"ts":1,"kind":"TerminalExited","version":1,"payload":{{"event":"TerminalExited","node":"{node}"}}}}"#
        )
    }
    fn removed(node: &str) -> String {
        format!(
            r#"{{"seq":1,"ts":1,"kind":"NodeRemoved","version":1,"payload":{{"event":"NodeRemoved","node":"{node}"}}}}"#
        )
    }

    /// **r4 achado #13 (dogfooding 2026-06-11):** nome REUSADO entre sessões resolvia para o
    /// lifecycle MORTO da sessão antiga — `lina check "@Bug Finder"` apontava o nó velho. O
    /// spawn batiza com sigil (`@Bug Finder`), que o match exato antigo nem casava. A resolução
    /// deve ser tolerante a `@`/caixa e preferir o nó VIVO mais recentemente batizado.
    #[test]
    fn reused_name_resolves_to_live_node_not_dead_homonym() {
        let log = [
            rename("n-velho", "Bug Finder"),
            status("n-velho", "Dead"),
            rename("n-novo", "@Bug Finder"), // spawn batiza COM sigil (run_spawn normaliza p/ @Nome)
            status("n-novo", "Idle"),
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&log, "Bug Finder").as_deref(),
            Some("n-novo"),
            "homônimo vivo vence o morto da sessão antiga (e o sigil não quebra o match)"
        );
    }

    /// O nome ATUAL de um nó é o último batismo: quem foi renomeado PARA OUTRO nome deixa de
    /// responder pelo antigo (senão um check de nome reciclado acharia o dono anterior).
    #[test]
    fn renamed_away_node_no_longer_answers_for_old_name() {
        let log = [
            rename("n1", "QA"),
            status("n1", "Idle"),
            rename("n1", "Revisor"), // n1 agora é outro — "QA" ficou órfão
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&log, "QA"),
            None,
            "nome abandonado não resolve mais para o ex-dono"
        );
        assert_eq!(
            resolve_check_node(&log, "Revisor").as_deref(),
            Some("n1"),
            "o nome novo resolve"
        );
    }

    /// Sem homônimo vivo, o ÚLTIMO batizado vence (exibição honesta do morto — não é erro);
    /// nome desconhecido → `None`. Linha-lixo tolerada (arquivo sob append).
    #[test]
    fn all_dead_falls_back_to_last_claimant_and_unknown_is_none() {
        let log = [
            rename("n1", "Bug Finder"),
            status("n1", "Dead"),
            "{lixo parcial".to_string(),
            rename("n2", "@bug finder"), // caixa diferente: mesmo nome normalizado
            status("n2", "Dead"),
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&log, "Bug Finder").as_deref(),
            Some("n2"),
            "todos mortos → o último a reivindicar o nome (status honesto)"
        );
        assert_eq!(resolve_check_node(&log, "Ninguem"), None);
        assert_eq!(resolve_check_node("", "Bug Finder"), None);
    }

    /// **#4/#14/#23c:** a morte de um nó também é registrada por `TerminalExited`/`NodeRemoved`,
    /// NÃO só por `NodeStatusChanged(Dead)`. Um nó que SAIU com último status "Idle" não pode
    /// vencer o homônimo vivo — antes vencia (o `check` apontava o lifecycle da sessão antiga).
    #[test]
    fn exited_or_removed_node_is_dead_even_with_idle_status() {
        // n-velho: status "Idle", mas SAIU (`TerminalExited`) → morto. n-novo vivo → vence.
        let log_exit = [
            rename("n-velho", "QA"),
            status("n-velho", "Idle"),
            exited("n-velho"),
            rename("n-novo", "@QA"),
            status("n-novo", "Idle"),
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&log_exit, "QA").as_deref(),
            Some("n-novo"),
            "TerminalExited mata o nó mesmo com último NodeStatusChanged = Idle"
        );
        // n-velho REMOVIDO (`NodeRemoved`) → morto, mesmo batizado DEPOIS e com status "Idle".
        let log_remove = [
            rename("n-novo", "QA"),
            status("n-novo", "Idle"),
            rename("n-velho", "@QA"), // batizado depois (mais recente)
            status("n-velho", "Idle"),
            removed("n-velho"),
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&log_remove, "QA").as_deref(),
            Some("n-novo"),
            "NodeRemoved derruba o homônimo mais recente: o vivo provado vence"
        );
    }

    /// **#4/#14 — "status ausente não é vivo por default quando há homônimo COM status":** entre um
    /// homônimo provado vivo (com status) e outro sem nenhum sinal, o COM-status vence — mesmo se o
    /// sem-sinal foi batizado depois. (Um MORTO, porém, perde até para o sem-sinal.)
    #[test]
    fn status_present_beats_statusless_homonym() {
        let alive_vs_unknown = [
            rename("n-vivo", "QA"),
            status("n-vivo", "Busy"),
            rename("n-sem", "@QA"), // batizado depois, mas SEM status algum
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&alive_vs_unknown, "QA").as_deref(),
            Some("n-vivo"),
            "o homônimo com status provado vence o sem-sinal mais recente"
        );
        // Controle: o sem-sinal (desconhecido) ainda vence um MORTO mais recente.
        let dead_vs_unknown = [
            rename("n-sem", "QA"), // sem status
            rename("n-morto", "@QA"),
            status("n-morto", "Dead"),
        ]
        .join("\n");
        assert_eq!(
            resolve_check_node(&dead_vs_unknown, "QA").as_deref(),
            Some("n-sem"),
            "desconhecido (não provado morto) vence o homônimo provado morto"
        );
    }
}

#[cfg(test)]
mod space_state_tests {
    use super::*;

    /// Linha de log mínima (só `kind` decide a projeção — o payload do freio é vazio).
    fn rec(kind: &str) -> String {
        format!(r#"{{"seq":1,"ts":1,"kind":"{kind}","version":1,"payload":{{"event":"{kind}"}}}}"#)
    }

    /// **`scan_space_state` (puro):** cada flag segue o ÚLTIMO evento de transição (último vence —
    /// idêntico ao replay de `restore_orchestration_state`/`CostLedger`). Controles provam a não-
    /// vacuosidade: vazio → default; Paused→Resumed volta a ativo; freio e teto são independentes.
    #[test]
    fn scan_space_state_tracks_last_transition_per_flag() {
        // Vazio → nada pausado (ausência de freio = ativo).
        assert_eq!(scan_space_state(""), SpaceState::default());

        // Freio: último vence.
        let paused = scan_space_state(&rec("OrchestrationPaused"));
        assert!(paused.orchestration_paused && !paused.cost_ceiling_hit);
        let resumed = scan_space_state(&format!(
            "{}\n{}",
            rec("OrchestrationPaused"),
            rec("OrchestrationResumed")
        ));
        assert!(
            !resumed.orchestration_paused,
            "Resumed depois de Paused volta a ativo (último vence)"
        );

        // Teto: independente do freio, mesmo padrão de transição.
        let hit = scan_space_state(&rec("CostCeilingHit"));
        assert!(hit.cost_ceiling_hit && !hit.orchestration_paused);
        let both = scan_space_state(&format!(
            "{}\n{}",
            rec("OrchestrationPaused"),
            rec("CostCeilingHit")
        ));
        assert!(
            both.orchestration_paused && both.cost_ceiling_hit,
            "freio e teto são gates independentes"
        );
        let cost_resumed = scan_space_state(&format!(
            "{}\n{}",
            rec("CostCeilingHit"),
            rec("CostCeilingResumed")
        ));
        assert!(!cost_resumed.cost_ceiling_hit);

        // Linha-lixo é tolerada (arquivo sob append).
        assert_eq!(scan_space_state("{lixo parcial\n"), SpaceState::default());
    }

    /// **`dispatch_pause_notice` (puro):** ativo → `None` (segue o fluxo normal); freio → a verdade
    /// do freio SEM "tente de novo"; teto → a verdade do teto; ambos → as duas. É o coração do FIX-4.
    #[test]
    fn dispatch_pause_notice_speaks_the_whole_truth() {
        assert_eq!(
            dispatch_pause_notice(SpaceState::default()),
            None,
            "Espaço ativo não retém — fluxo normal de confirmação"
        );

        let brake = dispatch_pause_notice(SpaceState {
            orchestration_paused: true,
            cost_ceiling_hit: false,
        })
        .expect("freio narra a verdade");
        assert!(
            brake.contains("PAUSADO") && brake.contains("▶ Retomar cooperação"),
            "{brake}"
        );
        assert!(
            !brake.contains("tente de novo"),
            "a verdade do freio NÃO manda o agente re-tentar (era a meia-verdade que fazia pedalar): {brake}"
        );

        let cost = dispatch_pause_notice(SpaceState {
            orchestration_paused: false,
            cost_ceiling_hit: true,
        })
        .expect("teto narra a verdade");
        assert!(cost.contains("teto de custo"), "{cost}");

        let both = dispatch_pause_notice(SpaceState {
            orchestration_paused: true,
            cost_ceiling_hit: true,
        })
        .expect("os dois narram");
        assert!(
            both.contains("Retomar cooperação") && both.contains("teto de custo"),
            "ambos os gates são contados quando os dois estão ativos: {both}"
        );
    }

    /// **`space_state_line` (puro):** aparece nos DOIS casos com o vocabulário certo. Controle:
    /// o ativo diz "ativa" e NÃO diz "PAUSADA" (não-vacuoso).
    #[test]
    fn space_state_line_renders_both_cases() {
        let ativo = space_state_line(SpaceState::default());
        assert!(
            ativo.contains("cooperação automática: ativa") && ativo.contains("teto de custo: ok"),
            "{ativo}"
        );
        assert!(
            !ativo.contains("PAUSADA"),
            "controle: ativo não diz PAUSADA: {ativo}"
        );

        let pausado = space_state_line(SpaceState {
            orchestration_paused: true,
            cost_ceiling_hit: true,
        });
        assert!(
            pausado.contains("PAUSADA") && pausado.contains("ATINGIDO"),
            "{pausado}"
        );
    }
}

#[cfg(test)]
mod history_verb_tests {
    use super::*;
    use lina_core::history;

    /// Linha de log mínima (só `kind` + `payload.node`/`status` decidem a projeção de membros).
    fn ev_status(node: NodeId, status: &str) -> String {
        format!(
            r#"{{"kind":"NodeStatusChanged","payload":{{"node":"{node}","status":"{status}"}}}}"#
        )
    }
    fn ev_exited(node: NodeId) -> String {
        format!(r#"{{"kind":"TerminalExited","payload":{{"node":"{node}"}}}}"#)
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "lina-history-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("mkdir tmp");
        p
    }

    /// `live_member_ids`: vivos são membros; morto (`TerminalExited`/`NodeRemoved`/status Dead) NÃO é.
    #[test]
    fn live_members_excludes_dead_and_removed() {
        let a = NodeId::from_u128(1);
        let b = NodeId::from_u128(2);
        let dead = NodeId::from_u128(3);
        let removed = NodeId::from_u128(4);
        let removed_ev = format!(r#"{{"kind":"NodeRemoved","payload":{{"node":"{removed}"}}}}"#);
        let content = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            ev_status(a, "Idle"),
            ev_status(b, "Busy"),
            ev_status(dead, "Idle"), // estava vivo…
            ev_exited(dead),         // …mas saiu → morto
            removed_ev,
        );
        let members = live_member_ids(&content);
        assert!(
            members.contains(&a) && members.contains(&b),
            "vivos sao membros"
        );
        assert!(!members.contains(&dead), "TerminalExited tira da fronteira");
        assert!(!members.contains(&removed), "NodeRemoved tira da fronteira");
    }

    /// **Gate (f) do #15:** um MEMBRO lê a tela do colega (devolve a saída); um leitor FORA da
    /// fronteira de pertencimento é BARRADO (default-deny, ADR 0006). Integra a derivação de membros
    /// (bin) com o gate cross (core) e o painel chaveado por NodeId (como o app escreve).
    #[test]
    fn belonging_member_reads_screen_outsider_is_barred() {
        use lina_core::scrollback::ScrollbackStore;

        let owner = NodeId::from_u128(1); // o colega cuja tela queremos ver
        let reader = NodeId::from_u128(2); // o Maestro (membro)
        let content = format!(
            "{}\n{}\n",
            ev_status(owner, "Idle"),
            ev_status(reader, "Busy")
        );
        let members = live_member_ids(&content);

        let dir = tmp("gate");
        let panel = owner.to_string(); // app chaveia o scrollback pelo NodeId
        let mut store = ScrollbackStore::open_default(&dir).expect("scrollback");
        store
            .push_line(&panel, "worker: rodando os testes".to_string())
            .expect("push");
        store
            .push_line(&panel, "worker: tudo verde".to_string())
            .expect("push");
        store.flush_all().expect("flush");
        let mut events = EventStore::open(dir.join("events")).expect("event store");
        let limits = HistoryLimits::default();

        // MEMBRO (reader) lê o painel do owner → a "tela" volta.
        let page = history::tail_cross(
            &mut events,
            &members,
            reader,
            owner,
            &store,
            &panel,
            Some(10),
            0,
            &limits,
        )
        .expect("membro le a tela do colega");
        let shown = render_tail(&page, false, "worker");
        assert!(
            shown.contains("worker: tudo verde"),
            "a tela do colega aparece: {shown}"
        );

        // FORA da fronteira (leitor não-membro) → BARRADO.
        let outsider = NodeId::from_u128(99);
        let denied = history::tail_cross(
            &mut events,
            &members,
            outsider,
            owner,
            &store,
            &panel,
            Some(10),
            0,
            &limits,
        );
        assert!(
            denied.is_err(),
            "leitor fora do Espaco e barrado (default-deny)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `render_tail`: linhas legíveis (default), `--json` (contrato F1), expirado e vazio honestos.
    #[test]
    fn render_tail_modes() {
        let page = HistoryPage {
            panel: "p".into(),
            start: 40,
            lines: vec!["ola".into(), "mundo".into()],
            next_cursor: Some(7),
            expired_before: 0,
            expired: false,
        };
        let txt = render_tail(&page, false, "worker");
        assert!(txt.contains("ola") && txt.contains("mundo"));
        assert!(txt.contains("--offset 7"), "paginação sinalizada");
        let json = render_tail(&page, true, "worker");
        assert!(
            json.contains("\"start\":40") && json.contains("\"lines\""),
            "json do contrato"
        );

        let expirado = HistoryPage {
            panel: "p".into(),
            start: 0,
            lines: vec![],
            next_cursor: None,
            expired_before: 5,
            expired: true,
        };
        assert!(
            render_tail(&expirado, false, "w").contains("expirado"),
            "expirado é honesto"
        );
    }

    /// `flag_value`: lê `--nome <valor>`; ausente → None.
    #[test]
    fn flag_value_reads_pairs() {
        let args: Vec<String> = ["@w", "--tail", "30", "--json"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(flag_value(&args, "--tail").as_deref(), Some("30"));
        assert_eq!(flag_value(&args, "--offset"), None);
        assert_eq!(
            flag_value(&args, "--json"),
            None,
            "flag booleana não tem valor"
        );
    }
}
