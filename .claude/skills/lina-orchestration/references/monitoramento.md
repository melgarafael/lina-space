# Monitoramento & correção de trajeto — recipe operacional

> A skill `lina-orchestration` consome as definições do **ADR 0019** (`docs/adr/0019-...md`). Ela **não as
> inventa**. Este reference é o "como medir" — puxe ao MONITORAR/CORRIGIR. Tudo é projeção do event log
> (inv#4): você lê o veredito, não o produz por impressão (inv#1).

## 1. As DUAS projeções nomeadas (ADR 0019)

### Projeção A — evento-de-progresso ("o worker realmente avançou?")
**PROGRESSO** numa janela = **(a)** o `tail_hash` do PTY do nó mudou entre amostras consecutivas, **OU**
**(b)** ≥1 `DomainEvent` novo atribuível ao nó (`RouteDelivered` de/para o nó, `TokenUsageReported`,
`PlanClaimed`/`PlanChecked`, `HandoffOpened`/`HandoffClosed`). Qualquer um basta.
- **Uso:** depois de despachar, NÃO pare no "ok" do verbo (o ack só diz que a injeção entrou — lição
  [[lina-a2a-routeblocked-race-e-ask-ok-cego]]). Espere o **PRIMEIRO evento-de-progresso** atribuível ao item
  — é a prova de que o worker pegou e começou. `lina check @X` responde dessa projeção (nunca de view cacheada).
- **Auto-report do agente NÃO conta** como progresso (ADR 0019 / inv#1: campo escrito por agente não decide).

### Projeção B — timeout-de-travamento ("travou, mesmo vivo?")
**TRAVAMENTO** = nó com status **`Busy`** acumulando **≥3 amostras consecutivas sem progresso** (~6 min) →
o sistema emite **`NodeStalled` uma vez (na transição)** → o item entra na fila de atenção como **WARN**.
- **PID vivo ≠ progresso:** um processo vivo que não muda o `tail_hash` nem emite evento está **hung**, não
  trabalhando. É exatamente o freeze que o monitoramento existe para pegar.
- **O relógio de stall SÓ corre em `Busy`.** `Blocked` (esperando permissão/custódia/`y/n`) e `Idle` **não
  acumulam** — um terminal aguardando confirmação humana **não está travado** (falso-positivo nº1 a NÃO repetir).

## 2. Correção de trajeto (story crit. 2)
Detectou (pelas projeções, **não por sorte**) um worker **travado** (Projeção B) ou **desviado** (entregou
fora do spec, ou terminou sem `PRONTO:`/`BLOCKED:` = violação de protocolo)?
1. Não re-despache cego: monte o re-despacho com a seção **"tentativas anteriores"** (`lina-dispatch`) — o
   que foi tentado, por que falhou, o que NÃO repetir.
2. O log deve mostrar a sequência **detecção → correção** (evento de stall/desvio → novo despacho com contexto).

## 3. Os DOIS breakers (não confundir)
| Breaker | Gatilho | Ação |
|---|---|---|
| **Stall** (ADR 0019) | `Busy` sem progresso por ≥6 amostras (~12 min) | **pausa-com-gate** (`lina resume --confirm`), **nunca kill** — preserva o contexto do worker |
| **Falha** (story crit. 3 / Hermes sticky) | mesmo item retorna `BLOCKED`/erro **2× consecutivas** | **STICKY:** sem 3ª tentativa automática → **escalada narrada ao usuário** em pt-br simples |
Ambos param a amplificação. Nenhum mata trabalho. Não "force" uma 3ª tentativa nem dê `resume` sem o gate.

## 4. Validar a entrega "de fora" (handoff §4.2)
O orquestrador **não aceita o auto-relato** do worker ("terminei, tá tudo certo"). Você confere o ARTEFATO:
lê o arquivo, roda o teste/app, e submete ao **`lina-cold-review`** (revisor isolado) no gate de saída. Só
narra "pronto" ao leigo com **cold-review PASS + critérios do plan cumpridos**. Evidência observada, não auto-relato (espelha `lina-verification`).

## 5. Anti-race no despacho (story crit. 4)
Dependências moram em `parents:` (dado estruturado), e a verificação acontece **no INSTANTE do despacho**, não
só quando o item foi criado/promovido:
- Antes de despachar T (com `parents: [P]`): **re-leia o plano AGORA** (`lina plan read`) e confirme que `P`
  está concluído **neste momento**. Uma leitura antiga pode estar velha — o worker de `P` pode ter falhado/voltado.
- Se algum `parent:` ainda está pendente → **recuse/adie** o despacho de T. (Anti-race ecoa
  `kanban_db.py:2996-3012` e [[lina-a2a-routeblocked-race-e-ask-ok-cego]].)

## 6. Fronteiras de escopo por worker (CLAUDE.md "Protocolo multi-terminal")
- Cada worker recebe uma **fronteira de arquivos** no despacho (campo DIRECIONAMENTO do `lina-dispatch`:
  "mexa SÓ em X"). Costuras (arquivos de contrato) têm **dono único por rodada**.
- A trava cooperativa é o **claim** do item do plano que cobre o arquivo (`lina plan claim`). Nunca despache
  dois itens que tocam o mesmo arquivo sem serializar — colisão de escrita corrompe trabalho.
- **Nunca** edite/reverta linha de peer; precisa de algo numa costura alheia → peça ao dono.

## 7. Mapa: critério da story → onde é exercido
- **crit.1 (é o gate):** o loop inteiro no cenário LP-3-terminais (§2 do onda-3).
- **crit.2 (correção de trajeto):** §1 (detecção pelas projeções) + §2 (re-despacho informado).
- **crit.3 (breaker):** §3 linha "Falha" (2× → escala, sem 3ª automática).
- **crit.4 (anti-race parents:):** §5 (re-verificação no instante do despacho).
- **crit.5 (saída cold-review):** §4 (gate de saída de fora, PASS).
