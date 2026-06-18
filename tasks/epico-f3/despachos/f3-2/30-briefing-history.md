# Despacho F3-2 · BRIEFING + observabilidade do Maestro (F3-2-6 + achado #15)
**Para:** Terminal I · **model·effort:** opus · Medium · **Dono de:** `crates/lina-core/src/briefing.rs` (novo) + `crates/lina-bootstrap/src/bin/lina.rs` (verbo `history`)

## CONTEXTO (puxe antes de codar)
- **cwd do repo:** `/Users/rafaelmelgaco/einstein workspace/lina-space` (caminho absoluto).
- **LEIA primeiro:**
  1. `tasks/epico-f3/onda-f3-2.md` §gap (itens 5 e 6) + §gate (e) e (f).
  2. Hermes §6.1 (briefing em camadas): `/Users/rafaelmelgaco/Documents/Obsidian Vault/Ecossistema Labs - Operação/Debriefing Vibe Coding/10 - Arquitetura e Runtime do Agente.md` §6.1/§6.2 (worker vs orquestrador; stable/context/volatile + verify-loop no boot).
  3. O que existe hoje (NÃO re-fazer): bootstrap de turno-0 `whoami_with_roles` em `crates/lina-bootstrap/src/lib.rs:448` (bloco `=== Lina Space BOOTSTRAP ===`); **não há** módulo de briefing em camadas (grep `briefing|stable-context|volatile` no core = vazio).
  4. Para o #15: `crates/lina-core/src/history.rs` (API de leitura cross-terminal JÁ existe — `HistoryRead` em `events.rs:757`, leitura por pertencimento ADR 0006 em `lib.rs:168`), mas `lina history` **NÃO está no `lina --help`** (confirmado: o verbo não é exposto no bin).

## FUNÇÃO
Você é o dono de **como o worker é briefado** (módulo puro de camadas) e de **como o Maestro vê a tela do colega** (verbo de leitura cross-terminal). Duas fatias disjuntas do core, ambas [6] Fluxo de informação.

## DIRECIONAMENTO (regras do jogo)
- **Mexa SÓ em** `crates/lina-core/src/briefing.rs` (crie o módulo + registre em `lib.rs` — peça ao Maestro se `lib.rs` for costura disputada) e `crates/lina-bootstrap/src/bin/lina.rs` (verbo `history`, consumindo a API de `history.rs` que já existe — **não reescreva `history.rs`**).
- **Briefing = função PURA** (`fn build_briefing(...) -> String` ou struct + render): recebe camadas e papel, devolve texto. **Gated por papel** (FRONTEND não recebe protocolo de orquestração). É texto acima do CLI (inv #1) + projeção do log (inv #4) — nada de LLM. Como é `pub` de uma lib, função sem chamador interno **não** dispara dead-code (diferente do bin — memória `lina-gpui clippy -D bloqueia costura meia`). **Não fie no spawn real** nesta rodada (a costura de consumo em `router.rs` é do CORE/B — defira e deixe a função pronta + testada).
- **`lina history`:** exponha o verbo no bin enfileirando/consumindo a API existente; **leitura pura por pertencimento** (não injeta nada no colega — igual `lina check`). Atualize o `--help`. Sem inventar autoridade: quem pode ler o quê é a fronteira de pertencimento (ADR 0006), não um campo do payload.
- Convenções: `cargo fmt -p` (só seus crates), `clippy -D` limpo, sem `unwrap()` em produção, teste que prova o critério.

## OBJETIVO (o porquê de negócio)
(a) Cada terminal nasce sabendo seu papel, regras e estado sem um muro de contexto genérico — briefing certo por papel. (b) O Maestro consegue **monitorar trajeto sozinho** (passo 6 do loop): hoje ele é cego ao conteúdo do colega e tem que perguntar ao humano "o que apareceu na tela?" (achado #15) — inaceitável para orquestração autônoma.

## ESCOPO — 2 fatias
- **F3-2-6 (briefing em camadas):** módulo `briefing.rs` puro: **stable** (papel + invariantes + verbos/skills) → **context** (plan.md/Decisões + arquivos do projeto) → **volatile** (estado vivo + timestamp date-only). Guias gated por papel. Verify-loop no boot (o terminal já sabe rodar `cargo test -p`/clippy + checar git, com "re-check before acting"). Teste de unidade: camadas montadas; papel FRONTEND **não** recebe a camada de orquestração; K-cap se aplicável.
- **#15 (`lina history`):** verbo no bin que devolve o último estado/saída de um colega via a API de `history.rs`, leitura por pertencimento. Teste: ler o histórico de um nó pertencente devolve a saída; nó fora da fronteira é barrado.

## RESULTADO ESPERADO (formato exato)
- `briefing.rs` novo (função pura + testes de camada/gating) + verbo `history` no bin (+ `--help` atualizado + teste).
- `cargo test -p lina-core` e `cargo test -p lina-bootstrap` (ou via manifest) verdes — rode e cole a contagem.
- `clippy -D` 0; `fmt` limpo (só seus arquivos). **NÃO commite.**
- Reporte o 1º progresso (`lina ask "@Terminal A" "comecei briefing+history" --intent status`).
- Termine com **`PRONTO: <resumo + arquivos + testes>`** ou **`BLOCKED: <motivo>`**.
