# Contrato do verbo `lina history` — F1-5-8 (fiação: time externo, `bin/lina.rs`)

> A fatia CORE está entregue em `crates/lina-core/src/history.rs` (testes:
> `crates/lina-core/tests/history_f1_5_8.rs`, 6/6 verdes). Este doc é o contrato para a
> fiação do verbo ser ~10 linhas: parsear flags → chamar a função core → imprimir.

## Sintaxe

```
lina history <painel> [--tail N] [--offset K]            # últimas N linhas (default)
lina history <painel> --search "<regex>" [--limit N] [--cursor IDX]
lina history <painel> --export json|txt --from IDX --to IDX
```

- `<painel>` = nome do terminal como aparece no roster (`lina list`). O DONO do painel.
- O REMETENTE (quem chama) é a identidade do terminal corrente (env `LINA_NODE_ID`, ADR 0026).

## Mapeamento flag → API core (tudo já pronto)

| Verbo | Função core | Notas |
|---|---|---|
| `--tail N --offset K` | `history::tail(store, panel, Some(N), K, &limits)` | default quando nenhuma flag de modo |
| `--search RE --limit N --cursor I` | `history::search(store, panel, RE, Some(N), Some(I), &limits)` | regex inválido → erro legível (`BadRegex`) |
| `--export FMT --from A --to B` | `history::export(store, panel, FMT, A, B, &limits)` | devolve `(payload, next_cursor)` — SÓ leitura própria |
| leitura de painel de OUTRO terminal | `history::tail_cross` / `search_cross` / `export_cross` `(events, members, reader, owner, …)` | **OBRIGATÓRIO no caminho cross — as TRÊS ops** — audita no log (`HistoryReadCross`, `query` = `tail`/`search`/`export`); fora do Espaço nega (`CrossDenied`) |

- `limits = HistoryLimits::default()` → página default 200, teto 1000, varredura máx. 10k/chamada.
  **Não exponha flag para subir o teto** — o limite duro é o ponto da story (anti-DoS por agente).
- `members` = nós vivos do Espaço (mesma fonte da `WorkspaceTrust` do A2A — `live_member_ids`).
- O store: `ScrollbackStore::open_default(<ws>/.lina)` (mesmo lar do `scrollback.db` da F1-5-2).

## Saída (estável, sem ANSI)

- **tail/search:** JSON por linha de resposta (uma `HistoryPage`/`SearchPage` serializada) —
  campos `panel/start/lines/next_cursor/expired_before/expired` (`hits` no search). O chamador
  pagina re-chamando com `--offset`/`--cursor` = `next_cursor` devolvido.
- **export json:** o próprio `HistoryPage` (round-trip provado no teste d).
- **export txt:** linhas cruas separadas por `\n`.
- **Janela expirada NÃO é erro:** resposta `expired: true` + `lines: []` → o verbo imprime
  `historico expirado (retencao de N dias)` e sai 0. Só erro real (regex, IO, negado) sai ≠0.

## Semântica de paginação (decisões já tomadas — não re-decidir na fiação)

- `tail`: `offset` conta do FIM (0 = cauda). `next_cursor` é o PRÓXIMO offset (mais antigo).
- `search`: `cursor` é índice GLOBAL de linha; varre no máx. `max_scan` linhas por chamada
  mesmo sem hit (o cursor avança pela região varrida — busca em log gigante converge).
- Pedir 10^9 → clampa no teto e segue por cursor (teste b).
- Cursor abaixo do piso de expiração é consumido pelo piso (nunca erro).

## Segurança (inegociável)

- O caminho cross NUNCA chama `tail`/`search`/`export` puros: SEMPRE a variante `*_cross`
  correspondente (auditoria é gate, não telemetria — falha ao auditar NEGA a leitura). O
  `export` é a leitura de MAIOR volume (um bloco inteiro) → a que MAIS precisa de rastro; não
  há exceção para ele.
- `query` no evento de auditoria é o TIPO ("tail"/"search"), nunca o conteúdo do payload.
- Identidade do leitor vem do env injetado pelo app (ADR 0026), jamais de flag/arquivo
  escrito por agente (doutrina: campo de agente não decide autorização).
