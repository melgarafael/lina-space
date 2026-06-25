# 08 — Referências externas

> Onde buscar info que não tá no nosso código.

---

## WAHA Plus — fontes de verdade

### Documentação oficial

- **WAHA docs principal**: https://waha.devlike.pro/ — começar aqui sempre. Concept + reference + engines + events.
- **API reference (Swagger)**: https://wa-shard-1.automatiklabs.com.br/docs (requer login `admin` / `WHATSAPP_SWAGGER_PASSWORD`) — playground real da nossa instância.
- **WAHA OpenAPI spec bruta**: https://waha.devlike.pro/docs/openapi/ — contrato completo.

### Changelog & upgrades

- **WAHA Releases**: https://github.com/devlikeapro/waha/releases — GitHub release notes. Ler antes de upgradar image.
- **Patreon posts**: https://www.patreon.com/devlikeapro/posts — anúncios, breaking changes, features novas pro tier Plus.
- **WAHA Plus release tags** (Docker Hub): https://hub.docker.com/r/devlikeapro/waha-plus/tags — lista tags disponíveis.

### Suporte e comunidade

- **Devlikeapro Discord**: https://discord.gg/waha (ou convite via Patreon) — canal #help, #plus, #bugs. Autor muito ativo.
- **GitHub Issues (WAHA OSS)**: https://github.com/devlikeapro/waha/issues — reports públicos. Plus issues vão por DM/Discord.
- **Stack Overflow tag**: https://stackoverflow.com/questions/tagged/waha — baixo volume, mas alguns threads úteis.

### Engines — entender diferenças

WAHA suporta 3 engines. Cada uma tem quirks:

| Engine | Tech | Prós | Contras | Quando usar |
|--------|------|------|---------|-------------|
| **WEBJS** | whatsapp-web.js + Puppeteer | Maduro, estável, events completos | Heavy RAM (~80MB/session) | **Default nosso** — estabilidade > economia |
| NOWEB | @whiskeysockets/baileys | Leve (~20MB/session), fast start | Menos events, bugs frequentes em Plus | Evitar em prod — tivemos session FAILED em lote |
| GOWS | Go whatsmeow | Leve, rápido | Beta, suporte limitado | Experimental only |

Ver docs: https://waha.devlike.pro/docs/how-to/engines/

### Fluxos críticos documentados

- **LID addressing**: https://waha.devlike.pro/docs/how-to/lid-chat/ — como funciona o novo sistema de identificadores.
- **Events & webhooks**: https://waha.devlike.pro/docs/how-to/events/ — o que chega em cada event type.
- **HMAC validation**: https://waha.devlike.pro/docs/how-to/webhooks/#hmac — como validar assinatura. Algorithm suportado: SHA256, SHA512.
- **Session lifecycle**: https://waha.devlike.pro/docs/how-to/sessions/#session-lifecycle — STOPPED → STARTING → SCAN_QR_CODE → WORKING → FAILED.
- **Media storage**: https://waha.devlike.pro/docs/how-to/media/ — como mídia fica armazenada na session, expirações.

---

## WhatsApp — políticas do provedor

**Importante**: estamos usando API não oficial. Risco de banimento sempre existe.

### Official policies relevantes (que WAHA contorna, mas risk > 0)

- **WhatsApp Business Policy**: https://www.whatsapp.com/legal/business-policy/
- **WhatsApp Commerce Policy**: https://www.whatsapp.com/legal/commerce-policy/
- **WhatsApp Terms of Service**: https://www.whatsapp.com/legal/business-tos/ — especificamente "Unauthorized use" section.

### Mitigações conhecidas

- **Warm-up**: número novo não pode disparar alto volume no dia 1. Ramp-up 1→5→10→25 msgs/dia primeira semana.
- **Rate limiting** local: não passar de 30 msgs/minuto nem 500/dia mesmo em conta aquecida (conservador).
- **Conteúdo**: evitar URLs encurtadas (bit.ly etc), spam signals (COMPRAR AGORA, frases all-caps repetidas).
- **Opt-out**: sempre ter `STOP` / `SAIR` funcional. Registrar opt-outs no CRM.
- **Avoid mass campaigns**: disparos em massa são o trigger #1 de ban.

---

## Hetzner Cloud

- **Docs Hetzner Cloud**: https://docs.hetzner.cloud/
- **Terraform provider**: https://registry.terraform.io/providers/hetznercloud/hcloud/latest/docs
- **Status page**: https://status.hetzner.com/ — outages SA região
- **Discord**: https://discord.gg/hetzner — sysadmins

### Relevante pra operação

- Server types / pricing: https://www.hetzner.com/cloud#pricing
- IPv6 network: https://docs.hetzner.com/cloud/networks/ipv6/
- Firewall rules: https://docs.hetzner.com/cloud/firewalls/

---

## Coolify

- **Docs**: https://coolify.io/docs/ — nota: docs crescendo rápido, às vezes ficam defasados
- **GitHub**: https://github.com/coollabsio/coolify — issues ativas
- **Discord**: https://discord.gg/coolify

### Quando buscar

- Compose apps stuck em "starting" / "failed deploy"
- Como adicionar domain + SSL via UI
- Like rollback de app para versão anterior

---

## Cloudflare

- **Docs Zero Trust Tunnels**: https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/
- **Docs DNS**: https://developers.cloudflare.com/dns/
- **API reference**: https://developers.cloudflare.com/api/
- **Community**: https://community.cloudflare.com/

### Quando buscar

- Tunnel disconnected / config issue
- DNS propagation troubleshoot
- Rate limit tuning (Zone → Rules → Rate limit)

---

## Tailscale

- **Docs**: https://tailscale.com/kb/
- **ACL reference**: https://tailscale.com/kb/1018/acls
- **Self-hosted coordination**: https://tailscale.com/kb/1242/tailscale-ssh (pra entender SSH via Tailscale)

---

## Supabase

- **Edge Functions docs**: https://supabase.com/docs/guides/functions — Deno runtime, deploy, env vars
- **Postgres docs** (pgcrypto): https://www.postgresql.org/docs/15/pgcrypto.html
- **pg_cron (scheduled functions)**: https://supabase.com/docs/guides/database/extensions/pg_cron
- **RLS guide**: https://supabase.com/docs/guides/auth/row-level-security
- **CLI reference**: https://supabase.com/docs/reference/cli/introduction

### Issues conhecidas

- CLI 2.39 (nossa versão) não suporta `functions logs` — usar dashboard. Upgrade quebra pipeline então pausamos.
- Secrets não aceitam prefixo `SUPABASE_*` (reservado). Use `MASTER_SUPABASE_*` ou outro prefixo custom.

---

## Backblaze B2

- **Docs B2 CLI**: https://www.backblaze.com/b2/docs/quick_command_line.html
- **Application keys management**: https://www.backblaze.com/b2/docs/application_keys.html
- **Pricing** (pra custo-check): https://www.backblaze.com/b2/cloud-storage-pricing.html

---

## Docker / docker-compose

- **docker-compose V2 reference**: https://docs.docker.com/compose/compose-file/
- **Registry auth (Patreon)**: `docker login -u <user> -p <token>` → stored em `~/.docker/config.json`

---

## Postgres — funções e extensões que usamos

- **pgp_sym_encrypt / pgp_sym_decrypt** (via pgcrypto): https://www.postgresql.org/docs/15/pgcrypto.html#PGCRYPTO-PGP-SYM-FUNCS
- **Supabase vault** (alternative pra secrets, não usamos): https://supabase.com/docs/guides/database/vault
- **gen_random_bytes**: https://www.postgresql.org/docs/15/functions-admin.html#FUNCTIONS-ADMIN-GENRANDOM

---

## Node.js + Deno (edges)

- **Deno reference**: https://deno.land/manual
- **esm.sh** (como importamos `@supabase/supabase-js` em edges): https://esm.sh/
- **Web Crypto API** (usamos no HMAC + randomHex): https://developer.mozilla.org/en-US/docs/Web/API/Web_Crypto_API

---

## Fontes internas (equipe Tomik)

- **Repo TomikCRM**: https://github.com/melgarafael/tomikcrm
- **ADRs**: `docs/architecture/ADR-*.md`
- **Runbooks**: `docs/runbooks/*`
- **Memórias do Claude Code**: `~/.claude/projects/-Users-rafaelmelgaco-Downloads-tomikcrm/memory/`
  - `feedback_waha_apikey_sha512_hash.md` — WAHA key é SHA512 hash
  - `feedback_supabase_encryption_key_per_env.md` — encrypt_key depende de app.encryption_key
  - `feedback_mcp_project_ref_verification.md` — MCP Supabase pode mentir project_ref
  - outros em `MEMORY.md`

---

## Canais de comunicação

### Incidents críticos (production down)

1. Rafael (founder) — WhatsApp pessoal (+55 31 98966-6398)
2. Discord devlikeapro (se issue for específico do WAHA Plus)
3. Hetzner status page (se VPS Hetzner afetado)
4. Cloudflare status

### Dúvidas em tempo real (build time)

- Discord Coolify
- Discord Supabase
- Stack Overflow

### Reports + roadmap

- GitHub Issues do repo próprio
- Linear (se/quando migrar tracking)

---

## Anti-padrões que a gente já pagou

Aprendizados documentados (links pra memória):

- **Não assumir API shape sem verificar**: WAHA Plus response shape muda entre versões (e engines). Test fixtures em `supabase/functions/_shared/__tests__/waha-events.json` refletem o que viu em prod, mas podem ficar stale em upgrade.
- **Not normalizing IDs**: muitos bugs vieram de `provider_message_id` com formato inconsistente. Sempre normaliza na entrada via `normalizeWahaProviderMessageId`.
- **Not caching channel variant**: outra dos drainers faz lookup Master DB por row — cache in-process economiza muito round-trip (`variantCache` em ambos drainers).
- **Hard-coding shard URL**: começamos hard-coded `wa-shard-1.automatiklabs.com.br`. Agora usamos env `WA_SHARD_1_HOST_URL` com default. Pra multi-shard no futuro vamos ter `WA_SHARD_N_*` pra cada.

---

## Próximos

- [09-melhorias-futuras.md](./09-melhorias-futuras.md) — roadmap
- [README.md](./README.md) — voltar ao índice
