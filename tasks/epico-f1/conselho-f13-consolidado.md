# Conselho de Gate — Onda F1-3 (Inteligência da Lina) — CONSOLIDADO

> **Data:** 2026-06-10 · **HEAD auditado:** `09876a4` · **Protocolo:** "Como executar" do épico 34
> (4 lentes independentes, read-only, auditores que NÃO construíram a onda).
> **Resultado: ZERO FALHA · 0 ALTA novos · 4× OK-COM-RESSALVAS → GATE F1-3 DECLARADO.**

## Os 4 vereditos

| Lente | Auditor | Veredito | Relatório |
|---|---|---|---|
| 1 Visão/Fios | Analista Concorrentes | OK-COM-RESSALVAS — 7 stories operacionalizam os fios com MECANISMO; sugere-nunca-aplica enforced por construção | `conselho-f13-lente1-visao.md` |
| 2 ADR↔Código | Spec Writer | OK-COM-RESSALVAS — zero violação bloqueante; checks a-g ✅ re-derivados arquivo:linha; 1 falso-positivo de finder derrubado | `conselho-f13-lente2-arquitetura.md` |
| 3 Segurança | Pesquisador Oportunidade | OK-COM-RESSALVAS — 0 ALTA novos nos 5 commits pós-red-team; anti-texto-colado preservado; bounds M1-M4 inalterados | `conselho-f13-lente3-seguranca.md` |
| 4 Pesquisa | UX flow | OK-COM-RESSALVAS — fidelidade 13.x→implementação; refutados não re-entraram; orçamentos cumpridos (medição independente) | `conselho-f13-lente4-pesquisa.md` |

## Evidência do gate (acumulada)

- **7/7 stories no código:** `7d2c198` → `988d1c7` (fundações + orchestration + dispatch + spawn + retro)
- **3 fixes do teste:** `dd32d46` (compile app) · `bc58f0b` (skills no kit) · `09876a4` (retenção not-ready)
- **Cenário real executado** (teste autônomo computer-use, `c7f9f6a`): pedido leigo → decompõe →
  delega → cold-review FAIL real → corrige → PASS → narração leiga → anti-loop
- **Critério 2 — worker INDUZIDO** (`baba99e`): desvio detectado por validação-de-fora + corrigido;
  travamento detectado ativamente (~30s) + formalizado (DLQ/breaker) + recuperado
- **⛔ Validação do fundador NA TELA:** ✅ "Validei tudo visualmente, pode prosseguir" (2026-06-10)
- **Red-team do spawn:** 0 ALTA (`1d173d2`)

## Backlog nomeado (com dono)

### → SEAM APP do spawn (próxima rodada nomeada — pré-condições)
1. **M2/R3-L2/R1-L1** — fiar autonomia REAL no RouterConfig do app (`bridge.rs:653` `..default()`→Assisted; ramo manual/cap desarmado em prod)
2. **M3** — dedupe durável do spawn (consultar log por `msg.id` antes de aprovar / admissão idempotente)
3. **M4/R4-L2** — terminal spawnado nasce COM binding de cascata (`SpawnApproved`→`admit_node` deve carimbar; senão anti-fork-bomb cai end-to-end)
4. **R2-L1** — skill `lina-spawn-terminal` JÁ instalada nos kits promete terminal que ainda não nasce fisicamente — fiar OU degradar a promessa na skill até o seam
5. **R1-L3** — retenção not-ready não sobrevive a restart (ledger não rastreia `MessageRetained` → drop fail-safe; família M3) — rastrear retenção no ledger

### → Pendências de story (não atravessam para F2 sem fechar)
6. A/B cego da doutrina F1-3-1 (Pesquisador avalia; harness do Analista)
7. Portabilidade 3-CLI REAL das skills F1-3-3 (transcripts Codex + 1 outro)
8. Breaker 2-falhas-do-mesmo-item + anti-race `parents:` F1-3-4 (não exercitados no cenário; adoção do plan.md = 0%, métrica acompanhada)
9. Repack `dist/Lina.app` com a onda inteira + fixes (o fundador testa o .app, não o dev build)

### → Doc (alinhar texto, sem código)
10. **M1/R5-L2** — ADR 0007/seam: documentar janela de liveness 60s do binding ("SEMPRE" tem caveat)
11. **R1-L2** — ADR 0019 §6: registrar o gap aceito do cap origem-burst (decisão do Maestro 2026-06-10)
12. **R2-L2** — cascata: código mais restritivo que o ADR (gate ANTES do cap) — alinhar texto
13. **B1** — teto de custo OFF por default (painel/seam decide ligar default)
14. Ratificar adiamento de `SkillPinned` para F2 (lente 4)

## Fechamento dos itens (atualizado 2026-06-10, sessão de fechamento da F1)

- **Itens 1-5 (SEAM APP):** ✅ fechados na rodada SEAM (`d59283b`) — M4 binding-na-admissão ·
  M2 autonomia real · M3 dedupe durável · R2 fiação física + banner · R1-L3 retenção no ledger.
- **Item 9 (repack):** ✅ `dist/Lina.app` 2026-06-10 13:20.
- **Itens 10-13 (doc):** ✅ fechados pelo Maestro 2026-06-10 — ADR 0007 (caveat janela de
  liveness 60s) · ADR 0019 §Decisão-6 (texto alinhado ao código: cascata gateada ANTES do cap;
  gap origem-burst registrado como aceito) · ADR 0005 (nota B1: teto OFF default, decisão de
  default fica com o painel).
- **Item 14:** ✅ **RATIFICADO pelo Maestro (2026-06-10):** `SkillPinned`/`absorbed_into`
  adiados para F2 — consistente com o sugere-nunca-aplica (v0 não tem verbo de mutação; pin é
  opt-out de mutação que ainda não existe).
- **Itens 6-8 (pendências de story):** em execução na sessão de fechamento da F1 (A/B doutrina ·
  3-CLI real · breaker 2×/anti-race).
