# Despacho r1 — Terminal D (QA): F2-0-2 1ª medição de input latency + F2-0-3 kit da régua

## CONTEXTO
Execução F2 (épico: vault `38` §F2-0). Você escreveu a régua (D0) — agora a instrumenta. Decisão F2-0-D dá as palavras-alvo do deck: **vivo · honesto · acolhedor · preciso · artesanal** (fusão T1+T3). Gap da sua própria lacuna L7: input latency NUNCA foi medida no Lina (a sonda mede frametime).

## FUNÇÃO
QA — dono da régua e das medições.

## DIRECIONAMENTO
1. **F2-0-2 — 1ª medição de input latency (keypress-to-photon):** Typometer (grátis) apontado ao terminal focado do Lina (`dist/Lina.app` atual) — idle E sob carga (LINA_LOAD ativa). Se o Typometer não cooperar com a janela gpui, registre o bloqueio técnico e use o fallback da régua (câmera 240fps do roteiro — nesse caso descreva o protocolo e deixe a captura física para a sessão de tela do fundador). Entregável: `tasks/epico-f2/baseline-input-latency.md` com p50/p95/p99 (ou protocolo+bloqueio honesto). É o ANTES que o gate da camada (d) compara — análogo ao `baseline-f1-0.md`.
2. **F2-0-3 — kit da régua** em `tasks/epico-f2/regua/`: (a) tradução SUS pt-BR validada — escolha UMA da literatura com fonte citada e gere o formulário de 10 itens pronto-para-aplicar; (b) deck de reaction cards: ~25 palavras com ≥40% negativas, randomização documentada, contendo as 5 palavras-alvo da fusão + kill-words (genérico/confuso/complicado/sem graça/frio); (c) roteiro de tarefas do leigo v1 (criar 1º agente · organizar o canvas · achar quem pede aprovação — espelha os gates F2-2/F2-3); (d) protocolo do line-up de distintividade (4 lookalikes nomeados na D0).
3. Fronteira: `tasks/epico-f2/` apenas (sem código). Não commite.

## OBJETIVO
Quando a primeira rodada de testers chegar (gate F2-2), o kit está pronto e o baseline de latência existe — a régua deixa de ser papel.

## RESULTADO ESPERADO
`baseline-input-latency.md` + pasta `regua/` com os 4 artefatos. Reporte começo/fim com `--intent status`. Última linha: `PRONTO:`/`BLOCKED:`.

## Tentativas anteriores
Nenhuma.
