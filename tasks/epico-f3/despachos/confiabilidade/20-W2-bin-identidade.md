# Despacho · W2 BIN — "entregue de fato" + resolução de nó vivo (#22/#23, #4/#14, #17)
**Para:** Terminal H · **model·effort:** opus · Medium · **Dono de:** `crates/lina-bootstrap/src/bin/lina.rs`

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (absoluto).
- **LEIA primeiro:** `tasks/epico-f3/rodada-confiabilidade-orquestracao.md` (diagnóstico §1 e correlatos).
- **O falso-entregue (impacto imediato):** `scan_log_outcome` (`bin/lina.rs:625`) trata **`MessageRouted` OU `MessageDelivered`** como `Delivered`, e "sucesso vence bloqueio" (`:636-643`). Mas `MessageRouted` é gravado ANTES da injeção física (`router.rs:1168`) — então o agente vê "entregue" mesmo quando nada foi injetado. É metade do achado #22c (B reportou PRONTO de arquivo inexistente; o report mentiu).
- **Resolução de identidade:** `resolve_check_node` (`lina.rs:454`, usado por `run_check` `:521`) prefere homônimo VIVO (`:492`) mas: "sem status registrado = vivo" (`:491`) e a ordem de recência pode favorecer um homônimo MORTO batizado depois (#4/#14/#23c — `lina check` reporta lifecycle de sessão antiga). `from` vem de `load_identity()` (`:134`, env-aware via `LINA_NODE_NAME` `:112`), mas o override de env é só para `terminal_name` (`:136`) — demais campos (ex.: autonomia em `run_spawn:858`) ainda vêm do `bootstrap.json` do cwd compartilhado (#17 residual).

## FUNÇÃO
Você é o **dono do bin** no que toca confiabilidade: faz o report de entrega só afirmar "entregue" quando a injeção REALMENTE ocorreu, e faz `lina check` nunca apontar para um nó morto de sessão antiga.

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `crates/lina-bootstrap/src/bin/lina.rs`. A EMISSÃO de `MessageDelivered` (quando o router considera entregue) é de B/W1 — você cuida do CONSUMO (o report do remetente). Precisa que `MessageDelivered` signifique algo específico? Combine com B via Maestro.
- **Identidade é autoridade — cuidado:** `from`/`by` server-side, ADR 0026. Não invente: a fonte da identidade é o env `LINA_NODE_ID/NAME`, não o `bootstrap.json` do cwd compartilhado. Para #17, AUDITE e feche (ou registre porta com o Maestro) os campos além do nome que ainda dependem do arquivo do cwd.
- Convenções: `cargo fmt -p`, `clippy -D` 0, sem `unwrap()` em produção, teste que prova o critério.

## OBJETIVO (o porquê de negócio)
Quando o Maestro despacha e o sistema diz "entregue", isso TEM que ser verdade — senão o orquestrador segue cego, marcando como feito o que nunca começou (#22c, impacto em cliente real). E `lina check` tem que olhar para o terminal VIVO, não para o fantasma de uma sessão anterior.

## ESCOPO
1. **Report honesto:** `scan_log_outcome`/`poll_route_outcome` passam a exigir **`MessageDelivered`** (não `MessageRouted`) para reportar "entregue". `MessageRouted` vira no máximo "roteada, aguardando entrega". Atualize a mensagem ao agente (sem jargão cru).
2. **`resolve_check_node` anti-nó-morto:** corrija a resolução para preferir o nó VIVO de forma robusta (status ausente não é "vivo" por default quando há homônimo com status; o mais recente VIVO vence). `lina check`/`list` devem concordar.
3. **#17 residual:** audite os campos derivados do `bootstrap.json` do cwd além do `terminal_name`; feche o que for identidade/autonomia (env-first) ou registre porta com o Maestro.

## RESULTADO ESPERADO (formato exato)
- Diff em `lina.rs`; testes provando: report só diz "entregue" com `MessageDelivered`; `resolve_check_node` recusa nó morto homônimo; #17 fechado/registrado.
- `cargo test --manifest-path crates/lina-bootstrap/Cargo.toml` (ou `-p lina-bootstrap`) verde; `clippy -D` 0; `fmt` limpo.
- **NÃO commite.** Reporte o 1º progresso (`lina ask "@Terminal A" "comecei W2 (bin/identidade)" --intent status`).
- Termine com **`PRONTO: <o que mudou + testes>`** ou **`BLOCKED: <motivo>`**.
