# Despacho fix-forward — Terminal D (QA): estender a cobertura do guard 0028 aos call-sites

## CONTEXTO
Veredito de completude do Arquiteto-revisor (PASS 91) trouxe 1 achado MÉDIO de hardening: o guard `a11y_conformance.rs` varre SÓ `src/ui/` (o catálogo). Os call-sites que a migração r6 consertou — `attention_ui.rs`, `main.rs`, `canvas.rs` — estão em `src/` raiz, FORA do alcance. A migração os arrumou (verificado), mas a NÃO-REGRESSÃO não está enforced lá: um `Role::Status` cru ou dot de estado novo num call-site futuro não seria pego. É o fechamento da fronteira que o seu próprio achado 1 nomeou (catálogo fechou, call-sites abertos — e é neles que o estado é renderizado).

## FUNÇÃO
QA — dono do guard; você tem a calibração de matchers (fronteira de palavra, anti-falso-positivo).

## DIRECIONAMENTO
1. Estenda a régua **DURA** (Role::Status/Alert/Politeness sem live_region = FALHA) a `src/**.rs` OU especificamente aos 3 call-sites (`attention_ui.rs`, `main.rs`, `canvas.rs`) — sua escolha de escopo, justificada.
2. **Calibração crítica** (por isso é seu, não meu): os call-sites e o módulo a11y_live TÊM menções legítimas a `Role::Status`/`Politeness` em (a) comentários que mapeiam o mecanismo, (b) o próprio `a11y_live.rs` que DEFINE o Element, (c) a delegação de `a11y.rs:217`. A régua dura não pode cair nesses — só em USO de Role de estado SEM composição live_region. Os matchers pinados + fronteira de palavra que você já tem são a base; pode precisar de uma allowlist (a11y_live.rs como definidor; comentários ignorados).
3. Prova por mutação como sempre: inserir um `div().role(Role::Status)` cru num call-site DEVE cair; os call-sites atuais (já migrados) NÃO podem cair.
4. Validação: `cargo test --test a11y_conformance` + suíte + clippy + fmt. Não commite. Reporte E continue.

## OBJETIVO / RESULTADO ESPERADO
A invariante do 0028 enforced ONDE o estado é renderizado, não só no catálogo — fronteira fechada. PRONTO:/BLOCKED:.

## Tentativas anteriores
Guard v1 (commit 3c42a56): varre src/ui/, 9/9 por mutação. Esta estende a cobertura aos call-sites.
