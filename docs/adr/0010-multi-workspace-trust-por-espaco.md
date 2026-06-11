# ADR 0010 — Multi-workspace: trust com namespace por Espaço (fecha PF-1) + escopo canônico da Fase 1

- **Status:** Aceito (escopo: decisão do FUNDADOR, 2026-06-06; desenho técnico: arquiteto/Maestro, mesma data)
- **Onda/Story:** F1-4 (Workspaces e PRO) — **bloqueante para toda a onda exceto a licença (ADR 0011)**
- **Data:** 2026-06-06
- **Fontes:** ADR 0006 · `_HANDOFF` §7-C item 7 (PF-1/PF-2) · doc vivo (item 2: workspaces multi-tenant) · `30 - SPEC Mestre` §4/§8 · `01 - Visao de Produto` §2 · `tasks/epico-f1/arquitetura.md` §b/§c.1

## Contexto

O ADR 0006 tornou a injeção A2A default-deny derivando `WorkspaceTrust` de
`live_member_ids(&sup)` — o roster **GLOBAL** do Supervisor, **sem namespace de Espaço**.
A porta-futura **PF-1** registrou: "seguro com 1 Espaço/app, **perigoso com N Espaços**"
(handoff §7-C). A F1-4 traz exatamente N Espaços (workspaces multi-tenant, free=1/PRO=N).
Sem namespace, um terminal do Espaço A injetaria num terminal do Espaço B **com a allow-list
dizendo sim** — quebra do invariante #5 ("pertencimento = conexão" é pertencimento *a um
Espaço*, não ao processo).

Este ADR também registra a **decisão de escopo da Fase 1** tomada pelo fundador em
2026-06-06, porque é ela que faz o multi-workspace entrar agora — e porque o roadmap antigo
precisa de um lugar canônico para não se perder.

## Decisão (técnica)

1. **Namespace `workspace_id` no Supervisor/roster.** Todo nó pertence a exatamente um
   Espaço. `live_members(workspace_id)` substitui o roster global nos call-sites de política.
2. **`WorkspaceTrust::from_members(live_members(ws))` por Espaço**, re-derivada **a cada
   entrega** (roster muda em runtime — mesmo princípio do 0006). Par cross-Espaço **não é
   gerado** ⇒ negado por construção. **Default-deny cross-workspace, sem exceções na F1** —
   uma eventual ponte cross-workspace futura exige ADR próprio com caso de uso.
3. **Eventos com `workspace_id` aditivo.** Eventos de roster/lifecycle ganham o campo via
   `#[serde(default)]` (legado = workspace único default) — replay antigo intacto
   (invariante #4; padrão ADR 0001).
4. **PF-2 alinhada:** payload de webhook permanece não-confiável mesmo quando o `from`
   resolver para membro; backstop gate/custódia (ADR 0004) intacto; o fix do teto-IP
   (MEDIA-1) considera o cenário multi-workspace.
5. **Critério de aceite da onda F1-4:** red-team cross-workspace (forjar entrega A→B entre
   Espaços distintos; nó migrando de Espaço; replay de log com workspaces mesclados).

## Decisão (escopo da Fase 1 — fundador, 2026-06-06)

**Fase 1 = esqueleto F1-0..F1-6** (Coordenação Confiável · Observabilidade e Cognição ·
Experiência · Inteligência da Lina · Workspaces e PRO · Render-Scale e Scrollback ·
Hardening e Saída). Esta direção **supersede o roadmap do doc `01` §2 / SPEC §8** para a
Fase 1. Para nada se perder, os itens do roadmap antigo não cobertos pelo esqueleto viram
**BACKLOG EXPLÍCITO DA FASE 2**, nominalmente:

1. **Engine de Webhooks** + nó-Gatilho + cloudflared 1-clique (SPEC §4 #24)
2. **Discovery ampla / Arsenal de Poderes** (trait `DiscoveryProvider`; SPEC §4 #25)
3. **Curador + feed de novidades** + perfil rico + P3 Radar (SPEC §4 #26)
4. **6 presets completos** (SPEC §4 #27)
5. **Ghost wires + Linha do Tempo** (SPEC §4 #30)
6. **Vault Obsidian** (injeção via env vars + bootstrap; SPEC §4 #23)
7. **Agendador por SO + tiers multi-CLI** (binário `lina` robusto; SPEC §4 #29)

A atualização da tabela do doc `01` §2 (norte) é ação pendente do dono do norte — este ADR é
a fonte canônica até lá.

## Limite explícito

- Namespace por Espaço **não autentica peer real**: L1-3 (auth por-nó não é fronteira de SO)
  segue conscientemente aberta (ADR 0006 §Limite; item de fronteira da F1-6/Fase 2).
- A **licença** (ADR 0011) gateia o *número* de workspaces (free=1, PRO=N) — nunca a
  segurança: as regras deste ADR valem identicamente para free e PRO.

## Alternativas rejeitadas

- **Supervisor único sem namespace (status quo)** — PF-1 vira vulnerabilidade ativa no
  momento em que o 2º Espaço existir.
- **Um Supervisor por workspace** — duplica broker/log/fiação e multiplica superfície de
  bugs de coordenação; o namespace alcança o mesmo isolamento lógico com 1 broker.
- **Um processo por workspace** — contraria a stack decidida (processo único Rust, doc `31`).
- **Allow-list cross-workspace configurável já na F1** — reabre exatamente o buraco que
  este ADR fecha, sem caso de uso que o justifique.

---

## Addendum (2026-06-10) — storage da F1-4-1 supersede o §3 (campo `workspace_id` por-evento)

Pedido pelo dono da F1-4-1 (Especialista em Dados) ao fechar a story em `08bbb92`; registrado
pelo Arquiteto na rodada 2. O §3 acima previa "eventos com `workspace_id` aditivo". A F1-4-1
decidiu e implementou **(b) um event store POR Espaço** (`<root>/.lina/events`; mini-ADR de
storage na entrega da story e na mensagem do commit):

1. **O pertencimento de um evento a um Espaço é dado pelo STORE em que ele vive** (isolamento
   físico), e não por campo `workspace_id` por-evento. O campo do §3 fica **superseded
   mecanicamente**: não é introduzido nos eventos; replay de logs antigos segue intacto
   (nenhum log legado carregava o campo — não há upcast a fazer).
2. **§1 e §2 PERMANECEM válidos e pendentes de story:** o namespace `workspace_id` no
   Supervisor/roster (o broker é um por processo — o namespace LÓGICO continua necessário
   mesmo com stores físicos separados), a `WorkspaceTrust` por Espaço re-derivada a cada
   entrega e o **default-deny cross-workspace** seguem o desenho aceito deste ADR
   (operacionalizado no ADR 0027, selado na mesma data).
3. **Autoridade sobre o ESTADO de cada Espaço = o log do próprio Espaço.** Os fatos (id, nome,
   path, arquivado) são re-deriváveis do store do Espaço; o registry global
   (`~/.lina/workspaces.json`) é **ponteiro de boot re-derivável** (merge-por-id + fsync +
   `open_verified` anti-adulteração — F1-4-1), **nunca autoridade** (invariante #4; mesmo
   princípio "registry é ponteiro" do `r1-dados.md`/ADR 0027). Precisão honesta: a ESCOLHA de
   qual Espaço abre primeiro (`last_focus`) é **conveniência exclusiva do ponteiro** ("nasce
   0", `workspace.rs`) — não há evento de foco no log hoje; perder o registry perde no máximo
   "qual abre primeiro", nunca um fato de Espaço. Se o foco um dia precisar ser fato auditável,
   é evento aditivo novo (story própria), não promoção do registry a autoridade.
