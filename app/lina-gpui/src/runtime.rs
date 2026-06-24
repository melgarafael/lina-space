//! M8 fatia (i) — **runtime por-Espaço** (fundação do switch vivo F1-4-4).
//!
//! Extração PURA do boot de workspace do `main.rs` (passos [2]-[17] do mapa de boot) para um
//! construtor reutilizável: [`boot_ws_runtime`] monta TUDO que é por-Espaço ([`WsRuntime`]) a
//! partir do que é por-PROCESSO ([`SharedInfra`]). Com UM runtime o comportamento é idêntico ao
//! boot inline anterior; a fatia (ii) fará o processo hospedar N runtimes e o switch A↔B <1s com
//! PIDs preservados (decisão do fundador: Espaços de fundo VIVOS) — a tela só re-aponta.
//!
//! Doutrina (proposta runtime-por-Espaço + veredito do Arquiteto, 2026-06-11):
//! - **Isolamento por OBJETO DISTINTO** (F1-4-1): `Supervisor`/`MailboxPump`/store POR Espaço —
//!   não há tagging nem filtro; cruzar barramentos é impossível por construção.
//! - **Dreno por-runtime** (veredito §1, obrigatório nesta fatia): o `spawn_pump` consome
//!   `delta_rx`+`bus_rx` e alimenta `meter.record_output` (teto de custo) e `watch.note_output`
//!   (Busy/Idle R2b). Cada `WsRuntime` carrega o SEU pump VIVO — nada de pump global; um Espaço
//!   de fundo sem dreno seria memória sem teto (mpsc unbounded) + custo/idle cegos.
//! - **Handles vivos**: os pumps moram DENTRO do struct (`_pump`/`_mailbox_pump`/`_broker_pump`)
//!   e nunca são dropados enquanto o runtime existir — ciclo de vida explícito (inv do CLAUDE.md).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use lina_bootstrap::Autonomy;
use lina_cli_profiles::ProfileRegistry;
use lina_core::{CliProfile, EventStore, Mailbox, PtyManager, Supervisor};
use lina_host::NodeId;
// W3-6c (ADR 0004): cofre de segredos (demo: backend em memória `MockStore`).
use lina_secrets::{MockStore, SecretVault};

use crate::bridge::{
    self, load_injection_profile, lock, scrub_foreign_orchestrator_env, scrub_pty_secret_env,
    spawn_pump, ApprovalWiring, AttentionHub, BootstrapWriter, BrokerPump, CmdFactory, CoreInput,
    CustodyDesk, Desk, GpuiBridgeHost, Grid, HooksShared, MailboxPump, Model, NodeManager, Pump,
    SharedModel,
};
use crate::{agent_modal, dashboard, obsidian, persistence_ui, wiring, workspace_boot};

/// Infra POR-PROCESSO: o que os N runtimes compartilham. `PtyManager` é dono dos processos do
/// SO (um só por app); a fábrica de shell e as dimensões do grid valem para todo Espaço. O tema
/// também é global, mas é APLICADO no `main` (antes de qualquer janela) — fica fora daqui.
#[derive(Clone)]
pub struct SharedInfra {
    pub pty: Arc<Mutex<PtyManager>>,
    pub cmd_factory: CmdFactory,
    pub cols: u16,
    pub rows: u16,
}

/// TUDO que é por-Espaço — o que a view consome hoje e o switch (fatia ii) vai re-apontar.
/// Os campos `_pump`/`_mailbox_pump`/`_broker_pump` são handles VIVOS: threads do Espaço que
/// continuam drenando/observando mesmo com o Espaço fora de cena (decisão "fundos VIVOS").
pub struct WsRuntime {
    pub ws_root: PathBuf,
    /// `<ws_root>/.lina` — mailbox A2A + event log + perfis, o "endereço" do Espaço.
    pub mailbox_dir: PathBuf,
    pub store: Arc<Mutex<EventStore>>,
    /// SEAM-3: bus escopado — UM `Supervisor` por Espaço; roster/presença/A2A nunca cruzam.
    pub sup: Arc<Supervisor>,
    /// SEAM-1: o funil ÚNICO de admissão de nós (⌘T/⌘N/seed/SpawnApproved — ADR 0022 §4).
    pub nodes: Arc<NodeManager>,
    pub model: Model,
    pub grids: Arc<Mutex<BTreeMap<NodeId, Grid>>>,
    pub input: Arc<CoreInput>,
    pub attention: Arc<AttentionHub>,
    pub desk: Desk,
    /// W4-3: freio por Espaço (pausa num Espaço não pausa outro — spec M8 §escopo).
    pub brake: wiring::Brake,
    pub autonomy: Autonomy,
    /// F1-0-2: o perfil de injeção REAL do boot (fallback dos alvos sem `profile_id`).
    pub injection_profile: CliProfile,
    pub profile_registry: Arc<ProfileRegistry>,
    /// M8 fatia (i) — listener de hooks DO runtime (1 por Espaço, porta efêmera própria);
    /// `None` = perfil sem capability `hooks` ou bind falhou (degradação limpa, como antes).
    #[allow(dead_code)] // handle de POSSE (mantém o listener vivo) — como os `_pump`s.
    pub hooks: Option<Arc<HooksShared>>,
    /// F4-WA-1: a engine de webhook ativo do Espaço (handle de POSSE — mantém o runtime tokio +
    /// listener vivos). `None` = perfil sem webhook ou bind falhou. Como `hooks`, vive pelo runtime.
    #[allow(dead_code)]
    pub webhook: Option<Arc<bridge::WebhookShared>>,
    /// `Arc` (fatia ii): a view re-aponta para o dash do runtime ATIVO no switch — o wiring
    /// continua sendo alimentado pelas threads do runtime mesmo com o Espaço de fundo.
    pub dash: Arc<dashboard::DashWiring>,
    /// Dreno por-runtime (veredito §1): consome delta/bus e alimenta meter+watch — VIVO sempre.
    pub _pump: Pump,
    pub _mailbox_pump: JoinHandle<()>,
    pub _broker_pump: JoinHandle<()>,
}

/// F1-4-4 fatia (ii) — o processo hospeda N runtimes: raiz→runtime + qual está ATIVO.
/// Compartilhado entre a view (troca/leitura do ativo) e o epílogo do `main` (parada limpa).
pub struct RuntimeMap {
    pub map: BTreeMap<PathBuf, WsRuntime>,
    pub active: PathBuf,
}

pub type Runtimes = Arc<Mutex<RuntimeMap>>;

/// Embrulha o runtime do boot como o ATIVO inicial do processo.
#[must_use]
pub fn runtimes_with_boot(rt: WsRuntime) -> Runtimes {
    let active = rt.ws_root.clone();
    let mut map = BTreeMap::new();
    map.insert(active.clone(), rt);
    Arc::new(Mutex::new(RuntimeMap { map, active }))
}

/// **O switch vivo do M8.** Garante o runtime do alvo MONTADO (boot sob demanda + restore
/// F1-4-3 dele, dentro do `boot_ws_runtime`) e o marca ATIVO. Os PIDs do Espaço que sai de
/// cena NÃO são tocados (decisão do fundador: fundos VIVOS — pumps/PTYs seguem). Durável:
/// apenda `WorkspaceFocusSet` no log do ALVO (F1-4-4 crit 1) + carimba o foco no ponteiro
/// global (`focus_target_workspace` — o PRÓXIMO boot abre o alvo). `Err` NÃO troca nada —
/// o Espaço atual segue na tela (inv#6; a sidebar mostra a linha-⚠ do alvo).
pub fn activate_workspace(
    rts: &Runtimes,
    target_root: PathBuf,
    shared: &SharedInfra,
) -> Result<(), String> {
    let mut r = lock(rts);
    if r.active == target_root && r.map.contains_key(&target_root) {
        return Ok(());
    }
    if !r.map.contains_key(&target_root) {
        let rt = boot_ws_runtime(target_root.clone(), shared, false)?;
        r.map.insert(target_root.clone(), rt);
    }
    // Log do ALVO = fonte da verdade da troca (projeção `focused_workspace` daquele Espaço).
    if let Some(rt) = r.map.get(&target_root) {
        let name = lock(&rt.store)
            .project()
            .ok()
            .and_then(|p| p.workspace_name)
            .unwrap_or_else(|| {
                target_root
                    .file_name()
                    .map_or_else(|| "Meu Espaço".into(), |n| n.to_string_lossy().into_owned())
            });
        if let Err(e) =
            lock(&rt.store).append(&lina_core::DomainEvent::WorkspaceFocusSet { workspace: name })
        {
            eprintln!("lina-gpui: [WS] troca sem WorkspaceFocusSet no log do alvo ({e}); seguindo");
        }
        // r5 park-seam: Espaço descarregado volta ao foco → religa os estacionados
        // ("religam ao focar", promessa do Descarregar). Idempotente (parked_pending);
        // runtime recém-bootado religa pelo restore de boot e cai aqui como no-op.
        let relit = relight_unloaded(rt);
        if relit > 0 {
            eprintln!("lina-gpui: [PARK] {relit} terminal(is) religado(s) no foco");
        }
    }
    // Carimbo durável no ponteiro global — best-effort (inv#6): falha loga e a troca segue.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    match lina_core::default_registry_path().map(lina_core::WorkspaceRegistry::load) {
        Some(Ok(mut reg)) => {
            if let Err(e) = workspace_boot::focus_target_workspace(&target_root, &mut reg, now_ms) {
                eprintln!(
                    "lina-gpui: [WS] carimbo de foco no ponteiro global falhou ({e}); seguindo"
                );
            }
        }
        _ => eprintln!("lina-gpui: [WS] ponteiro global indisponível na troca; seguindo"),
    }
    r.active = target_root;
    Ok(())
}

// ───────────────── r5 park-seam · Descarregar Espaço de fundo + religar no foco ─────────────────

/// Resultado do Descarregar — o rail narra a partir daqui ("X desligados; Y seguem trabalhando").
#[allow(dead_code)] // consumido pelo botão do rail (costura do C) — padrão `RestoreBadge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnloadOutcome {
    /// Terminais ociosos desligados (PTY morto; card/nó preservados).
    pub parked: usize,
    /// Vivos preservados (Busy/Blocked/em atividade — "sem interferir", restrição do fundador).
    pub kept: usize,
}

/// **Callback público do botão «Descarregar Espaço» (rail/sidebar — o C fia).** Desliga os
/// PTYs OCIOSOS de um Espaço de FUNDO sem remover nada (religam no próximo foco, via
/// [`activate_workspace`]). Recusa o Espaço ATIVO (descarregar o que está na tela não faz
/// sentido e mataria a sessão em uso). Espaço não-montado → `Ok(0/0)` (já não consome nada).
///
/// # Errors
/// Alvo é o Espaço em foco.
#[allow(dead_code)] // consumido pelo botão do rail (costura do C) — padrão `plan_restore`.
pub fn unload_workspace(rts: &Runtimes, target_root: &Path) -> Result<UnloadOutcome, String> {
    let r = lock(rts);
    if r.active == target_root {
        return Err("este Espaço está em foco — troque para outro antes de descarregá-lo".into());
    }
    let Some(rt) = r.map.get(target_root) else {
        return Ok(UnloadOutcome { parked: 0, kept: 0 });
    };
    let plan = lina_core::suspend::unload_plan(&rt.sup.list());
    let parked = rt.nodes.park_terminals(&plan.park);
    Ok(UnloadOutcome {
        parked,
        kept: plan.keep.len(),
    })
}

/// **PURO — ids estacionados que AINDA pendem religação:** união dos `parked` de todos os
/// `WorkspaceUnloaded` do log, filtrada a quem segue `Dead` NA PROJEÇÃO. O supersede do
/// restore (`NodeRemoved`) os tira da projeção → religar é idempotente por construção.
fn parked_pending(
    recs: &[lina_core::EventRecord],
    proj: &lina_core::ProjectedState,
) -> std::collections::BTreeSet<NodeId> {
    let mut out = std::collections::BTreeSet::new();
    for r in recs {
        if r.kind != "WorkspaceUnloaded" {
            continue;
        }
        let Some(parked) = r
            .payload
            .get("parked")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for v in parked {
            let Some(id) = v.as_str().and_then(|s| s.parse::<NodeId>().ok()) else {
                continue;
            };
            let dead = proj
                .nodes
                .get(&id)
                .is_some_and(|n| n.status.as_deref() == Some("Dead"));
            if dead {
                out.insert(id);
            }
        }
    }
    out
}

/// Religa os terminais estacionados de um runtime JÁ MONTADO (o caso "Espaço de fundo
/// descarregado voltou ao foco") — reusa o fluxo F1-4-3 inteiro: `plan_restore` (resgate de
/// órfão-nomeado) filtrado aos estacionados + `restore_terminals` (re-ergue, badge honesto,
/// supersede com `NodeRemoved`). Devolve quantos religou. (Runtime NOVO não passa aqui: o
/// restore de boot dentro de `boot_ws_runtime` já resgata os estacionados do log.)
fn relight_unloaded(rt: &WsRuntime) -> usize {
    let (recs, proj) = {
        let s = lock(&rt.store);
        match (s.events(), s.project()) {
            (Ok(r), Ok(p)) => (r, p),
            _ => return 0, // log ilegível → não religa às cegas (próximo foco re-tenta)
        }
    };
    let pending = parked_pending(&recs, &proj);
    if pending.is_empty() {
        return 0;
    }
    let mut plans: Vec<bridge::RestoredTerminal> =
        bridge::plan_restore(&proj, &rt.profile_registry, rt.nodes.scrollback().as_ref())
            .into_iter()
            .filter(|p| pending.contains(&p.node))
            .collect();
    // ADR 0037: re-lançar o CLI por caminho absoluto (o PATH pode estar pobre no boot via Dock).
    crate::absolutize_restore_commands(&mut plans);
    rt.nodes.restore_terminals(&plans)
}

/// Epílogo do processo: para o dreno de TODOS os runtimes (o `main` chama após `app.run`).
pub fn stop_all(rts: &Runtimes) {
    let mut r = lock(rts);
    for rt in r.map.values_mut() {
        rt._pump.stop();
    }
}

/// R2b: `LINA_ATTENTION_DEMO` setado = teatro determinístico do fundador — o loop
/// VIVO de detecção desliga (demo OU real, nunca os dois; sem duplicar pedidos).
/// Lido em DOIS pontos (aqui: desliga a detecção viva do pump; `main`: semeia o pedido do
/// teatro DEPOIS do boot) — helper único para as duas leituras usarem o MESMO parse.
#[must_use]
pub fn attention_demo_kind() -> Option<String> {
    std::env::var("LINA_ATTENTION_DEMO")
        .ok()
        .map(|v| v.trim().to_lowercase())
        .filter(|k| !k.is_empty() && k != "0")
}

/// Monta UM Espaço vivo a partir de `ws_root` — passos [2]-[17] do boot, sem mudança de
/// comportamento (refactor puro da fatia i). O `main` resolve QUAL Espaço abre (registry/ponteiro)
/// e opera demo/load-gen/janela SOBRE o runtime retornado. `Err` = falha que antes encerrava o
/// processo (store de fallback morto, ponte core→UI não sobe) — quem decide encerrar é o chamador
/// (inv#6: mensagem clara, nunca backtrace; um runtime de FUNDO falhando não pode matar o app).
pub fn boot_ws_runtime(
    ws_root: PathBuf,
    shared: &SharedInfra,
    demo: bool,
) -> Result<WsRuntime, String> {
    let mailbox_dir = ws_root.join(".lina");
    let dir = mailbox_dir.join("events");
    // F1-0-2 critério 5: o perfil de injeção REAL (claude-code.toml — ready_timeout/busy_markers
    // calibrados) substitui o demo hardcoded; fallback-com-warning no loader (nunca panic). O
    // caminho carregado + os campos do fix saem AQUI no log de boot, p/ inspeção.
    let injection_profile = load_injection_profile(Some(&mailbox_dir));
    // A2A UNIVERSAL: registry de TODOS os CLI Profiles (claude-code, codex, gemini, antigravity —
    // semeados write-if-absent no dir do usuário). A entrega A2A resolve o profile do ALVO por aqui
    // (pelo `profile_id` do nó, projetado de `CliProfileSet`), com `injection_profile` como FALLBACK.
    // Sem isto, a entrega usava o prompt_ready/busy do Claude p/ TODO alvo → a TUI do Codex (glifo
    // `›`) nunca casava → timeout (msg não injetada). Compartilhado (Arc) com o ⚡ demo e o pump.
    let profile_registry = Arc::new(agent_modal::load_profiles(&agent_modal::profiles_dir(
        &mailbox_dir,
    )));
    eprintln!(
        "lina-gpui: workspace em {} ({})",
        ws_root.display(),
        if demo {
            "DEMO/temp"
        } else {
            "produção/persistente"
        }
    );
    // inv#6 (app NUNCA quebrar): um `.db` corrompido NÃO panica o boot. Tenta o store persistente; se
    // falhar, loga ALTO sem destruir o arquivo (recuperação in-UI é W0-6, fora de escopo) e cai num
    // store temporário só nesta sessão — garante que o app abre. Instalação nova abre limpo, sem cair
    // aqui. (`.expect` antigo virava backtrace na cara do aluno.)
    let store = Arc::new(Mutex::new(match EventStore::open(&dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "lina-gpui: não abri seus dados em {} ({e}). Eles estão preservados; abrindo um \
                 espaço temporário só nesta sessão.",
                dir.display()
            );
            let tmp = std::env::temp_dir()
                .join("lina-space-fallback")
                .join(".lina")
                .join("events");
            match EventStore::open(&tmp) {
                Ok(s) => s,
                Err(e2) => {
                    return Err(format!(
                        "o store de fallback também falhou em {} ({e2})",
                        tmp.display()
                    ));
                }
            }
        }
    }));

    // F1-4-3: foto da geração ANTERIOR — capturada ANTES do fechamento abaixo, então quem
    // estava vivo no último quit ainda NÃO está `dead` nela (nós `dead` NÃO saem da projeção —
    // só `NodeRemoved` remove; o `plan_restore` PULA os `dead` de gerações mais antigas e o
    // executor aposenta cada re-erguido com `NodeRemoved`). É a fonte do plano de restore
    // (posições/nomes/papéis/cwd/CLI do log — inv#4); o opt-out por Espaço decide o uso adiante.
    let restore_proj = lock(&store).project().ok();

    // F1-0-8 (costura coordenada — mecanismo do Dev 02): fecha a GERAÇÃO ANTERIOR no log logo
    // após abrir o store e ANTES de registrar os nós da sessão nova — sem esta linha o mecanismo
    // existe mas não roda no caminho real (mesmo padrão da fiação W5-2). Nós da sessão passada
    // sem morte registrada viram `dead` póstumo (replay = roster vivo, 0 fantasmas — gate F1-0
    // (e)). Best-effort: falha loga ALTO e o boot segue (inv#6 — o app nunca morre no boot).
    match lina_core::lifecycle::close_previous_generation(&mut lock(&store)) {
        Ok(ghosts) => eprintln!(
            "lina-gpui: F1-0-8 — geração anterior fechada no boot: {} fantasma(s)",
            ghosts.len()
        ),
        Err(e) => eprintln!(
            "lina-gpui: F1-0-8 — não fechei a geração anterior ({e}); seguindo (replay pode mostrar fantasmas)"
        ),
    }

    // F1-4-1 (fiação app): auto-inscrição do Espaço aberto no ponteiro global + carimbo de
    // foco — é assim que o registry se popula (a lista do switcher M8 nasce daqui) e que o
    // PRÓXIMO boot sabe qual Espaço abrir. Best-effort: falha loga e o boot segue (inv#6).
    if !demo {
        match lina_core::default_registry_path()
            .map(lina_core::WorkspaceRegistry::load)
            .transpose()
        {
            Ok(Some(mut reg)) => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                match workspace_boot::register_boot_workspace(
                    &mut lock(&store),
                    &ws_root,
                    &mut reg,
                    now_ms,
                ) {
                    Ok(id) => eprintln!(
                        "lina-gpui: [WS] Espaço '{id}' inscrito e focado no ponteiro global"
                    ),
                    Err(e) => eprintln!(
                        "lina-gpui: [WS] auto-inscrição no ponteiro global falhou ({e}); seguindo"
                    ),
                }
            }
            Ok(None) => {
                eprintln!("lina-gpui: [WS] sem HOME/USERPROFILE — boot segue sem o ponteiro global")
            }
            Err(e) => {
                eprintln!("lina-gpui: [WS] ponteiro global ilegível ({e}); seguindo sem ele");
            }
        }
    }

    let sup = Arc::new(Supervisor::new());
    let bus_rx = sup.subscribe();
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();

    // W3-2 · BOOTSTRAP turno-0: o app escreve `<cwd>/CLAUDE.md` (8 blocos) por terminal e o
    // reescreve a cada mudança de roster. `ws_root`/`mailbox_dir` já foram definidos acima (o event
    // store do app mora em `<ws_root>/.lina/events`, o MESMO log do bin); vault/autonomia/bin `lina`
    // configuráveis por env (LINA_VAULT/LINA_BIN). O bootstrap é best-effort (não trava o app).
    // W3-4: a mailbox compartilhada do workspace (`<ws_root>/.lina`). `LINA_HOME` aponta os
    // terminais (que herdam o env) para cá → `lina ask`/`handshake` depositam no MESMO outbox que
    // o supervisor (o `MailboxPump`) observa, e o bin `lina do/guard` apenda no MESMO event log.
    if let Err(e) = std::fs::create_dir_all(&mailbox_dir) {
        // Não fatal (o resto do app — canvas, terminais — funciona sem A2A), mas VISÍVEL: sem a
        // mailbox, `lina ask` não terá para onde escrever.
        eprintln!(
            "lina-gpui: não criou a mailbox {} (A2A indisponível): {e}",
            mailbox_dir.display()
        );
    }
    // F1-4-4 fatia (ii) — LINA_HOME é POR SPAWN (`node_identity_env`, veredito §3): cada
    // terminal nasce apontando para o `.lina` do SEU Espaço. O `set_var` global foi REMOVIDO:
    // auditoria 2026-06-11 = zero leitores in-process no app (únicas ocorrências eram este
    // setter e uma allowlist de teste), e com N runtimes o global apontaria sempre para o
    // último Espaço bootado — spawns do Espaço A nasceriam com o `.lina` de B.
    // O segundo cérebro (onboarding) grava o vault escolhido em `.lina/vault.json`. Se houver um
    // `primary`, ele tem prioridade — reflete a escolha REAL do usuário e faz `{{vault_writable_paths}}`/
    // `{{vault_tino_path}}` apontarem pro vault linkado na próxima reescrita do bootstrap. Senão, cai no
    // override de dev `LINA_VAULT` e, por fim, no default `<ws_root>/vault`.
    let vault_path = obsidian::read_primary_vault(&mailbox_dir)
        .or_else(|| std::env::var("LINA_VAULT").ok())
        .unwrap_or_else(|| ws_root.join("vault").display().to_string());
    // Self-heal do segundo cérebro: se o vault está conectado (vault.json) mas o índice (PageIndex)
    // sumiu — a thread fire-and-forget do onboarding morreu, ou a 1ª execução foi negada no TCC de
    // Documentos — regenera-o agora, no contexto do app (já com o grant de Documentos do bundle).
    // No-op quando os índices já existem. Sem isso, o usuário não-técnico fica "conectado mas vazio".
    obsidian::heal_missing_indices(&mailbox_dir);
    let lina_bin = std::env::var("LINA_BIN").unwrap_or_else(|_| "lina".to_string());
    // SEAM-1 (M2): FONTE ÚNICA da autonomia do workspace — passada AO MESMO TEMPO ao `BootstrapWriter`
    // (doutrina do bin) E ao `RouterConfig` da `MailboxPump` (enforcement do gate). `LINA_AUTONOMY`
    // (default `assistido`, sem regressão); origem futura = `bootstrap.json`/workspace.
    // FIX-3: a MESMA função alimenta a UI (`WorkspaceView.ws_autonomy` — badge do card + prefill
    // do modal), para a superfície nunca divergir do enforcement.
    let autonomy = crate::workspace_autonomy();
    let mut bootstrap = BootstrapWriter::new(ws_root.clone(), vault_path, autonomy, lina_bin)
        .map_err(|e| eprintln!("lina-gpui: bootstrap desativado: {e}"))
        .ok();

    // ── F1-1-3/F1-1-5 (wiring) · HOOKS + TIMELINE + SESSÕES — antes de qualquer write/spawn. ──
    // Listener 1x no boot; cada terminal ganha token no settings SE o perfil declarar a
    // capability (TOML — inv#3). Degradação limpa: sem capability/bind → app como antes.
    // M8 fatia (i) — DESVIO do veredito §2 (registrado na entrega): em vez de listener GLOBAL +
    // token `{ws_id}/{name}`, o listener nasce DENTRO do runtime (1 POR Espaço, porta efêmera
    // própria; os kits do Espaço apontam pra porta do SEU runtime) — isolamento por OBJETO
    // DISTINTO (princípio da proposta §2); colisão de nome entre Espaços fica impossível por
    // construção. Com 1 runtime, comportamento idêntico ao listener único de antes.
    let dash = Arc::new(dashboard::DashWiring::new(
        injection_profile.id.clone(),
        ws_root.display().to_string(),
    ));
    let hooks = if injection_profile.capabilities.has("hooks") {
        if let Some(hooks) = HooksShared::start() {
            if let Some(bw) = bootstrap.as_mut() {
                bw.set_hooks(Arc::clone(&hooks));
            }
            let tl = Arc::clone(&dash.timeline);
            hooks.spawn_drain(move |ev| {
                // Sonda [DASH]: latência de INGESTÃO (chegada no listener → timeline).
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                eprintln!(
                    "lina-gpui: [DASH] hook node={} kind={:?} tool={:?} lat_ingest_ms={}",
                    ev.node_id,
                    ev.kind,
                    ev.tool_name,
                    now.saturating_sub(ev.ts)
                );
                if let Ok(mut t) = tl.lock() {
                    t.note(ev);
                }
            });
            Some(hooks)
        } else {
            None
        }
    } else {
        eprintln!(
            "lina-gpui: [DASH] perfil '{}' sem capability hooks — settings sem hooks (como antes)",
            injection_profile.id
        );
        None
    };
    // Sessões (camada 3 — custo~): r5 perf-ws — o runtime se INSCREVE no hub ÚNICO do
    // processo (antes: 1 thread + 1 SessionWatch ZERADO por Espaço = N cold-parses do HOME
    // inteiro + N caminhadas/stat a cada 500ms — a regressão de perf do multi-workspace).
    // O pattern segue vindo do TOML do perfil (F1-1-1); HOME real do usuário (não é teste).
    if let Some(pattern) = injection_profile.session_dir_pattern.clone() {
        subscribe_sessions(&injection_profile.id, &pattern, &dash.sessions);
    }

    // ── W3-6c (ADR 0004) · COFRE DE SEGREDO + ENV LIMPO (fios 1 e 3) — ANTES de spawnar qualquer PTY.
    // Cofre do workspace. DEMO: backend em memória (`MockStore`) p/ não tocar o keyring do SO no
    // teatro do fundador; PRODUÇÃO trocaria por `SecretVault::new("lina-space/<ws>")` (KeyringStore).
    // Semente OPCIONAL via `LINA_DEPLOY_TOKEN`: com ela, a custódia EXECUTA (segredo do cofre); sem
    // ela, o cofre fica vazio e a custódia BLOQUEIA (DeniedNoSecret) — ambos provam a blindagem.
    let custody_vault = SecretVault::with_store("lina-space/walking-skeleton", MockStore::new());
    if let Ok(token) = std::env::var("LINA_DEPLOY_TOKEN") {
        match custody_vault.set("deploy", "prod", &token) {
            Ok(()) => {
                eprintln!("lina-gpui: cofre semeado (deploy/prod) a partir de LINA_DEPLOY_TOKEN")
            }
            Err(e) => eprintln!("lina-gpui: nao semeou o cofre (deploy/prod): {e}"),
        }
    }
    // Fio 3: os PTYs filhos herdam o env do app no spawn → remover as vars de segredo do PRÓPRIO env
    // AQUI (antes de qualquer `wire_terminal`) garante que nenhum terminal do agente nasça com o token.
    let scrubbed = scrub_pty_secret_env();
    if !scrubbed.is_empty() {
        eprintln!("lina-gpui: env limpo — vars de segredo removidas do env do app (PTYs nao as herdam): {scrubbed:?}");
    }
    // R2c (bug de tela 21:40): se o Lina.app foi lançado de um terminal hospedado no
    // Maestri, `MAESTRI_*` vaza no env → a skill GLOBAL do Maestri vira "funcional" p/ o
    // agente do ⌘N (maestri list exit 127; "$MAESTRI_CLI" exit 126 — a var EXISTE). Scrub
    // ANTES de qualquer `wire_terminal`: nenhum agente nasce enxergando plumbing de fora
    // do Espaço (a comunicação entre terminais é EXCLUSIVA pelos verbos `lina`).
    let foreign = scrub_foreign_orchestrator_env();
    if !foreign.is_empty() {
        eprintln!(
            "lina-gpui: env limpo — vars de orquestrador estrangeiro removidas (PTYs nao as herdam; comunicacao so via `lina`): {foreign:?}"
        );
    }
    // Mesa de custódia compartilhada: a UI confirma (⌘⏎); a BrokerPump executa COM o segredo do cofre.
    let desk: Desk = Arc::new(Mutex::new(CustodyDesk::default()));
    // W4-3: FREIO compartilhado — a UI pede pausa/retoma; a MailboxPump aplica no Router e espelha.
    let brake: wiring::Brake = wiring::new_brake();

    let model: Model = Arc::new(Mutex::new(SharedModel::default()));
    let grids: Arc<Mutex<BTreeMap<NodeId, Grid>>> = Arc::new(Mutex::new(BTreeMap::new()));

    // Rodada 360 (ADR 0022): o NodeManager — dono dos nós em runtime E o FUNIL ÚNICO de
    // admissão — nasce ANTES do seed do demo, para as TRÊS portas (⌘T, ⌘N e o seed) entrarem
    // pelo MESMO `admit_node`. `seq_start = 0`: o demo admite A/B pelo funil (seq 0/1 → keys
    // t0/t1); em produção o 1º ⌘T do aluno nasce como "Terminal A".
    // SEAM-1: `Arc` — compartilhado com a `MailboxPump`/`AttentionHub` (SpawnApproved/banner criam
    // pelo MESMO funil `admit_node`, ADR 0022). O gpui View detém um clone; as pumps, outro.
    let nodes = Arc::new(NodeManager::new(
        Arc::clone(&shared.pty),
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&model),
        Arc::clone(&grids),
        BTreeMap::new(),
        delta_tx,
        shared.cols,
        shared.rows,
        0,
        Arc::clone(&shared.cmd_factory),
        bootstrap,
        mailbox_dir.clone(), // W4-2 M3/M4: `.lina/` onde notas/pastas persistem
    ));

    // F4-WA-1: engine de webhook ativo (POST externo → input no terminal vivo). Runtime tokio próprio
    // (molde HooksShared): bind loopback + serve; deposita a entrega-de-sistema na fila que o pump drena.
    let webhook =
        bridge::WebhookShared::start(Arc::clone(&store), Arc::clone(&sup), nodes.system_deliveries());

    // F1-4-3 · RESTORE do quit limpo: reabrir o app = o Espaço volta vivo (posições/nomes/
    // papéis do log + scrollback re-hidratado + resume do CLI quando o TOML declara + badge
    // honesto). Opt-out por Espaço (`restore_on_open` no settings.json — «Abrir este Espaço
    // sem religar os Agentes») e por env (`LINA_NO_RESTORE=1`, escape de emergência). Fora do
    // demo (o demo semeia A/B fixos) e best-effort: falha degrada com log, nunca trava o boot.
    if !demo
        && persistence_ui::load_settings(&dir).restore_on_open
        && !std::env::var("LINA_NO_RESTORE").is_ok_and(|v| v.trim() == "1")
    {
        if let Some(proj) = &restore_proj {
            let mut plans =
                bridge::plan_restore(proj, &profile_registry, nodes.scrollback().as_ref());
            // ADR 0037: re-lançar o CLI por caminho absoluto — no boot via Dock o PATH é mínimo e
            // o nome nu (`claude`) não resolveria; busca em dirs canônicos + nvm.
            crate::absolutize_restore_commands(&mut plans);
            if plans.is_empty() {
                eprintln!("lina-gpui: [RESTORE] nada a re-erguer (Espaço novo ou sem terminais)");
            } else {
                let up = nodes.restore_terminals(&plans);
                eprintln!(
                    "lina-gpui: [RESTORE] {up}/{} terminal(is) re-erguidos do quit limpo",
                    plans.len()
                );
            }
        }
    }

    // W3-7c · TETO DE CUSTO REAL. `LINA_TOKEN_BUDGET_DAY` (tokens/dia ESTIMADOS ≈ bytes/4 de output;
    // ver cost.rs) ARMA o teto; 0/ausente = desligado (default, sem regressão). A bomba mede o output
    // dos PTYs e apenda TokenUsageReported no MESMO store (`<ws_root>/.lina/events`, o log
    // compartilhado com o bin `lina`) que o CostLedger soma.
    let token_budget_day: u64 = std::env::var("LINA_TOKEN_BUDGET_DAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let idle_ms = injection_profile.idle_ms.unwrap_or(200);
    let bridge_host = GpuiBridgeHost::new(Arc::clone(&model));
    // F1-1-7: a FILA DE ATENÇÃO unificada — a projeção `AttentionQueue` do core cabeada
    // ao MESMO event log (replay no boot reconstrói pendência pós-crash — critério 5) e
    // à mesa de custódia. NENHUM byte vai ao PTY por este caminho (ADR 0021 §6).
    // Criada ANTES do pump (R2b): a varredura viva da detecção usa a projeção de mute
    // do hub (mesma fonte do log — defesa em profundidade nas duas pontas).
    // F1-1-8 / ADR 0024: o hub ganha a porta de escrita validada (gesto humano + auto-deny do
    // SLA digitam por aqui). `sup`/`grids` são os MESMOS do app (Arc compartilhado); as
    // `approval_keys` vêm do CLI Profile de injeção (lido ANTES do move para a MailboxPump).
    let attention = Arc::new(AttentionHub::new(
        Arc::clone(&store),
        Arc::clone(&desk),
        Arc::clone(&model),
        ApprovalWiring::new(
            Arc::clone(&sup),
            Arc::clone(&grids),
            injection_profile.approval_keys.clone(),
        ),
        // SEAM-1: aprovar um banner de spawn na fila cria o terminal pelo MESMO funil (admit_node).
        Arc::clone(&nodes),
    ));
    let live_detection = attention_demo_kind().is_none();
    if !live_detection {
        eprintln!("lina-gpui: [ATT] modo DEMO — detecção viva de grid DESLIGADA nesta sessão");
    }
    // inv#6/no-panic: sem a ponte core→UI o app não renderiza terminais — encerra com mensagem clara
    // em vez de backtrace. (Falha só em condição catastrófica de sistema; instalação nova não cai aqui.)
    // FIX DE GATE F1-0: o pump precisa do Supervisor p/ devolver o nó a Idle no fim-de-resposta
    // (transição event-sourced — destrava a retenção F1-0-4 na 2ª mensagem ao mesmo alvo).
    // R2b: + grids/hub/flag — a metade-app da detecção viva roda no tick do pump.
    let pump = match spawn_pump(
        bridge_host,
        delta_rx,
        bus_rx,
        Arc::clone(&store),
        idle_ms,
        Arc::clone(&sup),
        Arc::clone(&grids),
        Arc::clone(&attention),
        live_detection,
    ) {
        Ok(p) => p,
        Err(e) => {
            return Err(format!(
                "não subi a ponte core→UI ({e}); o app não consegue renderizar"
            ));
        }
    };

    let input = Arc::new(CoreInput::new(Arc::clone(&sup)));

    // W3-2: garante o CLAUDE.md/estado do roster do boot (vazio em produção — best-effort).
    // (O NodeManager nasce ANTES do seed, lá em cima — rodada 360/ADR 0022.)
    nodes.rewrite_bootstrap();

    // W3-4: sobe o SUPERVISOR que observa a mailbox (`<ws_root>/.lina/outbox/`). A partir daqui,
    // um `lina ask @B "oi"` de qualquer terminal trafega: outbox → guardrails → deliver_a2a → B.
    // Thread DAEMON (observador de fundo): o app sai via `process::exit`, que encerra a thread —
    // o handle é mantido só para deixar o ciclo de vida explícito.
    let _mailbox_pump = MailboxPump::new(
        Arc::clone(&sup),
        Arc::clone(&store),
        Arc::clone(&grids),
        Mailbox::new(&mailbox_dir),
        nodes.reinject_queue(), // FIX-A3: fila EM-PROCESSO de re-injeção (sem superfície de filesystem)
        Arc::clone(&model),
        Arc::clone(&brake), // W4-3: freio (pausa/retoma a auto-orquestração)
        token_budget_day,   // W3-7c: arma o teto de custo no Router (0 = desligado)
        // F1-0-2: o claude-code.toml real (fallback-com-warning no loader). Clone: o original
        // fica no `WsRuntime` (o demo ⚡ e a fatia ii leem de lá).
        injection_profile.clone(),
        // A2A UNIVERSAL: resolve o profile do ALVO (codex/gemini/…) na entrega.
        Arc::clone(&profile_registry),
        autonomy, // SEAM-1 (M2): autonomia REAL fiada no RouterConfig (manual bloqueia spawn)
        Arc::clone(&nodes), // SEAM-1: funil de admissão — SpawnApproved cria o terminal
        // F3-5-1: o MESMO snapshot de sessões que o dashboard recebe do hub — o pump correlaciona por
        // `cwd` e nasce o `SessionPersisted` do 1º prompt (vazio quando o CLI não grava session-file).
        Arc::clone(&dash.sessions),
    )
    .with_webhook(webhook.as_deref())
    .spawn();

    // W3-6c (ADR 0004): a BROKER PUMP observa a FILA DE BROKER (`<ws_root>/.lina/broker/`, irmã do
    // outbox A2A), aplica o gate humano (⌘⏎ na janela) e chama `run_custody` — que obtém o segredo do
    // cofre e executa. O agente NUNCA tem o token. Thread daemon (encerra com o `process::exit`).
    let _broker_pump = BrokerPump::new(
        Mailbox::new(mailbox_dir.join("broker")),
        Arc::clone(&store),
        custody_vault,
        Arc::clone(&desk),
        Arc::clone(&model),
        Arc::clone(&sup), // roster vivo: valida que a origem do pedido é um nó REAL (hole 3)
    )
    .spawn();

    Ok(WsRuntime {
        ws_root,
        mailbox_dir,
        store,
        sup,
        nodes,
        model,
        grids,
        input,
        attention,
        desk,
        brake,
        autonomy,
        injection_profile,
        profile_registry,
        hooks,
        webhook,
        dash,
        _pump: pump,
        _mailbox_pump,
        _broker_pump,
    })
}

// ───────────────── r5 perf-ws · hub ÚNICO de session-watch por PROCESSO ─────────────────
//
// ANTES (a regressão do multi-workspace): cada `boot_ws_runtime` subia SUA thread com um
// `SessionWatch` ZERADO — N Espaços = N cold-parses do HOME inteiro (medido na máquina do
// fundador: **10,8s** de parse por boot, 3.735 jsonl/1,3GB) + N caminhadas+stat a cada
// 500ms (medido: ~10ms/poll real, 37ms em cache frio ⇒ 4 Espaços ≈ 30k stats/s contínuos).
// DEPOIS: UM watcher por processo; fontes deduplicadas; cold-parse 1×; cada Espaço novo só
// se INSCREVE e recebe o snapshot corrente na hora. As sessões são dado GLOBAL da máquina
// (o HOME é um só) — não há nada por-Espaço numa varredura dessas.

/// Hub testável (sem thread/relógio próprios): o estado da varredura única + os
/// inscritos (`Weak` — runtime removido sai sozinho do fan-out).
struct SessionHub {
    watch: lina_session_watch::SessionWatch,
    sources: std::collections::BTreeSet<(String, String)>,
    known: std::collections::BTreeSet<(String, String)>,
    last_snapshot: Vec<lina_session_watch::Session>,
    subs: Vec<std::sync::Weak<Mutex<Vec<lina_session_watch::Session>>>>,
}

impl SessionHub {
    fn new(home: PathBuf) -> Self {
        Self {
            watch: lina_session_watch::SessionWatch::with_home(home),
            sources: std::collections::BTreeSet::new(),
            known: std::collections::BTreeSet::new(),
            last_snapshot: Vec::new(),
            subs: Vec::new(),
        }
    }

    /// Registra uma fonte (cli + pattern do TOML), DEDUPLICADA: N Espaços com o mesmo
    /// perfil = 1 varredura. Fonte nova (outro CLI/pattern) entra no próximo poll.
    fn add_source(&mut self, cli: &str, pattern: &str) {
        if self.sources.insert((cli.to_string(), pattern.to_string())) {
            self.watch.add_source(cli, pattern);
        }
    }

    #[cfg(test)]
    fn sources_len(&self) -> usize {
        self.sources.len()
    }

    /// Inscreve o destino de UM runtime e o SEMEIA com o snapshot corrente — um Espaço
    /// aberto depois do cold-parse não fica vazio esperando o próximo update.
    fn subscribe(&mut self, out: &Arc<Mutex<Vec<lina_session_watch::Session>>>) {
        if let Ok(mut dst) = out.lock() {
            dst.clone_from(&self.last_snapshot);
        }
        self.subs.push(Arc::downgrade(out));
    }

    /// Um passo: varre 1× e, se algo mudou, refaz o snapshot e alimenta TODOS os
    /// inscritos vivos (poda os mortos). Devolve quantos inscritos foram alimentados.
    fn poll_and_fanout(&mut self) -> usize {
        let updated = match self.watch.poll_once() {
            Ok(o) if !o.sessions_updated.is_empty() => {
                self.known.extend(o.sessions_updated);
                self.last_snapshot = self
                    .known
                    .iter()
                    .filter_map(|(c, s)| self.watch.scanner().session(c, s))
                    .collect();
                true
            }
            Ok(_) => false,
            Err(e) => {
                eprintln!("lina-gpui: [SESSÕES] poll do hub falhou (re-tenta): {e}");
                false
            }
        };
        if !updated {
            self.subs.retain(|w| w.strong_count() > 0);
            return 0;
        }
        let mut fed = 0;
        self.subs.retain(|w| match w.upgrade() {
            Some(out) => {
                if let Ok(mut dst) = out.lock() {
                    dst.clone_from(&self.last_snapshot);
                    fed += 1;
                }
                true
            }
            None => false,
        });
        fed
    }
}

/// Hub global do processo (nasce no 1º Espaço com `session_dir_pattern`).
static SESSION_HUB: std::sync::OnceLock<Arc<Mutex<SessionHub>>> = std::sync::OnceLock::new();

/// Inscreve o `dash.sessions` de um runtime no hub único (cria hub + thread na 1ª chamada).
fn subscribe_sessions(
    cli: &str,
    pattern: &str,
    out: &Arc<Mutex<Vec<lina_session_watch::Session>>>,
) {
    let Some(home) = std::env::var_os("HOME") else {
        return; // sem HOME não há o que varrer (mesma degradação de antes)
    };
    let hub = SESSION_HUB.get_or_init(|| {
        let hub = Arc::new(Mutex::new(SessionHub::new(PathBuf::from(home))));
        let for_thread = Arc::clone(&hub);
        let spawned = thread::Builder::new()
            .name("lina-session-hub".into())
            .spawn(move || loop {
                lock(&for_thread).poll_and_fanout();
                thread::sleep(Duration::from_millis(500));
            });
        if let Err(e) = spawned {
            eprintln!("lina-gpui: [SESSÕES] hub sem thread ({e}); dashboards sem sessões");
        }
        hub
    });
    let mut h = lock(hub);
    h.add_source(cli, pattern);
    h.subscribe(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **r5 park-seam — `parked_pending` (puro sobre log+projeção):** só ids de
    /// `WorkspaceUnloaded` que AINDA estão `Dead` na projeção pendem religação; depois do
    /// supersede (`NodeRemoved` do restore) saem sozinhos — religar 2× é impossível por
    /// construção (idempotência do foco).
    #[test]
    fn parked_pending_tracks_unloaded_until_superseded() {
        let dir = std::env::temp_dir().join(format!("lina-relight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = EventStore::open(dir.join(".lina/events")).expect("store");
        let node: NodeId = uuid::Uuid::now_v7();
        store
            .append(&lina_core::DomainEvent::NodeAdded {
                node,
                kind: "Terminal".into(),
                x: 1.0,
                y: 2.0,
                requested_by: None,
            })
            .expect("NodeAdded");
        store
            .append(&lina_core::DomainEvent::NodeRenamed {
                node,
                name: "Pesquisador".into(),
            })
            .expect("NodeRenamed");
        store
            .append(&lina_core::DomainEvent::NodeStatusChanged {
                node,
                status: "Dead".into(),
                from: "Idle".into(),
                reason: "unloaded_bg".into(),
            })
            .expect("morte registrada");
        store
            .append(&lina_core::DomainEvent::WorkspaceUnloaded {
                parked: vec![node.to_string()],
                kept: vec![],
            })
            .expect("gesto registrado");

        let recs = store.events().expect("events");
        let proj = store.project().expect("project");
        let pending = parked_pending(&recs, &proj);
        assert!(pending.contains(&node), "estacionado pende religação");

        // Supersede (o restore aposenta o antigo) → sai do pendente (idempotência).
        store
            .append(&lina_core::DomainEvent::NodeRemoved { node })
            .expect("NodeRemoved");
        let pending2 = parked_pending(&store.events().expect("e"), &store.project().expect("p"));
        assert!(pending2.is_empty(), "aposentado não religa de novo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **r5 perf-ws (regressão do multi-workspace):** N Espaços compartilham UMA varredura —
    /// fonte duplicada não duplica o walk, todo inscrito recebe o MESMO snapshot, e um
    /// inscrito TARDIO (Espaço aberto depois do cold-parse) nasce já alimentado.
    #[test]
    fn session_hub_single_scan_feeds_all_subscribers_and_dedupes_sources() {
        let home = std::env::temp_dir().join(format!("lina-hub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let proj = home.join(".claude/projects/proj-a");
        std::fs::create_dir_all(&proj).expect("corpus");
        std::fs::write(
            proj.join("sess-1.jsonl"),
            concat!(
                r#"{"type":"assistant","sessionId":"sess-1","requestId":"r1","message":{"model":"claude-opus-4-8","usage":{"input_tokens":10,"output_tokens":5}}}"#,
                "\n"
            ),
        )
        .expect("fixture");

        let mut hub = SessionHub::new(home.clone());
        hub.add_source("claude-code", "~/.claude/projects/*/*.jsonl");
        // N runtimes com o MESMO perfil registram a MESMA fonte → 1 varredura, não N.
        hub.add_source("claude-code", "~/.claude/projects/*/*.jsonl");
        assert_eq!(hub.sources_len(), 1, "fonte deduplicada");

        let a: Arc<Mutex<Vec<lina_session_watch::Session>>> = Arc::default();
        hub.subscribe(&a);
        let fed = hub.poll_and_fanout();
        assert_eq!(fed, 1, "1 inscrito alimentado no update");
        assert_eq!(lock(&a).len(), 1, "snapshot com a sessão do corpus");

        // Inscrito TARDIO: recebe o snapshot corrente NA INSCRIÇÃO (sem esperar update).
        let b: Arc<Mutex<Vec<lina_session_watch::Session>>> = Arc::default();
        hub.subscribe(&b);
        assert_eq!(*lock(&a), *lock(&b), "tardio nasce com o snapshot corrente");

        // Sem mudança no disco: poll não refaz snapshot nem realimenta ninguém.
        assert_eq!(hub.poll_and_fanout(), 0, "estável → zero fan-out");

        // Inscrito morto sai sozinho do fan-out (Weak podado).
        drop(a);
        std::fs::write(
            proj.join("sess-2.jsonl"),
            concat!(
                r#"{"type":"assistant","sessionId":"sess-2","requestId":"r2","message":{"model":"claude-opus-4-8","usage":{"input_tokens":1,"output_tokens":1}}}"#,
                "\n"
            ),
        )
        .expect("2ª sessão");
        assert_eq!(hub.poll_and_fanout(), 1, "só o inscrito VIVO é alimentado");
        assert_eq!(lock(&b).len(), 2, "snapshot cresceu com a sessão nova");

        let _ = std::fs::remove_dir_all(&home);
    }
}
