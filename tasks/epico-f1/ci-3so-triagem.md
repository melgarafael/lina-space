# CI 3-SO real — triagem teste×SO (story F1-6-8)

> **Dono:** Revisor (despacho `r1-ci-3so`) · **Data:** 2026-06-10
> **Regra desta tabela:** *zero skip silencioso*. Todo teste do workspace ou (a) roda nos 3 SOs,
> (b) está atrás de `#[cfg]` com o porquê anotado **no código** e nomeado aqui, ou (c) é `#[ignore]`
> documentado. Não existe quarta categoria.

## Método (como esta triagem foi feita)

1. **Fan-out de 20 agentes** (10 fatias de triagem + refutação adversarial por fatia) lendo o corpo
   de cada um dos **566 testes** do workspace e seus helpers, classificando portabilidade de RUNTIME
   (o workspace já COMPILA verde no Windows — o CI atual prova com `--no-run` + clippy; o risco é
   execução: hang/falha/flake).
2. **Mapa de spawns independente** (grep de `PtyCommand::new`/`CommandBuilder::new`/`Command::new`
   em todo código de teste) cruzado com os achados — cobertura 100%: todo spawn Unix em teste está
   na lista de gateados abaixo.
3. **Re-derivação manual no código** de TODO achado e de TODA refutação antes de assinar (lição
   do projeto: finders e refutadores erram — ver §3, que derrubou ~85 falso-positivos).
4. **Arbitragem pelo compilador:** clippy da visão Windows rodado localmente em cross
   (`--target x86_64-pc-windows-gnu`, mesma resolução de `cfg(unix)`/`cfg(windows)` do msvc) para
   provar que os gates não deixam `dead_code`/`unused` no Windows. (O alvo msvc não cross-compila
   no macOS: o build script C do `libsqlite3-sys` exige headers Windows.)

## §1 Tabela teste×SO

### 1a. Resumo por crate (566 testes + 2 parametrizados = 568 executados no macOS)

| Crate | Testes | Roda 3-SO | cfg-unix (novo) | cfg-unix (pré-existente) | cfg por SO | #[ignore] |
|---|---:|---:|---:|---:|---:|---:|
| lina-core (unit, src/) | 339 | 325 | **7** | 6 (`cli_discovery`) | — | — |
| lina-core (integração, tests/) | 34 | 19 | **7** | — | — | 1 (`gate_f102_real_claude`) |
| lina-core (bins c/ testes) | 12 | 12 | — | — | — | — |
| lina-bootstrap | 89 | 86 | — | 3 (`shim_cli` `#![cfg(unix)]`) | — | — |
| lina-vt | 29 | 29 | — | — | — | — |
| lina-pty | 4 | 4* | — | — | *4 c/ variante `cmd.exe`×`sh` | — |
| lina-host | 5 | 5 | — | — | — | — |
| lina-cli-profiles | 27 | 27 | — | — | — | — |
| lina-role-discovery | 16 | 16 | — | — | — | — |
| lina-session-watch | 15 | 15 | — | — | — | — |
| lina-secrets | 4 | 4 | — | — | — | — |
| lina-hooks | 10 | 10 | — | — | — | — |
| lina-otel | 14 | 14 | — | — | — | — |
| lina-webhooks | 14 | 14 | — | — | — | — |
| **Total** | **566** | **~540** | **14** | **9** | 4 | 1 |

\* os 4 testes do `lina-pty` rodam nos 3 SOs com comandos POR SO (`#[cfg(windows)] cmd.exe /Q /K` ×
`#[cfg(unix)] sh`) — são o **gate real do ConPTY** no runner e por design NÃO foram gateados para fora.
Doc-tests: 0 em todos os crates (seções `Doc-tests` rodam vazias) — nenhum vetor Windows aí.

### 1b. Os 14 testes gateados NESTA story (`#[cfg(unix)]` novo, porquê no código)

| # | Teste | Arquivo | Dependência Unix (runtime) | Equivalente Windows real? |
|---|---|---|---|---|
| 1 | `deliver_approval_with_grid_unblocks_real_prompt_via_supervisor` | `lina-core/src/lib.rs` | spawn `sh -c "printf…; read…"` em PTY real | viável: `cmd /C set /p` — **pendente-windows** |
| 2 | `flow_control_caps_memory_under_flood_and_others_stay_responsive` | `lina-core/src/lib.rs` | spawn `yes` (flood) + `sh -c printf` — **o hang histórico de ~70min** | flood via `cmd /C for /L` — **pendente-windows** |
| 3 | `panic_in_one_loop_is_contained_and_others_survive` | `lina-core/src/lib.rs` | spawn `yes` + `cat` | parcial (`cmd.exe` interativo como eco) — **pendente-windows** |
| 4 | `output_lands_in_parsed_grid` | `lina-core/src/lib.rs` | spawn `sh -c printf` | viável: `cmd /C echo` (molde já existe em `lina-pty/tests/pty.rs:197`) — **pendente-windows** |
| 5 | `watch_appends_choice_ask_within_sla_blocks_node_and_respects_mute` | `lina-core/src/permission_detect.rs` | spawn `sh -c` (caixa de escolha sintética em PTY) | viável via `cmd /C` — **pendente-windows** |
| 6 | `watch_emits_prompt_cleared_when_answered_in_terminal` | `lina-core/src/permission_detect.rs` | spawn `sh -c` | idem — **pendente-windows** |
| 7 | `pty_port_screen_changed_writes_nothing_then_fresh_hash_delivers` | `lina-core/src/approval.rs` | spawn `sh -c "printf…; read…"` | viável: `cmd /C set /p` — **pendente-windows** |
| 8 | `onda0_exit_gate_end_to_end` | `lina-core/tests/gate_onda0.rs` (`#![cfg(unix)]`) | 2 PTYs `cat`/`sh`, A2A faseada, crash-recovery | E2E inteiro — **pendente-windows** (cobre-se hoje pelo de-risk manual do bring-up) |
| 9 | `onda2_exit_gate_end_to_end` | `lina-core/tests/gate_onda2.rs` (`#![cfg(unix)]`) | `cat`/`sh` + **`kill -0`/`-9` (sinais Unix)** | exige redesenho do passo SIGKILL (`taskkill /F`) — **pendente-windows** |
| 10 | `w34_ask_routes_through_mailbox_and_b_receives_lina_msg` | `lina-core/tests/gate_w34.rs` (`#![cfg(unix)]`) | 2 PTYs `cat`/`sh` + bracketed-paste | **pendente-windows** |
| 11 | `gate_f103_lifecycle_with_real_pty` | `lina-core/tests/gate_f103.rs` (`#![cfg(unix)]`) | spawn `sh -c printf/sleep` (`sleep` não existe como binário no Windows) | `cmd /C timeout` — **pendente-windows** |
| 12 | `pty_host_cables_scrollback_to_store_with_zero_loss` | `lina-core/tests/scrollback_cable_w52.rs` (`#![cfg(unix)]`) | `sh -c` com loop POSIX (`while…done`) | `cmd /C for /L` — **pendente-windows** |
| 13 | `e_constraint_transfer_legivel_em_pty_real` | `lina-core/tests/gate_f1_0_5.rs` | `wire_node` com `cat` + `sh -c` (golden transcript) | **pendente-windows** |
| 14 | `ac_0021_7_executor_through_pty_host_unblocks_real_prompt` | `lina-core/tests/gate_f1_1_8.rs` | spawn `sh -c "printf…; read…"` | viável: `cmd /C set /p` — **pendente-windows** |

> **Por que gate em vez de equivalente Windows AGORA:** a fronteira desta fatia é
> *só atributos de `cfg` + comentário; zero mudança de lógica de teste* (despacho `r1-ci-3so`).
> Equivalentes Windows reais = lógica nova de teste ⇒ ficam NOMEADOS acima como backlog
> ("pendente-windows") para uma story futura. O mecanismo que esses 14 testes provam continua
> coberto no macOS/Linux a cada push; a *portabilidade do core* (o que a F1-6-8 promove a gate)
> é provada no Windows pelos ~540 que executam lá — incluindo os 4 do `lina-pty`, que exercitam
> o ConPTY real com `cmd.exe`.

### 1c. Gates pré-existentes (já estavam no código antes desta story — inventariados, não tocados)

| Item | Onde | O quê |
|---|---|---|
| 6 testes `#[cfg(unix)]` | `lina-core/src/cli_discovery.rs:214-308` | probe de versão spawna processo por SO; mocks `#[cfg(windows)]` nas linhas 34/41 |
| `#![cfg(unix)]` (3 testes) | `lina-bootstrap/tests/shim_cli.rs` | shim de git intercepta via `/bin/sh` |
| 4 testes com variantes por SO | `lina-pty/tests/pty.rs` | `test_shell()`: `cmd.exe /Q /K` no Windows × `sh` no Unix — **rodam nos 3 SOs** |
| 1 `#[ignore]` | `lina-core/tests/gate_f102_real_claude.rs` | exige `claude` autenticado + 2-4min; roda sob demanda (`--ignored`), nunca no CI |

Rodapé: os `cfg` de **produção** (`mailbox.rs::open_no_follow` O_NOFOLLOW, `scrollback.rs::peak_rss_bytes`,
`bench.rs::thread_count`, `cli-profiles::CURRENT_OS`) têm caminho Windows próprio com fallback portável —
o run Windows passa a exercitar essas variantes a cada push.

## §2 O diff do `ci.yml`, explicado

1. **`core-windows` virou gate bloqueante** — exatamente a promoção que o próprio arquivo pedia
   ("PÓS-BRING-UP: troque `--no-run` por `-- --test-threads=1` e remova o `continue-on-error`"):
   - `cargo test --workspace --no-run` → **`cargo test --workspace -- --test-threads=1`**
     (executa de verdade, serializado — mesma receita dos gates macOS/Linux; o store/recovery
     exige `--test-threads=1`);
   - **`continue-on-error: true` removido** — vermelho no Windows agora BLOQUEIA;
   - **`timeout-minutes: 25 → 40`**, calibrado por medição: a suíte completa serializada leva
     **179s de execução no macOS local**; o job Windows atual (compile+clippy apenas) já cabia
     em 25min; runner Windows é 2-4× mais lento na execução (estimativa ≤ 12min de testes) ⇒ 40min
     dá margem honesta sem mascarar hang (um teste pendurado ainda estoura e falha alto);
   - comentários do job reescritos: o estado "INFORMATIVO até o bring-up" virou histórico; o novo
     comentário aponta para esta tabela e instrui: *job estourou timeout ⇒ triague o teste novo
     na tabela antes de aumentar o timeout*.
2. **Cabeçalho do arquivo** atualizado (estado dos gates 2026-06-10; os 5 jobs são obrigatórios;
   checklist do run real → §4 daqui).
3. **Furo de gate fechado no `core-macos` (ajuste adjacente, explícito):** a lista de `fmt`
   por-pacote não incluía `lina-session-watch`, `lina-hooks` e `lina-otel` (crates entraram no
   workspace depois da lista) — fmt-drift neles passava sem gate. Os 3 foram verificados
   fmt-limpos localmente (exit 0 cada) e adicionados à lista.

## §3 Falso-positivos derrubados na re-derivação (registro para o red-team F1-6-9)

A rodada adversarial desafiou 79 vereditos "portável"; a re-derivação manual derrubou **TODOS os
desafios em massa** — e o padrão importa para quem auditar depois:

- **`let _ = std::fs::remove_dir_all(…)` é deliberado e portável.** As fixtures de teste
  (`TmpStore` do router, `TempDir` de lifecycle/events, cleanup dos E2E de webhooks, instaladores
  do bootstrap) ENGOLEM o erro de remoção. No Windows, deletar diretório com handle SQLite/JSONL
  vivo falha ⇒ o temp dir **vaza** (irrelevante: runner é VM efêmera) — mas o teste **passa**.
  Refutadores assumiram falha sem checar o tratamento do erro: 69 testes do router + 11 do
  lifecycle + 5 E2E de webhooks + 7 do bootstrap eram falso-positivo.
- **Path Unix como DADO não é dependência de SO.** `"/usr/local/bin/claude"` ou
  `"/Users/founder/projeto"` guardados em payload de evento e comparados consigo mesmos
  round-tripam idênticos em qualquer SO (nunca são dereferenciados no filesystem).
- **`PtyHost::new()` não toca ConPTY** — init puro de structs; só `spawn()` abre PTY. O teste
  `pty_port_missing_node_errors_cleanly` roda nos 3 SOs.
- **`nenhum_segredo_em_claro_no_disco`** (lina-secrets) varre a árvore do repo MAS pula
  `target/`/`.git/` e engole erros de leitura (`let Ok(…) else return`) — portável; custo extra
  de I/O no runner Windows anotado como risco de duração, não de falha.

## §4 O que SÓ o run real no GitHub prova (checklist pós-push — o push é do Maestro)

- [ ] **Os 5 jobs verdes** no run, com `core-windows` EXECUTANDO (log mostra `running N tests`/
  `test result: ok` por binário de teste — não só `Compiling`/`Finished`). Critério (a) da story.
- [ ] **Clippy da visão Windows-msvc verde** — validado localmente em cross via target *gnu*
  (mesma resolução de cfg); a confirmação msvc 1:1 é do runner.
- [ ] **Os 4 testes do `lina-pty` passam no ConPTY do runner** (`cmd.exe /Q /K` via
  `portable-pty`) — nunca executados num runner GitHub; passaram num Windows real no de-risk
  do bring-up. Se falharem aqui, é ambiente do runner (ex.: política de console), não regressão.
- [ ] **Duração do `core-windows` dentro de 40min com margem** — anotar a duração real do
  primeiro run e apertar o timeout se sobrar muito (alvo: timeout ≈ 2× a duração observada).
- [ ] **3 runs consecutivos verdes** (critério (c) — estabilidade/anti-flake; re-run do mesmo
  commit conta). Atenção à lição registrada: runs Windows são cancelados por push concorrente —
  não confundir `cancelled` com flake.
- [ ] **Link do run** colado no relatório da onda (critério (a)).
- Riscos nomeados a observar no log: duração de `nenhum_segredo_em_claro_no_disco` (I/O da árvore
  no disco do runner); asserts de timing com margem 50-60ms em `a2a.rs` (negativos com folga,
  improvável flakear — mas se flakear, está nomeado aqui).

## §5 Cobertura headless do roteiro `WINDOWS-QA-POS-MERGE.md`

O roteiro é majoritariamente de **olho humano na tela** (gpui não roda headless — limite conhecido,
documentado no ci.yml). O que o gate F1-6-8 passa a cobrir automaticamente a cada push:

| Item do roteiro | Cobertura headless agora |
|---|---|
| Teste 1 — detecção do Obsidian (`%APPDATA%\obsidian\obsidian.json`) | unit-tests de parsing/caminhos rodando NO Windows (antes: só compilavam) |
| Teste 2 — instaladores de CLI no ConPTY | receitas por SO (`CURRENT_OS=windows`, TOMLs de installer) testadas no Windows; o ConPTY em si: 4 testes do `lina-pty` com `cmd.exe` | 
| Teste 3 — ferramentas dev no PATH | lógica de detecção (`cli_discovery`) roda no Windows (probes reais são `cfg(unix)`; mocks `cfg(windows)` cobrem o parse) |
| Teste 4 — `lina vault` | testes do bin `lina` (bootstrap, 86 no Windows) cobrem resolução de paths/ficha/outbox |
| Teste 5 — atalhos | fora de alcance headless (gpui) — segue olho humano |
| ⚠️ aviso do roteiro: "NÃO rode `cargo test --workspace`… penduram" | **OBSOLETO com esta story** — pós-merge, `cargo test --workspace` é verde no Windows (os PTY-Unix estão gateados). Atualizar o aviso é 1 linha no doc do QA (fora da minha fronteira; pedido registrado na entrega). |

## §6 Validação local (evidência — exit codes diretos, sem pipe)

- Baseline pré-mudança: `cargo test --workspace -- --test-threads=1` → **exit 0**, 568 passed /
  0 failed / 1 ignored, **179s** (macOS, serializado).
- Pós-mudança (mesma máquina): `test_exit=0` (**598 passed / 0 failed / 1 ignored**, 189s) ·
  `clippy_mac_exit=0` · **`clippy_win_exit=0`** (visão Windows via `--target x86_64-pc-windows-gnu`,
  workspace + all-targets, `-D warnings`) · `cargo fmt -p` **exit 0 nos 12 crates** da lista nova.
- O salto 568→598 passed entre os dois runs NÃO é desta story: o working tree do canvas é
  compartilhado e colegas adicionaram testes (webhooks/events) entre os dois runs — todos verdes
  com os gates aplicados. Testes novos seguem a regra do ci.yml: *novo teste com spawn/PTY ⇒
  triagem nesta tabela antes de qualquer aumento de timeout*.
- O cross-clippy pegou, em tempo real, exatamente o tipo de erro que esta análise existe para
  evitar: 1 import órfão (`use serial_test::serial` em `lib.rs`, mod do PtyHost) — corrigido com
  gate e re-validado verde. Os demais 13 gates passaram limpos de primeira.
