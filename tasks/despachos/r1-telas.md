# DESPACHO r1-spike-a11y — Especialista em Telas
**id:** `spike-a11y` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

## Tarefa: SPIKE da F1-2-7 · live-region via custom Element (`set_live`) — time-boxed
**Fonte:** `tasks/epico-f1/ondas-2-4.md` linhas 138-153. Fato estabelecido pela pesquisa 13.15: o AccessKit 0.24 pinado JÁ expõe `set_live(Live::Polite/Assertive)`; o gpui do nosso SHA NUNCA chama `set_live` em `write_a11y_info` → `Role::Status` sozinho não vira live-region; leitores de tela não auto-anunciam. O ADR (0028) está com o Arquiteto NESTA rodada — seu spike é a evidência dele.

## Fronteira de arquivos (sua, dono único nesta rodada)
- `app/lina-gpui/src/a11y_live.rs` (NOVO — o custom Element)
- `app/lina-gpui/src/a11y.rs` (toques mínimos: hook/registro do element; o mecanismo de coalescing 1×/turno em a11y.rs:79-131 JÁ existe e está testado — REUSE, não duplique)
- Teste headless novo no app.
- **NÃO toque:** `main.rs` (dono: Especialista em IA) — se o wire final exigir 1 linha em main.rs, REGISTRE o pedido na entrega (o Maestro coordena); `bridge.rs` (dono: Bug Finder); `canvas.rs`.

## O que provar
1. **Caminho (a) do ADR — custom `Element`** que sobrescreve `write_a11y_info` e chama `node.set_live(Polite)` no nó de status: implemente o mínimo (request_layout/prepaint/paint + tipos associados) para UM elemento de anúncio.
2. **Teste headless da LÓGICA do node** (lição a11y do repo: asserte a lógica do node, não o TreeUpdate): o node produzido tem live=Polite + role/label honestos.
3. **Roteiro de validação na tela** (VoiceOver anunciando 1 frase SEM foco no elemento): escreva o passo-a-passo exato em `tasks/epico-f1/spike-a11y-roteiro.md` (como ligar, o que falar, o que esperar) — quem executa é o Maestro/fundador (gpui não roda headless).
4. **Se o caminho falhar tecnicamente** (API do gpui não permite o override sem patch): documente COM EVIDÊNCIA (arquivo:linha do gpui vendorado que bloqueia) — isso muda a recomendação do ADR 0028 para o caminho (b) patch-no-pin, e essa evidência é exatamente o valor do spike.

## Honestidade (critério da story)
Nenhuma copy/doc do produto pode afirmar "conforme ARIA live-region" enquanto a tela não confirmar — confira por grep e registre na entrega.

## Entrega
`tasks/epico-f1/.entrega-spike-a11y.md`. Marcador: `.iniciado-spike-a11y`. Validação por-pacote no app (cd app/lina-gpui), exits diretos.
