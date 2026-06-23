# DESPACHO F40-TOOLSCOPE — F4-0-4 · Pré-config de ferramentas/grupos por projeto (dono: Terminal M)

## CONTEXTO (leia ANTES de codar — obrigatório)
Onda **F4-0** da Fase 4 do Lina. Esta frente entrega o ponto **[6] Fluxo de informação** de Meadows: o leigo DECLARA, por projeto, quais ferramentas/grupos a IA pode enxergar — isso vira **contexto declarado** que a IA lê. A regra-mãe inviolável: **declarar ≠ autorizar**. Declarar expõe à LEITURA; não autoriza ação externa (essa segue pelo broker + gate humano, F4-0-3).

**Documentos a conferir (navegue — NÃO pule):**
1. **Peça da onda:** `tasks/epico-f4/onda-0.md` — seção **F4-0-4**.
2. **Épico no vault:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — critério **F4-0-4** (§III) + §IV (eventos `ToolScopeDeclared`/`ToolScopeRevoked`) + a "Regra de fronteira" do ponto [6] no §I (mostrar ≠ autorizar).
3. **Molde direto (prima das pistas):** `crates/lina-core/src/clue.rs` — F3-5-6 fez EXATAMENTE este padrão (projeção por replay, "remover = redefinir vazio", `paths` é dado não autoridade). Copie a estrutura.
4. **Stub com o norte:** `crates/lina-core/src/tool_scope.rs` (header já escrito por mim).

## FUNÇÃO
Você é o **Dev Core (contexto)** desta frente. O `tool_scope.rs` é gêmeo do `clue.rs` — não invente um padrão novo; **espelhe o `ClueSet`** (mesma casa, mesma disciplina de projeção pura). A elegância aqui é a consistência com o que já existe.

## DIRECIONAMENTO (território + como trabalhar)
- **Território (SÓ estes):** `crates/lina-core/src/tool_scope.rs` (preencher o stub: projeção `ToolScopeSet` + `declare_tool_scope`/`revoke_tool_scope`) + testes inline no próprio arquivo.
- **Camada de briefing (costura de OUTRO dono — coordene):** para a IA "ver" a ferramenta declarada, uma camada do `crates/lina-core/src/briefing.rs` precisa injetar "você me deu acesso a <X>" no contexto. **`briefing.rs` tem dono histórico (Terminal I).** NÃO edite à toa: produza o **diff mapeado** (arquivo:linha + o que inserir) e **proponha ao Maestro** (`lina ask "@Maestro 00" ...`); o Maestro coordena com o dono de `briefing.rs`. (Lição registrada: fronteira em camadas — Camada 1 pura e verde já em `tool_scope.rs`; Camada 2 = diff proposto.)
- **NÃO TOQUE:** `events.rs`/`lib.rs` (congelados — `ToolScopeDeclared`/`ToolScopeRevoked { project, channel, scope }` já existem; `pub mod tool_scope;` já está).
- **Worktree:** `git worktree add ../lina-f4-0-toolscope -b lina/f4-0-toolscope` da `main` (`fcb9371`). `git`/`cargo` de lá; `lina` do dir de produção.
- **Entregue:**
  1. **projeção `ToolScopeSet`** — `from_records(&[EventRecord])` PURA: o último `ToolScopeDeclared` de um `(project, channel, scope)` vence; `ToolScopeRevoked` RETRAI a chave (o escopo some no replay seguinte — **sem variante de remoção extra**, igual ao `ClueSet`). Mapa por projeto → conjunto de (channel, scope) declarados.
  2. **`declare_tool_scope(...)`/`revoke_tool_scope(...)`** — emitem os eventos no `EventStore`.
  3. **default-deny:** ferramenta/grupo NÃO declarado é INVISÍVEL (a projeção não o contém).
- **Invariante (red-team re-deriva):** **declarar ≠ autorizar.** Nenhum campo de `ToolScopeDeclared` entra no caminho de identidade/permissão — só informa o que a IA OLHA. A ação externa continua passando pelo broker (F4-0-3). Documente isso no doc-comment.
- **Convenções:** `cargo fmt` + `clippy -p lina-core --all-targets -D warnings` limpos; sem `unwrap()` em produção; projeção pura (sem relógio/I/O); replay byte-a-byte.

## OBJETIVO (critério observável)
Declarar "grupo X do canal Y no projeto P" → a projeção `ToolScopeSet` do projeto P contém (Y, grupo X); **revogar** → some no próximo replay (sem restart). Um teste prova: declarar → projetar do log → presente; revogar → projetar → ausente. (A injeção no briefing fica como diff proposto ao Maestro se cruzar a fronteira de `briefing.rs`.)

## RESULTADO ESPERADO
- Não commite. Trabalho na worktree `lina/f4-0-toolscope`.
- Cole exit codes: `cargo test -p lina-core tool_scope`, `cargo clippy -p lina-core --all-targets` (exit 0), `cargo fmt --check`.
- Reporte: **`PRONTO: F40-TOOLSCOPE`** + resumo + (se houver) o diff proposto para `briefing.rs` — OU **`BLOCKED: F40-TOOLSCOPE`** + o quê. Via `lina ask "@Maestro 00" "<...>" --intent status`.
