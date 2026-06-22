# Despacho — Terminal R (QA/Red-team) · PROVAS eval-first + INTEGRADOR (dogfooding do DevOps)

> Entregue via `lina handoff "@Terminal R"`. Marcador obrigatório no fim: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`. Você é FOLHA da árvore (depende de A e B); pode começar as provas eval-first em paralelo, mas a integração espera A+B prontos.

## 1. CONTEXTO (onde o trabalho está — PUXE antes de codar)

- **LEIA primeiro:** vault `36 - Spec F3 - Coordenacao de Codigo Multi-Agente` (salvaguardas inegociáveis §3) + plano `tasks/epico-f3/onda-f3-4.md` (GATE DE SAÍDA a/b/c/d/e/f/g) + ADRs 0040/0041/0042. Você é o QA de quase toda a Fase 3 (eval-first + segurança por mutação) — mesmo rigor aqui.
- **As trilhas entregam em branches isoladas:** `lina/f3-4-a` (Terminal B — worktree/git/guard) e `lina/f3-4-b` (Terminal I — router/attention/plan/projeção). A fundação (`CodeChanged`/`BranchIntegrated` em `events.rs`) está na `develop`.
- **Mapa para as provas:**
  - Guard: `is_gated_hard` (guard.rs:225), `decide` (guard.rs:199, `GatedHard => Ask` em TODOS os níveis :208). Prove por MUTAÇÃO: desligue o ramo novo (reset --hard/branch -D) → o teste falha → religue (padrão ouro que o Maestro já validou).
  - Segurança do router: a suíte existente em `crates/lina-core/tests/` (route_message zero-mock). `author_node` de `CodeChanged` deve vir do binding server-side — prove que um envelope com `author_node`/`paths` forjado NÃO altera o carimbo nem concede autoridade.
  - Replay: projeções de `code.rs` (Trilha B) reconstroem byte-a-byte; molde de fixture = construa o `DomainEvent` real serializado (`kind()`+`to_value`), NÃO json à mão (lição: projeção via `from_record` engole record sem a tag `event`).
  - Integração: `git worktree`/`git merge`/`git branch`. A develop é a base de integração.

## 2. FUNÇÃO

Dois chapéus: (1) **QA/Red-team** — você escreve as provas eval-first RED→GREEN de cada critério do gate e a segurança por mutação, em `tests/` NOVOS (não conserte produto alheio; se um alvo precisa de fiação em arquivo de peer, escreva o teste-alvo `#[ignore]`, RODE `-- --ignored` p/ provar RED, e o DONO liga). (2) **Integrador (DevOps, dogfooding)** — você é quem junta `lina/f3-4-a` + `lina/f3-4-b` na `develop` seguindo a skill `lina-integration` (que o B escreve), provando o gate (c) ao vivo.

## 3. DIRECIONAMENTO (as regras do jogo)

- **Provas em `tests/` NOVOS** (`crates/lina-core/tests/f3_4_*.rs`, `app/lina-gpui` se precisar de gate de tela). NÃO edite produto das trilhas — se o alvo exige fiação alheia, `#[ignore]` + prove RED + sinalize o dono.
- **eval-first:** o teste nasce RED (no HEAD, antes do fix) e fica GREEN com a entrega da trilha. Rode `-- --ignored` para provar o RED dos que dependem de fiação.
- **Segurança por MUTAÇÃO (0 ALTA para o gate passar):** desligue a guarda → veja o teste falhar → religue. Cubra: (e1) `CodeChanged.author_node` server-side (forja no payload ignorada); (e2) `paths`×claims nunca concede posse/autoridade (só abre atenção); (e3) `push --force`/`reset --hard`/`branch -D` bloqueados em autonomia `Autonomous` (nunca afrouxa); (e4) broadcast por pertencimento respeita só vivos.
- **Integração SEM perder órfão:** ao juntar as branches na develop, se houver conflito não-trivial → **NÃO** decida sozinho silenciosamente: gate humano narrado (a skill manda). Trabalho que não entra vira item de atenção, **nunca** `branch -D`. Cada merge provado → `BranchIntegrated{branch, into:"develop", commit}` no log.
- **Validar de fora (exit codes diretos, sem pipe):** `cmd >log 2>&1; echo $?` (zsh: `$pipestatus[1]`, NÃO `${PIPESTATUS[0]}`). Não confie em auto-relato das trilhas — leia o disco/log.
- Sem `unwrap()` em prod; nos testes pode, mas prefira asserts claros. `rustfmt` só nos seus arquivos.

## 4. OBJETIVO (o porquê de negócio)

A onda inteira é sobre **não perder trabalho** e **não deixar duas IAs se pisarem**. Sua prova é o que garante que a salvaguarda não é teatro: que `push --force` é mesmo bloqueado, que um agente não rouba autoridade escrevendo um campo, e que juntar o trabalho do time não apaga o de ninguém. E a integração ao vivo é a demonstração de que tudo isso funciona de ponta a ponta.

## 5. RESULTADO ESPERADO (formato exato da entrega)

- **Provas RED→GREEN** dos critérios (a)(b)(c)(d) do gate + segurança por mutação (e1-e4), em `tests/` novos, com os comandos e exit codes que você observou.
- **Relatório de integração:** o log do merge de `lina/f3-4-a` + `lina/f3-4-b` na `develop` (limpo ou com gate humano narrado nos não-triviais), os `BranchIntegrated` apendados, e a confirmação de que nenhum trabalho virou lixo (órfão→atenção).
- **Veredito de segurança:** PASS/FAIL por critério + contagem ALTA/MEDIA/BAIXA (0 ALTA para o gate passar).

Termine com **`PRONTO: <provas verdes + relatório de integração + 0 ALTA>`** ou **`BLOCKED: <motivo + o que precisa do Maestro/peer>`**.
