# Despacho F3-VAL · G (UI) — auditoria do card da Goal por CÓDIGO (zero-jargão / identidade / gestos)

**LEIA ANTES (puxe o contexto):**
- `tasks/epico-f3/onda-f3-val.md` (plano desta rodada).
- Spec 52 §"Superfície para o leigo" (nota 52:341). Índice do vault **stale** p/ a 52 — path direto: `/Users/rafaelmelgaco/Documents/Obsidian Vault/Ecossistema Labs - Operação/Debriefing Vibe Coding/52 - SPEC Goal-and-Loop - A Meta como Primitiva.md`.
- Skills de doutrina da casa: `lina-design-doctrine`, `lina-copy-doctrine`.
- O card: `app/lina-gpui/src/goal_card.rs` (`RenderOnce` em :433) + `app/lina-gpui/src/main.rs::render_goals_panel` (:2123, varre `lina_core::project_goals` — projeção VIVA, não o `mock_goal`).

## 1. CONTEXTO
O gate (h) bloqueante das duas ondas é **a tela**. O computer use por **screenshot está bloqueado por TCC** (o terminal roda dentro do Lina.app, sem permissão de Screen Recording). Logo, a prova **determinística** do zero-jargão / identidade / gestos é por **inspeção do render**. O Maestro fecha o "olho no pixel" à parte.

## 2. FUNÇÃO
Você é o **dono do card da Goal** (`app/lina-gpui`), atuando como **auditor visual por código** (leitura; correção só se eu pedir, e aditiva).

## 3. DIRECIONAMENTO (fronteira + regras)
- ⚠️ `main.rs` está **SUJO com WIP do Terminal A** (ADR 0037/0038). **NÃO toque, NÃO reverta** nenhuma linha do A. Sua entrega é um **relatório**; se houver fix, é aditivo, disjunto e **coordenado comigo** (não commite).
- Auditoria é **READ** de `goal_card.rs` + `render_goals_panel` (e helpers de copy/token que eles chamam).

## 4. OBJETIVO
Garantir que, quando o fundador olhar o card, ele lê **linguagem de dono de negócio**, não de engenheiro — a promessa não-técnico-first (invariante #6).

## 5. RESULTADO ESPERADO (formato exato — relatório por gate)
- **Zero-jargão (PASS/FAIL):** prove que NENHUM destes chega à tela do usuário — `goal_id`, `root_cause_id`, `ReviewVerdict`, `effort` cru, `check_kind`, `NodeId`/uuid, `Pass`/`Fail` em inglês, `{:?}` de enum. Se vazar, cite `arquivo:linha`.
- **Identidade (PASS/FAIL):** fonte de destaque ≠ corpo; tokens semânticos (não `Inter`/`px(n)` por inércia). Liste os tokens/famílias usados.
- **Gestos (PASS/FAIL):** botões "Sim, é isso" / "Quero ajustar" (ADR 0036) presentes e ligados ao canal `human_intent` (não a um `by` escolhido pela view).
- **Cobertura de fases:** o card renderiza Defined / Interpreted / Confirmed / Decomposed **e** o estado de escalada (`GoalEscalated` → aviso leigo "tentei 3x, quer um time mais reforçado?")? Liste fase→tratamento.

Qualquer jargão vazando ou fase sem render = **GAP** (reporte, não conserte). Termine com **`PRONTO: <resumo por gate>`** ou **`BLOCKED: <motivo>`**.
