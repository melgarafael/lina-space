# DESPACHO — Terminal G · Onboarding de Direções Visuais (F2-4-5) · id: f2-4-design

> Leia `tasks/epico-f2/despachos/f2-4/_contexto.md` (seções 0, 4-App, 5, 7) + `tasks/despachos/_regras-comuns.md` ANTES.
> Carregue as skills `senior-frontend` + **`lina-design-doctrine`** + `lina-copy-doctrine`.

## CONTEXTO
Quando o leigo abre um projeto, ele não sabe que "direção visual" dar. A F2-4-5 entrega o **onboarding
de direções visuais**: uma galeria local onde ele escolhe uma direção (um `DESIGN.md` no formato
**OpenDesign**, vendorizado), com os **territórios do próprio Lina no topo** + a shortlist curada, e um
link **opt-in** SÓ para `open-design.ai`. É a porta de entrada estética — não é sobre Poderes/skills,
é uma frente **independente** que roda em paralelo (não espera o ADR 0052 nem o scanner).

**Decisão já tomada** (épico 38 §VIII.3 — território "Instrumento de Estúdio com temperatura do Ateliê";
NÃO re-decida): vendorizar os 12 da shortlist + territórios do Lina como `DESIGN.md` próprios no topo do
picker; link opt-in **só** open-design.ai (rejeitado: recomendar instalar o app OpenDesign).

## FUNÇÃO
Você é o **dev de UI desta frente (FRONTEND)**. Entrega a galeria + os `DESIGN.md` vendorizados + a costura
para o Maestro fiar em `main.rs`. **Não toca `main.rs`/`bridge.rs`** (dono = Maestro); entrega a costura como diff.

**Fronteira (LEI):**
- CRIA: `app/lina-gpui/src/design_directions.rs` (NOVO)
- CRIA: assets vendorizados — `app/lina-gpui/assets/design-directions/*.md` (os DESIGN.md; confirme o
  diretório de assets do app antes — alinhe com o padrão de embed existente).
- ENTREGA (diff): a costura de `main.rs` (entrada no onboarding/galeria + abrir a direção).
- NÃO toca: `powers_panel.rs` (é do Especialista em Telas), `main.rs`/`bridge.rs` (Maestro). Colisão = registre → Maestro.

## DIRECIONAMENTO
1. **Leia as fontes:** `tasks/pesquisa-f2/curadoria-opendesign.md` §II (os 12 da shortlist + o formato
   OpenDesign) e o épico 38 §VIII.3. Veja o molde de galeria existente `app/lina-gpui/src/gallery.rs`
   (galeria de Focos — mesmo padrão grid+card+apply) e `onboarding.rs` (fluxo de entrada).
2. **Formato `DESIGN.md` vendorizado (OpenDesign):** defina o formato (frontmatter + corpo) e vendorize
   os 12 da shortlist + os territórios do Lina (o T1+T3 do épico §VIII.1 no TOPO). Cada um é um arquivo
   local — **zero rede por padrão** (local-first, inv#2); o ÚNICO acesso externo é o link **opt-in** a
   open-design.ai, sinalizado (nada sai da máquina sem o clique).
3. **A galeria (UI):** grid de cards de direção (nome + preview/descrição), territórios do Lina no topo,
   "aplicar/escolher" com 1 clique. Consome `ui::{Panel,Button,Badge}` + tokens; **ZERO literais**
   `px()`/`FontWeight::` (o `token_ratchet` reprova dívida em arquivo novo — nasça token-limpo como
   `mentality_panel.rs`). Com a cara da casa (território decidido).
4. **Copy:** `const` pt-br leigo + teste anti-jargão (molde `mentality_panel.rs:436`). "Direção visual",
   não "design token"; explique o opt-in em 1 frase honesta.

## OBJETIVO
Uma galeria local e bonita onde o leigo escolhe a personalidade visual do projeto em 1 clique, com as
direções do Lina em destaque, tudo offline por padrão, e o único link externo claramente opt-in.

## RESULTADO ESPERADO
- `design_directions.rs` renderiza a galeria (territórios do Lina no topo + shortlist), escolha em 1 clique.
- DESIGN.md vendorizados presentes nos assets; zero acesso de rede sem o opt-in.
- Teste anti-jargão verde; **`token_ratchet` intacto**; suíte do app verde.
- Costura de `main.rs` descrita como diff preciso.
- Validação: `cd app/lina-gpui && cargo test -- --test-threads=1` (SUÍTE COMPLETA, inclui ratchet),
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — verdes, exit direto, lido de arquivo.

> gpui não roda headless → validação final na tela do fundador. Render provado por teste + costura que compila.

Reporte ao Maestro: `lina ask "@Maestro 01" "<status>" --intent status`. Entrega
`tasks/epico-f2/despachos/f2-4/.entrega-f2-4-design.md`. Última linha: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
