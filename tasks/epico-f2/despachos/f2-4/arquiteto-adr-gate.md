# DESPACHO — Arquiteto · ADR 0052 (gate da Onda F2-4) · id: f2-4-adr

> Leia `tasks/epico-f2/despachos/f2-4/_contexto.md` + `tasks/despachos/_regras-comuns.md` ANTES.

## CONTEXTO
A Onda F2-4 cria a **Área de Poderes**: tornar visíveis skills/agents/hooks/commands/MCPs/plugins
instalados no computador do leigo, com 5 estados e ação de 1 clique. O scanner (core) e o painel (UI)
não podem começar sem um **contrato selado** — porque a onda toca duas doutrinas inegociáveis:
1. **inv#4 "o event log é a fonte da verdade"** × a Área de Poderes lê o **DISCO** (estado externo
   que o app não controla — skills instaladas por fora).
2. **doutrina de segurança "mostrar ≠ autorizar"** — campo lido do disco é DADO, jamais autoridade.

A pesquisa já resolveu a tensão (não re-decida do zero): `tasks/pesquisa-f2/entrega-d4-comandos-menus.md`
**§II achado A8** (ADR 0008 transplanta inteiro) + **§IV conflito #4** (o painel é projeção de scan, como
`lina list` é projeção de runtime; mudanças PODEM virar evento aditivo para auditabilidade). Seu ADR
**ratifica e fixa o contrato** para os workers — não inventa arquitetura nova.

## FUNÇÃO
Você é o **Arquiteto**. Entrega: **`docs/adr/0052-area-de-poderes-scan-determinista.md`** (curto, aceito).
É o ÚNICO arquivo que você toca. Revisor-cego das stories que comunicam estado (se o Maestro pedir depois).

## DIRECIONAMENTO (o que o ADR DEVE decidir — cada ponto é um contrato que destrava um worker)
Leia primeiro: `docs/adr/0008-deteccao-de-cli-em-camadas.md` (o padrão que você transplanta) e o §I/§III
da entrega-d4. Veja o estilo dos ADRs vizinhos (`docs/adr/0050-*`, `0008-*`) — curto, com Decisão/Limite/
Consequências/Alternativas rejeitadas. Decida e fixe:

1. **Fonte da verdade do scan = DISCO, manifest-first.** O scan é **projeção de disco efêmera**
   (re-derivável a cada abertura, ~0ms), NÃO event-sourced como fonte primária. Justifique contra inv#4:
   poderes são estado EXTERNO ao app (instalados por outras ferramentas); o log não os possui — igual
   `discovered_clis` é projeção do PATH, não do log. Manifesto pequeno SEMPRE; árvore pesada NUNCA.

2. **Evento de auditoria `PowerScanned` — aditivo, só METADADOS.** Decida se entra nesta onda (recomendado:
   sim, mínimo) e fixe seu contrato: **somente contadores** (`{ counts: por-kind, total }`) + timestamp —
   **NUNCA** nome/descrição/conteúdo de skill, **NUNCA** campo que decida autorização. Padrão META
   (no-op no `apply`, observabilidade pura — molde `SkillSelected` `events.rs:1387`). O painel **NÃO**
   depende do evento (lê o scan direto); o evento serve à camada (f) do release (auditoria anti-manipulação).

3. **Contrato de tipos (a interface Core↔UI — o view-model que o painel renderiza).** Fixe os enums/struct
   que o `powers.rs` produz e o `powers_panel.rs` consome. Proposta a ratificar/ajustar:
   - `PowerKind { Skill, Plugin, Agent, Command, Hook, Mcp }`
   - `PowerOrigin { Global, Project, Cli(String) }`  (origem **rotulada** — o leigo vê "global/deste projeto/do Gemini")
   - `PowerState { Ready, UpdateAvailable, NeedsRepair, InertHere, Disabled }`  (os 5 estados da entrega-d4 §III.b)
   - `Power { kind, name, description, origin, state }` + `PowerInventory { powers, counts }`
   Decida: a `description` é a **crua** do manifesto (a tradução leiga é responsabilidade da copy do painel)? (recomendado: sim — separa dado de apresentação.)

4. **mostrar ≠ autorizar (limite explícito, transplante do ADR 0008).** Declare: nenhum campo de
   `Power` (lido do disco) entra em caminho de identidade/ordem/autorização; aparecer no painel não
   habilita executar; gates continuam em custódia/WorkspaceTrust. Este é o critério que o QA prova por mutação.

5. **Caminhos por CLI vêm do perfil TOML (inv#3).** Campos novos no `CliProfile` (aditivos `#[serde(default)]`):
   `skills_dir` / `mcp_config_path` (ou nomes que você preferir) — CLI novo entra sem recompilar.
   Fixe os nomes para o Dev Core não adivinhar. Não repita o erro `KNOWN_CLIS` hardcoded (ADR 0008 §5).

6. **Estado da arte do watcher (F2-4-2).** Ratifique: scan-ao-abrir é o essencial; watcher raso debounced
   (≥750ms) no **diretório-PAI** dos ~5 manifestos é o "teto barato" (atomic saves trocam o inode →
   observar o pai); **watch recursivo da árvore = PROIBIDO** (inotify ENOSPC no Linux futuro). Diga onde a
   parte PURA mora (core: lista de paths-pai + política de debounce) e que a fiação do `notify` real é do Maestro (app).

## OBJETIVO
Selar o contrato mínimo que destrava o Dev Core (tipos + campos CliProfile + evento) e o Frontend
(view-model) e dá ao QA o critério inforjável (mostrar≠autorizar). Curto e decisivo — não um tratado.

## RESULTADO ESPERADO
`docs/adr/0052-area-de-poderes-scan-determinista.md` **aceito**, com os 6 pontos decididos, contrato de
tipos explícito (copiável para o código), Limite "mostrar≠autorizar", e Alternativas rejeitadas (ex.:
event-source o scan como fonte primária — rejeitado por ser estado externo; watch recursivo — rejeitado).
Reporte ao Maestro ao terminar: `lina ask "@Maestro 01" "ADR 0052 aceito: <1 linha>" --intent status`.

Última linha da sua entrega (`tasks/epico-f2/despachos/f2-4/.entrega-f2-4-adr.md`):
`PRONTO: ADR 0052 aceito — contrato de tipos + evento + mostrar≠autorizar fixados` ou `BLOCKED: <motivo>`.
