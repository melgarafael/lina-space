# F35-SKILL — costura para o Maestro fiar (Terminal J)

> Branch `lina/f3-5-skills` @ `b106024`. As funções PURAS (seletor/fábrica/guard) + envelopes
> estão entregues e testadas (34 testes, clippy `--all-targets -D` limpo, suíte 861 verde).
> Falta só a fiação de I/O que cruza arquivos do Maestro (`bin/lina.rs` dispatch + `router.rs`
> handlers + `events.rs` emissão). Tudo abaixo é texto pronto — eu não toquei esses arquivos.

## 1. Dispatch do verbo `lina skill` — `crates/lina-bootstrap/src/bin/lina.rs`

Adicionar o braço no match de verbos (perto de `Some("code-changed") => …`):

```rust
        Some("skill") => run_skill(&args[1..]),
```

E colar o handler (usa `lina_bootstrap::skills::*` — já `pub mod` — + primitivas do bin
`load_identity`/`mailbox_root`/`enqueue_per_node`/`parse_kv_flags`, todas existentes):

```rust
/// `lina skill <check|select|propose>` (F3-5-4/5). A lógica pura vive em `skills.rs` (seletor/
/// fábrica no core, anti-ciclo); aqui só o I/O. `select`/`propose` enfileiram o contrato — o
/// supervisor emite SkillSelected/SkillFactoryProposed carimbando o `node` SERVER-SIDE.
fn run_skill(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("check") => run_skill_check(args.get(1)),
        Some("select") => run_skill_select(&args[1..]),
        Some("propose") => run_skill_propose(&args[1..]),
        _ => {
            eprintln!("lina: uso: lina skill <check <path> | select --context <txt> [--have <tool>]... | propose <nome> [--ref <url>]...>");
            ExitCode::from(2)
        }
    }
}

/// `lina skill check <path>` — read-only: valida o formato (core) e classifica o risco de carga
/// (guard de inline-shell). Exit 3 = "precisa gate humano" (inline-shell), não erro de execução.
fn run_skill_check(path: Option<&String>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("lina: skill check exige <path-da-SKILL.md>");
        return ExitCode::from(2);
    };
    let md = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lina: nao foi possivel ler {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let check = lina_bootstrap::skills::skill_check(&md);
    match &check.format {
        Ok(()) => println!("formato: ok"),
        Err(e) => println!("formato: INVALIDO — {e}"),
    }
    if check.load_class == lina_core::ActionClass::Routine {
        println!("carga: liberada (skill e so dado, sem inline-shell)");
        ExitCode::SUCCESS
    } else {
        println!("carga: GATE HUMANO — a skill tem inline-shell (codigo nao-confiavel); revise antes de habilitar");
        ExitCode::from(3)
    }
}

/// `lina skill select --context <txt> [--have <tool>]...` — roda o seletor PURO sobre o índice
/// (catálogo + `.claude/skills`) filtrado pelas tools presentes; enfileira `skill.select` por
/// skill que casou.
fn run_skill_select(args: &[String]) -> ExitCode {
    let (scalars, have) = parse_kv_flags(args, "--have");
    let Some(context) = scalars.get("--context") else {
        eprintln!("lina: skill select exige --context <txt>");
        return ExitCode::from(2);
    };
    let from = match load_identity() {
        Ok(i) => i.terminal_name,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    let present: std::collections::BTreeSet<String> = have.into_iter().collect();
    let index = lina_bootstrap::skills::build_skill_index(Some(Path::new(".claude/skills")));
    let selections = lina_core::skill_index::select(&index, &present, context);
    if selections.is_empty() {
        println!("nenhuma skill casou o contexto (com as tools presentes)");
        return ExitCode::SUCCESS;
    }
    let mailbox = Mailbox::new(mailbox_root());
    for sel in &selections {
        let msg = lina_bootstrap::skills::build_skill_select_envelope(
            &from,
            &sel.name,
            sel.trigger.as_deref(),
            "catalog",
        );
        if let Err(e) = enqueue_per_node(&mailbox, &from, &msg) {
            eprintln!("lina: falha ao enfileirar skill.select: {e}");
            return ExitCode::from(1);
        }
        println!(
            "ok: skill '{}' selecionada (gatilho: {})",
            sel.name,
            sel.trigger.as_deref().unwrap_or("-")
        );
    }
    ExitCode::SUCCESS
}

/// `lina skill propose <nome> [--ref <url>]...` — a fábrica PROPÕE (gate humano antes de criar).
fn run_skill_propose(args: &[String]) -> ExitCode {
    let (_, refs) = parse_kv_flags(args, "--ref");
    let Some(name) = args.first().filter(|a| !a.starts_with('-')) else {
        eprintln!("lina: skill propose exige <nome> [--ref <url>]...");
        return ExitCode::from(2);
    };
    let from = match load_identity() {
        Ok(i) => i.terminal_name,
        Err(e) => {
            eprintln!("lina: {e}");
            return ExitCode::from(1);
        }
    };
    let msg = lina_bootstrap::skills::build_skill_propose_envelope(&from, name, &refs);
    let mailbox = Mailbox::new(mailbox_root());
    match enqueue_per_node(&mailbox, &from, &msg) {
        Ok(()) => {
            println!("ok: skill.propose '{name}' enfileirado (gate humano antes de criar/habilitar)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lina: falha ao enfileirar skill.propose: {e}");
            ExitCode::from(1)
        }
    }
}
```

## 2. Emissão dos eventos — `router.rs` (handler) + `events.rs` (já tem as variantes)

As variantes `SkillSelected{node,skill,trigger,source}` e `SkillFactoryProposed{skill_name,via,
references}` já existem (Fundação A). Falta o handler que as EMITE. No `route_message`, interceptar
por intent (alvo sentinela `"skill"`), espelhando `handle_code_changed`/`handle_params`:

- intent **`skill.select`** → `SkillSelected`: o `node` é a IDENTIDADE AUTENTICADA do remetente
  (server-side, ADR 0007/0026 — NUNCA do payload, que só traz `skill`/`trigger`/`source`).
- intent **`skill.propose`** → `SkillFactoryProposed`: payload traz `skill_name`/`via`/`references`;
  **sem** efeito colateral de escrita — só o evento (sugere, nunca aplica; criar é gesto humano).

> Sem este handler, o consumidor (Terminal I, camada de skills do briefing) não terá projeção
> `SkillSelected` para ler. É a única peça que mantém a feature viva ponta-a-ponta — provei o
> envelope (campos + `node` omitido) em `skills.rs::select_envelope_omits_node_and_carries_selection`,
> mas o intent→handler→evento tem que passar por `route_message` (memória: teste à-mão não prova
> a costura). Posso escrever o diff do handler se você me liberar a região do router.

## 3. Caminho AUTOMÁTICO (não-verbo) — para a integração com BRIEFING (Terminal I)

O verbo `lina skill select` é o caminho manual/diagnóstico. O caminho PRINCIPAL do design
(vault C.1) é o seletor rodando no fluxo de briefing/spawn: quando um terminal começa um turno,
o core chama `lina_core::skill_index::select(build_skill_index(...), present_tools, context)` e
emite `SkillSelected` por seleção; o Terminal I LÊ essa projeção para montar a camada de skills
do briefing. Nenhum de nós edita o arquivo do outro (J emite, I consome) — só precisamos do
ponto de emissão (item 2) fiado. As funções puras já estão prontas para esse caller.

## Exits do gate (colados)

```
CLIPPY_CORE_EXIT=0          (cargo clippy -p lina-core --all-targets -- -D warnings)
CLIPPY_BOOT_EXIT=0          (cargo clippy -p lina-bootstrap --all-targets -- -D warnings)
TEST_EXIT=0                 (cargo test -p lina-core -p lina-bootstrap → 861 passed, 0 failed)
  skill_index:   11 passed   guard (inline-shell): +8 (39 total)
  skill_factory:  8 passed   bootstrap/skills:     +7 (11 total)
FMT: limpo nos 4 arquivos    Fronteira: 0 toque em events.rs/lib.rs/router.rs/briefing.rs
```
