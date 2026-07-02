# ADR 0056 — Vault é config do USUÁRIO (global `~/.lina/`), herdado por todo Espaço

- **Status:** **Aceito** (decisão de produto/arquitetura aprovada pelo fundador via dogfooding; Arquiteto, 2026-06-27). Implementação: Arquiteto (fatia isolada) + coordenação no `bin/lina.rs` com o dono ativo.
- **Escopo:** ONDE mora o `vault.json` (o "segundo cérebro" linkado no onboarding) e em que ORDEM ele é resolvido. O vault deixa de ser config **de projeto** (`<ws>/.lina/vault.json`, por-Espaço) e passa a ser config **de usuário** (`~/.lina/vault.json`, global), herdada automaticamente por todo Espaço — atual ou criado depois.
- **Relacionados:** ADR 0010 (multi-workspace), ADR 0022/0038 (pasta de trabalho de uma admissão), `obsidian.rs` (contrato `vault.json` — config, não evento). Invariante #2 (local-first) intacta: nada sai da máquina; só muda o diretório local onde a config do usuário vive.

## Contexto

O usuário linka o vault no passo "Segundo cérebro" do onboarding. A escrita e a leitura sempre usaram o `.lina/` do Espaço:

- **Escrita** (`SecondBrainModel::confirm`, `obsidian.rs:1872/1882`): `write_vault_config(self.lina_dir, …)` + `write_vault_index(self.lina_dir, …)` — `self.lina_dir = <ws_root>/.lina`.
- **Leitura app** (`runtime.rs:440`): `read_primary_vault(<ws_root>/.lina)` → monta o `BootstrapWriter`.
- **Leitura CLI** (`bin/lina.rs:4816`): `mailbox_root().join("vault.json")` = `$LINA_HOME/vault.json` = de novo `<ws_root>/.lina/vault.json`.

`LINA_HOME` é **por-spawn** (`runtime.rs:431`): cada terminal nasce apontando para o `.lina` do SEU Espaço — decisão CORRETA para mailbox/event-log (são estado **do Espaço**). O erro foi pendurar o **vault** na mesma régua: o segundo cérebro é do **usuário**, não do projeto. Resultado observado (dogfooding, fundador): criar um Espaço novo → `.lina/` próprio e vazio → o terminal responde *"o vault ainda não foi linkado (passo Segundo cérebro pendente)"*, mesmo com o vault já linkado em outro Espaço.

A inconsistência fica clara contra o que JÁ é global: `~/.lina/` é a casa da config de usuário — `workspaces.json`, `license.json`, `bootstrap.json`, `keygen/` moram lá. A licença é do usuário (global); os Espaços são do usuário (global); o segundo cérebro era a única peça de "config do usuário" presa a um projeto.

## Decisão

O vault (`vault.json` + `vault-index/`) é **config do usuário** e mora em **`~/.lina/`** (global). A resolução é **em camadas**, com override de projeto preservado:

```
<ws_root>/.lina/vault.json   — override de PROJETO (opcional; porta aberta, custo ~zero)
        ↓ se ausente
~/.lina/vault.json           — GLOBAL (o normal): todo Espaço herda
        ↓ se ausente
$LINA_VAULT                  — override de dev
        ↓
<ws_root>/vault              — fallback
```

### (a) Princípio (a generalização que o fundador pediu — "qualquer projeto/CLI")
**Config-de-usuário mora no global (`~/.lina/`) e é herdada por todo Espaço; config-de-projeto mora no `.lina/` do Espaço.** O vault é a primeira aplicação. Qualquer config-de-usuário futura (ex.: CLI Profile default, que hoje também vive em `<ws>/.lina/profiles` — `bridge.rs:7341`) segue o mesmo trilho sem reabrir a discussão. Isto **abre** uma porta de continuidade; não fecha nenhuma.

### (b) Escrita — onboarding grava no global
`SecondBrainModel` passa a gravar `vault.json` + `vault-index/` em `~/.lina/` (resolvido no caller de produção, `onboarding.rs:763`, via `global_lina_dir()`; o model permanece testável recebendo o dir). Re-links futuros vão para o global → nunca mais divergem.

### (c) Leitura — precedência em camadas
`read_primary_vault_effective(ws_lina_dir)` / `read_vault_config_effective(…)` compõem `[<ws>/.lina, ~/.lina]` e o primeiro com `vault.json` válido vence. Tanto o app (`runtime.rs`, `bridge.rs`) quanto o CLI (`bin/lina.rs`) usam a mesma ordem — o agente que roda `lina vault search` num Espaço novo cai no global e enxerga o vault.

### (d) Migração one-shot (sem fricção, sem perda)
No boot, `migrate_vault_to_global()` promove para `~/.lina/` o `vault.json` (+ índice) do primeiro Espaço conhecido que o tiver — **varrendo `~/.lina/workspaces.json`**, não só o Espaço atual (senão um Espaço novo aberto direto nunca acharia a fonte). Idempotente: se o global já tem vault, é no-op. É como o vault já linkado passa a valer para todos os Espaços, incluindo os criados antes desta mudança. O `heal_missing_indices` passa a apontar para o global (regenera o PageIndex onde o vault agora mora).

## Consequências

- **Resolve o sintoma na raiz:** Espaço novo herda o vault sem refazer onboarding. O agente nunca mais diz "vault não linkado" tendo o usuário um vault linkado em qualquer Espaço.
- **Menor mudança que resolve:** reusa o contrato `vault.json` (config, não evento — sem evento novo, sem migração de log); só muda ONDE mora e a ORDEM de resolução. Nenhum diretório/conceito novo — `~/.lina/` já existe.
- **Override de projeto preservado** (porta aberta): um Espaço pode ter `<ws>/.lina/vault.json` próprio e ele vence o global. Sem UI por enquanto (YAGNI) — a leitura em camadas já o suporta.
- **Local-first intacto** (inv #2): a config continua 100% local; apenas migra de `<ws>/.lina` para `~/.lina`, ambos na máquina.
- **Risco/teto:** a migração lê `~/.lina/workspaces.json` para achar a fonte. Se um vault foi linkado num Espaço que não está no registro, ele não é varrido — `ponytail:` o upgrade é incluir o Espaço atual sempre na lista de candidatos (já feito) e, se preciso, varrer `~/Library/Application Support/Lina/*/.lina`.
