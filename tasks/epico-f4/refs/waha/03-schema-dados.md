# 03 — Schema de dados

> Tabelas envolvidas, colunas críticas, queries prontas pra diagnóstico.

---

## Visão geral

```
┌─────────────── MASTER DB (qck) ───────────────┐    ┌──── CLIENT DB (BYO ou tomik_hosted) ────┐
│                                                │    │                                          │
│  saas_messaging_channels                       │    │  messaging_contacts                      │
│  saas_messaging_channel_secrets                │◄──►│  messaging_conversations                 │
│  saas_agent_dispatch_retry_queue               │    │  messaging_messages                      │
│  saas_organizations                            │    │  agent_events_outbox                     │
│  saas_memberships (service_role_encrypted)     │    │                                          │
│                                                │    │                                          │
│  agent_executions (runtime)                    │    │                                          │
│  agent_execution_events                        │    │                                          │
└────────────────────────────────────────────────┘    └──────────────────────────────────────────┘
```

- **Master DB** = qck (`qckjiolragbvvpqvfhrj.supabase.co`). Tabelas da identidade, billing, credenciais de canais.
- **Client DB** = depende de `saas_organizations.database_provider`:
  - `byo` → URL própria em `saas_organizations.client_supabase_url` (key encriptada em `saas_memberships.service_role_encrypted`)
  - `tomik_hosted` → TomikHosted project (`phpdqhmurfcftwqclqwg.supabase.co`)

---

## Master DB — tabelas

### `saas_messaging_channels`

Um registro por instância de canal (Meta ou QR).

| Coluna | Tipo | Uso |
|--------|------|-----|
| `id` | uuid PK | Channel UUID. É o que sobe pelo frontend como `master_channel_id`. |
| `saas_organization_id` | uuid | FK → `saas_organizations.id` |
| `client_org_id` | uuid | Mirror pro client DB lookups |
| `owner_id` | uuid | FK → `saas_users.id`. RLS: `USING (owner_id = auth.uid())`. |
| `provider` | text | `'whatsapp'` ou `'instagram'` |
| `provider_variant` | text | `'meta_cloud'`, `'meta_instagram'`, `'unofficial_qr'`, `'baileys'` (deprecated) |
| `status` | text | `'disconnected'`, `'connecting'`, `'connected'`, `'error'` |
| `is_active` | bool | Liga/desliga canal |
| `label` | text | "Nova instância QR", nome custom do usuário |
| `display_phone_number` | text | E.164 do número pairado (preenchido em `session.status=WORKING`) |
| `last_seen_at` | timestamptz | Última atividade |
| `last_error_code`, `last_error_message` | text | Erros recentes |
| **Meta-específicas** |  | |
| `meta_waba_id`, `meta_phone_number_id`, `meta_business_account_id` | text | — |
| `meta_ig_user_id`, `meta_page_id` | text | Instagram |
| **Unofficial-específicas** |  | |
| `unofficial_instance_name` | text | Nome da session WAHA (ex: `tomik-fd4a824702b941f6`) |
| `unofficial_provider_host_url` | text | Shard URL (ex: `https://wa-shard-1.automatiklabs.com.br`) |
| `unofficial_session_id` | text | Reservado pra futuro multi-session por canal |
| `unofficial_last_qr_at` | timestamptz | Último fetch de QR (observabilidade) |
| `unofficial_disconnect_reason` | text | Razão da desconexão quando `status=disconnected` |

**CHECK constraints:**
```sql
provider IN ('whatsapp', 'instagram')
provider_variant IN ('meta_cloud', 'baileys', 'meta_instagram', 'unofficial_qr')
status IN ('disconnected', 'connecting', 'connected', 'error')
```

**RLS:**
- `saas_messaging_channels_owner_all` — full CRUD quando `owner_id = auth.uid()`
- `saas_messaging_channels_member_select` — SELECT quando membership active na org

**Indexes:**
- `(saas_organization_id, is_active)` — listagem por org
- `(client_org_id, is_active)`
- `(provider, provider_variant)`
- `(provider_variant, status) WHERE is_active` — drainers filtram por isso
- UNIQUE `(unofficial_instance_name) WHERE ... IS NOT NULL`

### `saas_messaging_channel_secrets`

| Coluna | Tipo | Uso |
|--------|------|-----|
| `channel_id` | uuid PK | FK → `saas_messaging_channels.id` ON DELETE CASCADE |
| **Meta** |  | |
| `meta_access_token_encrypted` | text | User Access Token encriptado |
| `meta_app_secret_encrypted` | text | Pra BYO Meta app (webhook HMAC) |
| `meta_webhook_verify_token_encrypted` | text | Challenge do GET /webhook |
| ... outros `meta_*_encrypted` | text | Refresh token, 2FA pin, CAPI, Pixel ID |
| **Unofficial** |  | |
| `unofficial_api_key_encrypted` | text | **SHA512 hash** do X-Api-Key da WAHA (NÃO plaintext, ver [05-credenciais.md](./05-credenciais.md)) |
| `unofficial_webhook_token_encrypted` | text | HMAC-SHA512 secret pra validar webhooks inbound |
| `unofficial_auth_state_encrypted` | text | Reservado (Baileys legado) |

**Encriptação**:
```sql
-- Escrever (via edge):
plaintext_enc := encrypt_key(plaintext);

-- Ler:
plaintext := decrypt_key(ciphertext);
```

`encrypt_key`/`decrypt_key` são RPCs `SECURITY DEFINER` usando `pgp_sym_encrypt` com senha em `app.encryption_key` (ALTER DATABASE). Ver [runbooks/encryption-master-key.md](../../runbooks/encryption-master-key.md).

### `saas_agent_dispatch_retry_queue`

Migration: `supabase/master-migrations/20260424100000_agent_dispatch_retry_queue.sql`

| Coluna | Tipo | Uso |
|--------|------|-----|
| `id` | uuid PK | |
| `organization_id` | uuid | Master org (mesmo da `saas_organizations.id`) |
| `channel_id` | uuid | Channel que receberia dispatch |
| `provider` | text | `'unofficial_qr'` / `'meta_cloud'` |
| `source` | text | Edge de origem (`whatsapp-meta-webhook`, `whatsapp-unofficial-inbound`) |
| `message_payload` | jsonb | `RuntimeBridgeMessage` completo — re-invocável |
| `attempts` | int | Incrementa a cada retry |
| `max_attempts` | int | Default 3 |
| `next_retry_at` | timestamptz | Backoff: 30s → 2m → 10m |
| `last_error` | text | Último erro |
| `status` | text | `'pending'` → `'completed'` | `'dead_letter'` |
| `created_at`, `updated_at`, `completed_at` | timestamptz | |

**Index crítico:**
```sql
idx_agent_dispatch_retry_due ON (next_retry_at) WHERE status = 'pending'
```

### `saas_memberships` (referência)

| Coluna | Uso |
|--------|-----|
| `organization_id_in_client` | UUID no client DB (não no Master) |
| `supabase_url`, `supabase_key_encrypted`, `service_role_encrypted` | Creds do client BYO |

Usado por `resolveOrgCredentials` em edges.

---

## Client DB — tabelas

### `messaging_contacts`

| Coluna | Uso |
|--------|-----|
| `id` | uuid PK |
| `organization_id` | client_org_id |
| `phone_e164` | **Número real** (via bridges LID). Ex: `+5531992903943`. UNIQUE com org. |
| `wa_id` | **JID raw** (`<lid>@lid` ou `<phone>@c.us`). Crítico pra outbound routing em LID mode. |
| `display_name` | pushName do WhatsApp ou nome custom |
| `profile_photo_url` | foto do WhatsApp |
| `is_blocked` | bool |
| `crm_lead_id` | FK pro CRM (se integrado com pipeline) |
| `provider`, `provider_variant` | Origem do contato |
| `last_inbound_at`, `last_outbound_at` | Freshness |
| `external_user_id`, `external_username` | Reservado (Instagram) |

**UNIQUE** `(organization_id, phone_e164)` — garante dedup ao cruzar inbound + manual "+".

### `messaging_conversations`

| Coluna | Uso |
|--------|-----|
| `id` | uuid PK |
| `organization_id`, `contact_id`, `master_channel_id` | Composite unique |
| `provider`, `provider_variant` | Mirror do canal |
| `handoff_mode` | `'agent'`, `'human'`, `'hybrid'`, null. Dispatch gate lê isso. |
| `last_message_at`, `last_message_preview` | Sort/listagem |
| `conversation_window_expires_at` | Meta 24h (não aplicável pra unofficial) |
| `unread_count` | Badge |
| `assigned_to_user_id` | Atendente atribuído |

**UNIQUE** `(organization_id, master_channel_id, contact_id)`.

### `messaging_messages`

| Coluna | Uso |
|--------|-----|
| `id` | uuid PK |
| `organization_id`, `conversation_id`, `contact_id`, `master_channel_id` | Links |
| `direction` | `'inbound'` ou `'outbound'` |
| `type` | `'text'`, `'image'`, `'video'`, `'audio'`, `'document'`, `'sticker'`, `'reaction'`, `'template'` |
| `text` | Body (null pra mídias sem caption) |
| `has_attachments` | bool |
| `provider_message_id` | **Normalizado** (strip `true_<chatId>_` prefix) |
| `reply_to_provider_message_id` | Se for thread reply |
| `status` | `'sent'`, `'delivered'`, `'read'`, `'failed'` |
| `sent_at`, `delivered_at`, `read_at` | Analytics |

**UNIQUE** `(organization_id, master_channel_id, provider_message_id)` — dedup inbound echo.

### `agent_events_outbox`

| Coluna | Uso |
|--------|-----|
| `id` | uuid PK |
| `organization_id` | client_org_id |
| `execution_id` | FK → `agent_executions` (Master) |
| `channel` | `'whatsapp'`, `'instagram'`, `'silence_followup'`, etc |
| `payload` | jsonb do evento (ex: `{to, type, text, channel_id, meta}`) |
| `status` | `'pending'`, `'processing'`, `'delivered'`, `'failed'`, `'dead_letter'` |
| `attempts` | int (max 3 então dead_letter) |
| `lock_token` | uuid pra claim race-free |
| `available_at` | Quando liberar retry (se backoff) |
| `last_error` | Último erro |
| `delivered_at` | Quando marcou delivered |

---

## Queries prontas

### Encontrar channel por WAHA session name

```sql
SELECT id, label, status, saas_organization_id, unofficial_provider_host_url
FROM public.saas_messaging_channels
WHERE unofficial_instance_name = 'tomik-1e00ae8e538648ba';
```

### Ver secrets de um canal (plaintext)

```sql
SELECT
  decrypt_key(unofficial_api_key_encrypted) AS api_key_sha512,
  decrypt_key(unofficial_webhook_token_encrypted) AS hmac_secret
FROM public.saas_messaging_channel_secrets
WHERE channel_id = '1e00ae8e-5386-48ba-a99c-7c6e1c7df70a';
```

### Ver todos contatos LID de uma org

```sql
-- Rodar no CLIENT DB (lwarydswfavmuzbccqmd pra Tomikos)
SELECT id, phone_e164, wa_id, display_name, last_inbound_at
FROM public.messaging_contacts
WHERE organization_id = '4d99a24f-9b1f-4eb5-ae22-063a26375f75'
  AND wa_id LIKE '%@lid'
ORDER BY last_inbound_at DESC;
```

### Retry queue — o que tá travado?

```sql
-- Master DB
SELECT status, count(*), max(attempts) AS max_retries
FROM public.saas_agent_dispatch_retry_queue
GROUP BY status;

-- Dead-letter recentes
SELECT id, organization_id, channel_id, provider, last_error, attempts, created_at
FROM public.saas_agent_dispatch_retry_queue
WHERE status = 'dead_letter'
ORDER BY created_at DESC
LIMIT 20;
```

### Outbox drainer health — está drenando?

```sql
-- Client DB
SELECT status, count(*), min(created_at) AS oldest, max(created_at) AS newest
FROM public.agent_events_outbox
WHERE created_at > now() - interval '1 hour'
GROUP BY status;
```

### Rotear por variant (o que o drainer faz)

```sql
SELECT id, provider_variant
FROM public.saas_messaging_channels
WHERE id = '<channel_uuid>';
-- unofficial_qr → wa-send-* edges
-- meta_cloud → whatsapp-meta-send
```

### Contatos com ghost phone (dígitos ≈ LID)

```sql
-- Client DB — detecta contatos onde phone_e164 bate com os dígitos do wa_id (LID não resolvido)
SELECT id, phone_e164, wa_id, display_name
FROM public.messaging_contacts
WHERE wa_id LIKE '%@lid'
  AND replace(phone_e164, '+', '') = split_part(wa_id, '@', 1);
-- Se retornar rows: as 3 bridges não resolveram phone real pra esses contatos.
-- Considerar WAHA chats overview pra cross-reference manualmente.
```

### Mensagens duplicadas (antes da normalize fix)

```sql
-- Client DB — se ainda vir essa query retornar rows em algum canal, dedup do provider_message_id tá quebrado
SELECT provider_message_id, count(*)
FROM public.messaging_messages
WHERE master_channel_id = '<channel_uuid>'
  AND created_at > now() - interval '1 day'
GROUP BY provider_message_id
HAVING count(*) > 1;
```

### Conversation history pra um contato

```sql
-- Client DB
SELECT m.direction, m.type, substring(m.text, 1, 80) AS preview,
       m.provider_message_id, m.status, m.created_at
FROM public.messaging_messages m
JOIN public.messaging_contacts c ON c.id = m.contact_id
WHERE c.phone_e164 = '+5531992903943'
  AND m.master_channel_id = '<channel_uuid>'
ORDER BY m.created_at DESC
LIMIT 20;
```

---

## Migrations relevantes

```
supabase/migrations/20260423120000_wa_unofficial_schema.sql          (schema inicial)
supabase/migrations/rollback/20260423120000_wa_unofficial_schema_rollback.sql
supabase/master-migrations/20260424100000_agent_dispatch_retry_queue.sql
```

Migrations no Client DB são aplicadas via `supabase/UPDATE-v<N>-CLIENTE-SQL.md` manifests + `SupabaseAutoUpdater` component (ver CLAUDE.md seção "Client DB Migration Pattern").

---

## Próximos

- [04-infra-hetzner-coolify.md](./04-infra-hetzner-coolify.md) — onde vive o WAHA
- [07-debugging.md](./07-debugging.md) — playbook completo com mais queries
