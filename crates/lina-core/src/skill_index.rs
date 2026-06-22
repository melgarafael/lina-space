//! F3-5-4 · frente SKILLS (dono: Terminal J) — STUB da largada.
//!
//! Seletor de skills DETERMINÍSTICO no core (ZERO LLM): índice (nome+trigger+requires),
//! filtra pelas tools/CLIs presentes no terminal, casa o gatilho, emite `SkillSelected`.
//! ANTI-CICLO (Cargo.toml crava `bootstrap → core`): o core NÃO importa `LINA_SKILLS`.
//! Aqui mora a FUNÇÃO PURA sobre um índice RECEBIDO — ex.: `SkillIndexEntry { name,
//! trigger, requires }` + `select(index, present_tools, trigger)`. Quem POPULA o índice
//! a partir de `LINA_SKILLS` + `.claude/skills` é o caller em `lina-bootstrap`.
//! Preencha aqui; o `pub mod` já está em lib.rs (não toque lib.rs).
