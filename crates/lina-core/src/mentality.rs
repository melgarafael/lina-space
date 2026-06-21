//! Mentality por papel (F3-3 — spec 35 §3-§4): a peça de MEMÓRIA da personalidade da Lina.
//!
//! A projeção `Mentality(papel)` reconstrói o estado das crenças de cada PAPEL do roster por
//! REPLAY do event log (inv #4, ADR 0005 — padrão `CostLedger`/`intelligence_adoption`), e a
//! política de promoção decide DETERMINISTICAMENTE (sem LLM, inv #1 — contadores sobre o log)
//! quando uma correção do usuário vira crença estabelecida.
//!
//! Eventos-fonte (`events.rs` §3): `CorrectionObserved` → `BeliefProposed` (provisória) →
//! `BeliefReinforced{situation_hash}` (N distintos) → `BeliefEstablished` | `BeliefChallenged`
//! (zera) | `BeliefRetired{reason}` (refutada/`expired`/`retired_by_human`).
//!
//! Regras duras a materializar aqui (M-PROMO / Terminal I):
//! - Promoção SSE N situações DISTINTAS (hash) confirmaram — a MESMA situação 2× NÃO conta
//!   (anti-gaming §6.4); `challenged` zera o progresso; provisória sem reforço em TTL expira.
//! - `now_ms` INJETADO (nunca wall-clock embutido — protocolo time-based mede contra o passado).
//! - Seleção `top-K` por recência/uso para o injetor (gate c — cap é critério, não opção).
//! - Crença é DADO comportamental, JAMAIS autoridade (§6.1; família ADR 0007). Filtros
//!   ESTRUTURAIS (sem PII, sem claims negativos sobre CLIs/ambiente — Hermes 20 A.2) ficam aqui;
//!   o filtro semântico anti-poisoning é do Refletor (M-DETECTOR).
//! - Crença NUNCA é deletada: rebaixada/aposentada, reconstruível por replay.
//!
//! Stub do contrato (Maestro): o `mod` e o doc moram aqui para a largada; a implementação é
//! da frente M-PROMO. Cada mecanismo prova-se com controle positivo E negativo (eval-first §7).
