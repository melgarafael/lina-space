# lina-shim — PATH-shim do gate de execução (W3-6, tier 2)

Gate de ação irreversível para CLIs **sem hook** (Codex / Gemini / shell). Complementa o hook
`PreToolUse` do Claude Code (tier 1, gate DURO de verdade). Mecanismo: o app prepõe `.lina/bin/` ao
`PATH` do agente com wrappers que interceptam o **comando real** antes de executar.

## Arquivos

- **`lina-shim.sh`** — wrapper genérico. O nome com que é invocado (via link) define a ferramenta:
  `git` → `lina-shim.sh` ⇒ `TOOL=git`. Consulta `lina guard --check-action` e só faz `exec` do
  binário real se a decisão for `allow`; em `ask`/`deny`, o canal de confirmação STUB (default
  "não") **não executa** o binário real e sai com código != 0.
- **`install.sh`** — copia o wrapper para um diretório-alvo e cria os links nomeados
  (`git rm kubectl terraform gh deploy`).

## Como o app integra

1. `sh install.sh "<workspace>/.lina/bin"`.
2. Prepor `<workspace>/.lina/bin` ao `PATH` do PTY do agente.
3. Exportar no PTY do agente:
   - `LINA_SHIM_DIR=<workspace>/.lina/bin` (resolução robusta do binário real);
   - `LINA_AUTONOMY=<manual|assistido|autonomo>` (nível vigente do workspace);
   - `LINA_HOME=<.lina compartilhado>` (onde o `lina guard` apenda `ActionGated`);
   - `LINA_CONFIRM` permanece **não setado** (stub recusa); o gate humano real liga aqui.
4. `lina` deve estar resolvível no `PATH` do agente (binário do app).

## Limite conhecido (honesto — design §1.3)

O shim é furável por **caminho absoluto**: `/usr/bin/git push --force` **ignora** o
`.lina/bin/git` e roda o git real. Só o hook `PreToolUse` (Claude Code) é gate verdadeiramente duro.
Para `gated-hard` **externo** (deploy/pagamento), a camada inquebrável é a **custódia de segredo**
(W0-7, token-por-nó no keyring): o agente nunca recebe a credencial, então não há caminho de
execução alternativo — ele **precisa** pedir ao Lina, que aí roda o gate humano.
