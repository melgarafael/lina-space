# Despacho V — Verificação adversarial da Pesquisa F2 (onda 2)

## CONTEXTO
A pesquisa F2 (UI/UX) tem 4 entregas prontas em `tasks/pesquisa-f2/`: `entrega-d0-eval.md` (régua), `entrega-d1-identidade.md` (identidade visual), `entrega-d2-gpui-ds.md` (design system gpui), `entrega-d3-canvas-ux.md` (canvas UX). A 5ª (D4 comandos/menus) chega em paralelo — NÃO espere por ela; se aparecer antes de você terminar, inclua. Plano e quality gates: `_plano-pesquisa-f2.md`. Esses achados vão virar decisões do épico F2 — o que sobreviver a você vira regra.

## FUNÇÃO
Red Team / verificador independente. Sua missão é REFUTAR, não confirmar. Default na incerteza: "incerto", nunca "confirmado".

## DIRECIONAMENTO
1. Alvo: TODOS os achados marcados **LOAD-BEARING: sim** e/ou **SUSPECT** (total ≈30 nas 4 entregas). Para cada um:
   - Tente derrubar com contra-evidência independente (busca própria, fonte DIFERENTE da citada).
   - **Teste de circularidade:** as fontes do achado compartilham a mesma origem? N posts citando o mesmo artigo = 1 fonte.
   - **Gate de citação (amostral):** re-fetch de ~2 URLs por entrega (escolha as mais load-bearing) — a página diz mesmo o que o achado afirma?
   - Veredito: CONFIRMADO / REFUTADO (com a contra-evidência) / INCERTO (mecanismo pode ficar, número sai).
2. Atenção especial (suspeitas que eu, Maestro, já anotei):
   - D0: referências cruzadas a um "A9" num doc com 8 achados (numeração interna) — confira se algum claim ficou órfão.
   - D0: números exatos de fonte única MeasuringU (SUS 68 / SEQ 5,3-5,6) — circularidade NN/g↔MeasuringU?
   - D1: claim "Linear redesign 2025 abandonou o próprio look" (confiança média declarada) e os dados internos do Arc (0,4-5,5% — fonte única da empresa).
   - D1: dossiê OpenDesign (stars/fork-ratio inconclusivo; entidade jurídica não identificada) — dá pra fechar?
   - D2: "compatibilidade do pin 09165c15 com gpui-component" é inferência por datas NÃO compilada — confirme que está marcada como tal (não derrube, só confira a honestidade).
   - D3: datas inconsistentes da thread HN (A3) e o claim do minimap do tldraw (URL primária 404, 2 fontes secundárias da própria tldraw = circularidade?).
3. Conflitos ENTRE entregas: procure contradições D0×D1×D2×D3 (ex.: budget de perf do D0 vs custo do "viewer vivo" do D1 vs LOD do D3) — liste cada uma com sua leitura de qual lado a evidência favorece.
4. Quality gates do plano valem (datar suas contra-fontes; fetch real; piso 5 buscas, teto 15).
5. Você NÃO reescreve as entregas — só emite vereditos. O Maestro aplica.

## OBJETIVO
O relatório consolidado da F2 só carrega achado que sobreviveu a um cético — e todo número exato de fonte única cai para "mecanismo" se não se sustentar.

## RESULTADO ESPERADO
`tasks/pesquisa-f2/entrega-v-verificacao.md`: tabela por entrega (achado → veredito → justificativa/contra-fonte datada) + seção "conflitos entre entregas" + lista "descartados" (o que cair, com o porquê). Reporte via `lina ask "@Terminal B - Effort: Ultra Code" "<status>" --intent status`. Última linha `PRONTO:`/`BLOCKED:`. Não commite; não edite o vault nem as entregas dos colegas.

## Tentativas anteriores
Nenhuma — 1º despacho da onda V.
