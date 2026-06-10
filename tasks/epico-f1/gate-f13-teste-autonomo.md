# Gate F1-3 — Teste AUTÔNOMO do cenário (computer-use, 2026-06-10)

> **Executor:** Maestro (Claude Code), dirigindo o app `lina-gpui` (build dev pós-`dd32d46`) por
> **teclado sintético** (osascript) + **screenshots** (screencapture) + **eventos do `log.jsonl`**
> (fonte da verdade). Workspace isolado `/tmp/lina-gate-f13` (LINA_DEMO=1, LINA_WS_ROOT).
> **3 claudes REAIS** (2.1.170): Terminal A (líder), Terminal B (revisor), Designer (criado ao vivo
> via ⌘N com sugestão automática de papel).
> ⚠️ **Este teste NÃO declara o gate** — a validação do fundador na tela é bloqueante por decisão
> (2026-06-06). É evidência forte de prontidão + 3 achados de produto.

## O pedido (leigo, sem ensinar nada)

> *"Constrói uma landing page simples do meu curso de IA para iniciantes, na pasta lp/ do seu
> diretório. Usa o time de terminais que está no Espaço com você — quero qualidade de verdade,
> não algo genérico."* (digitado com acentos via teclado sintético — IME ok)

## O que aconteceu (evento-a-evento, seqs do log.jsonl)

1. **Líder se auto-descobriu** (`lina --help`/`whoami`) e **consultou o vault** (briefing menciona
   "Rafael, ecossistema Automatik" — internalização do bloco 3).
2. **Decompôs**: escreveu `lp/briefing.md` (produto/público/objetivo/decisão técnica) + se
   auto-atribuiu a copy + **delegou direção visual ao @Designer** (seq 28-30: Routed hops:0 →
   Designer Idle→Busy → Delivered) com `--context` (pull-then-context).
3. **Designer entregou** `lp/direcao-visual.md` de alta qualidade: direção DECLARADA ("Editorial
   Caloroso") + justificativa pro público + tokens com hex + regras ("--brasa é a ÚNICA cor de
   ação; zero gradiente/glass/dark-neon") — doutrina lina-design-doctrine encarnada.
4. **Resiliência exercitada por acidente real** (líder em turno longo, prompt nunca pronto):
   resposta do Designer → 5× retry+backoff (seq 32-37) → **`MessageDeadLettered` com motivo
   legível + `CircuitOpened` + nó `Blocked{circuit_breaker}`** (seq 38-40). Nada perdido em
   silêncio. Recovery: novas msgs ao líder **`MessageRetained{circuit_open}`** (seq 45) →
   líder Idle → **drenadas e entregues** (seq 47-50). Retenção `target_busy` + drain-no-Idle
   funcionaram em TODOS os ciclos seguintes (seq 44→53-55, 56→65-67, 68→77-79, 69→72-74…).
5. **Líder construiu** `lp/index.html` (copy própria + direção do Designer), subiu http.server,
   **abriu o Chrome e verificou** (evidência observada), testou 1440px e 390px, gerou
   `preview-mobile.png`.
6. **Cold-review real** (handoff ao B com contrato `lina/msg@2`: output_schema/timeout 600s/
   retry manual/[EXPECTED] PRONTO:/BLOCKED:): B devolveu **FAIL — 1 bloqueador** (CTA final
   `href='#'`) + 3 MEDIA (contraste/aria) + 5 BAIXA + "o que PASSOU" honesto; pegou até prova
   social não-verificável (copy ética).
7. **Correção de trajeto**: líder corrigiu item-a-item (CTA→mailto c/ comentário pro checkout
   real; contraste #B34114 ≥4.5:1; <ol> semântico; copy suavizada), re-verificou visualmente,
   re-submeteu. **B re-derivou NO ARQUIVO** ("confiança é bom, evidência é melhor"; "recalculei
   o contraste") → **PASS com ressalva honesta** (link de pagamento é placeholder).
8. **Idempotência espontânea** nos 2 lados: duplicatas (re-envios pós-retain) reconhecidas
   ("o arquivo é idêntico ao que já avaliei, não preciso refazer").
9. **Narração leiga final** do líder: "Aprovada! ✅ … a única pendência real antes de divulgar é
   você me dizer qual plataforma de pagamento vai usar". **Anti-loop fechou a conversa**:
   `RouteBlocked{hop_limit}` no ping-pong de cortesias (seq 89); B: "trocar outro 'ok' só
   geraria ping-pong sem valor".

## Veredito por critério do gate (§2 onda-3.md)

| Critério | Veredito | Evidência |
|---|---|---|
| (1) Um terminal lidera, decompõe, define funções e despacha | ✅ observado | briefing.md + handoffs seq 28/41/53 c/ contrato @2 |
| (2) Workers executam; orquestrador detecta e corrige trajeto | ⚠️ **parcial** | Correção de trajeto REAL aconteceu (FAIL→fix→PASS + monitoramento por until-loop `lina check`), mas o cenário formal exige worker **induzido** a travar/desviar — não executado nesta rodada |
| (3) Cold-review com rubrica: PASS, zero slop duro | ✅ observado | FAIL c/ bloqueador → re-derivação no arquivo → PASS; estética: serif/terracota/direção declarada, ZERO Inter/gradiente-roxo |
| (4) Narração pt-br leiga, zero jargão | ✅ observado | "Aprovada! … troco em um minuto"; pendências narradas como decisões do dono |
| Medição evento-a-evento no log | ✅ | 92 seqs; cadeia íntegra |
| ⛔ Validação do fundador NA TELA | **PENDENTE (bloqueante)** | este teste não substitui |

**Plano compartilhado (F1-0-9):** B rodou `lina plan read` → "sem plano ainda" — o líder coordenou
por handoff direto, sem plan.md (telemetria de adoção: 0%, consistente com baseline; métrica
acompanhada, não critério binário).

## Achados de produto (carry-forward)

- **ACHADO-1 (fiação, MEDIA): skills F1-3 não chegam ao terminal.** O kit por-nó instala SÓ
  `lina-agent-bus`; instalei as 11 manualmente p/ o teste. Fiar `assets/lina-skills/*` no kit
  (lina-bootstrap) — sem isso o gate real não tem lina-orchestration/cold-review/dispatch.
- **ACHADO-2 (produto, MEDIA): descasamento lifecycle×prompt-real → DLQ em vez de retenção.**
  Com o lifecycle marcando Idle mas a TUI ocupada (turno longo), a entrega cai na rota
  retry→5-strikes→DLQ (seq 32-40) em vez de `MessageRetained{target_busy}`. O sistema NÃO perde
  nada (DLQ+breaker+recovery ok), mas o caminho ideal é checar prontidão REAL além do estado.
- **ACHADO-3 (compilação, fechado em `dd32d46`):** o app não compilava pós-F1-3-6 (8 call-sites
  `NodeAdded` sem `requested_by`) — corrigido + 283 testes verdes. Lição: fatia de core com campo
  aditivo exige build do app antes do commit (call-sites de construção não são cobertos por serde default).
- **BÔNUS (capacidade de teste): teclado sintético FUNCIONA no gpui** (osascript keystroke + IME
  com acentos perfeitos; paleta ⌘K "Focar: X" dá navegação 100% por teclado). Cliques seguem
  mortos. Atualiza [[computer-use-mac-gpui-ve-mas-dirige-mal]].

## Evidências
`/tmp/lina-gate-f13/`: shot-01..20.png · app-stderr.log · `.lina/events/log.jsonl` (92 seqs) ·
`n-*/lp/{briefing,copy,direcao-visual}.md` · `n-*/lp/index.html` + `preview-mobile.png`.
⚠️ /tmp evapora no reboot — LP copiada para `tasks/epico-f1/gate-f13-artefatos/`.
