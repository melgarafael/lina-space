# F2TRAD · Camada 2 — costura da skill `lina-translator`

> ✅ **APLICADO E VALIDADO** (Maestro autorizou; Terminal I confirmou zero colisão).
> Resultado: `test -p lina-role-discovery` = 19 passed · `test -p lina-bootstrap` = exit 0
> (`catalog_matches_assets_dir`, `installs_all_skills_with_references` (12), `descriptions_fit_codex`
> (896B), `role_skill_promises` todos ok; lib 59) · `clippy -D` = 0 · `fmt --check` = 0.
> Pontos finais: skill em `assets/lina-skills/lina-translator/SKILL.md`; catálogo `skills.rs:58`;
> contadores `skills.rs:175` + `lib.rs:1299` = 12; TRADUTOR `skills:[lina-translator, lina-orchestration]`.
> Workers não commitam — pronto para o gate de onda do Maestro.

A **Camada 1** (papel TRADUTOR no registry + `entry_origin` de degradação + testes) ficou
ENTREGUE e VERDE na fronteira `crates/lina-role-discovery/`. Esta Camada 2 instalou a
**doutrina própria** do Tradutor e cruza `crates/lina-bootstrap/` — que inclui `src/lib.rs`
(costura de dono único). O Terminal I confirmou não tocar `skills.rs`/`lib.rs`/`assets/`, então a
costura abaixo foi aplicada sem colisão. Diff aplicado (4 pontos, atômico):

## Diff (4 pontos, atômico)

**1. Mover a skill para o lugar canônico**
`tasks/epico-f3/despachos/f3-2/lina-translator.SKILL.md` → `assets/lina-skills/lina-translator/SKILL.md`
(description já medida: 896 bytes ≤ 1024 do Codex; corpo no estilo das demais `lina-*`).

**2. `crates/lina-bootstrap/src/skills.rs` — catálogo + contador**
- No array `LINA_SKILLS` (ordem alfabética), inserir entre `lina-spawn-terminal` e `lina-verification`:
  ```rust
  embed!("lina-translator", ["SKILL.md"]),
  ```
- Linha ~170 (`installs_all_skills_with_references`): `assert_eq!(dirs.len(), 11, "as 11 skills…")`
  → `12` (e o texto "as 12 skills").
- Doc-comment do topo (linha ~4 "as 11 skills da F1-3") → 12 (honestidade do doc).

**3. `crates/lina-bootstrap/src/lib.rs` — contador da safra**
- Linha ~1299: `assert_eq!(LINA_SKILLS.len(), 11, "a 1ª safra tem 11 skills")` → `12`.

**4. `crates/lina-role-discovery/src/default-roles.yaml` (MEU arquivo — faço eu na fatia)**
- TRADUTOR: `skills: [lina-orchestration]` → `skills: [lina-translator, lina-orchestration]`.
- E no `lib.rs` do meu crate, o teste `tradutor_classifies_the_entry_interpreter` passa a esperar
  `vec!["lina-translator", "lina-orchestration"]` (eu ajusto junto).

## Validação da Camada 2 (após aplicar)
- `cargo test -p lina-bootstrap` (precisa do `bin/lina.rs` do Terminal I compilando — HOJE está
  quebrado em `run_history`, erro do I, não meu): roda `catalog_matches_assets_dir`,
  `installs_all_skills_with_references`, `descriptions_fit_codex_limit`, `role_skill_promises…`.
- `cargo test -p lina-role-discovery` (meu — re-verde com o skills esperado atualizado).

## Camada 2b (OPCIONAL — só melhora a tela; degrada gracioso, NÃO bloqueia gate)
Hoje TRADUTOR cai no rótulo genérico legível ("Tradutor / Colabora no time…"). Para rótulo rico
(decisão do **Terminal G**, dono de `app/lina-gpui/`, e do dono de `lina-bootstrap/src/lib.rs`):
- `app/lina-gpui/src/role_suggester.rs` `humanize()`: braço
  `"TRADUTOR" => ("Tradutor", "Interpreta o seu pedido e devolve a estratégia antes de agir.")`.
- `crates/lina-bootstrap/src/lib.rs` `role_mission()`/`role_blurb()`: braços TRADUTOR equivalentes.
