# ADR 0021 — Modelo de segurança da aprovação de permissão (injeção y/n validada contra a tela)

- **Status:** **Aceito** (2026-06-07, Maestro/Orquestrador + fundador — exigência da F1-1-8 cumprida: aceito **ANTES** de qualquer código de injeção; a ordem é auditável por timestamp de commit)
- **Onda/Story:** F1-1 · F1-1-8 (governa a cadeia F1-1-6 detecção → F1-1-7 fila/toast → F1-1-8 injeção)
- **Data:** 2026-06-07
- **Fontes:** pesquisa `13.13` (achados 4/5/6/8; §ressalvas) · CVE-2024-27936 (ANSI spoofing) · CVE-2024-32477 (tcflush race) · GHSA-95cj-3hr2-7j5j (Deno) · ADRs 0004/0006/0007/0009/0019/0020 · `tasks/epico-f1/arquitetura.md` §c/§c.1 · `crates/lina-vt` (trait `VtBackend`) · `crates/lina-cli-profiles` (`Delivery`, `submit_delay_ms`)

## Contexto

O ADR 0009 (aceito 2026-06-06) fixou o modelo de alto nível — snapshot-hash do VT + stable ID +
idempotência + SLA — e impôs um **gate de processo**: a story de injeção só inicia após revisão de
segurança própria deste desenho. Este ADR cumpre esse gate: é a **especificação operacional** do
mecanismo, com o desenho do red-team embutido como critério de verificação. A F1-1-8 exige que ele
esteja aceito antes do primeiro commit de código de injeção.

O risco é real e documentado por CVE (não hipotético): o emulador envia replies ANSI pelo mesmo
canal do stdout, então input pendente pode combinar-se com um write remoto (GHSA-95cj-3hr2-7j5j,
CVE-2024-32477); o prompt pode **mudar** entre a notificação e o clique (aprovação do prompt
errado); e spoofing de prompt via conteúdo impresso é ataque conhecido (CVE-2024-27936). Ponto
epistemológico herdado do 0009/13.13: **não há padrão de indústria documentado** para a mitigação —
a técnica abaixo é decisão de engenharia do Lina, provada por teste de race próprio, não importada
como "padrão de mercado".

## Relação com o ADR 0009 — herda, refina, supersede (arbitragem do arquiteto)

- **Herda intacto:** snapshot como pré-condição dura do write; fila unificada com precedência
  custódia > permissão > custom; timeout **nega, nunca aprova**; aprovação não substitui a custódia
  (ADR 0004); same-uid não é fronteira de SO (L1-3, ADR 0006); fila serial por terminal (W0-9).
- **Refina:** a técnica exata do snapshot (região, normalização, atomicidade — §1); onde a
  deduplicação mora (§2); o SLA com números, escalação e semântica do auto-deny (§3); o mapa de
  riscos como invariantes (§4); a construção do binding (§5); a fronteira com a F1-1-7 (§6).
- **Supersede (dois pontos nomeados):**
  1. **Nomenclatura de evento:** `PermissionRequested` (0009 §5) → **`PermissionAsked`** — a
     F1-1-6 já define `PermissionAsked{node_id, tool?, evidence, stable_id}` e é a story que o
     implementa; manter dois nomes para o mesmo fato criaria divergência de contrato.
  2. **Forma da chave:** `idempotency_key: ULID` (0009 §2) → **`stable_id` determinístico**,
     derivado de `(session_id, tool_call, ts_detecção)` (padrão OpenCode, 13.13 achado 4). Um ULID
     aleatório **duplicaria pedidos no replay do log** (cada replay cunharia chave nova), violando
     o AC 4 da F1-1-6 ("replay do log não duplica pedidos"). A derivação determinística torna a
     idempotência propriedade do **dado**, coerente com o invariante #4.

## Decisão

### 1. Validação de tela pré-write: snapshot do grid via `VtBackend` decide; disciplina de escrita reduz a janela (híbrido com hierarquia explícita)

- **Técnica:** comparação de **snapshot da região do prompt**, lida do grid **parseado** pela trait
  `VtBackend` (`row_text()`/`screen()` — células pós-emulador, texto puro sem escapes), **nunca**
  de bytes crus do PTY. O hash (SHA-256) cobre: texto das últimas `K` linhas não-vazias do viewport
  (default `K = 8`, tunável), as dimensões `(cols, rows)` e a posição do cursor. **Atributos de
  cor/estilo ficam FORA do hash** — re-render e troca de tema não mudam a semântica do prompt.
- **Captura 1** na detecção: o detector da F1-1-6 anexa `vt_snapshot_hash` ao pedido.
  **Captura 2** imediatamente antes do write, **executada pelo dono único do PTY** (o loop do
  pty-host que chama `advance`, em `lina-core`): re-snapshot, comparação e write acontecem **no
  mesmo turno do loop**, sem `advance` intercalado — atomicidade local entre o check e o write.
- **Divergência ⇒ NÃO escreve.** Evento `ApprovalAborted{stable_id, reason:"screen_changed"}`;
  a UI reapresenta o estado atual do terminal e pede **novo gesto** do humano. Abort espúrio
  (spinner, relógio no prompt) é custo aceito — a direção do erro é fail-safe; `K`/região são
  hipóteses calibráveis no red-team (família 0019/0020), nunca afrouxadas sem red-team próprio.
- **Por que não "só melhoria de flush" (estilo Deno 1.42.2):** a correção do Deno endereça input
  pendente no stdin **local e imediato**; o nosso problema dominante é a **janela humana** de
  segundos/minutos entre a notificação e o clique — flush nenhum cobre o prompt que mudou nesse
  intervalo (0009 já rejeitou; mantido). **Por que não "só snapshot":** a disciplina de escrita
  ainda reduz a janela residual. Hierarquia: **o snapshot DECIDE; a disciplina REDUZ.**
- **Conteúdo do write — mínimo declarado, nunca dado de agente:** o write é exclusivamente a
  sequência declarada pelo CLI Profile (campo TOML **aditivo** `approval_keys`, ex.: `"y\r"`,
  `"\r"`, `ESC` para recusa; default conservador `"y\r"`/`"n\r"`), **sem bracketed-paste** —
  prompts y/n leem tecla crua; o faseamento bracketed-paste → `submit_delay` → Enter é protocolo
  de **mensagem A2A**, não de aprovação. Nenhum byte do pedido, do payload, do grid ou de qualquer
  campo escrito por agente entra no write — **por construção** não há o que sanitizar.

### 2. Idempotência: a deduplicação mora no executor de injeção (porta única de escrita), guiada por projeção do log

- **Chave:** o `stable_id` da F1-1-6 (determinístico — ver §Supersede). **Ledger:** projeção dos
  eventos `PermissionAsked` / `PermissionResolved` / `ApprovalInjected` (padrão ADR 0014/0020:
  o ledger é derivado do próprio event log, não tabela-autoridade paralela).
- **Regra:** antes de escrever, o executor consulta a projeção; `stable_id` já resolvido ⇒
  **no-op auditado** — aprovar 2× injeta exatamente 1×. A duplicata emite
  `ApprovalDuplicateIgnored{stable_id}` **no máximo 1× por pedido** (anti-amplificação, ADR 0003).
- **Por que não na UI:** o gesto tem múltiplas vias (toast, fila, Cmd+Enter) e o estado de toast
  não sobrevive a crash. **Por que não só em RAM:** replay/restart perderia o histórico. A dedup
  mora onde o write acontece — a única porta — e deriva do log (invariante #4).
- **Recovery NUNCA re-injeta** (postura A6/D1 do ADR 0020: re-injetar num PTY é irreversível;
  perder é recuperável pelo humano). Regra dura: **todo write exige gesto humano fresco no
  processo vivo + snapshot validado AGORA.** Crash entre `PermissionResolved` e o write ⇒ no
  restart nada é escrito; se o prompt persistir, a re-detecção cunha **novo** `stable_id` e o
  humano re-aprova (um clique — barato). Crash entre o write e o append de `ApprovalInjected` ⇒
  nada re-escreve, pela mesma regra.
- **Ordem dos eventos:** `PermissionResolved{stable_id, decision, via: human|timeout}` (a decisão)
  → validação de tela + write → `ApprovalInjected{stable_id, vt_snapshot_hash}` (o efeito).
  Aprovado-mas-abortado fica auditável como `Resolved` + `ApprovalAborted`, **sem** `Injected`.

### 3. SLA de pendência: escalação aos 5 min; auto-deny aos 10 min (nunca auto-approve, sem knob)

- **Escada:** toast ~6s com countdown (F1-1-7) → badge + som opcional 1×/30s enquanto a fila > 0
  (F1-1-7, 13.13 achado 9) → **5 min sem resposta ⇒ escalação forte** (badge pulsante + entrada
  persistente "Terminal X espera você há 5 min") → **10 min ⇒ auto-deny**:
  `PermissionResolved{decision:"deny", via:"timeout"}` + write da recusa (`approval_keys` de
  recusa) pelo **mesmo pipeline validado do §1** — simetria total: se a tela divergiu, aborta sem
  escrever (`ApprovalAborted`) e a pendência encerra como *deny-não-entregue*, com rótulo honesto
  na UI.
- **Por que auto-deny e não esperar para sempre:** terminal bloqueado eternamente é exatamente o
  estado-fantasma que esta onda existe para eliminar (13.13 §Panorama); pendências mortas
  acumulando viram fadiga de toast e enterram pedidos novos.
- **Por que N = 10 min:** alinhado à **calibração com dados reais** do ADR 0020 (turnos reais de
  200–600 s; `retention_timeout` = 10 min) e coerente com a escada do ADR 0019 (warn ~6 min,
  breaker ~12 min) — mesma ordem de grandeza da atenção humana num workspace vivo. Constante em
  `RouterConfig` (tunável); **hipótese calibrável** contra uso real, como os thresholds do 0019.
- **Justificativa para o usuário leigo (vai na microcopy):** *"um 'não' nunca executa nada — no
  pior caso o agente pergunta de novo; um 'sim' automático poderia executar algo que você nunca
  viu. Se você demorar, o Lina responde 'não' por você, avisa, e você re-aprova quando voltar."*
  A assimetria é estrutural: deny é re-emissível; approve é um Enter que nenhum replay
  "des-aperta" (ADR 0020).
- **Não existe knob de auto-approve.** Nem por configuração, nem por nível de autonomia (a
  autonomia nunca afrouxa gate humano de irreversível — ADR 0004 §regra dura). Anti-regressão por
  teste (AC-0021.5).

### 4. Mapa de riscos — invariantes e mitigação (postura defensiva)

| # | Risco | Invariante que o neutraliza | Mecanismo | Resíduo conhecido |
|---|-------|------------------------------|-----------|--------------------|
| R1 | Sequências ANSI maliciosas no conteúdo do terminal (família CVE-2024-27936) | **O grid é DADO renderizado, jamais autoridade.** | O snapshot lê **células pós-parsing** via `VtBackend` (escapes já consumidos pelo emulador; `row_text()` é texto puro); o texto do grid não decide identidade/autorização (§5); o contexto exibido no toast vem do hook `PreToolUse` correlacionado; pedidos com `evidence:"grid"` exibem rótulo "conteúdo não verificado". | Escapes que **alterem o estado visível** (limpar tela, reposicionar cursor) mudam o hash ⇒ abort — o resíduo trabalha **a favor** do fail-safe. |
| R2 | Race de timing entre detecção e write (CVE-2024-32477, GHSA-95cj-3hr2-7j5j) | **Nenhum write sem confirmação de estado no instante do write.** | Check+write atômicos no dono único do PTY (§1); fila serial por terminal (W0-9 — nunca write concorrente no mesmo PTY); write mínimo de `approval_keys`. | Output do filho ainda não lido (buffer do kernel) é inobservável — **janela irredutível**, documentada (0009 §Limite); o teste de race é a evidência da redução, não de eliminação. |
| R3 | Spoofing de prompt — nó que **imprime** um falso "(y/n)" | **Imprimir um prompt não cria autorização — cria no máximo um pedido rotulado.** | O pedido fake entra na fila como `evidence:"grid"`, não-verificado, atribuído ao nó **dono do grid** (o detector roda sobre o `VtBackend` daquele nó — o texto não tem como nomear outro nó). Se o humano aprovar, o write é só `approval_keys` **no próprio terminal do impressor**: blast radius = o próprio nó; não concede segredo (ADR 0004), não cruza PTY, não eleva autonomia. | Humano pode ser socialmente induzido a aprovar um fake plausível; mitigação: rótulo de evidência + nome do nó **sempre** derivado do roster. Taxa de FP do detector é **medida** (protocolo da F1-1-6; #28174). |
| R4 | Confusão de alvo na fila (aprovar o nó errado) | **O gesto referencia `stable_id`, nunca posição.** | Aprovação endereça o `stable_id` (não índice do toast/fila, que reordena sob drain round-robin); binding pedido→nó→tool de fonte interna (§5); **cross-check final no executor**: o `stable_id` deve pertencer ao `node_id` do PTY que receberá o write, senão `ApprovalAborted{reason:"target_mismatch"}`. A UI exibe o nome derivado do roster (`NodeId` do Supervisor — autoridade única), nunca string de payload. | Dois pedidos **reais e simultâneos do mesmo nó** exibem prompts parecidos; desambiguação pelo contexto da tool + ts no toast. |

### 5. Doutrina da casa (inegociável — herda ADRs 0004/0006/0007): como o binding pedido→nó→tool nasce e é verificado

**Regra herdada, sem exceção:** nenhum campo controlável pelo agente (texto do grid, payload de
hook, `from`, filename, `session_id` alegado) decide identidade, ordem ou autorização. A decisão
de aprovar nasce **no humano, via UI autenticada do app** (gesto local — a Fase 1 não tem aprovação
fora da máquina); a identidade vem de fonte interna não-forjável (família `derive_root_hops`/
binding do ADR 0007).

**Construção do binding (fonte interna em cada elo):**

- **`node_id`:** o **canal** por onde a evidência chega É a identidade. O hook é configurado
  por-nó no spawn e entrega num canal que o **Supervisor** associou ao `NodeId` na criação
  (NodeId = autoridade única de identidade); o fallback de grid roda sobre o `VtBackend` **do
  próprio nó** — dono por construção. Nunca extraído de payload.
- **`tool` (contexto):** cache do último `PreToolUse` correlacionado por `session_id` **dentro do
  canal do próprio nó** (correlação interna do supervisor, F1-1-6). O conteúdo do hook
  **contextualiza e exibe; jamais autoriza** — mesmo um `tool_name` forjado no payload não muda o
  que a aprovação concede (ver abaixo).
- **`stable_id`:** derivado **pelo detector** (código nosso) a partir dos campos internos do
  canal — nunca aceito de fora.
- **Verificação no gesto:** a UI resolve `stable_id` → pedido → `node_id` pela **projeção do
  log**; o executor re-verifica `node_id` ↔ PTY alvo antes do write (R4). Qualquer divergência
  aborta com evento.

**O que a aprovação CONCEDE (escopo mínimo, fechado):** exclusivamente **um** write de
`approval_keys` no PTY do nó do pedido, com tela validada. Não concede: segredo (custódia ADR 0004
intacta), injeção A2A (`WorkspaceTrust`/ADR 0006 inalterado), elevação de autonomia, nem aprovação
de pedidos futuros. Por isso um binding "enganado" no campo de **exibição** degrada para UX ruim,
nunca para escalada de capability.

### 6. Fronteira com a F1-1-7 — o que existe sem este ADR vs só com ele

| Sem o ADR aceito (F1-1-7 pode) | Só com o ADR aceito (F1-1-8) |
|---|---|
| Detectar (`PermissionAsked`, F1-1-6); exibir toast/fila/badge/som; enfileirar com precedência custódia > permissão; **auditar** (`PermissionResolved{via:"human"}` com a decisão registrada); recusar/dispensar **sem tocar o PTY** (remove da fila + evento); custódia Cmd+Enter existente intacta (zero regressão — AC 4 da F1-1-7). | **Qualquer write no PTY motivado por permissão** — `y`, `n`, Enter, Esc, qualquer byte. Inclui o write do auto-deny (§3): antes do ADR, o timeout apenas encerra a pendência na fila, **sem escrever**. |

**Ordem auditável (AC 1 da F1-1-8):** este arquivo é commitado e aceito **antes** do primeiro
commit de código de injeção — os timestamps do git são a prova.

## Limite explícito

- A mitigação **reduz a janela da race; não a elimina por prova formal** (herdado do 0009) — o
  teste de race (AC-0021.1) é parte da decisão, não nice-to-have.
- **Same-uid não é fronteira de SO** (L1-3, ADR 0006 §Limite): um processo malicioso na mesma UID
  pode escrever direto no PTY **por fora** do Lina. Fronteira herdada e consciente; o fechamento
  (sandbox por terminal / token-por-spawn) é o item de fronteira #2 da Onda 5 / pesquisa 13.14 —
  fora deste escopo.
- O hash de região (`K` linhas, normalização) é **hipótese calibrável**: prompts com spinner podem
  gerar aborts espúrios. Calibrar no red-team; **nunca** afrouxar para "ignorar mudanças pequenas"
  sem red-team próprio do afrouxamento (manter mecanismo + verificar por bounds — lição do projeto).
- Aprovação **não substitui a custódia** (ADR 0004): ação `gated-hard` externa continua exigindo o
  gate duro de segredo, independente de qualquer y/n aprovado.
- CLIs headless/sem grid ficam fora (recorte da F1-1-6); `Delivery::SessionResume` não tem injeção
  de aprovação — o mecanismo pressupõe PTY vivo com grid observável.

## Alternativas rejeitadas

- **Injetar direto, sem validação de estado** — é a race documentada por CVE (0009; mantido).
- **Só melhoria de flush (estilo Deno 1.42.2)** — resolve o problema do Deno (stdin local,
  imediato), não o nosso (write remoto após janela humana de segundos/minutos) (0009; mantido).
- **Hash da tela inteira com atributos/cores** — abort espúrio a cada re-render/troca de tema; a
  semântica do prompt mora no **texto** da região, não no estilo.
- **Dedup na UI (estado do toast)** — o gesto tem múltiplas vias e o estado de UI não sobrevive a
  crash; a porta única de escrita, guiada pelo log, é o lugar (padrão 0014/0020).
- **ULID aleatório como chave de idempotência (0009 §2)** — duplica pedidos sob replay do log;
  supersedido pela derivação determinística (F1-1-6 AC 4).
- **Auto-approve em timeout, ou knob de auto-approve** — inverte o fail-safe; timeout sempre nega
  (0009; doutrina do gate humano, ADR 0004).
- **Re-injeção automática pós-crash de aprovação pendente** — re-injetar é irreversível; exigir um
  gesto humano fresco custa um clique (postura A6/D1 do ADR 0020).
- **Só focar o terminal, sem injetar** — quebra o valor para o não-técnico (invariante #6); mantido
  apenas como fallback degradado quando o snapshot diverge (0009; mantido).

## Critério de verificação (red-team próprio — evidência exigida antes de "implementado")

- **AC-0021.1 (race — story AC 2):** o prompt do alvo muda entre `PermissionAsked` e o gesto ⇒
  `ApprovalAborted{reason:"screen_changed"}` e **zero bytes** chegam ao PTY (provado pelo log de
  writes do PTY).
- **AC-0021.2 (idempotência — story AC 3):** aprovar 2× o mesmo `stable_id` ⇒ **exatamente 1**
  write; a segunda via é no-op auditado (`ApprovalDuplicateIgnored`, no máximo 1×).
- **AC-0021.3 (spoofing — story AC 4):** nó que imprime fake "(y/n)" + payload malicioso: não
  obtém aprovação automática; o pedido entra rotulado `evidence:"grid"`/não-verificado; aprovação
  manual do fake escreve só `approval_keys` **no próprio nó**; a fila nunca atribui o pedido a
  outro nó.
- **AC-0021.4 (alvo — story AC 4):** com 2+ pendências de nós distintos e reordenação da fila
  entre exibição e gesto, o write vai para o `node_id` do `stable_id` aprovado;
  `target_mismatch` aborta com evento.
- **AC-0021.5 (SLA):** pendência sintética sem resposta: escalação aos 5 min observável
  (estado/evento), `PermissionResolved{via:"timeout", decision:"deny"}` aos 10 min; o write da
  recusa só ocorre com tela válida; **nenhum caminho produz approve por timeout** (busca negativa
  no código + teste).
- **AC-0021.6 (fronteira F1-1-7):** build da F1-1-7 sem o módulo de injeção: **nenhum** caminho de
  código escreve no PTY a partir da fila de atenção (auditoria das chamadas ao writer do PTY).
- **AC-0021.7 (na tela — story AC 5, gate da onda):** aprovar pelo toast destrava um Claude real
  bloqueado em pedido de permissão, com o audit trail completo legível no `log.jsonl`.
