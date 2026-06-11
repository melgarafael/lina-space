# DESPACHO r3-f1-4-3 — Bug Finder
**id:** `f1-4-3` · Leia ANTES: `_regras-comuns.md` + `coordenacao-maestri-externa.md` (fronteiras v2!)

## F1-4-3 · Restore de terminais vivos no boot normal (fonte: `tasks/epico-f1/ondas-2-4.md` linhas 201-218)
Quit limpo → reabrir → o Espaço volta: mesmas posições/nomes/papéis, scrollback re-hidratado do disco, doutrina re-injetada pelo bootstrap turno-0; **resume de sessão do CLI quando o profile TOML declarar** (campo novo de resume no CLI Profile — o verbo vem do TOML, ex. `claude --resume`, NUNCA do código; inv#3); **badge honesto**: "Conversa retomada" vs "Novo começo — o agente não lembra da conversa anterior" (strings EXATAS do `copy-f1-4.md` §4 — condicionadas ao OBSERVADO, não ao declarado); opt-out por Espaço. NÃO reimplemente T8 (pós-crash continua W0-6/W4-4; esta story é o quit LIMPO reusando o mecanismo).
**Critérios:** (1) headless: quit limpo com 3 nós (profile com resume declarado, shell puro, profile sem resume) → replay restaura posições/nomes/papéis + scrollback re-hidratado byte-idêntico (≥ janela viva); (2) o nó sem resume ganha o badge "novo começo" na PROJEÇÃO (lógica do estado, testável headless; render na tela = roteiro); (3) trocar o verbo de resume no TOML muda o comando SEM recompilar (teste com 2 TOMLs); (4) tudo derivável do log (limpar projeções → replay → mesmo restore); (5) a prova "claude responde com contexto da sessão anterior" NA TELA = escreva o roteiro em `tasks/epico-f1/roteiro-f1-4-3.md`.

## Fronteira (sua)
`app/lina-gpui/src/bridge.rs` (dono) · `crates/lina-cli-profiles/**` (campo resume no TOML + parse) · `profiles/*.toml` (campo novo aditivo) · testes.
**⛔ NÃO toque:** `main.rs`/`attention*`/`agent_modal`/`broker`/`pretooluse`/`bin/lina.rs`/`lib.rs`-bootstrap (EXTERNO) · `events.rs` SÓ hunk aditivo se for indispensável (avise na entrega) · scrollback.rs/workspace.rs (estáveis — consuma a API).
Entrega: `tasks/epico-f1/.entrega-f1-4-3.md` · Marcador: `.iniciado-f1-4-3`.

## ADENDO (Maestro, 11/06 ~12h) — F1-5-2 ENTRA NESTA FATIA (sinergia com o restore)
**Descoberta de seam:** o app NÃO usa `lina_core::PtyHost` — é `PtyManager`-direto (`wire_terminal`, reader thread + Grid próprios; comentário em `bridge.rs:4438`). Logo o "wire de 1 linha" da F1-5-2 (`set_scrollback_store`) não se aplica literalmente; o cabo real é:
1. Criar o `ScrollbackStore` no boot do bridge (`<ws>/.lina/scrollback.db` — MESMO lar de dados, arquivo separado do events).
2. No reader/advance do app (caminho do `wire_terminal`), colher `take_scrollback()` do `VtBackend` após o advance e empurrar ao store (o mecanismo sub-chunk/harvest já vive em lina-vt — REUSE; não duplique a lógica do core).
3. Subir o `FlushGuard` (F1-5-6) sobre o MESMO store — se `FlushGuard` exigir `PtyHost`, exponha um construtor standalone no scrollback.rs (hunk interno permitido) em vez de migrar o app para PtyHost (a migração é a Opção A do ADR 0024 — fora desta fatia).
4. `eprintln` único por spawn: `[SB] store anexado painel=<nome>` (o Maestro valida por log).
**Critérios F1-5-2:** (a) `grep set_scrollback_store|ScrollbackStore app/lina-gpui/src/` retorna wire real; (b) app vivo + output → `<ws>/.lina/scrollback.db` cresce; (c) seu RESTORE (story principal) re-hidrata DESTE store — prova as duas stories juntas; (d) suites verdes por-pacote.
Isto também destrava a medição ANTES/DEPOIS do harvest no app (registro no prof-baseline fica com o Maestro).
