# Inventário F1-2-5 — Resíduos de UI de dev/teste na superfície

**Story:** F1-2-5 · Faxina: remover resíduos de UI de dev/teste da superfície
**Fonte:** `ondas-2-4.md` linhas 102-118 · invariante **#6** ("zero jargão na superfície")
**Autor:** Especialista em Telas · **Data:** 2026-06-11
**Como ler:** o fundador percorre item a item no gate. Cada elemento de dev/teste recebe **UMA**
decisão: **REMOVER** · **ESCONDER atrás de `LINA_DEV=1`** · **PROMOVER a feature legível**.
Observabilidade INTERNA (logs/`eprintln`/sonda stderr) **FICA** — só sai da SUPERFÍCIE.

---

## Veredito-resumo

A superfície (percurso T0→T8 que o fundador grava no gate) **já está limpa de jargão cru**: zero
UUID/`NodeId` cru renderizado, zero label "DEBUG/DEV/TEST", zero caminho de arquivo interno, zero
métrica crua sem rótulo amigável. O invariante #6 já estava sendo respeitado pelos donos de cada
tela. O **único** resíduo de dev real era de natureza diferente do que a fonte previa: painéis de
diagnóstico **auto-abrindo no boot** via flags de env **ad-hoc e inconsistentes**, nenhuma sendo o
flag canônico `LINA_DEV` que a story pede. A faxina padronizou isso e provou o gate por teste.

**Achado que contradiz a fonte (rule 8 — registrado, não "adaptado em silêncio):** a fonte
(`ondas-2-4.md:106`) cita *"módulo `dev_tools.rs` presente no shell"* como exemplo de resíduo. Isso
está **desatualizado**: hoje `dev_tools.rs` é uma **feature legítima de onboarding** (a tela que
detecta/instala git, GitHub CLI, Node.js, Vercel CLI e Python para o leigo que vai começar a
programar) — texto 100% PT-BR, zero jargão. **NÃO é resíduo; não foi removido.** Ver item 7.

---

## Itens (cada um com decisão)

### 1. Painel "Atividade e Custos" (Dashboard) — auto-abre no boot via `LINA_DASH=1`
- **Onde:** gate em `app/lina-gpui/src/main.rs:658-663` (`dashboard_open: matches!(env LINA_DASH, "1")`);
  lógica/render em `dashboard.rs` (gpui-free) + `main.rs::render_dashboard`.
- **O que é:** o painel em si é **feature de usuário** (estado/custo/atividade por terminal, em
  linguagem leiga: "Trabalhando"/"Ocioso"/"Sem resposta"; custo com marcador honesto `~ (estimado)`).
  Em produção abre **sob demanda** (paleta `⌘K` / engrenagem). O env `LINA_DASH=1` só faz ele
  **auto-abrir no boot** para os roteiros de validação por dados do fundador.
- **Resíduo:** o auto-open no boot é override de dev por flag **ad-hoc** (`LINA_DASH`), fora do
  canônico `LINA_DEV`.
- **DECISÃO: ESCONDER atrás de `LINA_DEV=1`** (mantendo `LINA_DASH` por back-compat).
- **Status:** **COSTURA** — o gate mora em `main.rs` (EXTERNO, dono Maestri). Hunk proposto abaixo
  (§Costura). Não editado.

### 2. Painel de Persistência / Ajustes / Espaços / Recuperação — auto-abre no boot via `LINA_PERSIST_PANEL`
- **Onde:** `app/lina-gpui/src/persistence_ui.rs::should_show()` (era `LINA_PERSIST_PANEL=1|true|force`).
- **O que é:** o painel é **feature de usuário** ("chrome de confiança": Tema/acentos/som = T7,
  troca de Espaço = T6, recuperação pós-crash = T8). Em produção abre sob demanda (engrenagem / `⌘,`
  / paleta — fix F1-2-1). O env só fazia ele **auto-abrir no boot**.
- **Resíduo:** auto-open por flag **ad-hoc** (`LINA_PERSIST_PANEL`), fora do canônico `LINA_DEV`.
- **DECISÃO: ESCONDER atrás de `LINA_DEV=1`** (com `LINA_PERSIST_PANEL` mantido por back-compat).
- **Status:** ✅ **APLICADO** (arquivo meu). `should_show()` agora honra `LINA_DEV` **ou** o legado;
  extraída a decisão pura `should_show_from(lina_dev, persist_panel)` + helper `is_dev_flag_on`.
  **Teste não-vacuoso** `dev_panel_closed_by_default_opens_only_with_flag` cobre o AC#3.

### 3. Sonda de performance `[PROF]` — `LINA_PROF`
- **Onde:** `app/lina-gpui/src/prof.rs` (fora da minha fronteira; só inspecionado).
- **O que é:** decomposição de frametime impressa em **stderr** (`[PROF] ...`). **NÃO renderiza nada
  na UI.** Desligada por padrão (1 bool-check/frame).
- **DECISÃO: MANTER como está** — é **observabilidade INTERNA** (a story manda explicitamente:
  "logs/eprintln FICA — só sai da SUPERFÍCIE"). Não é resíduo de superfície.

### 4. Sonda `[DASH]` (latência evento→card) — stderr
- **Onde:** `main.rs::render_dashboard` (doc em `main.rs:953-957`).
- **O que é:** log de latência em stderr para o Maestro validar por dados. Não renderiza na UI.
- **DECISÃO: MANTER** — observabilidade interna.

### 5. Onboarding (T0→T3) — `LINA_ONBOARDING`
- **Onde:** `onboarding.rs::should_show` (`decide_show`, puro e testado).
- **O que é:** **feature legítima** (boas-vindas, check-up de assistentes, ferramentas de dev,
  segundo cérebro, provedor, Espaço). Texto sem jargão. O env é override de dev/teste para
  **forçar/pular** a tela; em produção decide pelo progresso salvo (1ª execução).
- **DECISÃO: MANTER** — não é resíduo; é a 1ª experiência do leigo. O override por env é legítimo
  (mesmo idioma `1|force|true`) e não vaza para a superfície.

### 6. Galeria de Focos (`gallery.rs`) e Inspetor de Nó (`inspector.rs`)
- **O que é:** módulos-biblioteca **puros, `#![allow(dead_code)]`, AINDA NÃO wirados ao render** do
  shell (aguardam o wiring T3/P4). Labels já em PT-BR limpo ("App", "Pesquisa & Conteúdo", "Em
  Branco"). **Não aparecem na superfície hoje.**
- **DECISÃO: MANTER** — nada renderizado = nada a esconder. Quando forem wirados, os rótulos já
  estão livres de jargão.

### 7. Tela "Ferramentas de Desenvolvimento" (`dev_tools.rs`)
- **O que é:** **feature legítima de onboarding** (instala git/gh/node/vercel/python p/ o leigo).
  Texto PT-BR, rótulos amigáveis ("GitHub CLI", "Node.js"), zero jargão. **Reusa** o instalador do
  onboarding.
- **DECISÃO: MANTER** — **NÃO é resíduo**, apesar de a fonte citá-lo como exemplo. Contradição da
  fonte registrada no veredito-resumo (rule 8).

### 8. Render de `node_id` na fila de atenção (`attention_ui.rs:760`) — **falso positivo**
- **O que é:** `.child(text!(item.node_id.clone()))` no painel compacto da fila (sininho).
- **Por que NÃO é resíduo:** neste app o `node_id` **É o nome amigável do agente** (ex.: "Ajudante
  Dev", "Maestro (2)"), não um UUID — confirmado em `dashboard.rs:235` (chave = NOME do nó) e
  `dashboard.rs:1111` (`node_id: "Ajudante Dev".into()`). Renderizar é transparência legível, não
  jargão.
- **DECISÃO: MANTER** (arquivo EXTERNO de qualquer modo). Listado para o fundador não tropeçar nele.

---

## Costura (arquivos EXTERNOS — hunk proposto, NÃO editado)

### C1 · `app/lina-gpui/src/main.rs:658-663` — canonizar o auto-open do dashboard sob `LINA_DEV`
Hoje:
```rust
// LINA_DASH=1 abre o painel no BOOT (validação por dados/roteiro do fundador,
// mesmo idioma de LINA_DEMO). Produção: fechado; abre pela paleta (Cmd+K).
dashboard_open: matches!(
    std::env::var("LINA_DASH").ok().as_deref().map(str::trim),
    Some("1")
),
```
Proposto (aditivo — `LINA_DEV` passa a também auto-abrir; `LINA_DASH` mantido por back-compat):
```rust
// LINA_DEV=1 (flag canônico de dev, F1-2-5) OU LINA_DASH=1 (legado) abrem o painel no BOOT
// para a validação por dados do fundador. Produção: fechado; abre pela paleta (Cmd+K).
dashboard_open: {
    let on = |k: &str| matches!(
        std::env::var(k).ok().as_deref().map(str::trim),
        Some("1") | Some("true") | Some("force")
    );
    on("LINA_DEV") || on("LINA_DASH")
},
```
**Por que costura:** `main.rs` é EXTERNO (Maestri). Quando a janela de `main.rs` abrir p/ a fiação
interna, aplicar este hunk fecha o item 1 sob o flag canônico.

---

## Cobertura dos critérios de aceite

| AC | Estado |
|---|---|
| **1.** T0→T8 sem elemento de dev/teste visível | ✅ Superfície já limpa (itens 1-8 auditados; painéis de dev não auto-abrem sem flag). |
| **2.** Com `LINA_DEV=1` as ferramentas de dev voltam | ✅ `persistence_ui::should_show()` honra `LINA_DEV`; hunk C1 estende ao dashboard. |
| **3.** Teste falha se o painel dev montar sem a flag | ✅ `dev_panel_closed_by_default_opens_only_with_flag` (provado não-vacuoso por mutação). |
| **4.** Fundador percorre sem apontar resíduo | ⏳ humano — este inventário é o roteiro do gate. |
