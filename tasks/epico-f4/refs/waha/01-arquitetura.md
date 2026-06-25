# 01 — Arquitetura

> Visão end-to-end: componentes, responsabilidades, interações. Se você só lê um doc, seja esse.

---

## Diagrama de alto nível

```mermaid
flowchart TB
  subgraph client["Cliente (WhatsApp)"]
    WA["WhatsApp App<br/>(Apolo, Preta, etc.)"]
  end

  subgraph hetzner["Hetzner shard-1 — wa-shard-1.automatiklabs.com.br"]
    COOLIFY["Coolify (orquestrador Docker)"]
    WAHA["WAHA Plus container<br/>(devlikeapro/waha-plus)<br/>Engine: WEBJS"]
    PG["Postgres WAHA<br/>(sessions state)"]
    REDIS["Redis cache"]
    COOLIFY --- WAHA & PG & REDIS
  end

  subgraph supabase["Supabase qck (Master)"]
    subgraph edges["Edge Functions"]
      IN["whatsapp-unofficial-inbound"]
      START["wa-start-session"]
      QR["wa-get-qr"]
      TEXT["wa-send-text"]
      MEDIA["wa-send-media"]
      RR["wa-send-reaction-reply"]
      DISC["wa-disconnect"]
      RETRY["agent-dispatch-retry"]
    end
    subgraph masterdb["Master DB"]
      CH["saas_messaging_channels"]
      SEC["saas_messaging_channel_secrets"]
      QUEUE["saas_agent_dispatch_retry_queue"]
    end
  end

  subgraph clientdb["Client DB (BYO ou tomik_hosted)"]
    CONV["messaging_conversations"]
    MSGS["messaging_messages"]
    CONTACTS["messaging_contacts"]
    OUTBOX["agent_events_outbox"]
  end

  subgraph runtime["Agent Runtime (Render)"]
    V1["agent-runtime V1"]
    V3["agent-runtime-v3"]
    DRAINER["outbox-drainer<br/>(V1 poll 15s, V3 poll 5s)"]
    V1 & V3 --- DRAINER
  end

  subgraph frontend["Frontend (Vercel)"]
    CHATLIVE["ChatLive"]
    CONNECTION["WhatsAppConnection"]
    CARD["WhatsAppUnofficialCard"]
  end

  WA <-->|HTTPS + HMAC| WAHA
  WAHA -->|POST webhook| IN
  IN --> CH & CONV & MSGS & CONTACTS
  IN -.->|triggerRuntimeDispatch| V1 & V3
  V1 & V3 --> OUTBOX
  DRAINER -->|resolve variant| CH
  DRAINER -->|route unofficial_qr| TEXT & MEDIA & RR
  CHATLIVE -->|invoke| TEXT & MEDIA & RR
  CONNECTION -->|invoke| START & QR & DISC
  CARD --> START & QR
  TEXT & MEDIA & RR & START & QR & DISC -->|factory + secrets| CH & SEC
  TEXT & MEDIA & RR & START & QR & DISC -->|HTTP + X-Api-Key| WAHA
  RETRY -->|cron 1-2min| QUEUE
```

## Componentes — responsabilidades

### Frontend (React + Vercel)

| Componente | Responsabilidade |
|------------|------------------|
| `WhatsAppConnection.tsx` | Lista canais de uma org. Botão "+ Adicionar instância". Realtime subscription em `saas_messaging_channels`. |
| `WhatsAppUnofficialCard.tsx` | State machine do QR flow: `idle → loading → qr → paired → error/banned/reconnecting`. Chama `wa-start-session` (uma vez) + `wa-get-qr` (polling 20s). |
| `ChatLive.tsx` | Interface unificada de mensagens. Roteia outbound para `invokeUnofficialSendText/Media/ReactionReply` quando `conversation.provider_variant === 'unofficial_qr'`. |
| `NewConversationModal.tsx` | Modal "+ Nova conversa". Para `unofficial_qr` pula a exigência de template Meta e cria conversa diretamente. |

### Edge Functions (Supabase Deno)

Todas vivem em `supabase/functions/`. Compartilham helpers em `supabase/functions/_shared/`.

#### Lifecycle de session

| Edge | Entrada | Responsabilidade |
|------|---------|------------------|
| `wa-start-session` | `{channel_id}` | Provisiona (se faltar): `unofficial_instance_name`, `unofficial_provider_host_url`, secrets encriptados. Chama `WAHAProvider.startSession`. Flip `channel.status = 'connecting'`. Idempotente. |
| `wa-get-qr` | `{channel_id}` | `WAHAProvider.getQR` → PNG base64. Se 422 com `status: 'WORKING'` → flip channel pra `connected`, retorna `paired: true` + `display_phone_number` (derivado de `me.id`). |
| `wa-disconnect` | `{channel_id}` | `WAHAProvider.disconnect` (logout). Flip `status = 'disconnected'`, `is_active = false`. Preserva secrets para reconexão. |

#### Inbound

| Edge | Responsabilidade |
|------|------------------|
| `whatsapp-unofficial-inbound` | Valida HMAC SHA512 do webhook. Handler triplo: `session.status/state.change` (só atualiza channel), `message/message.any` (contato + conversa + persist + dispatch runtime), `message.reaction` (upsert como `type='reaction'`). Aplica as 3 bridges LID↔phone. Dedupe por `provider_message_id` normalizado. |

#### Outbound

| Edge | Responsabilidade |
|------|------------------|
| `wa-send-text` | Envia texto. Pre-resolve `messaging_contacts.wa_id` para override do chatId (routing @lid). Persiste row outbound em `messaging_messages`. |
| `wa-send-media` | Envia image/video/audio/document/sticker via URL ou base64. Storage path passthrough para re-signing após 1h TTL. |
| `wa-send-reaction-reply` | Duas ações: `reaction` (emoji) ou `reply` (text threaded). Target identificado por `provider_message_id`. |

#### Retry

| Edge | Responsabilidade |
|------|------------------|
| `agent-dispatch-retry` | Cron worker (external). Drena `saas_agent_dispatch_retry_queue` no Master. Backoff 30s → 2m → 10m. Dead-letter após 3 tentativas. |

### Shared helpers (Deno edge libs)

Localizados em `supabase/functions/_shared/`.

| Arquivo | Função |
|---------|--------|
| `wa-provider.ts` | Interface `WAProvider` (startSession, getQR, getStatus, sendText, sendMedia, sendReaction, disconnect). Contrato canônico. |
| `wa-provider-waha.ts` | Implementação `WAHAProvider`. Traduz chamadas genéricas pro REST da WAHA. Timeouts (30s), retry em GET (5xx), HMAC config. |
| `wa-provider-factory.ts` | `getWAProviderForChannel(client, channelId)`. Lê channel + secrets + decripta + instancia `WAHAProvider`. Retorna `{ok, context}` ou `{ok:false, error, status}`. |
| `wa-channel-secrets.ts` | `resolveUnofficialChannelSecrets(client, channelId)`. Single round-trip SELECT + parallel decrypt. Cache por-request. |
| `waha-normalizer.ts` | Helpers sobre payloads WAHA: `phoneFromWahaJid`, `isGroupJid`, `isLidJid`, `canonicalMessageType`, `extractMessageText`, `mapWahaStateToChannelStatus`, `normalizeWahaProviderMessageId`, `chatIdFromPhone`, `verifyHmacSha512`. |
| `runtime-bridge.ts` | `triggerRuntimeDispatch(msg, opts)`. Compartilhado com Meta handler (PR #19 unificou). Daily limit + pipeline resolution + V1/V3 ingest routing. |
| `dispatch-gate.ts` | `shouldDispatchMessageEvents(client, org, convoId, event, opts)`. Avalia `handoff_mode='human'`. Fail-open em DB transient. |
| `dispatch-retry-queue.ts` | `enqueueDispatchRetry(master, input)`. Persiste payload numa nova row da queue no Master DB. |
| `wa-media.ts` | `downloadAndUploadMedia(...)` — baixa mídia WAHA, sobe pro Storage bucket `messaging-media`. Retorna signed URL + storage_path pra re-signing. |

### Master DB (Supabase qck)

Ver [03-schema-dados.md](./03-schema-dados.md) para schema completo.

| Tabela | Rows relevantes | Dono |
|--------|-----------------|------|
| `saas_messaging_channels` | Um canal por instância QR. Colunas `unofficial_*`. | `whatsapp-unofficial-inbound`, `wa-start-session` |
| `saas_messaging_channel_secrets` | Credenciais encriptadas. | `wa-start-session`, `wa-channel-secrets.ts` |
| `saas_agent_dispatch_retry_queue` | Retry pra runtime dispatch que falhou. | `dispatch-retry-queue.ts`, `agent-dispatch-retry` edge |

### Client DB (Tenant-específica)

| Tabela | Relevância |
|--------|------------|
| `messaging_contacts` | Contatos. Coluna `wa_id` crítica pra LID routing. Coluna `phone_e164` recebe o telefone real (via bridges). |
| `messaging_conversations` | Uma conversa por `(org, master_channel_id, contact_id)`. |
| `messaging_messages` | Histórico. `provider_message_id` + `direction` + `type` + `provider_variant`. Unique `(org, channel, provider_message_id)` para dedup. |
| `agent_events_outbox` | Fila que o drainer lê pra efetivar o envio. |

### Agent Runtime (Render — Node.js)

Duas versões rodando em paralelo enquanto migração V1→V3 é concluída.

| App | Path | Observação |
|-----|------|------------|
| agent-runtime (V1) | `apps/agent-runtime/` | Legacy. Drena V1 outbox + V3 outbox (hybrid). |
| agent-runtime-v3 | `apps/agent-runtime-v3/` | Novo. Tem seu próprio drainer paralelo (mas acessa a mesma tabela V1 `agent_events_outbox`). |

**Outbox Drainer** (em ambos) — poll loop que:
1. Claim rows pending (update status='processing')
2. Resolve `saas_messaging_channels.provider_variant` via Master (in-process cache)
3. Roteia:
   - `meta_cloud` → `WHATSAPP_SEND_ENDPOINT` (Meta edge)
   - `unofficial_qr` → `wa-send-text` / `wa-send-media` / `wa-send-reaction-reply` com `SUPABASE_SERVICE_ROLE_KEY`
   - `instagram` → `INSTAGRAM_SEND_ENDPOINT`
4. Marca `delivered` em 2xx. Retry/backoff + dead-letter em falha.
5. Stale guard: se chegou msg inbound mais nova que a execução, suprime o reply (evita responder fora de contexto).

### Infra — Hetzner shard-1

Ver [04-infra-hetzner-coolify.md](./04-infra-hetzner-coolify.md).

- VPS Hetzner CX22 (2 vCPU, 4GB RAM), SA region, Ubuntu 24.04
- Volume Hetzner 20GB montado em `/mnt/wa-data` (postgres, redis, waha-sessions, caddy, backups)
- Coolify v4 self-hosted (dashboard em `:8000`)
- WAHA Plus container + Postgres + Redis (docker-compose)
- Cloudflare Tunnel: `wa-shard-1.automatiklabs.com.br` → VPS `:443`
- Tailscale admin mesh: `ssh wa-shard-1` (IP interno `100.104.223.26`)
- Backup B2: snapshots diários `/mnt/wa-data/backups/`
- Terraform: `infra/terraform/wa-shard.tf` + `cloud-init.yaml` + `variables.tf`

---

## Fluxos

Resumo aqui; deep-dive em [02-fluxo-mensagens.md](./02-fluxo-mensagens.md).

### Fluxo A — Conectar novo número (QR)

```
User clica "+ Adicionar instância"
  → INSERT saas_messaging_channels (status=disconnected, unofficial_instance_name=null)
  → Realtime push → ChatLive refetch → card aparece com status=idle

User clica "Conectar via QR Code"
  → Card.startSession → wa-start-session
    → provisiona instance_name "tomik-<channel-id-12>"
    → provisiona secrets (api_key SHA512 + webhook_token random) + encrypt + INSERT
    → WAHAProvider.startSession (POST /api/sessions/ create → POST /start)
    → flip status=connecting
  → Card.fetchQrOnly → wa-get-qr
    → factory resolve secrets + config
    → WAHAProvider.getQR → PNG data-URI
    → Card renderiza QR com countdown 20s

User escaneia com celular
  → WAHA transita SCAN_QR_CODE → WORKING
  → Webhook session.status → whatsapp-unofficial-inbound
    → UPDATE channel status=connected, display_phone_number=<me.id digits>
  → Fallback: próximo poll de wa-get-qr retorna 422 → detecta WORKING → flip pra paired
  → Card flip state pra paired (via realtime push ou fallback 422)
```

### Fluxo B — Receber mensagem

```
Contato envia msg pra number do user
  → WAHA detecta + envia POST + HMAC
  → whatsapp-unofficial-inbound
    1. Match HMAC header x-webhook-hmac contra SHA512(body, secret)
    2. Resolve org do channel_id (query string)
    3. Event type switch:
       - state.change/session.status → UPDATE channel (status + display_phone)
       - message.* → fluxo completo:
         a. Extrai phone_e164 (3 bridges LID) + rawJid (@lid se presente)
         b. Normaliza providerMessageId (strip true_/false_)
         c. resolveMessagingContactByPhone → upsert contact com wa_id
         d. Upsert conversation (unique org+channel+contact)
         e. Insert messaging_messages (dedup via 23505)
         f. Se !fromMe → increment unread + dispatch runtime
       - message.reaction → persist como type=reaction
  → runtime-bridge.triggerRuntimeDispatch → V1/V3 Agent Runtime
    → Se HTTP fail → enqueueDispatchRetry no Master queue
```

### Fluxo C — Enviar mensagem (ChatLive humano)

```
Operator digita msg em ChatLive
  → invokeUnofficialSendText({channel_id, to, text, conversation_id})
    → wa-send-text edge
      1. Valida JWT (ownership check OR service role bypass)
      2. Factory → getWAProviderForChannel → {provider, config}
      3. Pre-resolve contact.wa_id (LID routing):
         SELECT wa_id FROM messaging_contacts WHERE org + phone_e164
         Se wa_id existe e contém "@" → usa como chatId
      4. WAHAProvider.sendText(config, {to: chatIdOverride || to, text, ...})
         → POST WAHA /api/sendText
      5. Persist messaging_messages (direction=outbound)
      6. Update conversation last_message_*
      7. Dispatch outbound webhook (external integrations)
  → Response: {ok, provider_message_id, message_id}
  → Realtime push → ChatLive renderiza bubble
```

### Fluxo D — Enviar mensagem (Agent Runtime)

```
Inbound dispatched pra Agent Runtime (V1 ou V3)
  → LLM + steps + tools executam
  → enqueue row em agent_events_outbox:
    {organization_id, channel='whatsapp', payload: {to, type, text, channel_id, conversation_id, meta}}

Drainer poll (15s V1 / 5s V3)
  → Claim rows pending
  → Pra cada row:
    a. resolveChannelVariant(channel_id) → unofficial_qr
    b. sendViaUnofficialEdge:
       - type=text → wa-send-text
       - type=image|video|audio|document|sticker → wa-send-media
       - type=reaction → wa-send-reaction-reply
    c. Headers: Authorization: Bearer SUPABASE_SERVICE_ROLE_KEY
    d. 2xx → markDelivered. Erro → retry/backoff.
  → Stale guard: se new inbound chegou durante a execução, suprime e mark delivered
```

---

## Pontos não-óbvios

### 1. `encrypt_key`/`decrypt_key` dependem de `app.encryption_key` por ambiente

- `app.encryption_key` é ALTER DATABASE-level. Valores encriptados em um ambiente (prod qck) **não viajam** pra outro. Não copie blocos encrypted de um env pro outro.
- Se está null em DEV, o fallback faz sha256 do db name — pode funcionar mas NÃO use isso em prod.
- Ver [docs/runbooks/encryption-master-key.md](../../runbooks/encryption-master-key.md).

### 2. Meta Cloud **vs** unofficial — mesmo schema, code paths diferentes

- Ambos usam `saas_messaging_channels` + `messaging_*`. Discriminados por `provider_variant`.
- Meta Cloud: edge `whatsapp-meta-webhook` inbound + `whatsapp-meta-send` outbound.
- Unofficial: `whatsapp-unofficial-inbound` + `wa-send-*`.
- Shared layer: `runtime-bridge` (agent runtime integration) e `dispatch-gate` (handoff check) atendem ambos.

### 3. LID mode

- WhatsApp migrou users para "LID addressing". Inbound webhook pode vir com `from: <lid>@lid`.
- LID ≠ phone number. Sem as 3 bridges, contatos ficavam com "phone" inexistente tipo `+8160488251617`.
- `wa_id` do contato guarda o JID `<lid>@lid` — é o único formato que roteia outbound pro usuário LID correto.
- Ver [02-fluxo-mensagens.md](./02-fluxo-mensagens.md) seção "LID bridges" pra detalhes.

### 4. Drainer tem dois binários

- V1 (`apps/agent-runtime`) e V3 (`apps/agent-runtime-v3`) rodam em paralelo no Render.
- Ambos fizeram o fix de routing por `provider_variant`. Ambos enxergam a mesma tabela `agent_events_outbox` em Client DB.
- Se um binário fica pra trás em deploy, pode hittar a mesma row duas vezes (race). A claim via `update status='processing'` previne.

### 5. Stale guard

- Se operador humano respondeu enquanto agente estava processando, o outbox do agente é suprimido (`outbox.suppressed.stale_guard` event).
- Comportamento intencional pra evitar spam de respostas obsoletas.
- Logs mostram o suppress; linha delivered pode ser "entregue via suppression" (ver campo `delivered_at` + event log).

### 6. Idempotência

- `wa-start-session` idempotente (422 "session already exists" + "already started" tratados como sucesso).
- `wa-get-qr` idempotente (cada call pega snapshot do QR atual).
- Outbound edges idempotentes por `(org, channel, provider_message_id)` unique constraint em `messaging_messages`.
- Inbound dedup via mesma constraint.

---

## Próximos passos

- [02-fluxo-mensagens.md](./02-fluxo-mensagens.md) — entender o ciclo de msg a fundo, especialmente LID
- [03-schema-dados.md](./03-schema-dados.md) — ver schema completo
- [04-infra-hetzner-coolify.md](./04-infra-hetzner-coolify.md) — entender a infra de quem hospeda o WAHA
