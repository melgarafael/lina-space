# Entrega Arquiteto — DAG, Portas e ADRs do Épico F1

> **Papel:** Arquiteto (guardião estrutural do épico — doc 34). **Data:** 2026-06-06.
> **Fontes lidas:** repo `CLAUDE.md` · ADRs 0001–0007 (`docs/adr/`) · vault `01 - Visao de Produto e Norte de Continuidade` (§3+§4 inteiros) · `32 - Epico Fase 0` (linhas 26–53) · `13 - Pesquisa Fase 1` (índice/vereditos) · 13.2, 13.3, 13.4, 13.5, 13.6, 13.7, 13.8, 13.9, 13.10, 13.11, 13.12, 13.13, 13.14, 13.15, 13.16 · `30 - SPEC Mestre` §8/§4 · `_HANDOFF` §7 (PF-1/PF-2, 5 MEDIA, teto de render) · doc vivo "Acrescentar ainda nessa versão do Lina" (14 itens).
> **Critério de julgamento:** doc 01 §3 (portas) + §4 (invariantes 1–7). Regra de ouro: *nenhuma decisão fecha porta da §3 sem registro; construir nunca justifica quebrar invariante da §4* (doc 01, cabeçalho).
> **🔄 Atualização batch 2 (2026-06-06, decisões do fundador via Maestro):** Dúvidas 1, 2 e 6 RESOLVIDAS e registradas em §c.1; ADR operacional redigido em §c.2; baseline P0/P1 promovido a parte FORMAL do gate F1-0; ordenação dos ADRs bloqueantes confirmada.

---

## a) Sequência e DAG das ondas

### a.1 Ordem recomendada (com justificativa de de-risk)

| # | Onda | Por que nesta posição (de-risk) |
|---|------|--------------------------------|
| 1 | **F1-0 Coordenação Confiável** | É o bloqueador de tudo: P0–P4 são os bugs **observados em 14h de uso real** e a pesquisa impôs ordem interna obrigatória — P0 (ready-state) → P1 (idle-guard) → P2 (handoff/check) → P3 (plano, pode paralelo) → P4 (lifecycle) (13.2 §P0–P4; 13 índice achado 4). Idempotência + dead-letter + heartbeat são **mandatórios antes** de observabilidade (13.11: "🔴 event log basta — refutado"). Sem F1-0, a F1-1 observa um sistema instável e a F1-3 orquestra sobre areia. ⚠️ **Mandato de baseline:** reproduzir P0/P1 com 5–8 Claudes reais ANTES do fix — sem baseline não há como declarar "resolvido" (13.2 §Validação prática). **[Batch 2] Este critério é parte FORMAL do gate de saída da F1-0** (decisão do Maestro; o Spec Writer está integrando em `ondas-0-1.md`). |
| 2 | **F1-2 Experiência — *só a story de design system*, antecipada e em paralelo à F1-0** | O design system (tokens, Theme structs, ColorScaleSet) é **pré-requisito de TODA UI nova** (13.3 [P0]: "Design System deve ser *done first* — todas UI posteriores herdam dele"). Roda em paralelo à F1-0 porque a fronteira é disjunta (app/lina-gpui vs lina-core). O resto da F1-2 (modal, a11y polish) vem depois da detecção de CLI. |
| 3 | **F1-1 Observabilidade e Cognição** | Depende da state machine da F1-0 (a fila de atenção precisa do estado `Blocked` vindo do lifecycle — ver a.2) e do design system da F1-2 (toast/badge/UI de custo). A sub-frente **detecção de CLI** é disjunta (lina-cli-profiles + cli_discovery) e pode começar cedo, em paralelo. A story de **aprovação remota** fica bloqueada pelo ADR 0009 (segurança — ver §c). |
| 4 | **F1-3 Inteligência da Lina** | Consome F1-0 (verbos `lina handoff`/`check` para a skill de orquestração — 13.2 §P2) e F1-1 (detecção de CLI para o agente-cria-terminal; custo para decisões). A sub-frente **doutrina/skills anti-slop** é arquivo-novo em `assets/` — pode antecipar em paralelo (13.4: personalidade = arquitetura, 🟢🟢). |
| 5 | **F1-4 Workspaces e PRO** | **Bloqueada por ADR (0010) antes de qualquer código**: multi-workspace sem namespace de trust reabre a porta-futura PF-1 como vulnerabilidade real (handoff §7-C item 7; ADR 0006). A sub-frente **licença** é crate novo, totalmente disjunto — pode rodar em paralelo desde cedo (13.6). |
| 6 | **F1-5 Render-Scale e Scrollback** | "Profiling primeiro" é mandato da pesquisa (13.7: 🔴 "dirty tracking = 3x" refutado; 13 índice achado 9: "gates medidos na tela, nunca número prometido"). A story de profiling pode começar a qualquer momento (instrumentação read-mostly); o **scrollback** (flush idle, retenção, API) é `lina-core/scrollback.rs` — disjunto do router, paralelo à F1-0. Culling/LOD fica **condicionado** à validação de UX (13.7 ADR-material). |
| 7 | **F1-6 Hardening e Saída** | Última por natureza (red-team final caça as **costuras** entre tudo que foi construído — lição handoff §8). O sub-bloco **infra** (CI 3-SO, Windows W5-3, assinatura) depende de hardware/certificados do fundador (handoff §7-B) — deve ser **desacoplado** num sub-bloco "infra-do-fundador" para não travar o épico. As 5 MEDIA têm dono natural dividido: MEDIA-3/4 (scrollback) → F1-5; MEDIA-1/2/5 (webhooks) → F1-6 (ver Dúvida 5). |

### a.2 Pré-requisitos cross-onda (deps backward — ler antes de paralelizar)

No formato do épico da Fase 0 (doc 32 §"Pré-requisitos cross-onda"):

- **F1-0 (P4 lifecycle/state machine)** → é pré-req da **fila de atenção da F1-1**: a fila unifica permissão+custódia e precisa do estado `Blocked` emitido pelo lifecycle; sem ele, a fila é polling cego sobre o grid (13.13 §matriz; 13.2 §P4).
- **F1-0 (verbos `lina handoff`/`lina check` + handoff contract)** → pré-req da **skill de orquestração da F1-3**: a skill instrui agentes a usarem primitivos que precisam existir e estar estáveis (13.2 §P2; 13.11 STORY 2).
- **F1-0 (semântica do `intent` — ADR 0012)** → pré-req da **fila da F1-1** (classificação/exibição por intent) e do **`[security]` nos CLI profiles da F1-6** (13.14: campo `intent` é 🟢 crítico da Fase 1).
- **F1-1 (detecção de CLI — ADR 0008)** → pré-req do **modal da F1-2** (dropdown de CLI pré-configurado + badge "CLI detectado" — itens 3 e 8 do doc vivo; 13.9 lista acionável 6) e do **agente-cria-terminal da F1-3** (precisa saber quais CLIs existem para criar terminal já configurado — 13.9).
- **F1-2 (design system/tokens)** → pré-req de **TODA UI nova**: UI de custo (F1-1), modal (F1-2), switcher e UI de licença (F1-4), restore visível (F1-5) (13.3 [P0]).
- **F1-0 (idempotência+dead-letter — ADR 0014)** → pré-req da **observabilidade da F1-1**: sem dedup/DLQ os eventos observados incluem duplicatas e mensagens zumbis (13.11 §dag).
- **ADR 0009 (security model da injeção remota)** → bloqueia a story **aprovação-remota** da F1-1 (detecção/sinalização podem andar antes; a INJEÇÃO de `y/n` não) (13.13 backlog item 5).
- **ADR 0010 (multi-workspace/trust)** → bloqueia **toda** a F1-4 exceto a licença (PF-1, handoff §7-C).
- **ADR 0013 (durabilidade de scrollback)** → pré-req das stories de scrollback da F1-5 **e** do acesso de agentes ao histórico na F1-3 (API paginada obrigatória — 13.16: "agentes nunca acessam scrollback sem paginação").
- **F1-5 (story de profiling)** → pré-req das demais stories de render da F1-5 (13.7 item 1: bloqueante).
- **F1-5 (`TerminalState::Recovered` + restore visível)** ↔ **F1-2 (UI)**: dependência cruzada herdada de W4-4↔W5-2 — a UI do "(encerrada) / Ver histórico" é da F1-2, o mecanismo é da F1-5; nomear o dono da costura no doc 34 (13.16 §Tese 2).

**Pendências da Fase 0 que são dependências de ENTRADA do épico** (não são stories novas — são fechamentos):
1. **`TokenUsageReported` real no app** (W0-10→app): sem isso o teto de custo "não morde em produção" e a UI de custo da F1-1 nasce vazia (ADR 0005 §Consequências). Fechar no início da F1-1.
2. **Fiação W5-2 no app** (`set_scrollback_store` nunca chamado em `app/lina-gpui/`): o benefício de RAM do W5-2 não está vivo; ligar e re-medir FPS é a primeira tarefa da F1-5 (handoff §7-A).
3. **a11y `set_live`**: bloqueado pelo gpui pinado, NÃO é fail-gate da F1-2 — banner visível + leitura ao focar fecham a onda; auto-anúncio vira story futura com ADR registrando o plano (13.15; memória do repo `gpui-role-status-nao-e-live-region`).
4. **Bloco infra** (push GitHub → CI, Windows W5-3, certs de assinatura): parqueado no fundador (handoff §7-B) — entra na F1-6 como sub-bloco condicionado.

### a.3 Diagrama (DAG das ondas)

```
                       ┌────────────────────────────┐
                       │ ADRs-gate: 0008..0014      │  (escrever ANTES das stories que bloqueiam)
                       └────────────┬───────────────┘
                                    │
  F1-2a (design system) ──────┐     │
        [app, disjunto]       │     │
                              ▼     ▼
  F1-0 (P0→P1→P2→(P3∥)→P4) ━━━━━━━━━━━━━━━━┓
   [lina-core router/mailbox]               ┃ state machine + intent + handoff
        │                                   ▼
        │            F1-1 (detecção CLI ∥ fila de atenção ∥ custo/tokens)
        │             [cli-profiles+discovery | broker/fila | app UI]
        │                                   │
        │                  ┌────────────────┤
        ▼                  ▼                ▼
  F1-5 (profiling → render-stories)   F1-3 (doutrina ∥ skill orq. ∥ agente-cria-terminal)
   [app render + scrollback core]      [assets/skills + bootstrap]
        │                                   │
        └────────────┬──────────────────────┘
                     ▼
  F1-4 (ADR 0010 → multi-workspace + restore + switcher) ∥ (licença: crate novo, desde cedo)
                     │
                     ▼
  F1-6 (5 MEDIA-webhooks + proxy credenciais + assinatura skills + red-team) + [sub-bloco infra-do-fundador: CI 3-SO]
```

### a.4 Paralelização por terminais (fronteiras disjuntas — lição da Fase 0)

A Fase 0 provou que 4 terminais no MESMO crate funcionam **com fronteiras de arquivo disjuntas por dono** (handoff §8: "módulo próprio; lib.rs/Cargo.toml tocados no mínimo; workers não-commitam; Maestro valida de fora"). Mapa proposto para a primeira rodada:

| Terminal | Onda/frente | Território (disjunto) |
|---|---|---|
| T1 | F1-0 | `crates/lina-core/src/{router.rs, mailbox.rs}` + módulo novo `lifecycle.rs` |
| T2 | F1-2a design system | `app/lina-gpui/src/` — arquivos NOVOS (`theme.rs`, `tokens.rs`); `main.rs` só import |
| T3 | F1-1 detecção de CLI | `crates/lina-cli-profiles/` + `lina-core/src/cli_discovery.rs` + módulo novo `cli_detector.rs` |
| T4 | F1-3 doutrina/skills | `crates/lina-bootstrap/assets/` (skills/doutrina — arquivos novos) |
| T5 (rodada 2) | F1-5 scrollback / F1-4 licença | `lina-core/src/scrollback.rs` / crate NOVO `crates/lina-license/` |

**Pontos de colisão nomeados** (não dá para evitar, então protocolizar):
- **`events.rs` (enum `DomainEvent`)**: TODA onda adiciona variantes. Protocolo: variantes são **append-only no fim do enum**, cada onda anexa as suas num bloco comentado com o id da onda, e UM terminal por rodada é o "dono do enum" que consolida (o app só CONSTRÓI variantes, nunca faz match exaustivo — ADR 0001 §2, então append não quebra ninguém).
- **`bridge.rs` (app)**: F1-1 (fila/custo) e F1-5 (render) tocam. Sequenciar, não paralelizar, as stories que o editam.
- **`lib.rs`/`Cargo.toml`**: tocados no mínimo, como na Fase 0.

---

## b) Teste das portas (§3) e invariantes (§4), onda a onda

Pergunta-padrão do norte: *"isto fecha alguma porta da §3? quebra invariante 1–7?"* (doc 01 §3/§5).

### F1-0 Coordenação Confiável
- **Envelope A2A (porta)** — ✅ respeitada por construção: `intent` JÁ está no contrato canônico do W0-4 (`id·root_cause_id·from·to·intent·hops·await·ttl·trace·ts` — doc 32 §Contrato). F1-0 define a **semântica** de um campo reservado; aditivo via `#[serde(default)]`, sem bump (padrão do campo `ref`, ADR 0001 §3). ⚠️ Guarda: `intent` é escrito pelo agente → **nunca decide autorização sozinho** (mesma família do `hops` forjável do ADR 0007; campo do agente só classifica/exibe, gates derivam de fonte não-forjável).
- **Event Store (porta) / invariante 4** — ✅ se a **dead-letter queue for projeção do log** (evento `MessageDeadLettered`, fila reconstrutível por replay), ❌ se for tabela-autoridade paralela. O ADR 0014 deve cravar isso. Dedup **preventivo** (sem logar duplicatas) já é doutrina do ADR 0003 (anti-amplificação) — manter.
- **Invariante 1 (sem LLM próprio)** — ⚠️ tentação real: "LLM que decide Busy/Idle" (13.2 §portas). Heartbeat/idle são **determinísticos** (silêncio de PTY, cycle-count+hash — 13.11), nunca julgamento de modelo. Igualmente: validação de handoff contract é do **router** (escritor único determinístico), nunca delegada ao LLM (13.11 🟢).
- **Bus/Supervisor (porta)** — ✅ lifecycle pendura no Supervisor (é "toda orquestração futura pendura aqui"). Circuit breaker segue o padrão do ADR 0005: **pausa-com-gate, nunca kill**.

### F1-1 Observabilidade e Cognição
- **Invariante 2 (local-first) vs OTel** — ✅ desde que: JSONL local é o **primário** (sempre escrito — 13.5 🔴 inverteu "fallback"); OTel é **overlay opt-in sinalizado, OFF por default** (13.10 🔴 refutou "OTel como pré-requisito de governança" — a governança básica lê estado do mailbox/router). ⚠️ O collector sidecar proposto em 13.5 escuta `0.0.0.0:4317/4318` por default — se adotado, **bind obrigatório em 127.0.0.1** e só quando o usuário ativar. Exposição não-sinalizada quebraria o invariante.
- **Invariante 3 (neutralidade) vs detecção de CLI** — ⚠️ tensão REAL já documentada: `KNOWN_CLIS` hardcoded exige recompilar para descobrir CLI novo (13.9 item 2, `.entrega-w41.md` AVISO 2) — contradiz a âncora "CLI Profiles TOML / novos CLIs sem recompilar". Resolver no ADR 0008 (derivar ids dos TOML carregados). A Camada 2 (process inspection de PID externo) foi 🔴 **refutada** — não construir (13.9).
- **Honestidade de custo (inv. 6)** — JSONL subconta (placeholders em ~75% das entradas — 13.5 🔴): a UI mostra **"estimativa ~$X"**, nunca "custo preciso 100% offline". Promessa não-medida virando meta é o risco 4 (§e).
- **Fila de atenção unificada** — ✅ estende o BrokerPump/custódia existente; precedência custódia > permissão > custom (13.13 item 4) preserva o ADR 0004 (custódia é o gate duro). A **injeção remota de y/n** abre superfície nova de injeção no PTY → ADR 0009 ANTES da story (CVEs reais — ver §c).
- **Gemini→Antigravity** — ✅ é exatamente o caso de uso da porta CLI Profiles: a transição entra como perfil novo (`agy` já está em `KNOWN_CLIS` — 13.9 🟢). Gemini vira "transitional, best-effort"; spike Antigravity pós-estabilização das docs (13.10).

### F1-2 Experiência
- **Invariante 7 (core/shell split)** — ✅ design tokens/Theme vivem no **shell** (app), zero no core. Não toca UiHost.
- **Invariante 3** — ⚠️ o modal criar/editar terminal serializa para `CliProfile` TOML e pede spawn ao Supervisor — usa as portas existentes. **Não** codificar conhecimento de CLI específico no modal (template-driven 3–5 controles + co-piloto — 13.3 [P0]).
- **a11y** — ✅ com honestidade: gpui pinado não expõe `set_live` (13.15 🟢) → a onda fecha com banner visível + `Role::Status` + gate WCAG + reduce-motion; **NÃO afirmar** "conforme ARIA live-region spec" (13.15 🔴). Não fecha porta: patch-no-pin ou upstream ficam documentados em ADR-curto/story futura.
- **Invariante 6** — ✅ é a onda que o serve diretamente (zero jargão, modal simples, remover resíduos de dev — itens 3, 5, 6, 10, 11 do doc vivo).

### F1-3 Inteligência da Lina
- **Invariante 1** — ✅ no fio da navalha, mas respeitado: a "personalidade da Lina" é **arquitetura** (skills compostas + revisor isolado + doutrina em camadas — 13.4 🟢🟢), injetada via Bootstrap turno-0 (porta) no CLI de terceiro. Nenhum LLM/harness próprio. Atenção: NÃO confundir com o A2A Protocol externo (Google/LF) — esse foi 🔴 refutado para a Fase 1 (13.4 §6; 13 índice achado 6); nosso envelope interno segue.
- **Agente-cria-terminal** — ⚠️ três guardas: (1) **NodeId tem autoridade única no Supervisor** — o verbo pede, o Supervisor aloca (lição registrada no repo); (2) criação de terminal consome recursos/$ → coberto pelo teto de custo (ADR 0005) + nível de autonomia; (3) o pedido nasce de campo de agente → o fluxo passa pelo router com policy real (ADR 0006), não por canal lateral.
- **Auto-aprimoramento v0 (Curator-like)** — ⚠️ invariante 4: transições de skill (`unused→stale→archived`) DEVEM ser **eventos no log**, não side-effect de daemon (13.8 §portas). ⚠️ Escopo: o doc vivo lista auto-aprimoramento no **"Futuro"** — o v0 na F1-3 é antecipação consciente; restringir a curadoria de skills com eventos + proposta-com-gate (nunca auto-edição silenciosa). Ver Dúvida 6.
- **Bootstrap turno-0 (porta)** — ✅ é o canal: doutrina, skills e mapa do time entram por ali, como previsto na âncora.

### F1-4 Workspaces e PRO
- **Invariante 5 vs PF-1 — O TESTE DE PORTA MAIS CRÍTICO DO ÉPICO.** Hoje `WorkspaceTrust::from_members(live_member_ids(&sup))` deriva do **roster GLOBAL do Supervisor, sem namespace de Espaço** — "seguro com 1 Espaço/app, perigoso com N" (handoff §7-C item 7; ADR 0006). Multi-workspace **sem** namespacing transforma a porta-futura PF-1 em vulnerabilidade ativa: terminal do workspace A injetando no workspace B com a allow-list dizendo sim. O invariante 5 ("pertencimento = conexão") exige que pertencimento seja **por Espaço**. ADR 0010 obrigatório ANTES de qualquer story. Idem **PF-2** (webhook→inject): com N workspaces o payload externo precisa continuar não-confiável + backstop gate/custódia.
- **Invariante 2 vs licença/PRO** — ✅ se for o desenho do 13.6: chave pública ed25519 embarcada, `license.json` **assinado** (não JSON cru — "sem assinatura, o feature-gating é teatro", 13.6 item 1), validação 100% local, **sem phone-home** (diferencial deliberado — 13.6 🔴 refutou "todo mundo faz offline"; nós seremos a exceção), grace period com **degradação graciosa, nunca travar o app** — e jamais bloquear event log/recovery ("nada se perde" não pode depender de licença). Gating data-driven por nº de workspaces (free=1, PRO=N).
- **Invariante 4** — ✅ workspace restore é projeção do log (a âncora Event Store cobre); switcher é UI sobre a projeção.

### F1-5 Render-Scale e Scrollback
- **Porta PortalEngine/ExternalTextureLayer** — ⚠️ tensão direta: culling+LOD com snapshot de dormentes exige **múltiplos render targets**, e a porta hoje externaliza UMA camada (13.7 §portas; Zed PR #26308 documenta multi-buffers como gargalo). Regra: o redesign da cena para LOD deve **alargar** a interface da porta (multi-target), nunca simplificá-la a ponto de excluir o browser-como-textura ("não simplifique a cena a ponto de excluí-lo" — doc 01 §3). PoC de arquitetura antes de commitar culling.
- **Vereditos 🔴 do render** — profiling-gate obrigatório: "dirty tracking = 3x" refutado; "LOD imperceptível" não provado; "28@40-50fps" é projeção (13.7). Coerente com a lição da Fase 0: o FPS real só apareceu na tela (handoff §7-A: 54fps@4 → 12fps@28).
- **Invariantes 4 e 6 vs suspend/scrollback** — `Suspended` é transição com evento + sinal visível (nunca "painel morto sem explicação"). Scrollback: a retenção de 30 dias **não** fere o invariante 4 porque scrollback **não é estado de domínio** (handoff MEDIA-4: "canvas/A2A/plano/eventos no EventStore síncrono — o 'nada se perde' do domínio vale") — essa distinção vai escrita no ADR 0013. Já o **restore invisível** fere o invariante 6 hoje ("estado sempre salvo *e visível*" — 13.16 🔴): `TerminalState::Recovered` + "Ver histórico" corrigem.
- **Trait VtBackend (porta)** — ✅ o cabo W5-2 já adicionou `take_scrollback` como default-method preservando a porta libghostty (handoff §7-C item 8). Manter o padrão em qualquer extensão.

### F1-6 Hardening e Saída
- **ADR 0004 (custódia) reforçado** — o proxy de credenciais (padrão Agent Vault: segredo nunca chega ao terminal, token efêmero — 13.14 §4) é a evolução natural do tier-3/4 da custódia. Local-first preservado (broker local).
- **Honestidade do isolamento (inv. 6 + comunicação)** — 🔴 a tese "same-uid é suficiente" foi refutada (13.14 §2): documentar como **isolamento processual, não-kernel**, com o leak `/dev/pts` nomeado (L1-3 segue fronteira aberta consciente — ADR 0006 §Limite). Nunca vender "isolado/seguro entre terminais".
- **Assinatura de skills** — complemento, não substituto: allowlist + review + hook PreToolUse deny-by-default são o mecanismo (lição ClawHavoc: 1.400+ skills maliciosos; assinatura sozinha não detecta "assinado E malicioso" — 13.14 §3).
- **Invariante 2 vs MEDIA-1** — o teto-IP global só é problema SE exposto via túnel (caso do nó-Gatilho futuro): o fix (teto deslizante/per-hook) alinha com PF-2 (handoff §7-D).

**Veredito agregado:** nenhuma onda do esqueleto **fecha** porta da §3 nem quebra invariante — DESDE QUE os 5 pontos ⚠️ acima virem ADR antes das stories correspondentes: trust por Espaço (0010), injeção remota (0009), KNOWN_CLIS×inv3 (0008), DLQ-como-projeção (0014), multi-render-target×PortalEngine (0017).

---

## c) ADRs a escrever na Fase 1 (numeração proposta 0008+)

> Convenção herdada: ADR curto, com Contexto/Decisão/Limite explícito/Alternativas rejeitadas (padrão dos 0001–0007). “Antes de” = a story não inicia sem o ADR aceito.

| # | Título | Decisão a tomar | Opções | Recomendação preliminar | Antes de |
|---|--------|-----------------|--------|--------------------------|----------|
| **0008** | **Padrão de detecção de CLI em camadas** | Como o supervisor sabe qual CLI roda em cada PTY; e como resolver `KNOWN_CLIS` hardcoded × invariante 3 | (a) 4 camadas com score; (b) Camada 1 (spawn-time) primária + C3 (session-watch) confirmação + C4 (grid) telemetria, **C2 descartada**; (c) ids derivados dos TOML vs lista compilada | **(b)** + ids derivados dos perfis TOML carregados no boot; terminais externos (se virarem requisito) via `lina register-self`, nunca introspecção de PID (13.9: C2 🔴 refutada — `proc_pidinfo` é self-bound, env var é falseável, caso de uso não está no backlog). Emite `CliDetected{node, cli, confidence, evidence}` | Stories de detecção (F1-1), modal com dropdown de CLI (F1-2), agente-cria-terminal (F1-3) |
| **0009** | **Security model da injeção remota de aprovação (y/n)** | Como clicar "aprovar" no canvas injeta `y\n` num PTY sem race/spoofing | (a) injetar direto; (b) snapshot/validação do estado do VT antes do write + stable ID + idempotência + timeout/SLA; (c) nunca injetar (só focar o terminal) | **(b)** — o risco é fato com CVE (CVE-2024-27936 ANSI spoofing; CVE-2024-32477 tcflush race; GHSA-95cj-3hr2-7j5j do Deno; 13.13 §5), mas a mitigação exata **não é padrão de indústria documentado** — é decisão de engenharia NOSSA, a provar com **teste de race próprio + red-team** (13.13 §ressalvas). Envelope `PermissionEnvelope{session_id, node_id, tool_name, tool_input_hash, idempotency_key}`; fila unificada com precedência custódia > permissão > custom; SLA: não-respondida em N min → escalate/auto-deny. **[Batch 2 — decidido]:** a mitigação por **snapshot-hash do estado do VT** fica registrada como decisão de engenharia do Lina (não importação de padrão); a exigência de **revisão de segurança própria antes da story de injeção** é mantida | Story "aprovação remota" (F1-1). Detecção/toast/fila podem andar antes |
| **0010** | **Multi-workspace e trust por Espaço (fecha PF-1)** | Como N Espaços coexistem sem que a allow-list global vire furo | (a) namespace `workspace_id` no Supervisor/roster e `WorkspaceTrust::from_members(live_members(ws))` por Espaço; (b) um Supervisor por workspace; (c) um processo por workspace | **(a)** — menor impacto, alinhado ao inv. 5 (pertencimento POR Espaço é a fonte da topologia; ADR 0006 já rejeita config estática). Default-deny cross-workspace; PF-2 (webhook→inject) tratado como payload não-confiável + backstop custódia. Red-team cross-workspace é critério de aceite. **[Batch 2 — decidido]:** o texto canônico deste ADR também registra a decisão de ESCOPO da Fase 1 e o **backlog nominal da Fase 2** (ver §c.1) | **TODA** story F1-4 exceto licença; idealmente aceito antes da F1-0 tocar o lifecycle (eventos já workspace-scoped) |
| **0011** | **Formato da licença local (Free/PRO)** | Estrutura, validação e ciclo de vida da licença sem servidor de contas | (a) JSON cru com `tier`; (b) `~/.lina/license.json` **assinado ed25519** `{tier, workspace_limit, entitlements, expiry, signature}` + chave pública embarcada; (c) DRM/node-locking | **(b)** — JSON cru é "feature-gating teatro" (13.6 item 1); node-locking não (arquitetar para suportar depois); grace 7–14d com **degradação graciosa (nunca travar o app, jamais bloquear recovery/event-log)**; gating por nº de workspaces (free=1, PRO=N); emissão via LemonSqueezy/Paddle só como canal (sem API em runtime); batch de chaves p/ alunos; **sem phone-home = diferencial deliberado documentado** (13.6 🔴: JetBrains/Sublime/Raycast fazem phone-home — nós não) | Stories de licença (F1-4) |
| **0012** | **Semântica do campo `intent` no envelope A2A** | Enum de intents, quem carimba, e o que ele PODE decidir | (a) string livre; (b) enum versionado (`ask`/`handoff`/`check`/`broadcast`/`reply`/`permission`…) carimbado no verbo `lina`, com regra dura: **intent classifica/exibe, nunca autoriza** | **(b)** — o campo já está reservado no contrato do W0-4 (doc 32 §Contrato) → preencher é aditivo (`serde(default)`, padrão ADR 0001 §3). Campo escrito por agente não decide autorização (família ADR 0007/`hops`; 13.14 🟢 "campo intent é crítico"); router/fila usam para rotular, gates continuam derivando de binding não-forjável | Verbos handoff/check (F1-0) e fila de atenção (F1-1) |
| **0013** | **Política de durabilidade de scrollback** | Quando flusha, quanto retém, como expõe | (a) status quo (flush por 2000 linhas, Drop-only); (b) + idle-flush 1–2s (thread única no PtyHost) + handler SIGTERM→`flush_all()` + retenção 30d com DELETE+VACUUM + API paginada obrigatória (`search/tail/export` com limit) + `TerminalState::Recovered` visível; (c) compressão zstd já | **(b)** — fecha MEDIA-3/MEDIA-4 (handoff §7-D) e as duas teses 🔴 do 13.16; **escrever a distinção doutrinária**: scrollback = log de conteúdo, NÃO estado de domínio → retenção não fere inv. 4; restore invisível fere inv. 6 (corrigir). zstd só pós-benchmark (estimativa 40–60% não medida) | Stories de scrollback (F1-5); API paginada antes de F1-3 dar acesso a agentes |
| **0014** | **Coordenação confiável: idempotência, dead-letter e heartbeat** | Onde vivem dedup/idempotency/DLQ/heartbeat e o que vira evento | (a) no agente/LLM; (b) **no router (escritor único)**: `idempotency_key` (UUIDv7) por operação com resultado cacheado em retry; DLQ **como projeção do log** (`MessageDeadLettered`); heartbeat determinístico (cycle-count + hash do tail, ~2–3min); circuit breaker N-strikes → **pausa-com-gate** (nunca kill, padrão ADR 0005) | **(b)** — 13.11 🟢 em todos: "router determinístico, LLM alucina"; checkpoints ≠ completion; "local-first não precisa de Temporal — event log + idempotência bastam" (documentar essa decisão implícita). Dedup preventivo sem logar duplicatas (coerente ADR 0003) | Stories de dead-letter/idempotência/heartbeat (F1-0) |
| **0015** | **State machine de lifecycle do nó** | Estados, fonte dos sinais e eventos | (a) auto-report do agente; (b) `Ready→Busy→Idle→Blocked→Dead` com transições derivadas de sinais **determinísticos** (prompt-ready regex, silêncio de PTY, heartbeat, hook de permissão) emitindo `NodeStatusChanged` aditivo | **(b)** — agente pode mentir (13.11 STORY 6 rejeita auto-report); resolve a tensão "NodeAdded sem NodeRemoved" do replay (13.2 §P4) e alimenta `lina check` + fila de atenção | Story P4 (F1-0) e fila de atenção (F1-1) |
| **0016** | **Observabilidade local-first (JSONL primário, OTel opt-in)** | Fonte da verdade de tokens/custo e papel do OTel | (a) OTel obrigatório; (b) JSONL local primário (com honestidade "estimativa ~$X" — undercount documentado) + OTel **opt-in sinalizado OFF-default** + governança básica via estado do mailbox/router (nunca dependente de OTel) + `sessions.db` como projeção derivada; collector (se embutido) **bind 127.0.0.1** | **(b)** — 13.10 🔴 refutou OTel-como-pré-requisito; 13.5 🔴 refutou JSONL-como-fallback (é primário) e a precisão dos números; inv. 2 exige opt-in sinalizado | Stories de tokens/custo/UI (F1-1) |
| **0017** | **Render-scale: profiling-gate, alvo de escala e a porta PortalEngine** | Qual técnica, em que ordem, e qual alvo de painéis | (a) dirty-tracking first (3x prometido); (b) profiling real (GPU/CPU/draw-calls p95) → instancing+atlas (alavanca 🟢) → frame budget; culling/LOD **condicionados a validação de UX** e a PoC de multi-render-target que **alargue** (não feche) a porta PortalEngine/ExternalTextureLayer | **(b)** — 13.7: 🔴 "3x" refutado; LOD exige múltiplos render targets (Zed #26308 = alerta). **[Batch 2 — decidido]:** profiling (F1-5-1) é **bloqueante primeiro**; alvo honesto = **8–12 terminais ativos**; culling/LOD **condicionais a evidência**; o número 28@40-50fps (🔴 13.7) **sai de qualquer alvo** — permanece citado apenas como claim refutado | Stories de render pós-profiling (F1-5) |
| **0018** | **Modelo de confiança de skills** | Como skills de terceiros (e da própria Lina) entram com segurança | (a) só assinatura; (b) allowlist de fontes (array vazio = lockdown) + review + hook PreToolUse deny-by-default na Fase 1; assinatura ed25519 + registry como **complemento** na F1-6/Fase 2 | **(b)** — lição ClawHavoc (12% maliciosos; 13.14 §3): assinatura não detecta "assinado E malicioso". A doutrina/skills da F1-3 já nascem sob o mesmo regime | Instalação de skills da Lina (F1-3) e assinatura (F1-6) |

Os seis primeiros (0008–0013) são os candidatos mínimos pedidos pelo Maestro; 0014–0018 são adições do arquiteto com fonte; 0019 foi redigido por ordem do batch 2 (§c.2). Nota: **não** propor ADR de stack de UI (sugestão espúria da pesquisa 13.12) — gpui foi decidido por spike medido e está assinado (doc 33; CLAUDE.md §Stack).

### c.1 Decisões do fundador (batch 2, 2026-06-06) — registradas

**Ordenação dos ADRs confirmada** (pedido 5 do Maestro): **0008–0010 são bloqueantes antes de código** das ondas que travam — 0008 → detecção/modal/agente-cria-terminal (F1-1/F1-2/F1-3); 0009 → story de injeção remota (F1-1); 0010 → toda F1-4 exceto licença. O 0009 mantém a exigência de **revisão de segurança própria (red-team) antes da story de injeção**, com a mitigação por **snapshot-hash do estado do VT** registrada como **decisão de engenharia do Lina** — a pesquisa 13.13 confirma que não há padrão de indústria documentado para essa mitigação; nosso teste de race é a prova.

**Dúvida 1 RESOLVIDA — escopo da Fase 1** (vai para o texto canônico do ADR 0010/PF-1): a direção nova do fundador **supersede o roadmap do doc 01 §2**. Fase 1 = esqueleto F1-0..F1-6 (confiabilidade + observabilidade + inteligência + produto). Para nada se perder, os itens antigos não-cobertos viram **BACKLOG EXPLÍCITO da Fase 2**, nominalmente:
1. **Engine de Webhooks** + nó-Gatilho + cloudflared 1-clique (SPEC §4 #24)
2. **Discovery ampla** / Arsenal de Poderes (trait `DiscoveryProvider`; SPEC §4 #25)
3. **Curador + feed** de novidades + perfil rico + P3 Radar (SPEC §4 #26)
4. **6 presets completos** (SPEC §4 #27)
5. **Ghost wires + Linha do Tempo** (SPEC §4 #30)

⚠️ Nota do arquiteto (mesmo espírito "para nada se perder"): dois itens do SPEC §8 **não citados** na decisão também estão fora do esqueleto — **Vault Obsidian** (SPEC §4 #23) e **agendador por SO + tiers multi-CLI** (SPEC §4 #29). Proponho incluí-los no mesmo backlog da Fase 2, salvo objeção. A atualização da tabela do doc 01 §2 segue **pendente com o dono do norte** (fora do meu escopo de escrita — regra 3).

**Dúvida 2 RESOLVIDA — alvo de render** (registrado no ADR 0017): profiling (**F1-5-1**) é bloqueante primeiro; alvo honesto = **8–12 terminais ativos**; culling/LOD **condicionais a evidência** (profiling + validação de UX). O número 28@40-50fps (🔴 refutado pela 13.7) sai de qualquer alvo.

**Dúvida 6 RESOLVIDA — auto-aprimoramento v0** (F1-3): Curator-like **com gate humano — sugere, NUNCA aplica**. Transições de skill são eventos no log (inv. 4; 13.8); qualquer aplicação exige confirmação humana (mesma família do pausa-com-gate do ADR 0005). O recorte "sem auto-edição silenciosa" que propus fica confirmado.

### c.2 Texto canônico preliminar — ADR 0019: definições operacionais (progresso, travamento, spawn caps, direção estética)

> **Nota de numeração:** o Maestro pediu este conteúdo como "ADR 0008" (batch 2, item 4). Mantive **0019** porque 0008-Detecção já está ancorado na própria pesquisa (13.9 §8: "Registrar como **ADR 0008 — Padrão de Detecção de CLI**") e na tabela do §c consumida pelos peers — renumerar agora quebraria referências cruzadas. Renumeração é trivial enquanto `docs/adr/0008+` não existirem como arquivos; decisão final do Maestro. O conteúdo abaixo é o que o LLM Engineer precisa, independente do número.

---

#### ADR 0019 — Definições operacionais: "progresso", "travamento", spawn caps e direção estética default

- **Status:** Proposto (decisões do fundador 2026-06-06 incorporadas; aceitar quando F1-0 implementar o heartbeat e F1-3 a doutrina)
- **Onda/Story:** F1-0 (lifecycle/heartbeat) · F1-1 (fila de atenção, `lina check`) · F1-3 (agente-cria-terminal, doutrina)

**Contexto.** A fila de atenção, o `lina check` e o agente-cria-terminal precisam de definições **mensuráveis** — não impressões de LLM — para "este terminal está progredindo?" e "este terminal travou?". A pesquisa fundamenta: heartbeat por timestamp puro NÃO distingue *alive+working* de *alive+hung* (13.11 🟢, Sol Framework: cycle count + hash); auto-report do agente é rejeitado (agente pode mentir — 13.11 STORY 6; invariante 1); o freeze do Maestri (UI viva, terminais sem progresso — 13.12 🟢, market feedback real) é o falso-negativo a não reproduzir, e a delegação instável dele é o falso-positivo a não reproduzir.

**Decisão.**
1. **Amostragem determinística por nó** (telemetria efêmera, NÃO evento): a cada `HEARTBEAT_SAMPLE_MS = 120_000` (2 min — 13.11: hash a cada 2–3 min), capturar `(cycle_count, tail_hash)` — `tail_hash` = SHA-256 dos últimos ~80 chars do tail do PTY (13.11 STORY 6); `cycle_count` incrementa a cada advance com output novo.
2. **PROGRESSO** (definição mensurável): houve progresso na janela se **(a)** `tail_hash` mudou entre amostras consecutivas, **ou (b)** ≥1 `DomainEvent` novo atribuível ao nó no período (`RouteDelivered` de/para o nó, `TokenUsageReported`, `PlanClaimed`/`PlanChecked`, `HandoffOpened`/`Closed`, …). Qualquer um basta.
3. **TRAVAMENTO** (default conservador): nó com status **Busy** acumulando `STALL_WARN_SAMPLES = 3` amostras consecutivas SEM progresso (~6 min) → emitir **`NodeStalled` uma única vez, na transição** (anti-amplificação — ADR 0003/0005) → entra na fila de atenção como WARN. Persistindo até `STALL_BREAKER_SAMPLES = 6` (~12 min) → **circuit breaker: pausa-com-gate (`lina resume --confirm`), nunca kill** (padrão ADR 0005). Constantes em `RouterConfig` (tunáveis; defaults conservadores para não repetir a delegação instável do Maestri — 13.12).
4. **O relógio de stall só corre em `Busy`.** `Blocked` (aguardando permissão/custódia — já está na fila por outro caminho) e `Idle` NÃO acumulam — elimina o falso-positivo nº 1 (terminal esperando y/n não está travado).
5. **Invariante 4:** o VEREDITO (progredindo/travado) é **projeção do event log** — toda transição vira evento (`NodeStatusChanged`, `NodeStalled`); as amostras cruas são efêmeras e nunca apendadas (alto volume/baixo sinal). `lina check @X` responde da projeção, nunca de view cacheada (13.11 STORY 3: agentes leem log recente).
6. **Spawn caps (agente-cria-terminal, F1-3 — decisão do fundador, batch 2):** `max_spawns_per_turn = 2` por nó solicitante. A distinção **origem vs cascata** reusa o binding NÃO-forjável do ADR 0007: pedido de spawn nascido em cascata (`hops ≥ 1` efetivo, via `derive_root_hops`) conta contra o cap E contra o `DELEGATION_BUDGET`; criação direta pelo humano (UI/comando) não passa pelo cap de agente. Spawn é sempre via Supervisor (**NodeId = autoridade única**); cada spawn conta no teto de custo (ADR 0005); evento: `NodeAdded{requested_by}` (campo aditivo, §d F1-3).
7. **Direção estética default da doutrina (F1-3, conforme 13.4):** a skill estética da Lina carrega **opinião explícita como default** — banir os genéricos (Inter/Roboto/Arial como tipografia default, gradiente branco-roxo, layout-padrão-de-template); adotar JetBrains Mono para código/terminal e tipografia display com personalidade **via tokens do design system da F1-2** (nunca fonte/cor hard-coded — 13.3 [P0]); motion de alto impacto SEMPRE subordinado a reduce-motion (13.15); paradigma prescritivo + encorajamento explícito de risco criativo (13.4 §3, Anthropic Cookbook). Vale para o que a Lina **produz** (código/design/artefatos) e para a UI do app. Customizável pelo usuário; o default é a opinião.

**Limite explícito.** Stall detection mede o **PTY e o log**, não a UI (o freeze de UI do Maestri é classe diferente — coberto por test-suite que replica os failure modes dele, 13.12). Os thresholds são hipóteses calibráveis: validar contra a baseline real (5–8 Claudes, gate formal da F1-0) antes de considerá-los definitivos.

**Alternativas rejeitadas.**
- **Auto-report do agente** ("estou progredindo") — agente pode mentir/alucinar (13.11 STORY 6; campo escrito por agente nunca decide — família ADR 0007).
- **Timeout de wall-clock puro** — não distingue *hung* de *long-task* (13.11 🟢).
- **LLM julgando o scrollback** — viola o invariante 1 e introduz não-determinismo no guardrail (13.2 §portas).
- **Kill no breaker** — perde contexto/trabalho; pausa-com-gate preserva (ADR 0005).

---

---

## d) Contrato de eventos — o que o log ganha por onda

Regras herdadas que mantêm o invariante 4: variantes **aditivas** no `DomainEvent` com `event_version=1` (o app só constrói, nunca faz match exaustivo — ADR 0001 §2); campos novos em structs existentes via `#[serde(default)]`; **upcasting** só quando mudar o shape de evento existente; anti-amplificação: evento de transição, não de repetição (ADR 0005 §3); eventos de alto volume/baixo sinal (frames, heartbeats saudáveis) **não** entram crus no log de domínio — agregação/sampling (coerente com ADR 0003).

| Onda | Eventos novos propostos (nomes a refinar pelos detalhadores) | Notas de compatibilidade |
|---|---|---|
| **F1-0** | `NodeStatusChanged{node, from, to}` (state machine 0015) · `HandoffOpened{id, from, to, intent, contract}` / `HandoffClosed{id, reason: Completed\|Timeout\|Aborted}` · `MessageDeadLettered{id, reason, attempts}` · `NodeStalled{node, cycle_count}` (só na transição) · `CircuitBreakerTripped{node, strikes}` | Todos aditivos. `AwaitOpened/AwaitClosed` JÁ existem (ADR 0002) — handoff **reusa** o lifecycle await, não duplica. `IdempotencyHit` NÃO vira evento (alto volume/baixo sinal — dedup preventivo, ADR 0003); contador em projeção. `intent` é campo do envelope, não evento. |
| **F1-1** | `CliDetected{node, cli, confidence, evidence}` · `CliSpawned{node, profile_id}` (13.9) · `PermissionRequested{node, tool, input_hash, idempotency_key}` / `PermissionResolved{id, decision, via: Human\|Timeout\|Policy}` · `CostAlert{workspace, threshold, current}` (só na transição, padrão CostCeilingHit) | `TokenUsageReported` JÁ existe (ADR 0005) — F1-1 **fecha a pendência** do app emitir de verdade. Se precisar de breakdown (thinking/cache), `TokenUsageReported` v2 **via upcasting** — caso canônico do framework de upcasting da âncora Event Store. A fila de atenção é PROJEÇÃO (permission+custódia+gates) — não inventa evento de fila. |
| **F1-2** | `TerminalRenamed{node, name}` · `TerminalReconfigured{node, profile_delta}` · `ThemeChanged{mode, overrides_ref}` | Disciplina: só vira evento o que muda estado **reconstruível** (renomear sim; hover não). Modal usa os mesmos eventos de spawn já existentes. |
| **F1-3** | `SkillInjected{node, skill, version}` · `ColdReviewRequested/Completed{author_node, reviewer_node, verdict}` · `SkillTransitioned{skill, from, to, by: Curator\|Human}` (auto-aprimoramento — transição é evento, nunca side-effect de daemon, 13.8) | Agente-cria-terminal: **reusar** `NodeAdded` com campo aditivo `requested_by: Option<NodeId>` (`serde(default)`) — correlação via `root_cause_id` do envelope; não criar evento paralelo (lição de fidelidade de contrato da Fase 0). |
| **F1-4** | `WorkspaceCreated/Closed{workspace_id}` · `WorkspaceSwitched{from, to}` · `WorkspaceRestored{workspace_id, nodes}` · `LicenseChecked{tier, workspace_limit, outcome}` | `LicenseChecked` NUNCA loga chave/assinatura (material sensível fora do log). Com o ADR 0010, eventos de roster passam a carregar `workspace_id` — campo aditivo `serde(default)` (workspaces antigos = workspace único default; replay antigo intacto). |
| **F1-5** | `PaneSuspended/PaneResumed{node, reason}` · `ScrollbackIdleFlushed{panel, lines}` · `TerminalRecovered{panel, lines_restored, crash_ts}` · `ScrollbackRetentionApplied{panels, lines_deleted}` · `RenderProfileSampled{p95_ms, draw_calls, panes_drawn}` (agregado por janela, não por frame) | Scrollback em si NÃO entra no event log (é log de conteúdo próprio — ADR 0013); o que entra são os FATOS de durabilidade. Profiling: amostra agregada, nunca por-frame (volume). |
| **F1-6** | `SkillSourceAllowed/Blocked{source, reason}` · `CredentialBrokered{node, secret_ref, ttl}` (referência, JAMAIS o segredo) · `WebhookConfigured` — **replay no boot** (fecha MEDIA-5: o evento existe; falta a projeção reconstruir os bindings — é correção de projeção, não evento novo) | MEDIA-5 é o exemplo vivo do inv. 4 estrito: o fato é durável, a projeção é que faltava (handoff §7-D). |

**Protocolo de colisão** (multi-terminal): ver §a.4 — append-only no fim do enum, bloco por onda, um dono do `events.rs` por rodada.

---

## e) Riscos estruturais — top-5 do épico inteiro

1. **PF-1: multi-workspace sem namespace de trust = injeção cross-workspace.** O mais grave porque transforma uma *feature PRO vendida* (F1-4) num furo do invariante 5: `WorkspaceTrust` global diria "sim" para pares de Espaços distintos (handoff §7-C; ADR 0006). **Mitigação:** ADR 0010 aceito antes de qualquer código F1-4; eventos workspace-scoped desde a F1-0; red-team cross-workspace como critério de aceite da onda; PF-2 tratado junto.
2. **Injeção remota de y/n sem red-team próprio.** A fila de atenção culmina em escrever no stdin de um PTY remoto — race ANSI/tcflush com CVEs reais e **sem padrão de indústria para a mitigação** (13.13 §5: a técnica é decisão nossa). É a feature mais "não-técnico-first" do épico (item 7 do doc vivo) e a mais perigosa. **Mitigação:** ADR 0009 + teste de race dedicado + idempotency_key + validação de estado do VT + SLA de timeout; story bloqueada até o ADR; red-team específico antes do merge (lição da Fase 0: red-team próprio, campos controláveis nunca decidem).
3. **Colisão estrutural entre ondas paralelas no `DomainEvent`/`bridge.rs`.** TODAS as 7 ondas adicionam eventos e 3+ tocam o app — em multi-terminal isso reproduz os fantasmas da Fase 0 (clippy/fmt vermelhos por dono alheio, diff compartilhado mal-atribuído, tree suja revertida — handoff §8). **Mitigação:** protocolo de append do §a.4 (dono do enum por rodada; blocos por onda), fronteiras de arquivo DECLARADAS por story no doc 34, workers não-commitam + validação de fora (processo que já funcionou na Fase 0).
4. **Promessas não-medidas viram metas contratuais.** A pesquisa refutou as magnitudes em quatro frentes: render ("3x", "LOD imperceptível", "28@40-50fps" — 13.7 🔴), custo ("preciso 100% offline" — 13.5 🔴 undercount), a11y ("conforme ARIA spec" — 13.15 🔴) e isolamento ("same-uid suficiente" — 13.14 🔴). Se as stories herdarem os números da proposta original, o épico fecha "verde" entregando menos do que comunica — anti-doutrina (verificação-antes-de-pronto). **Mitigação:** profiling-gate (ADR 0017), custo como "estimativa ~$X", formulações honestas já prontas nos docs 13.14/13.15, gates **medidos na tela** como na Fase 0.
5. **Cronograma externo: Gemini EOL 18/jun/2026 + bloco infra preso no fundador + concorrência acelerando.** A data do Antigravity cai DENTRO da F1 com docs A2A/OTel ainda privadas (13.10); CI 3-SO/Windows/assinatura dependem de compras do fundador (handoff §7-B); Hermes (~5 releases/mês, A2A em planejamento — 13.8) e Maestri (cadência quinzenal — 13.12) andam. **Mitigação:** Gemini como "transitional best-effort" + spike Antigravity pós-estabilização (~fim jul/2026); F1-6 com sub-bloco "infra-do-fundador" explicitamente desacoplado do critério de saída do épico; monitorar Hermes issues #514/#7708/#4454 (se o A2A deles sair do papel, a janela do nosso diferencial encurta).

*Menções honrosas (não top-5, mas para o doc 34):* (i) **F1-0 sem baseline reproduzido** — sem rodar P0/P1 com 5–8 Claudes reais antes do fix, "resolvido" é fé (13.2 §Validação prática — mandatório); (ii) **neutralidade multi-CLI é teoria não testada** — o único bring-up real é Claude Code; sem E2E com 3+ CLIs rodando as MESMAS skills, a porta do invariante 3 pode fechar por acoplamento acidental (13.12 ressalva crítica — vira critério de aceite da F1-1/F1-3); (iii) **fila de atenção com 20+ terminais** pode desmoronar em clutter (13.3 risco) — validar com o fundador usando, que é insubstituível (lição da Fase 0).

---

## Dúvidas para o Maestro

1. ~~**Naming/escopo da "Fase 1"**~~ — ✅ **RESOLVIDA (batch 2, fundador 2026-06-06):** Fase 1 = esqueleto F1-0..F1-6; a direção nova supersede o doc 01 §2; itens antigos não-cobertos viram backlog explícito nominal da Fase 2 no texto canônico do ADR 0010/PF-1 (ver §c.1). Permanece pendente apenas a atualização do doc 01 §2 pelo dono do norte.
2. ~~**Alvo de escala do render**~~ — ✅ **RESOLVIDA (batch 2):** profiling (F1-5-1) bloqueante primeiro; alvo honesto **8–12 terminais ativos**; culling/LOD condicionais a evidência; 28@40-50fps fora de qualquer alvo (ver §c.1 e ADR 0017).
3. **Pricing (bloqueia parte do ADR 0011):** perpetual ~$99 ou assinatura ~$8/mês — UM modelo na F1 (13.6: ambas as lentes só concordam em "não fazer os dois"). Recomendação da pesquisa: perpetual-first. Decisão do fundador.
4. **Terminais externos** (CLI aberto fora do Lina) são requisito de produto na F1? Decide se `lina register-self` entra no ADR 0008 ou fica fora (hoje não há caso de uso no backlog — 13.9).
5. **Partição das 5 MEDIA:** proponho MEDIA-3/MEDIA-4 (scrollback) → F1-5 e MEDIA-1/MEDIA-2/MEDIA-5 (webhooks) → F1-6, para não criar dois donos do mesmo arquivo. Confirmar para os detalhadores das ondas.
6. ~~**Auto-aprimoramento v0 (F1-3)**~~ — ✅ **RESOLVIDA (batch 2):** Curator-like **com gate humano — sugere, nunca aplica**; recorte "sem auto-edição silenciosa" confirmado (ver §c.1).
7. **Precedência da fila unificada:** assinar custódia > permissão > custom gates (13.13 item 4) como regra do produto? Afeta o ADR 0009 e a UI da F1-1.
8. **events.rs em rodadas paralelas:** validar o protocolo do §a.4 (dono único do enum por rodada, blocos append-only por onda) como regra de processo do doc 34.

---

PRONTO: DAG das 7 ondas com de-risk e mapa de paralelização por fronteiras disjuntas, teste de portas/invariantes onda a onda (5 tensões viram ADR-gate), 12 ADRs propostos (0008–0019) com recomendação e gatilho "antes de", contrato de eventos aditivo por onda preservando o invariante 4, top-5 riscos estruturais com mitigação e 8 dúvidas ao Maestro.

ATUALIZADO (batch 2, 2026-06-06): decisões do fundador registradas em §c.1 (escopo Fase 1 supersede doc 01 §2 + backlog nominal da Fase 2 no ADR 0010; alvo de render 8–12 ativos no ADR 0017; Curator sugere-nunca-aplica) · ADR 0019 redigido em §c.2 (progresso/travamento mensuráveis por projeção do log, defaults conservadores 13.11/13.12; spawn caps max_spawns_per_turn=2 com gating origem/cascata via ADR 0007; direção estética anti-slop default 13.4) · snapshot-hash assinado como decisão de engenharia no ADR 0009 (red-team próprio mantido) · ordenação 0008–0010 bloqueantes confirmada · baseline P0/P1 promovida a parte FORMAL do gate F1-0 (Spec Writer integrando em ondas-0-1.md) · Dúvidas 1, 2 e 6 fechadas.
