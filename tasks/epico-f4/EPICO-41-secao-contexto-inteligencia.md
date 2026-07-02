# [COLAR NO VAULT] Seção para a nota `41 - Epico Fase 4 — Integracoes Canais e Contexto`

> Este arquivo é um **bloco pronto para colar** na nota 41 do vault (não consegui escrever no vault direto — ver nota no fim). Cole o conteúdo abaixo da linha como uma nova seção `§VI` do épico. Tudo aqui consolida `docs/adr/0057..0059` + `tasks/epico-f4/onda-ctx-contexto-inteligencia.md`.

---

## §VI — Pilar Contexto & Inteligência (port deliberado do `ruvnet/ruflo`)

**Por que existe esta seção.** Em 2026-06-29 analisamos o repositório `ruvnet/ruflo` (o "meta-harness" líder para Claude, ex-Claude Flow) — clone + graphify nos 9 plugins relevantes (382 nós / 549 arestas) + leitura do código real. O objetivo: trazer para o Lina o que o ruflo faz de melhor, agora que a direção "o Lina não é harness" deixou de ser dogma. A descoberta que orienta tudo: **o que o ruflo anuncia ≠ o que o código faz** — e o Lina já tem o substrato (event log) que o ruflo precisou de um banco à parte para ter.

### O que NÃO copiamos (e por quê) — recusa do genérico
| Anunciado pelo ruflo | Realidade no código | Veredito |
|---|---|---|
| Consenso Byzantine/Raft/Gossip (swarm) | Rótulo de config; **nenhuma** tool de consenso existe | **Descartado** (branding) |
| "Witness verification / signed manifest" (RVF) | RVF é só container de memória vetorial; assinatura é "fase futura" | **Descartado** (vaporware) |
| Roteamento "neural/SONA/MoE" | Heurística (`hooks_route`) + outcomes gravados | **Copiado como heurística** (honesto) |
| Banco vetorial AgentDB como fundação | O Lina já tem fonte da verdade durável (event log) | **Substituído por projeção** |

### O que copiamos — 3 decisões (ADRs 0057, 0058, 0059)

**1. Memória de trajetória — ADR 0057** ⭐ (joia da coroa)
Importa o **pipeline de recall** do ruflo (SmartRetrieval ADR-090: expansão → multi-query+RRF → boost de recência → MMR → score composto) como **maturação da C2 do ADR 0045**. No Lina é **projeção sobre o event log** — não um banco novo. A Lina passa a lembrar "da última vez que enfrentei uma tarefa como esta, o caminho X funcionou". Reusa `skill_index.rs` (BM25) + `SkillOutcome`. Embeddings ficam como porta futura (aditiva), não MVP.

**2. Roteamento tarefa→terminal — ADR 0058**
**Realiza a porta que o ADR 0045 deixou aberta** ("o mesmo roteador serve para escolher terminal/modelo/effort"). Combina capacidade (Área de Poderes, ADR 0052) × outcome (0057) × disponibilidade → sugere o terminal certo com motivo legível, e o tier de esforço por complexidade. Heurístico (o ruflo confirma que é o caminho), respeita a autonomia (manual informa / assistido propõe / autônomo atribui). Porta também o **anti-drift** (o único pedaço de "swarm" real do ruflo).

**3. Gate de auto-aprimoramento — ADR 0059**
O ruflo (harness maximalista) **chegou à mesma conclusão de governança do fundador**: o "Darwin Mode" mede automático mas **só aplica com `--confirm` humano** ("ADR-153 rejects auto-evolving"). Importa o **score + bench** (a medição que faltava à Mentality) e o conecta ao gate `BeliefProposed→BeliefEstablished` que já existe: mede a melhoria candidata contra um corpus fixo e **PROPÕE** — nunca aplica. Operacionaliza o fio "auto-aprimoramento: sugere, nunca aplica".

### Onda de execução
**Onda F4-CTX — Contexto & Inteligência** · peça executável em `tasks/epico-f4/onda-ctx-contexto-inteligencia.md` · 7 stories de território disjunto:
- F4-CTX-1 projeção de trajetória · F4-CTX-2 recall 5-fases · F4-CTX-3 consolidação (ADR 0057)
- F4-CTX-4 roteador tarefa→terminal · F4-CTX-5 guardrail anti-drift (ADR 0058)
- F4-CTX-6 score+bench · F4-CTX-7 proposta medida → gate humano (ADR 0059)

**ADR-gate:** os 3 ADRs estão **Aceitos** (ratificados pelo fundador em 2026-06-29). As stories estão liberadas para sequenciamento pelo Maestro.

### Fios do fundador que esta seção operacionaliza
- **Inteligência da Lina** — memória de RESULTADO que nenhum CLI mono-processo tem (0057) + roteamento que aprende (0058).
- **Auto-aprimoramento (sugere, nunca aplica)** — gate de bench que mede e propõe, validado de forma independente pelo próprio ruflo (0059).
- **Governança + gates** — toda decisão segue sendo DADO, jamais autoridade; ação irreversível segue exigindo gate humano.
- **Local-first** — projeções e índices locais; **rejeitamos** explicitamente o export de padrões para IPFS/Pinata do ruflo.

### Invariantes/âncoras tocadas (nenhuma porta fechada)
- Tudo é **evento/projeção sobre o event log** (inv #4) — nenhum evento novo no MVP do 0057.
- **Neutralidade multi-CLI** (inv #3) e **local-first** (inv #2) preservadas — zero dependência externa no MVP (BM25/léxico).
- `Router`/`deliver_a2a` **intocados** — a suíte de segurança do router segue verde.

---

> **Nota operacional (por que este bloco está no repo e não no vault):** o vault Obsidian está protegido por permissão do macOS (TCC) — nem o sandbox nem o bypass conseguem ler/escrever `~/Documents/Obsidian Vault`, e a nota 41 não está no índice do `lina vault` (placeholder iCloud). Os artefatos canônicos (ADRs + peça de onda) já estão versionados no repo, que o `CLAUDE.md` define como "a fonte da verdade story-a-story". Para materializar no vault: cole a seção acima na nota 41. Se preferir que eu tente por outro caminho (ex.: liberar Full Disk Access ao terminal, ou um `lina vault write`), me avise.
