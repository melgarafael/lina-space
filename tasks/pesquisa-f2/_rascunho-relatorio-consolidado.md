# [RASCUNHO do Maestro — vira "37 - Pesquisa F2" no vault após a onda V] Pesquisa de fase: F2 — Interface e Experiência do Usuário

*Data: 2026-06-12 · Tipo: híbrido (produto+técnico, fase não-LLM) · Agentes: 5 dimensões + 2 verificadores cruzados · Buscas: ~210 (14+92+~15+30+56) · Fontes: ~120 fetchadas, núcleo 2024-2026*

## Veredito executivo (draft)

A F2 parte de fundação melhor do que o pedido supunha: o design system de COR já existe e é maduro (theme.rs, F1-2-1, gates WCAG em CI), a paleta ⌘K já existe, e o canvas por teclado já existe. O trabalho da fase é (1) completar o vocabulário de tokens (tipografia/espaçamento/radius/motion) e consolidar um catálogo próprio pequeno modelado no Zed — nunca copiado (GPL) nem dependido (gpui-component tracka HEAD); (2) dar ao Lina UMA identidade com coragem — recomendação do time: território "Instrumento de Estúdio" (cor semântica fixa + terminal como viewer vivo + flat honesto), com decisão final do fundador entre 4 territórios nomeados; (3) entregar organizar/redimensionar/mover com a receita comprovada da indústria (GPU contínua + PTY debounced; snap passivo + "Arrumar" sob demanda; câmera nunca anda sozinha; 1 gesto = 1 evento no log); (4) tornar visível o que já existe (paleta com porta rotulada; áreas de Poderes manifest-first com 5 estados leigos e ação de 1 clique). Tudo medido pela régua em 6 camadas da D0 — com "viciante" ético operacionalizado e tempo-no-app PROIBIDO como métrica. Maior risco: prometer 120Hz/resize-livre antes de validar o pacing (F1-5-1b) e o bug aberto de scrollback (#8576). O que mudaria a decisão: fundador escolher outro território; verificação derrubar achados estruturais (onda V em curso).

## Restrições internas herdadas (A5)

R1-R12 — tabela completa em `_plano-pesquisa-f2.md` (gpui pinned; core/shell split; portal slot; não-técnico-first; event log; a11y AccessKit; perf 120Hz; anti-slop; copy congelada; ADR 0019; pesquisa 13.x; **fundação theme.rs/palette.rs JÁ existe**). Confrontos explícitos desta pesquisa contra o interno:
- D1 CONTESTA o default "Dark OLED já aplicado" (fluxo 22/T0): proposta terminais sempre-dark + chrome segue o sistema. **Decisão do fundador no épico.**
- D3 ajusta o minimap (já decidido na visão): manter barato (estático + clique salta), reavaliar com telemetria.
- D4 CONTESTA dois detalhes shipped: paleta atalho-only (porta visível obrigatória) e ações hover-only no rail (pista permanente).
- D2 nomeia dívida que contradiz ADR 0019 §7: grid em Menlo hardcoded → JetBrains Mono via tokens (onda 1).

## Como mediremos sucesso (eval-first — D0, régua em 6 camadas) ✅ auditada (V-D0: zero gates caem)

(a) anti-slop ≥80/zero ALTA por story (reusa rubrica v1) · (b) leigo: 5/5 completam sem ajuda, 2 rodadas, SEQ mediana ≥5,5, SUS tendência ≥68 · (c) percepção: ≥60% acertam ≥2 palavras-alvo, zero kill-word repetida; 5s ≥8/10; line-up ≥8/10 · (d) perf: p95 ≤16,6ms + <1% do tempo acima do orçamento (avaliar 2ª condição "Steady": <0,1% excess time — V-D0); input p99 ≤50ms; 120Hz = meta de plataforma, NÃO gate · (e) a11y WCAG 2.2-AA-via-WCAG2ICT (versão vigente 2025-12-11) por story (contraste/alvo 24px/árvore AccessKit/teclado/motion-por-doutrina) · (f) viciante ético por release: zero deceptive pattern (pré-condição) + retorno voluntário W1 ≥60% (convenção; caveat HEART real: em contexto de trabalho, engagement pode não ser significativo — privilegiar Happiness/Task Success) + Sean Ellis ≥40% como tendência + zero arrependimento; **tempo-no-app proibido como métrica de sucesso**. Cadência de custo: ~1h/story · ~3h/rodada · ~3h/release. Preparação: tradução SUS pt-BR + 1ª medição input latency (Typometer) + estender [PROF] com p99/% acima do orçamento.

**Correções da auditoria V-D0 aplicadas:** SUS 68/80,3 e SEQ 5,3-5,6 = âncora de fonte única (Sauro&Lewis; "corroborações" NN/g/UXtweak/Lyssna são circulares), magnitude ~68-70 replicada por Bangor 2008/09 com limiares de nota conflitantes → seguem como termômetro, nunca gate (uso já era esse). 4%/6% de distintividade reatribuídos a JKR×Ipsos 2023 (26k respondentes; tianpan.co fundiu framework Romaniuk com dado Ipsos). "150ms Carmack" descartado (folclore; 20/50ms confirmados na fonte primária). "4-23 participantes" (reaction cards) descartado — sem rastro no paper original.

## Achados por dimensão (somente os que decidirem stories; detalhe e fontes nas entregas)

### D1 — Identidade (entrega-d1) [V: pendente]
A1 Linear-look = novo genérico, abandonado pelo originador · A2 "premium" é latência (e cobra juros públicos quando regride) · A3 manter cara de terminal, matar a PASSIVIDADE (nó sempre narra) · A4 calor na superfície, nunca na estrutura (novelty tax do Arc 0,4-5,5%; cursores nomeados do Figma) · A5 estética de instrumento é mecanismo (cor=significado fixo; viewer vivo; flat honesto; limite: nunca cortar undo) · A6 dark imposto não se sustenta (terminais dark = ilha de identidade; chrome segue sistema) · A7 sem variable fonts no gpui (estáticos obrigatórios); humanista p/ relance; OFL = risco zero (Plex/Fraunces/Atkinson) · A8 consistência de terceiros por arquitetura (catálogo fechado Raycast) + motion: nunca animar ação iniciada por input frequente.
**Territórios:** T1 Instrumento de Estúdio ⭐ · T2 Oficina de Precisão · T3 Ateliê Caloroso (funde com T1) · T4 Sala de Controle (não-default). **OpenDesign:** importar FORMATO (DESIGN.md/SKILL.md vendorizado) + picker "Direct"; link só open-design.ai, opt-in; nunca embedar/depender.

### D2 — Design system gpui (entrega-d2) [V: pendente]
A1 fundação de cor pronta; dívida = 68 text_size + 31 FontWeight + Menlo hardcoded + 380 px() · A2 modelo Zed: vocabulário em código toolkit-free, valores em JSON refinável · A3 theme/ui do Zed são GPL → modelar, JAMAIS copiar · A4 gpui-component: modelar, nunca depender (tracka HEAD; pin nosso é 2026-05-30) · A5 padrão convergente multi-backend (COSMIC RON; Slint consome tokens via Rust; DTCG = só vocabulário) · A6 ADR 0028 (DRAFT→selar) constrange toast/badge ao Element de live-region · A7 ecossistema: nada para depender.
**Arquitetura:** completar TypographyTokens/SpacingTokens/RadiusTokens/MotionTokens no módulo gpui-free; JSON aditivo como documento canônico; catálogo próprio pequeno (botão/painel/menu/badge/toast/input/modal); migração em 4 ondas com teste-catraca (contagem de px() inline só pode CAIR); extração de crate lina-theme só com 2º cliente.

### D3 — Canvas UX (entrega-d3) [V: pendente]
A1 resize vivo: GPU contínua + PTY debounce ~100ms + commit ao soltar; ghost outline não existe shipped · A2 reflow de scrollback = campo minado; **#8576 ABERTA no alacritty_terminal (nosso crate)** — validar pin ANTES da story · A3 CLIs de IA são o pior caso (flicker); curas = DEC 2026 + debounce (Codex PR #18575) · A4 snap passivo 8px (tldraw) + "Arrumar" como verbo; auto-arranjo contínuo quebra confiança · A5 frames/grupos: NÃO na 1ª onda (captura por geometria = erro de intenção; job de centenas, não de 3-9) · A6 invariantes niri: nada se redimensiona sozinho; preset lado-a-lado · A7 zoom semântico = LOD com thresholds (2 degraus + histerese); zoom-to-fit/selection; premissas do despacho refutadas (tldraw TEM minimap) · A8 atenção é PULL: badge + "o" vira Spacebar-de-aprovações; câmera NUNCA se move sozinha (cicatriz VS Code) · A9 persistência: 1 gesto = 1 evento (estado final); câmera FORA do log (sessão local); z-order por fractional index.

### D4 — Comandos/menus/Poderes (entrega-d4) [V: pendente]
Inventário real: 75 skills/66 SKILL.md (scan ~0ms) · plugins manifest 13KB vs árvore 1,9GB · skills em 4 CLIs ("não funciona NESTE motor" é o caso normal) · MCP por-projeto na prática.
A1 paleta escondida = morta (caso GitHub); nossa ⌘K está no estado pré-morte → porta visível search-first (modelo Notion) · A2 ranking: alias estrito > MRU estável > fuzzy > frecency por último (VS Code rejeita frequência desde 2017; Zed = blueprint no nosso stack) · A3 hierarquia: visível-rotulado > contextual redundante > busca > atalho (81% nunca usaram Ctrl+F; icon-only falha; Ribbon provou comandos visíveis) · A4 progressive disclosure: máx 2 níveis, porta rotulada · A5 estados: consequência + ação de 1 clique (Repair/Install-in-host); 5 estados leigos com âncora do termo técnico · A6 atalhos: hint passivo converte ≤35% em lab; pacote = hint inline + "?" + 1 just-in-time; **regra dura: nada existe SÓ atrás de atalho (candidata a invariante de UI)** · A7 disco: manifest-first + scan-ao-abrir + watcher raso debounced ≥750ms; watch recursivo PROIBIDO (inotify ENOSPC) · A8 ADR 0008 transplanta inteiro (registry determinístico; heurística nunca decide; **mostrar ≠ autorizar**).

## Recomendação (esqueleto de ondas para o épico — refinar com vereditos V)

- **Onda F2-0 (preparação/régua):** selar ADR 0028 · tradução SUS pt-BR · 1ª medição input-latency · estender [PROF] (p99 + % acima do orçamento) · validar pin vs #8576 · suporte DEC 2026 no emulador · **decisão do fundador: território estético (protótipos A/B na tela — "testar opções" do pedido original) + dark/light + forma OpenDesign**.
- **Onda F2-1 (vocabulário):** tokens typography/spacing/radius/motion + modo sistema + JetBrains Mono no grid (fecha dívida ADR 0019 §7). Gate: catraca + WCAG estendido.
- **Onda F2-2 (catálogo + identidade):** componentes núcleo consumindo tokens (toast/badge sobre live-region) + aplicação do território escolhido + porta visível da paleta + toolbar contextual do nó.
- **Onda F2-3 (canvas):** mover/redimensionar/organizar (receita D3 completa: eventos transacionais, snap passivo, Arrumar, preset lado-a-lado, zoom-to-fit/selection, "o"→aprovações).
- **Onda F2-4 (Poderes):** área de skills/agents/MCPs (manifest-first, 5 estados, mostrar≠autorizar) + skills-não-carregáveis + onboarding OpenDesign-formato.
- **Transversal:** régua D0 por story/rodada/release; cada onda passa [PROF].
- Candidato herdado (decidir no épico): Ghost wires + Linha do Tempo (ADR 0010 addendum).

## Custo e conformidade
- Custo por interação = frametime: budget p95 ≤16,6ms (gate F2) rumo a 8,33ms (plataforma); toda story de UI passa pela sonda [PROF]; token nunca vira indireção em hot path (resolver no build da cena).
- LGPD: **N/A justificado** — fase 100% local-first, zero dado pessoal novo, zero rede nova; área de Poderes lê disco local e nada sai da máquina (inv#2). Meta/WhatsApp: N/A (sem superfície de mensageria na F2).
- Licenças: tipografia OFL (custo zero) ou cotação comercial ANTES de decidir; Zed theme/ui GPL = não copiar; Fontshare = zona cinzenta não resolvida (não usar embutido).

## Riscos e incógnitas
| Risco | Sev | Redução |
|---|---|---|
| Pastiche do território (figurino sem mecanismo) | ALTA | cold-review DES-4 cita o statement D1; rubrica anti-slop |
| #8576 perde scrollback no resize | ALTA | teste do pin ANTES da story; cap de reflow |
| 120Hz prometido sem diagnóstico do pacing | MÉDIA | gate em 60Hz-p95; 120 = meta; F1-5-1b primeiro |
| Tentação de depender do gpui-component | MÉDIA | doutrina "modelar, nunca depender" no épico; exceção tática só com spike+vendor |
| Verificação V derrubar achado estrutural | — | aguardando onda V (este rascunho marca [V: pendente]) |
| Confiabilidade A2A da própria orquestração (achados #20-22) | MÉDIA | candidatas F2/F3 registradas no dogfooding |

## Lacunas declaradas (consolidado das entregas)
Reação quantitativa de leigos à estética de terminal (instrumentar via D0) · preços atuais de fontes comerciais (cotar) · FFL/Fontshare embedding · telemetria dark/light de população leiga · "terminal dark sobre canvas claro" sem precedente shipped (protótipo) · harness AccessKit automatizado (L6) · input latency nunca medida (L7) · benchmarks SUS/SEQ são web-anglófonos (âncoras, não gates) · alvos 60%/8-10 são convenções declaradas (calibrar 2 rodadas).

## Descartados na verificação
**Da V-D0 (auditoria da régua — 4 confirmados, 3 confirmados-com-correção, 1 incerto):**
- "Operacional com 4-23 participantes" (reaction cards) — sem rastro no paper Benedek & Miner lido na íntegra.
- "SUS corroborado por NN/g e UXtweak/Lyssna independentes" — circularidade comprovada (todos citam o banco Sauro&Lewis); substituído por "fonte única + magnitude ~70 replicada por Bangor 2008/09; limiares de nota são convenção contestada".
- Atribuição dos 4%/6% a Romaniuk — números reais, mas do estudo JKR×Ipsos 2023; reatribuídos.
- "Caveat HEART: medir semanal para produtividade" — inexistente no paper (lido na íntegra); o caveat real (engagement pode não ser significativo em enterprise) REFORÇA a régua.
- "150ms insuportável (Carmack)" — não está na fonte primária; 20/50ms ficam.
- WCAG2ICT "2024-10-08" — superada; vigente é 2025-12-11. SC 2.2.2 é nível A, não AA (sem efeito prático).
- "~18 testers p/ problema de 10%" não é de Faulkner — é fórmula binomial Sauro/Lewis com parâmetro oculto (85% de chance de observar); manter só com o parâmetro declarado.

**Da V-D1-D4:** [preencher quando o Terminal D entregar]
