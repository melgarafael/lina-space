# DESPACHO F40-CREDUI — F4-0-2 (UI) + F4-0-5 (badge) · Modal de credencial + sinalização (dono: Especialista em Telas)

## CONTEXTO (leia ANTES de codar — obrigatório)
Onda **F4-0** da Fase 4 do Lina. Duas peças de tela:
- **Área de credenciais (F4-0-2 UI):** o leigo sobe uma credencial de canal por um **modal no canvas** (não curses) → ela vai para o keyring (via o contrato core de F40-CRED) → some da vista. Zero jargão.
- **Sinalização de exposição (F4-0-5 badge):** enquanto ≥1 canal externo está ativo, a UI mostra um **badge persistente** "este Espaço fala com o mundo" (cor = feedback subconsciente, doc 40 §10). Diz QUAL canal, não um genérico. 0 canais → 0 badge.

**Documentos a conferir (navegue — NÃO pule):**
1. **Peça da onda:** `tasks/epico-f4/onda-0.md` — seção **F4-0-2 (UI) + F4-0-5 (badge)**.
2. **Épico no vault:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — critérios **F4-0-2** e **F4-0-5** (§III) + invariante #2 (exposição opt-in sinalizado) + #6 (não-técnico-first).
3. **Doutrina de design (CARREGUE a skill):** invoque `lina-design-doctrine` antes de estilizar — bane slop visual (Inter default, gradiente roxo de IA, glassmorphism genérico), exige direção estética declarada + tokens semânticos. E `lina-design-doctrine` da casa: respeite a identidade já estabelecida do app (não invente paleta nova; siga o design system existente do `app/lina-gpui`).
4. **Padrão de modal existente (copie a estrutura):** `app/lina-gpui/src/agent_modal.rs` (modal headless + render fina) + `app/lina-gpui/src/ui/modal.rs` (componente reusável). Fiação no `main.rs`: campo `Option<Modal>` na view, abrir via `self.<modal> = Some(...)`, commit aplica plano ao `Arc<NodeManager>` (veja `agent_modal` em `main.rs:533/2814/3024`).

## FUNÇÃO
Você é o **Frontend** desta frente e o **dono ÚNICO de `app/lina-gpui/src/main.rs` nesta rodada** (toda fiação de UI da onda passa por você — evita colisão de costura). Você consome o contrato core de F40-CRED (Terminal I) e o estado de "canal ativo" de F40-CHAN/BROKER. A tela tem que ser honesta e bonita — se um staff designer não assinaria, refaça.

## DIRECIONAMENTO (território + como trabalhar)
- **Território:** `app/lina-gpui/src/credential_modal.rs` (NOVO, modal headless no padrão `agent_modal.rs`) + um componente de badge de exposição (`app/lina-gpui/src/` — arquivo novo ou no header existente) + a fiação em `app/lina-gpui/src/main.rs` (abrir modal, commitar para o core, slot do badge no header).
- **IMPORTANTE — o app é EXCLUÍDO do workspace:** rode build/test/clippy de DENTRO de `app/lina-gpui` (`cargo build --manifest-path app/lina-gpui/Cargo.toml` ou `cd app/lina-gpui && cargo ...`). `cargo fmt --check` do app vem vermelho por dívida PRÉ-EXISTENTE de peer — **formate SÓ seus trechos** (prove com `git diff`), não toque arquivo alheio. O `token_ratchet` (catraca de tokens de UI) deve ficar INTACTO.
- **NÃO TOQUE:** `events.rs`/`lib.rs` do core (congelados). O contrato de credencial (helper `set_channel_credential` + emissão de `CredentialStored`) é de F40-CRED (I) — você o CHAMA, não o reimplementa.
- **Worktree:** `git worktree add ../lina-f4-0-credui -b lina/f4-0-credui` da `main` (`fcb9371`). `git`/`cargo` de lá; `lina` do dir de produção.
- **Dependência (trabalhe contra o contrato, integre no fim):** a API core de F40-CRED (Terminal I). Comece o modal contra a assinatura esperada (`set_channel_credential(channel, scope, key, value)`); quando I publicar (o Maestro avisa), integre o caminho real. O badge lê o estado de canal ativo (de `ChannelRegistry`, F40-CHAN/B). Reporte BLOCKED só se travar de verdade.
- **Entregue:**
  1. **modal de upload de credencial:** campos (canal, escopo/chave, valor mascarado) → ao confirmar, chama o contrato core → keyring → fecha. O valor NUNCA é logado nem exibido após salvar.
  2. **badge de exposição:** 0 canais ativos → sem badge; ≥1 canal ativo → badge "este Espaço fala com o mundo" + QUAL canal, cor de alerta sóbria (não decorativa). Subordinado a `prefers-reduced-motion` se houver animação.
  3. zero jargão na superfície (sem "keyring", "PTY", "scope" cru — fale humano).
- **Convenções:** sem `unwrap()` em produção; `clippy` do app limpo nos seus arquivos.

## OBJETIVO (critério observável — gate de tela do fundador, BLOQUEANTE)
Abrir o modal → digitar uma credencial → ela some no keyring (via core) e não reaparece na tela; 0 canais → 0 badge; registrar/conectar um canal-stub → badge aparece dizendo qual. `token_ratchet` intacto; build do app verde. (A validação visual final é captura do fundador — você entrega a tela funcionando e o Maestro coordena o gate de tela.)

## RESULTADO ESPERADO
- Não commite. Trabalho na worktree `lina/f4-0-credui`.
- Cole exit codes: `cd app/lina-gpui && cargo build`, `cargo clippy` (exit 0), `cargo test` (token_ratchet + seus testes). Mostre `git diff --stat` provando que só tocou seus arquivos.
- Reporte: **`PRONTO: F40-CREDUI`** + resumo + (se possível) um print/descrição da tela — OU **`BLOCKED: F40-CREDUI`** + o quê (ex.: esperando contrato de CRED). Via `lina ask "@Maestro 00" "<...>" --intent status`.
