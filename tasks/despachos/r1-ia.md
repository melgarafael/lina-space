# DESPACHO r1-f1-5-1 — Especialista em IA
**id:** `f1-5-1` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

## Story: F1-5-1 · Profiling de render — ABRE A ONDA F1-5; BLOQUEANTE de toda otimização (decisão do fundador 2026-06-06)
**Fonte integral:** `tasks/epico-f1/ondas-5-6.md` linhas 22-30 (LEIA INTEIRA). Contexto: teto real medido na tela = ~54fps@N=4 → 12fps@N=28, ~5ms/painel; a sonda atual (`app/lina-gpui/src/main.rs:372+`, log `[FPS]`) mede frametime AGREGADO — esta story DECOMPÕE.

## Fronteira de arquivos (sua, dono único nesta rodada)
- `app/lina-gpui/src/main.rs` (você é o DONO ÚNICO de main.rs do app nesta rodada)
- `app/lina-gpui/src/prof.rs` (NOVO — a sonda decomposta)
- Script/gerador de carga: `app/lina-gpui/tools/` ou bin auxiliar (NOVO; decida e registre)
- `tasks/epico-f1/prof-baseline.md` (NOVO — esqueleto; ver "o que entregar")
- **NÃO toque:** `bridge.rs` (dono: Bug Finder), `canvas.rs`, `crates/` (core).

## O que entregar NESTA fatia (o que dá headless) vs o que fica para a tela
gpui NÃO roda headless — a MEDIÇÃO final na tela é do Maestro (computer-use) + fundador. Sua fatia:
1. **Sonda `[PROF]`** no stderr (mesmo padrão da `[FPS]`: janela ~120 frames, percentis, sem spam): tempo de CPU por fase (poll/update do model, montagem de elementos POR PAINEL, layout) + melhor aproximação de GPU/present disponível no gpui; top-K painéis mais caros; contagem live/drawn e elementos/quads por frame.
2. **Overhead da própria sonda mensurável:** modo `LINA_PROF=0/1` (ou flag equivalente) para medir frametime com/sem `[PROF]` — o relatório final precisa do veredito "a medição é válida ou a sonda perturba".
3. **Cenário de carga REPRODUTÍVEL:** preset/script que sobe N painéis com gerador de output (rajada, spinner ANSI, silêncio) para a matriz N∈{4,16,28} + o cenário-alvo (8-12 ativos, resto ocioso). Sem depender de CLI de IA real (shell + script gerador basta). Documente COMO RODAR no prof-baseline.md.
4. **`tasks/epico-f1/prof-baseline.md` esqueleto:** seções prontas (matriz N, estágio dominante, veredito do overhead, TABELA DE ATIVAÇÃO das condicionais F1-5-3/F1-5-4a/4b/4c com a evidência que ATIVA ou DESCARTA cada uma) — o Maestro preenche com a medição na tela.
5. Smoke headless do que for lógica pura (agregação de percentis, top-K, formato da linha `[PROF]` — testável sem janela).

## Critérios (da peça)
(a) com app vivo N≈28 e output ativo, stderr emite `[PROF]` com ms por fase, p50/p95 por painel, top-K; (b) prof-baseline.md com a matriz (estrutura pronta; dados = sessão de tela); (c) overhead da sonda medível e registrado; (d) tabela de ativação presente — nenhuma otimização da onda inicia sem a linha dela.

## Âncoras
Instrumentação SÓ no shell (`lina-gpui`) — core atrás de `UiHost` intocado. Manter o padrão de honestidade da sonda atual ("FPS enquanto desenha"; idle-gap descartado — `main.rs:389`).

## Entrega
`tasks/epico-f1/.entrega-f1-5-1.md`. Marcador: `.iniciado-f1-5-1`. Validação: `cd app/lina-gpui` → test/clippy/fmt por-pacote, exits diretos; `cargo build` do app verde.
