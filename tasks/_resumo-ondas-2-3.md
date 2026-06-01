# Resumo executável — Ondas 2 & 3 (Épico Fase 0 / Lina Space)

> Síntese densa para distribuição de tarefas pelo Master Maestro. Fonte: `32 - Epico Fase 0 (MVP)` (vault Debriefing Vibe Coding). Sem reler o épico inteiro.
> **Status do time agora:** fechando Onda 2; Onda 3 ainda não começou.
> **Legenda:** `[FEITO]` · `[EM CURSO]` · `[FALTA]`.

---

## 1) ONDA 2 — Render GPU + Walking Skeleton (Trilho C / costura A+C)

**Meta da onda:** dissolver o teto de ~16 contextos WebGL (1 `wgpu::Device`/`Surface` p/ N grids) + primeiro corte vertical PTY→pixel com A2A pulsante e evento persistido, num só SO.
**Infra transversal:** ponte `UiHost`↔`gpui` (W0-11 + impl `GpuiHost`) — `[FEITO]`, sustenta toda a W2.

| Story | Tam | Dep (intra-onda) | Critério de aceite (1 linha) | Status |
|---|---|---|---|---|
| **W2-1** Pipeline de render GPU (`CellInstance` instanciado, 2 atlas gray/color, 3 passes Ghostty, damage+culling) | L | — (core/`VtBackend` da W1) | 1 painel por ANSI-fixture renderiza 1×/2×; digitar 1 linha re-upa **só** `dirty_rows` (assert no `write_buffer`); FPS estável | **[FEITO]** ("render usável") |
| **W2-2** Canvas scene-graph 2D retido (Vello, matriz de câmera, índice espacial, z-order) | L | W2-1 | ~50 nós: pan/zoom culla fora-de-viewport (draw-calls caem); `hit_test` retorna `nodeId`; maior `z` recebe o hit | **[EM CURSO]** add/remove nós (LLM Eng) · **[FALTA]** pan/zoom + culling/hit-test/z-order |
| **W2-3** Composição de N nós-terminal numa surface (1 render pass/painel, viewport+scissor, atlas compartilhado, estado `SUSPENSO`) | M | W2-1, W2-2 | 4 painéis distintos simultâneos sem ctx-perdido; arrastar p/ fora → `SUSPENSO` (0 draw-calls) com grid ainda vivo; voltar re-renderiza | **[FEITO]** ("todo terminal é shell real") |
| **W2-4** Texto correto + IME via winit (`cosmic-text`/HarfRust, grapheme UAX#29, emoji COLRv1, ligaturas, IME preedit/commit) | L | W2-1 | golden pixel-a-pixel: emoji-família = **2 células** e **colorido**, acento pt-br em 1 célula; IME compõe e **nada vai ao PTY até `Commit`** | **[EM CURSO]** seleção + mouse SGR no `lina-vt` (Arquiteto) · **[FALTA]** IME, grapheme-width, emoji COLRv1, ligaturas, corpus golden |
| **W2-5** Walking skeleton vertical (2 `claude` vivos no canvas + A2A faseado + pulso + 1 evento persistido) | L | W2-1, W2-2, W2-3, W2-4 | num só SO: `lina ask A→B` submete e B responde; pulso anima no nó B; `reply` volta a A; evento sobrevive a `kill -9` | **[FEITO]** (esqueleto roda) — falta robustez do gate, ver abaixo |

### GATE DE SAÍDA W2 (= **GATE DE PRAZO**, ~1 mês, 1 SO: Mac OU Linux)
Canvas c/ 2 nós-terminal reais rodando Claude Code · 1 A2A injetada **faseada** (bracketed-paste → `submit_delay` → Enter `0x0D` separado, payload sanitizado de `ESC[201~`) com **pulso visível no nó-alvo** · 1 evento persistido que **sobrevive a `kill -9`**. Se não fechar em ~1 mês → reabrir roadmap/framework (gatilho gpui→Slint).

### O que FALTA exatamente para o GATE W2 fechar
1. **W2-2 — pan/zoom + culling + hit-test + z-order** completos. *(pan/zoom é o gap explícito; é pré-req do "canvas como lar".)*
2. **W2-4 — cauda de corretude de texto:** terminar **seleção + mouse SGR** (em curso) **e entregar** IME via winit, largura por grapheme-cluster (UAX#29), emoji COLRv1 c/ override por codepoint, ligaturas/Nerd/Powerline, **corpus golden-render versionado**.
3. **add/remove de nós** no canvas (em curso, LLM Eng) — fechar gestão dinâmica de nós na cena (W2-2/W2-3).
4. **Robustecer o gate ponta-a-ponta:** garantir que pulso A→B é **evento real do pub/sub in-process** (não mock), que o fim-de-resposta fecha por `idle (damage)+prompt_regex > timeout`, e que a persistência (`rusqlite` WAL + JSONL) **sobrevive medido** a `kill -9` — tudo coeso num só SO. Esqueleto rodar ≠ gate fechado.

> **Nota:** o walking skeleton já roda, mas o GATE W2 só fecha com **W2-2 (pan/zoom)** + **W2-4 (texto correto/IME)** concluídos e a robustez observável acima.

---

## 2) ONDA 3 — Orquestração / Camada de IA (Trilho B)

**Meta da onda:** o cérebro distribuído — N terminais se descobrem pelo nome, se apresentam, leem o plano e cooperam **sozinhos no turno 0**, dentro de guardrails determinísticos (contador/grafo/timer, **nunca LLM**) contra loop, deadlock e explosão de delegação. O app nunca chama um LLM.

| Story | Tam | Dep (intra-onda) | Critério de aceite (1 linha) | Crate/arquivo |
|---|---|---|---|---|
| **W3-1** Role-discovery por nome (5 redes lendo registry YAML, puro Rust, zero LLM) | M | — | matriz passa; `@Arquiteto`-entrada → **ARQUITETO** (não MAESTRO); `terminal 3` → DEVELOPER+`needs_confirmation` | crate **`lina-role-discovery`** (isolada) |
| **W3-2** Bootstrap turno-0 (system message 8 blocos → `CLAUDE.md`/`AGENTS.md`/`GEMINI.md` + hook `SessionStart`) | M | W3-1 | add `@Dev Backend` → `CLAUDE.md` c/ 8 blocos e `role=BACKEND`; `whoami` lista colegas reais; renomear reescreve sem reiniciar | doutrina (arquivos) + renderer |
| **W3-3** Skill A2A (reconhece `[LINA::MSG]`/`[LINA::HANDSHAKE]` vs humano; responde no formato `expected:`; antídoto de eco) | M | W3-2 | injetar `[LINA::MSG] … expected:"…"` → produz artefato no formato e **não ecoa** o bloco; texto sem sentinela = humano | skill **`lina-agent-bus`** (markdown) |
| **W3-4** Spawn preguiçoso + handshake por arquivo (`agents.json`, supervisor = escritor único) | M | W3-2, W3-3 | 4 papéis → só **2 PTYs vivos** (`@Maestro`+owner item-1); `handoff` sobe o nó on-demand; **0 broadcast** no boot | **supervisor** (`lina-core`) |
| **W3-5** `plan.md` (parsing rígido + supervisor escritor único, event-sourced) | M | W3-4 | `plan claim T2` muda owner/status + emite `plan.claim` no `log.jsonl`; 2 claims concorrentes → 1 vence; reconstrói do log | **supervisor** (`lina-core`) + `plan.md` |
| **W3-6** Autonomia + gate DURO de ação irreversível (matriz manual/assistido/autônomo; `PreToolUse`+wrapper shell) | M | W3-3, W3-5 | `manual` recusa `handoff` localmente; `autônomo` intercepta `git push` e pausa; payload `ESC[201~` neutralizado | binário **`lina`** + hooks |
| **W3-7** Guardrails P0 (`root_cause_id`, `delegation_budget=8`, `fanout_max=3`, `max_depth=4`, wait-for graph, anti-loop por grafo de eventos) | M | W3-4, W3-5, W3-6 | 9 handoffs mesmo `root_cause` → 9º recusado; `ask --await` em deadlock recusado na hora; ciclo forjado barrado pelo grafo | **supervisor** (`lina-core`) sobre `log.jsonl` |

### Mapa de dependências internas (DAG)
```
W3-1 ─→ W3-2 ─→ W3-3 ─┬─────────────→ W3-6 ─→ W3-7
                       └─→ W3-4 ─→ W3-5 ─┘        ↑
                                  └──────→ W3-7 ──┘
```
- **W3-6** ← W3-3 + W3-5 · **W3-7** ← W3-4 + W3-5 + W3-6 (sorvedouro / gate da onda).

### Paralelizável vs Sequencial
- **PARALELIZÁVEL desde o dia 0:** **W3-1** (`lina-role-discovery`, crate isolada, puro Rust, sem deps) — arranca já, em paralelo ao fechamento da W2.
- **PARALELIZÁVEL após W3-2:** **W3-3** é **conteúdo de skill (markdown/doutrina)**, arquivos distintos do código do supervisor — pode ser escrita em paralelo ao esqueleto de **W3-4** desde que o **contrato do envelope/sentinela** (fixado em W3-2 + W0-4) esteja congelado; integra-se no fim. **W3-2** (doutrina/templates) também é arquivo, não código de supervisor.
- **SEQUENCIAL (mesma cadeia `lina-core`/supervisor + `log.jsonl` + `plan.md`):** **W3-4 → W3-5 → W3-7**, com **W3-6** pendurado em W3-5. Tocam o mesmo estado do supervisor (roster, mailbox, locks, event log) → **não paralelizar entre si** (risco de corrupção concorrente; o supervisor é escritor único por design).
- **Caminho crítico:** `W3-1 → W3-2 → W3-3 → W3-4 → W3-5 → W3-6 → W3-7`. Encurta-se rodando **W3-1 já** e **W3-2/W3-3 (arquivos)** em paralelo ao supervisor.

### GATE DE SAÍDA W3 (validável **headless** via `.lina/events/log.jsonl`)
2 agentes num Espaço com preset: supervisor sobe `@Maestro`+owner item-1 (W3-4) · ambos resolvem papel pelo nome (W3-1) + bootstrap 8 blocos (W3-2) · `lina handshake` por arquivo e se reconhecem (W3-3/W3-4) · um roda `lina plan claim` (W3-5) — **tudo SEM instrução de orquestração do usuário** · guardrails (W3-7) + gate de autonomia (W3-6) provam bloqueio de loop/deadlock/budget. Sequência esperada no log: `agent.joined → handshake → plan.claim`, sem evento originado em input humano além do turno-0.

---

## 3) Pré-requisitos cross-onda para INICIAR a Onda 3

A Onda 3 (Trilho B) depende do **core W0**, **não** do render W2 → seus gates são headless. Pode arrancar **em paralelo** ao fechamento do W2 (LLM Eng começa W3-1 enquanto Arquiteto/UX fecham W2-2/W2-4).

1. **W0-4 Workspace Bus / Supervisor** pronto — `NodeRegistry`, `MailQueues` serial, presença, locks, **wait-for-graph**. É a base que W3-4/W3-5/W3-7 estendem. *(Provavelmente já pronto: o walking skeleton W2-5 já o exercita.)*
2. **⚠️ Envelope A2A canônico versionado (definido em W0-4)** — `id · root_cause_id · from · to · intent · hops · await · ttl · trace · ts` (campos opcionais até o supervisor preenchê-los). **Risco apontado pelo crítico:** se o envelope foi entregue só com `correlation_id/ttl/trace`, então **W3-3 (`hops`)** e **W3-7 (`root_cause_id`)** herdam contrato fragmentado. **Conferir/consolidar ANTES de iniciar W3-3/W3-7**; o aceite de W3-7 deve assertar o envelope final num teste de serialização.
3. **W0-5 Event Store** (SQLite WAL + JSONL + snapshots) — `plan.md` event-sourced (W3-5) e `log.jsonl` (W3-4/W3-7) assentam aqui. **W0-6** (recuperação) idem.
4. **W0-8 CLI Profiles** + **W0-9 entrega A2A faseada** + **W0-10 fim-de-resposta** — a skill A2A (W3-3) e o gate de roteamento (W3-6) dependem desses contratos. *(Já exercitados pelo walking skeleton.)*
5. **W0-7 Secret Vault** (keyring por SO) — token-por-nó efêmero do bus.
6. **Gate W0 headless** confirmado (`lina-core --headless-gate`) — a Onda 3 valida-se sobre o core agnóstico; não exige a UI da W2.

> **Acionável p/ o Maestro:** o **gate W2 não bloqueia os gates internos da Onda 3**. Distribua: (a) **LLM Eng → W3-1 já** (crate isolada) e depois W3-2/W3-3; (b) **Arquiteto/UX → fecha W2-2 (pan/zoom) + W2-4 (texto/IME) + add/remove nós** para travar o GATE W2. A convergência Onda 3↔Onda 2 só é exigida na **Onda 4** (W4-2 ← W3-1+W2-3; W4-3 pulso ← W0-4+W3-3).

PRONTO
