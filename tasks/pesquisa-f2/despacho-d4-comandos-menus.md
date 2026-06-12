# Despacho D4 — Comandos, menus, paleta e as áreas de visibilidade (skills/agents/MCPs)

## CONTEXTO
Fase 2 do Lina Space = UI/UX (fundador, 2026-06-12). Dois pedidos se encontram aqui: (1) "melhorar comandos, menus, navegabilidade" e (2) as **áreas de visibilidade** do debriefing — ver todas as skills do PC, ver skills instaladas-mas-não-acionáveis via "/", ver agents/hooks/commands/MCPs independente de projeto. O usuário é LEIGO (R4: zero jargão — "skill" na superfície vira o quê?). Já existe ⌘O/rail/atalhos no app (roteiro r5) e detecção de CLI em camadas (ADR 0008). Plano: `tasks/pesquisa-f2/_plano-pesquisa-f2.md`.

## FUNÇÃO
Pesquisador de UX de comando e discoverability. Você traz como produtos excelentes expõem "o que eu posso fazer aqui" a quem não sabe o nome das coisas.

## DIRECIONAMENTO
1. **Interno primeiro:** roteiro de telas r5 (`tasks/`, grep "roteiro"/"rail"), ADR 0008 (detecção de CLI — o mesmo padrão em camadas serve para descobrir skills no disco?), `13.13` (UX de notificação/permissão), R9 (copy é fonte única de strings). Inventarie ONDE skills/agents/hooks/MCPs vivem no disco hoje (`~/.claude/skills`, plugins, `~/.claude/agents`, `.mcp.json`, equivalentes de Codex/Gemini — cheque a máquina REAL) — isso é metade da resposta da "área de skills".
2. **Externo (2025-2026):** command palette state-of-art (Raycast, Linear ⌘K, Zed, VS Code) — ranking, alias, "frecency", comandos com argumentos; menus para leigo (progressive disclosure, NN/g); padrões de "extension manager" que mostram instalado/ativo/quebrado (VS Code extensions, Raycast store, navegadores) — é o espelho da nossa área de skills com o estado "não carrega via /"; onboarding de atalhos (como apps ensinam teclado sem manual).
3. Responda em específico: (a) paleta única global vs menus por contexto — o que serve ao leigo com 9 terminais; (b) como sinalizar skill instalada-mas-inativa SEM jargão (estados: disponível/ativa/quebrada/não-carrega — vocabulário leigo); (c) a "área" é painel do app (gpui) lendo o disco — que padrão de atualização (scan ao abrir vs watcher) os gerenciadores reais usam e qual o custo; (d) navegabilidade: atalhos globais que faltam (ir-para-terminal, busca, foco) comparado ao roteiro r5.
4. Quality gates do plano (datar, refutar, fetch real, piso 5/teto 15). Armadilha: copiar paleta de dev-tool para leigo sem tradução de vocabulário — o Raycast serve de mecânica, não de linguagem.

## OBJETIVO
O épico F2 ganha stories de paleta/menus/áreas com mecânica comprovada e vocabulário leigo — e a "área de skills" nasce sabendo onde ler o disco.

## RESULTADO ESPERADO
`tasks/pesquisa-f2/entrega-d4-comandos-menus.md`: 5-8 achados (CLAIM | FONTE+URL | DATA | CONFIANÇA | REFUTAÇÃO | SUSPECT | LOAD-BEARING) + inventário real dos caminhos de skills/agents/MCPs nesta máquina + respostas posicionadas (a-d) + CONFLITOS · LACUNAS · RECÊNCIA. Reporte via `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status`. Última linha `PRONTO:`/`BLOCKED:`. Não commite; não edite o vault.

## Tentativas anteriores
Nenhuma — 1º despacho desta pesquisa.
