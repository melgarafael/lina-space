# DESPACHO r4-eol-a11y — Frontend (Terminal C)
**id:** `f1-eol-a11y` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Duas pendências NOMEADAS do gate de saída do épico F1, ambas no shell gpui. Rodada r4 (saída F1).

## CONTEXTO
- `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"` (HEAD `f1d4810`).
- **Item A — badge EoL do Gemini (render):** o conselho F1-1 deixou nomeado: *"Render do badge EoL Gemini (copy aprovada; falta o render)"* — `tasks/epico-f1/conselho-f11-consolidado.md:22,27`. A VERDADE do EoL vive no perfil: `profiles/gemini.toml:1` (`TRANSITIONAL / EoL 2026-06-18`) e a projeção honesta em `crates/lina-cli-profiles/src/lib.rs:1385-1403`. O render entra onde o leigo ESCOLHE o CLI: modal de criação de agente (`app/lina-gpui/src/agent_modal.rs` — grep `Gemini`) e, se houver listagem por-CLI, no dashboard (`dashboard.rs`). Critério da tela do fundador: `tasks/epico-f1/roteiro-tela-consolidado.md:50-52` ("Ao criar um agente Gemini, VEJA o aviso de fim-de-vida").
- **Item B — a11y do M9 (announcement → live-region):** refino que o Terminal #41 deixou pendente na entrega do modal M9: `.entrega-m8-t5.md:47,65-67` (raiz do repo) — o `CreateSpaceModal` (em `gallery.rs`) JÁ expõe `announcement() -> Option<String>`; falta FIAR ao live-region após cada movimento de foco/validação. A fiação do modal vive em `main.rs` (abre em `open_create_space_modal` ~856; teclado ~893-948; live-region: campo `a11y_live` em 502, observe em 2878-2879, elemento em 3796). Inclui o critério da spec: `tasks/epico-f1/spec-m8-m9-fiacao.md:117` — toast de arquivamento anunciado via live-region; `[ Desfazer ]` alcançável por teclado antes do timeout.

## FUNÇÃO
Você é o dono do shell gpui (frontend) nesta rodada — único worker tocando `app/lina-gpui` agora.

## DIRECIONAMENTO
- Fronteira: `app/lina-gpui/src/{agent_modal.rs, gallery.rs, sidebar.rs, dashboard.rs, a11y.rs, a11y_live.rs}` + **hooks mínimos** em `main.rs` (só a fiação announcement→live-region e o que o badge exigir; main.rs é costura — toque cirúrgico).
- **NÃO toque:** `runtime.rs`, `bridge.rs`, `wiring.rs`, crates do workspace raiz.
- Copy do badge: leiga, pt-br, honesta, derivada do que o `gemini.toml` declara (data 2026-06-18; sucessor Antigravity — ver `profiles/antigravity.toml:7`). Zero jargão ("EoL" é proibido NA TELA — diga "deixa de ser atualizado em 18/jun" ou equivalente). O badge NÃO bloqueia a escolha — informa.
- Banimentos de design da doutrina valem (nada de boilerplate visual; siga os tokens do design system existente em `theme.rs`).
- Testes headless que provam: (a) ao selecionar Gemini no modal, o aviso está presente no estado/render-tree; ao selecionar Claude, ausente (não-vacuoso); (b) announcement do M9 chega ao live-region após `focus_next()`/validação; (c) toast de arquivar anuncia e `[Desfazer]` é alcançável por teclado.

## OBJETIVO
Fechar 2 pendências do checklist de saída do F1 que dependem de render: o leigo que escolher Gemini precisa saber HOJE que ele morre em 18/jun (honestidade, inv#6), e o M9 precisa ser audível por leitor de tela (W4-6/F1-2-7 — o spike já provou que o macOS anuncia `value()`).

## RESULTADO ESPERADO
`tasks/epico-f1/.entrega-f1-eol-a11y.md` (modelo das regras comuns): arquivo:linha das mudanças, testes novos + suíte do app inteira verde (`cd app/lina-gpui && cargo test -- --test-threads=1`), clippy/fmt por-pacote com exit DIRETO. Marcador `.iniciado-f1-eol-a11y` no primeiro ato. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
