# Despacho F3-2 · TRADUTOR — o papel que interpreta-antes-de-executar (F3-2-1)
**Para:** Terminal H · **model·effort:** opus · Medium · **Dono de:** `crates/lina-role-discovery/` (especialmente `src/default-roles.yaml`) + a skill do Tradutor em `assets/`

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (caminho absoluto).
- **LEIA primeiro:**
  1. `tasks/epico-f3/onda-f3-2.md` §"frentes" (sua fronteira) + §gate (a).
  2. Spec 52 §"Portas que NÃO fecha" (linha ~394): *"O papel 'Tradutor' (linha 83) — o terminal que interpreta-primeiro — é o origin natural de `GoalInterpreted` (`origin:"@Tradutor"`), mas não existe no role-discovery."* Arquivo: `/Users/rafaelmelgaco/Documents/Obsidian Vault/Ecossistema Labs - Operação/Debriefing Vibe Coding/52 - SPEC Goal-and-Loop - A Meta como Primitiva.md`.
  3. O registry atual: `crates/lina-role-discovery/src/default-roles.yaml:22-73` (12 papéis: MAESTRO/ARQUITETO/FRONTEND/BACKEND/LLM_ENGINEER/DATA_ENGINEER/UIUX_DESIGNER/WRITER/QA/BUG_FIXER/CURADOR/AUTOMATOR + fallback DEVELOPER `:17`). Os doc-comments de `events.rs:591,597` já reservam `@Tradutor`.
  4. Veja um papel existente como modelo de entrada YAML (campos, ordem "first match wins", `needs_confirmation`).

## FUNÇÃO
Você é o **dono do papel Tradutor** no role-discovery. Entrega: a entrada no registry + a skill/doutrina que define o comportamento do Tradutor (interpreta o que o vibe coder envia; sempre devolve interpretação + estratégia ANTES de executar; é o único terminal com que o leigo trabalha quando aberto) + a **degradação** (sem Tradutor → o Maestro assume; é rótulo de proveniência, não credencial de segurança).

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `crates/lina-role-discovery/` e na skill nova em `assets/` (alinhe o caminho exato com o Maestro). **O ROTEAMENTO da 1ª mensagem ao Tradutor é costura de `router.rs` → é do CORE (Terminal B).** Você define o PAPEL e a regra de classificação; não toque router.rs. Se precisar que o CORE leia `origin:"@Tradutor"`, **peça ao Maestro**.
- **Regra-mãe:** o papel Tradutor é **rótulo de proveniência**, JAMAIS autoridade. Ter o papel não concede permissão; a autenticação em duas camadas segue soberana. `GoalInterpreted{origin:"@Tradutor"}` é só proveniência.
- Convenções: `cargo fmt -p lina-role-discovery`, `clippy -D` limpo, teste que prova a classificação (input do leigo → papel Tradutor; ausência → fallback).

## OBJETIVO (o porquê de negócio)
Quando o leigo abre o Lina e fala, ele deve falar com **um intérprete** que devolve "entendi X, vou fazer Y" antes de agir — não com um executor que sai fazendo. O Tradutor é a porta de entrada humana do método do doc-fonte (linha 83).

## ESCOPO — F3-2-1
- Adicione o papel **TRADUTOR** ao `default-roles.yaml` (descrição, gatilhos de detecção, `needs_confirmation` coerente com os pares). Decida a posição na ordem "first match wins" com critério (interpretar-primeiro é genérico — cuide para não capturar papéis específicos por engano; teste isso).
- Escreva a **doutrina/skill do Tradutor** (texto acima do CLI, inv #1): sempre devolver interpretação + estratégia + critério de aceite ANTES de executar; nunca decidir segurança; quando o pedido for técnico-específico, propor o time (não fazer tudo sozinho).
- **Degradação:** documente e teste que, sem Tradutor no roster, o fluxo cai no Maestro sem quebrar (rótulo, não credencial).

## RESULTADO ESPERADO (formato exato)
- Diff no `default-roles.yaml` + crate role-discovery (se a classificação exigir) + a skill nova.
- Teste de role-discovery verde: input típico de leigo → TRADUTOR; sem a entrada → fallback DEVELOPER/Maestro. Rode e cole a contagem.
- `cargo test -p lina-role-discovery` verde; `clippy -D` 0; `fmt` limpo.
- **NÃO commite.** Reporte o 1º progresso (`lina ask "@Terminal A" "comecei o Tradutor" --intent status`).
- Termine com **`PRONTO: <resumo + arquivos>`** ou **`BLOCKED: <motivo>`**.
