# ADR 0051 — Ingestão hospedada de webhook: um gateway na nuvem + conexão reversa de saída (local-first preservado)

- **Status:** **Proposto (2026-06-25).** Desenho do "ponto de ingestão hospedado". É **ADR-gate**: nenhuma story de webhook **hospedado** inicia até ser aceito.
- **Onda/Story:** **Ponta de F5 (épico 42 — cross-machine / Lina no VPS) puxada por dor de F4-WA.** Esta decisão NÃO move o escopo oficial: o endereço-público hospedado permanece tema de F5 (igual ao ADR 0034). F4-WA entregou o webhook **ativo** sobre `127.0.0.1` (ADR 0035); o que falta para virar produto é a **chegada hospedada**, que se apoia na porta cross-machine de F5. Aceitar este ADR ⇔ a primeira story de ingress hospedado implementar o transporte reverso + o gate de exposição com a suíte de segurança do Router verde.
- **Data:** 2026-06-25
- **Fontes:** achado de dogfooding "PRODUTO · webhook do Lina não recebe SaaS de prateleira" (2026-06-24, `tasks/despachos/achados-dogfooding-sessao.md`) · correção do fundador (o gargalo é EXCLUSIVAMENTE o endereço, não a tradução de formato — a IA viva interpreta o payload cru, ADR 0035) · **ADR 0034** (`BusTransport` — a porta cross-machine a REUSAR) · **ADR 0035** (origem `sistema/webhook` carimbada server-side; instrução=autoridade, payload=dado) · **ADR 0004** (custódia de segredo / gate-hard externo) · **ADR 0006/0007** (admissão default-deny; nenhum campo de agente é autoridade) · **ADR 0010** (multi-workspace, trust com namespace por Espaço) · invariantes #2 (local-first), #4 (event log), #5 (pertencimento), #7 (core/shell split) (`CLAUDE.md`).

## Contexto

O webhook **ativo** já funciona e está blindado: `lina-webhooks` (axum + HMAC, `crates/lina-webhooks/src/lib.rs`) recebe o POST, valida, append-aguardado de `WebhookReceived` (durabilidade primeiro), e despacha pelo canal A2A carimbando `MsgOrigin::System{webhook_id}` **server-side** — origem inforjável por construção (ADR 0035, ACEITO). A doutrina origem×autoridade está resolvida: o payload externo é **dado não-confiável**; a instrução do dono é **autoridade**; a IA viva lê o payload **cru** e age. **Não há problema de "tradução de formato/assinatura por provedor"** — isso é complexidade de um produto SEM IA, indevida aqui (decisão do fundador, registrada).

O único gargalo é o **endereço**. `ensure_local` (`lib.rs:612`) força bind em loopback e recusa qualquer interface de rede — é o enforcement do invariante #2 (local-first). Consequência prática (achado de dogfooding): a URL do hook é `http://127.0.0.1:<porta-aleatória>/hook/<id>`, inalcançável por um SaaS externo (Featurebase, Stripe, formulários) e instável entre reaberturas. Pedir ao usuário leigo que monte um túnel (ngrok/cloudflared na mão) é inviável em escala e quebra o "não-técnico-first" (invariante #6).

A pergunta que este ADR fecha não é *como interpretar o evento* (resolvido) nem *como traduzir assinaturas* (não é problema) — é **onde fica o ponto de chegada público e por onde os bytes entram na máquina do usuário sem o app abrir uma porta de escuta.** Isso toca diretamente o invariante #2 e a fronteira cross-machine do ADR 0034 → ADR antes de codar.

## Decisão

**A chegada do webhook passa a poder ocorrer num GATEWAY de ingestão hospedado pela Lina (endpoint público, domínio estável por hook), que NÃO escuta no lugar do app: o app local abre uma CONEXÃO REVERSA DE SAÍDA para o gateway, e o gateway empurra o POST recebido por essa linha já aberta. O gateway é uma APLICAÇÃO da fronteira `BusTransport` (ADR 0034) — transporte, jamais autoridade.**

### 1. O gateway é um *ingress* sobre o `BusTransport`, não um transporte paralelo

`BusTransport` (ADR 0034) abstrai **por onde** um `A2aEnvelope` chega ao nó destino. A ingestão hospedada é uma impl/aplicação dessa trait — um **`RemoteIngressTransport`** ao lado do futuro `RemoteOverSSH`, ambos do mesmo eixo F5 cross-machine. O envelope que chega tem origem `System{webhook_id}` (ADR 0035) e é entregue ao nó local pelo **mesmo `deliver_a2a`/Router de sempre**: a cadeia de validação (dedupe → anti-loop → remetente → alvo → autonomia → fan-out → orçamento → anti-deadlock) roda **idêntica**. O gateway só muda **a fonte dos bytes**; não cria motor de entrega novo, não amplia quem pode injetar em quem (admissão continua default-deny por pertencimento, ADR 0006/0010).

### 2. Conexão reversa de saída — o app NUNCA abre porta (local-first por construção)

O app local **disca** para o gateway (conexão de SAÍDA, iniciada de dentro pra fora). O gateway, ao receber um POST em `https://<host>/h/<capability>`, encaminha o payload pela conexão reversa correspondente. **Não existe `bind` de escuta no app** — o invariante #2 é preservado por ausência de porta, não por um guard que pode falhar. `ensure_local` permanece válido e inalterado para o modo loopback (dev / integrador que o usuário controla); o modo hospedado é um **caminho de entrada diferente**, não um afrouxamento do bind local.

### 3. O gateway é um relay cego (zero-knowledge do conteúdo) — autoridade fica no app local

O gateway move bytes e roteia por capability. Ele **NÃO**: confere o HMAC do Lina, carimba a origem `System`, interpreta o payload, nem precisa lê-lo. Todo o handler blindado de `handle_hook` (rate-limit em 3 camadas, 404 opaco, append-aguardado de `WebhookReceived`, carimbo `System{webhook_id}`, despacho A2A) roda **no app local, como hoje** — só recebe os bytes do gateway em vez do `TcpListener` loopback. Assim a doutrina origem×autoridade (ADR 0035/0007/0004) vale integralmente: **nenhum campo vindo do gateway ou do payload decide identidade, ordem ou autorização**. Um gateway comprometido pode, no pior caso, entregar lixo ou deixar de entregar — **nunca** forjar a origem `sistema/webhook` (o carimbo é server-side **no app**, não na nuvem) nem furar a custódia (ADR 0004) de ação irreversível.

### 4. Abrir a exposição é opt-in sinalizado + gate humano + escopo mínimo

Espelha o ADR 0034 §3 (cruzar a máquina é ação de impacto externo):

- **Opt-in sinalizado** — o modo hospedado só existe se o humano o criar; enquanto existir, a UI sinaliza visivelmente "este hook está exposto na internet via gateway da Lina" (espelho de "exposições são opt-in sinalizado", invariante #2).
- **Gate humano** — abrir a exposição pública de um hook é **ação irreversível de impacto externo** (classe ADR 0004 / `CLAUDE.md` §6): confirmação humana explícita. Nenhum agente abre exposição.
- **Escopo mínimo** — a exposição é **por hook** (uma capability), não "o Espaço inteiro na internet". Default = nada exposto.

### 5. Autenticação app↔gateway, e a chegada por capability (a superfície de segurança)

- **(a) app↔gateway (provar que ESTE app é o dono daquele hook):** ao criar um hook hospedado, o app registra-se no gateway e recebe um par **`capability` pública (na URL) ↔ `device_token` secreto**. O `device_token` vive no Secret Vault local (keyring, ADR 0004) — **nunca no log** (igual ao secret HMAC). A conexão reversa é autenticada por esse token (bearer sobre TLS, ou mTLS). O gateway entrega o POST de `/h/<capability>` **exclusivamente** na conexão reversa autenticada daquele device. Posse do hook = posse do `device_token`, jamais um campo do payload.
- **(b) multi-tenant (isolar usuários):** cada `capability` é opaca e não-enumerável (a base32 de 160 bits que o `hook_id` já é), ligada a **um** `device_token`. Vazar a URL de um usuário **não** dá acesso a outro: a URL só roteia para a conexão reversa do dono, e o conteúdo só é processado no app do dono. Isolamento por capability + binding de device, nunca por identificador adivinhável.
- **(c) DoS no endpoint público:** o gateway aplica as **mesmas 3 camadas** que `handle_hook` já implementa (balde por rota/capability → teto global → e o app aplica o budget-por-hook pós-recepção), + teto de corpo (64 KiB) + 404 opaco contra varredura. O gateway absorve o flood **antes** de gastar a conexão reversa e a CPU local — protege a banda do túnel e a máquina do usuário.
- **(d) privacidade em trânsito:** TLS provedor→gateway e gateway→app. O gateway é **relay sem armazenamento por padrão** (encaminha e descarta; não persiste payload — ele nem precisa lê-lo, §3). Se o app estiver offline, o desfecho default é **não bufferizar** (o provedor recebe um 5xx/timeout e re-tenta conforme a política dele); um buffer com TTL curto no gateway, se necessário, é **opt-in sinalizado** ao usuário (espelha a DLQ do ADR 0035 §4, que vive no app, não na nuvem).

### 6. Eventos aditivos — abrir/fechar exposição é fato do log (invariante #4)

Criar/abrir/fechar um hook hospedado e admitir a conexão reversa são **fatos do Espaço** → eventos aditivos (`serde(default)`, como toda a série), projetáveis e re-deriváveis por replay, numa story de F5. O `device_token` entra como **referência** ao Secret Vault, jamais em claro (igual ao secret HMAC em `WebhookConfigured`). O gateway não é dono de estado que escapa do log.

### 7. Wire-protocol: túnel HTTP/gRPC reverso (preenche a lacuna deixada pelo ADR 0034 — não diverge)

O ADR 0034 deixou explícito que **não** escolhe o wire-protocol e que "SSH é a base canônica de partida, não a decisão final — ADR próprio quando F5 abrir". Este é esse ADR próprio para o caso **ingress-webhook**: a base canônica aqui é um **túnel reverso HTTP/gRPC** (padrão `cloudflared`/FRP), **não** SSH — porque a outra ponta é um gateway HTTP público recebendo POSTs de SaaS, não outro Lina falando A2A. Isso **não diverge** do ADR 0034: a fronteira `BusTransport` é a mesma; `RemoteIngressTransport` (túnel HTTP reverso, ingress externo) e `RemoteOverSSH` (A2A PC↔PC/VPS) são duas impls da mesma trait, escolhidas pela natureza de cada ponta.

## Modelo de hospedagem — 3 opções avaliadas + recomendação

O eixo que decide é **a conexão reversa persistente de saída** (o app mantém uma linha longa aberta; o gateway empurra por ela). Endpoint HTTP público estável e absorção de DoS são commodity; segurar conexões longas multi-tenant com pouca ops é onde as opções diferem.

| Opção | Custo recorrente | Ops / complexidade | Conexões persistentes | Escala | Veredito |
|---|---|---|---|---|---|
| **VPS próprio** (1 droplet rodando relay FRP/próprio) | Fixo baixo (~$10–40/mês **total**, dividido por TODOS os usuários → fração de centavo/usuário acima de ~centenas) | Alta — você patcheia, monitora, escala à mão; ponto único de falha sem LB | Boa (processo long-lived é o caso natural) | Vertical até um teto; depois exige LB + múltiplos nós | Bom controle, sem vendor lock-in; paga em **ops**. Fallback se a dependência de fornecedor for veto. |
| **Serverless puro** (Lambda / Workers sem estado) | Por requisição (paga o que usa) | Baixa de servidor | **Fraca** — stateless/efêmero não segura conexões longas; Lambda tem timeout, exigiria API Gateway WebSocket (custo+complexidade) | Automática | **Rejeitado para o relay**: o calcanhar é exatamente a conexão persistente. |
| **Docker gerenciado** (Fly.io / Render / Railway) | Moderado por uso | Média — empacota o relay; a plataforma faz deploy/saúde/escala | Boa (Fly.io é forte em conexões longas + edge) | Horizontal gerenciada | Meio-termo sólido: menos ops que VPS, mais controle que serverless. |

**Recomendação decisiva: Cloudflare como base — operar o `cloudflared` (Cloudflare Tunnel) embutido no app, com o gateway na rede Cloudflare.** É a correspondência exata do desenho: o `cloudflared` É a conexão reversa de saída (app disca pra fora; Cloudflare empurra), já entrega TLS + domínio estável + absorção de DDoS na borda + escala multi-tenant — com o **menor custo por usuário e a menor ops**. Conexões persistentes são o caso de uso nativo do produto (não um workaround como em serverless puro). Se for preciso lógica própria no edge (rate-limit por capability, roteamento), **Workers + Durable Objects** complementam sem sair do mesmo fornecedor. **Fallback** (se a dependência de fornecedor for inaceitável): **VPS próprio com relay FRP** — mesmo desenho, trocando custo-de-ops por independência. **Serverless puro fica fora** para a função de relay.

**Ordem de grandeza do custo por usuário:** dominado pelo gateway e desprezível em qualquer opção a partir de escala modesta — **bem abaixo de US$ 0,10/usuário/mês** para o volume típico de webhooks (dezenas a centenas de POSTs/dia por usuário). O que varia entre as opções **não é o $ direto** — é a **ops** e o **teto de escala**. Por isso a recomendação pesa ops/escala, não preço de tabela.

## Limite explícito (o que este ADR NÃO faz)

- **Não implementa** o gateway nem o `RemoteIngressTransport`. Entrega o **desenho** + a costura (reuso de `BusTransport`/ADR 0034) para a story de F5.
- **Não mexe** no HMAC, no formato de assinatura, nem cria "perfis por provedor". O gargalo é o endereço; a interpretação é da IA viva (ADR 0035). *(Nota de fronteira, fora do escopo: para um SaaS de prateleira que assina diferente do Lina, o requisito de HMAC-no-formato-Lina de `handle_hook` precisaria de uma decisão SEPARADA — não é deste ADR, que trata só do transporte.)*
- **Não move o escopo oficial.** Endereço-público/VPS/cross-machine continuam F5 (épico 42). Isto é desenho de "ponta de F5 puxada por dor de F4-WA"; mover o escopo exige aval do fundador/Maestro.
- **Não escolhe** o contrato de eventos final nem o handshake do túnel a nível de wire — fica para a story de F5, sobre esta porta.

## Consequências

- **(+)** Reusa **dois padrões já provados**: a porta `BusTransport` (ADR 0034) e o handler blindado de `lina-webhooks` (rate-limit, append-aguardado, carimbo server-side). Sem motor novo; só troca o cano de entrada. O webhook hospedado é "mais uma impl de trait", não re-arquitetura.
- **(+)** Local-first preservado **por construção** (app nunca faz bind); a exposição é opt-in sinalizado + gate humano + escopo por-hook.
- **(+)** Gateway cego ao conteúdo → privacidade forte e superfície de comprometimento mínima: o pior caso de um gateway hostil é não-entrega/lixo, nunca forja de origem ou furo de custódia (a autoridade fica no app local).
- **(+)** URL pública estável por hook → o caso de uso central do fundador (receber feedback/bugs de SaaS) passa a funcionar sem o leigo virar técnico de rede.
- **(−)** Introduz **dependência operacional de uma peça hospedada** (a Lina passa a manter infra na nuvem) — custo recorrente real (ainda que baixo por usuário) e um componente a monitorar; é a contrapartida inevitável de "prover endereço público".
- **(−)** Disponibilidade da chegada passa a depender do gateway estar no ar; mitigado por re-try do provedor + (opt-in) buffer com TTL no gateway, mas é uma dependência que o modo loopback não tinha.
- **(−)** Cada novo eixo cross-machine (este ingress + o `RemoteOverSSH` de A2A) é uma impl de `BusTransport` a manter e a red-teamar.

## Alternativas rejeitadas

- **App abre porta pública (`0.0.0.0`) / toggle de "expor na internet".** Viola o invariante #2 por construção: local-first deixaria de ser garantido por ausência e passaria a depender de uma flag certa. O `ensure_local` existe exatamente para barrar isso. A conexão é de SAÍDA, sempre.
- **Cada usuário monta o próprio túnel (ngrok/cloudflared na mão).** Inviável em escala e quebra o não-técnico-first (invariante #6). O túnel tem de ser **operado pela Lina**, embutido no app.
- **Transporte de ingestão paralelo, fora do `BusTransport`.** Fecha a porta de continuidade (invariante #7): duplicaria roteamento/entrega e o core passaria a conhecer I/O de rede em dois lugares. O ingress é uma impl da trait, não um cano à parte.
- **Gateway que confere HMAC / interpreta / carimba a origem.** Põe autoridade na nuvem: um gateway comprometido forjaria a origem `sistema/webhook`. A autoridade fica no app local (ADR 0035); o gateway é relay cego.
- **Gateway que armazena o payload por padrão.** Viola privacidade/local-first sem necessidade — o gateway nem precisa ler o conteúdo. Buffer só com TTL curto e opt-in sinalizado.
- **Catálogo de "perfis de assinatura por provedor".** Rejeitado pelo fundador: a IA viva interpreta o payload cru (ADR 0035); traduzir formato é complexidade de produto sem-IA. Não é o gargalo.
- **Serverless puro (Lambda) para o relay.** O calcanhar é a conexão persistente de saída, que serverless stateless não segura bem. Cloudflare Tunnel / Durable Objects ou Docker gerenciado / VPS seguram conexões longas nativamente.

## Gate

Gate: stories que dependem desta decisão (ingress hospedado de webhook) **não iniciam até este ADR ser aceito** — mal-desenhado, fecha portas de continuidade (#7) ou regride a doutrina de segurança (#2 + admissão ADR 0006/0010 + origem×autoridade ADR 0035). Toda story que tocar a entrega tem como critério implícito a **suíte de segurança do Router verde** (origem `sistema/webhook` inforjável carimbada no app local + custódia ADR 0004 + admissão default-deny). Aceitação ⇔ a primeira story de F5/ingress implementar o `RemoteIngressTransport` (conexão reversa de saída) + o gate de exposição por-hook, com a suíte de segurança do Router verde e o red-team (forja de origem via gateway, isolamento multi-tenant, DoS no endpoint, privacidade em trânsito) passando.
