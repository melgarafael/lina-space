# ADR 0027 — PF-1 operacional: allow-list e fronteira de confiança POR Espaço (F1-4-2)

- **Status:** **Proposto (DRAFT)** — selo final condicionado ao fechamento da F1-4-1 (modelo de
  Espaço em construção nesta rodada, despacho `r1-dados.md`); os testes dos critérios 2–4 da
  story são do dono da F1-4-2 (dev02), não deste draft.
- **Onda/Story:** F1-4 · F1-4-2 (porta PF-1 do esqueleto do Épico F1)
- **Data:** 2026-06-10
- **Fontes:** ADR 0006 (default-deny por pertencimento) · ADR 0007 (origem vs cascata; `hops`
  não-forjável) · **ADR 0010** (decisão macro: namespace por Espaço, cross-workspace deny-all —
  este ADR a OPERACIONALIZA, não a reabre) · ADR 0004 (gate humano/custódia como backstop) ·
  `tasks/epico-f1/ondas-2-4.md` linhas 183-199 (story F1-4-2) · `tasks/despachos/r1-dados.md`
  (decisões do Maestro p/ F1-4-1: um-store-por-Espaço como inclinação; registry = ponteiro;
  pré-aviso do bug de identidade `LINA_NODE_ID`).

## Contexto

O ADR 0010 já decidiu o **quê**: namespace `workspace_id` no Supervisor/roster, trust derivado
POR Espaço, **default-deny cross-workspace sem exceções na F1**. A F1-4-1 (em execução paralela)
constrói o modelo de Espaço. Falta o **como** operacional — e é isso que a porta PF-1 exige
registrado ANTES de o multi-workspace abrir a superfície nova:

1. O que exatamente é "trust de um Espaço" e o que sobrevive a fechar/reabrir o app;
2. Como a allow-list de ações sensíveis de W4-3 ("confirmação humana para A2A sensível")
   escala de por-nó para por-Espaço sem reabrir o buraco que o ADR 0006 fechou;
3. Como `WorkspaceTrust` (single-workspace hoje) migra para N Espaços;
4. O red-team do modelo, em linguagem de invariantes.

**Distinção que este ADR torna canônica — dois "trusts" que NÃO se misturam:**

| Conceito | Natureza | Autoridade | Persistência |
|---|---|---|---|
| **`WorkspaceTrust`** (ADR 0006) — matriz de pares `(from, target)` de injeção A2A | **Efêmera**: re-derivada do roster VIVO a cada entrega | Supervisor (topologia viva) | NUNCA persistida (persistir = dessincronizar do vivo) |
| **Estado de confiança do Espaço** (`trusted`/`untrusted`) — "este Espaço pode rodar ações sensíveis sem re-perguntar?" | **Durável**: fato no event log do Espaço | Gate HUMANO no app (clique, nunca agente) | Herdada ao reabrir, via replay do log |

## Decisão

### (a) Trust do Espaço: herdado ao reabrir, NUNCA derivado de campo de agente

- Todo Espaço **nasce `untrusted`**. A promoção a `trusted` é **decisão humana explícita no app**
  (gesto de UI), registrada como evento no log do próprio Espaço (par proposto
  `WorkspaceMarkedTrusted`/`WorkspaceMarkedUntrusted` — nomes finais com o dono de `events.rs`;
  **deliberadamente SEM o prefixo `WorkspaceTrust`**, que é o identificador da struct EFÊMERA de
  `a2a.rs` — um grep por `WorkspaceTrust` não pode misturar os dois conceitos que este ADR manda
  separar; eventos **aditivos**, `serde(default)`, padrão ADR 0001). Reabrir o Espaço **herda** o
  estado pelo replay — o usuário não re-confia a cada manhã (invariante #6), e a herança vem do
  LOG, não de flag solta em config.
- **Nenhum campo escrito por agente** (`from`, payload, contrato, filename, nome de nó, env de
  PTY filho, cwd) participa da derivação desse estado — autoridade é o app/supervisor (doutrina
  da Fase 0; reforçada pelo pré-aviso `LINA_NODE_ID` do `r1-dados.md`: identidade de terminal
  hoje colapsa sob cwd compartilhado, então **cwd jamais pode ser componente de identidade ou de
  trust**).
- Demover (`Revoked`) é sempre possível e tem efeito imediato: a allow-list do Espaço volta ao
  default restritivo (abaixo).

### (b) Allow-list de ações sensíveis POR Espaço (evolui W4-3 sem reabrir ADR 0006)

- A allow-list de W4-3 (por nó, "o que exige confirmação humana") passa a ser **escopada pelo
  Espaço**: o documento de política vive como **projeção do log do Espaço** (eventos de
  aprovação humana → projeção re-derivável; **não** tabela-autoridade paralela, invariante #4).
- Conteúdo: ações sensíveis (ações irreversíveis do ADR 0004, broadcast amplo conforme ADR 0007,
  spawn/kill de terminal, exposições de rede) e as exceções que o humano aprovou **para aquele
  Espaço**. Espaço `untrusted` usa o default mais restritivo; `trusted` aplica as exceções
  acumuladas.
- **Não reabre o ADR 0006:** a matriz de pares de injeção continua efêmera e derivada do roster
  vivo — a allow-list por Espaço decide *o que precisa de humano*, nunca *quem pode falar com
  quem* (isso segue sendo pertencimento ao Espaço, por construção).
- Allow-list de um Espaço **não tem efeito** em nenhum outro Espaço (escopo é o store do Espaço;
  alinha com a inclinação um-store-por-Espaço do `r1-dados.md` — isolamento físico = isolamento
  de falha E de política). **Independência de layout:** a decisão de storage da F1-4-1 está
  ABERTA (a inclinação (b) pode perder para o log único particionado por `workspace_id`); se a
  F1-4-1 fechar no layout (a), leia "store do Espaço" neste ADR como **partição/namespace do
  Espaço** — as REGRAS (escopo da allow-list; negação no log do remetente) não mudam.

### (c) Cross-Espaço: deny-all por default; exceção futura = opt-in explícito auditável

- Reafirma o ADR 0010: par cross-Espaço **não é gerado** ⇒ negado **por construção** (não por
  checagem que pode ser esquecida). Nenhum canal cross-Espaço existe na F1.
- Exceção futura (ponte entre Espaços) exige: ADR próprio com caso de uso + opt-in explícito do
  humano **nos DOIS Espaços** + evento auditável em ambos os logs + a ponte nasce revogável.
- **Negação auditável** (critério 4 da story): a tentativa negada vira evento no log do Espaço
  do **remetente**, escrito pelo Router **no store que o processo já escreve** — nunca abrindo
  segunda conexão/escritor num store de outro Espaço (lição W5: nada de múltiplos escritores no
  mesmo log sem prova de append concorrente). Nota de precisão: o Router NÃO é o único
  componente que apenda no store hoje (bridge e engine de webhooks também apendam, pela MESMA
  conexão compartilhada) — a propriedade real é *uma conexão por store por processo*, não
  "Router como escritor exclusivo"; testes da F1-4-2 não devem assumir a segunda.

### (d) Migração `WorkspaceTrust` single → multi

- Hoje: `WorkspaceTrust::from_members(&live_member_ids(&sup))` — roster **global** (seguro com
  1 Espaço; é exatamente a PF-1).
- Alvo: `WorkspaceTrust::from_members(&sup.live_members(workspace_id))` — os call-sites de
  política derivam a matriz **do roster escopado** pelo Espaço do nó remetente. O tipo
  `WorkspaceTrust` em si **não muda** (continua matriz de pares efêmera); muda a FONTE dos
  membros. Re-derivação a cada entrega preservada (ADR 0006).
- Sequência segura: (1) F1-4-1 introduz `workspace_id` no roster; (2) call-sites trocam
  `live_member_ids` → `live_members(ws)`; (3) red-team cross-workspace (ADR 0010 §5) roda como
  gate. Enquanto (1) não fecha, o comportamento atual é o degenerado correto (1 Espaço = roster
  global ≡ roster do Espaço).
- `hops`/anti-cascata (ADR 0007) é **ortogonal e por Espaço**: o binding `delivered_root` já é
  interno ao supervisor; com namespace, cadeia causal nunca atravessa Espaços (não há entrega
  cross-Espaço para criar binding).

### (e) Red-team do modelo — em linguagem de INVARIANTES

| # | Invariante (o que NUNCA pode acontecer) | Ataque que ele mata |
|---|---|---|
| INV-1 | Nenhum campo escrito por agente (from/payload/contrato/filename/env/cwd/nome) participa da derivação de trust ou allow-list de Espaço algum. Autoridade = app/supervisor. | Agente "promove" o próprio Espaço via mensagem/arquivo forjado. |
| INV-2 | A matriz de injeção só contém pares do MESMO Espaço; par cross-Espaço **não é representável** na estrutura. | Injeção A→B entre Espaços "esquecida" por um call-site — não há o que esquecer: o par não existe. |
| INV-3 | Evento forjado/copiado no log de B referenciando nós de A não muda autorização: replay de B só materializa nós no namespace de B; IDs alheios não resolvem para membros vivos de B → par não gerado. | **Impersonação de Espaço via log** (colar eventos de A no log de B). O log é fonte de FATOS do próprio Espaço, não de autoridade alheia. |
| INV-4 | Mudança de trust de Espaço só nasce de gesto humano NO app (UI), nunca de A2A, webhook ou conteúdo de terminal; o evento resultante referencia a aprovação (padrão ADR 0021/0024). | Prompt-injection que convence um agente a "confiar" o Espaço. |
| INV-5 | Toda negação cross-Espaço é auditável no log do Espaço remetente (uma conexão por store por processo). | Sondagem silenciosa da fronteira (tentativas negadas sem rastro). |
| INV-6 | Identidade de nó nunca deriva de cwd ou de qualquer recurso compartilhável entre nós (bug `LINA_NODE_ID` em fix nesta rodada). | Dois nós no mesmo cwd colapsarem em uma identidade ⇒ herança indevida de pertencimento/trust. |
| INV-7 | A integridade do **trust durável** assume a integridade do ARQUIVO do store — e o ADR registra isso como fronteira, não finge resolver. A matriz de **injeção** nunca deriva do log (deriva do roster vivo do Supervisor, ADR 0006), justamente para que tamper no arquivo não compre injeção. | **Forja de evento de trust no PRÓPRIO log** (processo/agente de mesmo uid com shell apenda `WorkspaceMarkedTrusted` direto no arquivo → Espaço escala a `trusted` persistente). Mitigação hoje: backstop de gate humano + custódia (ADR 0004) segue na frente de toda ação irreversível MESMO em Espaço trusted; fechamento real é a fronteira L1-3/integridade do store (F1-6/Fase 2). |

**Limite explícito (herdado E ampliado — não escondido):** L1-3 segue aberta (ADR 0006 §Limite;
ADR 0010 §Limite) — namespace por Espaço **não autentica peer real** de mesmo uid fora do app.
**Este ADR AMPLIA a superfície dessa fronteira:** o estado durável de confiança introduz um alvo
novo de tamper-no-filesystem (INV-7) que a matriz efêmera não tinha (ela vive em memória,
derivada do roster vivo — não-forjável via arquivo). Quem fechar L1-3 (F1-6/Fase 2) deve cobrir
**integridade do arquivo do store** junto. Backstop até lá: gate humano + custódia (ADR 0004)
não é relaxado por Espaço `trusted` para ações irreversíveis.

## Alternativas rejeitadas

- **Trust único da máquina** (1 flag global): um Espaço de teste confiável daria carona a um
  Espaço hostil; contraria "pertencimento = conexão POR Espaço" (inv #5, ADR 0010).
- **Persistir a matriz de pares de injeção**: dessincroniza do roster vivo; o ADR 0006 já
  rejeitou allow-list estática (TOML) pelo mesmo motivo.
- **Allow-list global com coluna `workspace_id`** (tabela-autoridade fora do log): viola o
  invariante #4 (projeção sem fato no log) e cria escritor concorrente; a política é projeção
  do log do próprio Espaço.
- **Derivar trust de heurística** (ex.: "mesmo cwd ⇒ mesmo time ⇒ confia"): morta pelo INV-6 —
  o bug de identidade por cwd compartilhado prova que o sinal é forjável na prática.
- **Permitir exceção cross-Espaço já na F1** ("só leitura", "só status"): reabre o buraco que o
  ADR 0010 fechou sem caso de uso; qualquer ponte futura exige ADR próprio.

## Evidências pendentes para o selo (saem do DRAFT com a F1-4-1 fechada)

1. Teste: Espaço novo nasce `untrusted`; só gesto humano promove; evento auditável no replay.
2. Teste adversarial: nenhum campo controlável por agente altera trust/allow-list (INV-1).
3. Teste: A2A cross-Espaço negada por default + negação auditável no log do remetente (INV-5).
4. Red-team cross-workspace do ADR 0010 §5 (forja A→B; nó migrando; logs mesclados) verde.
