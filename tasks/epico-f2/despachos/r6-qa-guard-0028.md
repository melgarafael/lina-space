# Despacho r6 — Terminal D (QA): guard automatizado de conformidade 0028 + verificação final da rodada

## CONTEXTO
Rodada 6. Achado 1 do Arquiteto-revisor (PASS 90): a propriedade "componente que comunica estado NASCE sobre o a11y_live" não tem barreira automatizada — vale onde foi escrita, fronteira aberta p/ componente futuro. Mesmo espírito da sua catraca. Notas: tasks/epico-f2/notas-r6.md.

## FUNÇÃO
QA — fronteira: app/lina-gpui/tests/ (arquivo novo, ex. a11y_conformance.rs). Zero conflito com C/E.

## DIRECIONAMENTO
1. **Guard**: teste que FALHE se um componente em src/ui/ comunicar estado sem compor live_region/a11y_live. Desenho é seu (sugestões: marcador estrutural — todo arquivo de ui/ que usa palavras de estado/Role::Status/Alert deve importar a11y_live; ou allowlist explícita tipo SEAM_EXCEPTIONS com baseline atual; matchers PINADOS por teste como você fez na catraca). Mensagem de falha ENSINÁVEL (aponta o caminho certo).
2. Calibre contra falso-positivo (Panel/Button não comunicam estado — não podem cair) e falso-negativo (um BadgeNovo sem live_region TEM que cair — prove por mutação, seu padrão).
3. **No fechamento da rodada (ping combinado)**: sua verificação dupla já anunciada — migração preserva o contrato (value+live, glifo+texto, cor nunca só) + toast_stack mantém teto-3 e id único na árvore.
4. Validação: suíte + clippy -D + fmt nos seus. Não commite. Reporte E continue.

## OBJETIVO / RESULTADO ESPERADO
A invariante do 0028 deixa de depender de revisor humano — quebra sozinha na CI. Guard + (depois do ping) veredito da migração. PRONTO:/BLOCKED:.

## Tentativas anteriores
Nenhuma.
