# Despacho M-UI · Painel "Como o [papel] pensa" — Terminal G (opus · medium)

> Rodada **F3-3 Mentality**. Maestro desta rodada: **Terminal A** (reporte a ele; o Terminal B é worker da M-DETECTOR).
> ⛔ **NÃO INICIE** até o Maestro avisar: (a) "contrato commitado" E (b) "main.rs liberado pelo Terminal A" (você fia o painel em main.rs, costura que o A edita).
> Marcador OBRIGATÓRIO: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES:**
1. `tasks/epico-f3/onda-f3-3.md` — o plano (fronteira, gate (h) — fundador na tela).
2. **Vault (design fechado):** `35 - Proposta F3 - Mentalidade por Papel` §4.5 (Painel "Como o [papel] pensa": proveniência humanizada + aposentar 1-clique) + §5 (o critério binário que o painel ajuda a validar) + invariante #6 (zero jargão; não-técnico-first).
3. **Skills de design:** carregue `lina-design-doctrine` e `senior-frontend` — esta é UI para LEIGO; bana o slop (Inter default, gradiente roxo, glassmorphism genérico); siga a identidade visual JÁ estabelecida no app (não invente uma nova — reuse o design system da F2).
4. **O molde a espelhar:** os painéis/cards existentes do app (`grep -rnE 'fn render|Card|panel|dashboard' app/lina-gpui/src/*.rs | head` — confirme onde os painéis vivem; o card da Goal da F3-1 é o irmão mais próximo). A projeção de leitura vem de `crates/lina-core/src/mentality.rs` (M-PROMO/I).

## 2. FUNÇÃO

Você é o dono da **superfície humana da Mentality**: o painel onde o usuário leigo VÊ como cada papel da equipe "pensa" — em linguagem dele, com a origem de cada crença, e o poder de aposentar uma crença com 1 clique (o humano é o árbitro final).

## 3. DIRECIONAMENTO

- **Fronteira:** um arquivo/componente NOVO no app (ex.: `app/lina-gpui/src/mentality_panel.rs`) + a fiação mínima em `main.rs` (abrir o painel). **NÃO toque:** core (`events.rs`/`mentality.rs` — só CONSOME a projeção via a bridge), `bridge.rs` (J), `a2a.rs` (H). A fiação em `main.rs` é o ponto de costura — minimize e coordene com o Maestro (o A acabou de liberar; cuidado para não reverter linha dele).
- **Render da MESMA projeção em pt-br (spec 35 §4.5; inv #6):** proveniência humanizada — "aprendido em 10/06, quando você corrigiu X" (NÃO `belief_id`/hash/`CorrectionObserved` na tela). Estabelecidas como o que o papel "já sabe"; provisórias marcadas como "ainda testando — confirme ou corrija". **Botão aposentar** (1 clique → dispara `BeliefRetired`): some da próxima injeção daquele papel. Humano é o árbitro.
- **Zero jargão (regra dura):** nada de `ReviewVerdict`/`belief_id`/`hash-de-situação`/`top-K`/`TTL` na superfície. Use a voz da a11y (a mesma do `aggregate_badge`/status pt-br), não `{:?}` de Debug (vaza jargão que WCAG/ratchet não pegam — ver memória "tela honesta: Debug-leak").
- **Para a UI disparar `BeliefRetired` no core:** use o canal view→core já existente (fila no `NodeManager`, Arc compartilhado view↔pump — ver memória "UI gpui: canal view→core via NodeManager"); **NÃO** toque `MailboxPump::new`.
- Convenções: `cargo fmt --manifest-path app/lina-gpui/Cargo.toml` (só o app); `clippy --all-targets -D warnings` 0; o **token_ratchet** conta `FontWeight::`/`px(<lit>)` até em comentário — use os tokens semânticos do design system, não literais; zero `unwrap()` em produção. **Você NÃO commita.**

## 4. OBJETIVO

Dar ao fundador (leigo) a janela para o aprendizado da equipe: ele VÊ o que cada papel aprendeu com ele, de onde veio, e mantém o controle (aposentar). É o que torna o auto-aprimoramento confiável — humano no comando, transparência total, zero magia opaca.

## 5. RESULTADO ESPERADO

- O painel "Como o [papel] pensa" renderizando a projeção em pt-br (proveniência humanizada + aposentar 1-clique → `BeliefRetired`).
- Prova local (exits limpos): `cargo build --manifest-path app/lina-gpui/Cargo.toml 2>&1 | tail` + exit; `cargo test --manifest-path app/lina-gpui/Cargo.toml 2>&1 | tail`; `clippy --all-targets -D warnings`; confirme o token_ratchet intacto. **gpui não roda headless** — descreva o que o fundador deve ver para validar (gate h).
- Reporte ao Maestro com **`PRONTO: M-UI — painel "Como o papel pensa" (proveniência humanizada + aposentar 1-clique→BeliefRetired), zero jargão, token_ratchet intacto`** ou **`BLOCKED: <motivo>`**.

> Estética: este painel é a cara do diferencial da Lina (auto-aprimoramento humano-no-comando). Se ficar com cara de dashboard genérico de IA, refaça. Direção declarada > default. Mas a identidade visual é a do app (F2) — você SEGUE, não cria uma nova.
