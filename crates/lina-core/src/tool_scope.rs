//! F4-0-4 · frente CONTEXTO (dono: Terminal M) — **pré-config de ferramentas/grupos por projeto**.
//!
//! STUB DA LARGADA F4-0 (preencher nesta frente). Prima concreta das "pistas" (`clue.rs`, F3-5-6):
//! o leigo DECLARA, por projeto, quais ferramentas/grupos a IA pode enxergar (doc-fonte 63) — vira
//! **contexto declarado** (Meadows ponto [6] Fluxo de informação), que uma camada do briefing lê.
//!
//! ## O que esta frente entrega (critérios F4-0-4, épico 41 §III)
//! - **projeção `ToolScopeSet`** — por replay de `ToolScopeDeclared`/`ToolScopeRevoked` (padrão
//!   `ClueSet`: o último vence; `Revoked` RETRAI a chave, sem variante de remoção extra).
//! - **`declare_tool_scope(...)` / `revoke_tool_scope(...)`** — emitem os eventos (congelados na
//!   largada). Remover a declaração → o acesso some **no próximo turno** (push de evento, sem
//!   restart — doc 40 §9).
//! - **camada de briefing** que injeta "você me deu acesso a <ferramenta/grupo X>" no contexto do
//!   terminal daquele projeto (coordenar com `briefing.rs` — costura de I; ver despacho).
//!
//! ## Invariante-mãe (red-team do gate re-deriva no código)
//! - **Declarar ≠ AUTORIZAR.** Declarar uma ferramenta a EXPÕE à leitura da IA (fluxo de contexto);
//!   **não** autoriza ação externa — essa continua passando pelo broker (`broker.rs`, F4-0-3) +
//!   gate humano + custódia (ADR 0004). Nenhum campo de `ToolScopeDeclared` entra no caminho de
//!   identidade/permissão (ponto [6] regra de fronteira: mostrar ≠ autorizar).
//! - **default-deny:** ferramenta/grupo NÃO declarado é INVISÍVEL à IA (espelha ADR 0006).
//! - **Projeção do log (inv #4):** `from_records` PURA; replay reconstrói byte-a-byte; eventos
//!   indecodificáveis são pulados (derivada, não validadora — espelha `Clue`/`Mentality`).
//!
//! Eventos: `DomainEvent::ToolScopeDeclared`/`ToolScopeRevoked { project, channel, scope }`
//! (congelados na largada — não re-editar `events.rs`).
