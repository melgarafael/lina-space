# Onda F5-WH — Webhook Hospedado (endereço público de ingestão) — peça executável

> **Fonte da verdade story-a-story** desta onda. Deriva do **ADR 0051** (ingestão hospedada — Aceito como direção; vira Aceito-pleno quando a 1ª story de transporte entregar com a suíte de segurança do Router verde) + **ADR 0034** (`BusTransport` — a porta cross-machine a reusar). **Maestro da rodada:** Maestro 00.
> **Posição no roadmap:** **ponta de F5** (épico 42 — cross-machine / Lina no VPS) puxada por dor concreta de F4-WA. Roda **coordenada com a F4-1 ativa** (Maestro 01): costuras compartilhadas (`events.rs`, `lib.rs`, `a2a.rs`) exigem dono único por rodada — alinhar com o Maestro 01 ANTES de tocar.
> **Decisão do fundador (2026-06-25):** hospedagem = **Cloudflare Tunnel** (`cloudflared` embutido no app); VPS próprio com FRP como fallback documentado.
> **Meta:** o webhook do Lina deixa de viver só em `127.0.0.1` — passa a ter um **endereço público `.com` estável** que serviços externos (Featurebase, pagamentos, formulários) alcançam, SEM o app local abrir porta de escuta (conexão reversa de saída) e SEM trair o local-first (opt-in sinalizado + gate humano + relay cego). A IA viva continua interpretando o payload cru (ADR 0035 intacto).

## Estado de partida (reconhecido no código, 2026-06-25)
### Já existe (REUSAR)
- **`lina-webhooks`** (`crates/lina-webhooks/src/lib.rs`) — engine axum + HMAC + `handle_hook` (rate-limit 3 camadas, 404 opaco, append-aguardado de `WebhookReceived`, carimbo `MsgOrigin::System{webhook_id}` server-side). **Todo o handler blindado roda no app local, inalterado** — só passa a receber os bytes do gateway em vez do `TcpListener` loopback.
- **`lina-secrets`** — cofre custodiado (ADR 0004). O `device_token` do gateway vive aqui (escopo novo), nunca no env/log — mesma disciplina do secret HMAC.
- **Router / `deliver_a2a`** — a cadeia de entrega (dedupe→anti-loop→remetente→alvo→autonomia→orçamento) roda IDÊNTICA; o ingress só muda a FONTE dos bytes.

### NÃO existe (o delta desta onda)
1. **A porta `BusTransport`** — ADR 0034 está **Proposto**; a trait + `InProcessTransport` NÃO existem em código. É o pré-requisito de tudo (W-0).
2. **`RemoteIngressTransport`** — a impl que disca de saída pro gateway e injeta o POST recebido no Router.
3. **`cloudflared` embutido** — discar a conexão reversa real de saída.
4. **O gateway na nuvem** (Cloudflare) — endpoint público `.com` por capability, relay cego, multi-tenant.
5. **Auth app↔gateway** — par `capability` pública ↔ `device_token` custodiado.
6. **Eventos** `HookExposed`/`HookUnexposed` (abrir/fechar exposição = fato do log, aditivos).
7. **UI** — botão "expor na internet" + gate humano + sinalização "exposto".

## ADR-gates
- **ADR 0034 (`BusTransport`)** — hoje **Proposto**. **W-0 não fecha sem ele aceito.** A aceitação ⇔ a trait + `InProcessTransport` entregarem com ZERO regressão (toda a suíte do core verde, comportamento idêntico).
- **ADR 0051 (ingestão hospedada)** — direção Aceita; vira Aceito-pleno quando **W-1** entregar o transporte reverso + a suíte de segurança verde.

## As stories (ordem por dependência; território disjunto)

### W-0 — Fundação: porta `BusTransport` + `InProcessTransport` · dono **Backend + Arquiteto** · L · gate ADR 0034
- **Território:** `crates/lina-core/src/` — novo `bus_transport.rs` + costura em `lib.rs` (Supervisor) e `a2a.rs` (`deliver_a2a`). **COSTURA PESADA compartilhada com F4-1 — dono único; alinhar com Maestro 01.**
- **Entrega:** trait `BusTransport` (abstrai POR ONDE o envelope chega ao nó) + `InProcessTransport` (o comportamento atual, refactor SEM mudança). Aceitar o ADR 0034.
- **Gate (observável):** toda a suíte do core VERDE; comportamento de entrega A2A idêntico ao de hoje (zero regressão, provado). É o pré-requisito de W-1..W-6.
- **Status (2026-06-25 · Arquiteto Webhook):** **DESENHO PRONTO** — ADR 0034 promovido a **Aceito** com o contrato exato da trait (`BusTransport::deliver`, espelho de `deliver_a2a` menos o `&Supervisor`), `InProcessTransport` como passthrough zero-regressão, e o **mapa de costuras** (ver ADR 0034 §"Contrato (W-0)" C1–C4). **Refinamento sinalizado:** o `RemoteIngressTransport` (W-1) **consome** a porta (entrega local via `InProcessTransport`), não a reimplementa — a 2ª impl de `deliver` é o `RemoteOverSSH` (alvo remoto). Costura mínima: 1 arquivo novo (`bus_transport.rs`) + 1–2 linhas em `lib.rs:54`; `a2a.rs`/`events.rs`/call-sites **intactos** → colisão mínima com F4-1. **Falta:** janela de implementação coordenada (Maestro 00 × Maestro 01) — o Backend implementa seguindo o contrato aceito.

### W-1 — `RemoteIngressTransport` (impl da porta) · dono **Backend** · M/L · gate W-0
- **Território:** novo módulo `crates/lina-core/src/ingress_transport.rs` (não toca a costura de W-0 além do `impl`).
- **Entrega:** a impl que recebe um POST do gateway (por conexão reversa) e o entrega ao nó local pelo MESMO `deliver_a2a`, origem `System{webhook_id}` carimbada **no app** (relay cego não carimba).
- **Gate:** mock do gateway empurra um POST → o webhook cai no terminal vivo via a cadeia normal; origem System inforjável; suíte de segurança do Router verde. **(Aceita o ADR 0051 pleno.)**

### W-2 — `cloudflared` embutido (conexão reversa de saída) · dono **Backend** · M · gate W-1
- **Território:** core/app — supervisão do processo `cloudflared` (discar de saída, timeout, off-critical-path como ADR 0046).
- **Gate:** o app sobe o túnel de saída; **monitor de rede confirma ZERO porta de escuta no app** (local-first por construção); Cloudflare pendurado → erro narrado, canvas vivo.

### W-3 — Gateway na nuvem (Cloudflare) · dono **Backend/DevOps** · M · paralelo a W-1/W-2
- **Território:** infra separada (`infra/gateway/` — Cloudflare Worker + Durable Objects, NÃO o binário do app).
- **Entrega:** endpoint público `https://<host>/h/<capability>` que roteia o POST pro device autenticado correto; **relay cego** (não lê/assina/carimba); absorve DoS (3 camadas + teto de corpo + 404 opaco).
- **Gate:** POST em `/h/<capability>` chega ao device certo; capability inválida → 404; o corpo nunca é lido/persistido pelo gateway.

### W-4 — Auth app↔gateway + eventos de exposição · dono **Backend** · M · gate W-1
- **Território:** `events.rs` (costura — alinhar com Maestro 01) + `lina-secrets` + core.
- **Entrega:** par `capability` pública ↔ `device_token` custodiado (keyring, escopo novo); eventos aditivos `HookExposed{hook_id, capability}` / `HookUnexposed{hook_id}` (`#[serde(default)]`).
- **Gate:** `device_token` nunca no env/log (dump = 0 hits); forjar capability NÃO dá acesso a outro device (multi-tenant isolado); abrir/fechar exposição = evento re-derivável por replay.

### W-5 — UI: expor com gate humano + sinalização · dono **Frontend (Especialista em Telas)** · L · gate W-1+W-4
- **Território:** `app/lina-gpui/` (`webhook_modal.rs`, `bridge.rs`, badge). Dono único do app.
- **Entrega:** botão "expor este webhook na internet" no modal → **gate humano** (ação irreversível de impacto externo, ADR 0004/§6) → mostra o endereço `.com` → sinaliza "este hook está exposto na internet". Desativar zera. Estética pela `lina-design-doctrine`; zero jargão.
- **Gate de tela do fundador (BLOQUEANTE):** ver o endereço público, ativar com confirmação, ver o sinal "exposto", um POST externo real chega no terminal; desativar zera o cano.

### W-6 — QA segurança (transversal) · dono **QA** · M · liga conforme as frentes entregam
- **Território:** `crates/lina-core/tests/f5_wh_*.rs` (novo, dono único).
- **Gate (red-team 0 ALTA):** (a) relay cego NÃO forja origem System (mutação); (b) `device_token` fora do env (dump); (c) 0 exposição sem o "sim" humano; (d) multi-tenant isolado (capability de A não alcança device de B); (e) app não abre porta de escuta; (f) suíte de segurança do Router verde por mutação.

## GATE DE SAÍDA F5-WH (roda e se mede + tela do fundador)
Um serviço externo real (ex.: Featurebase) POSTa no endereço `.com` → o aviso chega no terminal vivo e a IA age conforme a instrução (tela); o app NUNCA abriu porta (monitor); expor exige o "sim" humano (0 sem confirmação); desconectar zera; `device_token`/conteúdo fora do log; multi-tenant isolado. **Conselho de gate de onda (4 lentes, revisor isolado): Visão/fios · Arquitetura (ADR 0034/0051↔código + invariantes) · Segurança (red-team 0 ALTA) · Pesquisa.**

## Riscos
- **Costura compartilhada com F4-1** (`events.rs`/`lib.rs`/`a2a.rs`) → dono único por rodada; coordenar com Maestro 01 ANTES de tocar.
- **App abrir porta sem querer** → local-first é por AUSÊNCIA de bind (conexão só de saída); W-2/W-6 provam.
- **Gateway virar vetor de injeção** → relay cego; origem System carimbada no app, nunca na nuvem (ADR 0051 §3); W-6 mutação.
- **Custo de infra na nuvem** → começa irrisório (<US$0,10/usuário/mês); o eixo é ops, não preço.
- **Feature-creep** → escopo MÍNIMO: um endereço público que entrega o POST ao app. Sem buffer persistente, sem multi-região no MVP (portas abertas, fora do escopo).
