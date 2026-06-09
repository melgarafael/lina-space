# ADR 0025 — Lina respeita o permission-mode do usuário por padrão (produção não fixa `defaultMode`)

- **Status:** **Aceito** (Dev 03 + Maestro, 2026-06-09). Reverte a decisão **R2b** (pin de produção do `permissions.defaultMode`) do `_proposta-permission-mode.md`; refina o ADR 0021 (modelo de segurança da aprovação) ao separar **camada de UX/toast** de **camada de segurança durável**.
- **Onda/Story:** Fase 1 · bug de arquitetura (uso real do fundador) — a Lina sobrescrevia o `~/.claude/settings.json` do operador humano.
- **Data:** 2026-06-09
- **Fontes:** invariante #3 do norte (neutralidade multi-CLI) · invariante #6 (não-técnico-first) · ADR 0021 (aprovação y/n — a superfície F1-1 do toast) · ADR 0006 (injeção A2A default-deny / fronteira L1-3 same-uid) · ADR 0004 (custódia de segredo como gate duro) · W3-6 (gate de execução `PreToolUse` + `lina do`) · precedência de settings do Claude Code (settings de PROJETO vencem os de USUÁRIO) · `crates/lina-bootstrap/src/lib.rs` (`hook_settings_json_with_mode`, `sanitized_permission_mode`) · `app/lina-gpui/src/bridge.rs` (write do settings de projeto, ~2796; HOME herdado no spawn, ~3481) · `crates/lina-core/examples/permission_probe.rs` (o gate-test força o modo por `--permission-mode`, não pelo settings).

## Contexto

O fundador roda o `claude` com `defaultMode: bypassPermissions` no **próprio** `~/.claude/settings.json`. No terminal puro do Mac e no Maestri, o `claude` abre já em bypass — a escolha explícita do humano. **No Lina, o agente nascia PEDINDO permissão** ("auto mode"), ignorando essa escolha.

Causa-raiz (decisão **R2b**, deliberada à época para forçar o toast F1-1):

1. `sanitized_permission_mode` (lib.rs) tinha allowlist `default|plan|acceptEdits` e **rebaixava `bypassPermissions` para `default`** (teste `permission_mode_allowlist_never_silent_bypass`).
2. `hook_settings_json_with_mode` fixava `"permissions": { "defaultMode": <mode> }` no settings de **projeto** que o bootstrap escreve no cwd do agente (`<cwd>/.claude/settings.json`).
3. No Claude Code, **settings de PROJETO vencem os de USUÁRIO** (a "sonda C" da proposta R2b provou isso). Logo o `permissions.defaultMode = "default"` do projeto **atropelava** o `bypassPermissions` do `~/.claude/settings.json` do fundador. O próprio comentário R2b admitia: *"settings por-cwd VENCE o `bypassPermissions` de usuário… todo terminal nasce PEDINDO permissão."*

Isso era um meio para o gate-test do F1-1 (forçar prompts pro toast), mas **vazou para o uso real** e passou por cima da escolha EXPLÍCITA do operador humano sobre o seu próprio CLI — colidindo com o invariante #3 (a Lina **orquestra** um CLI neutro; não é dona da política de permissão do `~/.claude` do humano) e com o invariante #6 (atrito desnecessário para o não-técnico que já optou por bypass).

**Ponto-chave (separação de camadas):** o permission-mode do `claude` é a superfície de **UX/toast** (F1-1). A **segurança durável** do Lina é uma camada SEPARADA e CONTÍNUA: o guard **W3-6** (hook `PreToolUse` → `lina guard --pretooluse` + custódia `lina do`), a custódia de segredo (ADR 0004) e o `WorkspaceTrust` da injeção A2A (ADR 0006). Nenhuma delas depende do permission-mode do claude. Respeitar o bypass do usuário **não reabre buraco**: só silencia o toast F1-1 — exatamente o que o usuário que optou por bypass quer.

## Decisão

**Em PRODUÇÃO, o Lina NÃO fixa `permissions.defaultMode`. O `~/.claude/settings.json` do operador humano é a autoridade do seu próprio permission-mode.**

1. **Produção omite a chave `permissions` inteira.** `hook_settings_json_with_mode(lina_bin, wiring, permission_mode)` com `permission_mode == None` (o caminho de produção, via `hook_settings_json_with_observability` → usado pelo `bridge.rs` e por `write_terminal_files`) escreve o settings de projeto **sem** o bloco `permissions`. Pela precedência do Claude Code, o `defaultMode` do `~/.claude/settings.json` do usuário (incl. `bypassPermissions`, `plan`, `acceptEdits` — o que ele escolheu) **flui por merge**. Os demais settings de usuário (model, MCP, comandos) já fluíam: o projeto nunca os escreveu.

2. **Pedido EXPLÍCITO continua possível (seam preservado).** `permission_mode == Some(m)` (futuro mapa por papel na UI / gate-test) fixa `defaultMode = sanitized_permission_mode(Some(m))`. A allowlist passa a **incluir `bypassPermissions`** como valor válido **quando explicitamente pedido** — bypass EXPLÍCITO do humano ≠ bypass silencioso herdado que o R2b temia. Valores DESCONHECIDOS (`dontAsk`/`auto`/lixo) ainda degradam para `default` (fail-safe).

3. **Os hooks de segurança ficam SEMPRE.** Independente de `permission_mode`, o settings de projeto sempre carrega `hooks.SessionStart` (turno-0 W3-2 → `lina whoami --bootstrap`) e `hooks.PreToolUse` (matcher `Bash|Skill` → `lina guard --pretooluse`, o gate de execução W3-6 + guard de skill estrangeira). Observabilidade (handlers HTTP) continua aditiva via `wiring`. **A segurança durável é o guard, não o permission-mode.**

4. **HOME do spawn = HOME real do usuário (verificado, não suposto).** O `PtyCommand` herda o env do processo do app via `portable-pty::CommandBuilder` (`get_base_env()` semeia de `std::env::vars_os()`); o spawn só ADICIONA `VIBE_ROLE`, nunca limpa/sobrescreve `HOME`; os scrubs (`scrub_foreign_orchestrator_env`/`scrub_pty_secret_env`) PRESERVAM `HOME` (teste `foreign_orchestrator_var_matches_maestri_prefix_only`). Como o app roda como o usuário, `HOME = ~` real → o `~/.claude/settings.json` (defaultMode, model, MCP, comandos) carrega. **O único valor que era sobrescrito era o `defaultMode`** (pelo settings de projeto); o resto já fluía por merge.

## Por que omitir (e não escrever `bypassPermissions` no projeto)

Escrever `bypassPermissions` no settings de projeto seria a Lina **decidir** a política do usuário (e quebraria quem NÃO optou por bypass). **Omitir** devolve a decisão a quem é dono dela: o `~/.claude` do humano. Ausência da chave no projeto ⇒ o valor do usuário vence por construção, qualquer que seja (`default`, `plan`, `acceptEdits` ou `bypassPermissions`). É a tradução literal do invariante #3: a especificidade de política mora na config do humano, não no renderer do bootstrap.

## Segurança (red-team próprio — linguagem de invariantes)

- **O guard W3-6 continua ativo.** Os hooks `SessionStart` + `PreToolUse (Bash|Skill → guard --pretooluse)` são escritos SEMPRE (provado por `production_settings_omit_permissions_so_user_choice_flows` e `write_terminal_files_production_omits_permissions_keeps_hooks`: o settings sem `permissions` ainda contém `whoami --bootstrap` e `guard --pretooluse`). A suíte `gate_onda3` (8/8) e os `guard_cli`/`shim_cli` do bootstrap seguem verdes — o gate de execução e a custódia `lina do` não foram tocados.
- **Respeitar o bypass do usuário NÃO dá ao AGENTE um auto-bypass.** A fonte do bypass é o `~/.claude/settings.json` do **humano** — um arquivo **fora** do cwd do agente, que o agente não escreve. O parâmetro `permission_mode` não tem **nenhum** caminho controlável por agente: ele é `None` em produção e só vira `Some` por config do app (humano) ou flag de teste. **Nenhum campo de payload/grid/`from`/filename do agente** alcança esse parâmetro (doutrina da casa, herdada de ADR 0021 §5 / 0006). Logo o agente não ganha capability nova: ele já tinha exatamente isto num terminal puro do Mac.
- **Bypass do permission-mode ≠ bypass da segurança do Lina.** Mesmo com `bypassPermissions`, toda chamada `Bash` passa pelo `PreToolUse → lina guard` (W3-6) e toda ação `gated-hard`/segredo exige a custódia (ADR 0004); a injeção A2A segue `WorkspaceTrust` default-deny (ADR 0006). O permission-mode do claude é UX; o enforcement do Lina é o hook.
- **Fronteira herdada (consciente):** same-uid não é fronteira de SO (ADR 0006 §Limite / 0021 §Limite L1-3) — um processo de mesmo uid já podia escrever no PTY por fora do Lina. Inalterada por este ADR.

## Limite explícito

- O toast F1-1 (ADR 0021) **só se aplica** quando o usuário roda um modo que pede permissão (`default`/`plan`/`acceptEdits`). Quem optou por `bypassPermissions` no próprio `~/.claude` não verá o toast de permissão — é a consequência desejada da escolha dele (a observabilidade dos hooks continua; o gate W3-6 continua).
- A precedência "projeto vence usuário" do Claude Code é a base do mecanismo (omitir ⇒ usuário vence). Se uma versão futura do Claude Code mudar essa semântica de merge, este ADR deve ser reavaliado.
- O mapa por papel da UI (pedido EXPLÍCITO de modo) é seam preservado, **não** implementado aqui.

## Alternativas rejeitadas

- **Manter o pin R2b (`defaultMode = default` em produção)** — rejeitado: atropela a escolha explícita do operador humano sobre o próprio CLI (viola invariante #3) e injeta atrito no não-técnico (invariante #6). O objetivo do R2b (forçar o toast para o gate-test) é alcançado pelo probe via `--permission-mode default` (flag de CLI), sem pin no settings de produção.
- **Escrever `bypassPermissions` no settings de projeto** — rejeitado: a Lina passaria a DECIDIR a política do usuário e quebraria quem não optou por bypass. Omitir é neutro.
- **Acoplar a segurança do Lina ao permission-mode do claude** — rejeitado: confunde camada de UX (toast) com camada de enforcement (guard W3-6/custódia). A segurança durável não pode depender de um knob do CLI de terceiro (invariante #3/#7).
- **Remover o seam de pedido explícito (`Some(m)`)** — rejeitado: o mapa por papel futuro e o gate-test precisam fixar um modo deliberadamente; o seam é seguro porque `m` nunca vem de campo de agente.

## Critério de verificação (evidência exigida)

- **AC-0025.1 (produção respeita o usuário):** `hook_settings_json_with_observability(bin, None|Some(wiring))` e o arquivo escrito por `write_terminal_files` **não** contêm a chave `permissions` (provado: `production_settings_omit_permissions_so_user_choice_flows`, `write_terminal_files_production_omits_permissions_keeps_hooks`).
- **AC-0025.2 (guard sobrevive):** o MESMO settings sem `permissions` contém `SessionStart → whoami --bootstrap` e `PreToolUse (Bash|Skill) → guard --pretooluse`; `gate_onda3` 8/8 verde.
- **AC-0025.3 (modo explícito):** `sanitized_permission_mode(Some("bypassPermissions")) == "bypassPermissions"`; `default/plan/acceptEdits` passam; `dontAsk/auto/garbage → default`; `hook_settings_json_with_mode(.., Some("bypassPermissions"))` fixa o bypass, `Some("garbage")` degrada, `None` omite (`permission_mode_explicit_request_honored_unknown_degrades`).
- **AC-0025.4 (sem auto-bypass do agente):** busca no código — nenhum call-site passa `Some(m)` derivado de campo de agente; o único caller de produção (`bridge.rs:2796`) passa `None`; o probe força por `--permission-mode` (flag), não pelo settings.
