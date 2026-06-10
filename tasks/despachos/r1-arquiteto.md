# DESPACHO r1-webhooks-adrs — Arquiteto
**id:** `f1-6-1-2` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Três entregáveis nesta fatia: 2 stories de código no crate isolado `lina-webhooks` + 2 ADRs draft.

## Fronteira de arquivos (sua, dono único nesta rodada)
- `crates/lina-webhooks/**` (crate inteiro é seu nesta rodada)
- `docs/adr/0027-pf1-trust-por-espaco.md` (NOVO, draft)
- `docs/adr/0028-live-region-caminho.md` (NOVO, draft)
- **NÃO toque:** `lina-core` (events.rs tem outro dono nesta rodada — se o replay precisar de helper no core, REGISTRE o pedido na entrega), `app/`, `lina-bootstrap`.

## Story A — F1-6-1 · Replay de `WebhookConfigured` no boot (MEDIA-5)
**Fonte:** `tasks/epico-f1/ondas-5-6.md` linhas 160-166. No boot do engine, replay dos `WebhookConfigured` do event store → rebind de rota/targetRef. Secret continua no keyring (o evento NÃO pode carregar segredo — se hoje carregar, é achado: corrija aqui e registre). Hook removido por evento posterior não renasce.
**Critérios:** (a) teste de integração: configura hook → derruba/re-sobe o engine → curl HMAC válido = **202** sem reconfigurar (hoje 404); (b) HMAC inválido pós-replay = 401 sem publicar (não-vacuoso); (c) scan: nenhum secret em claro no log (teste grep no artefato); (d) docstring que super-prometia agora bate com o código.

## Story B — F1-6-2 · `spawn_blocking` no append (MEDIA-2)
**Fonte:** `ondas-5-6.md` linhas 168-174. Mover o `store.append` para `tokio::task::spawn_blocking` PRESERVANDO a semântica síncrona-pré-202 de `7c04172` (handler AGUARDA o append; falha→5xx, nada publicado — só a thread muda).
**Critérios:** (a) append com latência injetada ~2s → 202 só APÓS o append E request a OUTRO hook responde normal durante a espera (worker não presa); (b) falha de disco → 5xx e nenhum evento publicado (regressão-guard); (c) suíte `lina-webhooks` verde.
Sequencie A antes de B (mesmo arquivo).

## ADR draft 1 — 0027 PF-1: allow-list e fronteira de confiança por Espaço (F1-4-2)
**Fonte:** `ondas-2-4.md` linhas 183-199. Draft AGORA (a F1-4-1 está sendo construída em paralelo pelo Especialista em Dados — alinhe contra o despacho dele em `tasks/despachos/r1-dados.md`); o selo final vem com a F1-4-1 fechada. Decida: (a) trust por Espaço herdado ao reabrir, NUNCA derivado de campo de agente; (b) allow-list de ações sensíveis POR Espaço evoluindo W4-3 sem reabrir ADR 0006; (c) cross-Espaço: deny-all por default, exceção futura = opt-in explícito auditável; (d) migração do `WorkspaceTrust` single→multi; (e) seção de red-team em LINGUAGEM DE INVARIANTES (impersonação de Espaço; evento forjado no log de B referenciando A). Referencie ADR 0006/0007.

## ADR draft 2 — 0028 Live-region: patch no gpui pinado × custom Element × upstream (F1-2-7)
**Fonte:** `ondas-2-4.md` linhas 138-153. As 3 opções com custos (patch no pin = re-aplicar a cada bump + encarece porta gpui↔Slint do doc 33; custom Element = implementar request_layout/prepaint/paint; upstream = critério objetivo de re-avaliação). O SPIKE está com o Especialista em Telas nesta mesma rodada (despacho `r1-telas.md`) — escreva o ADR apontando o spike como evidência pendente e deixe a decisão PROVISÓRIA (recomende custom Element salvo evidência contrária do spike). Critério de reversão explícito. Honestidade: nenhuma copy afirma "conforme ARIA live-region" até o spike passar.

## Entrega
`tasks/epico-f1/.entrega-f1-6-1-2.md`. Marcador: `.iniciado-f1-6-1-2`.
