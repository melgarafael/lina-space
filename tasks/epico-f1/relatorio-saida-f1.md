# RELATÓRIO DE SAÍDA — Épico F1 (Walking Skeleton da Lina Space)

> **Função deste documento:** é o que o fundador percorre no **gate final** para decidir se o
> épico F1 SAI. Declara a saída com **evidência real linkada** — não com adjetivos. Regra dura:
> **zero invenção; todo número aponta o arquivo-fonte real** (no mesmo diretório `tasks/epico-f1/`,
> salvo path absoluto). Onde algo ainda falta, está no **§4 Checklist de saída** — honesto, não
> maquiado.
>
> **Estado em 2026-06-11:** as ondas auditáveis passaram (conselhos com 0 FALHA, red-teams 0 ALTA,
> suítes verdes). O que segura a DECLARAÇÃO formal é o lote final do §4 (red-team de saída · run
> CI 3-SO verde linkado · telas do fundador · medição F1-5 · 2º output criativo do A/B).
> Padrão de todo gate desta fase: **"declara após a tela"** (decisão 2026-06-06).

---

## §1 — Mapa de gates por onda (com link à evidência)

Cada onda tem um critério de saída próprio (ver `ondas-0-1.md`, `ondas-2-4.md`, `onda-3.md`,
`ondas-5-6.md`). O veredito abaixo é o ESTADO AUDITADO; a coluna "Trava de declaração" é o que
ainda exige tela/run real (rastreado no §4).

| Onda | Tema | Veredito auditado | Evidência (arquivo-fonte) | Trava de declaração |
|---|---|---|---|---|
| **F1-0** | Bring-up / baseline | PASS (padrão "declara após a tela") | `baseline-f1-0.md` · gate E2E `onda0_exit_gate_end_to_end` (`gate_onda0.rs`) | — (fechado) |
| **F1-1** | Observabilidade e cognição do trabalho | **PASS CONDICIONAL** — 0 FALHA no conselho (3×OK + 1×OK-c/-ressalvas) · red-team **0 ALTA** | `conselho-f11-consolidado.md` · `redteam-gate-f1-1.md` (HEAD `2de17cd`) | AC-0021.7 na tela + render do badge EoL Gemini |
| **F1-2** | Reinject / superfície de aprovação | PASS (dobrado no conselho F1-1) — furo **A3 corrigido** `f6db7e3` (reinject vira fila em-processo, sem drop-zone de FS) | `redteam-gate-f1-1.md` §A3 · `.entrega-f1-2-6.md` · `roteiro-f1-2-6.md` | (coberto pela tela do F1-1) |
| **F1-3** | Inteligência da Lina (orquestração) | **DECLARADO** — conselho 0 FALHA (4×OK-c/-ressalvas) · red-team spawn **0 ALTA** · **fundador validou na tela** 2026-06-10 | `conselho-f13-consolidado.md` · `gate-f13-teste-autonomo.md` (92 seqs de log) · `redteam-spawn-f1-3-6.md` (commit `76a3ccd`) | — (fechado) |
| **F1-4** | UX / copy / acessibilidade | Implementado + auditado | `ux-flows.md` · `copy-f1-4.md` · `.entrega-f1-4-1.md` · `spike-a11y-roteiro.md` · `roteiro-tela-consolidado.md` | telas do fundador (roteiro consolidado) |
| **F1-5** | Performance / durabilidade do scrollback | Código + suíte verdes (**381/381**); sonda `[PROF]` pronta | `.entrega-f1-5-6-9.md` · `.entrega-f1-5-1.md` · `prof-baseline.md` | **medição final** (números saem da sessão de tela Maestro+fundador) |
| **F1-6** | Gate de saída do épico (CI 3-SO · portabilidade · anti-slop) | CI 3-SO triado (`core-windows` promovido a gate bloqueante); amostra anti-slop = §3 deste relatório | `ci-3so-triagem.md` · `.entrega-f1-6-1-2.md` · `.entrega-f1-6-3.md` · `.entrega-f1-6-4.md` | run real linkado + red-team final + esta amostra |

**Repack do app testável pelo fundador:** `/Users/rafaelmelgaco/einstein workspace/lina-space/dist/Lina.app`
+ `dist/Lina-0.1.0.dmg` (2026-06-11 11:24) — o fundador testa o `.app`, não o dev build
(item 9 do `conselho-f13-consolidado.md`, ✅ fechado).

---

## §2 — Conselhos de gate (as 4 lentes independentes por onda)

O protocolo do épico ("Como executar") manda auditar cada onda com **4 lentes read-only por
auditores que NÃO construíram a onda**, rodadas via Workflow + re-derivação manual no código
(finder infla nit→ALTA; verificador otimiza — re-derivação é obrigatória antes de assinar).

- **F1-1** (`conselho-f11-consolidado.md`, HEAD `2de17cd`): **ZERO FALHA · 0 ALTA.** Lentes
  Visão/Fios · ADR↔Código · Segurança (5 invariantes VALEM) · Pesquisa (OK-c/-ressalvas).
  Achados MEDIA→backlog nomeados em `redteam-gate-f1-1.md` (M2/M4/M6/M7).
- **F1-3** (`conselho-f13-consolidado.md`, HEAD `09876a4`): **ZERO FALHA · 0 ALTA novos** (4×
  OK-c/-ressalvas) → **GATE DECLARADO**. Evidência acumulada: 7/7 stories no código
  (`7d2c198`→`988d1c7`); cenário real autônomo (`gate-f13-teste-autonomo.md`); worker INDUZIDO
  a desviar/travar detectado+corrigido; **validação do fundador na tela** 2026-06-10.

**Backlog pós-F1 nomeado, com dono** (do `conselho-f13-consolidado.md` §Fechamento): breaker
2-falhas-POR-ITEM como mecanismo+evento auditável; `parents:` com enforcement no claim;
auditoria do conflito de claim ao log; `SkillPinned`/`absorbed_into` ratificados para F2.
**Não bloqueiam a saída de F1** — são contrato comportamental de skill, não regressão de core.

---

## §3 — AMOSTRA ANTI-SLOP (gate F1-6, item 4)

> **Critério literal do gate:** "amostra de **≥5 outputs reais de ≥2 CLIs** avaliada pela rubrica
> anti-slop, score registrado". A rubrica é `lina-cold-review/references/rubrica.md` (limiar de
> PASS = 80; marcadores DUROS reprovam: DES-4 direção estética não-declarada, COP-2 placeholder
> entregue, etc.).

### 3.1 — CLI nº 1 (Claude): 12 outputs reais, avaliados e pontuados

Fonte: **`ab-doutrina-resultado.md`** (A/B CEGO da doutrina gênio-criativo, F1-3-1). Protocolo:
3 tarefas criativas (hero de landing · e-mail de lançamento · naming+manifesto) × 2 seeds × 2
braços (**COM** a doutrina `assets/lina-doctrine/CLAUDE.md` lida antes vs **SEM**) = **12
geradores independentes**; **6 juízes CEGOS** (ordem X/Y alternada por índice) pela rubrica.
Workflow `ab-cego-doutrina-f1-3-1` (run `wf_196801af-b66`, 18 agentes).

**Placar: COM 5 × 1 SEM (0 empates).** Os 12 outputs, em 6 pares pontuados:

| Par | Vencedor | Score COM | Score SEM | Marcador duro no perdedor |
|---|---|---:|---:|---|
| landing/0 | **COM** | 90 | 72 | DES-4 ALTA (sem direção estética declarada) → FAIL |
| landing/1 | **COM** | 92 | 62 | DES-4 ALTA + entrega truncada → FAIL |
| copy/0 | **COM** | 93 | 74 | COP-2 ALTA (placeholder "[Seu nome]" como final) → FAIL |
| copy/1 | **COM** | 96 | 91 | — (ambos PASS; COM mais específico/comprometido) |
| naming/0 | **COM** | 94 | 86 | — (ambos PASS; SEM com MÉDIA de jargão vazando) |
| naming/1 | **SEM** | 88 | 92 | — (ambos PASS; única vitória do SEM, por menor ritmo-de-template) |

**Médias: COM ≈ 92,2 · SEM ≈ 79,5** (o braço SEM, na média, nem cruza o limiar 80 da rubrica).

**O que o dado prova** (`ab-doutrina-resultado.md` §Leitura): a doutrina **elimina os marcadores
DUROS** — 0 violação ALTA em 6/6 outputs COM; o braço SEM **reprovou na rubrica em 3/6** (DES-4
×2, COP-2 ×1). Não é ganho de "gosto": é a diferença entre PASSAR e FALHAR o gate de qualidade.
A derrota única (naming/1, por cadência anafórica empilhada) é honesta — a doutrina não imuniza
contra maneirismo; vira refinamento v2 sugerido ao `lina retro` (**sugere-nunca-aplica**), não
bloqueio. **Item 6 do conselho F1-3: ✅ FECHADO.**

> A amostra **já satisfaz o piso "≥5 outputs reais avaliados"** com folga (12 outputs Claude,
> todos com score pela própria rubrica do produto). O que falta para o piso "**≥2 CLIs**" é o §3.2.

### 3.2 — CLI nº 2: slot NOMEADO, plano HONESTO (não fingido)

O 2º CLI **não tem output criativo pontuável ainda — e isto está registrado, não disfarçado.**
A portabilidade das skills entre CLIs FOI testada e fechada no nível possível; o que resta é
**1 tarefa criativa rodada de ponta a ponta** num 2º CLI.

**O que JÁ existe (portabilidade de ATIVAÇÃO, não de output)** — `conselho-f13-consolidado.md`
item 7:
- **Bug REAL achado e corrigido:** o Codex rejeitava as 11 skills (descriptions > 1024 chars) →
  corrigido em `63d4e6b` + teste guardião.
- **Ativação cega re-validada pós-encurtamento: 66/66 (100%)** — 3 juízes cegos × 22 frases
  (workflow `wf_e5334b6f`).

**O que FALTA (output criativo num 2º CLI) e por quê — transcripts reais em `/tmp/lina-3cli/`:**
- **Codex** (`/tmp/lina-3cli/codex-transcript.txt`): a sessão chegou a carregar (gpt-5.5,
  reasoning xhigh) e recebeu o prompt de cold-review, mas **bateu o limite de uso da conta
  OpenAI ANTES de gerar o output criativo** (`ERROR: You've hit your usage limit … try again at
  Jul 9th, 2026`; `CODEX_EXIT=1`). É **limitação de AMBIENTE, não de produto** — o bug de skills
  (o risco real de portabilidade) já foi pego e corrigido.
- **Gemini** (`/tmp/lina-3cli/gemini-transcript.txt`): bloqueado por **auth ausente**
  (`Please set an Auth method … GEMINI_API_KEY`; `GEMINI_EXIT=41`) — não autenticado nesta máquina.

**Plano honesto para fechar o piso "≥2 CLIs" (o slot fica NOMEADO, não fingido):**
1. **Re-rodar 1 tarefa criativa no Codex quando a conta voltar — 09/jul/2026** (data do próprio
   erro), avaliando o output pela mesma rubrica; **OU**
2. **Usar o Gemini se o fundador autenticar** (`GEMINI_API_KEY`/Vertex/GCA) — desbloqueia o
   `GEMINI_EXIT=41` e roda a mesma tarefa criativa.

Qualquer um dos dois preenche o slot com **1 output real pontuado**, elevando a amostra de "12
outputs de 1 CLI" para "≥2 CLIs" e fechando o item 4 do gate F1-6 por completo.

---

## §4 — CHECKLIST DE SAÍDA (o que falta para DECLARAR o épico F1)

Tudo o que é auditável passou. A declaração formal espera este lote — cada item com fonte e dono.

- [ ] **Red-team FINAL de saída: 0 ALTA.** Os red-teams por onda já estão 0 ALTA
      (`redteam-gate-f1-1.md` · `redteam-spawn-f1-3-6.md`); falta a passada adversarial final
      sobre o épico inteiro no HEAD de saída, re-derivada no código. **Dono:** Bug Finder / Revisor
      (coordenação do Maestro).
- [ ] **Run CI 3-SO verde LINKADO.** A triagem está pronta e `core-windows` foi promovido a gate
      bloqueante (`ci-3so-triagem.md` §2). Falta o **run real no GitHub** (o push é do Maestro):
      5 jobs verdes com `core-windows` EXECUTANDO (`running N tests`/`test result: ok`, não só
      `Compiling`), **3 runs consecutivos verdes** (anti-flake), e o **link do run colado aqui**.
      Checklist completo em `ci-3so-triagem.md` §4. Baseline local já provado: **598 passed / 0
      failed / 1 ignored**, clippy mac+win(cross) e fmt limpos (`ci-3so-triagem.md` §6).
- [ ] **Telas do fundador** (padrão "declara após a tela"):
  - [ ] **AC-0021.7** — aprovar pelo toast destrava um Claude real bloqueado em y/n (audit
        `PermissionResolved→ApprovalInjected` no log) — `conselho-f11-consolidado.md`.
  - [ ] **Render do badge EoL Gemini** visível no modal/dashboard (copy aprovada; falta o render).
  - [ ] **Roteiro de tela consolidado** percorrido (`roteiro-tela-consolidado.md`).
- [ ] **Medição final F1-5.** A sonda `[PROF]` decomposta e o gerador de carga estão prontos
      (`.entrega-f1-5-1.md`); os **números oficiais saem da sessão de tela Maestro+fundador**
      ("Dados = sessão de tela", `prof-baseline.md` §rodapé). Medição oficial = macOS.
- [ ] **2º output criativo do A/B (§3.2).** Re-rodar 1 tarefa criativa no Codex (≥ 09/jul) OU no
      Gemini (se o fundador autenticar) e pontuar pela rubrica — fecha o piso "≥2 CLIs" do item 4.

**Itens que NÃO bloqueiam a saída** (registrados, com dono pós-F1): backlog do
`conselho-f13-consolidado.md` (breaker-por-item, `parents:` com enforcement, auditoria de
conflito de claim, `SkillPinned`→F2); MEDIA→backlog dos red-teams (M2/M4/M6/M7 de F1-1; M1/M3/B1
do spawn); 14 testes `pendente-windows` gateados `#[cfg(unix)]` com equivalente Windows nomeado
(`ci-3so-triagem.md` §1b) — o mecanismo segue coberto a cada push no macOS/Linux + os ~540 que
executam no Windows.

---

## §5 — Veredito

**F1 está pronto para o gate final, não declarado ainda.** A engenharia auditável passou com
folga: conselhos 0 FALHA, red-teams 0 ALTA, suítes verdes (598/0/1 no workspace; 381/381 no
core do scrollback), a doutrina anti-slop **comprovadamente** melhora o output de forma medível
(92,2 × 79,5; reprova o piso ruim em 3/6 sem ela), e o app está empacotado para o fundador
(`dist/Lina.app`). A declaração formal é uma **decisão do fundador no gate**, condicionada ao
lote do §4 — que é curto, nomeado e honesto: red-team final, o run CI linkado, as telas, a
medição F1-5 e o 2º output criativo. **Quando o §4 fechar, F1 sai.**

---

*Esqueleto + amostra anti-slop montados pelo Designer (despacho `r3-designer`, id `relatorio-epico`),
2026-06-11. Todo número aponta o arquivo-fonte; nada inventado. O Maestro valida e commita.*
