# WhatsApp Não-Oficial (WAHA Plus) — Guia do Dev

> **Contexto em 1 minuto:** O TomikCRM suporta dois backends de WhatsApp. O **Meta Cloud** (oficial, pago por mensagem, com regras de janela 24h + template) continua sendo o caminho principal. Para clientes que precisam de conexão por QR Code em números pessoais/comerciais — sem os limites e custos da Meta — introduzimos a stack **"Não-Oficial"** baseada em **WAHA Plus** rodando em Hetzner Cloud. Toda a plataforma (CRM, ChatLive, Agent Runtime, follow-ups) funciona nos dois canais de forma transparente via uma camada de abstração.

Esse guia é pra devs que vão **tocar a implementação daqui em diante**: arquitetura, infra, credenciais, manutenção, debugging, fontes de informação WAHA e roadmap.

---

## Índice

### Arquitetura & referência
1. **[01-arquitetura.md](./01-arquitetura.md)** — Visão end-to-end. Diagrama de componentes (edges, drainer, WAHA, factory). Fluxo inbound + outbound.
2. **[02-fluxo-mensagens.md](./02-fluxo-mensagens.md)** — Deep-dive no ciclo de mensagem. 3 bridges LID↔phone. Deduplicação. Normalização de IDs.
3. **[03-schema-dados.md](./03-schema-dados.md)** — Tabelas Master (`saas_messaging_channels`, `saas_messaging_channel_secrets`, `saas_agent_dispatch_retry_queue`) + Client (`messaging_contacts`, `messaging_conversations`, `messaging_messages`, `agent_events_outbox`).

### Infra & credenciais
4. **[04-infra-hetzner-coolify.md](./04-infra-hetzner-coolify.md)** — VPS shard-1 em Hetzner, Terraform, cloud-init, Coolify, docker-compose WAHA Plus, Tailscale admin mesh.
5. **[05-credenciais.md](./05-credenciais.md)** — Mapa completo de secrets: WAHA_API_KEY (SHA512), HMAC webhook, Patreon devlikeapro, B2 backups. Onde ficam e como rotacionar.

### Operação
6. **[06-manutencao.md](./06-manutencao.md)** — Playbook diário: recriar session, disconnect/reconnect, upgrade WAHA, backup+restore, provisionar shard-2.
7. **[07-debugging.md](./07-debugging.md)** — Triage: logs Supabase, WAHA API, queries SQL prontas, retry queue, outbox. Incidentes conhecidos.
8. **[08-referencias-externas.md](./08-referencias-externas.md)** — Docs oficiais WAHA, fóruns devlikeapro, Patreon, GitHub issues, Discord.

### Roadmap
9. **[09-melhorias-futuras.md](./09-melhorias-futuras.md)** — Backlog priorizado: grupos (envio, filtro IA, cron, reação), sequência Webhook unofficial, handoff unified.

### Runbooks operacionais
- **[wa-unofficial-disaster-recovery.md](../../runbooks/wa-unofficial-disaster-recovery.md)** — Recuperação de desastre: shard perdido, sessão corrompida, deploy errado.
- **[wa-unofficial-shard-provisioning.md](../../runbooks/wa-unofficial-shard-provisioning.md)** — Adicionar shard-N novo.
- **[wa-unofficial-session-troubleshooting.md](../../runbooks/wa-unofficial-session-troubleshooting.md)** — Session FAILED / STOPPED / SCAN_QR_CODE travado.
- Runbooks pré-existentes: [provisioning](../../runbooks/wa-unofficial-provisioning.md), [tailscale-setup](../../runbooks/wa-unofficial-tailscale-setup.md), [beta-rollout](../../runbooks/wa-unofficial-beta-rollout.md), [migration-verification](../../runbooks/wa-unofficial-migration-verification.md).

### Contexto histórico (ADRs — decisões arquiteturais)

Esses três documentos explicam **por que** escolhemos WAHA Plus, por que Hetzner, por que o schema é como é. Leitura obrigatória antes de mudanças arquiteturais grandes.

- [ADR-009 — Provider não-oficial](../../architecture/ADR-009-unofficial-wa-provider.md) — avaliação Evolution vs WAHA vs WPPConnect vs Baileys, escolha final WAHA Plus
- [ADR-010 — Hosting](../../architecture/ADR-010-unofficial-wa-hosting.md) — Hetzner + Coolify vs Railway vs Render vs Fly.io
- [ADR-011 — Schema](../../architecture/ADR-011-unofficial-wa-schema.md) — colunas adicionais em `saas_messaging_channels` + secrets, variant `unofficial_qr`

---

## TL;DR pra dev que acabou de entrar

1. **Stack**: React frontend → Supabase edge functions → WAHA Plus container no Hetzner shard-1 → WhatsApp
2. **Um shard = uma VPS Hetzner = um processo WAHA com N sessions** (hoje shard-1 roda em `wa-shard-1.automatiklabs.com.br`)
3. **Cada organização** cria 1+ canais `unofficial_qr` em `saas_messaging_channels`. Cada canal vira 1 session WAHA com nome `tomik-<channel-id-prefix>`.
4. **Credenciais da session** (api_key + webhook HMAC secret) são persistidas encriptadas em `saas_messaging_channel_secrets.unofficial_*_encrypted` via `pgp_sym_encrypt`. Descripta com RPC `decrypt_key` usando `app.encryption_key`.
5. **Inbound**: WAHA → webhook HMAC SHA512 → edge `whatsapp-unofficial-inbound` → normaliza + persiste msg → dispara Agent Runtime.
6. **Outbound**: CRM/Agent → edge `wa-send-text` / `wa-send-media` / `wa-send-reaction-reply` → WAHA → WhatsApp. Drainer V1/V3 lê `agent_events_outbox` e roteia por `channel.provider_variant`.
7. **LID mode**: contatos WhatsApp em LID têm identificador paralelo `<digits>@lid` ≠ telefone real. Três bridges tentam resolver phone real: `remoteJidAlt` → `senderPn` → sufixo do id `_<phone>@c.us`. `wa_id` na tabela contacts guarda o JID (@lid) pro outbound rotear corretamente.
8. **Deploy**: edges via `supabase functions deploy --no-verify-jwt`. Runtime via Render auto-deploy em push pra `main`. Frontend via Vercel auto-deploy em push pra `main`.
9. **Emergência**: SSH via Tailscale `ssh wa-shard-1`, WAHA REST direto em `https://wa-shard-1.automatiklabs.com.br/api/...` (X-Api-Key header), dashboard Coolify em `https://wa-shard-1.automatiklabs.com.br:8000/`.

## Convenções

- Toda conversa técnica sobre esse produto usa **"unofficial"** em código e **"não-oficial"** em UI em português.
- Variant value no DB: `'unofficial_qr'` (literal, com underscore).
- Todo canal `unofficial_qr` tem `provider='whatsapp'` no mesmo registro.
- Edge functions unofficial começam com prefixo `wa-` (sem `whatsapp-`). Exceção: `whatsapp-unofficial-inbound` (inbound também tem prefixo Meta-like por consistência com os outros webhooks inbound).

## Contatos

- **Infra**: dono Rafael (rafamelgaco123@gmail.com). Acesso Hetzner + Cloudflare + Backblaze em `~/.secrets/wa-unofficial/`.
- **WAHA Plus**: Patreon `devlikeapro` (conta `devlikeapro`), tier "Core" ($19/mês). Docker image `devlikeapro/waha-plus` (privada).
- **Shard-1 admin**: `root@wa-shard-1` via Tailscale ou IPv4 fallback `178.104.229.216`.
