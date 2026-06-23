# DESPACHO F40-MONITOR — F4-0-5 (monitor de rede) + F4-0-6 (catálogo/CI) (dono: Terminal G)

## CONTEXTO (leia ANTES de codar — obrigatório)
Onda **F4-0** da Fase 4 do Lina. Dois critérios do gate de saída vivem aqui:
- **(d) prova local-first por AUSÊNCIA:** enquanto 0 canais ativos, **zero conexão de saída atribuível ao Lina**. O Lina não abre socket externo a menos que um canal esteja conectado.
- **(e) curadoria = segurança para o leigo:** catálogo com trust tiers (core/curado/comunidade); manifesto de canal malformado **barra o merge** na CI (estar no catálogo = aprovado por PR — doc 40 §6/§11).

**Documentos a conferir (navegue — NÃO pule):**
1. **Peça da onda:** `tasks/epico-f4/onda-0.md` — seção **F4-0-5 (monitor) + F4-0-6**.
2. **Épico no vault:** `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/41 - Epico Fase 4 — Integracoes Canais e Contexto"` — critérios **F4-0-5** e **F4-0-6** (§III) + invariante #2 (exposição opt-in sinalizado).
3. **Doc 40 (Hermes)** §6/§11 (trust tiers, PR-as-gate, sem marketplace cru) e §10 (cor = feedback): `lina vault read "Ecossistema Labs - Operação/Debriefing Vibe Coding/40 - Skills MCP CLI e Usabilidade"`.
4. **Molde de bin read-only:** `crates/lina-core/src/bin/plan_adoption.rs` e `intelligence_adoption.rs` — copie a estrutura (leitor offline do `log.jsonl`, função pura, `--json`, ZERO mutação, auto-descoberto em `src/bin/`, sem editar lib.rs/Cargo.toml).
5. **CI atual:** `.github/workflows/ci.yml` (você ADICIONA um passo de validação de manifesto).

## FUNÇÃO
Você é o **Dev Core (observabilidade)** desta frente. O "monitor de rede" não é um sniffer — é uma **afirmação honesta e auditável**: o Lina projeta quais canais estão ativos (do log) e declara o estado de exposição. A honestidade é o produto (doc 40 §2): a saída diz a VERDADE, sem inflar. Headless — você não toca a UI (o badge visual é da frente CRED-UI; você entrega o DADO que ela e o auditor consomem).

## DIRECIONAMENTO (território + como trabalhar)
- **Território (SÓ estes):** `crates/lina-core/src/bin/network_monitor.rs` (NOVO, read-only) + `.github/workflows/ci.yml` (passo novo de validação de manifesto) + um teste de schema de manifesto em `crates/lina-core/tests/` (NOVO). **Headless — NÃO toque `app/lina-gpui/main.rs`.**
- **NÃO TOQUE:** `events.rs`/`lib.rs` (congelados). O catálogo reusa o campo `trust_tier` do `ChannelRegistered` (já existe).
- **Worktree:** `git worktree add ../lina-f4-0-monitor -b lina/f4-0-monitor` da `main` (`fcb9371`). `git`/`cargo` de lá; `lina` do dir de produção.
- **Entregue:**
  1. **`network_monitor.rs`** — bin read-only que varre o `log.jsonl`, projeta os canais **ativos** (registrados via `ChannelRegistered` E conectados — como `ChannelConnected` é F4-1+, aqui o estado é "declarado, não conectado" ⇒ **0 canais ativos**) e imprime um relatório honesto: "N canais declarados, 0 conectados ⇒ 0 conexão de saída esperada". `--json` para o gate consumir. Documente no doc-comment a verificação externa que o fundador/QA roda: `nettop -P -l 1 | grep -i lina` ou `lsof -i -a -p <pid-lina>` = 0 conexões de saída enquanto 0 canais conectados.
  2. **Catálogo (F4-0-6):** uma estrutura/função (pode ser no mesmo bin ou um módulo pequeno — se precisar de módulo novo, peça ao Maestro, NÃO edite lib.rs) que lista canais por `trust_tier` (core auto-disponível / curado / comunidade opt-in). Refs pinados, update explícito.
  3. **CI valida manifesto:** passo em `ci.yml` que roda a validação de schema do manifesto de canal (reusa o parser `serde` da frente CHAN — coordene a assinatura com Terminal B) → **manifesto malformado faz o job falhar** (vermelho barra o merge). Teste: um TOML inválido em `channels/` faz o validador retornar erro.
- **Dependência da frente CHAN (B):** o schema/parser de manifesto é de F4-0-1 (Terminal B). Comece pelo **monitor de rede** (independente — só lê `ChannelRegistered`); para o validador de manifesto na CI, use a assinatura que B publicar (avise o Maestro se precisar dela e ainda não saiu — `lina ask "@Maestro 00" ...`).
- **Convenções:** `cargo fmt` + `clippy -p lina-core --all-targets -D warnings` limpos; bin read-only (ZERO mutação, ZERO evento novo); tolerante a versão de schema (campo ausente vira `None`, nunca panic).

## OBJETIVO (critério observável)
**(d)** Com 0 canais conectados → `network_monitor` reporta 0 conexão de saída esperada + a verificação externa (`nettop`/`lsof`) documentada confirma 0 conexões do processo Lina. **(e)** Manifesto de canal malformado → CI **vermelha** (job falha), barrando o merge; manifesto válido → verde.

## RESULTADO ESPERADO
- Não commite. Trabalho na worktree `lina/f4-0-monitor`.
- Cole exit codes: `cargo run -p lina-core --bin network_monitor -- <events-dir> --json`, `cargo test -p lina-core` (schema), `cargo clippy -p lina-core --all-targets` (exit 0). Mostre o YAML do passo de CI.
- Reporte: **`PRONTO: F40-MONITOR`** + resumo (saída do monitor + prova de 0 conexão + o passo de CI) — OU **`BLOCKED: F40-MONITOR`** + o quê. Via `lina ask "@Maestro 00" "<...>" --intent status`.
