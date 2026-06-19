# Despacho R1 · RESOLUÇÃO no bin `lina` — Terminal H (opus · high)

> Rodada **F3-CONF-3 — "O Maestro Enxerga o Time"**. Maestro: Terminal B (Ultra Code).
> Marcador OBRIGATÓRIO ao terminar: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Sem marcador = falha de protocolo (o Maestro trata como travado).

## 1. CONTEXTO (onde o trabalho está — PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES de começar, nesta ordem:**
1. `tasks/epico-f3/onda-f3-conf-3.md` — o plano desta rodada (sua fronteira, o gate, o invariante de segurança).
2. `tasks/despachos/achados-dogfooding-sessao.md` — os achados **#27**, **#23c/#23b/#14**, **#21/#21c** (a evidência crua de cada bug; busque por esses números).
3. ADRs: `docs/adr/0026-identidade-terminal-env-spawn.md` (o spawn carimba `LINA_NODE_ID=n-<uuid>`), `docs/adr/0006-injecao-a2a-default-deny-por-pertencimento.md` (leitura cross só por pertencimento).
4. **Vault (norte):** navegue por `lina vault index` e leia o épico `39 - Epico Fase 3 — O Maestro Nativo` §I e §VI-3 (a observabilidade do Maestro fecha o passo-6 do loop; o risco é adoção 0%). Caminho: `Ecossistema Labs - Operação/Debriefing Vibe Coding/39 - Epico Fase 3 — O Maestro Nativo.md`. **Se `lina vault search` pendurar, é exatamente o bug #21 que você vai consertar — use `lina vault index` (determinístico) por enquanto.**

**Os 3 pontos exatos no código (todos em `crates/lina-bootstrap/src/bin/lina.rs` — SEU arquivo, dono único):**

- **#27 — `lina history` rejeita o NodeId real do spawn.** `reader_node_id()` em `lina.rs:608-615` faz `raw.trim().parse::<NodeId>()`. Mas o app carimba `LINA_NODE_ID` como `n-<uuidv7>` (veja `app/lina-gpui/src/bridge.rs:2760` `key`, `crates/lina-core/src/workspace.rs:147` "o app gera `format!("n-{uuid}")`"). `NodeId` (`pub use lina_host::NodeId`, newtype sobre Uuid) **não** parseia o prefixo `n-` → o Maestro vê `LINA_NODE_ID invalido: "n-019edde3-..."` e o `lina history` morre no caminho real. Evidência viva: achado #27.
- **#23c/#14 — `lina list` mente vs `lina check`.** `run_list()` em `lina.rs:3163` lista o roster de `agents.json` (`read_agents()`), que pode conter um **homônimo de sessão morta**. `lina check` já resolve o **vivo** via `resolve_check_node()` em `lina.rs:511` (semântica "vivo-vence-morto" — MORTE = `NodeStatusChanged(Dead)` ∨ `TerminalExited` ∨ `NodeRemoved`; leia o comentário :505-525, ele cita #4/#14/#23c). Resultado: `list` e `check` divergem para o mesmo nome; o Maestro re-despacha quem está trabalhando.
- **#21/#21c — `lina vault search` pendura.** `vault_search()` em `lina.rs:3361` varre **todo o vault Obsidian** recursivamente, síncrono, lendo cada `.md` linha-a-linha, **sem timeout, sem teto de arquivos, sem flush** — 50 min sem 1 byte em vault grande (medido). O índice PageIndex determinístico já existe: `vault_index()` em `lina.rs:3276` lê `<LINA_HOME>/vault-index/*.md`. `VAULT_SEARCH_MAX_HITS=40` (`lina.rs:3356`) limita HITS, não tempo/arquivos.

## 2. FUNÇÃO

Você é o **dono único do bin `lina`** (`crates/lina-bootstrap/src/bin/lina.rs`) nesta rodada. Conserta os 3 bugs de observabilidade do orquestrador SEM tocar nenhum outro arquivo.

## 3. DIRECIONAMENTO (as regras do jogo)

- **Mexa SÓ em `crates/lina-bootstrap/src/bin/lina.rs`.** É um arquivo-costura: você é o único dono nesta rodada. Qualquer outro arquivo (`lib.rs`, `router.rs`, `events.rs`, `bridge.rs`) é **OFF-LIMITS** — o Terminal A está em `events.rs`/`bridge.rs`/`main.rs` agora.
- **LINHA VERMELHA DE SEGURANÇA (não cruze):** a resolução "vivo-vence-morto" é da superfície de **OBSERVABILIDADE** (`list`). **NÃO** toque `Supervisor::node_by_name` (lib.rs:1534) — ele resolve remetente/alvo do **router** (autoridade) e JÁ ignora mortos (`is_alive()`). Se você concluir que consertar `lina list` exige mexer em `lib.rs`/supervisor → **PARE e devolva `BLOCKED:` ao Maestro** (ele decide o contrato). Não decida autoridade por conta própria (família ADR 0007: campo de agente nunca decide identidade).
- **#27:** o `n-` é prefixo do app (ADR 0026). Aceite `n-<uuid>` E uuid puro — strip do prefixo `n-` no leitor antes de parsear (de preferência um helper pequeno e testável, ex. `parse_node_id_lenient(&str)`, reusável se houver outros parseadores do env no mesmo arquivo). **Não relaxe a segurança:** o `LINA_NODE_ID` continua vindo do ENV do app (não de flag/arquivo de agente) — você só tolera o FORMATO que o app de fato escreve.
- **#23c:** faça `lina list` mostrar o estado do nó **VIVO** quando há homônimo morto, reusando a semântica que JÁ existe no arquivo (`resolve_check_node`/`live_member_ids` em `lina.rs:511`/`:621`). Não invente uma 2ª definição de morte — reuse/extraia a existente. Mantenha o `--json` coerente.
- **#21:** o objetivo é **nunca pendurar**. Direção (você escolhe a forma): (a) **flush incremental** — cada hit sai em streaming (`use std::io::Write; stdout.flush()` por hit ou por arquivo), o agente vê resultado conforme aparece; (b) **teto de tempo wall-clock** (ex. ~8s) E/ou **teto de arquivos visitados** → ao estourar, **saída parcial honesta** ("… parei em Xs/N arquivos — refine o termo ou use `lina vault index`") com exit 0; (c) prefira começar pelo conteúdo das pastas mais prováveis se for barato, mas o teto é o que garante o não-pendura. **Não** quebre o comportamento atual em vault pequeno (resultados completos).
- Convenções da casa: edition 2021; `cargo fmt` + `clippy --all-targets -D warnings` limpos; **zero `unwrap()`/`expect()` em caminho de produção** (use `?`/match com mensagem); sem `catch` que engole erro; comentário só para o PORQUÊ não-óbvio. Eventos/contratos: você não cria evento novo (é tudo leitura de log/fs).
- **Você NÃO commita.** Ao terminar cada bug, valide localmente (abaixo) e reporte; o Maestro valida de fora e commita por fatia.

## 4. OBJETIVO (o porquê de negócio)

O Maestro (o orquestrador do time de IA) precisa **enxergar o que cada terminal está fazendo de verdade** para monitorar e corrigir trajeto — é o coração da inteligência da F3. Hoje ele é cego: não consegue ver a tela do worker (#27), recebe leitura falsa de quem está vivo (#23c) e trava quando busca contexto nas notas do fundador (#21). Sem isso, toda a orquestração inteligente da F3 fica decorativa. Você devolve a visão ao Maestro.

## 5. RESULTADO ESPERADO (formato exato)

Para CADA bug, deixe:
- O diff em `lina.rs` (só esse arquivo).
- Uma forma de PROVAR (o QA/Terminal R escreve a suíte formal; você prova localmente que funciona):
  - **#27:** `LINA_NODE_ID=n-<uuid válido> lina history "@algum" --tail 5` não falha com "invalido"; um teste de unidade do helper de parse aceita `n-<uuid>` e uuid puro e rejeita lixo.
  - **#23c:** um teste sobre um `content` de log com homônimo morto+vivo → `run_list`/a função de resolução escolhe o NodeId VIVO, igual `resolve_check_node`.
  - **#21:** `lina vault search <termo>` num diretório de teste grande retorna em tempo limitado com saída parcial honesta; flush incremental observável.
- **Validação local (rode e LEIA o exit, sem pipe que mascara):**
  - `cargo build -p lina-bootstrap --bin lina 2>&1 | tail -5; echo "build exit: ${pipestatus[1]}"` (zsh) — ou rode sem pipe e `echo $?`.
  - `cargo test -p lina-bootstrap 2>&1 | tail -20` (os `identity_tests` em `lina.rs:3424` e correlatos devem passar).
  - `cargo clippy -p lina-bootstrap --all-targets -- -D warnings 2>&1 | tail` (0 warnings).
  - `cargo fmt -p lina-bootstrap` (formate **apenas** seu pacote — não rode `fmt --all`, que toca hunks de peers na árvore compartilhada).
- Reporte ao Maestro: `lina ask "@Terminal B - Effort: Ultra Code" "R1: <o que entregou, exits observados>" --intent status` e termine a mensagem com **`PRONTO: R1 — #27/#23c/#21 fechados em lina.rs, build/test/clippy verdes (exits X/Y/Z)`** ou **`BLOCKED: <motivo + o que tentou>`**.

> Se em qualquer ponto a correção exigir sair de `lina.rs` (ex.: `lina list` só conserta em `lib.rs`), **NÃO saia** — devolva `BLOCKED:` com o diagnóstico arquivo:linha e o Maestro decide. Cruzar para `lib.rs`/router sem aval é a única falha grave possível aqui.
