# F3-4 · Alvos diferenciais (QA/Red-team R) — LIGADOS na FASE 2 ✅

## STATUS FINAL (FASE 2 concluída — mapa de evidência do veredito)

Todos os alvos abaixo foram LIGADOS na develop integrada (suíte lib `lina-core` **481/0** auditada
rodando). Onde cada critério ficou provado:

| Critério | Onde foi provado | Por |
|---|---|---|
| (e3) salvaguardas gated-hard (reset --hard/branch -D/checkout -f) | `f3_4_salvaguardas_guard.rs` (`#[ignore]` removido → 5/5 GREEN; RED→GREEN provado) | R |
| (e1) author_node server-side (forja no payload ignorada) | `router.rs::code_changed_stamps_author_server_side_ignoring_payload` + `attention.rs::conflict_uses_event_author_not_item_owner_field` | I |
| (e1) author_node obrigatório (forja por omissão) | `f3_4_contrato_replay.rs::author_node_obrigatorio_forja_por_omissao_rejeitada` | R |
| (e2) paths×claims nunca concede posse | `f3_4_autoridade_pertencimento.rs` (sinal META não altera posse) | R |
| (b) pertencimento: interseção→atenção; disjunto→silêncio | `router.rs::code_changed_notifies_only_intersecting_claim_owner` + `attention.rs::code_conflict_fires_on_intersecting_claim_of_another_node`/`no_conflict_when_paths_are_disjoint` | I |
| (e4) broadcast respeita só VIVOS (dono morto excluído) | `f3_4_seguranca_integracao.rs::broadcast_por_pertencimento_ignora_dono_morto` | R |
| (forja) BranchIntegrated forjado nunca destrói código | `f3_4_seguranca_integracao.rs::branch_integrated_forjado_nunca_destroi_codigo` + `..._sem_campo_de_identidade_forjavel` | R |
| (c) gatilho/projeção branches-não-integradas | `code.rs::{code_changed_without_integration_is_pending,branch_integrated_removes_from_pending,later_commit_after_integration_reopens_branch}` + `router.rs::branch_integrated_emits_event_and_closes_branch_end_to_end` | I, A |
| (f) replay idêntico / META no-op | `f3_4_contrato_replay.rs` (4 GREEN) + `attention.rs::code_conflict_replay_is_idempotent` | R, I |

O texto abaixo é o PLANO original da FASE 1 (mantido como registro de como os alvos foram faseados).

---

> Precedente: `f3_0_params_adversarial.md`. Estes são os critérios do gate cuja prova **não compila
> hoje** porque dependem de API que as Trilhas A/B ainda não escreveram (`PlanItem.paths`, handler de
> `CodeChanged` no router, verbo `lina code-changed`, broadcast por pertencimento, módulo `code`).
> Um teste `#[ignore]` ainda PRECISA compilar — então um alvo que chama função inexistente não pode
> ser escrito como teste agora (envenenaria o pacote inteiro). Ficam aqui, com o comando e o critério
> RED→GREEN, para serem escritos/ligados quando a fiação chegar (a integração os fecha — alguns ao
> vivo, gate (g)).

## O que JÁ está provado (suíte padrão, sem fiação de peer)

| Critério | Arquivo | Estado |
|---|---|---|
| (d)/(e3) `reset --hard`/`branch -D`/`checkout -f` gated-hard | `f3_4_salvaguardas_guard.rs` | **RED provado** via `-- --ignored` (exit 101); GREEN quando Trilha A liga o classificador. Controles (push --force barrado; negativos não classificam demais) **GREEN**. |
| (f) round-trip byte-a-byte + `kind()` canônico | `f3_4_contrato_replay.rs` | **GREEN** |
| (e1-parcial) `author_node` obrigatório (forja por OMISSÃO barrada) | `f3_4_contrato_replay.rs` | **GREEN** |
| (f) META no-op (`CodeChanged`/`BranchIntegrated` não tocam `ProjectedState`) | `f3_4_contrato_replay.rs` | **GREEN** |
| (e2-alicerce) sinal de código nunca altera posse | `f3_4_autoridade_pertencimento.rs` | **GREEN** |
| (e4-alicerce) `node_by_name` ignora morto | `f3_conf3_router_autoridade.rs` (já existe) | **GREEN** — não re-provado aqui |

## Alvos que DEPENDEM da fiação (ligar na FASE 2)

### (b) Pertencimento: `CodeChanged{paths}` ∩ claim → atenção; sem interseção → silêncio
- **Depende de:** Trilha B — handler de `CodeChanged` no `router.rs` + `PlanItem.paths` (ADR 0041) +
  `AttentionKind::CodeConflict` (`attention.rs`).
- **Prova (zero-mock, caminho real):** via `route_message`/`pump` (nunca evento montado à mão —
  senão não prova a costura). Apendar um claim de `@A` em `T1{paths:[x]}`; rotear um `CodeChanged`
  de `@B` com `paths:[x]` → **exatamente 1** item de atenção `CodeConflict` para `@A`; com
  `paths:[y]` (sem interseção) → **zero** atenção e estado de `@A` inalterado.
- **Não-vacuosidade:** o caso "sem interseção" é o controle (não pode abrir atenção por qualquer
  commit).

### (e1) `author_node` carimbado SERVER-SIDE na emissão (forja no payload ignorada)
- **Depende de:** Trilha A — verbo `lina code-changed` + emissão pelo outbox autenticado (binding do
  nó, ADR 0026), que carimba `author_node` **ignorando** o que o agente escreveu.
- **Prova por MUTAÇÃO:** emitir um `CodeChanged` pelo caminho real com `author_node` FORJADO no
  payload (ex.: `@Maestro`) a partir de um nó autenticado `@B`; ler o EVENTO no log → `author_node`
  é `@B` (o binding), nunca o forjado. Desligar o binding server-side → o forjado vazaria → o teste
  falha → religar.
- **Já coberto:** a metade "forja por OMISSÃO" (campo obrigatório) está GREEN em
  `f3_4_contrato_replay.rs`.

### (e2) `paths` forjado não concede posse (com `PlanItem.paths` real)
- **Depende de:** Trilha B — `PlanItem.paths` no `parse_item`/render + a interseção no handler.
- **Prova:** round-trip de plano sem `paths` (replay byte-a-byte, inv #4); e um `CodeChanged` com
  `paths` que intersecta um claim de **outro** nó NÃO transfere posse — só abre atenção.
- **Já coberto (alicerce):** `f3_4_autoridade_pertencimento.rs` prova que o sinal é META e não muda
  o plano projetado; o ângulo com `PlanItem.paths` concreto entra quando o campo existir.

### (e4) Broadcast por pertencimento respeita SÓ vivos
- **Depende de:** Trilha B — o broadcast por pertencimento que notifica os nós vivos ao chegar um
  `CodeChanged`.
- **Prova por MUTAÇÃO:** registrar nós vivos e mortos; um `CodeChanged` notifica **apenas** os vivos
  (o morto não recebe). Desligar a guarda `is_alive()` no caminho do broadcast → o morto receberia →
  falha → religar.
- **Alicerce já provado:** `f3_conf3_router_autoridade.rs` (`node_by_name` ignora morto).

### (c) DevOps integrador: gatilho determinístico + `BranchIntegrated`
- **Depende de:** Trilha B — módulo `code.rs` (`branches_nao_integradas` + gatilho).
- **Prova:** `∃ CodeChanged{branch}` ∧ `∄ BranchIntegrated{branch}` ⇒ branch pendente; com
  `BranchIntegrated` ⇒ some. Gatilho só dispara com TODOS os nós Idle/Dead ≥ T.
- **Prova AO VIVO (gate c/g):** a integração executada por mim (integrador) seguindo a skill
  `lina-integration` — junta `lina/f3-4-a` + `lina/f3-4-b` na `develop`, cada merge → `BranchIntegrated`
  no log; conflito não-trivial → gate humano narrado; órfão → atenção, **nunca** `branch -D`.

### (d) Órfão vira atenção, nunca lixo
- **Prova AO VIVO:** durante a integração, qualquer branch não-mergeável vira item de atenção (não
  `branch -D`). Verificado no relatório de integração da FASE 2.

## Comando-base de prova (a usar quando a fiação chegar)

```sh
# de fora, exit direto (zsh): nada de pipe mascarando o código
cargo test -p lina-core --test f3_4_<arquivo> >/tmp/log 2>&1; echo $?
cargo test -p lina-core --test f3_4_salvaguardas_guard -- --ignored >/tmp/log 2>&1; echo $?  # destrava na integração
```
