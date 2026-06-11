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
| 4  | 26 | 40.9/41.7ms | 0.8ms | 0.1 | 0.7 | 0.0 | 11.2ms | 27.3ms | 2 | 92 |
| 16 | 26 | 40.6/43.0ms | 1.3ms | 0.1 | 1.2 | 0.0 | 14.8ms | 24.3ms | 4 | 184 |
| 28 | 26 | 40.2/42.2ms | 1.3ms | 0.1 | 1.2 | 0.0 | 14.7ms | 24.0ms | 4 | 184 |

> **Medição 2026-06-10 (Maestro, run autônomo headless-driven: janela default, zoom default,
> LINA_AUTOQUIT_MS=90000, ≥20 janelas [PROF]/célula, última janela estável; logs /tmp/prof-*.log).**
> ⚠️ **`drawn` ficou capado a 2-4 pela janela/zoom DEFAULT** (culling correto: só o viewport
> desenha). A célula decisiva **drawn≥12** (o canvas real do fundador, zoom-out, referência
> histórica 12fps@28) NÃO foi capturada neste run — fica para a sessão de tela do fundador.
> Leituras firmes mesmo assim: **live 4→28 não muda NADA** (painéis fora da vista custam ~0 —
> culling provado por dados) e o frametime fica num **platô de ~40ms (26fps) já com drawn=2**.

Referência histórica (sonda `8ff5728`, fundador 2026-06-03, AGREGADO): ~54fps@N=4 → 18fps@N=16
→ 12fps@N=28, drawn culado a 12-16, ~5ms/painel. A curva acima é a versão DECOMPOSTA disso.

## 3. Cenário-alvo do produto — N=28, 8-12 ativos *(preencher na tela)*

| Cenário | [FPS] fps | frame p50/p95 | estágio dominante | top-5 painéis (assemble) |
|---------|-----------|---------------|-------------------|--------------------------|
| 28 painéis, 10 ativos | 26 | 40.1/42.6ms | trabalho: `layout_paint` 14.8ms · relógio: `present_vsync` 23.6ms (embute espera) | Carga 01 (burst) 0.30ms · Carga 08 (spinner) 0.28 · Carga 02 (spinner) 0.29 · Carga 09 (burst) 0.26 — **top28 listou só "de 4 medidos"**: os 24 ociosos/fora-da-vista NÃO geram amostra de assemble (culling já os poupa por completo) |

## 4. Diagnóstico assinado: estágio dominante *(preencher na tela)*

> Critério da peça: **qual estágio responde por >50% do frametime** no N=28 e no cenário-alvo.
> Se nenhum estágio agregado passar de 50%, registrar a distribuição e o follow-up (profiling
> nativo Instruments/Metal — fora desta story por decisão da peça). **O mesmo follow-up
> dispara se um estágio COMPOSTO dominar:** `layout_paint` (layout taffy + paint juntos) ou
> `present_vsync` (present + espera de vsync) acima de 50% nomeia um balde, não a avenida de
> otimização — a fonte 13.7 exige separar CPU-layout de GPU-paint antes de escolher técnica.
> (O smoke da fatia a N=4 já sugere o caso: `layout_paint` ~12× o `cpu_render`.)

- Estágio dominante @N=28: **`present_vsync` p50 24.0ms = 60% do frame de 40.2ms — mas é o
  balde COMPOSTO (present + ESPERA de vsync), então pelo protocolo acima ele NOMEIA um balde,
  não uma avenida.** A leitura decomposta honesta: o trabalho ativo mensurável =
  `cpu_render` 1.3 + `layout_paint` 14.7 ≈ **16ms — cabe em 1 ciclo de 60Hz (16.6ms)**; o
  frame de ~40ms (26fps) constante até com drawn=2 indica que o RELÓGIO é dominado por
  **cadência/pacing do shell** (espera entre apresentações), não por custo de painel. Dados:
  `[PROF] frames=53 live=28 drawn=4 | frame p50=40.2 | cpu_render p50=1.3 | layout_paint
  p50=14.7 | present_vsync p50=24.0` (/tmp/prof-n28.log, idem n4/n16/alvo).
- Estágio dominante @cenário-alvo: idem (40.1/1.3/14.8/23.6 — /tmp/prof-alvo.log).
- **Follow-up disparado (estágio composto dominante, conforme o protocolo):** story curta
  nova **"F1-5-1b — separar trabalho de espera"**: instrumentar a cadência de `notify`/pedidos
  de frame do shell (ou 1 sessão de Instruments) para responder *por que 16ms de trabalho viram
  40ms de frame em carga leve* — se for tick de animação/pacing de ~40ms do app, subir a
  cadência pode render 60fps em carga leve SEM tocar painel; se for vsync adaptativo do SO,
  documenta-se o teto. **Esta investigação vem ANTES de qualquer otimização por-painel** (é o
  espírito da decisão do fundador 2026-06-06: medir primeiro, não apostar na alavanca errada).
- Assinado por: **Maestro (Fable 5), 2026-06-10** — com a ressalva da célula drawn≥12 (§2),
  que pertence à sessão de tela do fundador.

## 5. Overhead da sonda — veredito *(preencher na tela)*

Protocolo (critério c): mesma carga (`LINA_LOAD=28`), comparar o `[FPS]` (sempre ativo) entre
`LINA_PROF=0` e `LINA_PROF=1`. O overhead da sonda = a diferença de frametime entre as rodadas.

| Rodada | [FPS] p50 | [FPS] p95 | fps |
|--------|-----------|-----------|-----|
| `LINA_PROF=0` (controle) | 39.3ms | 43.3ms | 26 |
| `LINA_PROF=1` (medição)  | 40.2ms | 42.2ms | 26 |

**Veredito explícito (sem número mágico — é parte do diagnóstico):**
- [x] A medição é VÁLIDA — overhead ≈ +0.9ms no p50 (~2%), p95 dentro do ruído entre rodadas,
  fps idêntico (26). A sonda não perturba de forma material o que mede.
- [ ] A sonda PERTURBA demais → simplificar a sonda ANTES de o baseline valer (o quê: ______)

## 6. TABELA DE ATIVAÇÃO das stories condicionais *(preencher na tela — gate de cada story)*

> Cada condicional só INICIA com a linha dela preenchida (decisão do fundador 2026-06-06).
> O gate de evidência abaixo é o da peça (`ondas-5-6.md`) — não reinterpretar.

| Story | Gate de evidência (da peça) | Evidência medida (citar `[PROF]`) | Veredito |
|-------|------------------------------|-----------------------------------|----------|
| **F1-5-3** dirty/damage por painel | re-montagem de painéis SEM mudança é fatia relevante do frametime no cenário-alvo (builds/frame ≈ N mesmo com poucos sujos E montagem no estágio dominante) | `assemble` TOTAL p50 = 1.2-1.3ms com drawn=4 (~0.3ms/painel) num frame de 40ms (**≈3%**); no cenário-alvo com `LINA_PROF_TOPK=28`, só 4 painéis geram amostra — os ociosos fora da vista **nem montam** (culling já os poupa; soma da lista completa ≈ custo dos 4 drawn). A montagem NÃO está no estágio dominante (§4). | **DESCARTADA para o objetivo de FPS desta onda** — confirma a refutação do "3x" (13.7); o ganho residual (latência de input/bateria) fica registrado como candidata de eficiência F2. ⚠️ Reabrir SÓ se a célula drawn≥12 do fundador mostrar `assemble` agregado relevante. |
| **F1-5-4a** snapshot congelado (mecanismo) | ≥1 consumidora ativada (F1-5-3 OU F1-5-4b) — mecanismo não se constrói sem consumidor | F1-5-3 DESCARTADA; F1-5-4b pendente da célula drawn≥12 | **PENDENTE** (segue a 4b; sem consumidor ativado, não inicia) |
| **F1-5-4b** LOD da periferia | custo dos painéis desenhados FORA do conjunto de foco é fatia relevante do frametime no cenário-alvo | Evidência parcial A FAVOR: `layout_paint` é o maior trabalho ativo e cresce com drawn/runs (drawn 2→4, runs 92→184: 11.2→14.8ms ≈ +1.8ms/painel desenhado) — reduzir o que a periferia desenha ataca o estágio certo. MAS o run autônomo não capturou drawn≥12 (janela default cula a 4) — a fatia REAL da periferia no canvas do fundador não está medida. | **PENDENTE DA CÉLULA drawn≥12** (sessão de tela do fundador: zoom-out no canvas real, ler `[PROF]`). Nem ativada nem descartada sem o dado — não apostar às cegas é a decisão do fundador. |
| **F1-5-4c** fila de prioridade + K dinâmico | com as condicionais anteriores ativas (e/ou F1-5-5), a **RE-medição** ainda mostra excesso de painéis acima do budget — última a avaliar; depende da re-medição, NÃO deste baseline | (re-medição posterior) | PENDENTE DE RE-MEDIÇÃO |
| *(nova, do diagnóstico §4)* **F1-5-1b** separar trabalho de espera | estágio composto dominante no baseline (protocolo §4) | trabalho ativo ≈16ms cabe num ciclo de 60Hz, mas frame=40ms constante até com drawn=2 — cadência/pacing domina o relógio em carga leve | **ATIVADA — investigação ANTES de qualquer otimização por-painel** (story curta: sonda de notify/Instruments) |

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
