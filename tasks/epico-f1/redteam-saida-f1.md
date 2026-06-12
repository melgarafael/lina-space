# Red-team de SAÍDA do Épico F1 — passada adversarial FINAL sobre o épico inteiro

> **Auditor de invariantes ISOLADO** (terminal "Red Team", papel QA) — READ-ONLY no código de
> produção; nada editado/commitado. Não construí nenhuma onda. Meu trabalho foi **refutar**.
> **HEAD auditado:** `f1d4810` (gate de saída F1). **Método:** cada invariante **re-derivado no
> código** (arquivo:linha), focando o **DELTA F1-4/5/6** e as **COSTURAS entre ondas** que os
> red-teams por onda (`redteam-gate-f1-1.md` HEAD `2de17cd`; `redteam-spawn-f1-3-6.md` commit
> `76a3ccd`) não puderam cobrir porque o seam ainda não existia. Mapa do código levantado por
> 4 agentes `Explore` read-only; **toda classificação foi re-derivada por mim na linha-fonte
> antes de assinar** (finder infla nit→ALTA; o red-teamer re-deriva).

## Veredito

**0 ALTA aberto.** Critério do fundador para a saída do épico (**0 ALTA para sair; MEDIA→backlog
com dono**) — **ATENDIDO** no eixo adversarial. Os três MEDIAs que o red-team do spawn marcou
como *"o seam DEVE resolver"* (M2 autonomia fiada, M3 dedupe durável, M4 binding de cascata no
filho) foram **honrados no HEAD de saída** — re-derivados verdes abaixo. Restam **3 MEDIA** novos
(doc-vs-código / feature-não-fiada / modelo local-first) + os carry-forward já aceitos (M1 do
spawn, B1 custo-off) → backlog com dono, **não bloqueiam a saída**.

---

## Achados (severidade · invariante · arquivo:linha · evidência re-derivada · dono)

| # | Sev | Invariante tocado | Arquivo:linha | Evidência re-derivada | Dono sugerido |
|---|---|---|---|---|---|
| **MM1** | MEDIA | "Isolamento por OBJETO DISTINTO — cruzar barramentos é impossível por construção" (`runtime.rs:10-11`) **vs.** doutrina A2A "nenhum campo do agente decide autorização" | `runtime.rs:54-85,318-322` · `mailbox.rs:519,595` · `workspace_boot.rs:21-31` | A isolação **in-process** VALE (Supervisor/store/mailbox distintos por Espaço — nenhum caminho de código roteia o bus de A para B). MAS o **mailbox é filesystem** (`<ws>/.lina/outbox/<nó>/`), escrita do **mesmo usuário de SO**. Um agente malicioso em A lê `~/.lina/workspaces.json` (registry global, mesmo-usuário), descobre o path de B, faz `LINA_HOME=<B>/.lina` e escreve em `B/.lina/outbox/<peer>/` → injeta A2A no bus de B, `from` carimbado pelo subdir que ele escolheu. **Impacto BOUNDED** (∴ não-ALTA): a msg passa pelo gate COMPLETO de B (router→pertencimento→deliver_a2a→guard/custódia de B); **não** escala a ação irreversível (custódia exige ⌘⏎ humano de B + segredo do cofre de B), **não** forja aprovação de cascata (computada dos bindings de B), **não** spoofa decisão de aprovação (presa ao nó do LOG de B). É o **mesmo modelo same-user-FS já aceito** no single-workspace (A2 "from-spoof" refutado: `from` não decide autorização); multi-workspace só **estende** o alcance, sem mudar a consequência de autorização. Achado = **clareza de doc** ("impossível por construção" é verdade só in-process, não FS-level vs agente malicioso) + nota de defesa-em-profundidade. | core/multi-workspace + docs (ADR 0010/0027) |
| **MM2** | MEDIA | F1-5-8: "leitura cross-terminal respeita pertencimento e audita" | `history.rs:249-337` · chamadores | A API cross (`tail_cross`/`search_cross`/`export_cross`) é **correta**: default-deny por pertencimento (`cross_allowed` 249-251), **audita ANTES de ler** (`audit_cross` 312-337), e **auditoria-falhou → leitura NEGADA** (335). PORÉM **nenhum chamador de produção** — só testes (`history_f1_5_8.rs`); nenhum verbo CLI (`lina.rs` não tem `history`) nem chamada no `bridge` cruzam terminal (o único uso em prod é `store.tail(&panel,…)` `bridge.rs:3824`, re-hidratação do **próprio** painel no restore). **Bom para superfície de ataque (0 alcançável), mas doc-vs-código:** o critério do F1-5-8 vale **só no nível de API**; a feature não está fiada. **Corrobora o achado de dogfooding #15** (`achados-dogfooding-sessao.md`): o fundador apontou que o orquestrador é CEGO ao conteúdo dos colegas — exatamente a fiação de `lina history` que falta. Decisão honesta: fiar OU marcar deferida. | core/history (fiar ou deferir explicitamente) |
| **MM3** | MEDIA | Inv#4 (event log = verdade) sob **log adversarial** (axis 2) | `events.rs:1324-1338` · `runtime.rs:146-155` | O event log e o registry global (`workspaces.json`) são **same-user-graváveis** (modelo local-first). Forjar eventos/registry corrompe **projeções** (observabilidade/foco de boot) — MAS **não ressuscita autoridade** além de um gate humano vivo (custódia exige ⌘⏎ + segredo; o cost-pause é disponibilidade soft, não autorização) e **não quebra o boot**: linha JSONL ilegível/corrompida é **posta em quarentena** (`continue`/`warn`, sem panic), evento de kind desconhecido é ignorado na projeção (aditividade `serde(default)`). Impacto bounded = integridade de projeção, **o modelo local-first documentado**. Registro com o limite explícito. | aceito (local-first) / F2 se quiser hardening de integridade |

**Carry-forward já aceitos (NÃO reabertos):** **M1** do spawn (poda de binding 60s reabre um
ex-filho como "origem" — tempo≠campo; poda system-wide; sem velocidade de fork-bomb) **permanece**
como MEDIA aceito; **B1** (teto de custo OFF por default = opt-in) inalterado. Ambos convergem com
o gap de **origem-burst** já aceito (origem não é capada; a defesa central é a CASCATA).

---

## Confirmações positivas de invariantes (com a linha que ENFORÇA)

### As 3 promessas do seam (carry-forward do red-team do spawn) — RE-DERIVADAS VERDES

| # | Invariante (o seam DEVIA resolver) | Linha que garante | Veredito |
|---|---|---|---|
| **M4** | Terminal spawnado NASCE com binding de cascata (senão a defesa anti-fork-bomb cai end-to-end) | `bridge.rs:768-827` (`post_process_spawns` indexa `SpawnRequested` — fonte INFORJÁVEL de root/hops — e chama `seed_delivered_root(node, info.root, info.hops)` em `826-827`) **ANTES** de enfileirar o 1º prompt (`841`) | ✅ **RESOLVIDO.** O filho não age antes de nascer com cadeia: o 1º prompt (única coisa que o desperta) é enfileirado DEPOIS do seed, na MESMA função. `root/hops` vêm de `derive_root_hops` (`router.rs:1854`), que **ignora `msg.hops`/`msg.root_cause_id`**. |
| **M3** | Dedupe DURÁVEL do spawn (sem dupla-admissão pós-crash) | `bridge.rs:4974-4977` (`execute_spawn` consulta `admitted_node_for_spawn` ANTES de admitir) + `5001-5015` (varre `SpawnAdmitted{id}` no LOG — durável) + apêndice `4988-4990` | ✅ **RESOLVIDO.** Re-drain pós-crash (com `seen` em-memória perdido e o `delivery_ledger` não indexando spawn) re-roda `handle_spawn` → 2º `SpawnApproved`, mas `execute_spawn` acha o `SpawnAdmitted` no log e devolve o nó existente — **sem 2º terminal**. Janela residual (admite mas falha o append do `SpawnAdmitted`) é LOGADA "CRÍTICO" (`4994`), visível, não silenciosa. |
| **M2** | Autonomia REAL fiada (manual bloqueia spawn/delegação DE FATO em produção) | `runtime.rs:588` (passa `autonomy` real à `MailboxPump::new`) + `716` (`autonomy: autonomy_to_level(autonomy)` no `RouterConfig`, mata o `..default()→Assisted`) + `handle_spawn` `router.rs:1875` (`self.config.autonomy.blocks_delegation()→SpawnBlocked`) | ✅ **RESOLVIDO no código.** O ramo manual deixou de ser fake. (Prova-em-tela "manual bloqueia de fato" segue como item de gate-do-fundador, não furo de código.) |

### Multi-workspace / identidade A2A (axis 1)

| Invariante | Linha que garante |
|---|---|
| Isolamento por objeto distinto — 1 Supervisor/store/mailbox/broker POR Espaço; bus nunca cruza in-process | `runtime.rs:54-85` (`WsRuntime` campos próprios) · `467-481` (`NodeManager` por runtime) · `574-604` (Mailbox/Broker pump por `mailbox_dir`) |
| `LINA_HOME` é POR SPAWN — setter global REMOVIDO (senão, com N runtimes, spawns de A nasceriam com `.lina` de B) | `runtime.rs:318-322` (comentário+ausência do `set_var`) · `bridge.rs:2637` (`.env("LINA_HOME", lina_home)`) |
| Origem A2A é o subdir-dono, JAMAIS o `from` do JSON; canal flat anonimizado (anti-impersonação) | `mailbox.rs:519,595` (`msg.from = node` = nome do dir-dono) · `505,569` (`msg.from.clear()` no flat → router recusa `UnknownSender`) |
| `lina ask` sem fallback flat (msg com `from=""` SOME) — falha VISÍVEL, não degrada para um caminho descartado | `lina.rs:83-93` (`enqueue_per_node` ESTRITO) |
| Troca viva durável: log do ALVO é a verdade do foco; falha NÃO troca (inv#6) | `runtime.rs:111-158` (`activate_workspace`: `WorkspaceFocusSet` no log do alvo `135-139`; `Err`→Espaço atual segue) |

### Spawn / gate inforjável (axis 3) — re-confirmado no HEAD de saída

| Invariante | Linha |
|---|---|
| Nenhum campo do agente decide o gate — root/hops EFETIVOS do binding | `router.rs:1854` (`derive_root_hops`) + `1851-1853` (ignora `msg.hops/root`) |
| `SpawnRequested` SEMPRE logado ANTES da decisão (livro-razão; intent-vs-action) | `router.rs:1858-1869` |
| Cascata (`hops≥1`) → gate humano SEMPRE, sem caminho que aprove | `router.rs:1884-1891` (antes do ALLOW `1941`) |
| `requested_by` = sender AUTENTICADO, jamais `from` | `router.rs:1860` (`requested_by: sender`) |

### Webhooks / anti-starvation (axis 6)

| Invariante | Linha |
|---|---|
| HMAC OBRIGATÓRIO, ANTES de publicar (gate de publicação); ausência/divergência → 401 sem efeito | `webhooks/lib.rs:553-559` (passo 3, antes do publish no passo 6 `602-604`) |
| HMAC em tempo constante (`subtle`) | `lib.rs:764-772` (`verify_hex` → `mac.verify_slice`) |
| Anti-starvation = balde por-rota com contabilidade IDÊNTICA (sem oráculo de enumeração); NÃO reordena o HMAC | `lib.rs:531-537` (passo 1a pré-auth) — só rate-limit, HMAC segue passo 3 |
| Anti-starvation NÃO introduz fila ilimitada (DoS) — baldes capeados, purga com throttle | `lib.rs:159` (`MAX_ROUTE_BUCKETS=4096`) · `163` (`MAX_ROUTE_KEY_LEN=64`) · `712-722` (cap + purga `PURGE_MIN_INTERVAL`) |
| Budget por-hook SÓ após o HMAC (lixo sem secret nunca consome o budget do dono) | `lib.rs:561-565` (passo 4, após o 3) |
| Durabilidade primeiro: 202 só após o append durável; falha → 5xx, nada publicado | `lib.rs:578-617` (append em `spawn_blocking`, await antes do 202) |

### Guard / custódia / estado global (axis 4)

| Invariante | Linha |
|---|---|
| `GatedHard` (irreversível/externo) → `Ask` em TODOS os níveis — autônomo NUNCA afrouxa | `guard.rs:199-210` (`decide`: `GatedHard => Decision::Ask`) |
| Fragmenta por `;/&&/\|\|/\|` e pega a fração MAIS severa (anti-evasão) | `guard.rs:148-160` (`classify`) |
| `lina do`/`resume`: agente REGISTRA, NÃO executa; humano + segredo do cofre executam | `lina.rs:1258-1370` (broker enqueue; `BrokerDenied{unconfirmed}`) · `runtime.rs:596-604` (BrokerPump valida origem = nó REAL do roster; `run_custody` com o cofre) |
| `LINA_DEV` é toggle de PAINEL de UI — zero referência em `guard.rs`/`router.rs`/`lina.rs` | `persistence_ui.rs:580-610` (auto-abre painel; nada de gate) |
| Suspensão de ociosos NÃO é DoS de peer: sem verbo CLI; A2A dirigida DESPERTA o alvo | `suspend.rs` (sem `lina suspend`; `note_a2a_delivered`→Active) |

### Aprovação y/n + reinject (axis 7) — herdado do red-team F1-1, re-confirmado intacto no delta

| Invariante | Linha |
|---|---|
| Reinject inalcançável por agente (fila EM-PROCESSO, sem dropzone de FS — FIX-A3); texto REGENERADO do papel, nunca payload | `bridge.rs:1059-1079` (`drain_reinject`: alvo = `NodeId` autenticado; `doctrine_reinjection_text(role)`) |
| Deny/timeout NUNCA vira approve; binding do LOG, não do gesto; screen-changed aborta com zero bytes | (verde em `redteam-gate-f1-1.md` §Confirmações; nenhum commit do delta F1-4/5/6 tocou `attention.rs`/`approval.rs` na superfície de decisão) |

### Replay / restore (axis 2)

| Invariante | Linha |
|---|---|
| Boot NÃO panica com log adversarial — linha ilegível/corrompida em quarentena | `events.rs:1324-1338` (`continue`/`warn`, nunca panic) |
| Eventos ADITIVOS — replay de log antigo nunca quebra | `events.rs:183,199,221,238,…` (`#[serde(default)]` em todos os campos novos) |
| `.db` corrompido PRESERVADO (nunca apagado) + reconstrução do JSONL | `events.rs:1294-1340` (`preserve_corrupt` → `lina.db.corrupt-<ts>`) |
| Restore re-admite pelo funil único (identidade FRESCA cunhada no Supervisor) — não re-confia autoridade do log velho | `bridge.rs:3899,3967-3987` (`admit_node` por terminal restaurado) |

---

## Reconciliação com os red-teams anteriores

- **`redteam-spawn-f1-3-6.md`** deixou M1/M2/M3/M4 como *carry-forward para o seam*. **Re-derivei
  os 4 no HEAD de saída:** M2/M3/M4 **resolvidos** (linhas acima); M1 **permanece** como MEDIA
  aceito (poda de 60s; tempo≠campo; sem velocidade de bomb). **Nenhuma das 3 "ALTA dos céticos"
  daquela rodada sobrevive** — eram efemeridade/durabilidade de binding, agora fiadas.
- **`redteam-gate-f1-1.md`** fechou a superfície de aprovação y/n (A3 corrigido `f6db7e3`). Confirmei
  que **o delta F1-4/5/6 não tocou a superfície de decisão** de `attention.rs`/`approval.rs` — os
  invariantes y/n seguem verdes por construção (nenhum novo call-site afrouxou o gate).
- **Lição AUP honrada:** formulei cada achado como invariante violável (não narrativa de ataque) e
  **re-derivei no código** antes de classificar — os agentes `Explore` mapearam; eu assinei a linha.
- **Cross-check dos achados de dogfooding abertos** (`achados-dogfooding-sessao.md`): o **#17**
  (`lina spawn` deriva `from`/autonomia da ficha do CWD em dir compartilhado) é um gap de
  **ATRIBUIÇÃO de identidade**, não de autorização — re-derivei: `run_spawn` usa `load_identity()`
  (`lina.rs:789`), que aplica o override de env do ADR 0026 (env vence a ficha), e o **router
  re-carimba `from` pelo subdir-dono no drain** (`mailbox.rs:519,595`), de modo que o campo do bin
  **não decide o gate** (cascata/cap/manual computam da identidade autenticada). Logo o #17 degrada a
  *qualidade da atribuição* quando `LINA_NODE_NAME` falta, mas **não abre bypass** — consistente com
  o 0 ALTA. (Os #12/#13/#16 do dogfooding são bugs de **entrega/disponibilidade** do A2A — MEDIA de
  durabilidade, fora do eixo de autorização deste gate; já rastreados no arquivo de dogfooding.)

## Notas de honestidade / estado da árvore
- **READ-ONLY:** só escrevi este relatório, a entrega-resumo e o marcador `.iniciado-redteam-saida`.
  Meus subagentes foram `Explore` (sem Edit/Write) e não tocaram o repo.
- **Cobertura honesta:** os 7 eixos do despacho foram varridos com evidência por eixo. **Não**
  rodei a suíte de segurança do router (o despacho é READ-ONLY de produção; a suíte é do
  `redteam-gate-f1-1`/CI 3-SO, item separado do §4). **Não** executei o app (red-team de invariantes
  no código, não E2E de tela — esse é o "roteiro de tela do fundador" do §4).
- **Limite do modelo de ameaça:** as conclusões assumem o **modelo local-first declarado** (agentes
  = processos do mesmo usuário de SO que o usuário lançou; a autoridade vem do app/supervisor, não
  de cripto inter-agente). MM1/MM3 são exatamente a fronteira desse modelo — registrados, não
  inflados.

## Conclusão

**0 ALTA.** O épico F1, no HEAD de saída `f1d4810`, **passa o gate adversarial final**: a doutrina
de segurança (nenhum campo do agente decide identidade/ordem/autorização; ação irreversível exige
gate humano) **vale em arquivo:linha** em todos os eixos do delta, e as três dívidas que o seam
herdou do red-team do spawn (**M2/M3/M4**) foram **honradas e re-derivadas verdes**. Os 3 MEDIA
restantes (MM1 doc-vs-código da isolação FS multi-workspace; MM2 API cross-history correta mas
não-fiada; MM3 integridade do log no modelo local-first) **não bloqueiam a saída** — vão ao backlog
com dono, conforme o critério do fundador (**0 ALTA para sair; MEDIA→backlog**).

PRONTO: 0 ALTA — gate adversarial de saída ATENDIDO; 3 MEDIA→backlog com dono (MM1/MM2/MM3); M2/M3/M4 do seam re-derivados RESOLVIDOS.

---

## ADENDO — passada focada nos crates de LICENÇA/KEYGEN (commit `cb3180e`, F1-4-5/F1-4-7)

> Pedido pelo @Terminal A após o HEAD avançar durante o sweep. Crates: `crates/lina-license/src/
> {claims,token,state}.rs` + `crates/lina-keygen/src/{lib,main}.rs` (ed25519, gating free/PRO,
> ~1880 linhas). Eixos: (a) degradação-para-free contornável sem assinatura? (b) `canonical_bytes`
> tem ambiguidade forjável? (c) token aceita algoritmo/curva alternativa? (d) privada vaza por
> log/erro? (e) clock-skew abre algo no expiry? **Re-derivado no código, linha a linha.**

### Veredito do adendo
**0 ALTA na cripto** — os 5 eixos pedidos são **VERDES** no crate. **1 MEDIA de costura** (o crate
está correto mas **não-fiado** no app — o gate free=1 é teatro hoje; **já auto-sinalizado** no
código como bloqueante de release) + 2 BAIXA (OPSEC do fundador). O subsistema de licença é
**criptograficamente sólido**; o que falta é o app **consumir** o crate no ponto de gating.

### Achados do adendo

| # | Sev | Invariante | Arquivo:linha | Evidência re-derivada | Dono |
|---|---|---|---|---|---|
| **ML1** | MEDIA | "sem assinatura, o feature-gating é teatro" (`claims.rs:7`) — o gate free=1 deve consultar a licença ASSINADA | `main.rs:863-866,869,886,960` · `app/lina-gpui/Cargo.toml` (sem `lina-license`) | O crate `lina-license` está **CORRETO mas INTEIRAMENTE NÃO-FIADO no app**: não é dependência do `Cargo.toml` do app; **zero** uso de `LicenseState`/`activate`/`parse_token`/`Verifier`/`license.json` em `app/lina-gpui/src`. O gate de criar Espaço chama `lina_core::can_create_workspace(tier, …)` com `tier` **hardcoded `LicenseTier::Pro`** (`main.rs:866,886,960`) — todo usuário é PRO, o limite free=1 **não é enforçado hoje**. **NÃO é furo de cripto** (a pergunta "free contornável sem assinatura" responde-se: o crate é fail-closed; o app é que ainda não pergunta). O PRÓPRIO código sinaliza: *"tier STUB `Pro` até F1-4-5 — bloqueante de release (§6-B1)"* (`main.rs:863`). Fechar = o app carregar `LicenseState::load_default().effective(now).tier` e converter a `LicenseTier`. **Mesmo padrão do MM2** (crate pronto+testado, wiring atrasado). | app / F1-4-5 (fiação do gate à licença assinada) |
| **ML2** | BAIXA | Robustez do CSV do keygen (delimitador em campo livre) | `lina-keygen/src/lib.rs:92-101` | `render_csv` interpola `label` (arg livre do operador) sem quoting; `validate()` barra `,`/`\n` em campos de CLAIMS (tier/entitlements), MAS `label` **não** é claim — `--label "a,b"` ou com `\n` corromperia o CSV do lote. **Não é fronteira de ataque** (keygen é do FUNDADOR, offline; corromperia só o próprio CSV). Registro de robustez. | keygen (sanitizar/recusar delimitador em `label`) |
| **ML3** | BAIXA | OPSEC da chave privada no Windows | `lina-keygen/src/main.rs:177-190` | `restrict_permissions` aplica `0600` só no Unix; no Windows confia no ACL herdado do perfil (documentado, sem equivalente portátil simples). A privada do fundador é a raiz de confiança de TODO o esquema — vazá-la = forjar qualquer licença. Aceitável para ferramenta offline do fundador; registrar em `OPERACAO.md` que o Windows depende do ACL do perfil. | keygen / OPERACAO.md |

### Confirmações positivas da cripto (com a linha que ENFORÇA)

| Eixo pedido | Veredito | Linha que garante |
|---|---|---|
| (a) Degradação-para-free contornável SEM assinatura válida? | ✅ **NÃO** (fail-closed) | `state.rs:111-147` (`load` RE-VERIFICA a assinatura em TODA carga; **todo** caminho de erro — ausente/ilegível/JSON quebrado/base64 inválido/validate falho/sig falha — cai em `claims:None`→FREE; nenhum caminho concede PRO) · `state.rs:137` (`verify_claims` na carga, não confiança-pós-escrita) |
| (b) `canonical_bytes` com ambiguidade forjável (pegar token free e injetar PRO/entitlements)? | ✅ **NÃO** (injetivo) | `claims.rs:90-104` (TODOS os 6 campos assinados, ordem fixa, `Option`→"") + `claims.rs:46-48,53-86` (`validate` PROÍBE `\n`/`\r`/`,` nos campos de texto livre → sem colisão `["a","b"]`≡`["a,b"]`; numéricos viram só dígitos) + `token.rs:115` (`validate` ANTES do verify) |
| (c) Token aceita algoritmo/curva alternativa? | ✅ **NÃO** | `token.rs:84-85` (ed25519 fixo; `verify_strict` rejeita malleabilidade/torsion-S) · sem campo `alg` nos claims (`claims.rs:16-34`) · chave pública **embarcada** `token.rs:22` (o token NÃO carrega chave → sem injeção JWK) · `from_hex` valida on-curve `token.rs:71-72` |
| (d) Privada vaza por log/erro? | ✅ **NÃO** | `main.rs:102-111` (imprime só PATH + chave PÚBLICA, nunca a privada) · `main.rs:95-97` (privada → arquivo `0600`) · `main.rs:155-171` (erro de carga cita o PATH, não o conteúdo) · `lib.rs:8-9,53-87` (a lib recebe `&SigningKey`, nunca persiste/loga) |
| (e) Clock-skew no expiry abre algo? | ✅ **Bounded** (tradeoff offline inerente) | `state.rs:165-198` (`now` INJETADO pelo chamador; expiry só nos pontos de gating; expirado→FREE) · `claims.rs:28` (`expiry:None` perpétua imune a relógio). Rollback de relógio estende a PRÓPRIA licença — limitação inerente de licenciamento offline sem rede (inv#2), **não** um campo forjável. Skew p/ frente = perde acesso pago (disponibilidade), não bypass. |
| Extra: sem master key; cada chave auto-contida; auto-verificada antes de emitir | ✅ | `lib.rs:64-86` (UUIDv7 por chave + `verify_claims` self-check antes do CSV — `SelfCheckFailed` se quebrar) |
| Extra: degrade sem panic se a constante pública corromper | ✅ | `token.rs:57-59,78-80` (`official()`→`key:None`→verify=false→free) |

### Reconciliação do adendo
A pergunta-chave do @Terminal A ("free contornável sem assinatura?") tem resposta em **duas
camadas**, e a honestidade exige separá-las: **no crate**, NÃO — é fail-closed, re-verifica, e a
canonicalização é injetiva (impossível forjar PRO sem a privada do fundador). **No app HOJE**, o
gate **nem pergunta** à licença (stub `Pro` hardcoded) — então o limite free não é enforçado, mas
isso é **wiring pendente auto-sinalizado como bloqueante de release**, não um furo de cripto. O
crate `cb3180e` entregou a parte difícil (a cripto) **certa**; falta o app consumi-la (ML1).
