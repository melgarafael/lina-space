# Despacho r5 — Terminal C (FRONTEND): F2-2-3 — identidade da fusão no chrome e nos cards

## CONTEXTO
Rodada 5 (épico vault `38` §F2-2; gate r4 fechado em ef6c7af). Esta é A story em que o Lina ganha a cara da fusão T1+temperatura-T3 NA TELA — consumidora direta do seu catálogo. O statement canônico está no épico §VIII (decisão 1); os mecanismos: cor semântica fixa acoplada (âmbar=trabalhando · verde=pronto · vermelho=precisa-de-você · azul=mensagem-do-time), superfícies quentes, flat honesto, viewer vivo SÓ no foco (periferia congelada honesta — correção da onda V).

## FUNÇÃO
Frontend — **dono único de `main.rs` nesta rodada** (o render do chrome/cards vive lá; minhas costuras esperam você devolver o arquivo). Fronteira: main.rs + src/ui/ + canvas.rs se precisar.

## DIRECIONAMENTO
1. **Cards de terminal:** estados do nó pintados pela cor SEMÂNTICA dos tokens (badge/borda/indicador — a cor do indicador É a cor do elemento acionável, OP-1); título/narração na tipografia da fusão; superfície quente.
2. **Chrome (top bar, rail, sidebar):** superfícies/texto/espacamento pelos tokens; Plex na UI; zero gradiente decorativo.
3. **Os 5 ajustes visuais pendentes da r4** (✕ secondary, modal py, primary px, paste-field raised, Ghost) — formalize-os aqui (já estão no código; esta story os ASSUME no roteiro de tela).
4. **Fraunces nos momentos** (se houver momento natural nesta story — ex.: título do onboarding/conclusão; não force).
5. NÃO toque: toast/badge de live-region (F2-2-2 do E, em paralelo em ui/toast.rs+a11y_live.rs) · palette/sidebar ranking (fechado).
6. Catraca DEVE cair ou manter; [PROF] sem regressão (rode a cena de estresse antes/depois — sonda estendida está no ar); WCAG estendido verde.
7. Validação: suíte + catraca + clippy -D warnings + fmt nos seus. Não commite. Reporte E continue.

## OBJETIVO
Abrir o Lina e VER a fusão — o fundador olha a tela e reconhece o território que escolheu nos protótipos.

## RESULTADO ESPERADO
Chrome+cards com identidade + roteiro de tela da story (lista do que o fundador deve olhar). PRONTO:/BLOCKED:.

## Tentativas anteriores
Nenhuma.
