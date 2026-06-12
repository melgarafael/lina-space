# DESPACHO r4-validacao-saida — QA (Terminal D)
**id:** `validacao-saida` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Auditoria de EVIDÊNCIAS do checklist de saída do épico F1 + roteiro de tela v2 do fundador. Rodada r4 (saída F1).

## CONTEXTO
- `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"` (HEAD `f1d4810`).
- O épico F1 está a um lote curto da saída. O documento-mestre é `tasks/epico-f1/relatorio-saida-f1.md` — ele afirma fatos com fontes ("todo número aponta o arquivo-fonte"). Seu trabalho é a auditoria CÉTICA dessas afirmações + preparar o roteiro de tela que o fundador vai percorrer.
- Roteiro de tela anterior (referência de formato): `tasks/epico-f1/roteiro-tela-consolidado.md`. Estado da última tela: o fundador validou o roteiro consolidado em 2026-06-11 ~12h (gate F1-1 declarado); a tela M8/M9 (rail/criar Espaço) gerou a rodada fix-criar-espaco (`ef6acd6`+`f1d4810`) que AINDA NÃO foi re-testada na tela.
- Handoff de sessão (estado vivo, §7 topo): nota do vault `_HANDOFF - Continuar Lina Space.md` — não precisa ler inteiro; o bloco de 2026-06-11 ~20h lista o backlog nomeado.

## FUNÇÃO
Você é o QA do gate de saída: verificador independente com permissão de REPROVAR evidência fraca (princípio InsForge nº1 do projeto).

## DIRECIONAMENTO
- **Parte 1 — auditoria de evidências do `relatorio-saida-f1.md`:** para cada afirmação com fonte (§1 tabela, §2 conselhos, §3 amostra): o arquivo-fonte EXISTE? O número citado BATE com o que o arquivo diz? Os commits citados existem no git? Liste qualquer divergência (relatório diz X, fonte diz Y) — divergência é achado, não detalhe. Rode você mesmo as suítes que o relatório cita como verdes (`cargo test --workspace -- --test-threads=1` na raiz; `cd app/lina-gpui && cargo test -- --test-threads=1`) e registre os números REAIS com exit code direto.
- **Parte 2 — roteiro de tela v2:** escreva `tasks/epico-f1/roteiro-tela-r4.md` no formato do roteiro anterior (passos VEJA/imperativo, leigo-legível): (a) re-teste M8/M9 pós-fix (`ef6acd6`/`f1d4810`: modal clampa ao viewport, campos focam ao clique, terminais do Espaço novo nascem no Diretório de Trabalho em grade); (b) os itens de tela que a rodada r4 vai gerar (badge EoL ao escolher Gemini · announcement do M9 com VoiceOver · ativação de licença quando F1-4-6 sair — deixe o slot); (c) o que JÁ foi validado e NÃO precisa repetir (cite a data).
- READ-ONLY no código de produção: você só escreve o marcador, sua entrega e o roteiro novo.
- NÃO valide as entregas dos outros workers desta rodada ainda (elas não existem) — isso é a fatia 2, que o Maestro despacha quando os `.entrega-*` chegarem.

## OBJETIVO
O fundador decide o gate de saída do F1 em cima do relatório e da tela. Evidência citada errada = decisão errada. Sua auditoria é o que torna o relatório confiável — e o roteiro v2 é o que faz a sessão de tela dele render em 15 minutos em vez de 1 hora.

## RESULTADO ESPERADO
`tasks/epico-f1/.entrega-validacao-saida.md`: (1) tabela da auditoria — afirmação · fonte · CONFERE/DIVERGE (com o real); (2) números reais das suítes com exit codes; (3) link para `tasks/epico-f1/roteiro-tela-r4.md` criado. Marcador `.iniciado-validacao-saida` no primeiro ato. Última linha `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
