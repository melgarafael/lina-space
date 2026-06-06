# ADR 0009 — Segurança da aprovação remota (injeção de y/n): snapshot-hash do VT + stable ID + idempotência

- **Status:** Aceito (fundador/Maestro, 2026-06-06) — com **gate de processo**: a story de injeção só inicia após revisão de segurança própria (red-team interno) deste desenho
- **Onda/Story:** F1-1 (fila de atenção — story de aprovação remota)
- **Data:** 2026-06-06
- **Fontes:** pesquisa `13.13` (R2) · CVE-2024-27936 (ANSI spoofing) · CVE-2024-32477 (tcflush race) · GHSA-95cj-3hr2-7j5j (Deno) · `tasks/epico-f1/arquitetura.md` §c/§c.1

## Contexto

A fila de atenção (item 7 do doc vivo: "aviso quando algum terminal ficar travado por
permissões yes/no") culmina em o usuário clicar "aprovar" no canvas e o Lina **injetar `y\n`
no stdin de um PTY remoto**. O risco é real e documentado por CVE: o emulador envia replies
ANSI pelo mesmo canal do stdout, então uma resposta pode chegar **após** o flush e virar input
(GHSA-95cj-3hr2-7j5j); o prompt pode MUDAR entre a notificação e o clique (aprovação do prompt
errado); e spoofing de prompt via escapes é ataque conhecido (CVE-2024-27936).

Ponto epistemológico verificado pela pesquisa (13.13 §5): **não existe padrão de indústria
documentado** para mitigar essa race (a correção do Deno foi melhoria de `tcflush`, não
snapshot). A técnica adotada aqui é **decisão de engenharia do Lina** — registrada como tal,
e provada por teste próprio, não importada como "padrão de mercado".

## Decisão

1. **Snapshot-hash do estado do VT como pré-condição do write.** Ao detectar o pedido de
   permissão, o detector captura as linhas relevantes do grid (o prompt) e calcula um hash;
   no clique de aprovação, re-captura e compara. **Divergência ⇒ NÃO injeta** — a UI
   reapresenta o estado atual do terminal e pede novo gesto do usuário.
2. **Stable ID + idempotência.** `PermissionEnvelope { session_id, node_id, tool_name,
   tool_input_hash, idempotency_key (ULID), created_at, vt_snapshot_hash }`. Cada aprovação é
   processada **no máximo uma vez** (clique duplo/replay = no-op auditado).
3. **Fila unificada serial.** Permissão entra na MESMA fila da custódia (extensão do
   BrokerPump), com precedência **custódia > permissão > custom gates** e drain round-robin
   anti-starvation. A injeção respeita a fila serial por terminal do W0-9 (nunca write
   concorrente no mesmo PTY).
4. **SLA de não-resposta.** Pedido sem resposta em N minutos (default conservador: 10min)
   → escalada de atenção (badge/som); persistindo → **auto-deny com evento** (nunca
   auto-approve).
5. **Eventos (invariante #4):** `PermissionRequested { node, tool, input_hash,
   idempotency_key }` e `PermissionResolved { id, decision, via: Human|Timeout }` — toda
   decisão auditável por replay.
6. **Gate de processo (parte da decisão):** antes da story de injeção entrar em
   implementação, rodar **red-team interno** deste desenho com teste de race dedicado:
   (a) prompt muda entre toast e clique; (b) ANSI bypass/spoofing do prompt; (c) clique
   duplo/replay; (d) dois pedidos simultâneos de terminais distintos. O teste de race é a
   evidência da mitigação — sem ele, o ADR não se considera implementado.

## Limite explícito

- A mitigação **reduz a janela da race; não a elimina por prova formal** — por isso o teste
  de race é parte da decisão, não um nice-to-have.
- Same-uid continua **não** sendo fronteira de SO (L1-3, ADR 0006 §Limite) — este ADR não
  protege contra processo malicioso na mesma UID escrevendo direto no PTY.
- Aprovação remota **não substitui a custódia**: ação `gated-hard` externa continua exigindo
  o gate duro do ADR 0004 (segredo), independentemente de qualquer y/n aprovado.

## Alternativas rejeitadas

- **Injetar direto sem validação de estado** — é exatamente a race documentada por CVE.
- **Só melhorar o flush (estilo Deno 1.42.2)** — não cobre a mudança de prompt entre a
  notificação e o clique (janela de segundos/minutos, não de milissegundos).
- **Só focar o terminal, sem injetar** — quebra o valor para o não-técnico (invariante #6);
  mantido apenas como **fallback degradado** quando o snapshot-hash diverge.
- **Auto-approve em timeout** — inverte o fail-safe; timeout sempre nega.
