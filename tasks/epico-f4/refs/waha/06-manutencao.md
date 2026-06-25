# 06 — Manutenção

> Playbook operacional — operações corriqueiras e ocasionais.

---

## Índice rápido

- [Checagem de saúde diária](#checagem-de-saude-diaria)
- [Recriar session (QR corrompido / session FAILED)](#recriar-session)
- [Desconectar / reconectar usuário](#desconectarreconectar-usuario)
- [Atualizar versão WAHA](#atualizar-versao-waha)
- [Backup on-demand](#backup-on-demand)
- [Restore from backup](#restore-from-backup)
- [Adicionar shard novo](#adicionar-shard-novo)
- [Migrar session entre shards](#migrar-session-entre-shards)
- [Deploy de edges](#deploy-de-edges)
- [Rollback de edges](#rollback-de-edges)
- [Cleanup de rows orphan no DB](#cleanup-db)

---

## Checagem de saúde diária

Script-base (cola e roda):

```bash
# SSH config em ~/.ssh/config com Host wa-shard-1
ssh wa-shard-1 bash <<'EOF'
echo "=== Disk ==="
df -h /mnt/wa-data /
echo ""
echo "=== Memory ==="
free -m
echo ""
echo "=== Containers ==="
docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
echo ""
echo "=== WAHA sessions ==="
# Ajuste o WAHA_API_KEY hash abaixo
curl -s -H "X-Api-Key: $WAHA_API_KEY" http://localhost:3000/api/sessions | jq '[.[] | {name, status, engine: .config.metadata.engine}]'
echo ""
echo "=== Uptime ==="
uptime
EOF
```

Alertas que exigem ação:
- `df -h /mnt/wa-data` > 80% → purgar `waha-sessions` antigas OU aumentar volume
- `free -m` available < 500MB → restart WAHA OU escalar CX22 → CX42
- Session com `status: FAILED` → ver [recriar session](#recriar-session)
- Container `Restarting` há >5 min → `docker logs` + ver [07-debugging.md](./07-debugging.md)

---

## Recriar session

**Sintoma**: session em WAHA está `FAILED` ou `STOPPED`, usuário não consegue parear mais.

**Causa comum**: configuração do webhook apontando pra URL morta, ou `noweb`/engine wrong.

### Via nossa edge (preferido — usa deploy atual)

```bash
# Passo 1: delete a session do lado WAHA
SESSION=tomik-xxxxxxxxxxxx  # nome atual
WAHA_KEY=$(grep ^WAHA_API_KEY= ~/.secrets/wa-unofficial/shard-1.env | cut -d= -f2-)
curl -X DELETE "https://wa-shard-1.automatiklabs.com.br/api/sessions/$SESSION" \
  -H "X-Api-Key: $WAHA_KEY"

# Passo 2: reset channel row (mantém secrets)
psql "$SUPABASE_MASTER_DB_URL" <<SQL
UPDATE public.saas_messaging_channels
SET status='disconnected', is_active=false,
    unofficial_instance_name=NULL,
    unofficial_provider_host_url=NULL
WHERE id='<channel-uuid>';
SQL

# Passo 3: user clica "Conectar via QR Code" no frontend
# wa-start-session vai reprovisionar tudo
```

Resultado esperado: session com config atualizado (engine WEBJS, webhook na URL correta, HMAC secret re-gerado).

### Via script (manual direto)

Se quer controle total sem passar pelo edge:

```bash
WAHA_KEY=$(grep ^WAHA_API_KEY= ~/.secrets/wa-unofficial/shard-1.env | cut -d= -f2-)
SESSION=tomik-xxxxxxxxxxxx
WEBHOOK_URL="https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1/whatsapp-unofficial-inbound?channel_id=<uuid>"
HMAC_SECRET=$(openssl rand -hex 32)  # salvar isso criptografado no DB depois

# Delete old
curl -X DELETE "https://wa-shard-1.automatiklabs.com.br/api/sessions/$SESSION" \
  -H "X-Api-Key: $WAHA_KEY"

# Create new
curl -X POST "https://wa-shard-1.automatiklabs.com.br/api/sessions/" \
  -H "X-Api-Key: $WAHA_KEY" -H "content-type: application/json" \
  -d "{
    \"name\": \"$SESSION\",
    \"config\": {
      \"metadata\": {\"engine\": \"WEBJS\"},
      \"webhooks\": [{
        \"url\": \"$WEBHOOK_URL\",
        \"events\": [\"session.status\",\"message\",\"message.any\",\"message.reaction\",\"state.change\"],
        \"hmac\": {\"key\": \"$HMAC_SECRET\", \"algorithm\": \"sha512\"},
        \"retries\": {\"delaySeconds\": 2, \"attempts\": 5}
      }],
      \"noweb\": {\"store\": {\"enabled\": true, \"fullSync\": false}}
    }
  }"

# Start
curl -X POST "https://wa-shard-1.automatiklabs.com.br/api/sessions/$SESSION/start" \
  -H "X-Api-Key: $WAHA_KEY"

# Atualizar DB com novo HMAC secret:
psql "$SUPABASE_MASTER_DB_URL" <<SQL
UPDATE public.saas_messaging_channel_secrets
SET unofficial_webhook_token_encrypted = encrypt_key('$HMAC_SECRET')
WHERE channel_id = '<channel-uuid>';
SQL
```

---

## Desconectar / reconectar usuário

### Disconnect (logout só do lado provider, mantém canal no CRM)

Via frontend: botão trash em `WhatsAppUnofficialCard`.

Via edge curl:
```bash
curl -X POST "https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1/wa-disconnect" \
  -H "Authorization: Bearer $SUPABASE_SERVICE_ROLE_KEY" \
  -H "content-type: application/json" \
  -d '{"channel_id":"<uuid>"}'
```

Isso:
- Chama WAHA `POST /api/sessions/{name}/logout` (usuário precisa re-scanear próximo QR)
- `UPDATE saas_messaging_channels SET status='disconnected', is_active=false`
- Preserva `saas_messaging_channel_secrets` (api_key + HMAC não mudam)

### Reconnect

```bash
curl -X POST "https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1/wa-start-session" \
  -H "Authorization: Bearer $SUPABASE_SERVICE_ROLE_KEY" \
  -H "content-type: application/json" \
  -d '{"channel_id":"<uuid>"}'
```

Mesmo fluxo da primeira conexão, mas agora secrets já existem → só recria session WAHA.

---

## Atualizar versão WAHA

### Check version atual
```bash
ssh wa-shard-1 'docker inspect waha --format "{{.Config.Image}}"'
# Ex: devlikeapro/waha-plus:noweb-2026.4.2
```

### Update process

1. **Checar changelog** no Patreon do devlikeapro (https://patreon.com/devlikeapro) — upgrades podem quebrar.
2. **Staging first**: se tiver ambiente de staging com shard-staging, deployar lá primeiro.
3. **Backup antes do upgrade**:
   ```bash
   ssh wa-shard-1 /usr/local/bin/wa-backup.sh
   ```
4. **Pull nova image**:
   ```bash
   ssh wa-shard-1 "cd /data/coolify/services/waha-stack && docker compose pull waha"
   ```
5. **Recreate container** (via Coolify UI ou docker compose up -d):
   ```bash
   ssh wa-shard-1 "cd /data/coolify/services/waha-stack && docker compose up -d waha"
   ```
6. **Watch sessions**: algumas podem ir pra STARTING no restart → monitorar 5-10 min.
7. **Verificar response shape**: WAHA upgrades mudam payloads. Rodar test E2E:
   ```bash
   curl -s -H "X-Api-Key: $WAHA_API_KEY" https://wa-shard-1.automatiklabs.com.br/api/sessions | jq 'first'
   ```
   Comparar com fixture em `supabase/functions/_shared/__tests__/waha-events.json`.

### Rollback se algo quebrar

```bash
# Reverter via docker-compose.yml no Coolify: mudar tag da image pra versão anterior
# Ou:
ssh wa-shard-1 "docker pull devlikeapro/waha-plus:<versao-anterior> && docker tag devlikeapro/waha-plus:<versao-anterior> devlikeapro/waha-plus:latest && cd /data/coolify/services/waha-stack && docker compose up -d waha"
```

---

## Backup on-demand

```bash
ssh wa-shard-1 /usr/local/bin/wa-backup.sh
ls -la /mnt/wa-data/backups/$(date -u +%F)/
# Upload manual forçado (se script automático falhar):
ssh wa-shard-1 "b2 sync /mnt/wa-data/backups/ b2://tomikcrm-wa-backups/shard-1/"
```

---

## Restore from backup

Ver [runbooks/wa-unofficial-disaster-recovery.md](../../runbooks/wa-unofficial-disaster-recovery.md) (novo).

TL;DR:
```bash
# No shard (depois de VPS provisionada e docker-compose rodando):
DATE=2026-04-15
b2 sync "b2://tomikcrm-wa-backups/shard-1/$DATE" /tmp/restore/
docker compose stop waha
# Restaurar postgres
gunzip -c /tmp/restore/waha-pg.sql.gz | docker exec -i postgres psql -U waha -d waha
# Restaurar sessions files
tar -xzf /tmp/restore/waha-sessions.tgz -C /mnt/wa-data/
docker compose start waha
```

---

## Adicionar shard novo

Quando: >50 sessions no shard-1 OU distribuição geográfica (shard-2 em Europa pra clientes EU).

Resumo (ver [runbooks/wa-unofficial-shard-provisioning.md](../../runbooks/wa-unofficial-shard-provisioning.md) completo):

1. **Terraform**: copiar módulo `wa-shard.tf` com `shard_id=2`. `terraform apply`.
2. **DNS**: `wa-shard-2.automatiklabs.com.br` via Cloudflare API.
3. **Cloudflare Tunnel**: novo tunnel ou adicionar origin ao existente.
4. **Tailscale**: `tailscale up --auth-key=...` durante cloud-init.
5. **Coolify bootstrap**: dashboard port 8000, mesma config de shard-1 (docker-compose).
6. **Patreon login**: `docker login` no novo VPS com creds Patreon.
7. **Pull image + subir stack**: mesma versão do shard-1.
8. **Edge env**: adicionar `WA_SHARD_2_HOST_URL` + `WA_SHARD_2_API_KEY`.
9. **Sharding logic**: atualizar `wa-start-session` pra rotear novos channels pro shard-2 (round-robin ou por região).

---

## Migrar session entre shards

Não é trivial (sessions WhatsApp têm state local no Puppeteer profile). Opções:

### Option A — hard migration (re-scan QR)

- Desconectar no shard antigo (`wa-disconnect`)
- Reassignar `unofficial_provider_host_url` pro novo shard
- `wa-start-session` → usuário escaneia QR de novo no celular

**Custo**: cliente precisa re-pairing.

### Option B — soft migration (copy state)

```bash
# Assumindo session `tomik-abc123` em shard-1 indo pra shard-2:
ssh wa-shard-1 'docker exec waha tar -czf - /app/.sessions/tomik-abc123' \
  | ssh wa-shard-2 'docker exec -i waha tar -xzf - -C /'
# Restart WAHA em shard-2 pra picking up
ssh wa-shard-2 'docker restart waha'
# Update DB:
UPDATE saas_messaging_channels SET unofficial_provider_host_url='https://wa-shard-2.automatiklabs.com.br' WHERE id=...;
# Delete antiga pra evitar conflito:
curl -X DELETE "https://wa-shard-1.automatiklabs.com.br/api/sessions/tomik-abc123" -H "X-Api-Key:..."
```

**Risco**: tempos de clock drift + conflitos de state. Testar em canal de teste primeiro.

---

## Deploy de edges

Auto-deploy pelo CLI:
```bash
supabase functions deploy <edge-name> --project-ref qckjiolragbvvpqvfhrj --no-verify-jwt
```

Edges do MVP:
- `whatsapp-unofficial-inbound` (inbound)
- `wa-start-session`, `wa-get-qr`, `wa-disconnect` (lifecycle)
- `wa-send-text`, `wa-send-media`, `wa-send-reaction-reply` (outbound)
- `agent-dispatch-retry` (cron retry)

Paralelizar:
```bash
for edge in whatsapp-unofficial-inbound wa-start-session wa-get-qr wa-disconnect wa-send-text wa-send-media wa-send-reaction-reply agent-dispatch-retry; do
  supabase functions deploy $edge --project-ref qckjiolragbvvpqvfhrj --no-verify-jwt &
done; wait
```

---

## Rollback de edges

Não tem "undeploy" específico. Opções:

1. **Git revert + deploy again**:
   ```bash
   git revert <commit-que-quebrou>
   git push
   # Merge pra main → Vercel/Render auto-deploy pra frontend/runtime
   # Edges: redeploy manual
   supabase functions deploy <edge> --project-ref qckjiolragbvvpqvfhrj --no-verify-jwt
   ```

2. **Deploy versão anterior checkout temporário**:
   ```bash
   git checkout <sha-boa>
   supabase functions deploy <edge> --project-ref qckjiolragbvvpqvfhrj --no-verify-jwt
   git checkout develop
   ```

Pra produção, sempre via git (trackable + PR).

---

## Cleanup DB

### Ghost contacts (contato com LID como phone, mas real phone existe em outro contato)

```sql
-- Client DB
-- Encontra duplicatas onde um tem LID e outro tem real phone
WITH lid_contacts AS (
  SELECT id, phone_e164, wa_id, split_part(wa_id, '@', 1) AS lid_digits
  FROM public.messaging_contacts
  WHERE wa_id LIKE '%@lid'
    AND replace(phone_e164, '+', '') = split_part(wa_id, '@', 1)
),
real_phone_contacts AS (
  SELECT id, phone_e164
  FROM public.messaging_contacts
  WHERE phone_e164 NOT IN (SELECT phone_e164 FROM lid_contacts)
)
SELECT lc.id AS lid_contact_id, rc.id AS real_contact_id, lc.phone_e164 AS lid_phone, rc.phone_e164 AS real_phone
FROM lid_contacts lc
-- Sem mapping automático — cross-reference manual via WAHA chats overview
;
```

Pra fazer merge real (cuidadoso):

1. Identificar qual o real phone via WAHA `/api/sessions/{name}/chats/overview` (olhar `_data.key.remoteJidAlt`).
2. Atualizar contato:
   ```sql
   UPDATE public.messaging_contacts
   SET phone_e164 = '<real-phone>', display_name = '<nome-do-contato>'
   WHERE id = '<lid-contact-id>';
   ```
3. Se existir contato separado com real phone: migrar conversations + messages + deletar ghost.

### Outbox dead_letter antigas (cleanup pós-investigação)

```sql
-- Client DB
DELETE FROM public.agent_events_outbox
WHERE status = 'dead_letter'
  AND created_at < now() - interval '30 days';
```

### Messages órfãs (conversation_id aponta pra convo deletada)

```sql
-- Client DB
DELETE FROM public.messaging_messages m
WHERE NOT EXISTS (
  SELECT 1 FROM public.messaging_conversations c WHERE c.id = m.conversation_id
);
```

---

## Emergência: kill switch

Se precisar PARAR todo envio de mensagens (incident):

### Option A — desativar todas sessions
```bash
WAHA_KEY=...
for session in $(curl -s -H "X-Api-Key: $WAHA_KEY" http://wa-shard-1.automatiklabs.com.br/api/sessions | jq -r '.[].name'); do
  curl -X POST "https://wa-shard-1.automatiklabs.com.br/api/sessions/$session/stop" -H "X-Api-Key: $WAHA_KEY"
done
```

### Option B — desativar drainer (para runtime não enviar)

No Render dashboard → agent-runtime service → Environment → `START_WORKER=false` → restart.

Ou via git: PR + merge + auto-redeploy.

### Option C — desabilitar via feature flag

SQL pra toda a org:
```sql
UPDATE public.saas_organizations
SET beta_features = false
WHERE id = '<org-uuid>';
```

`beta_features=true` é o gate atual do unofficial_qr no frontend. Flipar desativa a UI de criar novas instâncias (não pausa as ativas).

Pra pausar as ativas:
```sql
UPDATE public.saas_messaging_channels
SET is_active = false, status = 'disconnected'
WHERE saas_organization_id = '<org-uuid>' AND provider_variant = 'unofficial_qr';
```

---

## Próximos

- [07-debugging.md](./07-debugging.md) — playbook de diagnóstico completo
- [08-referencias-externas.md](./08-referencias-externas.md) — quando buscar ajuda externa
