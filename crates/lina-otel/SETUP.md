# lina-otel — setup do collector OTel local (F1-1-4)

> **Enhancement, NÃO pré-requisito.** A governança e a observabilidade básicas do Lina
> funcionam **sem** OTel (o JSONL da F1-1-2 cobre tokens/custo/atividade). Ligar o OTel
> só **corrige o subconto de tokens** do JSONL e dá custo **medido** quando o CLI o emite.
> (13.10 item 7 refutou "OTel é pré-requisito de governança".)

## Invariantes (não-negociáveis aqui)

- **Local-first / opt-in (#2):** desligado por default. O receiver faz bind **só em
  `127.0.0.1:4318`** — **nada sai da máquina**, nenhuma porta escuta fora de loopback.
- **Redaction de PII obrigatória quando ligado** (13.5 item 3): e-mails, caminhos de home
  e tokens são mascarados antes de qualquer retenção/log.
- **Neutralidade (#3):** o que se injeta no CLI vem do `CliProfile` (`capabilities.otel`).

## Como ligar (opt-in)

1. Em Ajustes, ative "Observabilidade avançada (OTel)" — `OtelConfig{enabled:true}`.
2. O Lina sobe o receiver loopback e passa a spawnar os CLIs com as env de telemetria
   (`otel_env`), **apenas** para CLIs cujo profile declara `capabilities.otel = true`.
3. Sem ativar (ou para um CLI sem OTel): nada muda — o JSONL segue sozinho.

## Cobertura honesta por CLI (é DESIGUAL — 13.5 item 11)

| CLI | OTel? | Tokens | Custo | Observação |
|---|---|---|---|---|
| **Claude Code** | ✅ (`capabilities.otel=true`) | ✅ medido (`claude_code.token.usage`) | ✅ **medido** (`claude_code.cost.usage`) | 1ª classe; fonte preferida sobre JSONL |
| **Gemini / Antigravity** | parcial | ✅ tokens | ⚠️ **derivado** (tokens × pricing) — `cost_estimated` segue `true` | custo não-medido |
| **Codex** | ⚠️ gaps | ⚠️ parcial | ❌ | `codex exec` não emite métricas (13.5 achado 5) |
| **OpenCode / shell** | ❌ | — | — | só JSONL/atividade; OTel não aplica |

Quando o OTel só traz **tokens** (sem `cost.usage`), o custo permanece **estimativa**
(`source=otel`, `cost_estimated=true`): a fonte melhora os tokens, mas o custo continua
derivado — o dashboard exibe "~".

## Perda no encerramento abrupto

O risco do batch de 5s (13.5 achado 1) é mitigado em duas pontas: **(a)** intervalo de
export reduzido no CLI (`OTEL_METRIC_EXPORT_INTERVAL=5000`, vs default 60s); **(b)**
**persist-on-receipt** no receiver — cada export é gravado na hora, sem janela de batch
nossa. Dados anteriores ao último export já estão persistidos quando o terminal morre.

## Sanity check JSONL × OTel

Quando ambas as fontes existem, o `merge_and_check` calcula a divergência de custo
(`|otel − jsonl| / otel`); acima de **20%** loga um `warn` (13.5 item 1) — visível, nunca
fatal. O custo do OTel (quando medido) prevalece.

## O que NÃO entra (escopo)

Dashboard externo (Grafana/Jaeger) **não** é dependência; telemetria **nunca** liga por
default; **zero** envio para fora da máquina.
