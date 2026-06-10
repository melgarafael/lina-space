# Baseline de Render — F1-5-1 · Profiling decomposto (`[PROF]`)

> **ESQUELETO (entrega da fatia headless).** O gpui não roda headless: os DADOS desta página
> saem da **sessão de tela do fundador**, lidos pelo Maestro no stderr (protocolo §4.1 do
> handoff). Tudo que está `(preencher)` é da sessão de medição; a estrutura, o cenário de carga
> e a sonda são desta fatia. **Nenhuma otimização da onda F1-5 inicia sem a linha dela na
> tabela de ativação (§6)** — decisão do fundador 2026-06-06.

---

## 1. Como rodar (reprodutível)

Pré-requisito: macOS (a matriz oficial roda no Mac do fundador). Gerador: `sh` puro — **sem
CLI de IA real** (decisão da peça: shell + script gerador basta).

```sh
cd app/lina-gpui
cargo build

# SEMPRE com LINA_WS_ROOT isolado: sem ele o app abre o workspace PERSISTENTE de produção e
# cada painel de carga apenda 3 admissões sintéticas no event log canônico (fonte da verdade,
# inv#4) + entra no agents.json — a matriz inteira contaminaria a forense para sempre.
export LINA_WS_ROOT=/tmp/prof-ws

# ── Matriz N∈{4,16,28} (todos os painéis ATIVOS, alternando rajada/spinner):
LINA_PROF=1 LINA_LOAD=4  cargo run 2> /tmp/prof-n4.log
LINA_PROF=1 LINA_LOAD=16 cargo run 2> /tmp/prof-n16.log
LINA_PROF=1 LINA_LOAD=28 cargo run 2> /tmp/prof-n28.log

# ── Cenário-alvo do produto: 8-12 ativos, resto ocioso (fundador 2026-06-06).
#    LINA_PROF_TOPK=28 = top-K vira a lista COMPLETA por painel — o gate F1-5-3 precisa do
#    custo dos painéis OCIOSOS desenhados (os baratos nunca entrariam num top-5):
LINA_PROF=1 LINA_LOAD=28 LINA_LOAD_ACTIVE=10 LINA_PROF_TOPK=28 cargo run 2> /tmp/prof-alvo.log

# ── Controle do OVERHEAD da sonda (mesma carga, sonda DESLIGADA — só o [FPS]):
LINA_PROF=0 LINA_LOAD=28 cargo run 2> /tmp/prof-n28-controle.log

# Ler os blocos (1 por ~120 frames; descartar a 1ª janela de cada rodada — warm-up):
grep '\[PROF\]' /tmp/prof-n28.log
grep '\[FPS\]'  /tmp/prof-n28.log /tmp/prof-n28-controle.log
```

Regras da sessão: **não** setar `LINA_DEMO` (posições colidem com a grade de carga); janela em
tamanho/zoom padrão (o culling decide `drawn` — anotar se mexer); ≥5 janelas `[PROF]` por
célula da matriz; usar p50/p95 da janela mais estável (output já em regime).

**O que cada modo do gerador faz** (`tools/loadgen.sh` — taxas FIXAS no script):
- `burst` — rajada contínua de linhas coloridas (~250 linhas/s); painéis pares.
- `spinner` — spinner ANSI in-place via `\r` (~20 updates/s) + status colorido ocasional; ímpares.
- `silence` — 1 linha e silêncio (ocioso de verdade); painéis além de `LINA_LOAD_ACTIVE`.

**Contrato das linhas (o que cada número É — ver doc do `src/prof.rs`):**

```
lina-gpui: [PROF] frames=… live=… drawn=… | frame p50=…ms p95=…ms | cpu_render p50=… p95=… |
           poll p50=… | assemble p50=… | chrome p50=… | layout_paint p50=… p95=… |
           present_vsync p50=… p95=… | runs p50=…
lina-gpui: [PROF] top5 (de N medidos) assemble: <painel> p50=…ms p95=…ms runs=… · …
```

- `frame` = intervalo render→render (mesma medida do `[FPS]`; ociosidade descartada).
- `cpu_render = poll + assemble + chrome` (CPU dentro do `render`); `assemble` tem o
  detalhamento POR PAINEL no top-K (inclui lock do grid + snapshot — contenção é custo real).
- `layout_paint` = retorno do `render` → paint do sentinela (layout taffy + paint dos elementos).
- `present_vsync` = paint do sentinela → próximo `render` (present da GPU **+ espera de
  vsync**; em saturação a espera →0 e o número aproxima o present real).
- `runs` = spans de grid montados/frame (proxy de quads: cada run ≈ 1 quad de fundo + 1 run de texto).

---

## 2. Matriz N∈{4,16,28} — todos ativos *(preencher na tela)*

| N  | [FPS] fps | frame p50/p95 | cpu_render p50 | poll p50 | assemble p50 | chrome p50 | layout_paint p50 | present_vsync p50 | drawn | runs p50 |
|----|-----------|---------------|----------------|----------|--------------|------------|------------------|-------------------|-------|----------|
| 4  | (preencher) | | | | | | | | | |
| 16 | (preencher) | | | | | | | | | |
| 28 | (preencher) | | | | | | | | | |

Referência histórica (sonda `8ff5728`, fundador 2026-06-03, AGREGADO): ~54fps@N=4 → 18fps@N=16
→ 12fps@N=28, drawn culado a 12-16, ~5ms/painel. A curva acima é a versão DECOMPOSTA disso.

## 3. Cenário-alvo do produto — N=28, 8-12 ativos *(preencher na tela)*

| Cenário | [FPS] fps | frame p50/p95 | estágio dominante | top-5 painéis (assemble) |
|---------|-----------|---------------|-------------------|--------------------------|
| 28 painéis, 10 ativos | (preencher) | | | |

## 4. Diagnóstico assinado: estágio dominante *(preencher na tela)*

> Critério da peça: **qual estágio responde por >50% do frametime** no N=28 e no cenário-alvo.
> Se nenhum estágio agregado passar de 50%, registrar a distribuição e o follow-up (profiling
> nativo Instruments/Metal — fora desta story por decisão da peça). **O mesmo follow-up
> dispara se um estágio COMPOSTO dominar:** `layout_paint` (layout taffy + paint juntos) ou
> `present_vsync` (present + espera de vsync) acima de 50% nomeia um balde, não a avenida de
> otimização — a fonte 13.7 exige separar CPU-layout de GPU-paint antes de escolher técnica.
> (O smoke da fatia a N=4 já sugere o caso: `layout_paint` ~12× o `cpu_render`.)

- Estágio dominante @N=28: **(preencher)** — dados: (citar linhas `[PROF]`)
- Estágio dominante @cenário-alvo: **(preencher)** — dados: (citar)
- Assinado por: (Maestro/fundador, data)

## 5. Overhead da sonda — veredito *(preencher na tela)*

Protocolo (critério c): mesma carga (`LINA_LOAD=28`), comparar o `[FPS]` (sempre ativo) entre
`LINA_PROF=0` e `LINA_PROF=1`. O overhead da sonda = a diferença de frametime entre as rodadas.

| Rodada | [FPS] p50 | [FPS] p95 | fps |
|--------|-----------|-----------|-----|
| `LINA_PROF=0` (controle) | (preencher) | | |
| `LINA_PROF=1` (medição)  | (preencher) | | |

**Veredito explícito (sem número mágico — é parte do diagnóstico):**
- [ ] A medição é VÁLIDA (a sonda não perturba de forma material o que mede), ou
- [ ] A sonda PERTURBA demais → simplificar a sonda ANTES de o baseline valer (o quê: ______)

## 6. TABELA DE ATIVAÇÃO das stories condicionais *(preencher na tela — gate de cada story)*

> Cada condicional só INICIA com a linha dela preenchida (decisão do fundador 2026-06-06).
> O gate de evidência abaixo é o da peça (`ondas-5-6.md`) — não reinterpretar.

| Story | Gate de evidência (da peça) | Evidência medida (citar `[PROF]`) | Veredito |
|-------|------------------------------|-----------------------------------|----------|
| **F1-5-3** dirty/damage por painel | re-montagem de painéis SEM mudança é fatia relevante do frametime no cenário-alvo (builds/frame ≈ N mesmo com poucos sujos E montagem no estágio dominante) | (preencher) | ATIVADA / DESCARTADA |
| **F1-5-4a** snapshot congelado (mecanismo) | ≥1 consumidora ativada (F1-5-3 OU F1-5-4b) — mecanismo não se constrói sem consumidor | (derivado das linhas acima/abaixo) | ATIVADA / DESCARTADA |
| **F1-5-4b** LOD da periferia | custo dos painéis desenhados FORA do conjunto de foco é fatia relevante do frametime no cenário-alvo | (preencher) | ATIVADA / DESCARTADA |
| **F1-5-4c** fila de prioridade + K dinâmico | com as condicionais anteriores ativas (e/ou F1-5-5), a **RE-medição** ainda mostra excesso de painéis acima do budget — última a avaliar; depende da re-medição, NÃO deste baseline | (re-medição posterior) | PENDENTE DE RE-MEDIÇÃO |

Nota de leitura para F1-5-3 (**leia nas DUAS direções**): hoje todo painel desenhado re-monta
todo frame — builds/frame == `drawn` por construção; o contador explícito `builds por painel`
é critério da PRÓPRIA F1-5-3 (a) e entra no `[PROF]` quando/se ela ativar. O que ESTA baseline
responde: quanto custa a `assemble` dos painéis ociosos (silence) vs ativos. **Direção ativa:**
painéis sem output novo com custo relevante por painel ⇒ re-montagem de painel limpo é fatia
real. **Direção negativa (a armadilha):** AUSÊNCIA no top-5 NÃO é evidência de custo zero — os
ociosos são individualmente baratos mas podem ser fatia AGREGADA relevante (Σ de ~18 painéis).
Por isso o cenário-alvo roda com `LINA_PROF_TOPK=28` (lista completa por painel, com o
denominador "de N medidos" na linha): o veredito DESCARTADA para F1-5-3 só vale somando o
custo dos ociosos da lista completa, nunca pela ausência deles num top-5.

Nota sobre a linha F1-5-4c: o critério (d) da story pede literalmente "cada condicional marcada
ATIVADA ou DESCARTADA", mas o gate da PRÓPRIA F1-5-4c (peça, linha 80) diz que ela "depende da
re-medição, não do baseline" — a peça se contradiz internamente. Esta tabela resolve a favor do
gate específico (linha 4c = PENDENTE DE RE-MEDIÇÃO até as condicionais anteriores rodarem);
**o Maestro referenda (ou reverte) esta leitura ao aceitar a fatia** — registrado também na
entrega (.entrega-f1-5-1.md), conforme regras-comuns §8.

## 7. Limitações conhecidas da medição (honestidade)

- `layout_paint`/`present_vsync` vêm do SENTINELA de paint (canvas tamanho-zero, último filho
  da cena): é a melhor aproximação disponível no gpui SEM forkar o framework — `present_vsync`
  embute a espera de vsync (só aproxima o present real em saturação); o sentinela pinta em
  ordem de árvore (overlays pintam perto dele, custo deles cai majoritariamente em `layout_paint`).
- `assemble` por painel inclui o lock do grid + `screen()` (contenção com o pump É custo da
  thread de render — está no número de propósito). Layout/paint NÃO são atribuíveis por painel.
- O custo de CPU FORA da thread de render (reader de PTY, advance do VT, pumps) não aparece
  aqui — a story decompõe o frametime do shell; CPU total é outra sonda.
- Frames que fecham contra ociosidade (>250ms) são descartados (mesma honestidade do `[FPS]`:
  "medimos enquanto desenha") — cargas 100% silence quase não geram janelas, por construção.
- `runs` conta SÓ os spans do grid de texto (o conteúdo dos terminais) — elementos do chrome
  da cena (headers de card, topbar, rodapé, modais) ficam fora do proxy; o TEMPO deles está
  nas fases `assemble`/`chrome`. Não leia `runs` como "elementos totais da cena".
- A emissão do bloco `[PROF]` (sort+format+eprintln, 1×/janela) é atribuída de propósito à
  fase `poll` do frame que fecha a janela — 1 amostra inflada por ~120 frames (p100; p50/p95
  de poll seguem robustos).
- Gerador unix-only (`/bin/sh`); no Windows o spawn degrada com log (medição oficial = Mac).
  O caminho default do `loadgen.sh` é de COMPILE-TIME (`CARGO_MANIFEST_DIR`) — binário movido
  de máquina/repo renomeado: sete `LINA_LOADGEN=/caminho/real/loadgen.sh`.
- Painéis homônimos agregariam no mesmo top-K (a carga gera nomes únicos `Carga NN (modo)`;
  num Espaço real com nomes repetidos o ranking fundiria as amostras — dívida registrada).

---

*Fatia headless entregue por: Especialista em IA (r1-f1-5-1) · 2026-06-10 · sonda + carga +
esqueleto. Dados = sessão de tela (Maestro + fundador).*
