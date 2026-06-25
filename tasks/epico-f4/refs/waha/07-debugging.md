# 07 — Debugging

> Playbook de triagem: onde olhar primeiro, incidentes conhecidos, queries SQL prontas.

---

## Framework de diagnóstico

**Quando alguém reclama "mensagem não chegou" / "agente não respondeu" / "contato estranho"**, siga em ordem:

1. **Reproduz**: pede pro user mandar mensagem NOVA (pra ter timestamp fresh). Anota o minuto exato.
2. **Achar a entry inbound** no DB (veja [queries](#queries-de-triagem)).
3. **Seguir o caminho**: inbound → runtime execution → outbox → drainer → WAHA → destinatário. Em CADA passo, verificar status.
4. **Olhar logs**: Supabase edge logs (dashboard), Render runtime logs, WAHA container logs.

Cada etapa tem uma seção abaixo.

---

## Queries de triagem

### 1. Mensagem inbound chegou?

```sql
-- CLIENT DB
-- Última mensagem em uma conversation
SELECT m.direction, m.type, substring(m.text, 1, 80) AS preview,
       m.provider_message_id, m.status, m.created_at
FROM public.messaging_messages m
JOIN public.messaging_contacts c ON c.id = m.contact_id
WHERE c.phone_e164 = '+5531996787776'
  AND m.created_at > now() - interval '1 hour'
ORDER BY m.created_at DESC
LIMIT 10;
```

Sem rows → problema **antes** do inbound handler (WAHA não fez webhook OU HMAC falhou OU channel_id não bate). Ver [WAHA side](#waha-side).

Com rows mas só inbound → handler OK mas runtime não disparou OU resposta travou. Ir pra [runtime step](#execution-do-runtime).

Com rows inbound + outbound → enviou, mas chegou no chat certo? Ir pra [WAHA side](#waha-side).

### 2. Execution do runtime

```sql
-- MASTER DB
SELECT id, status, channel, idempotency_key,
       input_payload->'metadata'->>'source' AS source,
       substring(input_payload->>'message', 1, 60) AS input_msg,
       created_at
FROM public.agent_executions
WHERE organization_id = '<uuid>'
  AND created_at > now() - interval '30 min'
ORDER BY created_at DESC;
```

- Não tem execution → dispatcher não chamou runtime. Check `shouldDispatchMessageEvents` logic (pode ter `handoff_mode='human'`), check retry queue.
- Status = `running` há >1min → travou. Pode ser OpenAI timeout.
- Status = `failed` → ver `agent_execution_events` pro erro:
  ```sql
  SELECT event_type, created_at, payload
  FROM public.agent_execution_events
  WHERE execution_id = '<execution-id>'
  ORDER BY created_at;
  ```

### 3. Outbox: foi enqueued?

```sql
-- CLIENT DB
SELECT id, status, attempts, channel,
       payload->>'to' AS dest, payload->>'type' AS type,
       substring(payload->>'text', 1, 60) AS text_preview,
       last_error, created_at
FROM public.agent_events_outbox
WHERE organization_id = '<uuid>'
  AND created_at > now() - interval '30 min'
ORDER BY created_at DESC;
```

Pra cada row:
- `status=delivered` → drainer enviou.
- `status=pending` há >1min → drainer não tá rodando OU tá preso. Check Render logs.
- `status=processing` há muito tempo → crash depois de claim. Requeue via:
  ```sql
  UPDATE public.agent_events_outbox
  SET status='pending', lock_token=null, updated_at=now()
  WHERE id='<row-id>';
  ```
- `status=failed`/`dead_letter` + `last_error` com `meta_send_http_400:Invalid channel provider` → drainer rotou pra Meta endpoint em canal unofficial. Verificar `resolveChannelVariant` cache. **Incidente #1** conhecido (fixado PR #221).

### 4. Drainer enviou — chegou no WAHA?

Execute direto no WAHA pra ver últimas mensagens enviadas em uma session:

```bash
WAHA_KEY=$(grep ^WAHA_API_KEY= ~/.secrets/wa-unofficial/shard-1.env | cut -d= -f2-)
SESSION=tomik-xxxxxxxxxxxx
curl -s "https://wa-shard-1.automatiklabs.com.br/api/$SESSION/chats/<phone_or_jid>/messages?limit=5&downloadMedia=false" \
  -H "X-Api-Key: $WAHA_KEY" | jq '[.[] | {id, fromMe, body: (.body // "[media]" | .[:60])}]'
```

- Se a msg aparece com `fromMe: true` + matching body → WAHA aceitou e **provavelmente entregou**.
- Se não aparece → provider não recebeu OR WAHA descartou. Ver WAHA container logs.

### 5. Mensagem chegou em chat errado? (LID issue)

Se destinatário disser "recebi mensagem num chat fantasma", é LID issue — ver [02-fluxo-mensagens.md](./02-fluxo-mensagens.md) seção LID.

Quick check:
```sql
-- CLIENT DB
SELECT id, phone_e164, wa_id, display_name
FROM public.messaging_contacts
WHERE id = '<contact-id>';
```

Se `wa_id` é NULL ou vazio → outbound foi via `phone_e164@c.us` → caiu em shadow.

Fix: backfill manualmente (ver [06-manutencao.md](./06-manutencao.md#cleanup-db)) ou aguardar próximo inbound (handler vai popular via 3 bridges).

---

## Logs — onde olhar

### Supabase edges

**Dashboard**:
1. https://supabase.com/dashboard/project/qckjiolragbvvpqvfhrj/functions
2. Click no edge → "Logs" tab
3. Filter: "Error" first, depois "Info" se precisar narrative

**Via CLI** (Supabase CLI v2.80+):
```bash
supabase functions logs whatsapp-unofficial-inbound --project-ref qckjiolragbvvpqvfhrj --tail 100
```

Nossa CLI atual é v2.39 que não suporta — por isso usamos o dashboard.

**Padrão de logs**:
- `[wa-unofficial-inbound]` prefixo nos nossos logs
- `[Webhook]` no Meta handler (historical)
- `[runtime-bridge]` / `[dispatch-gate]` em shared helpers

### Agent Runtime (Render)

Render dashboard → `agentruntime-tomikcrm` service → "Logs" tab.

Ou via Render API:
```bash
KEY=$(grep ^RENDER_API_KEY= .env | cut -d= -f2-)
SERVICE=srv-d67nvti48b3s73cfmum0  # agentruntime-tomikcrm
curl -s -H "Authorization: Bearer $KEY" \
  "https://api.render.com/v1/services/$SERVICE/logs?limit=100" | jq
```

**Logs relevantes**:
- `[outbox-drainer]` — draining cycles
- `[outbox-drainer-v3]` — V3 drainer
- `[runtime-dispatch]` — incoming requests

### WAHA container

```bash
# Tail últimas 500 lines
ssh wa-shard-1 "docker logs --tail 500 waha"

# Follow live
ssh wa-shard-1 "docker logs -f waha"

# Filter errors only
ssh wa-shard-1 "docker logs waha 2>&1 | grep -iE 'error|fail'"
```

Log format é JSON quando `WAHA_LOG_FORMAT=json`. Parseie via `jq`:
```bash
ssh wa-shard-1 "docker logs --tail 200 waha" | grep -E '^\{' | jq -r '"\(.timestamp) \(.level) [\(.context // "")]\(.msg)"'
```

### Client DB queries (diagnóstico)

Nossa tabela-chave de rastreio é `agent_execution_events` (Master DB):

```sql
SELECT event_type, created_at,
       payload->>'status' AS status,
       payload->>'error' AS error,
       substring(payload::text, 1, 200) AS preview
FROM public.agent_execution_events
WHERE execution_id = '<execution-uuid>'
ORDER BY created_at;
```

Event types chave:
- `execution.started`
- `step.started` / `step.completed`
- `tool_call.recorded` (quando agente chama tool)
- `outbox.enqueued` (resposta gerada)
- `outbox.suppressed.stale_guard` (resposta suprimida — user já seguiu)
- `execution.completed` / `execution.failed`

---

## Incidentes conhecidos (com sintomas)

### #1 — Drainer routa TUDO pra Meta → 400 em canais unofficial

**Sintoma**: agente processa mas reply nunca chega. Row em `agent_events_outbox` com status=`dead_letter`, last_error=`meta_send_http_400:event_0:text:Invalid channel provider`.

**Root cause**: `WHATSAPP_SEND_ENDPOINT` (Meta edge) chamado pra `provider_variant=unofficial_qr`.

**Fix**: PR #221 adicionou `resolveChannelVariant` em V1 + V3 drainer. Rota correta: `unofficial_qr` → nossos `wa-send-*` edges.

**Verificar se o fix tá ativo**: `grep resolveChannelVariant apps/agent-runtime/src/services/outbox-drainer.ts` → se aparecer, OK.

### #2 — Contatos LID com phone inexistente

**Sintoma**: ChatLive mostra contato tipo `+8160488251617` (LID digits) em vez de `+5531992903943` (real). Mensagens do agente parecem ir pra "estranho".

**Root cause**: WAHA entregou webhook sem `_data.key.remoteJidAlt`, fallback extraiu LID digits.

**Fix**: 3 bridges (`remoteJidAlt` → `senderPn` → sufixo `_<phone>@c.us` do id → fallback). Ver [02-fluxo-mensagens.md](./02-fluxo-mensagens.md#13-as-3-bridges-lid--phone).

**Migrar contato manual** (temporário até próximo inbound com bridge):
```sql
UPDATE public.messaging_contacts
SET phone_e164='+5531992903943', display_name='Nome Real'
WHERE id='<lid-contact-id>';
-- wa_id MANTÉM o @lid
```

### #3 — Mensagem aparece 2x (texto + [text] vazio)

**Sintoma**: Em ChatLive, mesma msg de texto renderiza como duas bubbles. Pior: áudio seguido de uma bubble vazia `[text]`.

**Root cause**: echo fromMe de WAHA chegou com id `true_<chatId>_<shortId>` enquanto o outbound persistiu com `<shortId>`. Unique constraint não catchava.

**Fix**: PR #205 — `normalizeWahaProviderMessageId` strips `true_<chatId>_` prefix. Provider message_id sempre comparável.

**Verificar**: 
```sql
SELECT provider_message_id, count(*) FROM public.messaging_messages
WHERE master_channel_id='<uuid>' AND created_at > now() - interval '1 day'
GROUP BY provider_message_id HAVING count(*) > 1;
```
Retornar rows = normalize não tá rodando.

### #4 — Áudio roteando pro chat errado (LID shadow)

**Sintoma**: Texto chega certo no WhatsApp da Preta. Áudio chega em outro chat fantasma.

**Root cause**: `wa-send-text` tinha pre-lookup de `wa_id`, mas `wa-send-media` e `wa-send-reaction-reply` não. Áudio ia com `@c.us`, caía em shadow chat.

**Fix**: PR #206 — lookup `wa_id` replicado nos 3 edges outbound.

**Verificar**: `grep -l "chatIdOverride" supabase/functions/wa-send-*/index.ts` → deve listar 3 arquivos.

### #5 — wa_id sobrescrito `@lid` → `@c.us` → outbound quebra

**Sintoma**: Áudio começou OK, parou de chegar. Inspeção DB mostra `wa_id='<digits>@c.us'` (fraco) em vez de `@lid`.

**Root cause**: Webhook echo fromMe trazia `from: <lid>@lid` → mas depois alguma mensagem subsequente veio com `from: <lid>@c.us` (raro) ou inbound diff format → extraFields overwrite da `wa_id`.

**Fix**: PR #207 — inbound só persiste `wa_id` quando raw JID termina em `@lid`. `@c.us` não é persistido (redundante com `phone_e164`).

**Verificar**: 
```sql
SELECT phone_e164, wa_id FROM public.messaging_contacts
WHERE wa_id IS NOT NULL AND NOT wa_id LIKE '%@lid';
```
Rows aqui = problema. Escalar.

### #6 — Session FAILED logo após criar

**Sintoma**: Clicou "Conectar via QR", QR aparece, scaneia — mas session vai pra FAILED depois de 5-10 seg. Mensagens não entram nem saem.

**Root cause**: Webhook URL apontava pra domínio morto (`edge.automatiklabs.com.br`) — WAHA retry exhausted → session FAILED.

**Fix**: PR #202 — factory usa `MASTER_SUPABASE_URL` (qck canonical) em vez de `SUPABASE_URL` (podia resolver domínio customizado antigo).

**Verificar**: inspecionar a session no WAHA:
```bash
curl -s -H "X-Api-Key:$WAHA_KEY" https://wa-shard-1.automatiklabs.com.br/api/sessions/tomik-xxx | jq '.config.webhooks[0].url'
```
URL deve ser `https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1/whatsapp-unofficial-inbound?channel_id=...`. Se for custom domain morto, recriar session.

### #7 — WAHA Plus engine NOWEB → FAILED

**Sintoma**: session no WAHA-side vai direto pra FAILED sem receber QR.

**Root cause**: NOWEB engine instável em WAHA Plus `noweb-2026.4.2`. Pinamos `WEBJS` em `WAHAProvider.startSession`.

**Fix**: config `metadata.engine: 'WEBJS'` na criação. Já em produção desde PR #202.

**Verificar na session WAHA**:
```bash
curl -s -H "X-Api-Key:$WAHA_KEY" https://wa-shard-1.automatiklabs.com.br/api/sessions/tomik-xxx | jq '.config.metadata.engine'
```
Deve ser `"WEBJS"`. Se NOWEB, recriar.

### #8 — QR Code aparece como imagem verde (placeholder)

**Sintoma**: No ChatLive, QR mostra quadrado verde gigante em vez de QR real.

**Root cause**: Frontend estava usando `fetchQrStub` (placeholder). Antes de PR #218 os edges `wa-start-session`/`wa-get-qr` não existiam.

**Fix**: Frontend agora chama `invokeUnofficialStartSession` + `invokeUnofficialGetQr` (via `wa-unofficial-client.ts`). Edges provisionam session e retornam PNG real.

**Verificar**: `grep fetchQrStub src/` — não deve retornar match.

### #9 — Agente respondeu um contato "do além"

**Sintoma**: Rafael viu mensagem do agente em um chat de um contato que ele "não conhece".

**Root cause**: Athila Mendes realmente enviou mensagem, mas webhook trouxe `from: 8160488251617@lid` sem bridges. Contato salvo com phone LID. Agente respondeu via `wa_id=@lid` → chegou na conversa real mas user via no CRM um phone bogus.

**Fix**: PR #222 — bridge 3 (sufixo do id) captura phone real quando as outras duas faltam.

Se ainda acontecer em outros casos (contato LID sem NENHUMA bridge funcionar), considerar:
- Cross-reference manual via WAHA chats overview
- Adicionar handoff_mode=human pra contatos novos (ver roadmap [09-melhorias-futuras.md](./09-melhorias-futuras.md))

### #10 — MCP project ref mentiroso em dev

**Sintoma**: Você vê `supabase-DEV` no MCP mas migration tá batendo em um project estranho (`xfir...`).

**Root cause**: MCP wrapper npm pode ter project_ref mal-configurado.

**Fix**: Sempre cross-reference `MASTER_SUPABASE_URL` do `.env` antes de escrever. Ver [`memory/feedback_mcp_project_ref_verification.md`](../../../.claude/projects/-Users-rafaelmelgaco-Downloads-tomikcrm/memory/feedback_mcp_project_ref_verification.md).

---

## Fluxo de triagem completo (flowchart)

```
User reclama "agente não respondeu"
  ↓
Mensagem inbound chegou? (query em messaging_messages)
  SIM → Continua
  NÃO → Check HMAC webhook validation (server logs 401)
      → Check WAHA session status
      → Check channel_id no query string do webhook
  ↓
Execution criada? (agent_executions)
  SIM → Continua
  NÃO → Check dispatch-gate (handoff_mode=human?)
      → Check runtime-bridge logs (pipeline resolution failed?)
      → Check saas_agent_dispatch_retry_queue (runtime HTTP 5xx?)
  ↓
Outbox row criada? (agent_events_outbox)
  SIM → Continua
  NÃO → Execution falhou sem gerar output. Ver events.
  ↓
Outbox drained com sucesso? (status=delivered)
  SIM → Continua
  status=pending → drainer morto ou preso
  status=processing → crash durante claim
  status=failed/dead_letter → ler last_error
  ↓
Mensagem chegou no WAHA? (curl /chats/.../messages)
  SIM → Continua
  NÃO → WAHA container problem. Ver docker logs.
  ↓
Caiu no chat correto? (WAHA chat overview)
  SIM → Resolvido (talvez delay de WA cell)
  NÃO → LID routing falhou. Ver [incidente #2/#4/#5].
```

---

## Como reproduzir bugs end-to-end

Reproduzir no DB (sem precisar do Apolo mandar msg):

```bash
# 1. Inserir outbox row artificial
psql "$SUPABASE_MASTER_DB_URL" <<'SQL'
INSERT INTO public.agent_events_outbox
  (id, organization_id, execution_id, channel, payload, status, attempts, available_at, created_at, updated_at)
VALUES (
  gen_random_uuid(),
  '<org-uuid>',
  NULL,
  'whatsapp',
  jsonb_build_object(
    'to', '+<phone>',
    'type', 'text',
    'text', 'Teste reproduzir bug',
    'channel_id', '<channel-uuid>',
    'conversation_id', '<conv-uuid>'
  ),
  'pending',
  0,
  now(), now(), now()
)
RETURNING id;
SQL

# 2. Esperar drainer (15s V1 / 5s V3). Verificar status:
psql "$SUPABASE_MASTER_DB_URL" -c "SELECT id, status, attempts, last_error FROM public.agent_events_outbox WHERE id='<inserted-id>';"

# 3. Confirmar chegada no WAHA:
curl -s -H "X-Api-Key:$WAHA_KEY" https://wa-shard-1.automatiklabs.com.br/api/tomik-xxx/chats/<phone>@c.us/messages?limit=1 | jq
```

Probe direto de edge sem passar pelo drainer:
```bash
KEY=<SUPABASE_SERVICE_ROLE_KEY>
curl -X POST "https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1/wa-send-text" \
  -H "Authorization: Bearer $KEY" -H "content-type: application/json" \
  -d '{"channel_id":"<uuid>","to":"+5531996787776","text":"probe"}' | jq
```

---

## Ferramentas úteis

### psql remoto
```bash
# $SUPABASE_MASTER_DB_URL já em .env
source .env && psql "$SUPABASE_MASTER_DB_URL"
```

Alguns psqlrc:
```
\pset pager off
\x auto
```

### Client DB pra Tomikos

Não tem no `.env` direto — tem que decriptar via Master:
```bash
source .env
CKEY=$(psql "$SUPABASE_MASTER_DB_URL" -t -A -c "SELECT decrypt_key(service_role_encrypted) FROM public.saas_memberships WHERE organization_id_in_client='4d99a24f-9b1f-4eb5-ae22-063a26375f75' LIMIT 1;" | tr -d '[:space:]')
# CKEY agora tem a key do client DB. Usar em:
curl -H "apikey: $CKEY" -H "authorization: Bearer $CKEY" "https://lwarydswfavmuzbccqmd.supabase.co/rest/v1/messaging_contacts?limit=1"
```

### WAHA CLI-like (pronto pra colar)

```bash
# .bashrc / .zshrc
wa-key() { grep ^WAHA_API_KEY= ~/.secrets/wa-unofficial/shard-1.env | cut -d= -f2- ; }
wa-sessions() { curl -s -H "X-Api-Key:$(wa-key)" https://wa-shard-1.automatiklabs.com.br/api/sessions | jq '[.[] | {name, status, me: .me.id}]' ; }
wa-chat() {
  local session=$1; local chat=$2
  curl -s -H "X-Api-Key:$(wa-key)" "https://wa-shard-1.automatiklabs.com.br/api/$session/chats/$chat/messages?limit=5&downloadMedia=false" \
    | jq '[.[] | {id, fromMe, body: (.body // "[media]")[0:60]}]'
}
```

---

## Próximos

- [08-referencias-externas.md](./08-referencias-externas.md) — quando a issue foge do nosso código
- [09-melhorias-futuras.md](./09-melhorias-futuras.md) — roadmap e débitos técnicos
