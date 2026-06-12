# Baseline de Input Latency (keypress-to-photon) — F2-0-2

> **ESQUELETO INSTRUMENTADO (análogo ao `prof-baseline.md` da F1-5-1).** A latência de input
> **nunca foi medida** no Lina (lacuna L7 da régua D0 — a sonda `[PROF]` mede frametime, não
> tecla→pixel). Esta página registra: o protocolo completo PRONTO para a sessão de tela do
> fundador, o **bloqueio técnico honesto** que impediu a medição autônoma (§5), e tudo que JÁ
> foi validado nesta máquina (§2). Tudo que está `(preencher)` sai da sessão de tela.
> **Dono:** Terminal D (QA) · 2026-06-12 · despacho `tasks/epico-f2/despachos/r1-qa-regua.md`.

---

## 1. O que medimos e o gate (régua D0, camada d — achado A6)

**Input latency = keypress-to-photon:** do keydown físico até o glifo mudar na tela, no
terminal FOCADO do Lina. É outra métrica que frametime: amostra o caminho completo
teclado→PTY→VT→render→present, milhares de vezes por dia de uso — **a cauda é o que o
usuário sente** (Dan Luu).

| Métrica | Gate da F2 (D0) | Fundamento |
|---|---|---|
| p50 idle E sob carga | **≤25ms** | cobre o argumento motor-control de Fatin (efeito motor <20ms mesmo sem percepção consciente) |
| p99 sob carga | **≤50ms** | 50ms < JND de 55ms do drag indireto (interação mais sensível do canvas — Deber et al. CHI 2015) e na fronteira "laggy" de Carmack |

**Expectativa honesta do ANTES (desta página, não da sessão):** o dry-run de hoje (§2) mediu
frame p50 ~41ms / 26fps sob carga — com cadência de apresentação de ~40ms, a latência de
input fica quantizada por essa cadência (um keypress espera, em média, meia cadência só para
ter chance de aparecer). **O gate quase certamente FALHA hoje**, e deve falhar: este baseline
é o "antes" que a F1-5-1b (diagnóstico do pacing — trabalho ativo de 16ms virando frame de
40ms) e as otimizações da F2 vão comparar. Medir antes de otimizar (decisão do fundador
2026-06-06).

## 2. Validado HOJE nesta máquina (run autônomo, 2026-06-12)

O cenário de carga e o comando de lançamento do protocolo **rodaram e produziram dados**
(janela abriu, 20 janelas `[PROF]` no stderr, autoquit limpo, exit 0):

```sh
# Binário empacotado atual (build 2026-06-11 23:58) — suporta todas as envs necessárias
# (verificado por strings + run): LINA_LOAD, LINA_LOAD_ACTIVE, LINA_PROF, LINA_WS_ROOT,
# LINA_AUTOQUIT_MS, LINA_LOADGEN.
rm -rf /tmp/f2-lat-ws && \
LINA_WS_ROOT=/tmp/f2-lat-ws \
LINA_PROF=1 LINA_LOAD=12 LINA_LOAD_ACTIVE=10 \
LINA_LOADGEN="$REPO/app/lina-gpui/tools/loadgen.sh" \
"$REPO/dist/Lina.app/Contents/MacOS/lina-gpui"
```

Dados ambiente colhidos (referência para interpretar a latência, NÃO a medição de latência):
`[PROF] frames=52 live=12 drawn=4 | frame p50=41.3ms p95=41.9ms` · `[FPS] p50=41.4 p95=41.9
p99=42.0 fps=26` (log `/tmp/f2-lat-dryrun.log`). Coerente com o `prof-baseline.md` (platô de
~40ms já com drawn=2 — o pacing domina, F1-5-1b).

Regras herdadas do `prof-baseline.md` que VALEM aqui: **sempre `LINA_WS_ROOT` isolado**
(sem ele o app abre o workspace de produção e a medição contamina o event log canônico —
inv#4); **nunca `LINA_DEMO`** (posições colidem com a grade de carga); janela em
tamanho/zoom padrão (anotar se mexer).

## 3. Protocolo da sessão de tela — caminho principal: Typometer

### 3.0 Preparação (uma vez, ~10 min — comandos prontos)

```sh
# 1. Java (não existe na máquina — §5.1):
brew install --cask temurin          # JDK Temurin; Java 8+ basta ao Typometer

# 2. Typometer — caminho RECOMENDADO nesta máquina (Apple Silicon + Retina): o fork
#    frarees, buildado do master (captura nativa Cocoa + tolerância de cor p/ Retina;
#    o fix de Apple Silicon de 2023 NÃO está na release binária — ver §4):
brew install maven
git clone https://github.com/frarees/typometer && cd typometer
mvn clean package && java -jar target/typometer-*.jar

#    Fallback (upstream 2017, Intel/teste rápido):
#    curl -LO https://github.com/pavelfatin/typometer/releases/download/v1.0.1/typometer-1.0.1-bin.zip
#    unzip typometer-1.0.1-bin.zip && java -jar typometer-1.0.1/typometer-1.0.1.jar

# 3. Permissões macOS (mãos do fundador — §5.2), em Ajustes → Privacidade e Segurança,
#    para o RESPONSIBLE PROCESS (o app de terminal que lança o `java -jar`; se uma
#    entrada "java"/JDK aparecer na lista, marcar também):
#    - Gravação de Tela (leitura de pixels) — macOS Sequoia+ RE-PEDE confirmação
#      periodicamente: reconfirmar antes de cada sessão
#    - Acessibilidade (keypress sintético) — ATENÇÃO: a falta dela falha EM SILÊNCIO
#      (o sistema pede a primeira, mas pode nunca pedir esta — adoptium-support#235)
# 4. Teste de sanidade das permissões (30s): rodar 1 medição curta num TextEdit —
#    os "....." aparecem? (Acessibilidade ok) O Typometer acha as métricas? (Gravação ok)
```

### 3.1 Cena de medição (2 condições × ~3 runs)

| Condição | Setup |
|---|---|
| **idle** | `LINA_WS_ROOT=/tmp/f2-lat-ws-idle` + `LINA_LOAD=0`; criar 1 terminal interativo (⌘T) e focá-lo |
| **sob carga** | comando do §2 (12 painéis, 10 ativos rajada/spinner) + **criar 1 terminal interativo extra (⌘T)** e focá-lo — os painéis de carga rodam script, NÃO ecoam tecla; a digitação de medição precisa de um shell vivo no prompt |

Em ambas: o terminal focado no prompt do shell (echo ativo), cursor visível, janela do Lina
em primeiro plano, zoom/tamanho padrão. Desligar "smooth typing"/animação de cursor se houver.

### 3.2 Execução

1. Abrir o Typometer → o Lina em primeiro plano com o terminal focado no prompt; o
   Typometer detecta as métricas digitando "....." na janela focada (1 pixel por símbolo).
2. Configurar o setup de referência de Pavel Fatin: **200 caracteres · delay 150ms · modo
   síncrono** (espera cada char aparecer — mais preciso).
3. Rodar **3× por condição** (descartar o 1º run de cada condição — warm-up de JIT/atlas).
4. **Exportar o CSV cru** (1 latência por keypress — a GUI só dá min/máx/média/SD) e
   calcular p50/p95/p99 **sobre as amostras agregadas dos runs válidos** (não médias de
   médias). Com 2×200 amostras válidas, o p99 apoia-se em ~4 amostras — registrar o n
   junto do percentil; se o gate ficar na margem, subir para 500 chars/run (Dan Luu usou
   ~10k keypresses para p99.9).
5. Repetir a condição "sob carga" com pan/zoom parado vs. contínuo se der tempo (a cena de
   estresse do gate da camada d é com pan/zoom 60s — anotar qual variante foi medida).

### 3.3 Validação física (âncora da medição por software)

Gravar 1 run de cada condição com **câmera 240fps do celular** (slo-mo do iPhone) enquadrando
teclado+tela: contar frames entre o dedo tocar a tecla e o glifo aparecer (±1 frame ≈ 4ms a
240fps). 10-15 amostras manuais bastam como âncora: se a mediana da câmera divergir da do
Typometer por >10ms, a medição por software está enviesada (overhead de captura — §4) e a
câmera vira a fonte da verdade.

## 4. Typometer — fatos verificados (pesquisa web com verificação adversarial, 2026-06-12)

| Fato | Detalhe | Fonte (fetchada) |
|---|---|---|
| Identidade | Repo oficial `pavelfatin/typometer`, **abandonado** (último push 2020-09); release v1.0.1 (2017-09-22), asset `typometer-1.0.1-bin.zip` com `typometer-1.0.1.jar`; Java 8+ (app Swing/AWT) | github.com/pavelfatin/typometer/releases/tag/v1.0.1 |
| Fork p/ macOS moderno | **`frarees/typometer`** — v1.1.0 (2020) adiciona captura nativa **Cocoa** + tolerância de cor (fix do issue #9/Retina); commit 2023-04 corrige `InaccessibleObjectException` (Java 16+) e targeta **Apple Silicon** — só no master, exige `mvn package` | github.com/frarees/typometer |
| Princípio | Keypress sintético em nível de SO + leitura de pixel: digita "....." p/ calibrar (posição, passo, cor de fundo, caret), depois N chars medindo tecla→pixel; **1 pixel por símbolo** | github.com/pavelfatin/typometer#principle |
| Alvo agnóstico | NÃO exige app AWT: digita na janela FOCADA e lê a tela — Dan Luu mediu terminais nativos (iTerm2/Terminal.app/st); janela Metal/wgpu não é impeditivo em si | github.com/pavelfatin/typometer#troubleshooting · danluu.com/term-latency/ |
| Risco nº 1 (Retina) | Issue #9 (aberta pelo autor do iTerm2): dithering + antialiasing do macOS quebram comparação EXATA de cor → fork frarees (tolerância ~1% RGB); alvo precisa de fundo sólido, monospace, alto contraste, linha vazia longa, **caret barra/underline (não bloco)** | github.com/pavelfatin/typometer/issues/9 e /issues/5 |
| TCC | Gravação de Tela (`Robot.createScreenCapture`) + Acessibilidade (`Robot.keyPress`), concedidas ao **responsible process** (o terminal que lança o java; marcar entrada "Java" se existir). **Acessibilidade falha em silêncio**; Sequoia+ re-pede Gravação periodicamente | github.com/adoptium/adoptium-support/issues/235 |
| Saída | GUI: min/máx/média/SD + distribuição (sem percentis); **CSV cru com 1 latência por keypress** → p50/p95/p99 calculados fora | pavelfatin.com/typometer/ |
| Setup de referência | Pavel: 200 chars · 150ms · síncrono; fechar apps com hook global de teclado; alvo maximizado. Dan Luu: ~10k keypresses, p50/p90/p99.9, idle vs carga | pavelfatin.com/typing-with-pleasure/ · danluu.com/term-latency/ |
| Estado em macOS 14/15/26 | **Não-documentado** — nenhuma issue do repo menciona Sonoma/Sequoia/Tahoe (a mais recente é de 2022-08, pré-Sonoma); validar com 1 run curto antes da bateria (sanidade §3.0-4) | github.com/pavelfatin/typometer/issues |
| Compositor | O WindowServer SEMPRE compõe com vsync → ~1 frame de compositor embutido em TODA medição; constante na máquina → comparações relativas seguem válidas | dossiê da pesquisa (Pavel/Dan Luu) |
| Alternativas vivas | **Is It Snappy?** (iOS, câmera 240fps, repo com commit 2025-01 — o fallback §6); OSLTT (hardware open-source à venda, set/2024); NVIDIA LDAT (não vendido — só imprensa) | github.com/chadaustin/is-it-snappy · github.com/OSRTT/OSLTT |

> Implicação direta para o Lina: o cursor do terminal é **bloco** por padrão na maioria dos
> emuladores — se a calibração do Typometer falhar, mudar o caret para barra/underline na
> cena de medição (ou medir num campo de texto do chrome) e ANOTAR a variação.

## 5. BLOQUEIO TÉCNICO da medição autônoma (honesto, com evidência)

A medição NÃO pôde ser executada de ponta a ponta por agente nesta sessão. Cadeia de
bloqueio, na ordem em que ela morde:

1. **Zero JDK na máquina.** `java -version` → "Unable to locate a Java Runtime";
   `/usr/libexec/java_home` idem; `ls /opt/homebrew/opt | grep -i jdk` vazio (brew 6.0.1
   presente). O Typometer é um app Java — sem JDK ele nem abre. Instalável em ~10 min
   (§3.0), mas inútil sem o passo 2.
2. **Permissões TCC (Gravação de Tela + Acessibilidade) para o processo `java`** exigem
   clique humano em Ajustes do Sistema — não há caminho autônomo legítimo. Sem elas o
   Typometer não vê a tela nem injeta teclas.
3. **A medição exige o Lina FOCADO em primeiro plano recebendo teclas sintéticas** durante
   minutos — incompatível com sessão autônoma nesta máquina, que está rodando o Espaço vivo
   do time (5 terminais trabalhando agora): roubo de foco + segunda instância interativa do
   app durante a rodada não é aceitável sem o fundador presente.

O fallback sancionado pela régua D0 e pelo despacho é exatamente este: **protocolo completo
pronto (§3) + captura física na sessão de tela do fundador** — mesmo padrão da célula
drawn≥12 do `prof-baseline.md`. O que era automatizável sem mãos humanas FOI feito e está
no §2.

## 6. Fallback total: protocolo 100% câmera (se o Typometer não cooperar com a janela gpui)

Se na sessão o Typometer falhar contra a janela do Lina (render GPU/Metal pode confundir a
detecção de mudança de pixel — risco listado em §4), a medição inteira é a câmera:

1. iPhone em 240fps, tripé, enquadrando tecla + área do cursor do terminal focado.
2. **30+ keypresses por condição** (idle / sob carga), tecla solta e seca (ex.: `j`),
   ritmo ~1/s.
3. Contar do frame em que a tecla **começa a descer** (não do fundo do curso — a descida
   leva 4-8 frames; método Dan Luu) até o frame da primeira mudança do glifo; registrar
   cada amostra em ms (frames × 4,17ms). O "Is It Snappy?" (iOS, vivo — commit 2025-01)
   faz a marcação e o cálculo frame-a-frame sozinho.
4. p50/p95 sobre as amostras (com n=30, p99 não é honesto — reportar p50/p95 + máximo
   observado e a limitação).
5. Mesma âncora de validação: ±1 frame ≈ 4ms de resolução.

## 7. Tabela de resultados *(preencher na sessão de tela)*

| Condição | Ferramenta | n amostras | p50 | p95 | p99 | máx | Gate (p50≤25 / p99≤50 carga) |
|---|---|---|---|---|---|---|---|
| idle | Typometer | (preencher) | | | | | |
| idle | câmera 240fps (âncora) | | | | | | |
| sob carga (12/10 ativos) | Typometer | | | | | | |
| sob carga | câmera 240fps (âncora) | | | | | | |

- Build medido: `dist/Lina.app` de `(preencher — data/commit)` · macOS `(versão)` · monitor
  `(modelo/Hz — a cadência de refresh limita o piso: 60Hz = piso ~8ms só de espera de scan)`.
- Veredito do gate: `(preencher)` — se FALHA (esperado), anexar a leitura: quanto da latência
  é a cadência de ~40ms do pacing (F1-5-1b) vs. custo real do caminho de input.

## 8. Limitações conhecidas (honestidade)

- Typometer mede a partir do **evento sintético**, não do switch físico da tecla — exclui
  USB/teclado (~1-10ms reais a mais); a câmera inclui tudo. Os dois números não são
  diretamente comparáveis: documentar qual é qual (o gate da régua foi escrito pensando no
  caminho app — Typometer é a referência; câmera é âncora).
- Captura de tela do Typometer pode adicionar overhead na própria máquina medida — por isso
  a âncora física é obrigatória na 1ª sessão.
- O monitor e o compositor do macOS entram na medição (ProMotion/refresh adaptativo pode
  variar a cadência) — fixar o mesmo monitor/Hz em TODAS as rodadas comparáveis.
- Painéis de carga rodam `loadgen.sh` (burst/spinner ANSI) — carga de RENDER realista, mas
  não simula CPU de CLIs de IA reais parseando tokens; o "sob carga" é o cenário-alvo do
  produto (8-12 ativos), não o pior caso de CPU.
- `LINA_AUTOQUIT_MS` não deve ser usado nos runs de medição (o app precisa ficar vivo o run
  inteiro); ele serviu só ao dry-run autônomo do §2.
