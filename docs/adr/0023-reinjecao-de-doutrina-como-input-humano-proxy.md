# ADR 0023 — Re-injeção de doutrina como input humano-proxy (não A2A)

**Status:** Aceito (ratificado pelo Maestro, 2026-06-08). **Emendado em 2026-06-08 (FIX-A3).** O red-team do gate F1-1 derrubou a premissa de segurança original: a mailbox de **filesystem** (`<.lina>/reinject/`) era alcançável por **escrita direta** de qualquer agente do mesmo usuário de SO (todos os agentes rodam como o mesmo usuário). A entrega migrou de mailbox de filesystem para **fila EM-PROCESSO** + **texto regenerado no consumidor** + **alvo por `NodeId` autenticado** (§Segurança e §Consequências revisadas abaixo). Decisão forçada in-implementation pela story **F1-2-4** (editar terminal vivo); divergência do hint da story escalada com prova pelo Dev 01.

## Contexto

A F1-2-4 (edição viva de 1ª classe) precisa **re-injetar a doutrina atualizada** no terminal **vivo** quando o papel do nó muda (gatilho: confirmação humana no modal M6-E). A story sugeriu, como hint, fazer isso "pela fila serial (`deliver_a2a`) para o próprio nó".

## Problema

O caminho literal do hint **é impossível sem regressão de segurança**, verificado contra o disco:

- `route_message` (router.rs) **exclui o sender** em `resolve()` → `from == to == self` dá `NoTarget`. Um nó nunca se auto-entrega pelo router.
- `WorkspaceTrust::from_members` **exclui pares self** (`if from != target`, `a2a.rs:275`). Logo `deliver_a2a(target, from=target)` → `InjectionDenied`. Forçar com `InjectPolicy::AllowAll` seria **exatamente a regressão que a W5-5 / ADR 0006 fecharam** (default-deny por pertencimento).
- Não existe nó "system"/orquestrador no roster (os seeds são terminais) → não há `from` legítimo para uma doutrina "do sistema → nó".

## Decisão

A re-injeção de doutrina é uma **classe de confiança de INPUT HUMANO (human-proxy)**, não comunicação agente↔agente:

1. **Gatilho:** só `NodeManager::assign_role`, chamado **apenas** no Save do M6-E (UI, pós-confirmação humana explícita). Nenhum caminho de agente alcança `assign_role`.
2. **Entrega:** pela **fila serial de ESCRITA** do Supervisor (`lock_pty` + `enqueue_write`, o **mesmo cano do teclado humano** via `write_human`), **faseada** (`build_paste` → `submit_delay` → Enter separado), escritor único (sem interleave com a digitação humana).
3. **~~Mailbox separada de filesystem~~ → FILA EM-PROCESSO (EMENDADO FIX-A3):** `Arc<Mutex<VecDeque<ReinjectItem>>>` compartilhada (clone de `Arc`) entre o produtor (`NodeManager::assign_role`) e o consumidor (`MailboxPump::drain_reinject`). **Sem superfície de filesystem** — nenhum processo/agente externo a alcança. Não toca `deliver_a2a`/`Router`/`WorkspaceTrust` → a suíte de segurança do router segue intacta (`gate_w34` verde).
4. **Alvo autenticado por `NodeId` (EMENDADO FIX-A3):** o `target` é o **`NodeId` do roster**, cunhado **dentro do `assign_role`** (que já o tem, autenticado) — **jamais** um nome/string re-resolvido depois. O **texto** da doutrina é **REGENERADO no consumidor** a partir do `role` (`doctrine_reinjection_text(role)`), nunca um `payload` transportado: a própria estrutura em-processo não carrega texto arbitrário.
5. **Freio (W4-3):** com o freio ativo, o drain devolve `[]` sem tocar a fila → **enfileira, não injeta**; a doutrina só é injetada após `OrchestrationResumed`. (FIX-A3: a fila é em RAM — **não** mais crash-durável; ver tradeoff em §Consequências.)

## Segurança (revisada — FIX-A3)

### O furo da premissa original (ALTA A3, red-team do gate F1-1)

A premissa original — *"nenhum verbo `lina` escreve no reinject → a superfície não é alcançável por agente; no pior caso o carimbo `from`=dono-do-subdir restringe o alvo ao próprio remetente"* — **estava errada**. Ela assumia que a única forma de chegar à mailbox era um verbo `lina`. Mas a mailbox era um **diretório de filesystem** (`<.lina>/reinject/<nó>/`) e **todos os agentes rodam como o mesmo usuário de SO** — logo um agente malicioso escrevia **direto** em `<.lina>/reinject/<PEER>/x.json` (sem nenhum verbo). O drain carimbava `from`=nome-do-subdir=**o peer** → alvo = aquele peer → injetava o `payload` (texto **arbitrário**) no PTY do peer. E, ao contrário do A2A, o reinject **não tinha 2ª camada** (router/`WorkspaceTrust`), violando a defesa-em-profundidade. Permissões de arquivo **não resolvem** (mesmo usuário).

### Como o fix fecha o vetor

- **Sem superfície de filesystem.** A fila é **em-processo** (`Arc<Mutex<VecDeque>>`): um processo externo não tem como escrevê-la (memória do processo do app). Um drop bem-formado no caminho FS legado é **inerte** — provado por teste (`reinject_has_no_filesystem_dropzone_fix_a3`: drop no FS → 0 injeção; fila em-processo → injeta).
- **Texto regenerado, nunca transportado.** O consumidor gera o texto com `doctrine_reinjection_text(role)` a partir do `role`; a estrutura em-processo carrega `role`, não um `payload` — nenhum texto arbitrário pode trafegar mesmo que a fila fosse alcançada.
- **Alvo por `NodeId` autenticado.** O `target` é o `NodeId` do roster cunhado **no** `assign_role` (chamado só pós-confirmação humana no M6-E), não um nome re-resolvido — nenhum campo escrito por agente decide identidade. Mantém a doutrina §54 (contrato é dado, jamais autoridade).
- **Único produtor é o app.** Só `assign_role` enfileira (UI, gate humano). Equivale ao agente receber a auto-doutrina do próprio papel que o humano acabou de confirmar — sem privilégio novo e sem caminho de agente.

## Consequências

- Persiste uma **superfície de injeção fora do router** (write ao PTY pela fila serial de escrita, human-proxy), mas agora **só alimentada em-processo** pelo `assign_role` (gate humano). A superfície de **filesystem foi eliminada** (era o vetor A3).
- **Tradeoff de durabilidade (aceito pelo Maestro, FIX-A3):** a fila em RAM **não é mais crash-durável** — um crash entre o `assign_role` e o drain **perde o aviso inline** daquela troca de papel. **Mitigação:** a mudança de papel **em si segue durável** (`NodeRoleAssigned` no log + `CLAUDE.md` reescrito no disco), então o nó renasce com o papel certo; o que se perde é só o *aviso conversacional* no PTY vivo. **Decisão explícita:** o restore do log **NÃO re-deriva** a re-injeção (não há replay de `NodeRoleAssigned` → enqueue), para **evitar re-injetar a doutrina a cada boot**. Se o aviso for crítico, o fundador re-dispara pelo M6-E.
- **Invariante para o futuro:** qualquer injeção que **não seja** self **nem** human-proxy DEVE passar pelo `deliver_a2a`/`WorkspaceTrust` (o caminho autenticado de duas camadas). **Não introduzir** nenhum canal de filesystem (ou outro IPC inter-processo) para a re-injeção — a fila é em-processo **por segurança**, não por conveniência.
- Se surgir necessidade de doutrina "sistema → nó" (não-self, não-human-proxy), as opções abertas (decisão de costura, não desta story) são: (a) introduzir um nó orquestrador **"Lina"** no roster como `from` autenticado legítimo; ou (b) uma porta dedicada `system-inject` em `lina-core` com seu próprio modelo de confiança e gate. Qualquer das duas é um ADR próprio.

## Relacionados

ADR 0006 (WorkspaceTrust default-deny / W5-5), ADR 0010 (multi-workspace trust por Espaço), ADR 0021 (modelo de segurança da aprovação y/n — a injeção de aprovação tem o seu próprio seam atômico, F1-1-8 #1, ainda pendente).
