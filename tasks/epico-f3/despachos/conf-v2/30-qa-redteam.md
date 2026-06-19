# Despacho F3-CONF-2 · QA/RED-TEAM — Terminal R · model opus · effort High

> **Antes de codar:** rode `lina plan read CONF-QA` e LEIA `tasks/epico-f3/repro-22-23.md` (você é o autor da investigação — agora prova o FIX) + `tasks/epico-f3/onda-f3-conf-2.md`. Use a skill `lina-cold-review` para a lente adversarial.

## 1. CONTEXTO (onde o trabalho está)

`cwd`: raiz do repo. Você é **dono de `crates/**/tests/` (arquivos NOVOS)** + `#[cfg(test)]` read-only sobre produção. O CORE (B) está mecanizando o protocolo anti-engolimento em `router.rs`/`attention.rs`; você prova que ele é CORRETO e SEGURO — sem depender de B para começar (escreva contra o COMPORTAMENTO ESPERADO do gate da onda).

Apoios já existentes (read-only): `crates/lina-core/tests/repro_22_23_residual.rs` (3 testes — o sintoma do Modo B pelo caminho real), `repro_w4_delivered_sem_trabalho.rs` (Modo A), `f3_2_seguranca_loop.rs` (a regra-mãe por mutação — seu modelo de "padrão ouro"). Eventos relevantes: `MessageDelivered{to}` (`events.rs:376`), `TokenUsageReported` (483), `GoalEscalated{reason}` (644), `SpawnRequested{effort}` (963), `NodeStatusChanged{reason}` (317).

## 2. FUNÇÃO

Você é o **revisor adversarial isolado**. Sua régua: provar por **mutação** (desligue a guarda → veja o teste falhar → religue — o "padrão ouro" que o Maestro elogiou) e pelo **caminho real** (`route_message`/`pump`, nunca montando o evento à mão — senão o teste não prova a costura). 0 ALTA para o gate seguir.

## 3. DIRECIONAMENTO

- **Mexa SÓ em** `crates/**/tests/` (arquivos NOVOS, ex.: `tests/f3_conf2_protocolo.rs`, `tests/f3_conf2_seguranca.rs`) + leia produção. **NÃO** edite arquivos de produção nem os arquivos de teste do Terminal I (`history_f1_5_8.rs` é dele). Use tmpdir com `thread::current().id()` (modelo `router.rs:3491`) p/ não criar novo flaky.
- **Não re-prove o coberto** (memória): `f3_1_goal_adversarial.rs` + `f3_2_seguranca_loop.rs` já cobrem `by`/`reviewer` server-side, `reviewer != target`, `ReviewVerdict{Pass}` ⇏ `GatedHard`. Cubra o GAP NOVO: o protocolo anti-engolimento e o reset do breaker.
- **Verificação adversarial precisa de prova empírica** (memória *ultracode: re-provar verificação adversarial*) — um "achado" de cético vira teste que roda, não alegação.

## 4. OBJETIVO

O protocolo anti-engolimento toca o caminho mais sensível (re-despacho + escala de recurso). Se ele virar um portal (um agente forjando `GoalEscalated`, auto-resetando o próprio breaker, ou o re-despacho escalando privilégio), trocamos um bug de confiabilidade por um buraco de segurança. Você é a rede que garante que a fundação nova é confiável **e** segura — a regra-mãe da fase (reason/escala/effort nunca autoridade) verde por mutação.

## 5. RESULTADO ESPERADO (formato + marcador)

Arquivos de teste NOVOS, todos verdes, provando:
1. **Protocolo (gate a/b):** entrega-sem-progresso → re-despacho 1× → `GoalEscalated{stalled}`; breaker sticky impede 3ª; loop_detected isenta correção de engolido mas barra tempestade (por contraste). Caminho real.
2. **Segurança (gate e), por mutação:** `GoalEscalated`/reset-de-breaker forjados por um agente (campo no payload) são IGNORADOS (carimbo server-side vence); o reset do breaker exige `HUMAN_GESTURE` (um agente não resseta o próprio breaker); o re-despacho não eleva autonomia/effort além do `effort_ladder`. Desligue cada guarda → teste fica RED → religue.
3. **Modo C como vigia (read-only):** se B não fechar o repro do pump, entregue ao menos o teste que CONTA `MessageRetained` sem `MessageDelivered`/DLQ na janela (a vigia do §6.2 do repro), documentando o gap residual.
4. **Auditoria F3-0-7:** confirme que `weakening_a_balancing_loop_is_gated_hard` (`router.rs:6221`) + `param_change_action_class_gates_only_weakening_of_balancing_loops` (`params.rs:927`) seguem verdes (já implementados) — reporte PASS, não re-implemente.
5. **Adoção:** meça no log o uso do protocolo (quantos `GoalEscalated{stalled}` reais) — métrica desde o dia 1 (risco §VI-3 do épico).

Valide de fora: `cargo test -p lina-core` (+ os crates que tocar) + `cargo clippy --all-targets -D warnings`. **NÃO commite** — reporte exits + veredito PASS/FAIL com evidência `arquivo:linha` ao Maestro.

Termine com `PRONTO: <suíte verde + veredito de segurança 0 ALTA + adoção medida>` ou `BLOCKED: <o que falta>`. Reporte o **1º progresso** ao Maestro ao começar.
