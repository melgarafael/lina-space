# Despacho F3-CONF-2 · HARNESS/FLAKY — Terminal I · model opus · effort Medium

> **Antes de codar:** rode `lina plan read CONF-HARNESS` e leia o achado de 2026-06-16 (Terminal B, F3-0-1) e o de 2026-06-18 (Terminal I, F2BRIEF) em `tasks/despachos/achados-dogfooding-sessao.md` — o flaky sistêmico que envenena o gate paralelo do Maestro.

## 1. CONTEXTO (onde o trabalho está)

`cwd`: raiz do repo. Bug **sistêmico, já diagnosticado**: testes que criam `TempDir` qualificado só por `process::id()` + nanos (SEM `thread::current().id()`) colidem sob `cargo test` paralelo — dois testes do mesmo processo cunham a mesma pasta, o `remove_dir_all` de um apaga o dir do outro → falso-vermelho. Passa isolado/serial, falha ~1 em 4 em paralelo. **Custo real:** a cada validação de fora, o Maestro re-deriva 1-a-1 para descartar regressão fantasma; o gate pisca vermelho sem causa.

**Alvo canônico:** `crates/lina-core/tests/history_f1_5_8.rs:16-39` — `struct TempDir`/`fn new()`: path = `lina-hist158-{process::id()}-{nanos}`, **sem tag, sem contador, sem thread::id**, chamado por **6 testes** (`:65,92,111,128,148,279`) que disputam o mesmo namespace.

**Modelos CORRETOS (copie o padrão):** `crates/lina-core/src/router.rs:3491-3496` (`lina-router-{tag}-{pid}-{thread::current().id():?}`) e os testes do bin `crates/lina-bootstrap/src/bin/lina.rs:3503-3507`. ⚠️ `attention.rs` NÃO é modelo (usa `#[serial]`, não thread::id).

**Opcionais (zona cinzenta, só se houver folga):** `gate_w34.rs:60` (`{pid}-{now_ms}`, sem tag), `scrollback_cable_w52.rs:45` (`{tag}-{pid}`, sem nanos), `perf_probe.rs:41` (`{pid}` puro). Estes têm baixa colisão hoje (mitigados por `#[serial]` ou uso único) — só endureça se sobrar tempo.

## 2. FUNÇÃO

Você é o **dono da higiene do harness de teste**. Sua entrega faz o gate paralelo do Maestro parar de mentir — um falso-vermelho a menos por validação, em toda story futura.

## 3. DIRECIONAMENTO

- **Mexa SÓ em** `crates/lina-core/tests/history_f1_5_8.rs` (+ opcionais acima). **NÃO** toque produção, nem os arquivos de teste NOVOS do Terminal R, nem `router.rs`/`attention.rs` (do B). Fronteira estrita = arquivos de teste existentes.
- O fix é **mínimo e mecânico**: inclua `thread::current().id()` no nome do tempdir (e, idealmente, um `static SEQ: AtomicU64` por chamada — o padrão que os testes já-seguros usam, ex.: `f3_1_goal_adversarial.rs:26`). Não reescreva os testes, não mude o que eles provam.
- **Não silencie** com `#[serial]` global (isso esconde, não corrige, e serializa a suíte — mais lento). A correção é a unicidade do path.

## 4. OBJETIVO

O Maestro está prestes a rodar 4 frentes em paralelo nesta rodada — se o `lina-core` piscar vermelho em `history_f1_5_8` sem causa, ele perde tempo re-derivando e pode mascarar uma regressão REAL de outra frente. Sua frente é barata mas destrava a confiabilidade do próprio gate da rodada.

## 5. RESULTADO ESPERADO (formato + marcador)

Diff em `history_f1_5_8.rs` (+ opcionais) + prova de que o flaky morreu:
- Rode `cargo test -p lina-core --test history_f1_5_8` **10× seguidas** (ou em loop) sem falha; antes reproduza o flaky (rode o arquivo inteiro em paralelo algumas vezes e mostre 1 falha) para provar RED→GREEN.
- `cargo clippy -p lina-core --all-targets -D warnings` + `cargo fmt -p lina-core --check` limpos.

**NÃO commite** — reporte ao Maestro: o diff, a evidência do flaky antes (1 falha em N) e depois (10/10 verdes), e os exits.

Termine com `PRONTO: <flaky morto, evidência 10/10 + exits>` ou `BLOCKED: <o que falta>`. Reporte o **1º progresso** ao Maestro ao começar (`lina ask "@Terminal A" "comecei o HARNESS/FLAKY" --intent status`).
