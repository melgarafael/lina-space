# Conselho de Gate — Onda F1-1 (Observabilidade e Cognição do Trabalho) — CONSOLIDADO

> **Data:** 2026-06-10 · **HEAD auditado:** `2de17cd` · **Protocolo:** "Como executar" do épico 34
> (4 lentes independentes, read-only, auditores que NÃO construíram a onda; rodado via Workflow
> com agentes Explore + leitura por `git show HEAD:` nos caminhos em mutação pela rodada 1).
> **Resultado: ZERO FALHA · 0 ALTA · 3× OK + 1× OK-COM-RESSALVAS.**

## Os 4 vereditos

| Lente | Veredito | Síntese |
|---|---|---|
| 1 Visão/Fios | **OK** | 9 stories operacionalizam os 5 fios com MECANISMO; dashboard sem jargão (Trabalhando/Ocioso/…); custo honesto com "~" obrigatório; fila cobre TODO bloqueante (Yn/Choice/Trust, direção do fundador 2026-06-07) |
| 2 ADR↔Código | **OK** | ADR 0021/0023/0024 re-derivados no fonte: snapshot decide, dedup stable_id, auto-deny sem campo decision (auto-approve impossível por construção), write atômico lock grid→writer, reinject em-processo sem drop-zone de FS; invariantes 1-7 e portas intactas |
| 3 Segurança | **OK** | 5 invariantes VALEM: binding do LOG na cadeia inteira; check_screen aborta com zero bytes em divergência; auto-deny único desfecho automático; fix A3 sem regressão pós-`f6db7e3`; hooks/OTel loopback-only. **0 ALTA.** Relatório `redteam-gate-f1-1.md` fiel ao HEAD |
| 4 Pesquisa | **OK-COM-RESSALVAS** | Fidelidade às 13.5/13.9/13.13; nada refutado re-entrou (choice por âncora de chrome, nunca regex de lista); limites honestos declarados (FP threshold; Notification ~5,8s intrínseco) |

## Achados (nenhum bloqueia; todos com dono)

- [MEDIA→backlog] M2/M4/M6/M7 do red-team seguem MEDIA, confirmados (observe_grid cross-check; janela kernel-buffer documentada; colisão teórica K no hash; guard dinâmico do resolve) — donos nomeados no `redteam-gate-f1-1.md`.
- [MEDIA→spike nomeado] Calibração de timing/prompt do Antigravity contra binário real (gatilho: lib estável ~fim jul/2026) + `.agents/hooks.json` (capability `hooks=false` honesta até lá).
- [MEDIA→pendência de produto F1-1-7, já nomeada] Re-emissão de prompt obsoleto sob scroll (toast 2×; mitigação deve preservar recall — `fp-rodada-completa.md` §Anomalia).
- [BAIXA→rodada 2 desta sessão] Render do badge EoL Gemini (copy aprovada; falta o render).

## Pendências de TELA que seguram a DECLARAÇÃO do gate (não atravessam onda)

1. **AC-0021.7** — aprovar pelo toast destrava um Claude real bloqueado em y/n (audit `PermissionResolved→ApprovalInjected` no log). → roteiro consolidado do fundador (esta sessão).
2. Render do badge EoL Gemini visível no modal/dashboard. → rodada 2 (código) + mesma tela.

**Decisão do Maestro (2026-06-10): gate F1-1 = PASS CONDICIONAL** — toda a parte auditável passou
(conselho zero FALHA + red-team 0 ALTA + suítes verdes); a declaração formal sai junto com a tela
do fundador do roteiro consolidado. Mesmo padrão do gate F1-0 ("declara após a tela").
