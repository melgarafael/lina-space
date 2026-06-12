# Plano da Pesquisa Profunda — Fase 2 (UI/UX) · 2026-06-12

> **Maestro:** Terminal B (Ultra Code) · **Método:** skill `deep-research-fase` (interna-primeiro, eval-first, datada, refutação registrada)
> **Decisão do fundador (2026-06-12):** F2 = Interface e Experiência do Usuário. Coordenação/inteligência → F3 (specs 35/36 renomeadas; addendum ADR 0010).

## Meta (critério de saída do Maestro)

Relatório consolidado de pesquisa F2 no vault (`Debriefing Vibe Coding/`), base para o épico F2 e atualização do `_HANDOFF`. Pronto quando: toda afirmação ancorada e datada · refutação tentada em tudo que é load-bearing · custo (frametime) e conformidade (N/A justificado: app local-first, sem dado pessoal novo, sem Meta API nesta fase) checados · coerência com as restrições internas abaixo confrontada · cold-review PASS.

## Escopo da F2 (do debriefing editado, seção "Fazendo na Fase 2")

1. Refatorar o design (referências → design system → identidade → refatoração) — coração da fase.
2. Organizar, redimensionar e mover terminais no canvas.
3. Área de skills do PC · 4. Área de skills não-carregáveis via "/" · 5. Área de agents/hooks/commands/MCPs.
6. OpenDesign no onboarding. + Comandos/menus melhores, navegabilidade, doutrina de design permanente.
Candidato herdado do ADR 0010: Ghost wires + Linha do Tempo (superfície visual — decisão no épico).

## Restrições internas herdadas (A5 — TODA recomendação respeita ou contesta EXPLICITAMENTE)

| # | Restrição | Fonte |
|---|---|---|
| R1 | Design system nasce em **gpui** (Rust, GPU-first, sem JS/CSS; pin de SHA + vendoring) | `CLAUDE.md` repo §Stack · vault `31`/`33` |
| R2 | **Core/shell split**: core não importa tipos de toolkit; tokens/temas não podem soldar o core ao gpui (porta Slint viva via `UiHost`) | `CLAUDE.md` §Âncoras (inv#7) |
| R3 | A cena do canvas mantém o slot do **PortalEngine/ExternalTextureLayer** (browser futuro) | `CLAUDE.md` §Âncoras |
| R4 | **Não-técnico-first**: zero jargão na superfície, nunca tela em branco, 1º agente <2h | `CLAUDE.md` inv#6 |
| R5 | **Event log = fonte da verdade**: posição/tamanho/organização de terminal no canvas é evento+projeção reconstruível, nunca estado solto | `CLAUDE.md` inv#4 · ADR 0001 |
| R6 | A11y é 1ª classe: AccessKit; live-region selada (ADR 0028); canvas por teclado já existe (F1-2-6) — UX nova não regride | `docs/adr/0028` · épico 34 |
| R7 | Perf é restrição de design: 120Hz, sonda `[PROF]` (F1-5-1), frametime por célula — visual novo não pode regredir | épico 34 F1-5 |
| R8 | Doutrina anti-slop de design vigente: banido default-de-IA (fonte por inércia, gradiente roxo, glassmorphism genérico); direção única com coragem | `assets/lina-doctrine/` · skill lina-design-doctrine |
| R9 | Copy congelada como fonte única de strings (F1-4) — menus/labels novos passam por lá | épico 34 |
| R10 | Definições operacionais (inclui estética) já seladas | `docs/adr/0019` |
| R11 | Pesquisa interna existente — pesquisar só o DELTA: `13.3` (canvas+DS), `13.13` (UX permissão), `13.7` (render-scale), `22` (fluxo de telas), `12` (benchmark) | vault Debriefing |
| R12 | **A fundação do design system JÁ EXISTE** (achado do Maestro, 2026-06-12): `app/lina-gpui/src/theme.rs` (695 linhas, F1-2-1) = tokens nomeados por grupo (Surface/Text/Accent/State/Focus/Terminal), `ColorScale` dark/light, 8 acentos curados, gate WCAG em CI + lint anti-cor-literal, persistência local-first, zero acoplamento core↔tema. `palette.rs` (W4-2) = Cmd-K com fuzzy-match, modelo puro testável. **A F2 evolui isso (tipografia/espaçamento/motion/componentes/identidade), não recomeça** — recomendação que ignorar R12 é refutada na síntese | `app/lina-gpui/src/theme.rs` · `src/palette.rs` |

## Dimensões e donos

| ID | Dimensão | Dono | Entrega |
|---|---|---|---|
| D0 | **Régua/eval de UX** (PRIMEIRO): como medir "visual único e viciante" + usabilidade leigo + perf + a11y | Terminal D (QA) | `entrega-d0-eval.md` |
| D1 | **Identidade visual e referências** (anti-genérico; apps desktop de alto padrão 2025-2026) | Terminal C (FRONTEND) | `entrega-d1-identidade.md` |
| D2 | **Design system em gpui** (tokens/temas/componentes; como o Zed faz; porta Slint) | Terminal A (ARQUITETO) | `entrega-d2-gpui-ds.md` |
| D3 | **UX de canvas**: navegação, zoom, organização, redimensionamento, persistência de layout | Terminal E | `entrega-d3-canvas-ux.md` |
| D4 | **Comandos, menus, paleta, atalhos + áreas de visibilidade** (skills/agents/MCPs) e discoverability | Core A2A | `entrega-d4-comandos-menus.md` |
| V | **Verificação adversarial** dos achados load-bearing/suspect (onda 2) | Red Team (QA) | `entrega-v-verificacao.md` |

Ordem: D0 despachada PRIMEIRO (eval antes de arquitetura); D1-D4 em paralelo; V após as entregas; síntese do Maestro por último (só fecha depois do D0 voltar).

## Quality gates (copiados da skill — valem para todos)

- [ ] Recência: tudo datado; domínio rápido exige 2025+ (UNDATED → suspect)
- [ ] Falsear, não confirmar: refutação tentada e registrada por achado
- [ ] Interno antes de externo: ler R11 ANTES de buscar; só o delta
- [ ] Eval antes de arquitetura: síntese não fecha sem D0
- [ ] Custo p/ escopo da fase: frametime/render budget (não token); LGPD/Meta = N/A justificado (local-first, sem dado pessoal novo nesta fase)
- [ ] Hype-filter: praticante com produção real > vendor/leaderboard/award-site
- [ ] Gate de citação: nenhuma URL sem fetch real
- [ ] Parada: piso ~5 / teto ~15 buscas por agente; saturação = 2 rodadas sem novidade

## Protocolo

- Workers escrevem a entrega em `tasks/pesquisa-f2/entrega-*.md` (repo), formato de retorno da skill (CLAIM/FONTE/DATA/CONFIANÇA/REFUTAÇÃO/SUSPECT/LOAD-BEARING + CONFLITOS·LACUNAS·RECÊNCIA).
- Workers NÃO commitam; NÃO editam o vault; NÃO editam entrega de peer.
- Reporte: `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status` ao começar/terminar/travar; última linha da entrega = `PRONTO: <resumo 1 linha>` ou `BLOCKED: <motivo>`.
- Síntese final: Maestro escreve `37 - Pesquisa F2 - Relatorio Consolidado (UI-UX).md` no vault (template relatorio-final da skill) + cold-review antes de declarar pronto.
