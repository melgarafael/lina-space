# Despacho R3 · QA/RED — Terminal R (opus · high)

> Rodada **F3-CONF-3 — "O Maestro Enxerga o Time"**. Maestro: Terminal B (Ultra Code).
> Marcador OBRIGATÓRIO ao terminar: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Sem marcador = falha de protocolo.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES de começar:**
1. `tasks/epico-f3/onda-f3-conf-3.md` — o plano (o gate (a)–(f); o invariante de segurança "observabilidade ≠ autoridade").
2. `tasks/despachos/achados-dogfooding-sessao.md` — achados **#27**, **#23c/#14**, **#21**.
3. Os 2 despachos-irmãos que você vai cobrir: `tasks/epico-f3/despachos/conf-v3/R1-resolucao.md` (Terminal H — `lina.rs`) e `R2-adocao.md` (Terminal J — bin de adoção).
4. Os pontos no código (SÓ LEITURA da produção): `crates/lina-bootstrap/src/bin/lina.rs` — `reader_node_id():608`, `resolve_check_node():511`, `run_list():3163`, `vault_search():3361`. E `crates/lina-core/src/lib.rs:1534` `node_by_name` (a autoridade que NÃO pode regredir).
5. **Vault (norte):** `lina vault index` → épico `39` §VI (top riscos: #2 "crença/veredito/effort escalar para autoridade"; #3 "adoção 0%"). ADR 0007 (regra-mãe: carimbo server-side) e ADR 0019 (definições de progresso/travamento).

## 2. FUNÇÃO

Você é o **QA/Red-team** da rodada: escreve a suíte que PROVA os 3 fixes (repro RED→GREEN), prova a regra de morte por **mutação** (não por teste verde vacuoso), e garante que a autoridade do router **não regrediu**. Você lê produção e escreve **só testes novos** — não conserta o produto (isso é do H/J).

## 3. DIRECIONAMENTO

- **Escreva SÓ em arquivos de teste NOVOS** sob `crates/**/tests/` (ou `#[cfg(test)]` que você adiciona em módulo de teste, sem alterar lógica de produção). NÃO toque `events.rs`/`bridge.rs`/`main.rs` (Terminal A) nem o produto que H/J estão mexendo. Se um teste precisar de um helper `pub(crate)` que ainda não existe, **peça ao H** (não crie no arquivo dele) ou teste pela superfície pública/CLI.
- **Eval-first / regressão por mecanismo** (padrão da casa, spec 35 §7 e CONF-QA): **cada teste com controle positivo E negativo**. Um teste verde vacuoso é pior que vermelho.
  - **#27 (paridade de parse):** controle positivo — `n-<uuidv7 válido>` parseia para o NodeId certo; uuid puro também. Controle negativo — `"lixo"`, `""`, `"n-"` sozinho, `"n-naoehuuid"` são REJEITADOS (não viram um NodeId silencioso). Prove que o formato aceito é **exatamente** o que o spawn carimba (`n-{uuid}` — confira contra `bridge.rs:2760`/`workspace.rs:147`, só leitura).
  - **#23c (vivo-vence-morto por contraste):** monte um `content` de log com um nome reusado: nó A morto (`TerminalExited`/`NodeRemoved`/`NodeStatusChanged(Dead)`) + nó B vivo, mesmo nome. Prove que a resolução de `list` escolhe **B (vivo)** e bate com `resolve_check_node`. Controle negativo: SEM vivo (só o morto) → exibe o morto honestamente (não esconde, não inventa vivo). Cubra as 3 formas de morte (não só `NodeStatusChanged`).
  - **#21 (não-pendura):** teste que `vault_search` num diretório de teste GRANDE (gere N arquivos `.md` num tmpdir) **retorna em tempo limitado** com saída parcial honesta; e que em vault pequeno o resultado continua completo (não regrediu). Se o fix do H usa teto de tempo, o teste deve ser determinístico (teto de ARQUIVOS é mais determinístico que wall-clock para CI — alinhe com o H qual o knob testável).
- **Segurança do router (o que NÃO pode regredir):** rode/estenda a suíte de segurança do router e prove por mutação que `node_by_name` (autoridade) **continua ignorando mortos e não foi tocado** pela mudança de observabilidade. Se existir um teste como `delegation_budget_is_enforced_per_root_cause`/`spawn_cascade_requires_human_gate`/resolução de remetente — confirme verdes. **Critério duro: 0 ALTA.** Se algum fix de observabilidade vazou para a autoridade (ex.: `list` agora muda quem o router resolve), é ALTA → reporte imediato ao Maestro.
- **tmpdir por thread:** se criar tmpdir, use `thread::id` no nome (lição CONF-HARNESS `history_f1_5_8`) para não piscar falso-vermelho no `cargo test` paralelo. Marque `#[serial]` se o teste tocar estado de filesystem global.
- Convenções: edition 2021; `clippy --all-targets -D warnings` 0 (a catraca linta código de teste — sem `format!` aninhado/`unwrap` solto que o clippy pegue); `cargo fmt -p <seu pacote>` só do seu.
- **Você NÃO commita.** Reporte ao Maestro; ele valida de fora e commita por fatia.

## 4. OBJETIVO

Provar que o Maestro volta a enxergar o time — e que essa visão não abriu nenhuma brecha de identidade/autoridade. Os fixes do H valem só com prova empírica RED→GREEN e mutação; a segurança vale só se a suíte do router seguir verde por mutação. Você é a evidência que separa "parece consertado" de "está consertado".

## 5. RESULTADO ESPERADO

- Arquivos de teste novos cobrindo #27, #23c/#14, #21, + a não-regressão da autoridade do router.
- Cada bug com prova **RED no HEAD atual → GREEN com o fix do H** (rode o teste ANTES do fix do H entrar para confirmar que ele falha pelo motivo certo; depois com o fix, verde). Documente o RED observado no reporte.
- Prova local (rode e LEIA o exit, sem pipe que mascara — use `cmd > /tmp/r3.log 2>&1; echo $?` ou `${pipestatus[1]}` no zsh):
  - `cargo test -p lina-bootstrap 2>&1 | tail -25` (seus testes do bin) + exit.
  - `cargo test -p lina-core 2>&1 | tail -25` (suíte de segurança do router) + exit — **isolado e em paralelo**, para distinguir flaky de regressão real.
  - `cargo clippy --all-targets -- -D warnings` no(s) pacote(s) que você tocou.
- Reporte: `lina ask "@Terminal B - Effort: Ultra Code" "R3: <o que cobriu, REDs observados, exits>" --intent status` terminando com **`PRONTO: R3 — repro #27/#23c/#21 RED→GREEN + segurança router verde por mutação (0 ALTA), exits X/Y`** ou **`BLOCKED: <motivo>`**.

> Padrão-ouro (prove enforcement por mutação): para a segurança, desligue mentalmente/empiricamente a guarda e veja o teste falhar — um teste que passa com a guarda desligada não prova nada. Se achar QUALQUER caminho em que a resolução de observabilidade decida identidade/autoridade, é ALTA: pare e reporte ao Maestro antes de seguir.
