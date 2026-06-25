# 05 — Credenciais

> Mapa completo de secrets: o que, onde, como rotacionar.

---

## TL;DR

| Secret | Usado por | Onde fica persistido | Como rotacionar |
|--------|-----------|----------------------|-----------------|
| `WAHA_API_KEY` (SHA512 hash) | Todas chamadas REST pra WAHA | Edge env (qck `WA_SHARD_1_API_KEY`) + `.env` local + shard-1.env | Dashboard WAHA → regenerate. Atualizar edge + channel secrets. |
| `WHATSAPP_HOOK_HMAC_KEY` (per-session) | Validar inbound webhook | `saas_messaging_channel_secrets.unofficial_webhook_token_encrypted` | Regerar 32-byte random + encrypt + update. Reiniciar session. |
| `WAHA_DASHBOARD_PASSWORD` | UI admin do WAHA | VPS env + 1Password | Regenerate via docker exec + restart container |
| `WHATSAPP_SWAGGER_PASSWORD` | Swagger docs `/docs` | VPS env | idem |
| `POSTGRES_PASSWORD` (WAHA) | Postgres interno | VPS env | Rotate + restart postgres |
| `REDIS_PASSWORD` | Redis interno | VPS env | idem |
| Patreon `devlikeapro` (Docker registry) | Pull `waha-plus` image | Coolify registry config + `docker login` | Login via Patreon OAuth novamente |
| `HCLOUD_TOKEN` | Terraform + `hcloud` CLI | Dev `.env` | Dashboard Hetzner → API tokens (90d rotate) |
| `CLOUDFLARE_API_TOKEN` | DNS + Tunnel | Dev `.env` | CF dashboard → regen token com mesmo scope |
| `TAILSCALE_AUTH_KEY` | Join tailnet | Dev `.env` + cloud-init | Tailscale admin → novo key reusable |
| Backblaze B2 | Backup sync | VPS env + dev `.env` | B2 → App keys → novo |
| Supabase service role (qck) | Edges + Agent Runtime + scripts | `.env` `SUPABASE_SERVICE_ROLE_KEY` | Supabase dashboard → regen. Atualizar TODOS consumidores. |

---

## 1. WAHA API Key — SHA512 hash (CRÍTICO)

### O que é

WAHA Plus espera o header `X-Api-Key` com **hash SHA512 hex** do valor raw. Plaintext retorna 401.

```bash
# Gerar hash a partir do plaintext:
echo -n "mypassword" | sha512sum | awk '{print $1}'
# → hash de 128 chars hex
```

### Onde vive

1. **Valor plaintext** (usado apenas na geração do hash): local em `~/.secrets/wa-unofficial/shard-1.env` (`WAHA_API_KEY_PLAIN`)
2. **Valor hash** (esse é o que todo mundo usa): 
   - VPS env (docker-compose): `WAHA_API_KEY`
   - Dev `.env`: `WAHA_API_KEY`
   - Edge env Supabase qck: `WA_SHARD_1_API_KEY`
   - Encriptado em `saas_messaging_channel_secrets.unofficial_api_key_encrypted` (pra uso per-channel via factory)

### Como rotacionar

1. Gerar novo plaintext (password manager):
   ```bash
   openssl rand -base64 32
   ```
2. Calcular hash:
   ```bash
   HASH=$(echo -n "$NEW_PLAIN" | sha512sum | awk '{print $1}')
   ```
3. Atualizar VPS:
   ```bash
   ssh wa-shard-1
   # No Coolify UI: app waha-stack → env vars → WAHA_API_KEY = $HASH → redeploy
   ```
4. Atualizar edge Supabase:
   ```bash
   supabase secrets set --project-ref qckjiolragbvvpqvfhrj WA_SHARD_1_API_KEY="$HASH"
   ```
5. Atualizar `saas_messaging_channel_secrets` de TODOS canais do shard-1:
   ```sql
   UPDATE public.saas_messaging_channel_secrets s
   SET unofficial_api_key_encrypted = encrypt_key('<HASH>')
   FROM public.saas_messaging_channels c
   WHERE s.channel_id = c.id
     AND c.unofficial_provider_host_url = 'https://wa-shard-1.automatiklabs.com.br';
   ```
6. Atualizar dev `.env` local + 1Password Vault
7. **Testar**: `curl -H "X-Api-Key: $HASH" https://wa-shard-1.automatiklabs.com.br/api/sessions` → deve retornar lista

⚠ Rotacionar esse secret quebra TODAS as sessions no shard até o update propagar. Fazer em janela de baixo tráfego.

---

## 2. HMAC webhook secret — per-channel

### O que é

Secret compartilhado entre WAHA e nosso edge pra validar que o webhook veio da WAHA.

Algorithm: **HMAC-SHA512** do raw body com esse secret como key. Header: `x-webhook-hmac`.

### Como é provisionado

No `wa-start-session` edge, ao criar uma session nova:

```ts
webhookToken = randomHex(32)  // 64 chars hex (32 bytes)
// encripta e persiste:
encrypt_key(webhookToken) → saas_messaging_channel_secrets.unofficial_webhook_token_encrypted
// passa pro WAHA startSession config:
config.webhooks[0].hmac = { key: webhookToken, algorithm: 'sha512' }
```

Cada canal tem SEU próprio secret. Compromisso de uma session não compromete outras.

### Como rotacionar

**Cenário A — Rotate de rotina (annual/compromise)**:
1. Gerar novo token + encriptar:
   ```sql
   -- Master DB
   UPDATE public.saas_messaging_channel_secrets
   SET unofficial_webhook_token_encrypted = encrypt_key(encode(gen_random_bytes(32), 'hex'))
   WHERE channel_id = '<channel_uuid>'
   RETURNING decrypt_key(unofficial_webhook_token_encrypted) AS new_token;
   ```
2. Reiniciar session pro WAHA pegar o novo secret (aceitar o delete + create via `wa-start-session`):
   - Via edge: `POST /api/sessions/{name}/stop` + `/start` com nova config (pode fazer direto ou matar a session pra recriar)
   - Mais simples: DELETE session na WAHA + next clique em "Conectar" recria

**Cenário B — Compromise detectado** (webhook validation falhando/abuse):
- Além do passo acima: purgar Redis do WAHA (state) + restartar WAHA (pra invalidar caches). Ver [06-manutencao.md](./06-manutencao.md).

### Verificar validação

Em `supabase/functions/whatsapp-unofficial-inbound/index.ts`:
```ts
const valid = await verifyHmacSha512(rawBody, signatureHeader, webhookToken)
if (!valid) return 401
```

`verifyHmacSha512` (em `waha-normalizer.ts`) usa timing-safe compare.

---

## 3. Credenciais Coolify / Dashboards

### Coolify UI
- URL: `https://wa-shard-1.automatiklabs.com.br:8000/`
- Admin: `admin` / senha em `~/.secrets/wa-unofficial/shard-1.env` (`coolify_root` via Terraform random_password)

### WAHA Dashboard
- URL: `https://wa-shard-1.automatiklabs.com.br/dashboard`
- Admin: `admin` / senha em `WAHA_DASHBOARD_PASSWORD`
- Pra que: UI visual de sessions (start/stop, ver QR, logs)

### WAHA Swagger
- URL: `https://wa-shard-1.automatiklabs.com.br/docs`
- Admin: `admin` / senha em `WHATSAPP_SWAGGER_PASSWORD`
- Pra que: API playground + tentar chamadas ad-hoc

---

## 4. Patreon devlikeapro (Docker image)

### O que é

WAHA Plus é comercial. Pull da image privada `devlikeapro/waha-plus:*` requer login no Docker Hub com credenciais vindas do Patreon.

### Como obter

1. Pagar tier "Core" ($19/mês): https://patreon.com/devlikeapro
2. Mensagem do Patreon manda link + Docker login creds
3. `docker login -u devlikeapro -p <patreon-access-token>` no VPS (via Coolify registry config)

### Tiers
- **Core** — $19/mo — multi-session unlimited, webhook HMAC, 3 engines (NOWEB, WEBJS, GOWS). Atual.
- **Pro** — $99/mo — source code access, telemetry off. Considerado se bus-factor WAHA vira crítico.

### Renovação
Patreon cobra mensal. Se cartão falha → pull da image falha → se precisar recriar session/container, fica travado.

Verificar status: `https://www.patreon.com/manageRewards?user_id=devlikeapro` (se tiver access).

---

## 5. Credenciais Hetzner / Cloudflare / Tailscale / B2

Todas ficam em `~/.env` do dev Rafael + `~/.secrets/wa-unofficial/shard-1.env` + idealmente 1Password.

| Var | Onde usa | Rotate |
|-----|----------|--------|
| `HCLOUD_TOKEN` | Terraform, `hcloud` CLI, Render API | 90d — Hetzner dashboard → API tokens |
| `CLOUDFLARE_API_TOKEN` | DNS + Tunnel | 90d — CF dashboard → "My Profile" → API Tokens |
| `CLOUDFLARE_ZONE_ID` | DNS ops | Fixo (zona não muda) |
| `CLOUDFLARE_ACCOUNT_ID` | Tunnels | Fixo |
| `TAILSCALE_AUTH_KEY` | `tailscale up` em novos hosts | 90d — Tailscale admin → Keys → Generate |
| `B2_ACCESS_KEY_ID` + `B2_SECRET_ACCESS_KEY` | Backup script | B2 → App Keys → novo (revogar antigo após migração) |
| SSH key `~/.ssh/tomikcrm_wa_ed25519` | Admin SSH | Re-gerar + atualizar `authorized_keys` no VPS + backup em `.ssh/tomikcrm_wa_backup_ed25519` |

---

## 6. Credenciais Supabase (edge env)

Edges da stack unofficial usam:

| Env var | Valor | Set-se via |
|---------|-------|-----------|
| `SUPABASE_URL` | `https://qckjiolragbvvpqvfhrj.supabase.co` | Automático (project env) |
| `SUPABASE_SERVICE_ROLE_KEY` | JWT service role | Automático |
| `SUPABASE_ANON_KEY` | JWT anon | Automático |
| `MASTER_SUPABASE_URL` | Duplicata do URL (canônica) | `supabase secrets set MASTER_SUPABASE_URL=...` |
| `MASTER_SUPABASE_SERVICE_ROLE_KEY` | Duplicata da key | `supabase secrets set ...` |
| `WA_SHARD_1_HOST_URL` | `https://wa-shard-1.automatiklabs.com.br` | idem |
| `WA_SHARD_1_API_KEY` | SHA512 hash | idem |
| `WA_WEBHOOKS_BASE_URL` | `https://qckjiolragbvvpqvfhrj.supabase.co/functions/v1` | idem (override quando canônico difere) |
| `AGENT_RUNTIME_URL`, `AGENT_RUNTIME_V3_URL`, `AGENT_RUNTIME_TOKEN` | Render endpoints + shared bearer | idem |
| `SUPABASE_FUNCTIONS_BASE_URL` | **NÃO usar** — prefixo `SUPABASE_` é reservado | — |

Listar tudo:
```bash
supabase secrets list --project-ref qckjiolragbvvpqvfhrj
```

### Proibições
- Prefixo `SUPABASE_*` é reservado pra Supabase CLI. Secrets seu usem outros nomes (`MASTER_SUPABASE_*`, `WA_*`, etc).

---

## 7. Credenciais do Agent Runtime (Render)

Configurado via `render.yaml` + Render dashboard.

Variáveis críticas (check `apps/agent-runtime/src/config.ts`):

| Var | Uso |
|-----|-----|
| `SUPABASE_URL` + `SUPABASE_SERVICE_ROLE_KEY` | Master DB + chamadas de edges unofficial |
| `TOMIK_HOSTED_SUPABASE_URL` + `TOMIK_HOSTED_SUPABASE_SERVICE_ROLE_KEY` | Client DB pra orgs tomik_hosted |
| `REDIS_URL` | BullMQ queue |
| `OPENAI_API_KEY` | Pipelines |
| `WHATSAPP_SEND_ENDPOINT` | URL do `whatsapp-meta-send` edge (**só Meta path**) |
| `WHATSAPP_INTERNAL_SEND_TOKEN` | Bearer pra edge Meta |
| `WHATSAPP_SEND_API_KEY` | `x-api-key` header pra edge Meta |

**Novos unofficial**: drainer usa `SUPABASE_SERVICE_ROLE_KEY` direto, sem novos envs.

---

## 8. Credenciais do Tenant Gateway (local dev)

Gateway BFF em `apps/tenant-gateway/`. Rodar local requer:

```
# apps/tenant-gateway/.env (se presente, senão herda de apps raiz)
MASTER_SUPABASE_URL=...
MASTER_SUPABASE_SERVICE_ROLE_KEY=...
TOMIK_HOSTED_SUPABASE_URL=...
TOMIK_HOSTED_SUPABASE_SERVICE_ROLE_KEY=...
KMS_KEY_ID=...   # AWS KMS pra encrypt/decrypt de creds BYO
CORS_ORIGIN=http://localhost:5173
PORT=4000
```

Ver `apps/tenant-gateway/src/config.ts` pra lista completa.

---

## 9. Onde NÃO commitar

❌ **NUNCA** commitar em git:
- `WAHA_API_KEY_PLAIN` (valor raw)
- `WAHA_API_KEY` (mesmo sendo hash — atacante com acesso ao hash autentica)
- `SUPABASE_SERVICE_ROLE_KEY`
- `HCLOUD_TOKEN`, `CLOUDFLARE_API_TOKEN`
- `B2_SECRET_ACCESS_KEY`
- Senhas dashboard (Coolify, WAHA)
- SSH private keys (`tomikcrm_wa_ed25519`)
- Qualquer coisa dentro de `~/.secrets/`

✅ Pode commitar:
- `~/.secrets/wa-unofficial/README.md` (estrutura, sem valores)
- Nomes de env vars referenciados no código (sem valores)
- `.env.example` com placeholders

### Gitignore essentials

`.gitignore` deve ter:
```
.env
.env.local
.env.*.local
!.env.example
*.pem
*.key
id_ed25519*
tomikcrm_wa_*
.secrets/
```

---

## 10. Plano de rotação (recomendado)

| Secret | Frequência | Responsável |
|--------|-----------|-------------|
| WAHA_API_KEY | Anual OU se vazar | Dev senior |
| Webhook HMAC (per-channel) | Anual OU se vazar | Dev (DB UPDATE manual) |
| Patreon | N/A (account-scoped) | Rafael (renovação) |
| Hetzner token | 90 dias | Rafael |
| Cloudflare token | 90 dias | Rafael |
| Tailscale auth key | 90 dias | Rafael |
| B2 access key | 180 dias | Rafael |
| Supabase service role | Se vazar (senão fixo) | Dev senior |
| Coolify / WAHA dashboard passwords | Anual | Rafael |

Criar um tracking issue no Linear pra rotação trimestral. Vale mais a pena automatizar (secrets rotator Lambda ou GH Action) quando o time crescer.

---

## Próximos

- [06-manutencao.md](./06-manutencao.md) — operações diárias
- [08-referencias-externas.md](./08-referencias-externas.md) — onde buscar info nova do WAHA
