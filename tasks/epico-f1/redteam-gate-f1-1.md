# Red-team do Gate F1-1 — superfície de aprovação y/n (F1-1-8 + ADR 0024) + reinject (F1-2-4 + ADR 0023)

**Data:** 2026-06-08 · **Conduzido por:** Workflow read-only (29 agentes, 4 lentes + céticos) · **Re-derivado no código pelo Maestro** (finder infla nit→ALTA; verificador otimiza — re-derivação obrigatória). **HEAD na auditoria:** `5962705`.

## Veredito

**Auditoria inicial do Workflow: FAIL (3 ALTA).** Após re-derivação do Maestro no código: **2 refutados/rebaixados + 1 furo real (A3) — CORRIGIDO em `f6db7e3`.** Superfície da aprovação y/n (assunto da onda) **limpa**. **0 ALTA aberto → metade de segurança do gate PASSA.** 8 MEDIA → backlog. Falta a tela do fundador (AC-0021.7).

## ALTA — re-derivadas

| # | Achado do finder | Re-derivação do Maestro no código | Desfecho |
|---|---|---|---|
| **A1** | Aprovação cruzada via nó homônimo: `node_by_name` resolve o nó errado → escreve y/n no PTV da vítima | **Mitigado e não-alcançável.** `expected_hash` é da tela do nó X na detecção; resolver p/ homônimo Y → grid de Y ≠ hash → `check_screen` ABORTA, zero bytes (o próprio comentário `bridge.rs:1432` documenta). E o `node_id` vem do `AppPermissionWatch::scan` (nome do roster do app ao varrer aquele grid), **não escrito pelo agente**. | **NÃO-ALTA → BAIXA** (hardening: resolver por `NodeId`). Backlog. |
| **A2** | Spoof de origem no reinject: `from` do arquivo decide o alvo | **Refutado.** `drain_to_inflight` CARIMBA `msg.from = nome-do-subdir` (`mailbox.rs:590`), sobrescrevendo o `from` do JSON. Teste `per_node_drain_attributes_from_to_dir_owner`. | **REFUTADO.** |
| **A3** | Escrita não-mediada no `reinject_dir`: agente injeta no PTV do peer | **FURO REAL.** O reinject era mailbox de **filesystem**; agentes = mesmo usuário de SO → escrita direta em `<.lina>/reinject/<peer>/` → injeção de payload arbitrário no PTV do peer, **sem 2ª camada** (router). A premissa do ADR 0023 ("nenhum verbo `lina` alcança") ignorava escrita direta de FS. (O Maestro foi verificador-otimista ao aprovar o ADR 0023.) | **CORRIGIDO `f6db7e3`:** reinject vira fila EM-PROCESSO (sem FS), target=`NodeId` autenticado, texto regenerado do role (nunca payload). Teste `reinject_has_no_filesystem_dropzone_fix_a3`. ADR 0023 emendado. |

## MEDIA → backlog (com dono sugerido)

| # | Título | Arquivo:linha | Dono |
|---|---|---|---|
| M1 | Reinject pós-crash relia `from` do `.inflight` sem re-validar | (obsoleto pós-FIX-A3: fila em-processo não persiste) | — (fechado por A3) |
| M2 | `observe_grid` aceita `node_id` do chamador sem cross-check com dono do grid (fallback ~5,8s) | `permission_detect.rs:236-242,284-289`; `approval.rs:270-272` | core/detector |
| M3 | (obsoleto pós-FIX-A3: era o `drain_reinject` por `node_by_name(m.from)`) | — | — (fechado por A3) |
| M4 | Atomicidade: brecha de durabilidade kernel-buffer entre `write_all` e `flush` (janela irredutível, auto-reconhecida) | `lib.rs:1505-1508` | core/pty-host |
| M5 | (obsoleto pós-FIX-A3: assimetria de descarte/retry do reinject FS) | — | — (fechado por A3) |
| M6 | Snapshot K dinâmico: colisão teórica se K baixo e agente controla sequência (SHA-256 mitiga) | `approval.rs:70-110` | core/approval |
| M7 | `AttentionQueue::resolve` guard dinâmico (filtro `!= Yn`), não typestate | `attention.rs:388-390` | core/attention |
| Hardening | Esc/Yn-gate provados por teste dinâmico, não typestate (migrar p/ typestate resiste a regressão por novo call-site) | `attention.rs`, `bridge.rs:7052` | backlog |

> Nota: M1/M3/M5 eram do reinject de FS — fechados de raiz pela fila em-processo (FIX-A3). Restam M2/M4/M6/M7 + o hardening de typestate como backlog real.

## Confirmações positivas de invariantes (com a linha que enforça)

| Invariante | Linha |
|---|---|
| Deny/timeout NUNCA vira approve | `attention.rs:148-157` (`AutoDenyDue` omite `decision`); `bridge.rs:1727` literal `Deny` |
| Yn é pré-requisito de decisão | `attention.rs:388-390,490` |
| Esc nunca decide | teste `bridge.rs:7052-7077` (snooze, zero evento) |
| Screen-changed aborta com zero bytes | `approval.rs:448-461` + `lib.rs:531-560` |
| Binding do LOG, não do gesto (UI só carrega `stable_id`) | `approval.rs:419` (`node_of` ≠ `target` aborta) |
| Idempotência pós-Resolved (projeção pura) | `approval.rs:391-479`; teste `rebuild_from_log_never_rewrites` |
| Ordem escalate < auto-deny (compile-time) | `attention.rs:67` (`const assert`) |
| Reinject inalcançável por agente | **pós-FIX-A3:** fila em-processo (`bridge.rs` `ReinjectQueue`), teste `reinject_has_no_filesystem_dropzone_fix_a3` |

## Lição

A re-derivação no código é obrigatória nos DOIS sentidos: o finder rateou A1 (mitigado) e A2 (carimbo enforced) como ALTA, mas acertou A3 (real). E o Maestro havia sido **verificador-otimista** ao aprovar o ADR 0023 (chequei "sem verbo `lina`" mas não escrita direta de FS com mesmo-usuário-de-SO). O gate adversarial se pagou num único achado real. Ver memórias [[verificador-adversarial-otimista-demais]], [[redteam-reverificar-mecanismo-do-finder]], [[lina-from-autenticacao-duas-camadas]].
