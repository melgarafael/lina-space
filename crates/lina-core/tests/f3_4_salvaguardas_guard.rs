//! **F3-4 · Gate (d)/(e3) — salvaguardas git gated-hard (QA/Red-team, eval-first).**
//!
//! A onda F3-4 introduz o 1º `git` de runtime (worktree por agente, ADR 0040) e um DevOps
//! integrador (ADR 0042). A salvaguarda inegociável (spec 36 §3, épico §F3-4(d)): o integrador
//! — e qualquer agente — **NUNCA** roda `git reset --hard`, `git branch -D` (apaga branch
//! potencialmente não-mergeada) nem `git checkout --force` (descarta working tree). São os
//! comandos que apagam trabalho em SILÊNCIO — o pecado que esta onda existe para matar (inv #6).
//! Antes da F3-4 o guard só barrava `git push --force` (`guard.rs:230`); a Trilha A (Terminal B)
//! estendeu `is_gated_hard` com os outros três (ADR 0042 §4, commit `f033194`, integrado na develop).
//!
//! **Estratégia eval-first (despacho §3):** `guard::check_action` é função PÚBLICA e PURA, então
//! testo o comportamento-alvo direto, zero-mock. Os ALVOS (`reset --hard`/`branch -D`/`checkout -f`)
//! nasceram **RED no HEAD** pré-integração (provado via `cargo test -- --ignored`: exit 101, os 3
//! FAILED — o ramo do guard não existia) e viraram **GREEN** quando a Trilha A entrou na develop —
//! prova por MUTAÇÃO no eixo temporal (HEAD = guarda desligada → falha; entrega = guarda religada →
//! passa). O `#[ignore]` (removido na FASE 2 de integração) os manteve fora do gate de pacote
//! enquanto a trilha não havia entregue, sem nunca deixá-los verdes-por-não-rodar.
//!
//! Os CONTROLES rodam na suíte padrão e provam **não-vacuosidade**: (+) `push --force` JÁ barrado
//! mostra que a matriz gated-hard está VIVA e pede humano em TODOS os níveis; (−) `reset --soft`/
//! `branch -d`/`checkout main` devem PASSAR — calibram o alvo do peer (classificar DEMAIS treina o
//! humano a clicar "sim" no automático, perdendo o valor da guarda).

use lina_core::{check_action, classify, ActionClass, AutonomyLevel, Decision};

/// Os 3 níveis de autonomia. A salvaguarda gated-hard é piso confirmável em TODOS — `Autonomous`
/// NUNCA afrouxa (`guard.rs:208`): ação destrutiva pede humano mesmo no nível mais solto.
const NIVEIS: [AutonomyLevel; 3] = [
    AutonomyLevel::Manual,
    AutonomyLevel::Assisted,
    AutonomyLevel::Autonomous,
];

/// Asserção-alvo: `cmd` é gated-hard E pede gate humano (`Ask`) em TODOS os níveis de autonomia.
fn assert_gated_hard_todos_niveis(cmd: &str) {
    assert_eq!(
        classify(cmd),
        ActionClass::GatedHard,
        "`{cmd}` deve classificar como gated-hard (destrói trabalho em silêncio)"
    );
    for lvl in NIVEIS {
        assert_eq!(
            check_action(cmd, lvl).decision,
            Decision::Ask,
            "`{cmd}` em {lvl:?} deve PARAR e pedir gate humano (autônomo nunca afrouxa)"
        );
    }
}

/// Asserção de controle negativo: `cmd` NÃO é gated-hard (variante benigna do mesmo verbo git).
/// Falso-positivo aqui é tão nocivo quanto falso-negativo: gatear o rotineiro treina o humano a
/// aprovar no automático, esvaziando a guarda.
fn assert_nao_gated_hard(cmd: &str) {
    assert_ne!(
        classify(cmd),
        ActionClass::GatedHard,
        "`{cmd}` é variante benigna — NÃO pode ser gated-hard (falso-positivo treina o 'sim' automático)"
    );
}

// ───────────────────────── CONTROLES (suíte padrão — provam não-vacuosidade) ─────────────────────────

/// **(+) Controle positivo: a matriz gated-hard está VIVA.** `git push --force origin main` já é
/// barrado (`guard.rs:230`) e pede humano nos 3 níveis. Sem este controle, um RED dos alvos
/// poderia ser "o guard está quebrado", não "o ramo novo falta" — aqui a família funciona.
#[test]
fn push_force_em_main_gated_hard_controle_positivo() {
    assert_gated_hard_todos_niveis("git push --force origin main");
    assert_gated_hard_todos_niveis("git push -f origin feature");
}

/// **(−) Controle negativo: as salvaguardas NÃO classificam demais.** Os verbos git rotineiros —
/// `reset --soft`/`reset` (mixed)/`branch -d` (só mergeada, lowercase)/`checkout` (troca de branch
/// sem `-f`)/`status` — seguem `Allow`. Calibra o alvo da Trilha A: o ramo novo morde `--hard`/
/// `-D`/`--force`, jamais o cotidiano. QUEBRA SE… a Trilha A classificar amplo demais (ex.: todo
/// `git reset` ou `git checkout`) → o desenvolvimento normal viraria um muro de confirmações.
#[test]
fn salvaguardas_nao_classificam_demais_controle_negativo() {
    assert_nao_gated_hard("git reset --soft HEAD~1");
    assert_nao_gated_hard("git reset HEAD arquivo.rs");
    assert_nao_gated_hard("git branch -d feature-ja-mergeada");
    assert_nao_gated_hard("git checkout main");
    assert_nao_gated_hard("git checkout -b nova-branch");
    assert_nao_gated_hard("git status");
}

// ───────────────────────── ALVOS eval-first (RED no HEAD → GREEN pós-Trilha A) ─────────────────────────

/// **(e3-a) `git reset --hard` bloqueado em todos os níveis.** Descarta commits + working tree sem
/// recuperação trivial. ALVO da Trilha A (ADR 0042 §4: helper análogo a `has_force_flag`).
/// RED no HEAD (hoje `Allow`/`Routine`); GREEN quando B liga o ramo. Prova de mutação temporal.
/// QUEBRA SE… o ramo `reset --hard` não existir no `is_gated_hard` → `classify` devolve `Routine`
/// e o `assert_eq!(GatedHard)` falha. (Rode `cargo test -- --ignored` para observar o RED.)
#[test]
fn reset_hard_bloqueado_em_todos_niveis() {
    assert_gated_hard_todos_niveis("git reset --hard HEAD~1");
    assert_gated_hard_todos_niveis("git reset --hard origin/main");
}

/// **(e3-b) `git branch -D` / `--delete --force` bloqueado.** Apaga branch potencialmente
/// NÃO-mergeada (≠ `-d` lowercase, que o git só permite em branch mergeada). É o vetor exato de
/// "trabalho órfão virou lixo". ALVO da Trilha A. RED no HEAD; GREEN pós-entrega.
/// QUEBRA SE… o ramo `branch -D` não existir → `Routine`/`Allow` e o assert falha.
#[test]
fn branch_delete_force_bloqueado() {
    assert_gated_hard_todos_niveis("git branch -D feature-nao-mergeada");
    assert_gated_hard_todos_niveis("git branch --delete --force feature-nao-mergeada");
}

/// **(e3-c) `git checkout -f` / `--force` bloqueado.** Sobrescreve a working tree, descartando
/// alterações não-commitadas. ALVO da Trilha A. RED no HEAD; GREEN pós-entrega.
/// QUEBRA SE… o ramo `checkout --force` não existir → `Allow` e o assert falha.
#[test]
fn checkout_force_bloqueado() {
    assert_gated_hard_todos_niveis("git checkout -f");
    assert_gated_hard_todos_niveis("git checkout --force main");
}
