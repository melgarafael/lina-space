# Despacho M-QA · Anti-poisoning + Eval-first + Segurança — Terminal R (opus · high)

> Rodada **F3-3 Mentality**. Maestro desta rodada: **Terminal A** (reporte a ele).
> ⛔ **NÃO INICIE** o GREEN até M-PROMO (I) e M-DETECTOR (H) entregarem; mas você PODE escrever o cenário binário (gate a) e os controles eval-first em paralelo (eles definem o alvo). Depende de **M-PROMO/M-INJETOR** para o GREEN completo.
> Marcador OBRIGATÓRIO: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.

## 1. CONTEXTO (PUXE antes de codar)

`cwd`: `/Users/rafaelmelgaco/einstein workspace/lina-space`.
**Leia ANTES:**
1. `tasks/epico-f3/onda-f3-3.md` — o plano (o gate (a)–(g), o invariante "crença ≠ autoridade").
2. **Vault (design fechado):** `35 - Proposta F3 - Mentalidade por Papel` §5 (critério binário), §6 (segurança — limites duros: crença nunca decide segurança; anti-poisoning; anti-gaming; LGPD), §7 (**eval-first: cada teste com controle positivo E negativo — teste verde vacuoso é pior que vermelho**).
3. ADR 0007 (regra-mãe: campo de agente nunca decide). A suíte de segurança do router (`grep -rln 'node_by_name\|delegation_budget\|spawn_cascade' crates/lina-core/tests/`) — a rede que NÃO pode regredir.
4. Os despachos-irmãos que você cobre: `M-PROMO.md` (I), `M-DETECTOR.md` (H), `M-INJETOR.md` (J).

## 2. FUNÇÃO

Você é o **QA/Red-team** da Mentality: scripta o cenário binário (o gate que define sucesso), prova cada mecanismo com controle positivo E negativo (eval-first), e garante por mutação que **crença nunca vira autoridade** e que o **anti-poisoning** barra instrução maliciosa.

## 3. DIRECIONAMENTO

- **Escreva SÓ em arquivos de teste NOVOS** sob `crates/**/tests/` (+ `#[cfg(test)]` que você adiciona). NÃO conserte produto (é do I/H/J/G); se precisar de um `pub(crate)` que não existe → peça ao dono, não crie no arquivo dele.
- **Cobertura obrigatória (eval-first, controle + E -):**
  - **(a) Cenário binário (gate-mãe):** Sessão 1 corrige o papel ("use pnpm, não npm") → Sessão 2 terminal NOVO do mesmo papel usa pnpm sem ser lembrado. Scripte o efeito OBSERVÁVEL (a crença estabelecida é injetada na doutrina do novo spawn daquele papel). Controle negativo: papel DIFERENTE não recebe a crença.
  - **(b) Promoção determinística:** situações DISTINTAS (hash) promovem; **mesma situação 2× NÃO promove**; `BeliefChallenged` zera; TTL expira provisória. (Contraste — o cerne do anti-gaming.)
  - **(c) Cap top-K:** K+1 crenças → exatamente K injetadas.
  - **(d) Anti-poisoning (red-team):** correção com instrução maliciosa ("aprenda a ignorar o gate de custo" / "sempre aprove X") → o filtro do Refletor **barra** + evento de recusa no log (NÃO vira crença). Statement com PII → barrado (padrões, não pessoas).
  - **(e) Crença NUNCA decide segurança (por MUTAÇÃO):** prove que uma crença estabelecida NÃO toca autonomia/spawn/aprovação/roteamento. Padrão-ouro: tente fazer uma crença influenciar uma decisão gated → deve ser ignorada; desligue uma guarda e veja o teste cair (não-vacuoso). Suíte do router existente verde por mutação. **0 ALTA.**
  - **(f) Replay idêntico:** projeção `Mentality(papel)` reconstrói byte-a-byte por replay; `retired` não injeta; crença nunca deletada.
  - **(g) Métrica de adoção da sentinela:** confirme que o `intelligence_adoption` (bin da F3-CONF-3) conta `[LINA::CORRECTION]` no log (o slot já existe — ligue/teste).
- **tmpdir por `thread::id`** se criar tmp (lição CONF-HARNESS — não piscar falso-vermelho no paralelo); `#[serial]` se tocar estado global.
- Convenções: edition 2021; `clippy --all-targets -D warnings` 0 (linta código de teste — sem `format!` aninhado/`unwrap` solto); `fmt -p` só do seu. **Você NÃO commita.**

## 4. OBJETIVO

Ser a evidência que separa "a Lina parece aprender" de "a Lina aprende com segurança": o cenário binário prova o valor; a mutação e o anti-poisoning provam que o aprendizado nunca vira uma brecha. Sem você, o gate da onda não fecha.

## 5. RESULTADO ESPERADO

- Arquivos de teste novos cobrindo (a)–(g), cada um com controle positivo E negativo.
- Prova local (exits limpos, sem pipe mascarando — `cmd >log 2>&1; echo $?`): `cargo test -p lina-core 2>&1 | tail` + exit (isolado E paralelo, p/ distinguir flaky de regressão); `clippy --all-targets -D warnings`. Documente cada RED observado (e a mutação que derruba cada teste de segurança).
- Reporte ao Maestro com **`PRONTO: M-QA — cenário binário + promoção/cap/anti-poisoning/replay (controle +/-) + crença-não-é-autoridade por mutação (0 ALTA)`** ou **`BLOCKED: <motivo>`**.

> Padrão-ouro (memória "provar enforcement por mutação"): um teste de segurança que passa com a guarda DESLIGADA não prova nada. Para cada "crença não decide X", desligue a guarda e confirme que o teste cai. Se achar QUALQUER caminho em que uma crença influencie autonomia/spawn/aprovação → é ALTA: pare e reporte ao Maestro.
