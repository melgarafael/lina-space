# ADR 0004 — custódia de segredo é o gate DURO real de ação externa irreversível (não o pattern-match)

- **Status:** Proposto (será aceito quando W3-6 implementar o broker `lina do` + custódia no Secret Vault)
- **Onda/Story:** Onda 3 · W3-6 (autonomia + gate duro — §1.3 do `tasks/_design-w6-w7.md`)
- **Data:** 2026-06-02

## Contexto

O design exige um **gate DURO** para ações externas irreversíveis (`deploy`, pagamento,
`git push --force`/push em `main`, envio externo). O instinto é **interceptar o comando** por
pattern-match — mas isso é a camada **soft**, com dois furos estruturais:

- O **PATH-shim** (tier 2: prepor `.lina/bin/` ao `PATH` com wrappers de `git`/`rm`/
  `kubectl`/`terraform`/`gh`) é **bypassável por caminho absoluto** (`/usr/bin/git push
  --force`) e só cobre binários **conhecidos**.
- "Grep de intenção" (ler o texto do agente procurando "vou dar deploy") **lê justamente o
  vetor de prompt-injection** e viola o invariante **#1** (o app não interpreta semântica —
  guardrail é determinístico).

A auditoria A2A (achado **A3 — `from` auto-declarado**) expõe o agravante: a submissão A2A
**não autentica o remetente** (o Guardrail 0a do roteador só checa que o nome existe no roster
via `node_by_name`; `MailMessage` não carrega credencial). **Sem identidade autenticada, o gate de autonomia é
burlável**: um agente comprometido pode se passar por `@Maestro` para "autoautorizar" uma
ação, ou pin de identidade falso num pedido de ação privilegiada. Logo o gate não pode
depender de **quem o agente diz ser**.

Isso toca duas âncoras de continuidade — o **Secret Vault** (`lina-secrets`, W0-7,
token-por-nó) e o **Supervisor/autonomia** — → ADR antes de codar.

## Decisão

**O gate duro real de `gated-hard` externo é a CUSTÓDIA DE SEGREDO.** Pattern-match é
defesa-em-profundidade (soft). Composição em **camadas**, da mais fraca à inquebrável:

### 1. Claude Code (tier 1) — hook `PreToolUse` (gate duro nativo, ações locais)

`PreToolUse` → `lina guard --check-action --cmd "<comando>" --autonomy <nível>` → JSON
`hookSpecificOutput.permissionDecision = allow | ask | deny`. **Duro real** para `gated-soft`
+ `gated-hard` **locais**: o harness garante que **toda** chamada de tool (Bash/Write/…) passa
pelo hook antes de executar.

### 2. Tier 2 (Codex/Gemini/shell) — PATH-shim (soft, furável)

Wrappers em `.lina/bin/` chamam `lina guard --check-action` e só fazem `exec` do binário real
em `allow`. **Duro-no-comum, mas bypassável** por caminho absoluto (`/usr/bin/git …`) e
limitado a binários conhecidos. **Limite documentado**, ruidoso quando furado (apenda
`ActionGated{decision:"deny"}` — ADR 0003).

### 3. `gated-hard` EXTERNO (deploy/pagamento/envio externo), TODOS os CLIs — custódia de segredo

A ação externa **depende de um segredo** (deploy key, API key de pagamento, token de webhook).
Se o **Lina** detém o segredo no keyring (**Secret Vault W0-7, token-por-nó**) e **o agente
NÃO o tem**, o agente **não consegue executar a ação sozinho — por construção**. A ação vira
um verbo brokerado: `lina do deploy --env prod` / `lina do pay …`, que o app executa **com o
gate humano** e só ele injeta o segredo. **Inquebrável independente de hook**: sem o token, o
agente não tem caminho alternativo de execução.

### 4. Pré-condição de confiança: o broker AUTENTICA o requisitante (liga-se a A3)

A custódia remove o token do agente, mas o broker `lina do` decide **autorizar/atribuir** com
base em **quem pede**. Enquanto a identidade for **auto-declarada** (A3), a atribuição é
falsificável e qualquer auto-aprovação condicionada a papel/identidade (ex.: "deploy do
@Maestro é pré-aprovado") é burlável. **Portanto:** o broker deve usar a **identidade
autenticada do canal** (a correção de A3 — submissão com binding de identidade/token-por-nó),
não o campo `from`. Sem A3, o gate humano ainda funciona (o humano confirma), mas o **registro
de autoria** (`ActionGated`) é falsificável e **nenhum caminho de auto-aprovação por
identidade é seguro**.

> **Regra dura:** custódia de segredo > PreToolUse hook > PATH-shim. `gated-hard` externo
> **sempre** passa pela custódia + gate humano, em **todos** os níveis de autonomia (a
> autonomia afrouxa `gated-soft` de *confirma* → *auto*, mas **nunca** `gated-hard`).

## Alternativas consideradas

- **Só pattern-match de comando (wrapper/grep de intenção).** Rejeitada: bypassável (caminho
  absoluto), cobre só binários conhecidos, e "grep de intenção" lê o texto do agente — o
  próprio vetor de prompt-injection — ferindo o invariante #1. É camada soft, não o gate.
- **Sandbox por SO (seatbelt/bubblewrap/token de SO restrito).** Fecharia o furo do PATH-shim,
  mas é **pesado e frágil cross-platform** → fora do MVP (candidato a ADR futuro). A custódia
  entrega a garantia inquebrável sem o peso do sandbox.
- **Confiar no nível de autonomia + doutrina (bloco 5 do `CLAUDE.md`).** Rejeitada: a doutrina
  é **texto que o LLM "deveria" respeitar** — não é enforcement. Invariante: nunca confiar que
  o LLM honre texto para ação irreversível.
- **Gate condicionado à identidade `from` da mailbox.** Rejeitada por A3: `from` é
  auto-declarado; condicionar autorização nele é forjável.

## Consequências

- **(+)** `gated-hard` externo **inquebrável por construção** (sem segredo, sem ação),
  independente de hook → vale em **todos** os CLIs (neutralidade multi-CLI, invariante #3).
- **(+)** Compõe com a autonomia: a custódia é o **piso** que a autonomia não afrouxa; o gate
  humano de `gated-hard` é preservado em qualquer nível (invariante #6 — humano no laço para
  o irreversível).
- **(+)** O segredo nunca entra no env do PTY do agente (invariante #2 local-first +
  minimização de exposição de credencial).
- **(−)** Requer o **Secret Vault W0-7 operante** + um verbo `lina do <ação>` **por** ação
  externa suportada (não há custódia genérica para binário arbitrário — é a contrapartida de
  não usar sandbox; ações externas novas exigem um broker novo).
- **(−)** **Depende de A3** (identidade autenticada na submissão) para a atribuição/auto-aprovação
  serem confiáveis. Até A3, o broker opera em modo conservador: **confirma sempre**, sem
  auto-aprovação por identidade, e marca a autoria como não-autenticada no `ActionGated`.
- **(−)** O PATH-shim permanece **furável** (`/usr/bin/git`) — limite conhecido e documentado;
  a garantia forte está na custódia, não no shim.

## Critério de verificação (headless, observável no log)

- **AC-0004.1 (custódia):** com o token de deploy **só** no keyring do Lina (ausente do env do
  PTY do agente), `lina do deploy --env prod` dispara o gate humano e **bloqueia** sem
  confirmação; verificável: o env do agente **não** contém a chave; evento
  `ActionGated{ class:"gated-hard-external", decision:"ask"|"deny" }` no log (ADR 0003).
- **AC-0004.2 (soft tier 1):** `lina guard --check-action --cmd "git push --force origin main"
  --autonomy autonomo` → `ask` (nunca `allow`) + `ActionGated{ class:"gated-hard" }`.
- **AC-0004.3 (soft tier 2 — não-execução):** com `.lina/bin/git` no `PATH` e confirmação
  stubada "não", `git push --force` **não** exec'a o binário real; furo documentado:
  `/usr/bin/git push --force` **passa** (limite do shim).
- **AC-0004.4 (liga-se a A3):** uma vez autenticada a identidade do canal, um pedido `lina do`
  com identidade forjada é **recusado/sinalizado** (`ActionGated` registra autoria autenticada);
  antes de A3, nenhum caminho de auto-aprovação por identidade existe.
