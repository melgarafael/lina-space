//! F4-0-1 · frente CANAIS (dono: Terminal B — Ultra Code) — **registro de canais**.
//!
//! STUB DA LARGADA F4-0 (preencher nesta frente). Um `Channel` é uma **porta declarada** (âncora
//! de continuidade do doc 01 §3): a abstração que permite WhatsApp/e-mail/ferramentas entrarem como
//! impls de trait + manifesto, sem re-arquitetar o core (invariante #7 core/shell split).
//!
//! ## O que esta frente entrega (critérios F4-0-1, épico 41 §III)
//! - **trait `Channel`** — porta de continuidade: `id()`, acesso ao manifesto, `transport`/`auth`/
//!   `scopes`/`tools.default_enabled`/`install.ref`. O core NÃO importa tipo de canal concreto.
//! - **manifesto declarativo** (`channels/<nome>/manifest.toml`) lido e validado por `serde` ANTES de
//!   qualquer código de canal executar (gate de segurança barato, doc 40 §4/§6). Manifesto malformado
//!   → falha no schema (teste de schema), **nunca executa**.
//! - **`ChannelRegistry`** — projeção pura por replay de `ChannelRegistered` (padrão `ClueSet`/
//!   `CostLedger`: o último registro de um `channel` vence). `register_channel(...)` emite o evento.
//! - **trust default-deny por pertencimento** (espelha ADR 0006): registrar ≠ conectar; o canal nasce
//!   "declarado, não conectado". `trust_tier ∈ {core,curado,comunidade}` (F4-0-6 reusa este campo).
//!
//! ## Invariantes inegociáveis (red-team do gate vai re-derivar no código)
//! - **Manifesto é DADO, JAMAIS autoridade.** O gate de qualquer efeito externo é o broker
//!   (`broker.rs`, F4-0-3) + custódia de segredo (ADR 0004) — nenhum campo do manifesto (nome,
//!   transport, scope) decide identidade/ordem/autorização.
//! - **`install_ref` PINADO** (SHA/tag), nunca HEAD flutuante (doc 40 §6: ref explícito).
//! - **Projeção do log (inv #4):** `from_records` PURA sobre `EventRecord`s; replay reconstrói
//!   byte-a-byte (sem relógio, sem I/O); eventos indecodificáveis são pulados (projeção é derivada,
//!   não validadora do log — espelha `Mentality`/`ApprovalExecutor::replay`).
//!
//! Evento consumido/emitido: `DomainEvent::ChannelRegistered { channel, manifest_ref, trust_tier,
//! install_ref }` (congelado na largada — não re-editar `events.rs`).
