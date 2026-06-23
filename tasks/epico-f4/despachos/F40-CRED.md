# DESPACHO F40-CRED — F4-0-2 (core) · Credencial → keyring + TESTE DE DUMP DE ENV (dono: Terminal I)

## CONTEXTO (leia ANTES de codar — obrigatório)
Onda **F4-0** da Fase 4 do Lina. A fronteira inegociável: **a credencial de um canal vive no keyring do Lina (`lina-secrets`), NUNCA no env do PTY do agente**. Sem o segredo, o agente não consegue disparar a ação externa sozinho — por construção (ADR 0004 custódia como gate duro real). Esta frente entrega o **critério inforjável** que prova isso: um dump de env de PTY de agente spawnado que NÃO contém a credencial.

**Documentos a conferir (navegue — NÃO pule):**
1. **Peça da onda:** `tasks/epico-f4/onda-0.md` — seção **F4-0-2 (core)** + "Estado de partida" (o gap crítico: `lina-pty` NÃO faz `env_clear`; a defesa atual é só denylist no env do app; NÃO existe teste de dump de env do filho).
2. **Épico no vault:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — critério **F4-0-2** (§III) + §IV (evento `CredentialStored`) + §V risco #1 (credencial vazar para o env é o risco que mata a fase).
3. **ADR 0004** (`docs/adr/0004-custodia-de-segredo-como-gate-duro.md`) — leia AC-0004.1 (o dump de env é o espelho deste critério).
4. **spec 53 §credencial:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/53 - SPEC Buffers Geridos - Sessoes Disco e Estoques"` (seção credencial-como-segredo).

## FUNÇÃO
Você é o **Dev Core (segurança/custódia)** desta frente. O coração não é "guardar credencial" (o `SecretVault` já faz isso) — é **PROVAR, por inspeção do processo-filho real, que a credencial não vaza para o env de nenhum PTY**. Esse teste é o que um auditor de segurança exige ver. Faça-o à prova de mutação.

## DIRECIONAMENTO (território + como trabalhar)
- **Território (SÓ estes):** `crates/lina-secrets/src/lib.rs` (adicione helper de credencial-de-canal: `set_channel_credential`/`get`/`revoke` sobre um scope `channel:<nome>` — reuse o padrão scope/key existente; veja `KeyringStore`/`MockStore`) + binding que emite `CredentialStored`/`CredentialRevoked` (proponha o ponto de fiação ao Maestro; provavelmente um helper em `crates/lina-core/` que chama `vault.set` + `store.append(CredentialStored{..})`) + **teste headless NOVO** `crates/lina-core/tests/f4_0_cred_env_dump.rs`.
- **NÃO TOQUE:** `events.rs`/`lib.rs` (congelados — `CredentialStored{channel,scope,key_ref}` e `CredentialRevoked{channel,key_ref}` já existem). Não toque `app/lina-gpui/main.rs` (a UI é a frente CRED-UI do Especialista). Não toque o `scrub`/denylist do `bridge.rs` sem coordenar (é defesa existente; você PROVA que funciona, e estende se achar buraco — avise o Maestro antes de mexer no `bridge.rs`).
- **Worktree:** `git worktree add ../lina-f4-0-cred -b lina/f4-0-cred` da `main` (commit `fcb9371`). `git`/`cargo` de lá; `lina` do dir de produção.
- **O TESTE DE DUMP DE ENV (entrega-coração — gate (a)):**
  1. Guarde uma credencial-de-canal de teste no `SecretVault` (use `MockStore` para headless, OU `KeyringStore` se rodar local — o teste deve passar headless/CI).
  2. **Spawne um PTY de agente** (reuse o harness de `crates/lina-core/examples/permission_probe.rs` e/ou `lina-pty` `PtyManager::spawn` — o programa do PTY pode ser `sh -c 'env'` ou `printenv`).
  3. **Capture o env real do processo-filho** (a saída de `env`/`printenv` no PTY) e **grepe a string da credencial → asserte 0 hits**. Asserte também que a credencial não aparece em nenhuma `LINA_*`.
  4. **Prova por mutação (padrão-ouro):** mostre que se a credencial FOSSE injetada no env (caminho proibido), o teste PEGARIA (um sub-teste que injeta de propósito e vê >0 hits, provando que o grep funciona). Isso blinda contra falso-verde.
- **`CredentialStored` no log:** o evento carrega `key_ref` (REFERÊNCIA), **o valor NUNCA entra no log** — asserte que o payload não contém o valor da credencial. Backup silencioso `.bak` antes de sobrescrever (Hermes doc 40 §3): se já existe credencial para o `(channel,scope,key)`, registre/preserve a anterior antes de gravar.
- **Sem TTY/CI:** o caminho non-interactive não trava (guia em vez de pendurar).
- **Convenções:** `cargo fmt` + `clippy -p lina-secrets`/`-p lina-core --all-targets -D warnings` limpos; sem `unwrap()` em produção (use `expect` só em testes).
- **Proposta de produção:** o `SecretVault` é instanciado hoje em `app/lina-gpui/src/runtime.rs:507/510` com `MockStore` (produção `KeyringStore` comentada). Proponha ao Maestro a virada para `KeyringStore::new("lina-space/<ws>")` — mas NÃO edite `runtime.rs` sem o ok (é fiação de app, coordenar com CRED-UI).

## OBJETIVO (critério observável)
Subir uma credencial-de-canal → `vault.get(scope, key)` a retorna E o **dump de env do PTY de agente spawnado = 0 hits da credencial** (provado por inspeção do processo-filho real, com mutação que prova que o grep pegaria um vazamento) + `CredentialStored{channel,scope,key_ref}` no log com o **valor ausente**.

## RESULTADO ESPERADO
- Não commite. Trabalho na worktree `lina/f4-0-cred`.
- Cole exit codes: `cargo test -p lina-core --test f4_0_cred_env_dump`, `cargo test -p lina-secrets`, `cargo clippy --all-targets` (exit 0).
- Reporte: **`PRONTO: F40-CRED`** + resumo (com o trecho do dump mostrando 0 hits) + arquivos — OU **`BLOCKED: F40-CRED`** + o quê. Via `lina ask "@Maestro 00" "<...>" --intent status`.
