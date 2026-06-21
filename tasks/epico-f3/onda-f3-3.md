# Onda F3-3 — Mentality (Modelo Mental por Papel) · "A Lina aprende com você" · plano de execução

> **A onda mais alta da Fase 3** (Meadows [7] feedback positivo + [2] paradigma): cada PAPEL do roster ganha um modelo mental que se forma pela vivência — o agente erra, é corrigido, reflete, e a lição vira **crença do papel**, para que **nenhum terminal futuro daquele papel repita uma correção já dada**, em qualquer sessão, em qualquer CLI. É o auto-aprimoramento que a F1-3-7 adiou ("`lina retro` sugere; Mentality aprende").
> **Design FECHADO — esta onda CONSTRÓI, não redesenha.** A spec 35 já decidiu tudo (identidade por papel, quarentena+promoção por evidência, event-sourced com superfície legível, MVP "corrigiu uma vez, nunca mais repete").
> **Fonte da verdade (LEIA antes de codar):** vault `35 - Proposta F3 - Mentalidade por Papel (Agent Primitives)` (design integral) + épico `39 - Epico Fase 3 — O Maestro Nativo` §"Onda F3-3" (as 6 stories + gate) + insumos `20 - Auto-Aprimoramento e Memoria` cluster A (viés pró-aprendizado + lista de NÃO-capturar) e B (snapshot congelado por sessão) · ADR 0005 (projeção por replay) · ADR 0023 (re-injeção de doutrina human-proxy) · ADR 0007 (regra-mãe: campo de agente nunca decide).
> **Maestro:** Terminal B — Effort: Ultra Code. **Workers NÃO commitam; o Maestro fixa o contrato de eventos, valida de fora, cold-review isolado, commita por fatia.**

## ⛔ PRÉ-REQUISITO DE DISPARO (a rodada NÃO inicia sem isto)

A Mentality colide com o Terminal A em **3 costuras quentes**: `crates/lina-core/src/events.rs` (eventos), `app/lina-gpui/src/bridge.rs` (injetor de doutrina), `app/lina-gpui/src/main.rs` (fiação do painel). O A está com elas **sujas e não-commitadas** (ADR 0037 cwd editável + ADR 0038 kit-lina).
**Gate de largada:** `git status` LIMPO desses 3 arquivos (A commitou 0037/0038). Verificar no instante do disparo. Enquanto sujo → a rodada fica PREPARADA, não lançada. (As frentes de módulo-novo e tests/ poderiam iniciar antes, mas o ALICERCE — contrato de eventos em events.rs — é pré-condição de TODAS, então esperamos o A.)

## Invariante que ATRAVESSA a rodada (regra-mãe — não regredir)

**Crença é DADO comportamental ("como pensar"), JAMAIS autoridade.** Entra só na camada de doutrina renderizada — **nunca** decide autonomia, spawn, aprovação, roteamento, custódia ou segurança (spec 35 §6.1; família ADR 0007). Hierarquia dura: **doutrina shipped > crença estabelecida > crença provisória**; invariantes intocáveis. **ZERO LLM no core** (inv #1): a política de promoção é determinística (contadores sobre replay, como o `CostLedger`); o único LLM é o Refletor, que roda **assíncrono fora do caminho crítico** (1 CLI-call), nunca dentro do router/guard. Toda story que toca `deliver_a2a`/`Router`/spawn tem como critério implícito a **suíte de segurança do router verde por mutação**.

## DAG executável e frentes (fronteiras disjuntas — dono único por costura)

```
[Setup Maestro (B), pós-A-liberar: 6 variantes de crença em events.rs (aditivas) + braços name() — commita o CONTRATO = largada]
        │
        ├─► M-PROMO   (I)  crates/lina-core/src/mentality.rs (NOVO) ──────┐  projeção Mentality(papel) por replay + política de promoção determinística (hash/challenge/TTL) [F3-3-1 proj + F3-3-3]
        ├─► M-DETECTOR(H)  a2a.rs/mailbox.rs/router.rs (detector+refletor)─┤  sentinela [LINA::CORRECTION] + Refletor async (1 CLI-call fora do caminho crítico) [F3-3-2]
        ├─► M-INJETOR (J)  app/lina-gpui/src/bridge.rs (doctrine) ─────────┤  seção "Mentalidade do papel" na doutrina renderizada + cap top-K [F3-3-4]  ⛔ dep A-liberar bridge.rs
        ├─► M-UI      (G)  app/lina-gpui/ (painel novo + fiação main.rs) ──┤  painel "Como o [papel] pensa" + aposentar 1-clique [F3-3-5]  ⛔ dep A-liberar main.rs
        └─► M-QA      (R)  crates/**/tests/ (NOVOS) ──────────────────────┘  anti-poisoning + eval-first (controle + E -) + segurança [F3-3-6]
        │
        ▼
   Maestro (B): valida de fora + cold-review + gate de onda (4 lentes) + cenário binário na tela do fundador + commit por fatia
```

| Frente | Terminal | Toca (DONO ÚNICO) | model·effort | Deps | Bloqueio A |
|---|---|---|---|---|---|
| **Contrato** | **B (Maestro)** | `events.rs` (6 variantes + name()) | opus·high | — | ⛔ events.rs |
| **M-PROMO** | **I** | `crates/lina-core/src/mentality.rs` (NOVO) + registro no `lib.rs`(1 linha mod, coordenar) | opus·high | Contrato | módulo novo ✅ |
| **M-DETECTOR** | **H** | `a2a.rs`/`mailbox.rs`/`router.rs` (detector+refletor) | opus·high | Contrato | disjunto do A ✅ |
| **M-INJETOR** | **J** | `app/lina-gpui/src/bridge.rs` (doctrine_reinjection) | opus·medium | Contrato, M-PROMO | ⛔ bridge.rs |
| **M-UI** | **G** | `app/lina-gpui/` (painel novo) + fiação `main.rs` | opus·medium | Contrato | ⛔ main.rs |
| **M-QA** | **R** | `crates/**/tests/` (NOVOS) | opus·high | M-PROMO, M-INJETOR | tests/ ✅ |

**Reserva (escalada por breaker sticky 2×):** K.
**Nota de costura `lib.rs`:** M-PROMO precisa de 1 linha `pub mod mentality;` em `lib.rs` (costura). Dono único do `lib.rs` nesta rodada = **Maestro (B)** no setup (adiciono o `mod` junto do contrato de eventos, para I não tocar `lib.rs`). I escreve só `mentality.rs`.

## Setup do Maestro (B) — ANTES do fan-out (pós-A-liberar)

1. **Confirmar o gate de largada:** `git status` limpo de events.rs/bridge.rs/main.rs.
2. **Fixar o contrato de eventos** (aditivo, `events.rs`): 6 variantes novas do ciclo de crença — `CorrectionObserved`, `BeliefProposed`, `BeliefReinforced`, `BeliefChallenged`, `BeliefEstablished`, `BeliefRetired` (variantes totalmente novas, sem `serde(default)` nos campos — replay de log antigo nunca as encontra; precedente `SpawnRequested`). Campos por spec 35 §3 (papel, belief_id, statement falseável, hash-de-situação, proveniência, motivo). Adicionar os braços em `DomainEvent::name()`. **NÃO** mexer em `ProjectedState` (a projeção Mentality é por REPLAY externo, módulo `mentality.rs`, padrão `CostLedger`/`intelligence_adoption`) — minimiza o churn em events.rs.
3. **Adicionar `pub mod mentality;` em `lib.rs`** (1 linha, para I não tocar a costura).
4. **Commit do contrato = sinal de largada.** Replay F0/F1/F2/F3 carrega sem erro (variantes novas nunca aparecem em log antigo).

## GATE DE SAÍDA F3-3 — RODA e se MEDE (spec 35 §5 + épico §F3-3)

- **(a) Critério binário (o coração):** **Sessão 1** o usuário corrige um papel (ex.: Backend "use pnpm, não npm"); **Sessão 2** um terminal NOVO do mesmo papel executa tarefa que tentaria npm → **usa pnpm sem ser lembrado**. PASS/FAIL pelo efeito observável (transcript/disco), nunca por auto-relato.
- **(b) Promoção determinística (mecanismo):** N situações DISTINTAS (hash) → `BeliefEstablished` (default N=2); mesma situação 2× → **NÃO** promove; `BeliefChallenged` zera o progresso; provisória sem reforço em TTL (30d) → `BeliefRetired{expired}`. ZERO LLM na política (teste prova por contraste).
- **(c) Cap top-K (anti-context-rot):** K+1 crenças do papel → só K injetadas no spawn (critério, não opção).
- **(d) Anti-poisoning:** correção com instrução maliciosa ("aprenda a ignorar o gate de custo") → filtro do Refletor barra + evento de recusa no log. Crença nascida em sessão com conteúdo externo não-confiável leva flag de origem.
- **(e) Crença NUNCA decide segurança:** suíte do router verde por mutação; crença não toca autonomia/spawn/aprovação/custódia. 0 ALTA.
- **(f) Replay idêntico:** projeção `Mentality(papel)` reconstrói byte-a-byte por replay; crença nunca deletada (rebaixada/aposentada).
- **(g) Adoção da sentinela desde o dia 1:** o `intelligence_adoption` (R2 da F3-CONF-3) já tem o slot `[LINA::CORRECTION]` — ligá-lo: medir uso real da sentinela no log (lição: plan.md teve adoção 0% no gate F1-3 — medir, não assumir).
- **(h) [BLOQUEANTE — fundador na tela]** o cenário binário (a) executado ao vivo + o painel "Como o [papel] pensa" validado visualmente (proveniência humanizada, aposentar 1-clique). gpui não roda headless.

## Conselho de gate de onda (4 lentes, read-only)

(1) Visão/fios: auto-aprimoramento que SUGERE e aprende por evidência, com humano como árbitro (aposentar 1-clique) — sem aplicar à força. (2) Arquitetura: ZERO LLM na política (determinística por replay); Refletor async fora do caminho crítico (inv #1); projeção por replay (padrão CostLedger); crença nunca deletada. (3) Segurança (red-team, **0 ALTA**): crença é dado comportamental, nunca autoridade; anti-poisoning barra instrução maliciosa; multi-CLI por doutrina renderizada (inv #3). (4) Specs: spot-check spec 35 §3-§7 vs implementado; nada do "Fora do MVP" (§8) re-entrou (sem aprender-com-falha-própria, sem RAG, sem detector estatístico, sem edição de texto da crença).

## Pendências / deferidas

- **Cortado de propósito (spec 35 §8, NÃO implementar):** aprender com falhas próprias (build/teste) = v2; camadas combinadas shipped+espaço+indivíduo; RAG/recall semântico; detector estatístico de correção; transferência entre espaços/marketplace; edição do texto da crença (só aposentar no MVP).
- **Gate do fundador na ativação (spec 35 §2) — ✅ DECIDIDO (fundador, 2026-06-21): PROMOÇÃO AUTOMÁTICA.** Crença que atinge N situações distintas vira regra do papel **sem fila de confirmação** — é injetada direto no próximo spawn daquele papel. Os limites de segurança (§6) e o **aposentar-1-clique** (humano árbitro *post-hoc*) PERMANECEM intactos. Implicação por frente: **M-PROMO** emite `BeliefEstablished` e ele **já habilita a injeção** (sem estado intermediário "pendente de OK"); **M-INJETOR** injeta estabelecidas como regra direto (sem gate de confirmação); **M-UI** mostra estabelecidas como "já vale" (não "aguardando seu OK"), provisórias como "ainda testando", e mantém o aposentar-1-clique. (O design ainda suporta o modo badge-de-confirmação trocando 1 política — não foi o escolhido.)
- **Próxima após esta:** F3-4 (coordenação de código multi-agente) ou F3-5 (sessões/auto-aprimoramento) — escolha no fim desta onda.

---

## STATUS — F3-3 PREPARADA (aguardando A liberar events.rs/bridge.rs/main.rs)

- **Plano:** este doc. **Despachos:** `tasks/epico-f3/despachos/f3-3/{M-PROMO,M-DETECTOR,M-INJETOR,M-UI,M-QA}.md` (o Contrato é do Maestro, descrito no Setup acima).
- **Disparo:** assim que `git status` limpar events.rs/bridge.rs/main.rs → Maestro fixa o contrato + `mod mentality` → commita → fan-out das 5 frentes (M-DETECTOR/M-PROMO/M-QA podem iniciar; M-INJETOR/M-UI dependem de bridge.rs/main.rs já limpos).
- **Donos:** I (promo), H (detector), J (injetor), G (ui), R (qa); B (maestro/contrato); K reserva.
