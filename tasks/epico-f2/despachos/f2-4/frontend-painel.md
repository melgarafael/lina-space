# DESPACHO — Especialista em Telas · Área de Poderes UI (F2-4-3 + F2-4-4) · id: f2-4-ui

> Leia `tasks/epico-f2/despachos/f2-4/_contexto.md` + `tasks/despachos/_regras-comuns.md` +
> **`docs/adr/0052-area-de-poderes-scan-determinista.md`** (o contrato de view-model — ESPERE-o) ANTES.
> Carregue as skills `senior-frontend` + **`lina-design-doctrine`** + **`lina-copy-doctrine`**.

## CONTEXTO
O leigo tem dezenas de Poderes instalados (75 skills + 33 plugins + agents/hooks/MCPs) e **não vê
nenhum**. Você constrói a **vitrine**: um painel onde ele vê o que tem, entende o que funciona em qual
terminal e **conserta com 1 clique** — em pt-br, sem jargão, com a cara da casa (território "Instrumento
de Estúdio com a temperatura do Ateliê" — épico 38 §VIII; cor semântica fixa âmbar/verde/vermelho/azul).

A pesquisa de UX está cravada em `tasks/pesquisa-f2/entrega-d4-comandos-menus.md` — **§III.b (os 5
estados), §II.A4 (progressive disclosure: MÁX 2 NÍVEIS, porta visível rotulada), §II.A5 (estado sem
ação é banido)**. Se sua tela contradisser a fonte, PARE e escale.

## FUNÇÃO
Você é o **Especialista em Telas (FRONTEND)**. Entrega o painel novo + a copy leiga embutida + a costura
para o Maestro fiar em `main.rs`. **Não toca `main.rs`/`bridge.rs`** (dono = Maestro); entrega a costura como diff.

**Fronteira (LEI):**
- CRIA: `app/lina-gpui/src/powers_panel.rs`
- ENTREGA (diff, não edita): a costura de `main.rs` (5 toques) na sua entrega.
- O Maestro já fez a largada: `pub mod powers_panel;` no `main.rs` e o campo de inventário no
  `WorkspaceView` já existem como stub; você preenche o módulo e descreve a costura final.

## DIRECIONAMENTO
**Copie o esqueleto de `src/mentality_panel.rs`** — é o molde perfeito (painel 2 níveis, builder+`RenderOnce`,
consome `ui::{Panel,Button,Badge}`, `const` strings pt-br + teste anti-jargão `:436`, nasce token-limpo).

### F2-4-3 — O painel (2 níveis, máx 2 — NN/g)
- **Nível 1 = resumo contado e traduzido:** ex. "75 Poderes · 33 Plugins · 1 Gatilho" (contadores do
  `PowerInventory.counts`). Linguagem leiga ("Poderes", "Gatilho" p/ hook). Empty-state honesto se vazio.
- **Nível 2 = lista com origem rotulada:** cada Poder = um `Panel::card` com nome + descrição leiga +
  **origem rotulada** (global / deste projeto / "do Gemini") + badge de estado. NADA essencial num 3º nível.
- **Porta de entrada VISÍVEL:** o painel NÃO pode existir só atrás de atalho (fio condutor #3 + regra
  anti-Zed §III.d.7 da entrega-d4). Entrega a costura: entrada rotulada na topbar (modelo do 🔔 em
  `main.rs:5730`) + atalho pareado escrito nela. "Poderes" como rótulo.
- Consome `ui::{Panel,Button,Badge}` e tokens (`theme::active()`). **ZERO literais** `px()`/`FontWeight::`/
  `text_size(px(literal))` — 100% via tokens (o `token_ratchet` reprova qualquer dívida em arquivo novo).

### F2-4-4 — Os 5 estados leigos (texto+ícone+cor SEMPRE — WCAG 1.4.1)
Mapeie `PowerState` → `BadgeTone` + glyph (reuse `ui/badge.rs:50 glyph()` — cor nunca sozinha) e a ação acoplada:
| Estado | Rótulo (const pt-br) | BadgeTone | Ação de 1 clique (obrigatória) |
|---|---|---|---|
| `Ready` | "Pronto pra usar" | Success | — |
| `UpdateAvailable` | "Atualização disponível" | Info | botão **Atualizar** (manual) |
| `NeedsRepair` | "Precisa de um conserto" | Warning | botão **Consertar** |
| `InertHere` | "Não funciona neste motor" | Neutral (card esmaecido) | **frase do porquê** ("está na pasta do Gemini; este terminal usa Claude Code") + ação nomeada |
| `Disabled` | "Desligada" | Neutral | toggle — só renderize se o app puder religar (senão NÃO mostre: a tela nunca mente) |
- **Estado sem ação é BANIDO** (§II.A5). **Âncora do termo técnico no detalhe:** mostre "(skill)"/"(MCP)"
  discreto — o leigo cruza tutoriais externos. **mostrar ≠ autorizar:** o card exibe e oferece a ação, mas
  a EXECUÇÃO da ação passa pelo gate existente (você dispara o gesto; quem autoriza é o supervisor/custódia).

### Copy (você dobra a função de Writer — a fonte D4 já dá os 5 rótulos)
- `const` pt-br no próprio módulo (molde `mentality_panel.rs:84-105`) + um **teste anti-jargão** espelhando
  `mentality_panel.rs:436` (falha se vazar "frontmatter"/"manifest"/"mcp"/"hook" cru na superfície leiga).
  Use a `lina-copy-doctrine`: rótulo que diz a CONSEQUÊNCIA, zero molde de IA. Se a copy ficar fraca, sinalize
  ao Maestro que vale um par de olhos de Writer.

### Dados (contrato, não implementação)
Você renderiza contra o view-model do **ADR 0052** (`PowerInventory`/`Power`). Enquanto a ponte
`bridge.rs`→view (do Maestro) não está fiada, renderize com um **mock** no teste/preview. A ponte real
é costura do Maestro — você descreve na entrega qual campo do `WorkspaceView` lê.

## OBJETIVO
Uma vitrine que um leigo abre, entende em 5s o que tem, e sabe consertar o que está quebrado — com a
cara da casa, zero jargão, zero dívida de token, e sem que ver um Poder signifique autorizá-lo.

## RESULTADO ESPERADO
- `powers_panel.rs` renderiza nível 1 (resumo) → nível 2 (lista com origem+estado+ação), 5 estados com
  texto+ícone+cor, contra um `PowerInventory` mock.
- Teste anti-jargão verde; **`token_ratchet` intacto** (arquivo novo = zero dívida); suíte do app verde.
- Costura de `main.rs` descrita como diff preciso (mod já existe; campo de estado; método render;
  child no overlay ~:6135; entrada na topbar ~:5730 + atalho ~:4478).
- Validação: `cd app/lina-gpui && cargo test -- --test-threads=1` (SUÍTE COMPLETA, inclui ratchet),
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — verdes, exit direto.

> gpui NÃO roda headless → a validação FINAL é na tela do fundador. Seu trabalho: render provado por
> teste (mock) + costura que compila. O Maestro recompila o app; o fundador valida na tela.

Reporte ao Maestro: `lina ask "@Maestro 01" "<status>" --intent status`. Entrega
`tasks/epico-f2/despachos/f2-4/.entrega-f2-4-ui.md`. Última linha: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
