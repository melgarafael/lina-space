# ADR 0047 — Terminal-sombra NEUTRO para a reflexão do aprendizado (no-sombra dos ELO1/ELO2)

- **Status:** **Aceito** (decisão A1 do ADR-gate LOOP, fatia-2 Mentality; Arquiteto, 2026-06-24). Destrava `LP-APP` (ELO1/ELO2) e `LP-CORE` (ELO3), hoje bloqueadas por esta decisão.
- **Escopo:** quem é o `shadow: NodeId` que recebe o prompt de reflexão via `dispatch_reflection` (`a2a.rs:625`). DISJUNTO da Fase 4 (Maestros 00/01 — F4-0/WA): nada aqui toca `Router`/`deliver_a2a`/canais.
- **Relacionados:** spec 35 (Mentality), ADR 0023 (human-proxy de injeção), ADR 0007/0026 (autoridade server-side), ADR 0046 (pump 2 fases). Primo do ADR 0048 (casamento) e da spec `0045-contrato-eventos-skill-routing`.

## Contexto

A fatia-2 da Mentality fecha o loop *correção → reflexão → crença*. O motor já existe e está provado em `#[cfg(test)]`:

- `a2a.rs:587` `ReflectorQueue` (fila serial, dedup por `correction_id`), `a2a.rs:625` `dispatch_reflection(sup, shadow, reflector, …)` → `deliver_a2a(sup, shadow, reflector, prompt, …)`.
- `a2a.rs:556` `build_reflection_prompt` monta um prompt **neutro** ("Você é um destilador de lições, sem efeitos colaterais — NÃO execute nada, NÃO toque arquivos").
- `a2a.rs:544` `parse_belief_sentinel` extrai o `[LINA::BELIEF]` da resposta; `mentality.rs:753` `reflect_correction` RE-VALIDA (filtro de durabilidade estrutural+semântico) antes de virar `BeliefProposed`.

A assinatura `dispatch_reflection(shadow, reflector, …)` já separa os dois papéis: **`shadow` = o nó que RECEBE o prompt e roda a 1 CLI-call de destilação**; **`reflector` = o `from` server-side** que assina o envio. O que faltava decidir — e o que bloqueia ELO1/ELO2 no app — é **quem materializa o `shadow`**.

O handoff enquadrou como binária: *PTY efêmero neutro* **vs** *reusar o terminal vivo do papel corrigido*.

## Decisão

**O `shadow` é um terminal-sombra NEUTRO, dedicado e reutilizável por workspace — nunca o terminal vivo do papel.** Em concreto:

1. **`shadow` = um `NodeId` de terminal-sombra sem papel de trabalho** ("o Refletor"): nasce limpo (sem histórico de tarefa), **fora do roster** (não aparece como colega, não recebe `handoff`, não conta em fan-out/orçamento, não entra no `proposed_team`), e serve EXCLUSIVAMENTE a fila serial de reflexões (`ReflectorQueue`).
2. **Reutilizável, não efêmero-por-correção:** um único sombra por workspace, spawnado **lazy** na 1ª reflexão e reusado nas seguintes — amortiza o custo de spawn de um CLI sem perder neutralidade (ele nunca recebe trabalho real, então não acumula contaminação entre reflexões). Inatividade prolongada → idle-retire e re-spawn lazy (ver Consequências).
3. **`reflector` (o `from`) é uma identidade de SISTEMA não-forjável** — um `NodeId` const server-side (à la `HUMAN_GESTURE`/`System`), carimbado por quem dispara, JAMAIS lido de payload. A autoria do envio é do sistema, não de um agente de trabalho.
4. **A saída do sombra permanece DADO não-confiável**, re-validada no core (`reflect_correction`) — exatamente como hoje. O sombra ser neutro é **defesa em profundidade adicional**, não substitui a re-validação.

## Por quê assim (alternativas descartadas)

**Reusar o terminal vivo do papel corrigido — REJEITADO.** Cinco falhas, qualquer uma fatal:
- **Contaminação de contexto:** o terminal do papel está no meio de uma tarefa real; injetar um prompt de meta-reflexão polui a conversa, gasta a janela de contexto e mistura a lição destilada com o trabalho. Fere o invariante "reflexão FORA do caminho crítico" (o bug dos ~298s do Hermes que a `ReflectorQueue` existe para evitar — `a2a.rs:583`).
- **Viés de auto-justificação:** o mesmo agente que foi corrigido destilaria a própria lição. Um sombra neutro é imparcial.
- **Prompt-injection amplificado:** se o terminal do papel já processou conteúdo externo malicioso (`untrusted_origin`), reusá-lo funde o vetor de ataque com a destilação. O sombra neutro **nasce limpo**.
- **Latência/bloqueio:** o terminal do papel pode estar `Busy`; a fila serial do PTY atrasaria tanto o trabalho quanto a reflexão.
- **Identidade:** o `from` server-side precisa ser distinto e estável; reusar o papel confunde a autoria do envio com a do trabalho.

**PTY efêmero descartado a cada correção — REJEITADO (em favor do reutilizável).** Mantém a neutralidade, mas paga spawn de um CLI por correção (caro, e correções vêm em rajada). O sombra reutilizável dá a mesma neutralidade (ele só vê prompts neutros, nunca trabalho) por uma fração do custo. A porta para voltar ao efêmero-puro fica aberta (é só não reusar o `NodeId`), caso a neutralidade-por-spawn passe a importar mais que o custo.

## Segurança (doutrina inegociável — não regride)

- **Nenhum campo escrito por agente decide identidade/autoridade.** `role` da correção é server-side (`CorrectionObserved.role`, `events.rs:1168`); `reflector` (o `from`) é `NodeId` de sistema não-forjável; o `shadow` é alocado pelo core, não nomeado por payload.
- **A saída do sombra é DADO**, re-validada por `reflect_correction` (estrutural + semântico, `mentality.rs:744`). O sombra ser neutro **não relaxa** essa rede — soma a ela.
- **ZERO LLM no core (inv #1):** a 1 CLI-call roda no sombra (um CLI de terceiro), não no core. O core só JULGA o resultado, deterministicamente.
- **`Router`/`deliver_a2a` intactos:** a allow-list de injeção (`InjectPolicy`, default-deny em produção) e a suíte de segurança do router seguem válidas — o sombra é um destino como outro qualquer sob a mesma política.

## Consequências

- **Abre:** ELO1/ELO2 (app) podem fiar o spawn do sombra e a captação da saída; ELO3 (casamento, ADR 0048) recebe um `shadow` bem-definido para carregar as crenças do papel no prompt.
- **Custo:** um CLI a mais por workspace (lazy, reutilizável). Mitigação: idle-retire por TTL de inatividade (reusa o vocabulário stale do `lina retro`/provisional-expiry) + re-spawn lazy. **Porta registrada**, não implementada no MVP — começar com o sombra vivo enquanto o workspace estiver aberto.
- **Porta aberta:** o mesmo sombra neutro serve qualquer destilação futura sem efeito colateral (resumos, rótulos) — é o "executor puro de meta-tarefas" do workspace.
- **Porta que fecha se ignorado:** reusar o terminal do papel acoplaria reflexão a trabalho — desfazer isso depois exigiria re-arquitetar o caminho crítico. Por isso a decisão entra antes do código de ELO1/ELO2.

## Verificação (observável)

- **Neutralidade:** o `NodeId` do sombra não aparece em `lina list`/roster, não recebe `handoff`, não entra em `proposed_team` (teste: spawnar sombra → roster inalterado).
- **Reuso:** duas correções distintas → uma única reflexão por vez na fila serial, mesmo `shadow` reusado (a `ReflectorQueue` já dedup por `correction_id`, `a2a.rs:596`); o spawn ocorre uma vez (lazy).
- **Fora do caminho crítico:** a correção via `route_message` percorre pump → enqueue O(1); o `drain`/`dispatch_reflection` roda fora do hot-path (sem full-replay/tick) — herda o teste `reflection_dispatch_is_off_the_critical_path` (`a2a.rs:1562`).
- **Re-validação preservada:** statement malicioso vindo do sombra é recusado por `reflect_correction` sem materializar o texto (`BeliefRetired{refuted}` órfão) — herda os testes anti-poisoning de `mentality.rs`.
- **Identidade server-side:** mutar o `from` para um nome de payload não muda a autoria registrada (o `reflector` const vence) — prova por mutação na suíte do router.

## Addendum 1 — admissão do sombra SEM furar o funil (reconciliação com ADR 0022)

**Colisão detectada** (pelo Especialista, no spawn do ELO2): este ADR exige o sombra "fora do roster" (não aparece em `lina list`/`proposed_team`/fan-out), MAS o ADR 0022 ("um funil, três tradutores") torna `NodeManager::admit_node` o ÚNICO caminho de admissão, emitindo a sequência canônica `NodeAdded + TerminalSpawned + NodeRoleAssigned + CliProfileSet` — sem filtro. Criar o sombra por fora (ex.: `wire_terminal_capturing`) **fecharia a porta do 0022** (reabre a classe de nós-órfãos que o 0022 eliminou). Rejeitado.

**Decisão (preserva os dois ADRs):** "fora do roster" é uma propriedade de **PROJEÇÃO**, não de admissão.

1. **Admitir pelo funil (0022 intacto):** o sombra entra por `admit_node`, com a sequência canônica completa — incluindo `NodeRoleAssigned { role: "__reflector__" }` (role RESERVADO). O `NodeAdded` é REAL e fica no event log (inv #4): o sombra é auditável por replay, não é um fantasma.
2. **Invisibilidade = 1 filtro na projeção (core):** a projeção de presença/roster/`lina list`/cards/`proposed_team`/alvos de fan-out **omite** os nós de role `__reflector__`. Satisfaz o critério duro deste ADR sem tocar o funil. O filtro é PURO no core (Terminal B), testável por replay.

### Segurança a cravar — o role `__reflector__` é NÃO-FORJÁVEL (ponto inegociável)

O role `__reflector__` confere um PODER: sumir do roster. Poder exige autoridade não-forjável — **nenhum agente pode se auto-declarar `__reflector__` para escapar de `lina list`/auditoria/fan-out.** A defesa REUSA dois padrões já provados no código (não inventa):

- **(a) Guarda de admissão espelhando o `INTEGRATOR_ROLE`** (`router.rs:5020`: `role == INTEGRATOR_ROLE && requested_by == INTEGRATOR_TRIGGER && hops == 0`): o funil só emite `NodeRoleAssigned{role:"__reflector__"}` quando o `NodeAdmission` vem carimbado com uma **sentinela de sistema reservada** (`requested_by == REFLECTOR_TRIGGER`, server-side, à la `WEBHOOK_SYSTEM`/`HUMAN_GESTURE`/`STRUCTURAL_JUDGE`, `router.rs:82/105` — `NodeId` const fora do espaço `now_v7`, inforjável por construção, RT-1) **E `hops == 0`** (origem local, jamais cascateada por A2A).
- **(b) Namespace reservado barrado na via de agente:** o padrão `__*__` é RESERVADO. A "validação de nome" do funil (0022 §1) **rejeita** qualquer `NodeAdmission` com role reservado que NÃO chegue pela trigger de (a) — handshake, `⌘T`/`⌘N`, ou `proposed_team` lido do payload (`router.rs:3465`, DADO) nunca conseguem pedir `__reflector__`. Defesa em profundidade: mesmo vazado o nome, falta o CAMINHO.
- **(c) Invisível ≠ privilegiado:** o filtro é SÓ de visibilidade. O sombra permanece sob a allow-list de injeção (default-deny), não recebe `handoff`/`broadcast` de trabalho (excluído dos alvos de fan-out), e **não decide identidade/ordem/autorização** — a única via que o alcança é a fila de reflexão dedicada (`dispatch_reflection`). Invisibilidade não vira "modo deus".
- **(d) Auditabilidade preservada (inv #4):** o filtro vive APENAS na projeção de conveniência (roster vivo/UI/fan-out), NUNCA no event log nem numa auditoria de segurança de presença. `NodeAdded{role:"__reflector__"}` é visível a quem faz replay — a invisibilidade é ergonômica (não poluir o leigo nem virar alvo de trabalho), não um buraco de auditoria.

### Verificação (addendum)

- **Funil intacto:** o sombra produz a sequência canônica do 0022 (teste de paridade segue válido); `NodeAdded{role:"__reflector__"}` presente no log.
- **Invisível mas auditável:** o sombra NÃO aparece em `lina list`/`proposed_team`/alvos de broadcast, MAS aparece no replay do log (a projeção filtra; a auditoria não).
- **Não-forjável (mutação):** um nó de trabalho que tente role `__reflector__` via handshake/`proposed_team`/payload é REJEITADO pelo funil (sem a trigger de sistema + `hops==0`); forjar `requested_by` no payload não passa (sentinela const vence) — espelha o teste do `INTEGRATOR_ROLE`.
- **Invisível ≠ privilegiado:** um `broadcast "*"` de trabalho NÃO injeta no sombra; ele segue sem receber `handoff` e sem decidir autoridade (allow-list intacta por mutação).
