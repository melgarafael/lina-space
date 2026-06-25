# ADR 0050 — Canal Waha (WhatsApp): transporte de saída e custódia de sessão

- **Status:** **Aceito** (decisão de arquitetura do Arquiteto, 2026-06-25). ADR-gate da onda F4-1: **F4-1-1 (transporte) e F4-1-4 (envio gated) não iniciam sem ele**. As demais frentes (projeção `Connected`, leitura/scope, UI shell, QA-RED) já correm com os stubs. O parágrafo de decisão vai ao `plan.md` (item `F41-ADR`) pelo Maestro; a **dependência nova** (`ureq`) é a porta que este ADR abre — o gate de produto do fundador a valida na tela ao fim da onda.
- **Escopo:** o **primeiro I/O de SAÍDA do Lina** por um canal concreto. Decide as 5 questões do ADR-gate (onda-1.md §ADR-GATE): cliente HTTP, onde mora o transporte, custódia do segredo/sessão, contrato do transporte, caminho observável de envio. **DISJUNTO** da implementação do canal (territórios de Terminal B / Especialista / Terminal K).
- **Relacionados:** ADR 0004 (custódia: gate humano + segredo no cofre — o piso que este ADR reusa, não reescreve), ADR 0046 (pump 2-fases: I/O fora do lock do `EventStore`), ADR 0036/`[[human-gesture-sentinela-origem-nao-autoridade]]` (`approved_by`/`session_ref` = proveniência, jamais autoridade), invariantes 1 (sem runtime próprio supérfluo) · 2 (local-first) · 3 (neutralidade) · 4 (event log fonte da verdade).

## Contexto

A onda F4-1 conecta uma conta de WhatsApp via **Waha** (engine REST de terceiro) por QR Code: a IA **lê** conversas (contexto de entrada, sem gate) e **envia** mensagens (efeito externo irreversível, com gate humano). A largada deixou um substrato sólido — **o gate de envio já existe** (`broker.rs`: `BrokerRequest::for_channel`, `run_custody`, `channel_action_in_manifest`; default-deny contra o manifesto; `ActionGated{gated-hard-external, ask}` SEMPRE; segredo lido do cofre só no `execute(&secret)`). O verbo `lina do <canal> <ação>` já produz o `ref do:channel:<canal>:<ação>` e enfileira `broker.do` (F4-0-3).

O **delta** que falta — e que abre porta nova — é o **transporte HTTP de saída**: o projeto **não tem cliente HTTP** (só `hyper`/`axum` server-side, loopback). Falar com o Waha exige uma **dependência nova** e o primeiro byte que sai do Lina para um serviço externo. É uma decisão de continuidade: como o I/O de saída entra sem fechar portas (runtime, local-first, core↔shell).

## Decisão

### (1) Cliente HTTP de saída — `ureq` síncrono, fora do hot-path, com timeout

Adota-se **`ureq` (2.x)** — cliente HTTP **bloqueante, sem runtime async**. Rejeita-se `reqwest`/`hyper`-client (async).

- **Por quê não async:** o core (`lina-core`) só depende de `tokio` com feature **`sync`** (o bus broadcast). `reqwest`/`hyper`-client exigem um runtime async; arrastá-lo para o core **fecharia a porta** "core blocking-puro, shell substituível" (âncora `UiHost`) e contraria o invariante 1 (não multiplicar reatores). `ureq` é independente de `tokio`/`hyper` — o core continua agnóstico de runtime.
- **A chamada de rede roda FORA da thread de render e com timeout curto** (connect+read ~10 s). Reusa o padrão do **ADR 0046** (entrega A2A fora do lock do `EventStore`) e as lições `[[lina-freeze-attention-heartbeat-replay]]`/`[[lina-freeze-a2a-deliver-store-lock]]`: **nenhuma chamada Waha pode segurar o lock do store nem bloquear o pump**. Como `run_custody` chama `execute(&secret)` inline, **a story do app (F4-1-4) invoca a custódia de canal na thread off-critical-path que o app já mantém** (o tokio dedicado + fila in-process que o pump drena) — a render thread nunca espera a rede. Waha pendurado → timeout → erro narrado, **canvas vivo** (critério observável).

### (2) Onde mora o transporte — módulo `channel_waha` no core, atrás de borda de I/O injetável (não crate)

O cliente vive em **`crates/lina-core/src/channel_waha.rs`** (stub já criado na largada), **não** num crate novo.

- **Por quê não crate (ARQ-1/ARQ-3):** um crate é empacotamento sem 2º consumidor — o transporte tem um consumidor (o core: `execute` do broker + `channel_read`). Crate só se justificaria por reuso externo ou fronteira de compilação; não há. Módulo no core é a menor estrutura que resolve.
- **A testabilidade headless vem da borda de I/O injetável** (não do empacotamento): um trait fino `WahaHttp` isola a única superfície que toca a rede. Duas implementações **reais hoje** (ARQ-1 satisfeito): `ureq` (produção) e um mock de teste que conta requests.

### (3) Custódia — `X-Api-Key` no cofre (escopo `channel:whatsapp`), `session_ref` é referência, não autoridade

- **O `X-Api-Key`** (valor de auth REST do Waha — o segredo crítico) vive **só** no `lina-secrets`, escopo **`channel:whatsapp`** (`CHANNEL_SECRET_SCOPE_PREFIX` que `for_channel` deriva server-side). A **chave** dentro do escopo é a `secret_key` que o gesto de credencial passa (`store_channel_credential(channel, key, val)` — F4-0-2); **não é dogma** deste ADR, mas B e o Especialista **alinham um literal único** entre quem grava (modal) e quem lê (`for_channel`/`WahaClient`) — o gesto vivo no app usa `sessao` (`bridge.rs` test). **Nunca** no env do PTY do agente, **nunca** no log/evento. Lido **server-side** nos três pontos de I/O — `connect`, `read`, `send` — e passado só como parâmetro à chamada. O agente nunca o vê: sem segredo, sem caminho (custódia ADR 0004).
- **Assimetria de gate, custódia idêntica:** **`send` passa pelo gate humano** (`run_custody`, irreversível). **`connect`/`qr`/`read` não** — conectar é o gesto humano do QR; ler é fluxo auditado. Mas os três leem o `X-Api-Key` server-side do MESMO escopo; o que difere é o gate, não a exposição do segredo.
- **`session_ref`** = **referência de cofre**, não o token nem o nome cru da sessão. O `connect_channel` vivo (`channel.rs`, F4-1-2-core) já grava a forma **`keyring:channel:whatsapp`** (uma `CredentialRef`) em `RegisteredChannel.session_ref` via `ChannelConnected{channel, session_ref, scope}`. Sozinho não envia nada — a autoridade é o segredo no cofre. `session_ref`/`approved_by` **nunca** decidem autorização (lição `[[human-gesture-sentinela-origem-nao-autoridade]]`).
- **`base_url`/endpoint** = config, não segredo. O `connect_channel` vivo carrega o endpoint efetivo no campo **`scope`** do `ChannelConnected` (`waha@127.0.0.1:3000` no teste vivo) — o transporte o projeta daí, sem inventar fonte nova. Default **`http://127.0.0.1:3000`** (loopback, local-first). Endpoint remoto (Waha em VPS, `https://…`) é **exposição de rede opt-in sinalizada** (invariante 2): liga a feature aditiva `tls` (rustls) do `ureq` — fora do escopo MVP, porta aberta.

### (4) Contrato do transporte — `WahaHttp` (borda) + `WahaClient` (domínio)

A borda injetável é a de **I/O cru** (não a de domínio): mockar no nível HTTP prova que `send` faz *exatamente* 1 `POST /api/sendText` com `X-Api-Key` no header (gate b). Mockar no nível de domínio só provaria que `send()` foi chamado.

```rust
// crates/lina-core/src/channel_waha.rs — escopo MÍNIMO (sem LID/dedup/outbox/retry)

/// A ÚNICA superfície que toca a rede. Impls reais: `UreqHttp` (prod) + mock de teste (conta requests).
pub trait WahaHttp {
    fn execute(&self, req: WahaHttpRequest) -> Result<WahaHttpResponse, WahaError>;
}
pub struct WahaHttpRequest {
    pub method: HttpMethod,           // Get | Post
    pub path: String,                 // "/api/sendText", "/api/sessions", …
    pub api_key: String,              // header X-Api-Key — NUNCA logado
    pub body: Option<serde_json::Value>,
}
pub struct WahaHttpResponse { pub status: u16, pub body: Vec<u8> }

/// Domínio: traduz os verbos do canal → requests HTTP. Genérico sobre a borda → testável sem rede.
pub struct WahaClient<H: WahaHttp> { /* http: H, base_url, session */ }

impl<H: WahaHttp> WahaClient<H> {
    pub fn connect(&self, api_key: &str) -> Result<(), WahaError>;                       // POST /api/sessions + start
    pub fn qr(&self, api_key: &str) -> Result<QrImage, WahaError>;                       // GET /api/{session}/auth/qr (PNG/data-uri)
    pub fn status(&self, api_key: &str) -> Result<SessionStatus, WahaError>;             // GET /api/sessions/{name}
    pub fn read(&self, api_key: &str, chat_id: &str, limit: u32) -> Result<Vec<WahaMessage>, WahaError>;
    pub fn send(&self, api_key: &str, chat_id: &str, text: &str) -> Result<SentRef, WahaError>; // POST /api/sendText
    pub fn disconnect(&self, api_key: &str) -> Result<(), WahaError>;                    // logout/stop
}

pub enum SessionStatus { Starting, ScanQrCode, Working, Failed, Stopped } // espelha o guia Waha
```

- **`api_key` é SEMPRE parâmetro** (vem do `execute(&secret)` do broker, ou da leitura server-side do cofre para `connect`/`read`). `WahaClient` não lê env nem loga o `api_key`.
- **`read` devolve o conteúdo ao chamador** (vira contexto do agente — é o objetivo da leitura); `channel_read` (Terminal K) apenda só `ChannelMessageRead{count, refs}` — **conteúdo nunca no log**.
- **`chat_id`/`text` são DADO** fornecido pelo gesto (destinatário + texto que o humano aprova vendo); a autoridade é o `api_key`. Escopo MÍNIMO: `chat_id` passa direto (`<digits>@c.us` ou `@lid`), **sem** as bridges LID/normalização/dedup do guia de produção (épico §V.4, anti-creep).
- **Assinatura combinada Terminal B ↔ Terminal K** logo no começo (a onda já pede): `read` é o ponto de contato.

### (5) Caminho observável de envio — `ref do:channel:whatsapp:send` (já existe; F4-1-4 só liga o `execute`)

O caminho **já existe ponta-a-ponta no registro do gate**, por duas bocas que convergem no mesmo `ref`:

1. **CLI** `lina do whatsapp send <chatId> <texto>` → `run_do_channel` (valida canal declarado no `ChannelRegistry`) → `ActionGated{ask}` + `BrokerDenied{unconfirmed}` + enfileira `broker.do` com `ref:"do:channel:whatsapp:send"`. O agente **registra, não executa**.
2. **Gesto A2A** `MailMessage{to:"broker", intent:"broker.do", ref:"do:channel:whatsapp:send"}` → `bridge.rs` intercepta → `enqueue_custody` → item na fila de atenção (gate humano).

Após o "sim", **`execute_custody` é o único delta de F4-1-4** (porta `F41-BROKER-APP`): hoje usa `BrokerRequest::new` (builtin); passa a reconhecer a forma de canal `do:channel:<canal>:<ação>` (split por `:`), resolver o manifesto via `RegisteredChannel.manifest_ref` → `channel_action_in_manifest` → `BrokerRequest::for_channel("whatsapp","send",…)` → no `execute(&secret)` chamar `WahaClient::send(secret, chat_id, text)` + apendar `ChannelMessageSent{broker_exec_ref, approved_by}`. **O gate (`run_custody`) já existe — F4-1-4 não o reescreve, só liga o `execute`.** O verbo CLI é disparador alternativo do MESMO `ref`; o gesto A2A basta e é observável sozinho.

**Consistência ação↔manifesto (cravar, senão o envio é negado antes da custódia):** a ação no `ref` é um **slug sem `:`** (`run_do_channel` rejeita `:` no canal e na ação) — `send`, `read`. E `channel_action_in_manifest` faz **string-match exato** contra `scopes`/`tools.default_enabled`. Logo a ação brokerada **tem que estar literal no manifesto**: o `manifest.toml` do whatsapp (F4-1-1) declara `tools.default_enabled = ["send", "read"]` (slugs que casam o `ref`). Os `scopes = ["messages:send","messages:read"]` da onda são **vocabulário descritivo** — não casam `send` sozinhos (têm `:`). Sem esse alinhamento, `channel_action_in_manifest(manifest,"send")` → `false` → default-deny recusa antes do gate. (B e o Especialista usam a MESMA grafia de ação no manifesto, no `for_channel` e no `ref`.)

## Segurança (portas que NÃO fecham)

- **Custódia é o piso (ADR 0004):** envio sem "sim" = `run_custody(confirmed=false)` → `BrokerDenied{unconfirmed}`, cofre nem tocado, **0 POSTs** (mock conta 0 — gate b). Sem `api_key` no escopo → `DeniedNoSecret`, mesmo confirmado.
- **Segredo fora do agente:** `api_key` lido server-side, nunca no env do PTY — provado pelo teste de dump (gate a; mutação: injeta o segredo → a sonda morde).
- **`session_ref`/`approved_by` = DADO, jamais autoridade** (mutação: forjá-los não muda o escopo do segredo nem libera o efeito — herda a doutrina `[[human-gesture-sentinela-origem-nao-autoridade]]`).
- **Manifesto é DADO** (lista o que o canal expõe); quem libera o efeito é a custódia, não o `channel_action_in_manifest` (default-deny).
- **Conteúdo nunca no log** (só `count`/refs); leitura só de grupos `ToolScopeDeclared` (default-deny — F4-1-5).
- **Desconectar zera o cano:** `ChannelDisconnected` → `Declared`, `session_ref` limpo → próximo envio falha "não conectado".

## Por quê assim (alternativas descartadas)

- **`reqwest`/`hyper`-client (async):** exige runtime async no core → fecha a porta core blocking-puro + multiplica reator (invariante 1). Rejeitado.
- **Crate `lina-channel-waha`:** empacotamento sem 2º consumidor (ARQ-1). Rejeitado; módulo no core.
- **Mock no nível de domínio:** não prova a request HTTP real (método/path/header). Rejeitado; borda no nível de I/O.
- **Leitura por webhook/push:** exige expor um túnel/endpoint (contraria local-first); pull on-demand é mais simples para "a IA lê quando perguntada" (D4 da largada). Porta de push (`MsgOrigin::System`, F4-WA) fica aberta, fora desta onda.
- **TLS no MVP:** o default local-first é Waha loopback (http); TLS só serve ao endpoint remoto, que é exposição opt-in (invariante 2). Feature aditiva, porta aberta.

## Consequências

- **Habilita** F4-1-1 (transporte), F4-1-3 (read), F4-1-4 (send gated), F4-1-QA (mock) — todos contra o contrato `WahaHttp`/`WahaClient`.
- **Dependência nova:** `ureq` (a porta que este ADR abre); subárvore mínima, sem `tokio`/`hyper`. Pinada no `Cargo.toml` do core por F4-1-1.
- **Custo:** uma borda de I/O + um cliente fino; a chamada de rede precisa rodar off-critical-path (a story do app reusa o padrão ADR 0046).
- **Porta que fecha se ignorado:** I/O de rede no hot-path/sob o lock do store → freeze (lições #1/#2). O ADR torna o invariante "Waha pendurado não congela o canvas" critério observável da onda.

## Verificação (observável)

- **0 sem "sim":** `run_custody(false)` → mock conta **0 POSTs**; com "sim" + `api_key` no cofre → **exatamente 1** `POST /api/sendText` com `X-Api-Key` no header (mock).
- **Segredo custodiado:** dump de env do PTY do agente → **0 hits** do `api_key` (mutação morde).
- **`session_ref` não-autoridade:** forjar `session_ref`/`approved_by` no payload não muda o escopo do segredo (mutação).
- **Sem freeze:** transporte com timeout, chamado off-critical-path → Waha indisponível não bloqueia o pump (herda o teste de "off the critical path" do ADR 0046).
- **Desconectar zera:** `ChannelDisconnected` → próximo `send` falha "não conectado" (gate d do QA).
- **Local-first:** monitor de rede (F4-0-5) → 0 conexão externa sem o canal conectado; default loopback.
