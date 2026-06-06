# ADR 0008 — Detecção de CLI por terminal em camadas (spawn-time primário; process inspection descartada)

- **Status:** Aceito (decisão técnica do arquiteto ratificada pelo Maestro na largada da execução F1)
- **Onda/Story:** F1-1 (detecção de CLI) · consumido por F1-2 (modal criar/editar terminal) e F1-3 (agente-cria-terminal)
- **Data:** 2026-06-06
- **Fontes:** pesquisa `13.9` (R2, verificada contra o código) · `crates/lina-core/src/cli_discovery.rs` · `crates/lina-cli-profiles/src/lib.rs` · `.entrega-w41.md` (AVISO 2) · `tasks/epico-f1/arquitetura.md` §c

## Contexto

Para rotular cards na UI ("este terminal roda Claude Code 2.x"), rotear A2A por capacidade,
aplicar guardrails específicos por CLI e permitir que a Lina crie terminais já configurados,
o Supervisor precisa responder em runtime: *qual CLI roda neste PTY?*

Estado real da Fase 0: a descoberta no `PATH` existe (`cli_discovery.rs`: `KNOWN_CLIS` hardcoded
com 6 ids — incluindo `agy`/Antigravity —, `find_in_path` pura, `query_version` com timeout
anti-DoS), e o comportamento de CLI vive em `CliProfile` TOML — **mas sem campos de detecção**
(não existem `session_dir_pattern`/`output_markers`/`display_name` no struct real). A pesquisa
13.9 propôs detecção em 4 camadas; a verificação adversarial contra o código **refutou a
Camada 2** (process inspection) e expôs a contradição `KNOWN_CLIS` hardcoded × invariante #3
("novos CLIs entram sem recompilar").

## Decisão

Um `CliDetector` no Supervisor (`lina-core`), com as camadas e papéis FIXADOS:

1. **Camada 1 — spawn-time profile = FONTE PRIMÁRIA (determinística).** O app é quem invoca o
   binário: `cli_type` + versão saem do perfil no momento do spawn. Emite
   `DomainEvent::CliSpawned { node_id, profile_id }` e
   `DomainEvent::CliDetected { node_id, cli_type, confidence, evidence }` (confidence = máxima).
2. **Camada 3 — session-file watch = CONFIRMAÇÃO** (retomada/pós-crash, `--resume`). Correlação
   por `(cwd, mtime)` — **sem componente PID** (herdaria a fragilidade da Camada 2). Nunca fonte
   primária; eleva/derruba confiança.
3. **Camada 4 — grid heuristics = validação cruzada/telemetria.** Regex sobre as últimas N linhas
   do grid (`VtBackend`). **Nunca decisória** (a própria pesquisa a rebaixa: frágil sob prompt
   custom/pipe/headless).
4. **Camada 2 (process inspection de PID externo) — DESCARTADA.** Motivos verificados no código:
   o único `proc_pidinfo` do repo é *self-bound* (`bench.rs` com `getpid()`); a identidade no
   Lina é por nome via `.lina/bootstrap.json` **escrito pelo app** (canal autorizado); env var é
   falseável e não serve como fronteira de identidade; e não há caso de uso "terminais externos"
   no backlog. **Se** terminais externos virarem requisito de produto: comando explícito
   `lina register-self` (registro autorizado gerando o próprio `bootstrap.json`) — nunca
   varredura de PIDs/environ.
5. **Ids derivados dos TOML (fecha a contradição com o invariante #3):** a lista de CLIs
   conhecidos passa a ser **derivada dos perfis TOML carregados no boot**, não de `const`
   compilada. `KNOWN_CLIS` vira fallback de bootstrap (perfis embutidos), não autoridade.
   Campos novos no `CliProfile`: `display_name`, `session_dir_pattern`, `output_markers[]`
   (consolidando a assinatura de cada CLI num só lugar).
6. **Agregação:** primeira camada acima do threshold vence; o veredito vai ao log via
   `CliDetected` com `confidence` + `evidence` legível ("100% via spawn-time profile").
   Fallback de UI: "Terminal (CLI desconhecido, shell detectado)".

## Limite explícito

**Detecção ≠ autenticação.** `CliDetected` informa UI e roteamento por capacidade; **nunca**
decide autorização — gates continuam derivando de `WorkspaceTrust` (ADR 0006/0010) e custódia
(ADR 0004). A Camada 4 jamais decide sozinha, nem para exibição de confiança máxima.

## Consequências

- O modal da F1-2 consome `CliDetected` (badge de CLI + versão) e o registry derivado dos TOML
  (dropdown de CLIs pré-configurados).
- O agente-cria-terminal (F1-3) consulta o mesmo registry — nenhum conhecimento de CLI no core.
- A transição Gemini→Antigravity (EOL 2026-06-18, pesquisa 13.10) entra como perfil TOML
  (`agy` já está na descoberta); Gemini fica *transitional/best-effort*.
- Testes: fake CLIs em temp dirs (descoberta), mock de criação de session-files (Camada 3),
  agregação de score com camadas conflitantes.

## Alternativas rejeitadas

- **4 camadas com process inspection como pilar (confiança alegada 90%)** — refutada na
  verificação: infraestrutura existente é self-bound; env var falseável; sem caso de uso real.
- **Manter `KNOWN_CLIS` hardcoded** — contradiz o invariante #3 e a âncora CLI Profiles TOML
  (CLI novo exigiria recompilar `lina-core`).
- **Grid heuristics como fonte decisória** — frágil (prompts custom, redirects, headless) e
  rebaixada pela própria pesquisa a *tertiary check*.
