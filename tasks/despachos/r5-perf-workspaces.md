# DESPACHO r5-perf-workspaces — Core A2A
**id:** `perf-ws` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

BUG 3 da tela do fundador (2026-06-11 ~23h, uso real): **"depois do sistema de workspaces o Lina ficou lento demais, trava toda hora, tá impossível de usar"**. Investigação de causa-raiz + mecanismo de baixo consumo para Espaços de fundo. Rodada r5 (fix-sidebar).

## CONTEXTO
- `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"` (HEAD `be9a45b`). Você acabou de mergulhar no runtime/dreno — o contexto é seu.
- **Suspeito nº 1 (achado do Maestro no reconhecimento — CONFIRME/MEÇA antes de aceitar):** `runtime.rs:392-418` — CADA `boot_ws_runtime` sobe uma thread SessionWatch com `loop { poll_once(); sleep(500ms) }` varrendo o HOME real (`~/.claude/projects` etc. — milhares de arquivos nesta máquina). N Espaços vivos = N varreduras de disco/s contínuas, inclusive em fundo, para sempre; `known` cresce sem teto. Era a pendência nomeada do T1 (`.entrega-m8-t1.md` §pendências: "SessionWatch por-runtime consolidar").
- **Outros suspeitos a cobrir (não pare no primeiro):** (a) dreno barato de fundo (`runtime.rs`, veredito §1) — está MESMO barato? consome e descarta ou processa VT advance completo?; (b) `WorkspaceMiniStatus` (`e7fb3c6`) — polling/recálculo caro no render?; (c) N runtimes mantêm N `NodeManager`+grids+scrollback inteiros na RAM (qual o custo por Espaço de fundo?); (d) PTYs de fundo jorrando output (claude streaming) custando CPU em advance de VT que ninguém vê.
- **Fundações que JÁ existem (reuse, não reinvente):** F1-5-5 suspensão real de ociosos NO CORE (`626228c`: Active→Idle→Suspended, drenagem nunca para) — verifique se está FIADA aos runtimes de fundo do app (provavelmente não); sonda `[PROF]` decomposta (`prof.rs`, F1-5-1) para o que exigir tela; `bench_load` headless (W5-1).
- **O que o fundador pediu (modelo Maestri "Descarregar"):** Espaços de fundo devem consumir quase nada; um mecanismo de descarregar (desligar terminais SEM remover — religam ao focar). **Restrição dele, literal: "sem interferir nos processos que estiverem rodando nesse workspace"** — terminal Busy/trabalhando NÃO pode ser morto em silêncio; descarregue o ocioso, preserve (ou peça confirmação para) o ativo.

## FUNÇÃO
Você é o investigador de causa-raiz e dono do mecanismo de consumo de fundo nesta rodada.

## DIRECIONAMENTO
- **MEÇA antes de consertar** (systematic-debugging): para cada suspeito, derive o custo com evidência (teste headless de custo, contadores, instrumentação `eprintln` que o Maestro/fundador lê no stderr). O fix sem a medição do ANTES não fecha o bug do fundador.
- Ordem do trabalho: (1) causa-raiz da lentidão sistêmica (provável SessionWatch — consolidar em 1 watcher por PROCESSO compartilhado entre runtimes, ou por-runtime gated a foco); (2) custo de fundo: fiar a suspensão F1-5-5 aos runtimes não-ativos (ociosos de fundo → Suspended; drenagem/mailbox SEGUE viva — A2A cross-Espaço não pode morrer); (3) mecanismo "Descarregar Espaço" manual: desliga os PTYs OCIOSOS do Espaço de fundo sem remover nada (estado durável; religam no próximo foco), Busy preservado ou gate de confirmação.
- Fronteira: `app/lina-gpui/src/runtime.rs` + `crates/lina-core/**` (se a fiação da suspensão exigir) + `crates/lina-session-watch/**` (se a consolidação exigir) + testes. **NÃO toque** `sidebar.rs`/`gallery.rs`/`persistence_ui.rs`/`main.rs` (dono na r5: Terminal C) — o botão "Descarregar" no rail e qualquer fiação em main.rs viram PEDIDO DE COSTURA na entrega (o Maestro/C fiam).
- Doutrina de segurança intacta (regra 7); eventos novos aditivos; suíte do router verde se tocar core.
- gpui não roda headless: o que só se prova na tela (fps/fluidez), instrumente e deixe o roteiro de 3 passos para o fundador; o que dá para provar headless (threads, I/O por segundo, bytes por Espaço de fundo), prove por teste.

## OBJETIVO
O fundador disse "impossível de usar" — este é o bug nº 1 do produto AGORA. Critério de sucesso: causa-raiz nomeada com número do ANTES e do DEPOIS; Espaço de fundo ocioso custa ~zero de CPU/disco; nada que estava trabalhando morre sem consentimento.

## RESULTADO ESPERADO
`tasks/epico-f1/.entrega-perf-ws.md`: por suspeito — custo medido (ANTES) · veredito (causa/inocente) · fix aplicado (arquivo:linha) · custo DEPOIS; pedidos de costura para o rail; roteiro curto de validação na tela. Marcador `.iniciado-perf-ws`. Validação por-pacote exit direto. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Reporte status ao @Terminal A (--intent status) ao começar/terminar/travar.
