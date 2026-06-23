# Onda F4-0 — Substrato de canais & credenciais (peça executável)

> **Fonte da verdade story-a-story** da onda F4-0. Consolida o épico [[41 - Epico Fase 4 — Integracoes Canais e Contexto]] §III. **Precedência:** esta peça vence o §II do épico em detalhe; o ADR aceito vence ambos. **Maestro da rodada:** Maestro 00.
> **Meta da onda:** construir o *cano e o estoque* antes de qualquer canal específico — área de credenciais custodiadas + registro de canais (porta de continuidade) + pré-config de ferramentas + sinalização honesta. Nada vaza por padrão.

## Estado de partida (reconhecido no código, 2026-06-23)

- ✅ **`lina-secrets`** completo: `SecretVault` (set/get/delete/ensure_webhook_secret), trait `SecretStore`, `KeyringStore` (prod), `MockStore` (teste headless). Chave física = `("{namespace}:{scope}", key)`. Precedentes de scope: `webhook/hook_id`, `deploy/prod`, `payment`, `external`.
- ✅ **broker** `crates/lina-core/src/broker.rs`: `run_custody()` já faz o fluxo do gate — `ActionGated{class:"gated-hard-external", decision:"ask"}` → `BrokerDenied{unconfirmed}` (sem "sim") → `BrokerDenied{no_secret}` (sem segredo) → `execute(&secret)` → `BrokerExecuted`. Registry estático `CUSTODY_ACTIONS` (deploy/pay/send). Lado-agente: `lina do <ação>` em `lina.rs:run_do` enfileira em `<LINA_HOME>/broker/`; o agente **nunca** executa.
- ✅ Eventos `ActionGated`/`BrokerExecuted`/`BrokerDenied`/`WebhookConfigured` existem.
- ✅ **defesa de env existente** (W3-6c): `SECRET_ENV_DENYLIST` + `scrub_pty_secret_env()` (`app/lina-gpui/src/bridge.rs:3327/3342`), chamada no boot (`runtime.rs:521`) ANTES de qualquer spawn. `node_identity_env` (`bridge.rs:3268`) carimba só identidade (`LINA_NODE_*`/`LINA_HOME`/`LINA_AUTONOMY`/`LINA_EFFORT`) — nenhum segredo.
- ❌ **greenfield:** trait `Channel` / `ChannelRegistry` / manifesto (não existe nada).
- ❌ **gap crítico:** `lina-pty` **não** faz `env_clear` (o PTY filho herda o env do app); a contenção hoje é só denylist no env do app. **Não existe teste de DUMP DE ENV do processo-filho real** — é o critério inforjável do gate (a) que esta onda precisa construir.
- ✅ ADRs-gate presentes: 0004 (custódia, **aceito**), 0006, 0007, 0034, 0035, 0043. F4-0 **consome** 0004; não exige ADR novo bloqueante.

## LARGADA (feita pelo Maestro 00 — dono único, base commitada)

1. **`events.rs`** — 5 eventos aditivos congelados (contrato): `CredentialStored{channel,scope,key_ref}`, `CredentialRevoked{channel,key_ref}`, `ChannelRegistered{channel,manifest_ref,trust_tier,install_ref}`, `ToolScopeDeclared{project,channel,scope}`, `ToolScopeRevoked{project,channel,scope}` + arms em `kind()` + chain META no `apply` + teste de round-trip/kind/META (`f4_0_substrate_events_roundtrip_and_kind_matches_serde_tag`).
2. **`lib.rs`** — `pub mod channel;` `pub mod tool_scope;` (dono único Maestro; ninguém edita lib.rs).
3. **Stubs** `crates/lina-core/src/channel.rs` e `tool_scope.rs` (header rico com o norte da frente).

### Decisões registradas (Maestro — divergências da spec, justificadas)

- **D1 — sem `*_at` redundante no payload.** O §IV do épico listava `stored_at`/`revoked_at`/`declared_at`. **Decisão:** OMITIDOS. O instante de cada fato já vive no `ts` do `EventRecord` (carimbado pelo `EventStore`); duplicá-lo viola DRY e a convenção dominante de `events.rs` (`ClueSetDefined`/`SkillSelected` não duplicam ts). Exceção da casa (`WebhookReceived.ts`) só existe quando o instante é semanticamente DISTINTO do de persistência — não é o caso aqui. (Se uma frente precisar do instante, lê do `EventRecord`.)
- **D2 — `broker.rs` e `lina-secrets` NÃO ganham módulo novo.** F4-0-3 estende `CUSTODY_ACTIONS` + executor; F4-0-2 estende o cofre com helper de scope de canal. Reuso > criação (épico §IV "reusar eventos existentes").

## As 6 stories — território de arquivo DISJUNTO por frente

> Costuras com dono único: `events.rs`/`lib.rs` = Maestro (já feito na largada). Nenhuma frente toca a costura. `app/lina-gpui/src/main.rs` (fiação de UI) = **dono único Especialista em Telas** nesta rodada.

### F4-0-1 — `ChannelRegistry` + trait `Channel` · dono **Terminal B (Ultra Code)** · TAG [10]
- **Território:** `crates/lina-core/src/channel.rs` (preencher o stub) + `channels/<stub>/manifest.toml` (exemplos) + testes inline.
- **Entrega:** trait `Channel` (porta de continuidade) + manifesto TOML (`transport`/`auth`/`scopes`/`tools.default_enabled`/`install.ref` pinado) lido e validado por `serde` ANTES de executar + `ChannelRegistry` (projeção pura por replay de `ChannelRegistered`, padrão `ClueSet`) + trust default-deny por pertencimento (`trust_tier ∈ {core,curado,comunidade}`).
- **Observável:** registrar um canal-stub → aparece no registro como "declarado, não conectado"; manifesto malformado → falha no schema (teste), **nunca executa**. (Absorve o atributo `trust_tier` que F4-0-6 reusa.)
- **Invariante:** manifesto é DADO, jamais autoridade; `install_ref` pinado (nunca HEAD).

### F4-0-2 (core) — Credencial → keyring + **teste de dump de env** · dono **Terminal I** · TAG [10]+ADR0004
- **Território:** `crates/lina-secrets/` (helper de scope de canal: `set_channel_credential`/`get`/`revoke`) + binding que emite `CredentialStored`/`CredentialRevoked` (em `crates/lina-core/` — coordenar ponto de fiação com o Maestro, **não** tocar `events.rs`) + **teste headless de dump de env** em `crates/lina-core/tests/f4_0_cred_env_dump.rs` (NOVO).
- **Entrega-coração (gate (a), critério inforjável):** um teste que SPAWNA um PTY de agente (reusar o harness de `examples/permission_probe.rs` / `lina-pty`), roda `printenv`/`env` no PTY e **grepa a chave da credencial → 0 hits**. Espelha AC-0004.1. A credencial entra no cofre via `SecretVault`, indexada por canal/escopo; **o valor NUNCA entra no log nem no env** — só a referência (`key_ref`).
- **Observável:** subir credencial → `vault.get(scope, key)` a retorna E o dump de env do PTY = 0 hits + `CredentialStored{channel,scope,key_ref}` no log (valor ausente). Backup silencioso antes de sobrescrever (`.bak`, Hermes doc 40 §3). Sem TTY/CI → guia non-interactive, não trava.
- **Coordenar:** entrega o CONTRATO core (helper + binding + prova de segurança headless). A UI de upload é F4-0-2-UI (Especialista). Ponto de instanciação do `SecretVault` em produção: `runtime.rs:507/510` (hoje `MockStore` — propor a virada para `KeyringStore` namespaced ao workspace, coordenado com o Maestro).

### F4-0-2 (UI) + F4-0-5 (badge) — Modal de credencial + sinalização · dono **Especialista em Telas** · TAG [10]+[6]
- **Território:** `app/lina-gpui/src/credential_modal.rs` (NOVO, modal headless no padrão `agent_modal.rs`) + `app/lina-gpui/src/` componente de badge de exposição + fiação em `main.rs` (**dono único de main.rs nesta rodada**: abrir o modal via `self.credential_modal = Some(...)`, commitar via funil para o core, slot do badge no header).
- **Entrega:** UI de upload de credencial no canvas (modal, zero curses) → chama o contrato core de F4-0-2 (Terminal I) → keyring. Badge persistente "este Espaço fala com o mundo" enquanto ≥1 canal externo ativo (cor = feedback, doc 40 §10; diz QUAL canal, não genérico).
- **Observável (gate de tela do fundador, BLOQUEANTE):** abrir modal → digitar credencial → some no keyring; 0 canais → 0 badge; conectar canal-stub → badge aparece. `token_ratchet` intacto; zero jargão.
- **Deps:** contrato core de F4-0-2 (I) e o estado de "canal ativo" de F4-0-3 (K). Pode começar contra stub e integrar.

### F4-0-3 — Broker de canal `lina do <canal> …` · dono **Terminal K** · TAG [10]+[6]+ADR0004
- **Território:** `crates/lina-core/src/broker.rs` (estender `CUSTODY_ACTIONS`/executor para ações de canal) + `crates/lina-bootstrap/src/bin/lina.rs` (`run_do` aceitar `lina do <canal> <ação>`).
- **Entrega:** o efeito externo de qualquer canal é o verbo brokerado `lina do <canal> <ação>` (doutrina InsForge: verbo = ponto de gate). O app — e só ele — injeta o segredo do keyring no momento da execução; o agente nunca o vê. Ação externa irreversível dispara `ActionGated{class:"gated-hard-external", decision:"ask"}` em QUALQUER nível de autonomia (a autonomia afrouxa `gated-soft`, **nunca** `gated-hard` externo — ADR 0004 piso).
- **Observável (gate (b)):** `lina do <canal-stub> send` com o token só no keyring → `ActionGated{gated-hard-external, ask}` no log + bloqueio sem confirmação; env do agente não contém a chave. Reusa `run_custody`/`BrokerExecuted`/`BrokerDenied`.
- **Deps:** F4-0-1 (trait Channel — trabalhar contra a assinatura) + F4-0-2 (scope de credencial). Suíte de segurança do router VERDE é critério implícito.

### F4-0-4 — Pré-config de ferramentas/grupos por projeto · dono **Terminal M** · TAG [6]
- **Território:** `crates/lina-core/src/tool_scope.rs` (preencher o stub: projeção + `declare_tool_scope`/`revoke`) + camada de briefing (coordenar com `briefing.rs` — costura de outro dono; **diff mapeado + propor ao Maestro**, não editar à toa).
- **Entrega:** o leigo declara, por projeto, quais ferramentas/grupos a IA enxerga (`ToolScopeDeclared`) → entra no contexto que a IA lê. **Declarar ≠ autorizar** (fluxo, não autoridade — ação externa segue pelo broker+gate). Default-deny: não-declarado é invisível.
- **Observável (gate (c)):** declarar "grupo X" → a IA, ao perguntar, vê "você me deu acesso ao grupo X"; remover → o acesso some no próximo turno (push, sem restart).

### F4-0-5 (monitor) + F4-0-6 (catálogo/CI) — Monitor de rede honesto + curadoria · dono **Terminal G** · TAG [6]
- **Território:** `crates/lina-core/src/bin/network_monitor.rs` (NOVO, read-only, molde `plan_adoption.rs`) + `.github/workflows/ci.yml` (passo de validação de manifesto) + `crates/lina-core/tests/` teste de schema de manifesto. **Headless — não toca main.rs.**
- **Entrega (monitor, gate (d)):** prova local-first por AUSÊNCIA — enquanto 0 canais ativos, **0 conexão de saída atribuível ao Lina**. Indicador honesto: bin read-only que projeta os canais ativos do log (`ChannelRegistered`/`Connected`) e afirma "0 canais ativos → 0 sockets de saída esperados"; documentar a verificação externa (`lsof`/`nettop` no macOS = 0 conexões do processo Lina). Catálogo (F4-0-6): trust tiers (core auto / curado-por-PR / comunidade opt-in) reusando `trust_tier`; CI valida manifesto em merge-time (gate (e): manifesto malformado **barra o merge**).
- **Observável (gates (d)+(e)):** 0 canais → monitor reporta 0 + `nettop` confirma 0 conexões; manifesto inválido → CI vermelho.
- **Deps:** F4-0-1 (lê `ChannelRegistered`).

### F4-0-QA — Gate da onda · dono **Terminal R** · TAG transversal
- **Território:** `crates/lina-core/tests/f4_0_gate_*.rs` (NOVO) + red-team.
- **Entrega:** prova o GATE DE SAÍDA inteiro por evidência: (a) dump de env 0 hits (re-derivado), (b) `ActionGated{gated-hard-external,ask}` + bloqueio, (c) tool scope aparece/some, (d) monitor 0 conexões. **Red-team:** manifesto não é autoridade (campo de manifesto no caminho de permissão → recusado); credencial não vaza por caminho esquecido; declarar ≠ autorizar. Suíte de segurança do router VERDE por mutação. **0 ALTA para o gate passar.**

## GATE DE SAÍDA F4-0 (roda e se mede — entregável = evidência, não "testes passaram")
- **(a)** credencial real pela UI → no keyring E dump de env de PTV de agente spawnado **não a contém** (grep 0 hits, log com `CredentialStored` sem valor).
- **(b)** `lina do <canal-stub>` com efeito externo → `ActionGated{gated-hard-external, ask}` + bloqueio sem "sim".
- **(c)** declarar ferramenta por projeto → aparece no contexto da IA; removê-la → some sem restart.
- **(d)** 0 canais ativos → monitor de rede registra **zero conexão de saída** atribuível ao Lina.
- **(e)** manifesto de canal malformado **barra o merge** na CI.
- **Entregável:** relatório com o dump de env (credencial ausente) + trecho de log do `ActionGated` — não só "verde".

## Conselho de gate de onda (4 lentes, read-only, agentes que NÃO construíram): Visão/fios · Arquitetura (ADR↔código + invariantes 1-7 + portas) · Segurança (red-team, 0 ALTA, foco: nenhum segredo no env + nenhuma ação externa sem gate) · Pesquisa (fontes vs implementado).

## Convenções (CLAUDE.md repo): `cargo fmt`/`clippy -D warnings` limpos POR PACOTE · sem `unwrap()` em produção · eventos aditivos · workers NÃO commitam (Maestro valida de fora por exit code direto e commita por fatia) · validar app de DENTRO de `app/lina-gpui` (excluído do workspace).
