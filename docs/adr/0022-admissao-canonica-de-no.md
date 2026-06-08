# ADR 0022 — Admissão canônica de nó (um funil, três tradutores)

- **Status:** Aceito (rodada 360, 2026-06-07)
- **Contexto:** diagnóstico 360° da admissão (`tasks/diagnostico-360-admissao.md`, 7 auditores + síntese, commit `63fcebb`). Root cause confirmado no código em HEAD `7b2bff6`: a admissão de um terminal **não existe como UMA função** — são três implementações paralelas (`seed_one` main.rs:3261, `add_node` bridge.rs:2595, `create_agent_with` bridge.rs:2704), cada uma persistindo um subconjunto diferente do contrato (31 dos 36 nós do log vivo não têm papel nem motor — incluindo os 2 de maior gasto, 759k e 201k tokens), enquanto o cwd REAL do spawn nunca é persistido (`TerminalSpawned { node, cli }`, events.rs:221) e o ramo pasta-própria do ⌘N pula o kit de integração (comentário "SEM CLAUDE.md aqui", bridge.rs:2745) escrevendo-o num dir-fantasma `t{seq}` que ninguém lê. A única dimensão hoje OK é a identidade — exatamente a única com autoridade única (`Supervisor::register`, lina-core/src/lib.rs:1149). Este ADR estende esse padrão vencedor às demais dimensões.

## Decisão

1. **Funil único.** `NodeManager::admit_node(NodeAdmission)` (app/lina-gpui/src/bridge.rs) passa a ser o ÚNICO lugar do app que transforma a intenção "quero um terminal" em nó vivo: validação de nome, alocação de seq/key, política de cwd, bootstrap, `wire_terminal`, sequência de eventos, projeção atômica (model/grids/keys) e rewrite. Os três entry points atuais viram **tradutores finos** de intenção → `NodeAdmission` (`add_node` → terminal default; `create_agent_with` → tradução do CreatePlan do modal; `seed_one` → admissão com flag de seed/demo) — **proibidos de apendar eventos ou tocar o Supervisor diretamente**. Toda porta futura (API, CLI, inteligência da Lina) entra por aqui ou não entra.

2. **Sequência canônica e COMPLETA de eventos** (toda admissão, qualquer porta):
   `NodeAdded` + `TerminalSpawned { node, cli, cwd }` + `NodeRoleAssigned` **SEMPRE** ("terminal" também é papel — fim da classe dos 31-sem-papel) + `CliProfileSet` sempre que o profile é conhecido. Um **teste de paridade** garante que as três portas produzem a MESMA sequência módulo parâmetros.

3. **Binding node↔cwd persistido.** Campo **aditivo** `cwd` no payload do `TerminalSpawned` (`#[serde(default)]` — replay de log antigo nunca quebra; doutrina de eventos aditivos do CLAUDE.md). É o pré-requisito de toda correlação nó↔sessão↔custo futura; sem ele a correlação é irreconstituível para sempre. Para o log histórico (sem cwd), as projeções degradam **honestamente** ("custo não rastreável aqui"), nunca chutam.

4. **Política de kit de integração por tipo de pasta** (`CwdPolicy`):
   - `Managed(key)` — kit completo (CLAUDE.md/doutrina + settings/hooks + skill `lina-agent-bus`) no dir gerenciado, como hoje.
   - `UserDir(path)` — kit entregue **no cwd REAL** mediante **consentimento explícito no modal** ("a Lina vai criar arquivos de orquestração nesta pasta"); escrita **merge-safe/append** — um `CLAUDE.md` pré-existente do usuário **jamais é sobrescrito** (recusa com aviso). Sem consentimento → **degradação VISÍVEL** (badge "agente sem doutrina/observabilidade" no card). Silencioso, nunca.
   - Fim do dir-fantasma: `rewrite_bootstrap` itera o binding node→cwd-real; não cria mais `<ws_root>/t{seq}` órfão para nós em pasta própria.

5. **Persiste-antes-de-projetar.** O nó não existe no roster vivo (agents.json, AppPermissionWatch) sem existir no log: `NodeAdded` apendado antes da projeção — ou, onde a ordem prática impedir (o NodeId nasce no `register`), **compensação obrigatória** em falha de append (unregister + `retire_pty`), com teste.

6. **Dois event stores, fronteira intencional** (passo 8 do blueprint): `<ws>/.lina/events` é a **única fonte da verdade do Espaço** (replay de projeções lê SÓ dele). `onboarding/events` (onboarding.rs:386) é **scratch descartável** do fluxo de onboarding — não participa de replay do Espaço; se algum fato do onboarding precisar sobreviver (ex.: `WorkspaceCreated`), ele é re-emitido no store principal no fechamento do onboarding. Sem terceiro store, nunca.

## Por quê assim

- **A obrigação mora no caminho, não na memória de quem escreve a próxima porta.** Hoje cada nova forma de criar agente precisa LEMBRAR de N obrigações espalhadas; com o funil, não tem como esquecer — é o mesmo argumento que fez `Supervisor::register` ser a autoridade do NodeId (única dimensão sem bug no 360°).
- **Aditivo > paralelo** ([[contrato-evento-fidelidade-vs-contorno]]): o cwd entra no evento que JÁ marca o spawn, não num evento novo — consumidores antigos ignoram o campo, replay antigo segue válido.
- **Degradação visível > mágica silenciosa** (inv#6, não-técnico-first): se o kit não pode ser entregue, o fundador VÊ o badge — nunca um agente "meio-cidadão" invisível.

## Consequências

- `add_node`/⌘T deixa de criar nós-órfãos de papel; custo/atividade/skill passam a funcionar para pasta-própria (consentida); o painel ganha o dado para distinguir "sem sessão hoje" de "trabalhando fora da pasta do Espaço".
- O log HISTÓRICO continua com 31 nós sem papel/cwd — irrecuperável por replay (aceito; as projeções degradam honestamente).
- Escrever na pasta do usuário é decisão de produto com gate humano: o **copy do consentimento passa pelo fundador** antes do reempacote.
- Revisitar este ADR se: (a) surgir admissão remota (API/webhook → exige autenticação além do consent local); (b) multi-Espaço (ADR 0010) exigir admissão cross-workspace.

## Verificação

Teste de paridade (3 portas → mesma sequência de eventos módulo parâmetros) + testes existentes (`add_node_grows_count_persists_and_reconstructs`, bridge.rs:4415) + teste de compensação (falha de append → nó fora do roster) + teste merge-safe (CLAUDE.md pré-existente intocado) + tela do fundador no `dist/Lina.app` reempacotado (teste duplo: skill `lina` no ⌘N + avisinho vivo).
