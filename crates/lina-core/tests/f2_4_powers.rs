//! **F2-4 · QA red-team — Área de Poderes (ADR 0052).**
//!
//! A Área de Poderes lê o DISCO do leigo (skills/plugins/agents/commands/hooks/MCPs) e os mostra
//! na tela. Há duas formas de essa onda regredir o produto, e esta suíte é a barreira contra ambas:
//!   1. **mostrar virar autorizar** — um campo lido do disco (`name`/`description`/`origin`) decidir
//!      identidade, ordem ou autorização. Furaria a doutrina de segurança inteira do Lina.
//!   2. **o scan tocar a árvore pesada** — varrer `~/.claude/plugins/` (1,9GB) em vez do manifesto
//!      de 13KB → freeze no mac, inotify ENOSPC no Linux futuro (a "bomba" do A7 da entrega-d4).
//!
//! ## Dois grupos
//!   - **GRUPO 1 — contrato selado** (independe do scan): mostrar ≠ autorizar via o ÚNICO egress do
//!     scanner-DADO para o domínio-do-log (`PowerInventory::audit_counts` → `PowerScanned`), evento
//!     só-metadados, replay/round-trip, observabilidade p/ adoção, contrato dos 5 estados. O
//!     teste-âncora é **provado por mutação** (a mutação da guarda é executada numa worktree git
//!     isolada para não tocar a produção compartilhada; ver `.entrega`).
//!   - **GRUPO 2 — scan REAL** (o Terminal B/F24CORE entregou `scan_powers`/`watch_targets`):
//!     manifest-first (a bomba na árvore), frontmatter-inválido→`NeedsRepair`, inerte-aqui, watcher
//!     não-recursivo. Reconciliados ao CONTRATO REAL (skills via `profile.skills_dir`, manifesto
//!     objeto-mapa, watcher observa o PAI) — quando a impl divergiu das minhas suposições, **o ADR/
//!     impl venceram o teste** (regras-comuns §13), não o contrário.
//!
//! Fronteira (LEI): este arquivo é o ÚNICO que o QA cria; NÃO toca produção (ADR 0052; `_contexto.md` §5).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use lina_core::{
    apply, scan_powers, watch_targets, CliProfile, DomainEvent, EventStore, Power, PowerInventory,
    PowerKind, PowerOrigin, PowerRoots, PowerScope, PowerState, ProjectedState,
};

// ─────────────────────────── sentinelas de forja ───────────────────────────
// Valores plantados num `Power` para PARECEREM autoridade. Se QUALQUER um destes atravessar do
// scanner-DADO para o log/projeção (o domínio que o app possui), "mostrar virou autorizar".
// São strings improváveis para que um `contains` no JSON do evento seja prova limpa de vazamento.
const FORJA_NOME: &str = "@Maestro-IDENTIDADE-FORJADA";
const FORJA_CLI: &str = "system-origem-forjada";
const FORJA_SEGREDO: &str = "SEGREDO-role=admin-NAO-DEVE-VAZAR";

/// Um `Power` cujo `name`/`origin`/`description` foram forjados para tentar virar autoridade.
fn power_forjado() -> Power {
    Power {
        kind: PowerKind::Skill,
        name: FORJA_NOME.to_string(),
        description: FORJA_SEGREDO.to_string(),
        origin: PowerOrigin {
            scope: PowerScope::Global,
            cli: Some(FORJA_CLI.to_string()),
        },
        state: PowerState::Ready,
    }
}

/// `Power` legítimo de apoio (controle), parametrizado por kind/nome/cli.
fn power(kind: PowerKind, name: &str, cli: &str) -> Power {
    Power {
        kind,
        name: name.to_string(),
        description: "descrição crua qualquer".to_string(),
        origin: PowerOrigin {
            scope: PowerScope::Global,
            cli: Some(cli.to_string()),
        },
        state: PowerState::Ready,
    }
}

fn inventario(powers: Vec<Power>) -> PowerInventory {
    PowerInventory {
        powers,
        // `counts` do view-model é resumo do nível 1; a auditoria deriva o próprio mapa de `powers`.
        counts: BTreeMap::new(),
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// tmpdir único por (processo, THREAD, contador) — `thread::id` mata o flaky que envenenaria o gate
/// paralelo do Maestro (molde de `f3_4_contrato_replay`/`f3_3_mentality_replay`).
fn unique(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tid: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "lina-f24powers-{tag}-{}-{tid}-{n}",
        std::process::id()
    ))
}

// ════════════════════════════ GRUPO 1 — VIVOS AGORA ════════════════════════════
// O scanner produz DADO de exibição. O ÚNICO ponto onde esse dado cruza para o domínio que o app
// POSSUI (o event log) é `PowerInventory::audit_counts()` → `DomainEvent::PowerScanned`. É ali, e
// só ali, que "mostrar poderia virar autorizar" — então é ali que o teste-âncora morde.

/// **(C1 · teste-âncora, mutação) O egress de auditoria é CEGO ao conteúdo de poder.** Um `Power`
/// com `name`/`origin`/`description` forjados para parecer autoridade NÃO contamina o evento que vai
/// ao log: o `PowerScanned` derivado carrega só contadores; nenhuma sentinela forjada aparece nele.
///
/// Mutação da guarda (`PowerInventory::audit_counts`, executada em worktree isolada — ver `.entrega`):
/// fazer o mapa de auditoria chavear/incluir `p.name` em vez de `p.kind.key()` → `FORJA_NOME` passa
/// a aparecer no JSON → este teste fica RED. Religando (chave = kind) → GREEN. Prova que mostrar não
/// vira autorizar pelo caminho que o app possui.
#[test]
fn auditoria_nunca_vaza_nome_origem_ou_segredo_de_poder() {
    let inv = inventario(vec![
        power_forjado(),
        power(PowerKind::Plugin, "plugin-legitimo", "claude-code"),
    ]);

    let (total, counts) = inv.audit_counts();
    let ev = DomainEvent::PowerScanned {
        total,
        counts,
        scanned_at_ms: 0,
    };
    let json = serde_json::to_string(&ev).expect("serializar PowerScanned");

    // alvo: nenhuma sentinela forjada (nome/origem/segredo) atravessou para o evento do log.
    assert!(
        !json.contains(FORJA_NOME),
        "nome forjado vazou no evento de auditoria — mostrar virou autorizar (identidade): {json}"
    );
    assert!(
        !json.contains(FORJA_SEGREDO),
        "descrição/segredo de poder vazou no evento de auditoria: {json}"
    );
    assert!(
        !json.contains(FORJA_CLI),
        "origem forjada vazou no evento de auditoria: {json}"
    );

    // controle de não-vacuosidade: o evento REALMENTE carrega os contadores (não é egress vazio).
    assert_eq!(total, 2, "audit_counts contou os 2 poderes (não-vacuoso)");
    assert!(
        json.contains("\"total\":2"),
        "o evento carrega o total — o egress existe e é observável: {json}"
    );
}

/// **(C1 · reforço) `name`/`origin` forjados NÃO mudam o que vai ao log; `kind`/quantidade SIM.**
/// Dois inventários idênticos exceto pelo `name`/`origin` (um legítimo "claude-code", outro forjado
/// "system") produzem auditoria IDÊNTICA — o egress é cego ao que tentaria ser autoridade. Em
/// contraste, mudar o `kind` ou a quantidade MUDA a auditoria — prova que `audit_counts` não é uma
/// constante que "passa por acaso", mas mede exatamente os metadados e nada além.
#[test]
fn nome_e_origem_forjados_nao_alteram_o_egress_mas_kind_e_quantidade_alteram() {
    let legitimo = inventario(vec![power(PowerKind::Skill, "skill-x", "claude-code")]);
    let forjado = inventario(vec![Power {
        name: FORJA_NOME.to_string(),
        origin: PowerOrigin {
            scope: PowerScope::Project,
            cli: Some(FORJA_CLI.to_string()),
        },
        ..power(PowerKind::Skill, "skill-x", "claude-code")
    }]);
    assert_eq!(
        legitimo.audit_counts(),
        forjado.audit_counts(),
        "name/origin/scope forjados NÃO podem mudar o egress de auditoria (mostrar ≠ autorizar)"
    );

    // controle de não-vacuosidade: o que a auditoria DEVE medir (kind, quantidade) de fato a muda.
    let outro_kind = inventario(vec![power(PowerKind::Plugin, "skill-x", "claude-code")]);
    assert_ne!(
        legitimo.audit_counts(),
        outro_kind.audit_counts(),
        "mudar o KIND muda a auditoria (não-vacuoso: audit_counts não é constante)"
    );
    let dois = inventario(vec![
        power(PowerKind::Skill, "a", "claude-code"),
        power(PowerKind::Skill, "b", "claude-code"),
    ]);
    assert_ne!(
        legitimo.audit_counts(),
        dois.audit_counts(),
        "mudar a QUANTIDADE muda a auditoria (não-vacuoso)"
    );
}

/// **(C1 · estrutural, anti-regressão) Nenhum caminho de autoridade consome Poderes.** "Mostrar ≠
/// autorizar" só se sustenta se o produto do scanner NUNCA for lido por quem DECIDE identidade/
/// ordem/autorização. Provo lendo o fonte de produção dos 4 caminhos de autoridade do core
/// (`a2a.rs`/`guard.rs`/`mailbox.rs`/`router.rs`) e exigindo 0 referência a tipos/funções de Poder.
///
/// Mutação: ligar `use crate::powers::...` (ou `scan_powers`/`PowerOrigin`) em qualquer um deles →
/// este teste fica RED. É a forma de provar que forjar um Poder não tem POR ONDE virar autoridade:
/// o tipo não é importado no caminho que decide. Não-vacuoso: os arquivos são lidos via `include_str!`
/// (existem em compile-time) e o controle confirma que `powers.rs` em si É o produtor (cita os tipos).
#[test]
fn nenhum_caminho_de_autoridade_consome_poderes() {
    // include_str! resolve em compile-time relativo a este arquivo → some o arquivo, quebra o build.
    let caminhos_de_autoridade = [
        ("a2a.rs", include_str!("../src/a2a.rs")),
        ("guard.rs", include_str!("../src/guard.rs")),
        ("mailbox.rs", include_str!("../src/mailbox.rs")),
        ("router.rs", include_str!("../src/router.rs")),
    ];
    // marcadores que denunciam que um Poder do disco entrou no caminho de decisão.
    let marcadores = [
        "powers::",
        "scan_powers",
        "PowerInventory",
        "PowerOrigin",
        "PowerScanned",
    ];
    for (nome, src) in caminhos_de_autoridade {
        for marcador in marcadores {
            assert!(
                !src.contains(marcador),
                "{nome} referencia `{marcador}` — um Poder lido do disco entrou no caminho de \
                 autoridade (mostrar virou autorizar). Gates de execução são custódia/WorkspaceTrust."
            );
        }
    }

    // controle de não-vacuosidade: o módulo PRODUTOR de fato cita os tipos (o marcador não é morto).
    let powers_src = include_str!("../src/powers.rs");
    assert!(
        powers_src.contains("PowerInventory") && powers_src.contains("scan_powers"),
        "powers.rs deveria definir os tipos de Poder (controle do marcador — não-vacuoso)"
    );
}

/// **(C6 · mutação) `PowerScanned` carrega APENAS metadados.** As chaves do objeto serializado são
/// exatamente `{event,total,counts,scanned_at_ms}` — nenhum campo de conteúdo de poder. E as chaves
/// de `counts` são SÓ os 6 kinds em minúsculo, nunca nomes de poder.
///
/// Mutação: adicionar ao evento um campo `names`/`descriptions` (ou chavear `counts` por nome) →
/// o conjunto de chaves diverge → RED. Garante que o evento de auditoria é observação, não conteúdo.
#[test]
fn power_scanned_carrega_apenas_metadados_nao_conteudo() {
    let ev = DomainEvent::PowerScanned {
        total: 3,
        counts: BTreeMap::from([("skill".to_string(), 2), ("plugin".to_string(), 1)]),
        scanned_at_ms: 1234,
    };
    let val = serde_json::to_value(&ev).expect("serializar PowerScanned");
    let obj = val.as_object().expect("PowerScanned é objeto JSON");

    let mut chaves: Vec<&str> = obj.keys().map(String::as_str).collect();
    chaves.sort_unstable();
    assert_eq!(
        chaves,
        vec!["counts", "event", "scanned_at_ms", "total"],
        "PowerScanned só pode ter metadados (event/total/counts/scanned_at_ms) — zero conteúdo de poder"
    );

    let kinds_validos = ["skill", "plugin", "agent", "command", "hook", "mcp"];
    for chave in obj["counts"].as_object().expect("counts é mapa").keys() {
        assert!(
            kinds_validos.contains(&chave.as_str()),
            "chave de `counts` não é um kind ({chave}) — vazamento de conteúdo de poder no contador"
        );
    }
}

/// **(C7) Round-trip byte-a-byte pelo EventStore + `kind()` canônico.** O log com `PowerScanned`
/// reabre do disco idêntico (alicerce de qualquer projeção por replay) e a tag persistida casa o
/// nome da variante (chave do `from_record` — lição do fixture sem a tag `event`).
#[test]
fn power_scanned_round_trip_byte_a_byte_e_kind_canonico() {
    let tmp = unique("round-trip");
    let _ = fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");

    let seq = [
        DomainEvent::WorkspaceCreated {
            name: "Projeto X".to_string(),
            focus_preset: String::new(),
        },
        DomainEvent::PowerScanned {
            total: 75,
            counts: BTreeMap::from([("skill".to_string(), 42), ("plugin".to_string(), 33)]),
            scanned_at_ms: 99,
        },
    ];

    let primeira = {
        let mut store = EventStore::open(&dir).expect("abrir store");
        for ev in &seq {
            store.append(ev).expect("append");
        }
        store.events().expect("events")
    };
    let segunda = EventStore::open(&dir)
        .expect("reabrir store")
        .events()
        .expect("events");

    let as_json = |rs: &[lina_core::EventRecord]| -> Vec<String> {
        rs.iter()
            .map(|r| serde_json::to_string(r).expect("serializar registro"))
            .collect()
    };
    assert_eq!(
        primeira.len(),
        seq.len(),
        "todos os eventos persistiram (não-vacuoso)"
    );
    assert_eq!(
        as_json(&primeira),
        as_json(&segunda),
        "o log com PowerScanned reabre BYTE-A-BYTE (replay determinístico)"
    );
    assert!(
        segunda.iter().any(|r| r.kind == "PowerScanned"),
        "PowerScanned persistido sob a tag canônica"
    );

    let _ = fs::remove_dir_all(&tmp);
}

/// **(C7 · replay-safe) Log antigo carrega: campos aditivos têm `#[serde(default)]`.** Um payload
/// mínimo de `PowerScanned` só com `total` (como um log gravado por uma versão futura mais enxuta,
/// ou um replay antigo) desserializa com `counts` vazio e `scanned_at_ms` 0 — replay nunca quebra.
/// Também prova que um log SEM nenhum `PowerScanned` (Espaço pré-F2-4) carrega intacto.
#[test]
fn power_scanned_replay_safe_campos_default_e_log_antigo() {
    // (a) payload mínimo (só `total`) → os aditivos caem no default.
    let minimo = serde_json::json!({ "event": "PowerScanned", "total": 5 });
    let ev: DomainEvent = serde_json::from_value(minimo).expect("PowerScanned mínimo desserializa");
    match ev {
        DomainEvent::PowerScanned {
            total,
            counts,
            scanned_at_ms,
        } => {
            assert_eq!(total, 5);
            assert!(
                counts.is_empty(),
                "counts ausente → default vazio (#[serde(default)])"
            );
            assert_eq!(scanned_at_ms, 0, "scanned_at_ms ausente → default 0");
        }
        outro => panic!("desserializou na variante errada: {outro:?}"),
    }

    // (b) um log de um Espaço pré-F2-4 (sem PowerScanned) reabre intacto.
    let tmp = unique("log-antigo");
    let _ = fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");
    {
        let mut store = EventStore::open(&dir).expect("abrir");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "Antigo".to_string(),
                focus_preset: String::new(),
            })
            .expect("append");
    }
    let lido = EventStore::open(&dir)
        .expect("reabrir")
        .events()
        .expect("events");
    assert_eq!(
        lido.len(),
        1,
        "log pré-F2-4 (sem PowerScanned) carrega intacto"
    );
    let _ = fs::remove_dir_all(&tmp);
}

/// **(C7 · META no-op) `apply(PowerScanned)` NÃO toca `ProjectedState`.** O evento é auditoria pura
/// (projeção dedicada se preciso, padrão `SkillSelected`); o painel lê o scan de disco direto. Provo
/// partindo de um estado não-trivial e mostrando que aplicar `PowerScanned` o deixa idêntico.
///
/// Não-vacuoso: o controle prova que `apply` MUDA o estado para um evento mutante conhecido
/// (`WorkspaceCreated`). Mutação: tirar `PowerScanned` da lista no-op de `apply` → o snapshot diverge.
#[test]
fn apply_de_power_scanned_e_no_op() {
    let mut estado = ProjectedState::default();
    apply(
        &mut estado,
        &DomainEvent::WorkspaceCreated {
            name: "Projeto X".to_string(),
            focus_preset: String::new(),
        },
    );
    assert_eq!(
        estado.workspace_name.as_deref(),
        Some("Projeto X"),
        "controle: apply de WorkspaceCreated MUTA a projeção (não-vacuoso)"
    );

    let snapshot = estado.clone();
    apply(
        &mut estado,
        &DomainEvent::PowerScanned {
            total: 10,
            counts: BTreeMap::from([("skill".to_string(), 10)]),
            scanned_at_ms: 7,
        },
    );
    assert_eq!(
        estado, snapshot,
        "PowerScanned é META: NÃO toca ProjectedState (auditoria, não autoridade)"
    );
}

/// **(C8 · adoção, opcional) `PowerScanned` é observável no log para o `intelligence_adoption`.**
/// O fio condutor do fundador (observabilidade) exige que o uso da Área de Poderes seja medível por
/// replay. Confirmo que N scans aparecem como N eventos contáveis no log — sinalizo, não bloqueio.
#[test]
fn power_scanned_e_observavel_no_log_para_adocao() {
    let tmp = unique("adocao");
    let _ = fs::remove_dir_all(&tmp);
    let dir = tmp.join(".lina/events");
    {
        let mut store = EventStore::open(&dir).expect("abrir");
        store
            .append(&DomainEvent::WorkspaceCreated {
                name: "W".to_string(),
                focus_preset: String::new(),
            })
            .expect("append");
        for ms in [1u64, 2] {
            store
                .append(&DomainEvent::PowerScanned {
                    total: 3,
                    counts: BTreeMap::from([("skill".to_string(), 3)]),
                    scanned_at_ms: ms,
                })
                .expect("append");
        }
    }
    let eventos = EventStore::open(&dir)
        .expect("reabrir")
        .events()
        .expect("events");
    let scans = eventos.iter().filter(|r| r.kind == "PowerScanned").count();
    assert_eq!(
        scans, 2,
        "os 2 scans são contáveis no log (observável p/ intelligence_adoption)"
    );
    let _ = fs::remove_dir_all(&tmp);
}

/// **(C5 · contrato dos 5 estados)** A camada WCAG (texto+ícone+cor) mora no painel UI
/// (`powers_panel.rs`, F24UI — ainda não entregue; ver `.entrega` §sinalizações). O que o CORE pode
/// garantir é o contrato: os 5 estados são variantes DISTINTAS (a UI não pode "perder" um estado e
/// renderizar dois iguais). Anti-regressão barato do enum.
#[test]
fn contrato_dos_cinco_estados_e_distinto() {
    let estados = [
        PowerState::Ready,
        PowerState::UpdateAvailable,
        PowerState::NeedsRepair,
        PowerState::InertHere,
        PowerState::Disabled,
    ];
    for (i, a) in estados.iter().enumerate() {
        for (j, b) in estados.iter().enumerate() {
            assert_eq!(
                i == j,
                a == b,
                "os 5 PowerState devem ser 2-a-2 distintos: {a:?} vs {b:?}"
            );
        }
    }
}

// ════════════ GRUPO 2 — SCAN REAL (F24CORE entregue: scan_powers/watch_targets vivos) ════════════
// O Terminal B entregou a varredura real. Estes testes LIGARAM (sem `#[ignore]`) e provam o
// comportamento contra a impl, reconciliados ao CONTRATO REAL (não às minhas suposições iniciais —
// ADR/impl vencem o teste): (1) skills vêm de `profile.skills_dir`, não de convenção vazia; (2)
// `installed_plugins.json` é objeto-mapa `{"plugins":{"nome@mkt":[…]}}`, não array; (3) `watch_targets`
// observa o PAI `~/.claude/plugins` (não-recursivo, 1 fd) — barrar só DESCENDENTES da árvore pesada.

/// Monta uma skill de fixture em `<home>/.claude/skills/<nome>/SKILL.md` (casa com `skills_dir
/// = "~/.claude/skills"`, que o scanner expande contra `PowerRoots.home`).
fn montar_skill(home: &std::path::Path, nome: &str, frontmatter: &str) {
    let dir = home.join(".claude/skills").join(nome);
    fs::create_dir_all(&dir).expect("criar dir da skill");
    fs::write(dir.join("SKILL.md"), frontmatter).expect("escrever SKILL.md");
}

/// `CliProfile` claude-code mínimo via a API PÚBLICA `from_toml_str` (sem dep `toml` no teste, sem
/// tocar `Cargo.toml`). `skills_dir`/`mcp_config_path` são o que o scanner consome (inv#3).
fn profile_claude(skills_dir: Option<&str>) -> CliProfile {
    let mut toml = String::from(
        "id = \"claude-code\"\nprogram = \"x\"\ndelivery = \"pty_inject\"\nprompt_ready_regex = \">\"\n",
    );
    if let Some(s) = skills_dir {
        toml.push_str(&format!("skills_dir = \"{s}\"\n"));
    }
    toml.push_str("[end_signal]\nkind = \"idle\"\n");
    CliProfile::from_toml_str(&toml, "teste-claude").expect("perfil de teste válido")
}

fn roots_em(home: PathBuf, profiles: Vec<CliProfile>) -> PowerRoots {
    PowerRoots {
        home,
        project_dir: None,
        profiles,
    }
}

/// **(C2 · manifest-first — a bomba na árvore)** O scan lê SÓ o manifesto pequeno
/// (`installed_plugins.json`, 13KB, objeto-mapa) e NUNCA varre a árvore pesada de plugins (1,9GB).
/// Planto uma "bomba" de duas formas: um diretório `repos/repo-BOMBA-FALSA/` E um `SKILL.md` fundo
/// dentro dele — qualquer varredura da árvore (listar repos OU descer recursivo) surfaria "BOMBA".
/// manifest-first ⇒ a bomba nunca aparece; só o plugin do manifesto.
///
/// Provado por mutação (worktree isolada — ver `.entrega`): trocar a leitura do ARQUIVO em
/// `scan_plugins` por um `read_dir` recursivo do diretório-pai → "BOMBA" entra no inventário → RED.
/// Controle positivo: o plugin do MANIFESTO aparece (o scan REALMENTE leu o arquivo).
#[test]
fn scan_e_manifest_first_nunca_varre_a_arvore_pesada() {
    let home = unique("manifest-first");
    let _ = fs::remove_dir_all(&home);
    let plugins = home.join(".claude/plugins");
    fs::create_dir_all(&plugins).expect("criar plugins/");

    // manifesto REAL: objeto-mapa `{"plugins":{"nome@mkt":[…]}}` (formato verificado no disco).
    fs::write(
        plugins.join("installed_plugins.json"),
        r#"{"version":1,"plugins":{"plugin-do-manifesto@oficial":[{"scope":"global"}]}}"#,
    )
    .expect("escrever manifesto");

    // BOMBA na árvore pesada: um repo (dir) + um SKILL.md fundo. Só entra se a árvore for varrida.
    let fundo = plugins.join("repos/repo-BOMBA-FALSA/skills/BOMBA-ARVORE-PESADA");
    fs::create_dir_all(&fundo).expect("criar árvore funda");
    fs::write(
        fundo.join("SKILL.md"),
        "---\nname: BOMBA-ARVORE-PESADA\ndescription: se isto aparece, o scan varreu 1,9GB\n---\n",
    )
    .expect("plantar bomba");

    // profiles vazio: plugins vêm da convenção Claude (o arquivo), não de skills_dir.
    let inv = scan_powers(&roots_em(home.clone(), Vec::new()), Some("claude-code"));

    assert!(
        inv.powers
            .iter()
            .any(|p| p.kind == PowerKind::Plugin && p.name == "plugin-do-manifesto@oficial"),
        "controle: o plugin do manifesto deve aparecer (o scan leu o arquivo de 13KB)"
    );
    assert!(
        !inv.powers.iter().any(|p| p.name.contains("BOMBA")),
        "a bomba da árvore pesada apareceu — o scan varreu 1,9GB (manifest-first violado)"
    );
    let _ = fs::remove_dir_all(&home);
}

/// **(C2 · reforço watcher) `watch_targets` observa o PAI dos manifestos; nunca DESCE na árvore pesada.**
/// O ADR §6 manda observar `~/.claude/plugins` (não-recursivo = 1 fd) — então o pai É um alvo legítimo
/// (asserção ingênua `!ends_with("plugins")` contradiria o ADR). O proibido é um alvo DESCENDENTE da
/// árvore (`plugins/repos/…`), que levaria a watch recursivo (ENOSPC no Linux).
#[test]
fn watch_targets_observa_o_pai_mas_nunca_desce_na_arvore_pesada() {
    let home = unique("watch");
    let plugins_dir = home.join(".claude/plugins");
    let alvos = watch_targets(&roots_em(
        home.clone(),
        vec![profile_claude(Some("~/.claude/skills"))],
    ));

    assert!(!alvos.is_empty(), "controle: há diretórios-pai a observar");
    assert!(
        alvos.contains(&plugins_dir),
        "o PAI ~/.claude/plugins é observado (não-recursivo, 1 fd) — ADR §6 (não-vacuoso)"
    );
    for alvo in &alvos {
        assert!(
            *alvo == plugins_dir || !alvo.starts_with(&plugins_dir),
            "watch_targets desce na árvore pesada de plugins ({alvo:?}) — recursivo é PROIBIDO (ENOSPC)"
        );
    }
    let _ = fs::remove_dir_all(&home);
}

/// **(C3) Frontmatter inválido → `NeedsRepair` (estado + ação), nunca some nem derruba o scan.**
/// Uma skill com `SKILL.md` sem `name` (incompleto/inválido) aparece como "precisa de conserto"; a
/// skill boa ao lado continua visível (o scan não entrou em pânico, degradou por skill).
#[test]
fn frontmatter_invalido_vira_needs_repair_sem_derrubar_o_scan() {
    let home = unique("needs-repair");
    let _ = fs::remove_dir_all(&home);
    montar_skill(
        &home,
        "skill-boa",
        "---\nname: skill-boa\ndescription: frontmatter válido\n---\ncorpo\n",
    );
    // frontmatter inválido/incompleto: SEM o campo `name` (impl: name vazio ⇒ NeedsRepair).
    montar_skill(
        &home,
        "skill-quebrada",
        "---\ndescription: faltou o name\n---\ncorpo\n",
    );

    let inv = scan_powers(
        &roots_em(home.clone(), vec![profile_claude(Some("~/.claude/skills"))]),
        Some("claude-code"),
    );

    let quebrada = inv
        .powers
        .iter()
        .find(|p| p.kind == PowerKind::Skill && p.name == "skill-quebrada");
    assert!(
        quebrada.is_some(),
        "a skill quebrada NÃO pode sumir silenciosamente — deve aparecer para conserto"
    );
    assert_eq!(
        quebrada.expect("achou a quebrada").state,
        PowerState::NeedsRepair,
        "skill com frontmatter inválido = NeedsRepair (botão Consertar)"
    );
    assert!(
        inv.powers
            .iter()
            .any(|p| p.name == "skill-boa" && p.state == PowerState::Ready),
        "controle: a skill boa ao lado continua visível e Ready (o scan não derrubou)"
    );
    let _ = fs::remove_dir_all(&home);
}

/// **(C4) Inerte-aqui é o caso NORMAL do multi-CLI.** Uma skill na pasta do Claude, com o terminal
/// rodando OUTRO motor (foco "gemini"), aparece como `InertHere` com a origem correta — não `Ready`,
/// não sumida. No motor certo (foco "claude-code"), a MESMA skill é `Ready`. (`apply_focus` só
/// sobrescreve `Ready`: um `NeedsRepair` não se mascara atrás de "não funciona neste motor".)
#[test]
fn skill_de_outro_motor_e_inert_here_nao_some_nem_ready() {
    let home = unique("inert-here");
    let _ = fs::remove_dir_all(&home);
    montar_skill(
        &home,
        "skill-x",
        "---\nname: skill-x\ndescription: skill do Claude\n---\ncorpo\n",
    );

    // terminal rodando Gemini → a skill do Claude (cli=claude-code) é inerte AQUI.
    let inv_gemini = scan_powers(
        &roots_em(home.clone(), vec![profile_claude(Some("~/.claude/skills"))]),
        Some("gemini"),
    );
    let x = inv_gemini
        .powers
        .iter()
        .find(|p| p.kind == PowerKind::Skill && p.name == "skill-x")
        .expect("a skill não pode sumir — inerte ainda é visível");
    assert_eq!(
        x.state,
        PowerState::InertHere,
        "skill do Claude com foco Gemini = InertHere (caso normal multi-CLI)"
    );
    assert_eq!(
        x.origin.cli.as_deref(),
        Some("claude-code"),
        "a origem (de qual CLI) é preservada e correta"
    );

    // mesmo disco, foco no motor certo → Ready.
    let inv_claude = scan_powers(
        &roots_em(home.clone(), vec![profile_claude(Some("~/.claude/skills"))]),
        Some("claude-code"),
    );
    let x2 = inv_claude
        .powers
        .iter()
        .find(|p| p.kind == PowerKind::Skill && p.name == "skill-x")
        .expect("a skill aparece no motor certo");
    assert_eq!(
        x2.state,
        PowerState::Ready,
        "a MESMA skill no motor certo = Ready"
    );
    let _ = fs::remove_dir_all(&home);
}
