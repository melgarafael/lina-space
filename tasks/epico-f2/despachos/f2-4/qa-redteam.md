# DESPACHO — Terminal R · QA red-team da Onda F2-4 · id: f2-4-qa

> Leia `tasks/epico-f2/despachos/f2-4/_contexto.md` + `tasks/despachos/_regras-comuns.md` +
> **`docs/adr/0052-area-de-poderes-scan-determinista.md`** ANTES. Carregue a skill `ai-agent-qa`.

## CONTEXTO
A Área de Poderes lê o disco do usuário e mostra poderes na tela. Há **duas formas de essa onda
regredir o produto** e você é a barreira contra ambas:
1. **mostrar virar autorizar** — um campo lido do disco (nome de skill, frontmatter, JSON de plugin)
   acabar decidindo identidade/ordem/autorização. Isso seria furar a doutrina de segurança inteira do Lina.
2. **o scan tocar a árvore pesada** — abrir `~/.claude/plugins/` (1,9GB, repos git) em vez do manifesto
   de 13KB → trava no mac, ENOSPC no Linux futuro (a "bomba" do A7 da entrega-d4).

## FUNÇÃO
Você é o **QA (red-team)**. Entrega testes que PROVAM enforcement por mutação — não testes que passam por
acaso. **Não toca código de produção.** Pode COMEÇAR cedo (escreva os testes contra o contrato do ADR 0052)
e LIGAR conforme o Dev Core entrega o `powers.rs`.

**Fronteira (LEI):**
- CRIA: `crates/lina-core/tests/f2_4_powers.rs` (+ `app/lina-gpui/tests/f2_4_*.rs` se precisar cobrir a UI).
- NÃO edita produção. Achou um bug? Registre na entrega → o DONO da fatia conserta, você re-prova.

## DIRECIONAMENTO — os critérios inforjáveis (cada um é um teste não-vacuoso)
1. **mostrar ≠ autorizar (o teste-âncora, por mutação):** prove que nenhum campo de `Power` lido do disco
   entra em caminho de autorização. Construa um inventário com um `Power` cujo nome/origem seja forjado para
   parecer autoridade (ex.: origem "system", nome com sentinela) → prove que ele **NÃO** habilita execução,
   não vira identidade, não muda ordem. A suíte de segurança do router (se a onda a tocar) segue **VERDE por
   mutação** (desligue a guarda → RED → religue). 0 achados ALTA.
2. **manifest-first (perf/segurança):** prove que `scan_powers` lê plugins **só** do `installed_plugins.json`
   e **nunca** abre a árvore pesada. Técnica: fixture com um arquivo-sentinela dentro de um
   `plugins/<repo-falso>/` que, se aberto, falha o teste (ou contador de leituras / caminho que registra acesso).
3. **frontmatter inválido → `NeedsRepair` com ação:** skill com `SKILL.md` quebrado aparece como
   "precisa de conserto" (estado + ação), nunca some silenciosamente nem derruba o scan.
4. **inerte-aqui é o caso normal:** skill na pasta do CLI X com terminal rodando CLI Y → `InertHere` com a
   origem correta (não `Ready`, não sumido).
5. **5 estados sempre texto+ícone+cor (WCAG 1.4.1):** se cobrir a UI, prove que cada estado tem glyph + rótulo,
   não só cor (mutação: remova o glyph → RED).
6. **evento `PowerScanned` (se existir) só carrega metadados:** prove que o evento emitido **não** contém
   nome/descrição/conteúdo de skill — só contadores. Campo de conteúdo no evento = RED.
7. **replay/round-trip:** se houver evento aditivo, log antigo (sem `PowerScanned`) ainda carrega
   (`#[serde(default)]`); replay reproduz contadores idênticos.
8. **(opcional, se houver fôlego) métrica de adoção:** confirme que o scan/uso da Área de Poderes é
   observável no log para o `intelligence_adoption` futuro — sinalize, não bloqueie.

## OBJETIVO
Garantir que a vitrine de Poderes é **observação pura**: mostra tudo, autoriza nada; lê o manifesto,
nunca a árvore; e nenhum estado mente nem some. Prova por mutação, não por ausência de objeção.

## RESULTADO ESPERADO
- `crates/lina-core/tests/f2_4_powers.rs` (+ app se preciso) cobrindo os critérios 1-7, **verdes**, com
  pelo menos os testes 1 e 2 provados por mutação (desliga a guarda → RED documentado → religa → GREEN).
- 0 achados ALTA; log de scan sem segredo/conteúdo de skill.
- Validação: `cargo test -p lina-core -- --test-threads=1` (+ `cd app/lina-gpui && cargo test` se cobriu UI),
  `cargo clippy -p lina-core --all-targets -- -D warnings`, exit direto, lido de arquivo.

Reporte ao Maestro: `lina ask "@Maestro 01" "<status>" --intent status`. Entrega
`tasks/epico-f2/despachos/f2-4/.entrega-f2-4-qa.md` com a tabela critério→teste→arquivo:linha→resultado de
mutação. Última linha: `PRONTO: <resumo>` ou `BLOCKED: <motivo>`.
