# 02 — Fluxo de Mensagens (Deep-Dive)

> Como uma mensagem inbound vira agent reply visível no WhatsApp do contato, em detalhes. Foco nas partes não-óbvias: LID bridges, normalização de IDs, dedup, routing outbound.

---

## Por que isso importa

A maioria dos bugs que enfrentamos durante o MVP foram nesse pipeline:

- **Ghost contact**: contato salvo com LID como phone (+96229... em vez de +5531...)
- **Reply no chat errado**: outbound com `@c.us` cai em shadow chat, não no chat real
- **Mensagem duplicada**: echo do fromMe criava row fantasma
- **Agente respondeu desconhecido**: LID sem bridge criou contato "de outro planeta"

Todos esses foram resolvidos pelas bridges e normalizações descritas aqui.

---

## 1. Ciclo inbound passo a passo

### 1.1. WAHA fires webhook

Request:
```
POST https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1/whatsapp-unofficial-inbound?channel_id=<uuid>
x-webhook-hmac: <sha512_hex>
content-type: application/json

{
  "event": "message",                  // ou message.any | message.reaction | state.change | session.status
  "session": "tomik-xxxxxxxxxxxx",
  "payload": { ... WAHA message ... }
}
```

### 1.2. Handler — etapas

Código: `supabase/functions/whatsapp-unofficial-inbound/index.ts`

```
1. Validar método + body
2. Resolver channel por channel_id (query string)
3. HMAC validation: SHA512(rawBody, webhook_token_plaintext)
4. Parse envelope
5. Handle state.change / session.status → UPDATE channel, return 200
6. Resolve tenant credentials (resolveOrgCredentials)
7. Create client (tenant DB)
8. Pra event=message/message.any/message.reaction:
   8.1. Extrair providerMessageId (normalizado)
   8.2. Resolver phone_e164 + rawJid (3 bridges LID)
   8.3. Upsert contact com wa_id (só @lid)
   8.4. Upsert conversation
   8.5. Resolver reply_to (se houver)
   8.6. Insert row em messaging_messages (dedup 23505)
   8.7. Se !fromMe:
        - Update conversation last_message + unread_count
        - Dispatch gate (handoff_mode check)
        - Se dispatch:ok → triggerRuntimeDispatch → Agent Runtime
        - Se dispatch falhou → enqueueDispatchRetry
9. Return 200
```

### 1.3. As 3 bridges LID ↔ Phone

**Problema**: WhatsApp introduziu "LID addressing" em que contatos têm dois identificadores:
- **Phone number JID**: `<digits>@c.us` (formato tradicional)
- **LID JID**: `<random-digits>@lid` (shadow identifier, não é telefone)

Para contatos em LID mode, WAHA entrega `from: <lid>@lid` no webhook. Se a gente extrai os dígitos como se fosse telefone, salva contato com número inexistente.

Temos 3 caminhos alternativos pra achar o phone real. Aplicados em ordem, o primeiro que retornar não-null é usado:

#### Bridge 1 — `_data.key.remoteJidAlt` (WAHA Plus 2026+)

```json
"_data": {
  "key": {
    "remoteJid": "96229312749721@lid",
    "remoteJidAlt": "553192903943@s.whatsapp.net",   ← phone real aqui
    "addressingMode": "lid"
  }
}
```

`phoneFromWahaJid("553192903943@s.whatsapp.net")` → `+5531992903943` (com canonicalização BR mobile 12→13 dígitos).

Presente em ~60% dos payloads. Primeiro a checar.

#### Bridge 2 — `senderPn`

```json
{
  "from": "96229312749721@lid",
  "senderPn": "553192903943@s.whatsapp.net"
}
```

Campo irmão. Nem sempre presente na mesma mensagem que `remoteJidAlt`.

#### Bridge 3 — Sufixo do `payload.id`

```
id: "2ACE06974F75920BE0FD_558588950411@c.us"
                         ~~~~~~~~~~~~~~~~~
```

WAHA NOWEB (e alguns paths do WEBJS) põe a JID do chat como sufixo do id. Regex: `/_(\d{10,15})@c\.us$/`.

Essa bridge foi crucial no caso **Athila Mendes** que chegou sem `remoteJidAlt` e sem `senderPn`. Ver incident em [07-debugging.md](./07-debugging.md).

#### Fallback final

Se nenhuma bridge retornou phone, usa os dígitos do próprio `from` (mesmo que seja `@lid`). Cria contato com phone inexistente, mas pelo menos o `wa_id` vai amarrar o outbound corretamente.

Código:
```ts
const phoneE164 =
  phoneFromWahaJid(remoteJidAlt)          // bridge 1
  || phoneFromWahaJid(messagePayload.senderPn)  // bridge 2
  || phoneFromWahaJid(idPhoneJid)          // bridge 3 (sufix)
  || phoneFromWahaJid(messagePayload.from) // fallback
  || phoneFromWahaJid(messagePayload.author)
```

#### Extra — self-healing por wa_id

Se após as 3 bridges o phone ainda é igual aos dígitos do `@lid` (nenhuma bridge resolveu), faz query final:

```sql
SELECT phone_e164
FROM messaging_contacts
WHERE organization_id = ? AND wa_id = ?
```

Se já existe contato com esse `@lid` salvo (de um inbound anterior que TEVE bridge), reaproveita o phone_e164 salvo. Previne criação de ghost contacts em eventos echo (fromMe=true) que WAHA não enriquece.

### 1.4. `rawJid` — a chave pra outbound

Paralelo à extração do phone, montamos `rawJid` — o JID que vai no campo `messaging_contacts.wa_id`:

```ts
const rawJid = primaryFrom.endsWith('@lid')
  ? primaryFrom                                         // @lid quando é o real identificador
  : (remoteJidAlt || messagePayload.senderPn || primaryFrom || null)
```

Regra de escrita no DB: **só persistimos `wa_id` se termina em `@lid`**. `@c.us` é redundante com `phone_e164` (outbound default já constrói isso). Persistir `@c.us` overwrites uma `@lid` previamente resolvida — bug que perseguimos no fix "preserve @lid wa_id".

### 1.5. Normalização do `provider_message_id`

WAHA usa 3+ formatos diferentes pro mesmo id lógico:

| Fonte | Formato | Exemplo |
|-------|---------|---------|
| Response de `sendText` | `<shortId>` | `3EB0C621994F26BCC61AC9` |
| Webhook echo (fromMe) | `true_<chatId>_<shortId>` | `true_96229312749721@lid_3EB0C621994F26BCC61AC9` |
| Webhook inbound (NOWEB) | `<shortId>_<chatId>` | `2ACE06974F75920BE0FD_558588950411@c.us` |

Pra dedup funcionar via unique constraint, normalizamos pro formato canônico `<shortId>`:

```ts
// waha-normalizer.ts
export function normalizeWahaProviderMessageId(raw) {
  if (!raw) return null
  const s = String(raw).trim()
  const m = s.match(/^(?:true|false)_[^_]+_(.+)$/)
  return m ? m[1] : s
}
```

### 1.6. Dedup

Unique constraint em `messaging_messages`:
```sql
UNIQUE (organization_id, master_channel_id, provider_message_id)
```

INSERT returna 23505 (unique violation) → handler captura + faz SELECT pra achar a row existente + retorna ela como idempotent ack.

### 1.7. Dispatch pro Agent Runtime

Via `triggerRuntimeDispatch(message, options)` (shared em `runtime-bridge.ts`).

Options:
```ts
{
  client,                  // tenant supabase (pra pipeline lookup em BYO)
  masterFallbackClient,    // Master (pipelines podem estar só em Master)
  limitClient: master,     // daily messaging limit check
  source: 'whatsapp-unofficial-inbound',
  provider: 'unofficial_qr',
}
```

Pipeline resolution:
1. Filtro por `agent_pipelines WHERE organization_id=? AND is_active=true`
2. Publicações em `agent_pipeline_versions WHERE status='published'`
3. Match por `trigger.channel_ids` → primeiro com canal específico, senão generic
4. V3 se `definition.version >= 3` (POST `/v3/ingest`), senão V1 (POST `/api/v1/runtime/ingest`)

Erro de HTTP → `enqueueDispatchRetry` na `saas_agent_dispatch_retry_queue` do Master. Cron externo drena.

---

## 2. Ciclo outbound (humano via ChatLive)

### 2.1. Chamada do frontend

```ts
// ChatLive.tsx
await invokeUnofficialSendText({
  channel_id: activeConversation.master_channel_id,
  to: activeContact.phone_e164,       // "+5531992903943" (phone real)
  text: messageText,
  conversation_id: activeConversation.id,
  reply_to_provider_message_id: replyDraft?.providerMessageId || null,
})
```

### 2.2. Edge wa-send-text

Código: `supabase/functions/wa-send-text/index.ts`

```
1. Auth: Bearer JWT (validate) ou service role (bypass)
2. Ownership: channel.owner_id === user.id (skip em service role)
3. Factory: getWAProviderForChannel → {provider, config, channel}
4. Pre-resolve contact.wa_id:
   SELECT wa_id FROM messaging_contacts WHERE org + phone_e164 = to
   Se wa_id tem "@" → chatIdOverride = wa_id
5. Provider.sendText(config, {to: chatIdOverride || to, text, replyTo})
   - WAHAProvider.toChatId: se "to" contém "@" usa direto, senão monta "<digits>@c.us"
   - POST /api/sendText { chatId, text, reply_to }
6. Persist messaging_messages (direction=outbound, dedup por provider_message_id)
7. Update conversation last_message_at + preview
8. dispatchOutboundWebhooks (external integrations, non-blocking)
9. Return {ok, provider_message_id, message_id, conversation_id}
```

### 2.3. Por que `wa_id` é essencial

Cenário: Preta tá em LID mode. Real phone `+5531992903943`, LID `96229312749721`.

**Sem wa_id lookup**:
```
to: "+5531992903943"
→ toChatId → "5531992903943@c.us"
→ WAHA tenta criar novo chat shadow → NÃO chega no WhatsApp real da Preta
```

**Com wa_id lookup**:
```
to: "+5531992903943"
→ query DB → wa_id = "96229312749721@lid"
→ chatIdOverride = "96229312749721@lid"
→ toChatId pass-through (já é JID) → "96229312749721@lid"
→ WAHA resolve LID → Preta real → msg entregue ✓
```

### 2.4. Persist outbound

```ts
INSERT INTO messaging_messages (
  organization_id, conversation_id, contact_id, crm_lead_id,
  master_channel_id,
  direction='outbound',
  type='text',
  text,
  has_attachments=false,
  provider='whatsapp',
  provider_variant='unofficial_qr',
  provider_message_id,
  reply_to_message_id, reply_to_provider_message_id,
  status='sent',
  sent_at,
  ...
)
```

Em 23505 (dedup) → retorna row existente.

### 2.5. Echo do WAHA (mesma mensagem recebida duas vezes?)

Quando enviamos outbound, WAHA dispara webhook `message` com `fromMe=true` pra nossa session. Inbound handler recebe.

- `providerMessageId` normalizado é o mesmo `<shortId>` que persistimos.
- INSERT em messaging_messages hitta unique constraint → noop.
- `dispatch_gate` previne triggerar runtime pra msg própria.

Sem normalização, o echo teria id `true_<chatId>_<shortId>` — diferente do `<shortId>` que salvamos. Unique constraint não catcharia → ghost row com type=text e text=null.

---

## 3. Ciclo outbound (Agent Runtime via outbox)

Ver [01-arquitetura.md](./01-arquitetura.md) fluxo D.

Passos-chave:
1. Pipeline gera resposta → persist em `agent_events_outbox` (Client DB) com payload contendo `channel_id`, `to`, `type`, `text`, `meta.conversation_id`.
2. Drainer V1/V3 poll e claim rows pending.
3. `resolveChannelVariant(channelId)` via Master cache → `unofficial_qr`.
4. `sendViaUnofficialEdge(channelId, to, eventPayload)` → roteia:
   - `text` → `wa-send-text`
   - `image|video|audio|document|sticker` → `wa-send-media`
   - `reaction` → `wa-send-reaction-reply`
5. Header `Authorization: Bearer ${SUPABASE_SERVICE_ROLE_KEY}` — edges aceitam service role como internal-trust.
6. 2xx → markDelivered. Erro → retry ou dead_letter.

### Stale guard

Se `meta.stale_guard_enabled=true` + `conversation_id` + `execution_id`:
- Query `messaging_messages` por `direction='inbound' AND created_at > execution.started_at`
- Se achar: mark delivered + emit event `outbox.suppressed.stale_guard`, NÃO envia

Evita que agente responda com delay de minutos depois que humano já seguiu a conversa.

---

## 4. Erros comuns e como identificar

| Sintoma | Causa provável | Evidência |
|---------|----------------|-----------|
| Contato no ChatLive com número estranho tipo `+8160488251617` | Bridges não acharam phone real; fallback pegou LID digits | `messaging_contacts.phone_e164` bate com `wa_id` dígitos |
| Mensagem do agente vai pra chat fantasma | `wa_id` null no contato OU foi sobrescrito pra `@c.us` | `SELECT phone_e164, wa_id FROM messaging_contacts WHERE ...` — wa_id deveria ser `@lid` |
| Mensagem aparece duplicada em ChatLive (texto + [text] vazio) | Echo não foi deduplicado | `SELECT provider_message_id, text FROM messaging_messages WHERE conversation_id=...` — vê 2 rows com IDs `<shortId>` e `true_..._<shortId>` |
| Agente recebeu + processou mas reply nunca chegou | Drainer rotou pra Meta endpoint e tomou 400 | `SELECT status, last_error FROM agent_events_outbox WHERE id=?` → `"meta_send_http_400:...Invalid channel provider"` |
| QR aparece verde (placeholder) | Stub não substituído | Log frontend `[WhatsAppUnofficialCard] fetchQrStub in use` (não deveria existir mais; se aparecer, revisar `fetchQrOnly`) |
| Session FAILED recorrente | Webhook URL aponta pra domínio morto (antes: `edge.automatiklabs.com.br`) OU engine NOWEB em shard que preferia WEBJS | WAHA `/api/sessions/{name}` retorna `status=FAILED` |

Mais detalhes em [07-debugging.md](./07-debugging.md).

---

## 5. Checklist pra implementar outbound novo (ex: wa-send-location)

Se precisar adicionar um novo tipo de envio, siga:

1. **Edge**: criar `supabase/functions/wa-send-location/index.ts` seguindo o padrão dos existentes.
   - Auth pattern (JWT + service role bypass)
   - Factory → provider + config
   - **Pre-resolve contact.wa_id** (não esquece — LID routing)
   - Provider call + persist + dispatchOutbound
2. **Provider**: adicionar `sendLocation(config, params)` em `WAProvider` interface + implementar em `WAHAProvider`.
3. **Client wrapper**: `src/lib/wa-unofficial-client.ts` — `invokeUnofficialSendLocation`.
4. **ChatLive**: chamar o novo wrapper quando `conversation.provider_variant === 'unofficial_qr'`.
5. **Drainer routing**: `sendViaUnofficialEdge` em ambos drainers — adicionar `case 'location'` pra rotear.
6. **Fixture tests**: `supabase/functions/_shared/__tests__` — adicionar fixture pro payload.

## Próximos

- [03-schema-dados.md](./03-schema-dados.md) — os campos e queries prontos
- [07-debugging.md](./07-debugging.md) — playbook de diagnóstico
