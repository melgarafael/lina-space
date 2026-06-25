# ADR 0052 — Área de Poderes: scan determinístico de disco (projeção efêmera, mostrar ≠ autorizar)

- **Status:** **Aceito** (decisão de arquitetura do Arquiteto, 2026-06-25). ADR-gate da Onda **F2-4**: o **Scanner core (F24CORE)** e o **Painel UI (F24UI)** não começam sem este contrato selado; o **QA (F24QA)** prova o limite `mostrar ≠ autorizar` por mutação contra ele. As demais peças (design directions) correm à parte. O parágrafo de decisão vai ao `plan.md` (item `F24ADR`) pelo Maestro 01, dono único das costuras (`events.rs`, `lib.rs`, `main.rs`, `bridge.rs`).
- **Escopo:** tornar **visíveis** os poderes instalados na máquina do leigo (skills/plugins/agents/commands/hooks/MCPs), com 5 estados e ação de 1 clique. Decide as 6 questões do gate: (1) fonte da verdade do scan, (2) evento de auditoria, (3) contrato de tipos Core↔UI, (4) limite `mostrar ≠ autorizar`, (5) caminhos por CLI no perfil TOML, (6) estado da arte do watcher. **DISJUNTO** da implementação (territórios de Terminal B / Especialista em Telas / Terminal R).
- **Relacionados:** **ADR 0008** (detecção de CLI em camadas — o padrão que este ADR transplanta: registry determinístico primeiro, heurística nunca decide, ids por TOML, `detecção ≠ autorização`), ADR 0004 (custódia — o gate de execução que NÃO se move), ADR 0010 (multi-workspace / escopo global vs projeto), invariantes **#3** (neutralidade multi-CLI: CLI novo = perfil TOML, sem recompilar) e **#4** (event log = fonte da verdade — delimitado abaixo). Fontes: `tasks/pesquisa-f2/entrega-d4-comandos-menus.md` (§I inventário real, §II achados **A5/A7/A8**, §IV conflito **#4**) · `tasks/epico-f2/despachos/f2-4/_contexto.md`.

## Contexto

O leigo tem skills/plugins/agents/hooks/MCPs instalados — em **4 CLIs distintos** (medido: `~/.claude/skills` 75 pastas, `~/.gemini/skills` 13, `~/.codex`, `~/.config/opencode`) — e **não vê nenhum**. A F2-4 cria a vitrine: ele vê o que tem, entende o que funciona em qual terminal, e conserta com 1 clique. Duas doutrinas inegociáveis se cruzam aqui, e é por isso que a onda precisa de um contrato selado antes do código:

1. **inv#4 "o event log é a fonte da verdade"** × a Área de Poderes lê o **DISCO** — estado que o app **não controla** (skills instaladas por outras ferramentas, fora do log).
2. **doutrina de segurança "campo lido do disco é DADO, jamais autoridade"** — o painel mostra nome/descrição/frontmatter; nada disso pode decidir autorização.

A pesquisa já resolveu a tensão (achado **A8**: o ADR 0008 transplanta inteiro; conflito **#4**: o painel é projeção de scan como `lina list` é projeção de runtime, e mudanças PODEM virar evento aditivo para auditabilidade). Este ADR **ratifica e fixa o contrato** — não re-decide do zero.

## Decisão

### (1) Fonte da verdade do scan = DISCO, manifest-first, projeção efêmera (NÃO event-sourced como fonte primária)

O inventário de poderes é uma **projeção de disco re-derivável a cada abertura do painel** (~0ms medido: listar 75 pastas ≈ 0ms), reconstruída do zero sem cache a invalidar — exatamente como `discovered_clis` é projeção do **PATH** (`events.rs:1909`), não do log.

- **Por que isto NÃO viola inv#4:** o invariante governa o domínio que o **app controla** (terminais, mensagens, custódia, crenças — coisas que o app cria e cujo histórico ele possui). Poderes são estado **EXTERNO**: instalados por `npx`, `git clone`, outro CLI — o log nunca os possuiu, então o log não pode ser a fonte primária deles, do mesmo modo que não é fonte primária do `$PATH` ou dos session-files em disco (ADR 0008, Camada 3). Fonte primária = disco. O log permanece fonte da verdade **do que é dele**.
- **manifest-first é LEI (achado A7, custo medido):** ler o **manifesto pequeno SEMPRE**, varrer a **árvore pesada NUNCA**. Plugins: `~/.claude/plugins/installed_plugins.json` (**13KB**) — jamais a árvore de **1,9GB** (repos git + cache → bomba no bring-up Linux: inotify ENOSPC degrada a máquina inteira). Skills: frontmatter de cada `SKILL.md` (scan raso, ~0ms), reusando **`skill_index.rs:409 parse_frontmatter`**. Molde de varredura→struct→projeção: **`cli_discovery.rs:167 discover_clis_in`** — copiar o padrão, não reinventar.
- **Os 5 estados se computam contra o terminal focado.** A entrada do scanner leva o CLI do terminal em foco; o estado `InertHere` é `origin.cli != focused_cli` (skill na pasta do CLI X, terminal roda CLI Y — o caso **NORMAL** do multi-CLI, §I). Assinatura fixada:

  ```rust
  // crates/lina-core/src/powers.rs (NOVO)
  /// roots derivados dos CliProfiles (ponto 5) + convenção; focused_cli decide InertHere.
  /// Sem foco (None) nada é inerte. Re-deriva a cada abertura/troca de foco (~0ms, efêmero).
  pub fn scan_powers(roots: &PowerRoots, focused_cli: Option<&str>) -> PowerInventory;
  ```

### (2) Evento de auditoria `PowerScanned` — aditivo, padrão META, SÓ contadores

Entra nesta onda, **mínimo**: serve à camada (f) do release (auditoria anti-manipulação / observabilidade — fio condutor do fundador), **não** ao painel. Contrato fixado (molde META = `SkillSelected:1387` — no-op no `apply`, projeção dedicada se preciso; campos aditivos):

```rust
// events.rs — DONO: Maestro 01 (largada). ADITIVO: replay de log antigo nunca quebra.
/// F2-4: um scan da Área de Poderes ocorreu — METADADOS para auditoria anti-manipulação.
/// META: no-op no `apply`; o painel NÃO depende deste evento (lê o scan direto). ZERO
/// nome/descrição/conteúdo de poder; NENHUM campo decide autorização.
PowerScanned {
    /// total de poderes vistos (soma de todos os kinds).
    total: u32,
    /// contagem por kind — chave = PowerKind em minúsculo ("skill","plugin",…). String,
    /// não enum, para o evento sobreviver a um PowerKind novo (replay-safe).
    #[serde(default)]
    counts: BTreeMap<String, u32>,
    #[serde(default)]
    scanned_at_ms: u64,
},
```

- **Limite duro:** o evento carrega **somente contadores** + timestamp. **NUNCA** nome, descrição, frontmatter ou conteúdo de skill; **NUNCA** campo que entre em caminho de autorização. Se o Maestro optar por não fiar o emit nesta onda para reduzir superfície, **o painel não quebra** (é META, independente) — mas o recomendado é emitir o mínimo, porque a auditoria anti-manipulação é critério do release.

### (3) Contrato de tipos Core↔UI — o view-model que `powers.rs` produz e `powers_panel.rs` consome

```rust
// crates/lina-core/src/powers.rs (NOVO) — o contrato que a UI renderiza.
pub enum PowerKind { Skill, Plugin, Agent, Command, Hook, Mcp }

/// AJUSTE ao contrato proposto: origem são DOIS eixos ORTOGONAIS (medido §I — uma skill é
/// "global" E "do Gemini" ao mesmo tempo). Colapsar num enum único forçaria o painel a
/// escolher qual eixo perder. Struct de 2 campos = a menor representação fiel.
pub enum PowerScope { Global, Project }        // vale em tudo (home) ou só no Espaço aberto
pub struct PowerOrigin {
    pub scope: PowerScope,
    /// pasta de QUAL CLI ("claude-code","gemini","codex",…); None = não atrelado a um CLI.
    /// É o que o leigo vê rotulado ("global / deste projeto / do Gemini") e o que decide InertHere.
    pub cli: Option<String>,
}

pub enum PowerState { Ready, UpdateAvailable, NeedsRepair, InertHere, Disabled }

pub struct Power {
    pub kind: PowerKind,
    pub name: String,         // id/nome CRU do manifesto
    pub description: String,  // descrição CRUA do manifesto — ver limite abaixo
    pub origin: PowerOrigin,
    pub state: PowerState,
}

pub struct PowerInventory {
    pub powers: Vec<Power>,
    pub counts: BTreeMap<PowerKind, u32>,  // resumo do nível 1 ("75 Poderes · 33 Plugins · …")
}
```

- **`description` é a CRUA do manifesto** (decisão: separar dado de apresentação). A tradução leiga — rótulo "Poder", âncora "(skill)" no detalhe (lição do rename do Obsidian, A5), strings via `const`/`copy_*` + teste anti-jargão — é responsabilidade da **copy do painel** (Frontend), nunca do scanner.
- **`Disabled` só se materializa se o app REALMENTE puder religar** o poder (toggle com enforcement). No MVP o Lina não religa skills → na prática este estado **não aparece**; permanece no enum como contrato, mas **a tela nunca mente**: estado sem ação de 1 clique é banido (A5). Os outros 4 (`Ready`/`UpdateAvailable`/`NeedsRepair`/`InertHere`) têm ação acoplada obrigatória (`_contexto.md` §3).

### (4) Limite explícito — **mostrar ≠ autorizar** (transplante do ADR 0008; critério inforjável do QA)

**Nenhum campo de `Power` lido do disco** (`name`/`description`/`origin`/frontmatter/JSON do manifesto) entra em caminho de **identidade, ordem ou autorização**. Aparecer no painel **NUNCA** habilita executar: os gates de execução continuam exatamente onde estão — **custódia (ADR 0004)** e **WorkspaceTrust (ADR 0006/0010)**. O scanner é leitura de mundo externo; seu produto é **DADO de exibição**, jamais autoridade. Espelha o limite "detecção ≠ autenticação" do ADR 0008 §Limite. É o que o QA prova por mutação: forjar nome/origin de um poder **não** abre nenhum caminho de execução.

### (5) Caminhos por CLI vêm do perfil TOML (inv#3) — campos aditivos no `CliProfile`

Para o scanner descobrir as pastas de um CLI **sem caminho hardcoded** (não repetir o erro `KNOWN_CLIS` do ADR 0008 §5), dois campos aditivos no `CliProfile` (`crates/lina-cli-profiles/src/lib.rs:147`, hoje `#[serde(deny_unknown_fields)]`), no molde EXATO do `session_dir_pattern:200` (`Option<String>`, glob com `~`, `#[serde(default)]` ⇒ perfil antigo carrega intacto):

```rust
/// F2-4: pasta de skills deste CLI (glob, suporta `~`), ex. "~/.claude/skills", "~/.gemini/skills".
/// `None` = CLI sem pasta de skills. Consumido por powers.rs::scan_powers.
#[serde(default)]
pub skills_dir: Option<String>,
/// F2-4: caminho do arquivo de config de MCP deste CLI (formato por-CLI), ex. "~/.claude.json"
/// (Claude, JSON) ou "~/.codex/config.toml" (Codex, TOML). `None` = CLI sem MCP. Suporta `~`.
#[serde(default)]
pub mcp_config_path: Option<String>,
```

- **Escopo dos campos (ponytail/ARQ-1 — sem campo especulativo):** `skills_dir` e `mcp_config_path` têm consumidores reais HOJE (4 CLIs têm pasta de skills; Claude e Codex guardam MCP em locais/formatos diferentes). **Plugins/agents/commands/hooks NÃO viram campo** nesta onda — são convenção do profile **Claude** (derivados de `~/.claude/`), porque não há 2º CLI que os tenha. Vira campo aditivo **quando** um 2º CLI os introduzir — porta aberta, não pré-aberta.

### (6) Watcher (F2-4-2) — scan-ao-abrir é o essencial; watcher raso debounced é o teto barato; recursivo é PROIBIDO

Ratifica o achado **A7**:

- **Essencial:** scan-ao-abrir + refresh oportunista no foco da janela. Já entrega "nunca dado velho ao abrir", sem cache.
- **Teto barato:** watcher **raso, debounced ≥750ms**, sobre o **diretório-PAI** dos ~5 manifestos (`installed_plugins.json`, `~/.claude.json`, `.mcp.json` do projeto, `config.toml` do Codex, pasta `skills/` rasa). Atomic saves trocam o **inode** → observar o **pai**, nunca o arquivo. Custo no macOS (FSEvents): nulo. Ganho: o efeito "instalei pelo terminal e o Poder apareceu sozinho".
- **PROIBIDO:** watch **recursivo** da árvore pesada (1,9GB) — inotify ENOSPC no Linux futuro quebra TODAS as ferramentas da máquina do usuário.
- **Onde mora o quê:** a parte **PURA** no core (`powers.rs`) — a lista de diretórios-pai a observar + a política de debounce como constante:

  ```rust
  // powers.rs — PURO, testável headless.
  pub fn watch_targets(roots: &PowerRoots) -> Vec<PathBuf>; // os diretórios-PAI dos manifestos
  pub const POWERS_DEBOUNCE_MS: u64 = 750;                  // ≥750ms (A7)
  ```

  A **fiação do `notify` real** (criar o watcher/debouncer no app, ligar ao refresh do painel) é do **Maestro** (app/`bridge.rs`/`main.rs`) — gpui não roda headless, então a lógica fica pura e provada por teste; a costura é diff na entrega.

## Segurança (portas que NÃO fecham)

- **mostrar ≠ autorizar (ponto 4):** o produto do scanner é DADO de exibição; gates de execução permanecem em custódia/WorkspaceTrust. Mutação (forjar nome/origin) não abre caminho de execução — critério do QA.
- **`PowerScanned` é observação, não autoridade:** só contadores + timestamp; nenhum campo decide nada. Mutar os contadores não muda comportamento (é métrica).
- **manifest-first não-negociável:** ler a árvore de 1,9GB é a porta que fecha (freeze no scan, ENOSPC no Linux). O ADR torna "manifesto pequeno SEMPRE" critério observável da onda.
- **Heurística nunca decide estado** (transplante ADR 0008 §Camada 4): adivinhar o estado de um poder pelo output de um terminal é **banido**. Estado vem do disco (frontmatter válido? arquivo presente? na pasta do CLI focado?), determinístico.
- **a tela nunca mente:** estado sem ação de 1 clique não existe (`Disabled` sem enforcement não é emitido).

## Por quê assim (alternativas descartadas)

- **Event-source o scan como fonte PRIMÁRIA do painel** — rejeitado: poderes são estado externo que o app não possui; forçá-los no log inverteria a relação (o log seria projeção do disco, não o contrário) e exigiria reconciliação a cada instalação feita por fora. O log registra a **observação** (`PowerScanned`), não detém a verdade. (Precedente: `discovered_clis` é projeção do PATH.)
- **`PowerOrigin` como enum `{Global, Project, Cli(String)}`** — rejeitado: colapsa dois eixos ortogonais (medido §I: skill é global E de um CLI). Struct de 2 campos é fiel sem perder eixo.
- **Watch recursivo da árvore** — rejeitado (A7): ENOSPC no Linux, custo desproporcional; o ganho ("apareceu sozinho") vem do watcher raso no pai dos manifestos.
- **Caminhos por CLI hardcoded no core** — rejeitado: repete a contradição `KNOWN_CLIS` × inv#3 já vivida e resolvida no ADR 0008. Vão para o TOML (`skills_dir`/`mcp_config_path`).
- **6 campos por CLI (plugins/agents/commands/hooks) já agora** — rejeitado (ARQ-1): sem 2º consumidor real hoje; convenção Claude basta. Campo aditivo quando um 2º CLI os tiver.
- **`description` traduzida no scanner** — rejeitado: mistura dado e apresentação; a copy leiga + âncora "(skill)" é do painel.

## Consequências

- **Habilita** F24CORE (scanner contra `scan_powers`/`PowerInventory`/campos `CliProfile`), F24UI (painel contra o view-model + 5 estados), F24QA (mutação `mostrar ≠ autorizar` + manifest-first + frontmatter-inválido→conserto + evento só-metadados).
- **Costuras do Maestro 01:** variante `PowerScanned` em `events.rs` (+ braço em `kind():1681`); `pub mod powers;` em `lib.rs`; `pub mod powers_panel;` + fiação em `main.rs`; ponte scanner→view-model em `bridge.rs`; fiação `notify` real. Os 2 campos do `CliProfile` são da fronteira do Dev Core (aditivos).
- **Custo:** um módulo de scan puro + 2 campos de perfil + 1 evento META. Nenhuma dependência nova no core (o `notify` já existe no app/`Cargo.toml`, conforme a fronteira da onda).
- **Porta que fecha se ignorado:** varrer a árvore pesada (freeze/ENOSPC) ou deixar campo de disco virar autoridade (regressão de segurança). O ADR crava ambos como critério inforjável.

## Verificação (observável)

- **manifest-first:** scan lê `installed_plugins.json` (13KB) e frontmatter; teste prova que a árvore de 1,9GB **nunca** é varrida (mutação: apontar para a árvore → o teste morde).
- **mostrar ≠ autorizar:** forjar `name`/`origin` de um `Power` → **0** caminho de execução habilitado (custódia/WorkspaceTrust intactos por mutação).
- **5 estados:** frontmatter inválido → `NeedsRepair`+Consertar; skill na pasta do CLI X com terminal CLI Y → `InertHere`+frase; estado sem ação não é emitido.
- **evento só-metadados:** `PowerScanned` no log não contém nome/descrição (grep no log = 0 hits de conteúdo).
- **inv#3:** CLI novo com `skills_dir`/`mcp_config_path` no TOML é escaneado **sem recompilar** o core.
- **replay-safe:** log antigo (sem `PowerScanned`, perfil sem os 2 campos) carrega intacto (`#[serde(default)]`).
