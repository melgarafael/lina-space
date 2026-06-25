# 04 — Infra: Hetzner + Coolify + WAHA Plus

> Onde o WAHA Plus vive, como ele foi provisionado e como administrar.

---

## Topologia

```
   ┌─────────────────────────── Internet ──────────────────────────┐
   │                                                                 │
   │  Cliente (Cloudflare Tunnel HTTPS)                             │
   │  wa-shard-1.automatiklabs.com.br                                │
   │                           │                                     │
   │                           ▼                                     │
   │  Cloudflare Zero Trust (edge) ──── Tunnel ───┐                 │
   │                                              ▼                 │
   │                                     ┌────────────────────┐     │
   │                                     │  Hetzner VPS CX22  │     │
   │                                     │  SA (São Paulo)     │     │
   │                                     │  Ubuntu 24.04 LTS   │     │
   │                                     │  IPv4 178.104.229.216     │
   │                                     │  Tailscale 100.104.223.26 │
   │                                     │                    │     │
   │                                     │  ┌──────────────┐  │     │
   │                                     │  │ Coolify v4   │  │     │
   │                                     │  │ (orchestra)  │  │     │
   │                                     │  └──────┬───────┘  │     │
   │                                     │         ▼          │     │
   │                                     │  ┌──────────────┐  │     │
   │                                     │  │ WAHA Plus    │  │     │
   │                                     │  │ Postgres     │  │     │
   │                                     │  │ Redis        │  │     │
   │                                     │  │ Caddy (TLS)  │  │     │
   │                                     │  └──────────────┘  │     │
   │                                     │                    │     │
   │                                     │  /mnt/wa-data      │     │
   │                                     │   (20GB volume)    │     │
   │                                     │  /mnt/wa-data/     │     │
   │                                     │   ├ postgres/      │     │
   │                                     │   ├ redis/         │     │
   │                                     │   ├ waha-sessions/ │     │
   │                                     │   ├ caddy/         │     │
   │                                     │   └ backups/       │     │
   │                                     └────────────────────┘     │
   │                                              │                  │
   │                                       Backblaze B2 bucket       │
   │                                       (backups diários)         │
   └─────────────────────────────────────────────────────────────────┘
```

---

## Informações-chave do shard-1

| Item | Valor |
|------|-------|
| **Dominio** | `wa-shard-1.automatiklabs.com.br` |
| **Hetzner server name** | `wa-shard-1` |
| **IPv4 público** | `178.104.229.216` |
| **IPv6** | habilitado |
| **Tailscale tailnet IP** | `100.104.223.26` |
| **Tailscale MagicDNS** | `wa-shard-1.tail0424fd.ts.net` |
| **Região** | Hetzner SA (São Paulo) |
| **Plano** | CX22 — 2 vCPU AMD, 4 GB RAM, 40 GB SSD |
| **Volume adicional** | 20 GB ext4 em `/mnt/wa-data` |
| **OS** | Ubuntu 24.04 LTS |
| **Coolify version** | v4.x |
| **WAHA image** | `devlikeapro/waha-plus` (private Docker registry) |
| **Session engine** | **WEBJS** (pinned em código — NOWEB falhou em testes) |

---

## Cloudflare Tunnel

- Zone: `automatiklabs.com.br`
- Record: `wa-shard-1` (CNAME → tunnel)
- Ruta ao origin: VPS `:443` (Caddy dentro do Coolify)
- Proxied: sim (TLS termination em Cloudflare, reencrypta pro origin)
- Zero Trust Tunnel ID: gerenciado via `cloudflared` no VPS (config em `/etc/cloudflared/` se rodar nativo, ou via Coolify app se rodar como container)

Pra rotacionar o tunnel ou mudar origin, ver Cloudflare Zero Trust dashboard: `https://one.dash.cloudflare.com/` → Access → Tunnels.

---

## Terraform

Arquivos:
- `infra/terraform/wa-shard.tf` — resource `hcloud_server` + volume + firewall + SSH key
- `infra/terraform/cloud-init.yaml` — bootstrap do OS: SSH hardening, UFW, fail2ban, Docker, Coolify install, Tailscale join, volume mount
- `infra/terraform/variables.tf` — `shard_id`, `image`, `server_type`, `tailscale_auth_key`, `volume_size_gb`, `domain_base`

Proteções críticas aplicadas:
```hcl
resource "hcloud_server" "wa_shard" {
  delete_protection  = true    # API Hetzner recusa delete
  rebuild_protection = true    # API Hetzner recusa rebuild (perderia sessions)

  lifecycle {
    prevent_destroy = true     # Terraform `destroy` bloqueado
    ignore_changes = [
      user_data,    # cloud-init só roda na primeira boot — mudanças não triggeram replace
      ssh_keys,     # Hetzner só aplica ssh_keys no provisioning inicial
    ]
  }
}

resource "hcloud_volume" "wa_data" {
  delete_protection = true
  lifecycle { prevent_destroy = true }
}
```

**Rebootar** é seguro. **Recriar** = perder sessions (todos usuários teriam que re-scanear QR).

Pra mudar config ou adicionar novo shard, ver [runbooks/wa-unofficial-shard-provisioning.md](../../runbooks/wa-unofficial-shard-provisioning.md) (novo) ou [provisioning.md](../../runbooks/wa-unofficial-provisioning.md) (original).

---

## Coolify

Coolify é o orquestrador Docker auto-hospedado que gerencia containers via UI.

- **Dashboard**: `https://wa-shard-1.automatiklabs.com.br:8000/` (HTTP basic auth — login `admin`, senha em `~/.secrets/wa-unofficial/shard-1.env` `WAHA_DASHBOARD_PASSWORD`)
- **Root password Coolify** (pra SSH-less operations): também em shard-1.env (`coolify_root` random 32 chars)

Aplicações configuradas:

1. **waha-plus** — container principal. Config docker-compose:

```yaml
# infra/wa-unofficial/docker-compose.yml (template, deploy via Coolify)
services:
  waha:
    image: devlikeapro/waha-plus:noweb-2026.4.2
    restart: always
    environment:
      WAHA_API_KEY: ${WAHA_API_KEY}  # SHA512 hash — ver 05-credenciais
      WAHA_LOG_LEVEL: info
      WAHA_PRINT_QR: "false"
      WAHA_LOG_FORMAT: json
      WAHA_ENGINE: WEBJS
      WHATSAPP_SWAGGER_USERNAME: admin
      WHATSAPP_SWAGGER_PASSWORD: ${WHATSAPP_SWAGGER_PASSWORD}
      WAHA_DASHBOARD_USERNAME: admin
      WAHA_DASHBOARD_PASSWORD: ${WAHA_DASHBOARD_PASSWORD}
      WHATSAPP_HOOK_HMAC_KEY: ${WHATSAPP_HOOK_HMAC_KEY}
      POSTGRES_HOST: postgres
      POSTGRES_USER: waha
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: waha
      REDIS_URL: redis://:${REDIS_PASSWORD}@redis:6379/0
    depends_on:
      - postgres
      - redis
    volumes:
      - /mnt/wa-data/waha-sessions:/app/.sessions
    expose:
      - "3000"

  postgres:
    image: postgres:15-alpine
    restart: always
    environment:
      POSTGRES_USER: waha
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_DB: waha
    volumes:
      - /mnt/wa-data/postgres:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    restart: always
    command: redis-server --requirepass ${REDIS_PASSWORD}
    volumes:
      - /mnt/wa-data/redis:/data

  caddy:
    image: caddy:2-alpine
    restart: always
    ports:
      - "443:443"
      - "80:80"
    volumes:
      - /mnt/wa-data/caddy:/data
      - ./Caddyfile:/etc/caddy/Caddyfile
    depends_on:
      - waha
```

2. **Caddy** — reverse proxy + TLS (termination para `:80` e `:443`). `Caddyfile`:
```
wa-shard-1.automatiklabs.com.br {
    reverse_proxy waha:3000
    log {
        output file /data/access.log
        format json
    }
}
```

3. **Cron backups** (systemd timer ou cron dentro do container) — gera dump PG + snapshot volumes pra B2 todos dias 03:00 UTC.

---

## Volume layout

`/mnt/wa-data/` (20GB) subdividido pela `wa-data-mount.service` criada no cloud-init:

| Path | Conteúdo | Pra que serve |
|------|----------|---------------|
| `postgres/` | Dados do Postgres (sessions) | WAHA persiste state de session aqui |
| `redis/` | Redis snapshot (AOF) | Cache de state transiente |
| `waha-sessions/` | `.sessions/` do WAHA | WhatsApp session files (profile photo, creds Puppeteer) |
| `caddy/` | Certs Let's Encrypt + logs access | Reverse proxy |
| `backups/` | Dumps SQL + tarballs | Backup diário + subida pro B2 |

**Growth rate observado**: ~10MB/mês por sessão WhatsApp (cache de mídia decrescente depois de limpeza). Monitora via:
```bash
ssh wa-shard-1 "df -h /mnt/wa-data"
```

---

## Backups

### Estratégia dupla

1. **On-disk snapshot** (`/mnt/wa-data/backups/`): diário, últimos 7 dias.
2. **Off-site em Backblaze B2**: bucket `tomikcrm-wa-backups`, replicação diária com retenção 30 dias.

### Script de backup (exemplo em cron diário)

```bash
#!/bin/bash
# /usr/local/bin/wa-backup.sh — rodado via cron @ 03:00 UTC
set -euo pipefail

TS=$(date -u +%F)
BACKUP_DIR="/mnt/wa-data/backups/$TS"
mkdir -p "$BACKUP_DIR"

# 1. PG dump
docker exec postgres pg_dump -U waha waha | gzip > "$BACKUP_DIR/waha-pg.sql.gz"

# 2. Sessions snapshot
tar -czf "$BACKUP_DIR/waha-sessions.tgz" -C /mnt/wa-data waha-sessions

# 3. Upload B2
/usr/local/bin/b2 sync "$BACKUP_DIR" "b2://tomikcrm-wa-backups/shard-1/$TS"

# 4. Cleanup local > 7 days
find /mnt/wa-data/backups/ -type d -mtime +7 -exec rm -rf {} +
```

### Restore

1. Stop Coolify compose: `coolify app stop waha-stack`
2. Restaurar volumes:
   ```
   tar -xzf waha-sessions.tgz -C /mnt/wa-data/
   gunzip -c waha-pg.sql.gz | docker exec -i postgres psql -U waha -d waha
   ```
3. Restart: `coolify app start waha-stack`

Ver também [runbooks/wa-unofficial-disaster-recovery.md](../../runbooks/wa-unofficial-disaster-recovery.md).

---

## Admin mesh: Tailscale

Por que: IPv4 fallback é instável (ISP residencial, NAT), e a ideia é acesso seguro sem abrir porta 22 pro mundo.

Setup:
- Tailscale daemon rodando no VPS (instalado via cloud-init)
- Auth key reusable no tailnet `tail0424fd.ts.net`
- Hostname tailnet: `wa-shard-1`
- ACL tag `tag:wa-shard` (para futuros shards)

SSH via Tailscale (de qualquer máquina membro do tailnet):
```bash
ssh root@wa-shard-1                     # usa MagicDNS
ssh root@100.104.223.26                 # IP direto
ssh root@wa-shard-1.tail0424fd.ts.net   # FQDN
```

SSH config em `~/.ssh/config` no dev Rafael:
```
Host wa-shard-1
  HostName 100.104.223.26
  User root
  IdentityFile ~/.ssh/tomikcrm_wa_ed25519
  ServerAliveInterval 60

Host wa-shard-1-public
  HostName 178.104.229.216
  User root
  IdentityFile ~/.ssh/tomikcrm_wa_ed25519
```

Backup SSH key: `~/.ssh/tomikcrm_wa_backup_ed25519` (disaster recovery se a primária corromper).

Ver [runbooks/wa-unofficial-tailscale-setup.md](../../runbooks/wa-unofficial-tailscale-setup.md).

---

## Firewall

### Hetzner Cloud Firewall (nuvem — fora do VPS)

Aplicado via Terraform. Permite:
- `22/tcp` SOMENTE de IPs em `var.admin_ips` (lista de /32 dos devs)
- `80/tcp`, `443/tcp` de qualquer IP (`0.0.0.0/0`)
- ICMP (ping/traceroute)

### UFW dentro do VPS

Bloqueia tudo por default. Permite SSH + `Coolify-Web` (80, 443). Coolify bootstrap (`:8000`) fica bloqueado — acesso só via tunnel.

Ver `infra/terraform/cloud-init.yaml` para setup exato.

---

## Métricas e monitoring

Temos hoje (mínimo):
- WAHA `/api/sessions` expõe status (WORKING/FAILED)
- Coolify dashboard mostra container health (restarts, logs)
- Cloudflare analytics (requests/erros)

**Falta** (melhoria futura):
- Prometheus + Grafana
- Alertas via PagerDuty/Telegram quando session cai
- Dashboard com: # sessions ativas, msg/hora por session, erro rate

Ver [09-melhorias-futuras.md](./09-melhorias-futuras.md).

---

## Planilhas operacionais essenciais

Pra checkin saudável (pelo menos 1x por semana):

```bash
# 1. Session count e status
ssh wa-shard-1 'curl -s -H "X-Api-Key: $WAHA_API_KEY" http://localhost/api/sessions | jq "[.[] | {name, status}]"'

# 2. Disk usage no volume
ssh wa-shard-1 "df -h /mnt/wa-data"

# 3. Container health
ssh wa-shard-1 "docker ps --format 'table {{.Names}}\\t{{.Status}}'"

# 4. WAHA container logs (últimos 200)
ssh wa-shard-1 "docker logs --tail 200 waha"

# 5. Backblaze B2 backup últimos 3 dias
b2 ls b2://tomikcrm-wa-backups/shard-1/ | tail -3
```

## Escalabilidade

### Capacidade observada
- Um shard CX22 aguenta **~50 sessions ativas** antes de degradar (RAM Chrome headless é o gargalo no WEBJS).
- **Sessions** (no status WORKING) consomem ~60-80MB de RAM cada.

### Quando escalar
- `free -m` no VPS mostra <500MB livre, OU
- `docker stats waha` mostra >80% CPU sustained, OU
- Users começam a reportar atrasos de inbound delivery

### Opções
1. **Vertical**: migrar pra CX42 (4 vCPU, 16 GB) → suporta ~150 sessions
2. **Horizontal**: provisionar shard-2, shard-3... — ver [runbooks/wa-unofficial-shard-provisioning.md](../../runbooks/wa-unofficial-shard-provisioning.md)

Balanceamento entre shards ainda é manual (nova session vai no shard com `env.WA_SHARD_1_HOST_URL` default). Pra automatizar round-robin, é um item do roadmap.

---

## Próximos

- [05-credenciais.md](./05-credenciais.md) — mapa completo de secrets
- [06-manutencao.md](./06-manutencao.md) — playbook de operação
