# ADR 0060 — Toggle mestre do gate por-Espaço (`workspace.json → guard: on|off`)

- **Status:** **Aceito** (pedido direto do fundador, dono do ambiente, 2026-07-01). Governança: define uma chave de configuração por-Espaço que desliga, **na origem** (renderer + CLI + custódia), o gate de execução W3-6 e as pausas de custódia (ADR 0004). **Não** é uma mudança do default do produto — é um interruptor que o dono aciona por Espaço.
- **Depende de / toca:** ADR 0004 (custódia de segredo), W3-6 (gate de execução `PreToolUse`), ADR 0025 (permission-mode omitido em produção).

## Contexto

O gate de execução (W3-6) instala em cada terminal um hook `PreToolUse` → `lina guard --pretooluse` que intercepta comandos (`git push`, deploy, `rm -rf`, pagamento) e **pausa pedindo confirmação y/n**. O mesmo vale para a custódia de `lina do` (deploy/pay/send): o agente registra, o app pausa no gate humano (⌘⏎) e só então o broker executa com o segredo do cofre (ADR 0004).

Esse gate é o default correto do produto (não-técnico-first, ação irreversível pede humano). Mas o **dono do ambiente**, dogfoodando com um time grande em autonomia autônoma, aprova sistematicamente todas as pausas — o gate só quebra o fluxo. Editar o `settings.json` na mão não resolve: o renderer (W3-2) o **reescreve** a cada mudança de roster, e o hook volta.

A decisão precisa remover a interrupção **na origem** — e ser reversível, por-Espaço, sem tocar o default de ninguém.

## Decisão

Uma chave **`"guard": "on" | "off"`** em `<ws_root>/.lina/workspace.json`, **fonte única** lida por todos os pontos de enforcement (`lina_core::guard_enabled`). Fail-safe assimétrico: **só `"off"` literal desliga**; arquivo/campo ausente, JSON quebrado ou qualquer outro valor ⇒ **ligado**. O gate nunca degrada por acidente — só por escolha explícita do dono.

### G1 — Renderer omite o hook (item #2/#5)
Com `guard:off`, o `settings.json` gerenciado é escrito **sem** o handler `command` do `guard --pretooluse` no `PreToolUse`. SessionStart (turno-0) e os handlers HTTP de observabilidade (F1-1-3) ficam **intocados**; um `PreToolUse` esvaziado é podado (settings limpo). Como o toggle é relido a cada regeneração, o hook **não volta** numa mudança de roster (idempotência).

### G2 — CLI curto-circuita para `allow` (item #3)
`lina guard --pretooluse` e `--check-action` devolvem `allow` imediato quando `guard:off`, **sem** apendar `ActionGated`. É a rede da **chamada residual**: um `settings.json` antigo que ainda tenha o hook (antes da regeneração) nunca trava — defesa em profundidade com o G1.

### G3 — Custódia executa direto em `guard:off` **E** autônomo (item #4)
Os pedidos `lina do` (deploy/pay/send/envio-de-canal) executam **direto** — o broker obtém o segredo do cofre e age, sem o gate humano ⌘⏎ — **apenas** na combinação explícita `guard:off` + autonomia `autônomo`. Em `assistido`/`manual`, ou com `guard:on`, o gate humano permanece a autoridade. O broker relê o toggle a cada pedido (`BrokerPump::with_guard`).

## Segurança — o que este ADR muda e o que NÃO muda

- **A custódia continua inquebrável na sua mecânica (ADR 0004):** mesmo no bypass, o **agente nunca vê o segredo** — quem obtém a chave do cofre e executa é o broker, server-side. O que o `guard:off`+autônomo remove é a **confirmação humana**, não a custódia da chave. O token nunca vaza ao log (provado por teste).
- **Porta que este ADR abre, conscientemente:** em `guard:off`+autônomo, efeitos externos que gastam segredo (deploy/pay/send real) rodam **sem** o último checkpoint humano. Isso contraria o gate §6 do `CLAUDE.md` ("ação irreversível exige gate humano") **por escolha explícita e reversível do dono do ambiente**, escopada ao Espaço que ele marcou. É configuração que o dono aciona, não um enfraquecimento global de segurança.
- **Default preservado:** sem a chave (todo Espaço existente), tudo é byte-idêntico ao histórico — gate ligado, custódia gated. Nenhum usuário herda o bypass.
- **`from`/payload continuam sem autoridade:** o toggle vem de um arquivo do **dono** no `.lina/` do Espaço (a mesma fronteira de posse do `bootstrap.json`), nunca de um campo escrito por agente. Um agente não pode se auto-conceder `guard:off`.
- **Reversível a frio:** trocar para `"on"` (ou apagar a chave) e regenerar restaura o gate — sem migração, sem estado preso.

## Verificação (observável)

- `guard_enabled`: só `"off"` desliga; ausente/`"on"`/lixo/JSON quebrado ⇒ ligado (`lina-core`).
- Renderer: `guard:off` sem observabilidade → `PreToolUse` ausente; com observabilidade → só o handler HTTP (nenhum `guard --pretooluse`); `guard:on` byte-idêntico ao histórico (`lina-bootstrap`).
- CLI (binário real): `git push --force origin main` em autônomo → `allow` **e** zero `ActionGated` com `guard:off`; `ask` + evento com `guard:on` (`guard_cli.rs`).
- Custódia: `guard:off`+autônomo → `BrokerExecuted` + marcador no 1º tick, fila do gate vazia, token nunca no log; default (sem `with_guard`) segue gated (`bridge.rs`).
