# ONDA F2-4 — Áreas de Poderes (skills/agents/hooks/MCPs visíveis) — contexto compartilhado

> Leia ESTE doc + `tasks/despachos/_regras-comuns.md` ANTES de tocar código.
> Maestro desta onda = **Maestro 01** (dono único das costuras: `events.rs`, `lib.rs` do core,
> `main.rs` e `bridge.rs` do app). Você entrega seu MÓDULO PRÓPRIO + a costura como diff textual
> preciso na sua entrega. **Workers não commitam.** O Maestro valida de fora (exit codes diretos)
> e commita por fatia.

## 0. Specs que você DEVE ler (navegue o Obsidian pelos links)
A pesquisa está embutida nas stories — se sua implementação contradisser a fonte, **PARE e escale**.
1. **Épico (norte+gate):** vault `Ecossistema Labs - Operação/Debriefing Vibe Coding/38 - Epico Fase 2.md`
   → seção **III, Onda F2-4** (linhas ~109-125) + **§V ADRs** + **§VIII Decisões** (território estético).
   Leia com `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/38 - Epico Fase 2.md"`.
2. **Fundamento de UX (8 achados, decision-grade):** `tasks/pesquisa-f2/entrega-d4-comandos-menus.md`
   — inventário REAL do disco (§I), os **5 estados leigos** (§III.b), padrão de scan (§III.c), navegabilidade (§III.d).
3. **Contrato arquitetural:** `docs/adr/0008-deteccao-de-cli-em-camadas.md` (padrão em camadas: registry
   determinístico primeiro · heurística nunca decide · ids por TOML · **mostrar ≠ autorizar**) +
   **`docs/adr/0052-area-de-poderes-scan-determinista.md`** (gate desta onda — o Arquiteto entrega; ESPERE-o).
4. Regras de fronteira/validação/segurança: `tasks/despachos/_regras-comuns.md`.

## 1. Norte (por que esta onda existe)
Fio condutor #3 do fundador: **"visível-rotulado-first — nada existe só atrás de atalho; o que fica
invisível não existe para o leigo"**. O leigo tem skills/plugins/agents/hooks/MCPs instalados no
computador e **não vê nenhum**. A F2-4 cria a **vitrine**: ele vê o que tem, entende o que funciona
em qual terminal, e **conserta com 1 clique** — sem jargão e **sem que mostrar vire autorizar**.

Invariantes da onda (critério implícito de TODA fatia):
- **mostrar ≠ autorizar** — nenhum campo lido do disco (nome/descrição/frontmatter/JSON) decide
  identidade, ordem ou autorização. Gates de execução continuam onde estão (custódia/WorkspaceTrust).
- **manifest-first** — ler o manifesto pequeno (`installed_plugins.json` 13KB), **NUNCA** varrer a
  árvore pesada (`~/.claude/plugins` = 1,9GB com repos git → bomba no Linux futuro, inotify ENOSPC).
- **a tela nunca mente** — estado leigo SEMPRE com ação de 1 clique; estado sem ação é banido.
- **WCAG 1.4.1** — todo estado é **texto + ícone + cor**, nunca cor sozinha.

## 2. Inventário REAL do disco (medido nesta máquina — fonte: entrega-d4 §I)
| Poder | Caminho | Fonte da verdade |
|---|---|---|
| Skills (Claude) | `~/.claude/skills/` | 75 pastas, frontmatter de cada `SKILL.md`; scan ~0ms |
| Plugins (Claude) | `~/.claude/plugins/` | **`installed_plugins.json` (13KB)** — NUNCA a árvore de 1,9GB |
| Agents | `~/.claude/agents/` | 6 `.md` (scan raso) |
| Commands | `~/.claude/commands/` | 9 `.md` (scan raso) |
| Hooks | `~/.claude/settings.json → hooks` | config JSON (não pasta) |
| MCPs | `~/.claude.json → mcpServers` (global) + por-projeto + `.mcp.json` do projeto | MCP é **por-projeto** na prática |
| Codex | `~/.codex/config.toml → [mcp_servers.*]` | TOML por-CLI |
| Gemini/OpenCode/Copilot | `~/.<cli>/skills/` | pasta de skills por-CLI |

**Fato estruturante:** a mesma skill pode estar na pasta de um CLI e faltar na de outro → o estado
**"instalada-mas-não-funciona-NESTE-motor"** (inerte-aqui) é o caso **NORMAL** do multi-CLI, não raro.

## 3. Os 5 estados leigos (fonte: entrega-d4 §III.b — TEXTO+ícone+cor sempre, ação obrigatória)
| Estado (id técnico) | Rótulo leigo | Quando | Ação acoplada (obrigatória) |
|---|---|---|---|
| `Ready` | **"Pronto pra usar"** | SKILL.md válido na pasta do CLI do terminal focado | — |
| `UpdateAvailable` | **"Atualização disponível"** | manifesto/marketplace mais novo | botão **Atualizar** (manual, nunca automático) |
| `NeedsRepair` | **"Precisa de um conserto"** | frontmatter inválido / arquivo faltando (detectado no scan) | botão **Consertar** (re-scan/diagnóstico) |
| `InertHere` | **"Não funciona neste motor"** | skill na pasta do CLI X, terminal roda CLI Y | card **esmaecido + frase do porquê** + ação nomeada |
| `Disabled` | **"Desligada"** | SÓ se o app REALMENTE puder religar | toggle — **senão o estado não existe** (tela nunca mente) |

Regra de tradução: rótulo "Poder", **âncora do termo técnico no detalhe** ("(skill)" visível) — o leigo
cruza tutoriais externos que dizem "skill" (lição do rename do Obsidian). Strings via `const`/`copy_*` + teste anti-jargão.

## 4. O que JÁ EXISTE no código (REUSE — não reinvente; cold-review premia compor padrões provados)
### Core (`crates/lina-core`, `crates/lina-cli-profiles`, `crates/lina-bootstrap`)
- **`skill_index.rs:409` `parse_frontmatter`** — parser YAML-lite de `SKILL.md` (name/description/trigger/requires). **REUSE para skills.**
- **`cli_discovery.rs:167` `discover_clis_in`** + `DiscoveredCli` — **molde EXATO de "scanner"**: varredura → struct serializável → projeção em `ProjectedState.discovered_clis` (`events.rs:1909`). Copie o padrão.
- **`channel.rs:307/314` `replay`/`from_records`** — molde de projeção pura reconstruível (skip silencioso em decode-erro; último-vence).
- **`skills.rs:237` `read_disk_skills`** (bootstrap) — exemplo de varrer `<root>/<nome>/SKILL.md`.
- **`CliProfile` (`cli-profiles/lib.rs:147`)** — `#[serde(deny_unknown_fields)]`; tem `session_dir_pattern:200` mas **NÃO** tem `skills_dir`/`mcp_config_path` (campos a criar, aditivos `#[serde(default)]`).
- **`events.rs:280` `DomainEvent`** (`#[serde(tag="event")]`); `kind():1681`; padrão META (evento no-op no `apply` + projeção dedicada) — ex. `SkillSelected:1387`.
- Crates `lina-session-watch` e `lina-hooks` existem — cheque se há motor de watch reutilizável.
- **Pegadinha:** o app chama `build_skill_index(None)` (`bridge.rs:1636`) → hoje nem indexa as skills de disco do usuário (passa `None`). O ponto de extensão já existe.

### App (`app/lina-gpui` — FORA do workspace cargo; compila à parte com `cd app/lina-gpui`)
- **Molde de painel:** `src/mentality_panel.rs` (painel 2 níveis: resumo do papel → lista de cards;
  builder+`RenderOnce` `:329/:352`; consome `ui::{Panel,Button,Badge}` `:39`; `const` strings pt-br
  `:84-105`; teste anti-jargão `:436`; **nasce token-limpo = zero dívida no ratchet**). **COPIE este esqueleto.**
- **Catálogo `src/ui/`:** `Panel::surface:136`/`card:140`, `Button:118` (variantes por significado `:24`),
  **`Badge:89` (`BadgeTone:21` + `glyph():50` = tradução estado→cor+ícone WCAG 1.4.1)**, `Input`, `Modal`, `Toast`.
- **Tokens `src/theme.rs`:** `StateTokens:325` (success=verde/warning=âmbar/danger=vermelho), `AccentTokens.primary`
  (azul=mensagem), tipografia (IBM Plex Sans / Fraunces / JetBrains Mono), spacing 4/8/12/16/24/32, radius, motion.
  Acesso: `theme::active():779`. Consumo: `rgb(t.state.warning)`, `px(f32::from(t.spacing.md))`, `FontWeight(f32::from(t.typography.weight.semibold))`.
- **Token ratchet:** `tests/token_ratchet.rs` + `token_ratchet_snapshot.txt`. Arquivo novo deve nascer com
  **ZERO** `px(<literal>)`/`FontWeight::`/`text_size(px(literal))` — 100% via tokens, como `mentality_panel.rs`.
- **Fiação em `main.rs` (DONO = Maestro; você entrega o diff):** `mod` (~:19); campo de estado em
  `WorkspaceView` (~:502-656); método `render_powers_panel` (modelo `render_mentality_panel:2705`);
  `child` na faixa de overlays (~:6135); entrada na topbar (~:5730/5686) + atalho em `handle_key` (~:4478/4004).
- **Ponte dados core→view = `bridge.rs` (DONO = Maestro):** o painel recebe o inventário via um campo no
  `WorkspaceView` que o Maestro preenche chamando o scanner do core. Você renderiza contra o **contrato de
  view-model do ADR 0052** (com dados mock no teste enquanto a ponte não está fiada).

## 5. Fronteiras de arquivo desta onda (LEI — não cruze)
| Frente | Dono | Arquivos (cria/edita) |
|---|---|---|
| ADR-gate | **Arquiteto** | `docs/adr/0052-*.md` (novo) |
| Scanner core | **Terminal B (Ultra Code)** | `crates/lina-core/src/powers.rs` (NOVO) · `crates/lina-cli-profiles/src/lib.rs` (campos aditivos) |
| Painel UI | **Especialista em Telas** | `app/lina-gpui/src/powers_panel.rs` (NOVO) + diff-costura p/ `main.rs` |
| QA red-team | **Terminal R** | `crates/lina-core/tests/f2_4_powers.rs` (NOVO) + `app/lina-gpui/tests/f2_4_*.rs` se preciso (NOVO) |
| Costuras (largada+fim) | **Maestro 01** | `events.rs`, `lib.rs` (core), `main.rs`, `bridge.rs`, `Cargo.toml` (dep notify) |

**Largada do Maestro (já feita quando você receber):** o evento `PowerScanned` (se o ADR 0052 mandar),
o `pub mod powers;` no `lib.rs` e o `pub mod powers_panel;` no `main.rs` já existem como stub — você
preenche seu módulo sem tocar costura. Precisa de 1 linha numa costura? **Registre na entrega; o Maestro aplica.**

## 6. Gate de saída da onda (o que prova "pronto")
**Teste com leigo (camada b da régua):** "você tem a skill X? funciona neste terminal? conserte a que
está quebrada" — **5/5 respondem/agem sem ajuda**. Camada (f) do release: auditoria anti-manipulação
(mostrar≠autorizar por mutação). gpui **não roda headless** → a validação FINAL é na tela do fundador;
seu trabalho é lógica pura provada por teste + costura que compila + rebuild. **O ciclo só fecha na tela.**

## 7. Protocolo (além das regras comuns)
1. 1º ato: `touch .iniciado-<sua-fatia>` na raiz (ex.: `.iniciado-f2-4-core`).
2. Reporte ao Maestro: `lina ask "@Maestro 01" "<status>" --intent status` ao começar/terminar/travar.
3. Valide POR PACOTE, exit DIRETO (sem pipe): `cmd > log 2>&1; echo $?`. App: `cd app/lina-gpui` e rode a
   SUÍTE COMPLETA (inclui `token_ratchet` — o filtro de módulo não casa). Catraca bidirecional (`LINA_RATCHET_UPDATE=1` se a dívida cair).
4. Eventos aditivos `#[serde(default)]`; sem `unwrap()`/`expect()` em produção; testes não-vacuosos.
5. Entrega: `tasks/epico-f2/despachos/f2-4/.entrega-<sua-fatia>.md` — o que mudou (arquivo:linha),
   evidência (comandos+exit+nº testes), **costura para arquivos do Maestro (diff textual)**, achados/riscos.
   Última linha: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
