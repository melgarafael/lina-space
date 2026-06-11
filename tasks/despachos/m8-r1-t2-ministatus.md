# Despacho M8-T2 — Mini-status por Espaço (headless, dashboard.rs)

> Rodada M8-r1 · 2026-06-11 · Maestro externo → Terminal #40 (DATA_ENGINEER)
> Marque início: `echo INICIADO > "/Users/rafaelmelgaco/einstein workspace/lina-space/.iniciado-m8-t2"`

## CONTEXTO
A sidebar M8 (switcher de Espaços) mostra por linha: «{N} Agentes» + ● estado dominante + custo de
hoje + ⚠ pasta-não-encontrada. A spec congelada é `tasks/epico-f1/spec-m8-m9-fiacao.md` §1 (a
tabela campo-a-campo) e §6-B5 (risco perf: projetar N stores a cada abertura → cache com staleness
declarada + busy_timeout). Mapa do código pronto: `/tmp/mapas-m8/dashboard-status.md` (UiState,
CostLine, workspace_cost_today — linhas exatas). HEAD: `a3cb75e`.

## FUNÇÃO
Engenheiro de dados/projeções: funções PURAS e testáveis headless sobre o event log.

## DIRECIONAMENTO
1. `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"`.
2. Em `app/lina-gpui/src/dashboard.rs`, crie (público, gpui-free):
   - `pub struct WorkspaceMiniStatus { pub agents_alive: usize, pub dominant: Option<UiState>,
     pub cost_short: Option<String>, pub cost_tooltip: String, pub unreachable: bool }`
   - `pub fn workspace_mini_status(events_dir: &Path) -> WorkspaceMiniStatus` — abre o store
     READ-ONLY (busy_timeout+retry, F1-4-1 crit 5), projeta, conta `kind=="Terminal"` vivos via
     `UiState::from_projection`, dominância PIOR-PRIMEIRO (Sem resposta > Esperando você >
     Trabalhando > Ocioso > Dormindo > Encerrado — regra da spec §1), custo via
     `workspace_cost_today` (forma curta SEM sufixo; «estimado» vai no tooltip). Honestidade:
     sem dado → `cost_short=None` (a UI mostra «—»), NUNCA «0,00». Store que não abre/projeta →
     `unreachable=true` (a linha vira ⚠, NUNCA some — anti-padrão 6 da spec).
   - `pub struct MiniStatusCache` — cache por abertura do M8: `refresh(entries: &[PathBuf])`
     projeta 1× cada e guarda; `get(&Path)` devolve o último + idade. Staleness declarada
     (campo `computed_at_ms` injetado pelo caller — função pura, sem relógio interno).
3. TDD: fixtures com EventStore real em temp dir (padrão dos testes existentes do arquivo) —
   casos: espaço com 2 vivos estados mistos (dominância correta), espaço sem agentes, espaço com
   só Encerrados (N agentes + dominant Encerrado — célula "todos-Encerrado" da spec §1), store
   corrompido/ausente (unreachable=true), custo sem sessão (None, nunca zero).

## FRONTEIRA DE ARQUIVOS (exclusiva sua nesta rodada)
- `app/lina-gpui/src/dashboard.rs`. Mais NADA. (main.rs/runtime.rs são do Terminal J AGORA.)
- **NÃO COMMITE.** O Maestro valida de fora e commita.

## OBJETIVO
Mini-status honesto por Espaço consumível pela sidebar (rodada 2) sem nenhum gpui: suíte do app
verde, clippy `-D warnings` e fmt EXIT=0 (exit direto, redirecionado p/ arquivo e lido).

## RESULTADO ESPERADO
`.entrega-m8-t2.md` na raiz: assinaturas finais, decisões (ex.: forma curta do custo), evidência
dos exits, e a lista dos testes novos. Termine com `PRONTO:` ou `BLOCKED: <motivo>`.
