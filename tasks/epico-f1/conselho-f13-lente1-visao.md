# Conselho de Gate F1-3 — LENTE 1 · VISÃO / FIOS CONDUTORES

> **Auditor independente read-only** (não construí nada da onda). HEAD auditado: `bc58f0b`.
> **Lido na íntegra:** as 11 skills de `assets/lina-skills/` + `references/` (rubrica, monitoramento) ·
> `assets/lina-doctrine/CLAUDE.md` · `tasks/epico-f1/onda-3.md` · `gate-f13-teste-autonomo.md` ·
> `docs/adr/0019` · entregas `.entrega-f1-3-{1..7}.md`, `.entrega-achado1.md`, `.entrega-seam-spawn.md` ·
> `redteam-spawn-f1-3-6.md`. **Verificado no código (spot-checks):** `bridge.rs:655` (RouterConfig),
> `router.rs:33/77/135/1676` (autonomy), `retro.rs` (porta única), `lina.rs:43` (verbo spawn),
> `fixtures-coldreview/` (existem). Escopo do que NÃO verifiquei declarado em §5.

## Veredito: **OK-COM-RESSALVAS**

A onda operacionaliza os fios — mecanismo, não cosmética. Nenhuma story "só menciona" o fio
que cita; o risco 8 da onda (personalidade regredir a adjetivo) foi honrado: todo item tem
mecanismo associado (rubrica com IDs e regra de veredito; loop com breaker; gate por
construção no retro; binding inforjável no spawn). As ressalvas são de **fiação e validação
pendente**, não de visão errada. Lista numerada em §4; nenhuma é bloqueante NOVA — a
bloqueante (tela do fundador) já está declarada pela própria onda.

---

## §1 — Pergunta (a): cada story operacionaliza o fio que cita?

| Story | Fio(s) | Operacionaliza? | Evidência |
|---|---|---|---|
| F1-3-1 doutrina | inteligência da Lina | ✅ | Arquétipo vira COMPORTAMENTO: "busca antes de assumir" (bloco 3 como internalização), "dono do resultado" (bloco 1), banimentos nominais + princípio-raiz anti-datação (bloco 8). Contrato W3-2 verificado na entrega (8 blocos em ordem, 0 placeholder novo, md5 idêntico nos 3 espelhos). |
| F1-3-2 rubrica + cold-review | gates de qualidade | ✅✅ (a mais forte da onda) | 17 marcadores com ID/severidade/como-verificar; veredito mecânico (≥1 ALTA ⇒ FAIL; limiar 80); calibração BIDIRECIONAL contra os dois modos de falha documentados nas memórias da Fase 0. **Evidência empírica real** (.entrega-f1-3-2): 6 revisores cegos — slop 14/13/14 de 14 detectadas, FAIL×3; bom PASS×3 (96/100/100), 0 violações inventadas não-BAIXA, **zero flip**. No gate ao vivo: FAIL c/ bloqueador real → fix → re-derivação NO ARQUIVO → PASS com ressalva honesta. |
| F1-3-3 catálogo skills | inteligência + gates de qualidade | ✅ | 11 skills no padrão: description rica p/ ativação semântica, corpo ≤2k tokens, "Notas por CLI", cada doutrina ancorada nos IDs da rubrica (DES-1..5, COD-1..5, COP-1..4, ARQ-1..3) — as skills e a rubrica são o MESMO sistema, não duas listas. Ativação 9-10/10 ×3 juízes (entrega). ACHADO-1 (skills não chegavam ao terminal) corrigido no HEAD auditado. |
| F1-3-4 orchestration | inteligência + observabilidade | ✅ | Loop de 8 passos é PROCEDIMENTO (não princípios); monitoramento CONSOME o ADR 0019 (não inventa — "Não-objetivos" explícito); anti-race no INSTANTE do despacho; breaker sticky 2×; gate de saída = cold-review PASS + validar de fora. Gate real: critérios 1, 2 (induzido: desvio detectado+corrigido; travamento detectado+recuperado) e 5 ✅ observados evento-a-evento. |
| F1-3-5 dispatch | governança + gates de qualidade | ✅ | 5 campos como PROTOCOLO (sem `PRONTO:`/`BLOCKED:` = falha de protocolo, não "esqueceu"); pull-then-context usa campos EXISTENTES do envelope (não inventa formato — âncora honrada); re-despacho informado. Exercitado ao vivo no gate (contrato @2, [EXPECTED], re-cobrança citando o que faltava). |
| F1-3-6 spawn | governança | ✅ no design/core · ⚠️ fiação | Red-team de invariantes: **0 ALTA aberto**, cada invariante re-derivado com arquivo:linha (campo de agente nunca decide — `router.rs:720/1657/1479-83`; cascata sempre gated — `:1685`; cap inforjável — `:1697-1709`; NodeId só Supervisor). MAS: M2 = autonomia **desarmada em prod** (ver R1) e seam da tela pendente (ver R2). |
| F1-3-7 retro | auto-aprimoramento sugere-nunca-aplica | ✅✅ | Ver §2. |

**Cruzamento com o doc 01/âncoras:** as stories usam os canais existentes (bootstrap turno-0,
envelope A2A com campos existentes, event log como fonte de veredito) — nenhuma abriu canal
paralelo nem fechou porta de continuidade que eu detectasse nos docs.

## §2 — Pergunta (b): o auto-aprimoramento respeita "sugere-NUNCA-aplica"?

**SIM — enforced por construção em três camadas, não por promessa:**

1. **Código:** `retro.rs` — verbo determinístico ZERO-LLM, projeção pura (`project_retro`
   sobre `&[EventRecord]` + `now_ms` injetado); `classify_retro_args` é **porta única** que
   recusa qualquer arg de mutação (`apply`/`archive` não existem); testes de integração
   rodam o **binário real** (`retro_cli.rs`).
2. **Skill:** `lina-retro` repete o contorno do fundador verbatim ("você NUNCA aplica", "não
   existe `lina retro apply`", "recusado de propósito"), exige evidência apontável por
   proposta ("sem evidência ⇒ sem proposta"), propõe nos TRÊS tipos exatos da decisão
   (skills/papéis/presets), e tem first-run honesto ("dados insuficientes → NÃO invente").
3. **Eventos:** `SkillInvoked`/`SkillCreated` reservados up-front no log (`events.rs:683+`)
   — dado para sugestão, nunca gatilho de ação.

O risco 6 da onda (má leitura do Hermes → mutação autônoma) está travado. Nenhuma violação
encontrada em skill, doutrina ou código spot-checado.

## §3 — Pergunta (c): inv#6 — narrações/copy vazam jargão ao leigo?

**OK com 3 observações BAIXAS (nenhuma violação dura).** O bloco 8 da doutrina proíbe jargão
de orquestração (PTY/handoff/broadcast/sentinela) e TODA skill com superfície ao leigo tem
seção anti-eco com exemplos ❌/✅ (agent-bus §3, cold-review §4, spawn "Narração ao leigo",
orchestration §guardrails, retro §gate). A doutrina ainda manda o vocabulário interno de slop
("Inter, glassmorphism") NUNCA virar fala ao usuário. No gate real, a narração final foi
exemplar ("Aprovada! ✅ … a única pendência real é você me dizer qual plataforma de pagamento").

- **BAIXA-1:** "QA" como sigla aparece em narrações-EXEMPLO canônicas (agent-bus §3 "O QA
  acabou de me mandar…"; spawn "Trouxe um especialista de QA…"), enquanto a mesma tabela usa
  "equipe de qualidade" em outra linha. Inconsistência de vocabulário leigo nos exemplos que
  os agentes vão imitar.
- **BAIXA-2:** narração-exemplo do retro usa "skill"/"preset" ("Sugiro arquivar a skill X").
  É vocabulário de produto (defensável), mas está um degrau acima de "papel/especialista" no
  registro leigo — decisão de produto a confirmar, não defeito.
- **BAIXA-3:** `lina-design-doctrine` não repete o antídoto de eco (as irmãs repetem). A
  doutrina bloco 8 cobre o caso; custo de uma linha padronizaria.

## §4 — Pergunta (d): pendências de tela/validação acumuladas (TODAS as encontradas)

**Bloqueante declarada pela onda:**
1. ⛔ **Validação do fundador NA TELA** do gate da onda (cenário LP-3-terminais) — onda-3 §2
   + gate-f13:60. O teste autônomo de 2026-06-10 é evidência forte de prontidão, **não** o gate.

**Ressalvas de fiação (governança prometida > mecanismo fiado):**
2. **R1 — Autonomia DESARMADA em prod** (red-team M2; confirmado por mim no HEAD:
   `bridge.rs:655` cria `RouterConfig { ..default() }` → `Assisted`; `router.rs:33/77` admite
   que o parser `workspace.json → autonomy` não existe). Consequência: o bloqueio em `manual`
   que a doutrina (bloco 5) e a skill spawn ("o verbo recusa na hora") PROMETEM não vale em
   produção até fiar. O core enforça (`router.rs:1676` + teste `manual_autonomy_blocks_delegation`)
   — é fiação, não design. Padrão conhecido: promessa canônica à frente do mecanismo.
3. **R2 — Seam da tela do spawn pendente**: a criação física (admit_node/PTY/1º prompt/banner)
   foi deliberadamente separada (Rodada 3 = CORE-só, decisão do Maestro; `.entrega-seam-spawn`
   aprovada como DOC). No gate real, o Designer nasceu via **⌘N humano** — o caminho
   agente-spawna-terminal **nunca foi exercitado end-to-end**. Risco de coerência: a skill
   `lina-spawn-terminal` JÁ está nos kits (bc58f0b instala a safra completa) ensinando um
   verbo cuja ponta física não está provada — o agente pode prometer "trouxe o especialista"
   e o terminal não nascer. Sugestão: sequenciar (gate da skill no kit OU seam antes da tela
   do fundador) e exercitar o cenário feliz do spawn (crit. 1 da story) na validação de tela.
4. **R3 — Binding efêmero do gate de cascata** (red-team M1): poda de 60s do
   `delivered_root` pode reclassificar cascata como origem. Decisão de durabilidade pendente.

**Critérios de story ainda não exercidos (validação pendente, não defeito):**
5. F1-3-1 crit.1 — **teste A/B** doutrina v-atual vs F1-3-1 com cold-review cego: pendente
   (estava bloqueado pela rubrica; F1-3-2 fechou — está DESBLOQUEADO, falta rodar).
6. F1-3-1 crit.2 — golden test do bootstrap pós-delta: escalado na entrega (fora do escopo
   do autor); presumivelmente coberto pelos 283 testes verdes do `dd32d46`, mas sem registro
   explícito nomeando a família `pretooluse_golden`.
7. F1-3-3 crit.2 — **portabilidade real em 3+ CLIs com transcript**: a ativação foi medida
   por juízes lendo os `description` ("como o router de um CLI vê"), não por CLIs reais
   (Codex/Gemini) executando o cenário-teste. Vale rodar ao menos 1 espelho real.
8. F1-3-3 crit.4 — A/B "com skills vs sem" melhora o score: desbloqueado pela F1-3-2; não rodado.
9. F1-3-4 crit.3 — **breaker 2-falhas-do-mesmo-item** não exercitado (nota de rigor do
   próprio gate: nenhum item falhou 2×).
10. F1-3-4 crit.4 — **anti-race `parents:`** não exercitado: o cenário real teve **adoção 0%
    do plan.md** (o líder coordenou por briefing+handoff direto — funcionou, mas o mecanismo
    de dependências estruturadas que a skill prega como regra de ouro ficou sem uso autônomo).
    Sinal de produto a observar: ou a doutrina do plano precisa de reforço, ou o handoff
    direto é o caminho natural em times pequenos — hoje a skill e o comportamento real divergem.
11. F1-3-4 — **revisão cruzada do Arquiteto** (risco 7: a skill não pode ensinar a contornar
    o ADR 0007) citada na entrega como condição de fechamento — sem registro de ter ocorrido.
12. **ACHADO-2 do gate** (produto, MEDIA): descasamento lifecycle×prompt-real manda entrega
    p/ DLQ em vez de `MessageRetained{target_busy}` quando a TUI está em turno longo —
    carry-forward aberto.
13. **Repack do .app** (.entrega-achado1): o fundador testa `dist/Lina.app` — o fix do
    ACHADO-1 só chega à tela dele após rebuild + `make-app.sh`. Pré-requisito operacional da
    validação de tela (item 1).
14. **ADR 0019 — thresholds são hipóteses calibráveis** (2min/3/6 amostras): validar contra a
    baseline real do gate formal da F1-0 antes de considerar definitivos (limite explícito do ADR).

## §5 — Escopo do verificado (honestidade do relato)

- **Verifiquei por leitura integral:** os 14 docs/skills listados no cabeçalho.
- **Verifiquei no código (grep/leitura dirigida):** os pontos citados com arquivo:linha em §1/§2/§4.
- **NÃO verifiquei:** a ponta `SpawnConfirm→admit_node` no app (parei no enum do bin —
  R2 está formulada como "não provado end-to-end", não como "não existe"); o conteúdo das
  entregas F1-3-4/5/7 além dos vereditos/ressalvas grepados; os 283 testes do `dd32d46`
  (citados do doc do gate, não re-rodados — sou read-only e os gates de suite são do Maestro).
- Fonte 13.x (vault) não consultada — as stories carregam o "Por quê" embutido e o despacho
  desta lente aponta os docs do repo; nenhuma contradição interna exigiu ir à fonte.

---

**Uma frase:** a F1-3 construiu MECANISMO para os 5 fios — rubrica que transfere julgamento,
loop com freios, retro que não pode aplicar, spawn que não confia em campo de agente — e o
que falta é fiação (autonomia em prod, seam da tela) e validação declarada (tela do fundador,
A/B, breaker 2×), não visão.
