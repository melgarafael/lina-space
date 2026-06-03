# ADR 0006 — injeção A2A é default-deny por pertencimento ao Espaço (não `AllowAll`)

- **Status:** Aceito (W5-5a implementado e validado de fora: `a2a::` 16/16, build do app OK, gate de render A→B com política real 1/1, red-team de forja negado)
- **Onda/Story:** Onda 5 · W5-5 (gate de segurança — allow-list de injeção A2A)
- **Data:** 2026-06-03

## Contexto

O gancho de allow-list de injeção A2A (`InjectPolicy`) existe desde a W0-9, mas o único app
(`lina-gpui`) passava **`InjectPolicy::AllowAll`** nos **3 call-sites de produção**
(`bridge.rs:286` botão ⚡, `:415` pump real, `:3116` gate de render). As variantes
`AllowOnly`/`Deny` nunca eram instanciadas fora de teste → `policy.allows()` retornava `true`
para **qualquer par** → **o gancho de segurança existia mas estava desligado: qualquer agente
injetava em qualquer agente** (furo **L1-1** da re-auditoria adversarial, `.crosscheck-rt-a2a.md`).

A injeção A2A vira **input do CLI de terceiro** (invariante #1 — é a FEATURE, não um bug), então
uma injeção indevida é vetor real de prompt-injection. A cadeia mais séria do crosscheck era
`AllowAll` (L1-1) + `from` forjável (L1-2) + bypass do guard (L2-1). **L1-2 e L2-1 já foram
fechados** (Round 6 `9d0356c`/`4de454d` — drain flat anonimiza `from`; guard fragmenta
subshell/substituição/traversal), restando **L1-1** como o elo aberto.

Isso toca a âncora de continuidade **Workspace Bus/Supervisor** e o **Envelope A2A** → ADR antes de fechar.

## Decisão

**A política de injeção de produção é DEFAULT-DENY, derivada da topologia VIVA do Espaço — não `AllowAll`.**

- Novo tipo owned **`WorkspaceTrust`** (`a2a.rs`): `from_members(&[NodeId])` deriva a matriz de
  confiança = **todo par ordenado distinto entre os membros vivos do mesmo Espaço**;
  `policy() -> InjectPolicy::AllowOnly(&pares)`. Par com `from`/`target` fora do Espaço
  (id desconhecido/forjado) **não é gerado** → `allows()` nega por construção.
- Os **3 call-sites** de `bridge.rs` passam `WorkspaceTrust::from_members(&live_member_ids(&sup)).policy()`
  — a confiança é re-derivada **a cada entrega** (`sup.list()` filtrado por `is_alive()`), porque o
  roster muda em runtime.
- `InjectPolicy` e `allows()` ficam **inalterados** (Copy + lifetime preservados) → os gates
  existentes (`gate_onda0/2/3/w34`) que usam `AllowAll` explícito **em teste controlado** seguem
  verdes. `AllowAll` passa a significar, na doc, "testes/headless", não "produção".

**Por que derivar da topologia viva (e não de config TOML estática):** o invariante #5 do norte é
*"pertencimento = conexão"* — quem está no Espaço é a fonte da verdade da topologia. Uma allow-list
estática em arquivo desincronizaria com nós que entram/saem em runtime. A confiança DEVE ser
projeção do roster vivo, não um cabo declarado à parte.

## Limite explícito (defesa em profundidade — não fingir que resolve tudo)

Esta camada (L1-1) enforce **PERTENCIMENTO ao Espaço**, NÃO autentica a identidade de um **peer
REAL vivo**: se `@Boss` é nó vivo e um processo de **mesmo uid** forja `from=@Boss` no outbox
por-nó, o router resolve para o `NodeId` vivo (que É membro) e a allow-list permite. Essa é a
fronteira **L1-3** (auth por-nó não é fronteira de SO) — **item de fronteira #2 da Onda 5**
(sandbox por terminal / token-por-spawn), conscientemente NÃO fechado aqui.

As camadas de defesa, da origem ao backstop:
1. **drain anonimiza `from`** do outbox FLAT (L1-2, Round 6) — canal não-autenticado vira `UnknownSender`.
2. **`WorkspaceTrust` barra não-membro** (L1-1, este ADR).
3. **gate de execução + custódia de segredo** (W3-6, ADR 0004) — backstop para ação destrutiva, mesmo que a injeção passe.

## Consequências

- O diálogo legítimo **A→B→A** continua: `from_members([A,B])` gera `(A,B)` **e** `(B,A)` →
  ambas as direções confiadas (validado: `a2a_roundtrip_pulse_persist_and_screen` passa com a
  política real). Par desconhecido/forjado é negado nos dois sentidos (`InjectionDenied`, 0 writes).
- `a2a_sanitizes_bracket_terminator` trava a sanitização `ESC[201~` (CVE-2021-31701) contra regressão.
- A próxima sessão NÃO deve "simplificar" de volta para `AllowAll` em produção — seria reabrir L1-1.

## Alternativas rejeitadas

- **Manter `AllowAll` ("MVP permissivo")** — rejeitado: dead code de segurança normaliza o hábito de
  embarcar o produto com o gate desligado.
- **Allow-list estática em config TOML** — rejeitado: o roster muda em runtime; config estática
  desincroniza com a topologia viva (viola invariante #5).
