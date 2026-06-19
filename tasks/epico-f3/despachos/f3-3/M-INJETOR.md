# Despacho M-INJETOR · Injetor de Mentalidade no spawn — Terminal J (opus · medium)

> Rodada **F3-3 Mentality**. Maestro: Terminal B (Ultra Code).
> ⛔ **NÃO INICIE** até o Maestro avisar: (a) "contrato commitado" E (b) "bridge.rs liberado pelo Terminal A". Você toca `bridge.rs`, costura que o A está editando (ADR 0037/0038).
> ⛔ Depende de **M-PROMO** (Terminal I): você consome a função de seleção top-K da projeção `Mentality(papel)`. Confirme com o Maestro que I já entregou a interface antes de fiar.
> Marcador OBRIGATÓRIO: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES:**
1. `tasks/epico-f3/onda-f3-3.md` — o plano (fronteira, invariante, gate (c) cap top-K).
2. **Vault (design fechado):** `35 - Proposta F3 - Mentalidade por Papel` §4.4 (Injetor no spawn — seção "Mentalidade do papel"; estabelecidas=regra, provisórias marcadas "hipótese em teste — confirme/conteste"; **cap top-K**) + §4.1 (fechamento do loop: a provisória instrui o agente a confirmar/contestar pela sentinela).
3. **ADR 0023** (`docs/adr/0023-reinjecao-de-doutrina-como-input-humano-proxy.md`): a doutrina é entregue como **input HUMANO-PROXY** pela fila serial de ESCRITA do supervisor (NÃO via A2A — o caminho A2A self é negado por pertencimento). Você ANEXA a seção de Mentalidade à doutrina renderizada, não cria um novo canal.
4. **O ponto exato (confirme no disparo, pós-A-liberar):** `app/lina-gpui/src/bridge.rs` — `doctrine_reinjection_text(role)` (~:140), `enqueue_doctrine_reinjection` (~:150), e o write da doutrina no spawn (`write_terminal_files`/`install`). Os `arquivo:linha` vão mudar com o commit do A — re-confirme com `grep -nE 'doctrine_reinjection_text|fn .*doutrina' app/lina-gpui/src/bridge.rs`.
5. A interface da projeção: `crates/lina-core/src/mentality.rs` (M-PROMO/I) — a função top-K que devolve as crenças do papel.

## 2. FUNÇÃO

Você é o dono da **ponta de SAÍDA do aprendizado**: anexar à doutrina renderizada de cada terminal a seção "Mentalidade do [papel]" — as crenças estabelecidas como regra, as provisórias marcadas como hipótese — respeitando o **cap top-K** para não inflar o contexto.

## 3. DIRECIONAMENTO

- **Fronteira:** `app/lina-gpui/src/bridge.rs` (a renderização/entrega de doutrina). **NÃO toque:** `events.rs` (Maestro), `mentality.rs` (I — só CONSOME a interface dele), `a2a.rs`/`router.rs` (H), `main.rs`/painel (G). Se a interface de I não existir ainda → **BLOCKED + avisa o Maestro** (não improvise a projeção).
- **Renderização (spec 35 §4.4):** anexe à doutrina do papel a seção "Mentalidade do papel" da projeção `Mentality(papel)`: **estabelecidas** entram como regras; **provisórias** marcadas *"hipótese em teste — confirme com `[LINA::BELIEF_CONFIRMED id]` ou conteste"* (fecha o loop com o M-DETECTOR). Texto acima do CLI (inv #1; multi-CLI de graça — é doutrina renderizada, inv #3).
- **Cap top-K (gate c — CRITÉRIO, não opção):** injete no máximo K crenças (use a seleção top-K de `mentality.rs` por recência/uso). K+1 crenças → só K injetadas. Sem cap, a mentalidade vira o próprio context rot. Teste: K+1 → exatamente K na saída renderizada.
- **Segurança (regra-mãe):** a seção de Mentalidade é doutrina COMPORTAMENTAL ("como pensar") — **nunca** instrução de autonomia/spawn/aprovação. Hierarquia na renderização: doutrina shipped vem ANTES e vence; crença estabelecida; provisória por último (marcada). A entrega segue ADR 0023 (human-proxy, fila de escrita) — você não abre canal A2A nem toca a autoridade.
- Convenções: edition 2021; `fmt` só do app (`cargo fmt --manifest-path app/lina-gpui/Cargo.toml`); `clippy --all-targets -D warnings` 0; o **token_ratchet** do app pode pegar literais — não escreva `FontWeight::`/`px(n)` em comentário; zero `unwrap()` em produção. **Você NÃO commita.**

## 4. OBJETIVO

Fechar a ponta de SAÍDA do aprendizado: cada novo terminal de um papel já NASCE sabendo o que aquele papel aprendeu com o usuário — sem repetir correções, sem inflar o contexto, e sem nunca deixar uma crença virar regra de autoridade.

## 5. RESULTADO ESPERADO

- A doutrina renderizada do spawn passa a incluir a seção "Mentalidade do [papel]" (estabelecidas como regra, provisórias marcadas), com cap top-K.
- Prova local (exits limpos): `cargo build --manifest-path app/lina-gpui/Cargo.toml 2>&1 | tail` + exit; `cargo test --manifest-path app/lina-gpui/Cargo.toml 2>&1 | tail` (sem --lib — pega bridge/admit); `clippy --all-targets -D warnings`. Teste: K+1 crenças → só K renderizadas; estabelecida vira regra, provisória vem marcada.
- Reporte ao Maestro com **`PRONTO: M-INJETOR — seção Mentalidade na doutrina (estab=regra/prov=hipótese), cap top-K provado, via human-proxy ADR 0023`** ou **`BLOCKED: <motivo>`**.

> Lembre: a crença é doutrina COMPORTAMENTAL. Se em algum momento você se pegar injetando algo que muda autonomia/spawn/aprovação, PARE — isso viola a regra-mãe (§6.1). Só "como pensar", nunca "o que pode fazer".
