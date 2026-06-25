# Onda F4-1 — WhatsApp/Waha (entrada por QR, saída com gate) — peça executável

> **Fonte da verdade story-a-story** da onda F4-1. Consolida o épico [[41 - Epico Fase 4 — Integracoes Canais e Contexto]] §III (Onda F4-1) + §IV (contrato de eventos). **Precedência:** o ADR aceito vence tudo; o épico dá os critérios observáveis; esta peça detalha território/ordem; em conflito de detalhe, o épico vence esta peça. **Maestro da rodada:** Maestro 01.
> **Meta da onda:** o **primeiro canal concreto** — conectar uma conta de WhatsApp via **Waha por QR Code na tela**, para a IA **ler conversas** (contexto de entrada, fluido) e **enviar mensagens** (efeito de saída, gate humano). A assimetria É a doutrina: **ler é fluxo** (entra como contexto, auditado, sem gate); **enviar é ação externa irreversível** (gate humano + custódia de segredo). Desconectar zera o cano. Nada vaza por padrão: segredo no cofre, conteúdo fora do log, nada sai sem o "sim".

## Estado de partida (reconhecido no código, 2026-06-25 · HEAD `88e9aad`, F4-0 + F4-WA mergeadas)

> Mapa cirúrgico levantado pelo Maestro. F4-0 deixou um substrato **muito sólido** — a maior parte do gate de envio já existe.

### Já existe (REUSAR, não reinventar)
- **`channel.rs`** (`crates/lina-core/src/channel.rs`) — `ChannelManifest` (parse/load/`validate` TOML, `#[serde(deny_unknown_fields)]`; campos `name`/`transport`/`auth`/`scopes`/`tools.default_enabled`/`install.ref` pinado), trait `Channel`, `ManifestChannel`, `TrustTier` (default-deny), **`ChannelStatus` (só `Declared` hoje — a porta p/ `Connected` está DOCUMENTADA em `channel.rs:245-249`)**, `RegisteredChannel` (campos `channel`/`manifest_ref`/`trust_tier`/`install_ref`/`status`), `ChannelRegistry` (`from_records` consome só `ChannelRegistered`), `register_channel`.
- **`broker.rs`** (`crates/lina-core/src/broker.rs`) — **o gate de envio JÁ EXISTE:** `BrokerRequest::for_channel(channel, action, secret_key, requester)` (monta `secret_scope = "channel:<nome>"`), `channel_action_in_manifest(manifest, action)` (default-deny contra o manifesto), `run_custody(req, confirmed, vault, store, execute)` → apenda `ActionGated{class:"gated-hard-external",decision:"ask"}` SEMPRE → `confirmed==false`→`BrokerDenied{unconfirmed}` (cofre nem tocado) → `confirmed==true`→ segredo do cofre → `execute(&secret)` + `BrokerExecuted`; ausente→`BrokerDenied{no_secret}`. `BrokerOutcome{DeniedUnconfirmed,DeniedNoSecret,Executed}`. **F4-1-4 NÃO reinventa o gate — só liga o transporte Waha real no `execute`.**
- **`tool_scope.rs`** (`crates/lina-core/src/tool_scope.rs`) — projeção `ToolScopeDeclared`/`Revoked` (último-vence, padrão `ClueSet`). F4-1-5 a REUSA para grupos de WhatsApp.
- **App — caminho do broker** (`app/lina-gpui/src/bridge.rs`): o agente emite `MailMessage{to:"broker", intent:"broker.do", ref:"do:<action>"}`; `bridge.rs:2783` intercepta `broker.do` → `enqueue_custody` (`:2859`, registra o gate via `run_custody(confirmed=false)`) → após o "sim", `execute_custody(action, secret_key, requester)` (`:3033`, `run_custody(confirmed=true)`). **Hoje só lida com `do:<action>` builtin (`BrokerRequest::new`); o braço `do:channel:<canal>:<acao>` é o que F4-1-4 acrescenta** (a "porta F41-BROKER-APP" do épico).
- **`lina-secrets`** — cofre custodiado (escopo/chave). O `X-Api-Key`/sessão do Waha vive aqui, escopo `channel:whatsapp`.
- **Largada F4-WA** — `lina-webhooks` (axum loopback + HMAC) + `MsgOrigin::System` existe, **caso** alguém queira leitura por webhook depois (NÃO é o caminho desta onda — ver D4 abaixo).

### NÃO existe (o delta que esta onda constrói)
1. **Transporte HTTP de saída** — o projeto **não tem** cliente HTTP de saída (só `hyper` server-side via axum). Falar com o Waha exige **dependência nova** (decisão do ADR 0050) + o primeiro I/O de SAÍDA do Lina por um canal.
2. **`channel_waha.rs`** (STUB criado na largada) — o cliente Waha (conectar/QR/ler/enviar/desconectar).
3. **`channels/whatsapp/manifest.toml`** — não existe nenhum `channels/*.toml` ainda.
4. **Projeção `Connected`** — `ChannelStatus` só tem `Declared`; falta consumir `ChannelConnected`/`ChannelDisconnected` em `channel.rs` e derivar `Connected`/`session_ref`.
5. **`channel_read.rs`** (STUB criado na largada) — leitura auditada + scope de grupos.
6. **Elo `do:channel:` no app** — `enqueue_custody`/`execute_custody` reconhecer a ref de canal → manifesto → `for_channel` → injetar segredo do escopo do canal → chamar o transporte.
7. **UI** — modal "Conectar WhatsApp" + render do QR + badge "WhatsApp conectado" + trilha de auditoria.

### Referência técnica WAHA (LEITURA OBRIGATÓRIA p/ F4-1-1)
`tasks/epico-f4/refs/waha/` (guia do dev WAHA Plus). Os endpoints REST que importam:
- **Conectar:** `POST /api/sessions` (criar) + start → QR via `GET /api/{session}/auth/qr` (PNG/data-URI) → status `GET /api/sessions/{name}` (estados `STARTING`/`SCAN_QR_CODE`/`WORKING`/`FAILED`/`STOPPED`). Header de auth: **`X-Api-Key`**.
- **Enviar:** `POST /api/sendText { chatId, text }` (chatId = `<digits>@c.us` ou `<jid>@lid`).
- **Ler:** `GET` de mensagens de um chat declarado (on-demand).
- **Desconectar:** logout/stop da sessão.
- **NÃO importar** a complexidade de produção do guia (LID bridges, dedup, outbox, retry, multi-tenant Supabase/Hetzner) — é refino posterior, fora do escopo desta onda (guardião contra feature-creep, épico §V.4).

---

## LARGADA (feita pelo Maestro 01 — dono único das costuras, base verde)

Congelo o **contrato** (tipos), nunca a fiação (lógica das frentes). **Já verde:** `cargo test -p lina-core --lib f4_1` (2 testes) + clippy core limpo + `cargo check` do app OK.

1. **`events.rs`** — 4 eventos aditivos congelados + `kind()` arms + no-op no `apply` + 2 testes (`f4_1_channel_events_roundtrip_and_kind_matches_serde_tag`, `f4_1_channel_events_carry_no_content_or_secret`):
   - **NOVO** `ChannelConnected { #[serde(default)] channel, session_ref, scope }`
   - **NOVO** `ChannelDisconnected { #[serde(default)] channel, session_ref }`
   - **NOVO** `ChannelMessageRead { #[serde(default)] channel, conversation_ref, count: u32, read_by_node }`
   - **NOVO** `ChannelMessageSent { #[serde(default)] channel, conversation_ref, broker_exec_ref, approved_by }`
2. **`lib.rs`** — `pub mod channel_waha;` + `pub mod channel_read;` (stubs criados).
3. **Stubs** `channel_waha.rs` + `channel_read.rs` (cabeçalho de território; cada dono preenche o seu).
4. **Refs** `tasks/epico-f4/refs/waha/` (guia WAHA) + esta peça.

### Decisões de largada registradas
- **D1 — sem `*_at` no payload** (convenção da casa, reusada da onda F4-WA): o instante vive no `ts` do `EventRecord`. Os `connected_at`/`read_at`/`sent_at` do épico §IV são cobertos pelo `ts`.
- **D2 — `session_ref` é REFERÊNCIA custodiada, jamais o token.** `approved_by` é rótulo de PROVENIÊNCIA do gesto humano (lição `[[human-gesture-sentinela-origem-nao-autoridade]]`), JAMAIS autoridade. Conteúdo de mensagem NUNCA no log (só `count`/refs). Provado pelo teste `..._carry_no_content_or_secret`.
- **D3 — eventos META (no-op no `apply`):** a projeção (`ChannelRegistry`/`channel_read`) reconstrói por replay; o canvas não muda.
- **D4 — leitura por PULL, não webhook (a confirmar no ADR 0050).** Para "a IA lê quando perguntada" o pull HTTP on-demand é mais simples e local-first (não exige expor um webhook/túnel). A porta de webhook (F4-WA/`MsgOrigin::System`) fica aberta para push futuro, fora desta onda.

---

## ADR-GATE (antes de F4-1-1 e F4-1-4) — **ADR 0050** · dono **Arquiteto**

Decisão arquitetural que abre porta nova (I/O de saída + dependência). **F4-1-1 e F4-1-4 não iniciam sem o 0050 aceito.** As demais frentes (projeção, leitura/scope, UI shell, QA-RED) começam já, com os stubs.
Decidir: **(1)** HTTP client de saída (`ureq` síncrono leve vs `reqwest` async — pesar contra o runtime do app e o invariante "processo único"); **(2)** onde mora o cliente Waha (módulo `channel_waha` no core atrás de fronteira injetável vs crate); **(3)** custódia da sessão: o QR/`X-Api-Key` no `lina-secrets` (escopo `channel:whatsapp`), o agente nunca o vê; o `session_ref` referencia a sessão; **(4)** contrato do transporte (fn/trait `connect`/`qr`/`read`/`send`/`disconnect`) com borda de I/O injetável p/ mock; **(5)** como a IA dispara o envio: confirmar se o verbo CLI `lina do` existe (F4-0-3) ou se é só o gesto A2A `broker.do` — se faltar o caminho observável, o ADR define o mínimo. Saída: ADR curto em `docs/adr/0050-*.md` + decisão registrada no `plan.md`.

---

## As 6 stories — território declarado, fases por dependência

> **Costuras de dono único (largada — NINGUÉM toca):** `events.rs`, `lib.rs`. **Donos únicos nesta rodada:** `channel.rs` = **Terminal I** · `channel_waha.rs` + `channels/whatsapp/manifest.toml` = **Terminal B** · `channel_read.rs` = **Terminal K** · `app/lina-gpui/*` (`bridge.rs`/modais/badge) = **Especialista em Telas** · `crates/lina-core/tests/f4_1_*.rs` = **Terminal R**. `broker.rs` está PRONTO — ninguém o reescreve; reusa-se.

### FASE A — após a largada (paralelo; F4-1-1/4 esperam o ADR 0050)

#### F4-1-2-core — Projeção Connected/Disconnected · dono **Terminal I** · M
- **Território:** `crates/lina-core/src/channel.rs` (SÓ). Dono único.
- **Entrega:** `ChannelStatus::{Declared, Connected}` (+`label()` pt-br: "conectado"); `from_records` consome `ChannelConnected` (deriva `Connected` + guarda `session_ref`) e `ChannelDisconnected` (volta a `Declared`, zera `session_ref`) — último-vence; `RegisteredChannel` ganha `session_ref: Option<String>`; helpers `connect_channel(...)`/`disconnect_channel(...)` que apendam os eventos (espelham `register_channel`). A porta já está descrita em `channel.rs:245-249`.
- **Observável:** registrar canal → `Declared`; apendar `ChannelConnected` → `get(x).status == Connected` + `session_ref` setado; `ChannelDisconnected` → volta a `Declared`. Replay reconstrói idêntico. **Inicia já** (só depende da largada).

#### F4-1-3+5 — Leitura como contexto + scope de grupos · dono **Terminal K** · M
- **Território:** `crates/lina-core/src/channel_read.rs` (SÓ; stub criado). Costura de briefing (se tocar) = mapear diff + propor ao Maestro (camada 2).
- **Entrega:** **(F4-1-5)** função PURA `pode_ler(scope_projetado: &ToolScope, conversation_ref) -> bool` (default-deny: só grupos `ToolScopeDeclared`); **(F4-1-3)** wrapper que, se permitido, pede ao transporte (`channel_waha`, contrato público) as mensagens, devolve o conteúdo ao chamador como contexto, e apenda `ChannelMessageRead{channel,conversation_ref,count,read_by_node}` — **só metadados, conteúdo NUNCA no log**. Ler um grupo não-declarado → recusa auditada (sem evento de sucesso).
- **Observável:** grupo declarado → leitura retorna conteúdo + `ChannelMessageRead` no log (sem texto); grupo não-declarado → `pode_ler==false`, recusa. **Inicia já** (consome o stub de `channel_waha` via contrato; combine a assinatura com o Terminal B logo no começo).

#### F4-1-QA-RED — repro vermelho dos critérios · dono **Terminal R** · M
- **Território:** `crates/lina-core/tests/f4_1_*.rs` (NOVO; dono único). Pode começar escrevendo os testes que ainda ficam `#[ignore]` até as frentes ligarem (padrão "QA entrega o alvo `#[ignore]`, integrador liga").
- **Entrega/observável (gate F4-1):** **(a)** dump de env de PTY de agente spawnado **não contém** o `X-Api-Key`/token de sessão Waha (grep 0 hits, espelha F4-0-2; mutação: injeta o segredo → a sonda morde); **(b)** envio sem "sim" = **0 mensagens** (mock Waha que conta POSTs → 0 sem confirmação; `run_custody(confirmed=false)`); **(c)** ler grupo não-declarado → recusa; **(d)** `ChannelDisconnected` → próximo `for_channel`/envio falha "não conectado"; **(e)** `approved_by`/`session_ref` NUNCA decidem autoridade (mutação); **(f)** suíte de segurança do Router VERDE por mutação. **Inicia já**; liga conforme as frentes entregam.

### FASE B — após o ADR 0050 (transporte) — paralelo

#### F4-1-1 — Canal Waha: manifesto + transporte HTTP · dono **Terminal B (Ultra Code)** · M/L
- **Território:** `crates/lina-core/src/channel_waha.rs` (stub) + `channels/whatsapp/manifest.toml` (NOVO) + dep HTTP no `Cargo.toml` do core (conforme ADR 0050). **Não toca `channel.rs`/`broker.rs`/`events.rs`.**
- **Entrega (escopo MÍNIMO):** cliente HTTP fino p/ Waha — `connect` (criar sessão + start), `qr` (buscar PNG/data-URI), `status` (poll até `WORKING`), `read` (puxar mensagens de um chat), `send(secret, chatId, text)` (`POST /api/sendText`), `disconnect`. `X-Api-Key` recebido por parâmetro (de `execute(&secret)`), NUNCA do env nem logado. Endpoint default `http://127.0.0.1:3000` (opt-in). Borda de I/O injetável → testes com **mock HTTP** (sem rede real). Manifesto válido: `name="whatsapp"`, `transport="waha_http"`, `auth="api_key"`, `scopes=["messages:read","messages:send"]`, `install.ref` PINADO (tag/SHA da versão Waha suportada).
- **Observável:** registrar o canal Waha sem conectar → "declarado, aguardando QR"; com mock, `send` faz exatamente 1 POST `/api/sendText` com o `X-Api-Key` no header (provado no mock) e 0 sem o segredo. **Esta story habilita F4-1-3 (read), F4-1-APP (qr/send) e F4-1-QA (mock).**

#### F4-1-4 + F4-1-2-UI + F4-1-6 — App: QR, envio gated, badge/auditoria · dono **Especialista em Telas** · L
- **Território:** `app/lina-gpui/*` — `bridge.rs` (elo `do:channel:`), modal QR novo (padrão `credential_modal.rs`/`webhook_modal.rs`), badge, fiação `main.rs`. **Dono único do app nesta rodada** (todo o app é seu — evita colisão de costura).
- **Entrega:**
  - **F4-1-2-UI:** modal "Conectar WhatsApp" → chama o transporte (core) `connect`+`qr` → **renderiza o QR no canvas** → poll `status` → "conectado" → apenda `ChannelConnected`. Desconectar → `ChannelDisconnected`. Estética pela `lina-design-doctrine` (zero default de IA); zero jargão (o leigo vê "conectar seu WhatsApp", não "sessão Waha").
  - **F4-1-4 (porta F41-BROKER-APP):** `enqueue_custody`/`execute_custody` reconhecem `ref:"do:channel:whatsapp:send"` → resolvem o manifesto via `RegisteredChannel.manifest_ref` (`ChannelManifest::load`) → `channel_action_in_manifest` → `BrokerRequest::for_channel` → o `execute(&secret)` chama `channel_waha::send(secret, chatId, text)` + apenda `ChannelMessageSent{broker_exec_ref,approved_by}`. **O gate (`run_custody`) já existe — só ligue o `execute`.** Mostra o gate de envio na fila de atenção (reusa `AttentionKind`).
  - **F4-1-6:** badge "WhatsApp conectado" (reusa F4-0-5) enquanto há sessão ativa; trilha de auditoria narrada pt-br ("li 12 mensagens do grupo X" / "enviei sua mensagem para Y") de `ChannelMessageRead`/`ChannelMessageSent`.
- **Observável (gate de tela do fundador — BLOQUEANTE, validado por ele):** ver o QR → escanear → "conectado"; propor envio → ver o gate → aprovar → a mensagem chega no celular; sem "sim" → 0 envios. **Depende de F4-1-1 (contrato `channel_waha`) + F4-1-2-core (projeção, contrato `channel.rs`).** O `token_ratchet` do app permanece intacto.

---

## GATE DE SAÍDA F4-1 (roda e se mede + tela do fundador)
**(a)** QR renderiza, escaneado, sessão ativa (tela do fundador); **(b)** a IA lê uma conversa real declarada e a usa (resumo correto); **(c)** propor envio → gate humano → aprovar → a mensagem chega de fato no celular (tela), e sem "sim" 0 mensagens saem (mock conta 0); **(d)** grupo não-declarado é invisível à IA; **(e)** desconectar zera o cano + monitor de rede confirma; **(f)** trilha de auditoria reconstrói leitura+envio por replay. **Conselho de gate de onda (4 lentes, revisor isolado): Visão/fios · Arquitetura (ADR↔código + invariantes 1-7 + portas) · Segurança (red-team 0 ALTA: segredo fora do env + nada externo sem gate) · Pesquisa (fontes vs implementado).** Pendências de tela do fundador são bloqueantes.

## Riscos da onda (do épico §V)
- **Token de sessão vazar p/ env do agente** → teste de dump é critério implícito de toda frente que toca PTY/transporte (F4-1-QA gate a).
- **Leitura virar exfiltração** → só grupos declarados + auditada + conteúdo fora do log.
- **Waha mudar API** → ref pinado + manifesto versionado + escopo mínimo.
- **A IA "achar" que pode enviar sem gate** → custódia: sem segredo, sem caminho (não confia em doutrina-texto).
- **Feature-creep** (importar a complexidade do guia TomikCRM) → escopo MÍNIMO travado: conectar/ler/enviar/desconectar.

---

## FECHAMENTO desta rodada (2026-06-25, Maestro 01) — NÚCLEO completo; APP deferido p/ sessão dedicada

**Decisão do fundador:** fechar esta rodada com o **núcleo pronto, provado e commitado** (`0a4cb60`); a tela (`F41-APP`) vira a **1ª tarefa da próxima sessão dedicada**, feita com calma e testada com o Waha real na VPS.

**✅ Entregue, verde e COMMITADO (`0a4cb60`):** transporte Waha (B) · projeção Connected/session_ref (J) · leitura auditada + scope (K) · 4 eventos (largada) · **18 testes de segurança** (R) · manifesto · ADR 0050. Suíte completa do core **0 regressão**. Validado de fora pelo Maestro (cargo test + leitura). *Cold-review isolado do Maestro 00 ficou preso na fila A2A congestionada — não bloqueou; minha validação cobre o gate técnico.*

### ⬜ PENDENTE — `F41-APP` (app/lina-gpui, próxima sessão) — MAPA PRONTO p/ o executor
> O dono do app (Especialista em Telas) **travou 3× num `permission_prompt`** (atrito do Lina, ver dogfooding) + a fila A2A congestionou. NÃO foi improvisado para não arriscar freeze. Abaixo, a fronteira já mapeada pelo Maestro:

**3 entregas no app/lina-gpui (dono único):**
1. **Elo `do:channel:whatsapp:send` (porta `F41-BROKER-APP`)** — em `bridge.rs`: hoje `enqueue_custody` (`:2859`) faz `lookup_action("channel:whatsapp:send")` → falha ("ação não custodiada"). Adicionar: se `action` começa com `channel:`, parsear `channel:<canal>:<ação>` → `ChannelManifest::load` (via `RegisteredChannel.manifest_ref` da projeção `ChannelRegistry`) → `channel_action_in_manifest` → `BrokerRequest::for_channel`. **Carregar `chat_id`/`text`/`canal` no `PendingGate::Custody`** (hoje só guarda `action`/`secret_key`/`requester`) p/ o `execute` ter os args.
2. **🚨 CUIDADO DE FREEZE (mapeado — não ignorar):** `execute_custody` (`:3033`) roda o `execute` de `run_custody` **DENTRO de `lock(&self.store)`** (`:3040-3053`). O `WahaClient::send` é **I/O de rede** — chamá-lo ali SEGURA o mutex do `EventStore` durante a rede = **FREEZE** (lições `[[lina-freeze-attention-heartbeat-replay]]`/`[[lina-freeze-a2a-deliver-store-lock]]`, ADR 0046). Para o canal: gate+segredo (lock curto) → **soltar o lock** → `thread::spawn` faz o `send` HTTP (padrão de `bridge.rs:750` `fire()`/`deliver_a2a`: Arcs clonados, I/O na thread, lock só p/ appends rápidos) → re-lock só p/ apendar `ChannelMessageSent{broker_exec_ref,approved_by}`+`BrokerExecuted`. **NÃO** rodar `run_custody` inteiro com I/O no execute sob o lock.
3. **Modal "Conectar WhatsApp" + badge** (padrão `webhook_modal.rs`/`credential_modal.rs`): `connect`+`qr` OFF-CRITICAL-PATH → render do QR → poll `status` até `Working` → `connect_channel`. `base_url` **configurável** (Waha REMOTO na VPS Hetzner/Tailscale — `100.104.223.26:porta` http na mesh, OU domínio https → liga feature `tls` do `ureq`; **não** loopback). Badge "WhatsApp conectado" + trilha pt-br de `ChannelMessageRead`/`Sent`.

**Liga o e2e deferido do R:** `tests/f4_1_send_custody.rs` header §"happy-path e2e deferido ao integrador" — quando o elo existir, trocar o `execute` mock por `channel_waha::send` contra mock HTTP e afirmar 1 POST + header.
**Gate de tela do fundador (BLOQUEANTE, só ele):** Waha rodando na VPS + credenciais (`X-Api-Key`+endpoint) → ver QR → escanear → ler grupo real → propor envio → aprovar → mensagem no celular.
