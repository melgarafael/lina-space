# DESPACHO r4-redteam-saida — Red-teamer (terminal spawnado "@Red Team")
**id:** `redteam-saida` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Item 1 do checklist de saída do épico F1: a passada adversarial FINAL sobre o épico inteiro, no HEAD de saída. Rodada r4 (saída F1).

## CONTEXTO
- `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"` (HEAD `f1d4810`).
- O que o item exige (LEIA): `tasks/epico-f1/relatorio-saida-f1.md` §4, primeiro checkbox — *"falta a passada adversarial final sobre o épico inteiro no HEAD de saída, re-derivada no código"*. Critério do fundador: **0 ALTA para sair; MEDIA→backlog com dono** (sem red-team infinito).
- Red-teams por onda JÁ FEITOS (não repita o que eles cobriram; foque no DELTA e nas COSTURAS entre ondas): `tasks/epico-f1/redteam-gate-f1-1.md` (HEAD `2de17cd`) · `tasks/epico-f1/redteam-spawn-f1-3-6.md`. O delta desde então inclui TODA a F1-4/5/6: multi-workspace (boot por ponteiro global `ecda988`, restore `236bf2f`/`0649b22`/`a3cb75e`, runtime-por-Espaço `e08ad5a`, troca viva + LINA_HOME por spawn `3eb915a`, criar Espaço `cb8ec2d`/`8075565`/`dad735d`, fixes `ef6acd6`/`f1d4810`) · scrollback/história (`9a37399`, `1bdaeba`→F1-5-6/9, `28f7e79` F1-5-8 API cross com auditoria) · suspensão de ociosos (`626228c`) · anti-starvation webhooks (`dd5389b`) · verbos de estado global (`ce554e9`/`4c56d3d`) · faxina LINA_DEV (`f3bbc35`).
- Invariantes do produto: `CLAUDE.md` do repo (os 7 + âncoras). ADRs: `docs/adr/` (em especial 0004 custódia, 0006 WorkspaceTrust, 0007 origem×cascata, 0019 definições, 0022 admissão canônica, 0026 identidade por env do spawn, 0027 anti-starvation).
- Doutrina de segurança (regra 7 das regras comuns): nenhum campo escrito por agente decide identidade/ordem/autorização.

## FUNÇÃO
Você é o red-teamer ISOLADO do gate de saída — você NÃO construiu nada dessas ondas. Seu trabalho é tentar REFUTAR a segurança/integridade do épico, não confirmá-la.

## DIRECIONAMENTO
- **READ-ONLY no código de produção.** Você só escreve: o marcador `.iniciado-redteam-saida`, o relatório final e (se quiser provar um furo) testes em arquivo NOVO de teste claramente nomeado `redteam_*` — nunca edita produção.
- **Linguagem de INVARIANTES, não narrativa de ataque** (lição AUP do projeto): formule cada achado como "o invariante X (fonte) é violável em arquivo:linha porque Y — evidência re-derivada Z".
- **Re-derive NO CÓDIGO.** Não confie em entregas/relatos dos workers; abra o fonte e confirme a linha que barra (ou que NÃO barra).
- Eixos mínimos do sweep (amplie se farejar algo): (1) identidade/autorização A2A no multi-workspace — a troca viva de Espaço preserva o pertencimento? `LINA_HOME` por spawn vaza entre Espaços? (2) replay/restore — log adversarial (eventos forjados/fora de ordem/antigos) quebra o boot ou ressuscita autoridade? (3) spawn — cascata/binding/dedupe seguem inforjáveis no HEAD? (4) guard/custódia — bypass novo via verbos/estado global adicionados? (5) scrollback/history API — a leitura cross-terminal respeita pertencimento e audita? (6) webhooks — anti-starvation introduziu DoS ou bypass do HMAC/rate-limit? (7) fila de atenção/aprovação — spoofing de origem?
- Classifique: ALTA (bypass de autorização/integridade/injeção) · MEDIA (disponibilidade/durabilidade/doc-vs-código) · BAIXA. **Não infle nit→ALTA** (finders inflam; você re-deriva antes de classificar).
- Orçamento: parada quando os eixos acima estiverem cobertos com evidência por eixo — não é red-team infinito.

## OBJETIVO
Sem este relatório o épico F1 NÃO declara saída. É a última linha de defesa antes do fundador decidir o gate — um furo ALTA achado agora custa horas; achado depois do release, custa a confiança do produto.

## RESULTADO ESPERADO
`tasks/epico-f1/redteam-saida-f1.md`: tabela de achados (invariante violado · arquivo:linha · severidade · evidência re-derivada · dono sugerido p/ MEDIA) + confirmações positivas COM a linha que barra (igual aos red-teams anteriores) + veredito final (`0 ALTA` ou a lista). Entrega-resumo em `tasks/epico-f1/.entrega-redteam-saida.md`. Marcador `.iniciado-redteam-saida` no primeiro ato. Última linha `PRONTO: <veredito>` ou `BLOCKED: <motivo>`.
