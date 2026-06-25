# 09 — Melhorias futuras & Roadmap

> Features solicitadas + débitos técnicos + direções de escalabilidade. Priorizadas em ondas.

---

## Priorização

**Onda 1 (0–30 dias)** — débitos técnicos que fecham o MVP:
- [W1.1 — Cron do retry worker](#w11--cron-do-retry-worker)
- [W1.2 — Monitoring básico](#w12--monitoring-básico)
- [W1.3 — Shard-2 provisioning automation](#w13--shard-2-provisioning-automation)
- [W1.4 — Suppress agent em primeiro contato](#w14--handoff-automático-em-primeiro-contato)

**Onda 2 (30–60 dias)** — features que user listou:
- [W2.1 — Envio em grupos + filtro IA](#w21--envio-em-grupos--filtro-ia-para-responder)
- [W2.2 — Cron de disparos em grupos](#w22--disparo-em-grupos-cron)
- [W2.3 — Sequência por reação em grupos](#w23--sequência-por-reação-em-grupos)
- [W2.4 — Sequência Webhook com API não-oficial](#w24--sequência-webhook-com-api-não-oficial)
- [W2.5 — Handoff unificado para canais não-oficiais](#w25--handoff-unificado-para-canais-não-oficiais)

**Onda 3 (60+ dias)** — escalabilidade + robustez:
- [W3.1 — Observability stack (Prometheus + Grafana)](#w31--observability-stack)
- [W3.2 — Multi-region shards](#w32--multi-region-shards)
- [W3.3 — Auto-scaling shards (horizontal)](#w33--auto-scaling-shards)
- [W3.4 — Sandbox test shard pra devs](#w34--sandbox-test-shard)
- [W3.5 — Substituir WAHA Plus por fork próprio](#w35--opcional--fork-próprio-do-waha)

---

## Onda 1 — Débitos técnicos

### W1.1 — Cron do retry worker

**Estado**: edge `agent-dispatch-retry` deployed mas **não tem cron acionando**. Queue enche mas não drena.

**Runbook tem passo-a-passo**: [agent-dispatch-retry-cron.md](../../runbooks/agent-dispatch-retry-cron.md) — opção A pg_cron, opção B GitHub Actions.

**Esforço**: 15 min (só rodar o SQL pg_cron no qck).

**Acceptance**:
- Row em `saas_agent_dispatch_retry_queue` com status=pending é processada em <2min
- Dashboard Supabase → Database → pg_cron jobs mostra job ativo

### W1.2 — Monitoring básico

**Estado**: hoje diagnóstico é manual via queries SQL + docker logs.

**Mínimo desejado**:
- **Alertas Telegram/Discord**: session FAILED, drainer stuck >5min, dead_letter row nova, shard disk >85%.
- **Dashboard simples** (Metabase? Grafana?): 
  - Sessions ativas por shard
  - Inbound/outbound rate (msgs/hora)
  - Outbox delivery latency (p50, p99)
  - Error rate em wa-send-*

**Esforço**: 1-2 dias. Pode começar simples com Supabase `pg_notify` + Deno Cron function que pings Telegram bot.

**Próximo passo**: escrever spec + escolher stack (Prometheus em container proprio vs Grafana Cloud free tier).

### W1.3 — Shard-2 provisioning automation

**Estado**: shard-1 feito manualmente com `terraform apply` + cloud-init. Adicionar shard-2 é replicar manualmente.

**Melhorar**:
- Parametrizar `shard_id` em `infra/terraform/wa-shard.tf` (já tá — variable)
- Script `./scripts/provision-shard.sh <shard-id> <region>` que:
  - `terraform workspace new shard-<id>`
  - Aplica com valores certos
  - Adiciona DNS CF API
  - Registra `WA_SHARD_<id>_*` nos edge secrets
  - Rodar health check no fim

**Esforço**: 4-6 horas.

**Runbook** [wa-unofficial-shard-provisioning.md](../../runbooks/wa-unofficial-shard-provisioning.md) — escrito mas ainda manual.

### W1.4 — Handoff automático em primeiro contato

**Estado**: agente pode responder automaticamente a ANY inbound, inclusive contatos completamente novos/desconhecidos. Risco de:
- Spam accidental (ex: contato errado que escreveu por engano)
- Responder em momento sensível sem curadoria

**Proposal**:
- Quando `messaging_conversations.created_at > now() - 1 hour` (conversa nova) + nenhum outbound prévio → automaticamente set `handoff_mode='human'`
- Operador humano revisa + marca `agent` pra liberar auto-reply
- Configurável por org: `saas_organizations.auto_reply_new_contacts = false`

**Esforço**: 1 dia (código + migration + UI toggle).

**Por que Onda 1**: incidente do Athila Mendes levantou isso. Controle > latency.

---

## Onda 2 — Features do user

### W2.1 — Envio em grupos + filtro IA para responder

**Contexto user**: "Envio em grupos, filtro na IA para responder grupos ou não"

**Pedaço 1: Envio em grupos**

WAHA suporta groups (`@g.us` JIDs). Nosso stack hoje só lida com individuais.

Mudanças necessárias:
- **DB**: `messaging_conversations.is_group BOOLEAN DEFAULT false` + `group_jid TEXT`
- **Inbound**: detectar `from @g.us` em `waha-normalizer.isGroupJid`, set `is_group=true`. Extrair também `participant` (quem dentro do grupo mandou).
- **Contato**: a concept de "contato" em grupo é ambíguo — temos 3 modelos:
  1. Um contato por grupo (participantes são apenas metadata)
  2. Contatos individuais + grupo como container
  3. Híbrido: criar contact pro participant (tem phone) + conversa parent = grupo
- **Frontend**: ChatLive renderiza grupo diferente (avatar genérico, nome do grupo, mostrar sender em cada bubble).
- **Outbound**: enviar pra `group_jid` já funciona via `toChatId` pass-through em `@`-containing input.

**Pedaço 2: Filtro IA "responder grupo: sim/não"**

Setting por canal ou por org:
- `saas_messaging_channels.respond_in_groups = false` (default)
- No inbound handler: se `is_group && !channel.respond_in_groups` → skip dispatch runtime (ainda persiste a msg pra visibilidade).

Adicional: flag per-pipeline `trigger.include_groups = false` default.

**Esforço**: 3-5 dias.

**Riscos**: WhatsApp ban signal aumenta em grupos — monitorar rate.

### W2.2 — Disparo em grupos (cron)

**Contexto user**: "Disparo em grupos, cron de disparos em grupos"

Feature: agente/operador agenda disparo massa pra membros de um grupo (ou pro grupo inteiro).

Componentes:
- **Definição de campanha**: `saas_group_campaigns` (novo) — `{id, org, channel, group_jid, template_text, schedule_cron, status}`
- **Scheduler**: pg_cron job que lê `saas_group_campaigns WHERE next_run_at <= now()`
- **Execução**: enqueue em `agent_events_outbox` um row por destinatário com respect ao rate limit da org.
- **Rate limit**: queue com `delay` pra espaçar envios (1 msg a cada 3-5 seg).

Ligação com W2.1 — depende de grupos estarem mapped.

**Esforço**: 5-7 dias (UI + backend + edge).

**Riscos**:
- Ban risk ALTO em disparo massa. Incluir throttling defensive.
- Compliance: opt-in prévia (GDPR/LGPD) é obrigatório. Enforce check antes de enqueue.

### W2.3 — Sequência por reação em grupos

**Contexto user**: "Ativar sequencia com base em reação em grupos"

Feature: quando participant reage com 👍 (ou emoji específico), trigger sequência de follow-up.

Componentes:
- **Inbound** já recebe `message.reaction` events.
- **Novo**: `saas_reaction_triggers` table — `{emoji, channel_id, pipeline_id, requires_group bool}`
- **Handler**: em `message.reaction`, matchear vs triggers ativos. Se matched, dispatch pipeline com contexto do participant.
- **Pipeline**: novo trigger type `reaction` em `agent_pipeline_versions.definition.trigger.type`.

**Esforço**: 3-4 dias.

### W2.4 — Sequência Webhook com API não-oficial

**Contexto user**: "Sequência no Webhook com API não oficial também"

Hoje temos sistema de sequences (follow-ups) que dispara via Meta. Queremos extender pro unofficial.

Dois pedaços:

**Pedaço 1**: Confirmar que sequences atuais rodam em canais unofficial.
- `agent-followup-scheduler` edge dispara via outbox → drainer V1 → `WHATSAPP_SEND_ENDPOINT` (Meta). Precisa passar pelo `resolveChannelVariant` (já tá após PR #221).
- Validar em prod com 1 sequence unofficial.

**Pedaço 2**: Webhook incoming trigger sequences via unofficial também.
- Gateway recebe webhook de sistema externo → cria saída via edge → respectando `provider_variant`.
- Já funciona em tese — só precisa testar + documentar endpoint.

**Esforço**: 2-3 dias (validação + testes + fix gaps).

### W2.5 — Handoff unificado para canais não-oficiais

**Contexto user**: "Sistema de handoff para canais de api não oficial/disparo/estratégias com follow-up usando api não oficial"

Hoje handoff:
- `messaging_conversations.handoff_mode` = `'agent'` / `'human'` / `'hybrid'`
- Dispatch gate (`shouldDispatchMessageEvents`) checka isso antes de triggerar agente
- **Funciona igual em Meta e unofficial** (já unificado via `dispatch-gate.ts` shared)

Mas user quer:
- Botão "assumir manualmente" em ChatLive que flipa `handoff_mode='human'` + notifica atendente
- Botão "devolver ao agente" que flipa de volta pra `'agent'`
- Dashboard de conversas em `human` (atendente vê fila)
- Timeout: se ninguém respondeu em 30 min em handoff, retorna pra agente

**Esforço**: 5-7 dias.

**Nota**: maior parte tá no frontend + UX. Backend shared é sólido.

---

## Onda 3 — Escalabilidade

### W3.1 — Observability stack

Prometheus + Grafana + Loki. Ou equivalente SaaS (Grafana Cloud, Datadog).

Métricas chave:
- `waha_sessions_active_total{shard, status}`
- `outbox_rows_total{status}`
- `outbox_drain_duration_seconds{variant}`
- `inbound_handler_duration_seconds`
- `wa_send_duration_seconds{type, status}`

Alertas:
- session FAILED × qualquer shard
- outbox dead_letter growth rate
- edge 5xx rate >5%
- shard disk >85%

**Esforço**: 1 semana (setup + dashboards iniciais).

### W3.2 — Multi-region shards

Pra clientes fora do Brasil (USA, EU), latência via SA shard é alta. 

Adicionar:
- `shard-2-us` em Hetzner FR (região Europa ou Ashburn)
- Sharding logic no `wa-start-session` escolhe shard por `saas_organizations.region` ou IP do cliente.

**Esforço**: 3-5 dias (assumindo W1.3 automation feito).

### W3.3 — Auto-scaling shards

Hoje shard é 1:1 com VPS manual. Pra escalar:
- Quando shard passa de 80% capacity (ex: >40 sessions), auto-provision shard-N
- Distribui novas sessions round-robin
- Sessions existentes NÃO migram (custaria re-pairing) — só novas.

**Esforço**: 2-3 semanas. Dependências de W1.3 + W3.1.

### W3.4 — Sandbox test shard

Shard dedicado pra dev: `wa-shard-staging.automatiklabs.com.br`.

- Isolado de prod
- Pode ser derrubado/recriado sem afetar users
- Tests automatizados (Playwright E2E, WAHA probes) apontam pra ele

**Esforço**: 1-2 dias.

### W3.5 — (Opcional) Fork próprio do WAHA

**Contexto**: WAHA Plus é closed-source. Se devlikeapro sumir ou bugar num release crítico, ficamos presos.

Opção A: Subscribe tier Pro ($99/mês) → acesso ao source code. Mantemos um fork privado.

Opção B: Migrar pra engine open-source (whatsapp-web.js direto, Baileys, Venom). Requer re-implementar manager de sessions etc.

**Esforço**: 3-4 semanas se for pela opção B.

**Quando considerar**: se WAHA Plus tiver 2+ incidents críticos em 6 meses, ou se escalar pra 500+ sessions e custo/features Pro não fizer sentido.

---

## Débitos técnicos prioritários

Além das features acima, tem débitos técnicos identificados:

### Edge function ffmpeg transcoder

**Estado**: deferred (#18 no backlog original).

Quando user envia voice note pela WhatsApp via WEBJS engine, vem em OGG/Opus. Frontend às vezes precisa MP3 (iOS Safari tem issues com OGG). Edge `messaging-media-transcode` seria um middleware.

**Esforço**: 2-3 dias.

### Migrar Meta webhook pra shared dispatcher

**Estado**: feito (PR #19 Meta agora usa `runtime-bridge.ts`).

### Retry pattern pra outbound falhado

**Estado**: feito (`saas_agent_dispatch_retry_queue` + edge `agent-dispatch-retry`).

### N+1 realtime subscription

**Estado**: feito (PR #53 — ChatLive hoistou pro parent).

### Outbox drainer unificado

Hoje V1 e V3 drainers são duplicados. Gerenciando os 2 é desnecessário.

**Proposta**: deprecar V1, só rodar V3. Requer migrar pipelines restantes de V1 pra V3 (maioria já está).

**Esforço**: 1 semana + 1 semana de soak.

### Tests E2E cobrindo unofficial

Hoje `supabase/functions/_shared/__tests__/*` tem fixtures WAHA mas tests unitários. Falta E2E que exercita:
- wa-start-session + wa-get-qr round trip (mock WAHA)
- Inbound fromMe echo dedup
- Outbound via drainer

**Esforço**: 2-3 dias (Playwright + mock WAHA ou shard staging).

---

## Como priorizar

Sugestão de ordem concreta pra próxima sprint:

1. **W1.1 cron retry worker** (15 min) — produção já deveria ter.
2. **W1.4 handoff auto em primeiro contato** (1 dia) — mitigação de risco da Athila incident.
3. **W1.2 monitoring básico** (1-2 dias) — unblock W1.3 e W3.
4. **W2.5 handoff unificado UI** (1 semana) — user quer isso.
5. **W2.1 grupos** (1 semana) — mais alto demanda depois.

Depois rediscutir prioridades com user.

---

## Como propor mudanças novas

Quando o time quiser adicionar feature ou deixar gap documentado, seguir:

1. **Criar ADR novo**: `docs/architecture/ADR-XXX-<nome>.md` — decisão + alternativas consideradas.
2. **Atualizar este guia** (09-melhorias-futuras.md): adicionar seção com esforço + risk.
3. **PR pra approval**: até devs senior OKarem a direção antes de codar.
4. **Criar issue Linear** com link pro ADR.

Não criar features sem ADR pra coisas que mudam arquitetura (ex: novo provider, novo storage, mudança de schema).

---

## Voltar

- [README.md](./README.md) — índice
