# Conselho de Gate F1-3 — LENTE 4 (PESQUISA)

> **Auditor:** Lente 4 (independente, read-only — não construiu a onda) · **Data:** 2026-06-10
> **HEAD auditado:** `bc58f0b` · **Working tree:** 1 arquivo de peer modificado (`crates/lina-core/src/a2a.rs`, fora do escopo desta lente; não tocado)
> **Função:** spot-check fontes de pesquisa 13.x citadas pelas stories F1-3 vs o implementado —
> nada 🔴 REFUTADO re-entrou? nenhuma recomendação 🟢 foi "adaptada" em silêncio?
> **Fontes lidas na íntegra/trecho:** `13.4` (íntegra), `13.8` (seções Curator + matriz), vault
> `Debriefing Vibe Coding/` · `tasks/epico-f1/onda-3.md` · `gate-f13-teste-autonomo.md` ·
> `redteam-spawn-f1-3-6.md` · `.entrega-f1-3-7.md` · `assets/lina-doctrine/CLAUDE.md` (+espelhos
> por grep) · 11 SKILL.md + `rubrica.md` · `crates/lina-bootstrap/src/retro.rs` · `events.rs` (grep dirigido).

## VEREDITO: **OK-COM-RESSALVAS** (nenhuma infidelidade à pesquisa; 3 ressalvas, todas BAIXA e declaradas)

---

## Check (a) — 13.4 achado 2: revisor ISOLADO preservado na skill? ✅ PRESERVADO

- `lina-cold-review/SKILL.md:27-29`: *"Sua entrada são SÓ três coisas: (1) o artefato, (2) os
  critérios de aceite, (3) a rubrica. Você não tem e não pede o histórico/raciocínio do autor.
  Se ele chegar junto, **ignore-o** — o isolamento É o mecanismo."* — é exatamente o mecanismo
  do Superpowers que o achado 2 confirmou (sessão sem contexto do autor impede auto-endosso).
- O isolamento se propaga às skills vizinhas (não foi diluído na composição):
  `lina-orchestration/SKILL.md:54-56` (passo 8: *"cold-review (revisor ISOLADO, sem o contexto
  do autor → PASS)"*) e `lina-dispatch/SKILL.md:69` (exemplo Review: *"FUNÇÃO: revisor ISOLADO
  (sem o contexto do autor) · DIR: não conserte, só julgue"*).
- As duas calibrações anti-teatro (otimista/paranoico) das memórias citadas pela story estão no
  corpo (SKILL §3.1/3.2) e na rubrica §6 — fidelidade também às lições da Fase 0 que a story arrolou.
- Evidência de exercício real: fixtures cegas em `tasks/epico-f1/fixtures-coldreview/{bom,slop-plantado}/`
  e o gate (PARTE 1 item 6-7): handoff de review só com artefato+contrato; revisor B *re-derivou
  no arquivo* e deu FAIL→PASS com evidência.

## Check (b) — 13.4 achado 5: personalidade por ESTRUTURA, não adjetivos? ✅ SEGUE

- A doutrina codifica o achado literalmente: `assets/lina-doctrine/CLAUDE.md:48-49` — *"Isso não
  é um rótulo que você veste; é como você trabalha. O encanto vem do resultado avançando, nunca
  de adjetivos."*
- Cada traço vem amarrado a comportamento ou mecanismo (risco 8 da onda mitigado): dono-do-resultado
  → causa raiz sem hand-holding (51-55); "se você não assinaria, não está pronto" (57-59); bloco 3
  reframado como internalização (*"você consulta o vault porque é assim que você trabalha"*, 99-103);
  bloco 8 com opinião estética operacional — princípio-raiz + banimentos + direção declarada +
  vault-vence-o-gosto (300-319). Os mecanismos associados EXISTEM (rubrica, cold-review, template,
  gates) — nenhum item de personalidade é só cosmético.
- Achado 9 (padrão do system prompt da Anthropic) também encarnado: zero filler + "uma recomendação
  decisiva vence um menu" (321-327).
- Espelhos `AGENTS.md`/`GEMINI.md` carregam as mesmas seções de personalidade (verificado por grep
  dirigido às seções novas; escopo: o DELTA de personalidade, não o arquivo inteiro — tiers diferem
  por design).
- Dúvida 1 ao Maestro (processo vs identidade fixa) resolvida a favor do **processo**, coerente nos
  dois níveis: doutrina (*"Você não tem um visual fixo; tem o hábito de ter opinião"*, 313-314) e
  `lina-design-doctrine` (direção declarada POR PROJETO; "Solarpunk tech" aparece só como exemplo
  de statement, SKILL.md:32-34) — fidelidade ao paradigma duplo do cookbook (achado 3).

## Check (c) — A2A Protocol externo (🔴 REFUTADO, 13.4 achado 6): re-entrou? ✅ NÃO RE-ENTROU

- Grep por marcadores do protocolo externo (`agent card`, `json-rpc`, `a2a-protocol`,
  `.well-known`, `linux foundation`) em `assets/lina-skills/`, `assets/lina-doctrine/` e
  `crates/*/src/`: **zero ocorrências reais** (matches de "sse" eram falsos positivos:
  "e**sse**"/"de**sse**").
- Toda menção "A2A" nas skills é o **envelope interno** do Lina — e o uso é explicitamente
  conservador: `lina-dispatch/SKILL.md:50-51` — *"campos que JÁ existem no Envelope A2A; o
  template os USA, não inventa formato novo"*. `intent: spawn` é extensão registrada do contrato
  interno (onda-3 F1-3-6), não transport externo.
- Isso é exatamente a correção de rota que o próprio 13.4 (item 4 da lista priorizada) prescreveu:
  *"camada de coordenação própria, mínima e local… A2A-ready como design boundary, não entrega"* —
  e a proposta 5 do onda-3 a registra como fronteira global. Implementação fiel à refutação.

## Check (d) — Curator (13.8): lifecycle determinístico + julgamento pelo AGENTE? ✅ RESPEITA

- **Verbo determinístico, zero LLM:** `crates/lina-bootstrap/src/retro.rs` é projeção Rust pura
  sobre `&[EventRecord]` com `now_ms` INJETADO (wall-clock só no bin) — limiares como constantes
  re-derivadas arquivo:linha da fonte (`STALE_MS` 30d / `ARCHIVE_MS` 90d citando `curator.py:58-59`,
  retro.rs:31-34) e teste de boundary sem flakiness (retro.rs:723+).
- **Sugere, nunca aplica — por construção:** toda tentativa de mutação (`apply`/`archive`/`pin`/
  flags) retorna `RetroInvocation::Refused{mutation:true}` ANTES de qualquer caminho de escrita
  (retro.rs:481-542); *"não existe `lina retro apply`"*. Critério 4 da story é estrutural, não doc.
- **Julgamento pelo agente (inv#1):** `lina-retro/SKILL.md` põe o juízo no agente do terminal,
  lendo o relatório, propondo nos 3 tipos (skills/papéis/presets) com evidência apontável e
  **gate humano sempre** (SKILL:54-58) — o padrão dry-run+gate que o próprio Hermes usa.
- **First-run honesto:** *"dados insuficientes… Sem sugestao de manutencao"* (retro.rs:344),
  gated por `has_data` — o anti-mass-prune do 13.8 copiado.
- **Refutação do "quorum" respeitada:** zero ocorrências de "quorum" em assets/ e retro.rs;
  a correção do Maestro (`max_iterations=9999`, não 8) está registrada no onda-3 e é irrelevante
  à implementação (zero LLM).
- Eventos `SkillInvoked`/`SkillCreated` reservados up-front com upcasting (`events.rs:683-692`,
  2647+) — nasceu projeção, não sidecar imperativo (o anti-pattern do Hermes evitado).

## Check (e) — orçamentos ≤2k tokens (13.4 achado 2)? ✅ CUMPRIDOS (medição independente desta lente)

| Artefato | Medida | Est. tokens (chars/4 ÷ chars/3,5) | ≤2k? |
|---|---|---|---|
| **Delta de personalidade da doutrina** (986b6e8→7d2c198, adição pura, 0 remoções) | +533 palavras / +3.198 chars | ~800–915 | ✅ folga |
| lina-verification | 3.064 chars | 766–875 | ✅ |
| lina-code-doctrine / architecture / copy / design | 3.259–3.650 chars | 814–1.043 | ✅ |
| lina-design-doctrine | 3.650 chars | 912–1.043 | ✅ |
| lina-retro | 4.851 chars | 1.212–1.386 | ✅ |
| lina-cold-review | 5.275 chars | 1.318–1.507 | ✅ |
| lina-orchestration | 5.855 chars | 1.463–1.673 | ✅ |
| lina-spawn-terminal | 6.344 chars | 1.586–1.813 | ✅ |
| lina-dispatch | 6.751 chars | 1.687–1.929 | ✅ (o mais cheio; sob o teto mesmo na estimativa conservadora pt-br) |

- A profundidade foi movida para `references/` (rubrica.md 169 linhas, monitoramento.md) com
  carga on-demand — o mecanismo de economia de tokens do achado 2 aplicado por construção, não
  só obedecido no número.
- `lina-agent-bus` (~13.8k chars ≈ 3,4–3,9k tokens) EXCEDE 2k, **mas** é herança W3-3 pré-onda,
  não pertence à 1ª safra do catálogo, e está **byte-idêntica** ao commit pré-F1-3 (`986b6e8`) —
  a onda não a inflou. Fora do critério; registrado como nota informativa.

## Spot-checks adicionais de fidelidade

- **Red-team do spawn (F1-3-6 critério 5, fonte 13.14/ADR 0007):** `redteam-spawn-f1-3-6.md` —
  **0 ALTA no código committed**, com downgrades re-derivados no código (não assinados de fé) e
  riscos de seam (M3/M4) **declarados** como forward, não silenciados. Skill `lina-spawn-terminal`
  ensina os gates como física do mundo (cascata→humano SEMPRE, teto 2/turno, binding interno
  não-forjável, `intent: spawn` no log) — fiel ao vocabulário intent-vs-action do 13.14 achado 5.
- **Rubrica:** traduz a definição operacional do 13.4 achado 1 (os 3 eixos, §0) em marcadores
  com "como verificar"; veredito reprodutível por construção (ALTA⇒FAIL; BAIXA nunca flipa;
  limiar 80) — coerente com o critério 4 de F1-3-2.

## Ressalvas (nenhuma muda o veredito de fidelidade; todas declaradas, não silenciosas)

1. **[BAIXA — adiamento declarado]** `SkillPinned`/`absorbed_into` ("padrões do Curator a copiar
   já na v0", onda-3:187) NÃO foram implementados — `.entrega-f1-3-7.md:29-30` declara o corte
   com razão: *"ficam para quando existir verbo de mutação — F2; aqui o verbo SÓ relata"*.
   Internamente consistente (pin é opt-out de mutação; v0 não tem mutação alguma; executar
   consolidação já era F2 pela própria story). É desvio de escopo declarado, não adaptação
   silenciosa de recomendação — mas o Conselho deve ratificar o adiamento.
2. **[BAIXA — informativa]** `lina-agent-bus` segue >2k tokens (herança pré-onda, intocada pela
   F1-3). Se o orçamento do achado 2 virar regra de TODO o catálogo (não só da 1ª safra), ela é
   a única fora — candidata a dieta em story própria.
3. **[BAIXA — cross-ref, dono já registrado]** ACHADO-1 do gate (`gate-f13-teste-autonomo.md`):
   o kit por-nó instala SÓ `lina-agent-bus` — as 11 skills da onda não chegam ao terminal sem
   instalação manual. Não é infidelidade à pesquisa, mas enquanto a fiação não fechar, os
   mecanismos 🟢 validados não ativam em produção. Já é carry-forward declarado do gate; esta
   lente apenas o referenda como pré-condição de eficácia.

---

**Conclusão:** nada REFUTADO re-entrou; nenhuma recomendação 🟢 foi adaptada em silêncio — os dois
desvios encontrados estão declarados por escrito com razão técnica. Fidelidade pesquisa→implementação
da onda F1-3: **OK-COM-RESSALVAS**.
