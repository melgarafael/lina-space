# InsForge — avaliação como referência de quality gates (registro do Maestro)

> **Origem:** o fundador indicou `github.com/InsForge/InsForge` como possível referência para os gates de qualidade do trabalho dos agentes. Avaliado em 2026-06-06 (auditoria fan-out da sessão do Épico F1, agente web dedicado). Este arquivo é o registro citado pelo doc `34 - Epico Fase 1` (§II "doutrina InsForge" e apêndice X).

## Veredito: NÃO adotar o produto — categoria errada

O InsForge (Apache-2.0, ~92% TypeScript) é um **BaaS all-in-one agent-native** voltado a *coding agents*: Postgres (pgvector/RAG), auth (JWT+OAuth), storage S3-compatível, edge functions, compute, hosting e um AI model gateway, expostos via MCP server ("semantic layer") + CLI + Skills. Ele é o **substrato onde agentes constroem** — não um framework que **julga/valida/aprova** o trabalho de agentes cooperando (o problema do Lina). Docs e site confirmam: não há subsistema de quality gate, guardrail, supervisão ou validação de saída. **Não existe código de prateleira a portar; o que vale é doutrina.**

## Os 4 princípios que ficam para o Lina

1. **Verificador independente e ortogonal.** O gate é um agente/processo separado, com contexto próprio e **permissão explícita de REPROVAR** — nunca o autor do trabalho. (Consenso da literatura: dual quality gates, Stop hooks. Reforça a lição do projeto: verificador adversarial otimista demais → re-verificar vereditos "não-problema" no código.)
2. **Gate lê sinal determinístico, não auto-declaração.** Aprovação/reprovação pelo **log/event store** (sucesso-falha determinístico, efeito atribuível por NodeId, reversível), jamais pelo "ok" do agente. (Dor já conhecida do Lina: `lina ask` "ok" cego; formalizar como invariante do gate.)
3. **Pass^N por repetibilidade** (do benchmark MCPMark/Pass⁴): só promover trabalho de agente se a checagem passa de forma determinística em execuções independentes; **flaky = reprovado**.
4. **Context-first como gate preventivo complementar.** Entregar a cada terminal o estado real (roster/projeção/canvas) antes de agir reduz defeito na fonte — mas **não substitui** o gate de aceitação.

Corolário adotado pelo épico (§II do doc 34): **capacidades sensíveis são VERBOS estruturados do `lina`, nunca shell direto** — o verbo é o ponto de gate (contratos de tool determinísticos, mutações reversíveis).

## Próximo passo sugerido (não-bloqueante)

Para benchmark concreto: estudar o **MCPMark** (métrica Pass⁴) — o artefato mais reutilizável do ecossistema InsForge. Para o design do gate em si: padrões de *dual quality gates* e *Stop hooks* para coding agents.

## Fontes

- https://github.com/InsForge/InsForge · https://insforge.dev/ · https://docs.insforge.dev
- https://insforge.dev/blog/mcpmark-benchmark-results · https://insforge.dev/blog/insforge-launch-v2
- https://www.sagarmandal.com/2026/03/15/agentic-engineering-part-7-dual-quality-gates-why-validation-and-testing-must-be-separate-processes/
- https://fbakkensen.github.io/ai/devtools/development/2026/03/27/quality-gates-for-coding-agents-how-stop-hooks-make-validation-mandatory.html
- https://medium.com/codetodeploy/how-context-first-mcp-design-reduces-agent-failures-on-backend-tasks-3b3b5bae796a
