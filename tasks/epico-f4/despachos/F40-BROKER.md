# DESPACHO F40-BROKER — F4-0-3 · Broker de canal `lina do <canal> …` (dono: Terminal K)

## CONTEXTO (leia ANTES de codar — obrigatório)
Onda **F4-0** da Fase 4 do Lina. O efeito externo de QUALQUER canal é um **verbo brokerado**: `lina do <canal> <ação>` (doutrina InsForge — o verbo é o ponto de gate, nunca o agente chamando a API direto). O app — e SÓ ele — injeta o segredo do keyring no momento da execução; **o agente nunca vê o segredo**. Ação externa irreversível dispara **gate humano** em qualquer nível de autonomia (a autonomia afrouxa `gated-soft`, JAMAIS `gated-hard-external` — ADR 0004 é o piso).

**Documentos a conferir (navegue — NÃO pule):**
1. **Peça da onda:** `tasks/epico-f4/onda-0.md` — seção **F4-0-3**.
2. **Épico no vault:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — critério **F4-0-3** (§III) + a "Âncora transversal" (doutrina InsForge) no §II.
3. **ADR 0004** (`docs/adr/0004-custodia-de-segredo-como-gate-duro.md`) — o gate duro real.
4. **O broker que JÁ existe (você ESTENDE, não reescreve):** `crates/lina-core/src/broker.rs` — leia `run_custody()` (linhas ~149-211), `CUSTODY_ACTIONS` (~47-63, deploy/pay/send), `BrokerRequest`, `BrokerOutcome`. E o lado-agente: `crates/lina-bootstrap/src/bin/lina.rs` `run_do()` (~3686-3763) — enfileira em `<LINA_HOME>/broker/`, o agente NUNCA executa.

## FUNÇÃO
Você é o **Dev Core (A2A/custódia)** desta frente. **Reuso máximo:** o motor do gate (`run_custody` → `ActionGated{gated-hard-external,ask}` → `BrokerDenied{unconfirmed/no_secret}` → `execute(&secret)` → `BrokerExecuted`) JÁ está pronto e testado. Sua frente é **generalizar `lina do <ação>` para `lina do <canal> <ação>`** — fazer o registry de custódia acomodar ações de canal, com o segredo vindo do scope do canal (F40-CRED). Não reinvente o gate; pendure o canal nele.

## DIRECIONAMENTO (território + como trabalhar)
- **Território (SÓ estes):** `crates/lina-core/src/broker.rs` (estender o registry/executor para ações de canal) + `crates/lina-bootstrap/src/bin/lina.rs` (`run_do` aceitar a forma `lina do <canal> <ação>`).
- **NÃO TOQUE:** `events.rs`/`lib.rs` (congelados — `ActionGated`/`BrokerExecuted`/`BrokerDenied` já existem e você os REUSA; não crie evento novo). Não toque `channel.rs` (é de B — você USA a trait `Channel`).
- **Worktree:** `git worktree add ../lina-f4-0-broker -b lina/f4-0-broker` da `main` (`fcb9371`). `git`/`cargo` de lá; `lina` do dir de produção.
- **Dependências (trabalhe contra o CONTRATO, integre no fim):**
  - **F40-CHAN (B):** você precisa da trait `Channel`/`ChannelRegistry` para resolver `<canal> → manifesto/scope`. Comece a estrutura contra a **assinatura** (a peça F4-0-1 descreve os campos do manifesto: `scopes`, `auth`); quando B publicar a assinatura (o Maestro avisa), integre. Se travar de verdade, reporte BLOCKED.
  - **F40-CRED (I):** o segredo do canal vive no scope `channel:<nome>` do `SecretVault`. A custódia injeta esse segredo (o broker já lê do vault por scope/key — você só aponta para o scope do canal).
- **Entregue:**
  1. **`lina do <canal> <ação>`** no `run_do` — parse da forma com canal; valida o canal no `ChannelRegistry` (declarado?) e a ação no manifesto (`scopes`/`tools`); monta `BrokerRequest` com `secret_scope = "channel:<nome>"`.
  2. **gate inquebrável:** ação externa de canal → `ActionGated{class:"gated-hard-external", decision:"ask"}` em QUALQUER autonomia + bloqueio sem confirmação (reusa `run_custody`). O agente registra, o app executa só após o "sim", injetando o segredo do keyring — **o env do agente nunca contém a chave**.
  3. Mantenha o lado-agente em `lina.rs` enfileirando em `<LINA_HOME>/broker/` (o agente nunca executa — `lina-bootstrap` nem linka `lina-secrets`).
- **Invariantes (red-team re-deriva — NÃO regrida):** nenhum campo escrito por agente (`from`/canal/payload) decide autorização; a custódia é o piso que a autonomia não afrouxa; **a suíte de segurança do router VERDE é critério implícito** (rode-a). Sem `unwrap()` em produção.
- **Convenções:** `cargo fmt` + `clippy -p lina-core`/`-p lina-bootstrap --all-targets -D warnings` limpos.

## OBJETIVO (critério observável — gate (b))
`lina do <canal-stub> send` com o token só no keyring → `ActionGated{gated-hard-external, ask}` no log + **bloqueia sem confirmação** (0 execução) + o env do agente não contém a chave. Teste de integração (molde: `crates/lina-bootstrap/tests/broker_cli.rs` `lina_do_deploy_registers_and_blocks_without_executing`) provando ActionGated + BrokerDenied{unconfirmed} + ZERO BrokerExecuted sem "sim".

## RESULTADO ESPERADO
- Não commite. Trabalho na worktree `lina/f4-0-broker`.
- Cole exit codes: `cargo test -p lina-core broker`, `cargo test -p lina-bootstrap broker_cli`, suíte de segurança do router (`cargo test -p lina-core router` ou o alvo de segurança), `cargo clippy --all-targets` (exit 0).
- Reporte: **`PRONTO: F40-BROKER`** + resumo (com o trecho de log do ActionGated) — OU **`BLOCKED: F40-BROKER`** + o quê (ex.: esperando assinatura de CHAN). Via `lina ask "@Maestro 00" "<...>" --intent status`.
