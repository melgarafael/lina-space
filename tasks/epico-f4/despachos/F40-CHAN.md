# DESPACHO F40-CHAN — F4-0-1 · trait `Channel` + `ChannelRegistry` (dono: Terminal B / Ultra Code)

## CONTEXTO (leia ANTES de codar — é obrigatório)
Você está na onda **F4-0 (Substrato de canais & credenciais)** da Fase 4 do Lina Space. A Fase 4 abre o Lina para o mundo externo (WhatsApp, e-mail, ferramentas) **sem trair o local-first**: nada sai da máquina por padrão; canal externo é opt-in sinalizado; o segredo é custodiado, nunca no env do agente.

**Documentos a conferir (navegue pelos links do Obsidian — NÃO pule):**
1. **Peça da onda (fonte da verdade story-a-story):** `tasks/epico-f4/onda-0.md` — leia a seção **F4-0-1** (seu território e critérios) + o cabeçalho "Estado de partida" + "Decisões registradas".
2. **Épico no vault (visão e fios condutores):** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — leia §I (Meadows [10] Estrutura), a Onda F4-0, e o critério **F4-0-1** (§III) + o §IV (contrato de eventos).
3. **Doc 40 (mineração do Hermes)** §4 (manifesto antes de executar) e §6 (curadoria/trust = a segurança para o leigo): `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/40 - Skills MCP CLI e Usabilidade"`.
4. **ADR 0006** (`docs/adr/0006-injecao-a2a-default-deny-por-pertencimento.md`) — o `trust default-deny por pertencimento` espelha este ADR.
5. **Stub com o norte já escrito:** `crates/lina-core/src/channel.rs` (eu deixei o header com invariantes e o que entregar).

## FUNÇÃO
Você é o **Dev Core** desta frente — a abstração central da fase. A trait `Channel` é uma **porta de continuidade** (doc 01 §3): o que permite WhatsApp/e-mail/ferramentas entrarem como impls + manifesto, **sem re-arquitetar o core** (invariante #7 core/shell split). Se a porta ficar mal-desenhada, tranca tudo a jusante (Meadows: estrutura é decisiva-no-início). Faça-a elegante e mínima — um staff engineer assinaria.

## DIRECIONAMENTO (território + como trabalhar)
- **Território de arquivo (SÓ estes):** `crates/lina-core/src/channel.rs` (preencher o stub) + `channels/<nome-stub>/manifest.toml` (1-2 exemplos) + testes inline no próprio `channel.rs`.
- **NÃO TOQUE** (costuras de dono único, já congeladas na largada): `crates/lina-core/src/events.rs`, `crates/lina-core/src/lib.rs`. O evento `ChannelRegistered { channel, manifest_ref, trust_tier, install_ref }` **já existe** — você só o emite/projeta. O `pub mod channel;` já está em lib.rs.
- **Worktree isolada:** crie `git worktree add ../lina-f4-0-chan -b lina/f4-0-chan` a partir da `main` (que já tem a largada, commit `fcb9371`). Rode `git`/`cargo` de lá; rode os verbos `lina` do dir de produção (`/Users/rafaelmelgaco/einstein workspace/lina-space`).
- **Entregue:**
  1. **trait `Channel`** — porta de continuidade. Métodos mínimos: identidade (`id()`/`name()`), acesso ao manifesto parseado. O core **não** importa tipo de canal concreto.
  2. **manifesto declarativo** `channels/<nome>/manifest.toml` com `transport`, `auth`, `scopes`, `tools.default_enabled`, `install.ref` (pinado em SHA/tag). Parse + **validação por `serde`** (struct tipada) — manifesto malformado → erro de schema **ANTES de qualquer execução** (gate barato, doc 40 §4). Teste de schema: um TOML malformado falha o parse, nunca executa.
  3. **`ChannelRegistry`** — projeção PURA por replay de `ChannelRegistered` (molde: `crates/lina-core/src/clue.rs` `ClueSet::from_records` — o último registro de um `channel` vence; eventos indecodificáveis são pulados). `register_channel(...)` emite `ChannelRegistered` no `EventStore`.
  4. **trust default-deny:** `trust_tier ∈ {"core","curado","comunidade"}` (campo do evento). Registrar ≠ conectar: o canal nasce **"declarado, não conectado"** (status derivado: registrado mas sem `ChannelConnected`, que é F4-1+ — aqui só o estado declarado).
- **Invariantes (o red-team do gate vai re-derivar no código — não falhe):**
  - **Manifesto é DADO, JAMAIS autoridade.** Nenhum campo do manifesto (nome/transport/scope) decide identidade/ordem/autorização. O gate de efeito externo é o broker (F4-0-3), não o manifesto.
  - **`install_ref` PINADO** (SHA/tag), nunca HEAD flutuante.
  - **Projeção do log (inv #4):** `from_records` PURA (sem relógio, sem I/O); replay reconstrói byte-a-byte.
- **Convenções (CLAUDE.md repo):** `cargo fmt` + `cargo clippy -p lina-core --all-targets -D warnings` LIMPOS; sem `unwrap()` em produção; testes que provam o critério.
- **Sinal de coordenação:** assim que a **assinatura da trait `Channel`** estiver estável, avise o Maestro (`lina ask "@Maestro 00" "F40-CHAN: assinatura da trait Channel pronta — <resumo>" --intent status`) — as frentes BROKER (K) e MONITOR (G) trabalham contra ela.

## OBJETIVO (critério observável — o que prova que está pronto)
Registrar um canal-stub → ele aparece no `ChannelRegistry` como **"declarado, não conectado"** com seu `trust_tier`; um `manifest.toml` malformado → **falha no schema** (teste), nunca executa. `register_channel` emite `ChannelRegistered` (provado por replay: um teste que registra, projeta do log e re-encontra o canal).

## RESULTADO ESPERADO
- Não commite (o Maestro valida de fora por exit code e commita por fatia). Deixe o trabalho na worktree `lina/f4-0-chan`.
- Rode os gates você mesmo e cole os exit codes: `cargo test -p lina-core channel` , `cargo clippy -p lina-core --all-targets` (exit 0), `cargo fmt -p lina-core --check`.
- Reporte com o marcador na primeira linha: **`PRONTO: F40-CHAN`** + 3-5 linhas (o que entregou, os exit codes, arquivos tocados) — OU **`BLOCKED: F40-CHAN`** + o que falta/por quê. Reporte via `lina ask "@Maestro 00" "<...>" --intent status`.
