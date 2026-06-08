# ADR 0023 — Re-injeção de doutrina como input humano-proxy (não A2A)

**Status:** Aceito (ratificado pelo Maestro, 2026-06-08). Decisão forçada in-implementation pela story **F1-2-4** (editar terminal vivo); divergência do hint da story escalada com prova pelo Dev 01.

## Contexto

A F1-2-4 (edição viva de 1ª classe) precisa **re-injetar a doutrina atualizada** no terminal **vivo** quando o papel do nó muda (gatilho: confirmação humana no modal M6-E). A story sugeriu, como hint, fazer isso "pela fila serial (`deliver_a2a`) para o próprio nó".

## Problema

O caminho literal do hint **é impossível sem regressão de segurança**, verificado contra o disco:

- `route_message` (router.rs) **exclui o sender** em `resolve()` → `from == to == self` dá `NoTarget`. Um nó nunca se auto-entrega pelo router.
- `WorkspaceTrust::from_members` **exclui pares self** (`if from != target`, `a2a.rs:275`). Logo `deliver_a2a(target, from=target)` → `InjectionDenied`. Forçar com `InjectPolicy::AllowAll` seria **exatamente a regressão que a W5-5 / ADR 0006 fecharam** (default-deny por pertencimento).
- Não existe nó "system"/orquestrador no roster (os seeds são terminais) → não há `from` legítimo para uma doutrina "do sistema → nó".

## Decisão

A re-injeção de doutrina é uma **classe de confiança de INPUT HUMANO (human-proxy)**, não comunicação agente↔agente:

1. **Gatilho:** só `NodeManager::assign_role`, chamado **apenas** no Save do M6-E (UI, pós-confirmação humana explícita). Nenhum caminho de agente alcança `assign_role`.
2. **Entrega:** pela **fila serial de ESCRITA** do Supervisor (`lock_pty` + `enqueue_write`, o **mesmo cano do teclado humano** via `write_human`), **faseada** (`build_paste` → `submit_delay` → Enter separado), escritor único (sem interleave com a digitação humana).
3. **Mailbox separada:** `<.lina>/reinject` — **irmã** do outbox A2A, que o **Router NUNCA lê**. Não toca `deliver_a2a`/`Router`/`WorkspaceTrust` → a suíte de segurança do router fica intacta **por construção** (`gate_w34` verde).
4. **Alvo autenticado:** deriva da **origem autenticada** (o `from` carimbado pelo dono-do-subdir no drain), **jamais** do campo `to` (forjável).
5. **Freio (W4-3):** com o freio ativo, o drain devolve `[]` sem tocar a fila → **enfileira, não injeta**; durável a crash; a doutrina só é injetada após `OrchestrationResumed`.

## Segurança (por que não é escalada)

- O texto é **doutrina gerada pelo app** (`doctrine_reinjection_text(role)`), não controlável por agente.
- O alvo é o **próprio nó** (auto-doutrina) → equivale ao agente digitar o texto no próprio PTY (que ele já pode fazer). Sem privilégio novo.
- **Nenhum verbo `lina` escreve no reinject** (não há `lina reinject`) → a superfície não é alcançável por agente. No pior caso de um drop forjado, o carimbo `from`=dono-do-subdir restringe o alvo ao próprio remetente (auto-injeção inócua).

## Consequências

- Passa a existir uma **superfície de injeção fora do router** (write direto ao PTY do próprio nó, human-proxy). Ela é estreita por construção (mailbox dedicada, sem verbo, alvo=self autenticado).
- **Invariante para o futuro:** qualquer injeção que **não seja** self **nem** human-proxy DEVE passar pelo `deliver_a2a`/`WorkspaceTrust` (o caminho autenticado de duas camadas). **Não alargar** a mailbox `reinject` para aceitar alvos cross-node.
- Se surgir necessidade de doutrina "sistema → nó" (não-self, não-human-proxy), as opções abertas (decisão de costura, não desta story) são: (a) introduzir um nó orquestrador **"Lina"** no roster como `from` autenticado legítimo; ou (b) uma porta dedicada `system-inject` em `lina-core` com seu próprio modelo de confiança e gate. Qualquer das duas é um ADR próprio.

## Relacionados

ADR 0006 (WorkspaceTrust default-deny / W5-5), ADR 0010 (multi-workspace trust por Espaço), ADR 0021 (modelo de segurança da aprovação y/n — a injeção de aprovação tem o seu próprio seam atômico, F1-1-8 #1, ainda pendente).
