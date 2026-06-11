# Despacho M8-T4 — Sidebar M8 (rail esquerdo expansível) · rodada 2

> Rodada M8-r2 · 2026-06-11 · Maestro externo → Terminal #34 (FRONTEND)
> Pré-requisitos NO CHÃO antes de começar: `.entrega-m8-t1.md` (WsRuntime) e `.entrega-m8-t2.md`
> (mini-status) com `PRONTO:`. Leia os DOIS primeiro — os contratos finais estão lá.
> Marque início: `echo INICIADO > "/Users/rafaelmelgaco/einstein workspace/lina-space/.iniciado-m8-t4"`

## CONTEXTO
Pedido direto do fundador: Espaços "multi-tenant" à la Maestri numa SIDEBAR LATERAL EXPANSÍVEL,
lembrando os terminais de cada Espaço. Spec congelada linha-a-linha:
`tasks/epico-f1/spec-m8-m9-fiacao.md` §1 (gramática da linha: `▣ {nome} {N} Agentes ● ~?$ {valor}
🔔 ⌘{n}` — moeda PENDENTE-MAESTRO: use «—»/US$ conforme §6-A1 variante (a)), §4 (a11y M8) e §5
(critérios). Strings 100% literais de `tasks/epico-f1/copy-f1-4.md §5` (mapa com transcrições:
`/tmp/mapas-m8/gallery-m9.md`). Padrões de render/modal/tema do app: `/tmp/mapas-m8/ui-render.md`
(WorkspaceView, theme::active(), atalhos, precedência de teclado).

## FUNÇÃO
Frontend gpui: componente novo, estado headless testável separado do render (padrão attention_ui).

## DIRECIONAMENTO
1. `cd "/Users/rafaelmelgaco/einstein workspace/lina-space"`.
2. Crie `app/lina-gpui/src/sidebar.rs` com DUAS camadas (padrão do app — testável headless):
   - **Estado gpui-free:** `pub struct SidebarState { pub expanded: bool, pub query: String,
     pub selected: usize, pub rows: Vec<SidebarRow> }` + `pub struct SidebarRow { pub name: String,
     pub ws_root: PathBuf, pub focused: bool, pub status: dashboard::WorkspaceMiniStatus,
     pub shortcut_index: Option<usize> }` + `pub fn build_rows(registry_entries, varredura_fallback,
     active_root, cache: &MiniStatusCache) -> Vec<SidebarRow>` — fonte canônica = WorkspaceRegistry;
     varredura é fallback; Espaço que não projeta vira linha-⚠ `unreachable` (NUNCA some — §1).
     Filtro de busca substring case/acento-insensível (função pura).
     ⌘{n} = índice ESTÁVEL por ordem de criação no registry (NUNCA posição visual — §1).
   - **Render gpui:** rail esquerdo colapsado (ícones ▣ por Espaço, largura ~52px) ↔ expandido
     (~280px, busca no topo + linhas da gramática §1 + «Espaços arquivados ▸» + item
     `+ Novo Espaço…`). Tokens SÓ de `theme::active()` (lint barra rgb literal). Linha focada com
     `focus.ring`. Tooltip honesto por célula (sem dado → omite fragmento, não lê "zero" — §4).
3. Interações (callbacks injetados — a fiação real é do Maestro na integração):
   `on_switch(ws_root)`, `on_create()`, `on_rename(ws_root, novo)`, `on_archive(ws_root)`.
   Exponha-os como campos de closure no componente; NÃO implemente a troca aqui.
4. A11y §4: roving ↑↓, Enter troca, Esc fecha sem trocar, aria-label = concatenação dos
   fragmentos congelados; selo «PRO» no item criar quando o caller mandar `create_blocked=true`.
5. TDD da camada headless: build_rows (canônico+fallback+⚠), filtro com acento, índice ⌘n
   estável com lista reordenada, dominância exibida vinda do mini-status.

## FRONTEIRA DE ARQUIVOS (exclusiva sua nesta rodada)
- `app/lina-gpui/src/sidebar.rs` (novo). Mais NADA. A fiação no main.rs (mount do rail, atalhos
  ⌘O/⌘1..9, switch real) é do MAESTRO na integração — declare na entrega exatamente o que o main
  precisa chamar.
- **NÃO COMMITE.**

## OBJETIVO
Sidebar completa e honesta, compilando com o app, suíte verde + clippy `-D warnings` + fmt EXIT=0
(exits diretos), camada de estado 100% testada headless.

## RESULTADO ESPERADO
`.entrega-m8-t4.md`: assinaturas públicas, o contrato de integração ("o main chama X no mount, Y
no atalho, Z no switch"), strings usadas (cite o § da copy), evidência dos exits + testes novos.
Termine com `PRONTO:` ou `BLOCKED: <motivo>`.
