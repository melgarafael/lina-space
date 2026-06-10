# DESPACHO r1-ux-f14 — Designer
**id:** `ux-f14` · Leia ANTES: `tasks/despachos/_regras-comuns.md`

Fatia 100% de DOC/COPY (zero código). Prepara a onda F1-4 para que o código nasça com copy e UX resolvidos.

## Fronteira de arquivos (sua, dono único nesta rodada)
- `tasks/epico-f1/ux-flows.md` (edições cirúrgicas nas seções citadas — preserve o resto)
- `tasks/epico-f1/copy-f1-4.md` (NOVO)
- **NÃO toque:** código, ADRs, peças de onda (`ondas-*.md`).

## Tarefa 1 — Reconciliar o T7§P com o gate 100% offline (CONFLITO REAL entre specs)
**Fonte:** `tasks/epico-f1/ondas-2-4.md` linhas 310-318 (Proposta de ajuste #6) + a decisão: o licenciamento é **100% offline** (13.6: validação local ed25519, SEM phone-home, SEM node-locking na F1, SEM revogação). No `ux-flows.md` T7§P/M9: CORTE/reformule os estados que pressupõem servidor ("confirmação online quando houver rede", "chave já ativa em outro computador", "chave revogada", "sem internet e validação exige rede", selo "associada a este computador"). MANTENHA os acertos: trim-ao-colar, duas-colunas honestas, erros acionáveis. Estados válidos restantes: chave incompleta/malformada · assinatura inválida · chave expirada (expiry das chaves de aluno) · sucesso.

## Tarefa 2 — Espelhar a decisão free=1
O T7§P/M9 desenha "✓ até 2 Espaços" no Free; a decisão do gate da onda é **free = 1 workspace**. Corrija TODAS as ocorrências no ux-flows (M8/M9/T7§P).

## Tarefa 3 — `copy-f1-4.md`: a copy congelada da onda (pt-br, zero jargão, lei do glossário: "Agente"/"Espaço", NUNCA "terminal"/"workspace" na superfície)
1. **Upsell honesto** (bloqueio do 2º Espaço no free): explica o limite SEM culpa, 2 saídas claras ("Já tenho uma chave" / "Quero o PRO" → navegador), nunca beco.
2. **Ativação** (modal de colar chave): rótulos, placeholder, 4 erros acionáveis e leigos (ex.: "essa chave está incompleta — confira o e-mail de compra"), sucesso sem restart.
3. **Painel de licença no T7**: plano, validade, "Espaços: usados/limite", trocar/remover chave.
4. **Badges honestos do restore (F1-4-3):** "sessão retomada" vs "novo começo — o agente não lembra da conversa anterior" + variação para aviso de expiry não-bloqueante (F1-4-5 critério 7).
5. **Switcher M8 (F1-4-4):** rótulos do mini-status por Espaço (agentes vivos, estado dominante, custo do dia, pendência de atenção) + estados vazios ("nenhum Espaço arquivado").
Para cada item: a copy final + 1 linha de racional. Anti-padrões a evitar: paywall surpresa; jargão técnico; mentir estado (lição: badge honesto é REQUISITO, não polish).

## Entrega
`tasks/epico-f1/.entrega-ux-f14.md` (diffs principais + decisões). Marcador: `.iniciado-ux-f14`.
