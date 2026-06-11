# DESPACHO r3-f1-5-5 — Especialista em IA
**id:** `f1-5-5` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md` (fronteiras v2!)

## F1-5-5 · Suspensão real de ociosos — FATIA CORE (fonte: `ondas-5-6.md` linhas 90-98; P0 NÃO-condicional)
Máquina `Active → Idle → Suspended` no core: threshold configurável (default conservador) E sem prompt pendente E sem custódia pendente E não-pinado → Suspended. **Decisão load-bearing:** CONTINUAM o reader do PTY (NUNCA pare de drenar — flow-control W0-3 travaria o CLI), o advance do VT e o harvest do scrollback; PARAM render/montagem (política do SHELL = costura, fora desta fatia). Despertar: foco/clique (shell), A2A dirigida, prompt/custódia detectados, retomada de output (configurável). Eventos: use `NodeStatusChanged` existente se cobrir; variante nova em events.rs SÓ aditiva (você é interno autorizado a hunk aditivo — registre na entrega).
**Critérios headless:** (a) máquina de estados com TODOS os guards (prompt pendente bloqueia; custódia bloqueia; pinado bloqueia; A2A acorda; foco acorda); (c) `lina ask` para nó suspenso ENTREGA e acorda (caminho real estilo gate_onda3); (d) harvest continua durante suspensão (output íntegro no store). O critério (b) [PROF]/zero-trabalho-de-render e o (e) tela do fundador = shell/roteiro, fora da fatia — deixe os ganchos (`SuspendPolicy` consultável pelo shell).

## Fronteira (sua)
`crates/lina-core/src/lib.rs` (PtyHost/Supervisor — estados) · NOVO `crates/lina-core/src/suspend.rs` se preferir módulo · `events.rs` SÓ hunk aditivo · testes.
**⛔ NÃO toque:** attention.rs/broker.rs (EXTERNO) · router.rs (estável — se o despertar-por-A2A pedir hook no deliver, REGISTRE a costura) · app/ · scrollback.rs (consuma).
Entrega: `tasks/epico-f1/.entrega-f1-5-5.md` · Marcador: `.iniciado-f1-5-5`.
