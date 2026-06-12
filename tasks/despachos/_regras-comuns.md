# REGRAS COMUNS DA RODADA (leia antes da sua tarefa)

Você é um worker do time Lina Space executando UMA fatia da Fase 1. O Maestro valida de fora e commita.

## Protocolo (obrigatório)
1. **PRIMEIRO ATO:** `touch .iniciado-<seu-id>` na raiz do repo (sinal de vida; o id está no seu despacho).
2. **NÃO commite. NUNCA rode** `git commit/checkout/reset/stash/push`. O Maestro commita por fatia.
3. **Fronteira de arquivos é LEI:** mexa SÓ nos arquivos listados no seu despacho. Precisa de 1 linha em arquivo alheio (events.rs/router.rs/lib.rs/main.rs de outro dono)? NÃO edite — registre o pedido na sua entrega e o Maestro coordena.
4. **Reporte de status (rodada r4+):** o bug de identidade foi FECHADO (`4e1f3a8`/ADR 0026) — verbos `lina` liberados. Reporte ao Maestro com `lina ask "@Terminal A" "<status>" --intent status` ao COMEÇAR, ao TERMINAR e se TRAVAR. O arquivo de entrega continua sendo a fonte da verdade da fatia.
5. **Validação:** `cargo test -p <crate> -- --test-threads=1`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo fmt -p <crate> --check` (POR PACOTE; para o app: `cd app/lina-gpui` e rode lá). **Redirecione a saída para um arquivo e LEIA o arquivo; capture o exit code DIRETO** (`echo $?` imediatamente, sem pipe).
6. **Convenções:** edition 2021 · sem `unwrap()`/`expect()` em código de produção · eventos novos SEMPRE aditivos (`serde(default)`, replay de log antigo nunca quebra) · testes que provam o critério (não-vacuosos: o teste falha se o mecanismo for removido).
7. **Doutrina de segurança (inegociável):** nenhum campo escrito por agente (from/payload/contrato/filename/env de PTY filho) decide identidade, ordem ou autorização em caminho que o AGENTE controla; autoridade vem do app/supervisor. Se sua story tocar `deliver_a2a`/`Router`, a suíte de segurança do router precisa seguir verde.
8. **Se a fonte citada na story contradisser sua implementação: PARE e registre na entrega** — não "adapte" em silêncio.

## Entrega (obrigatório)
Escreva `tasks/epico-f1/.entrega-<seu-id>.md` com:
- O que foi feito (arquivo:linha das mudanças principais);
- Evidência de validação (comandos + exit codes DIRETOS + nº de testes passando);
- O que NÃO foi feito e por quê; pedidos de costura (arquivos alheios); riscos/achados;
- Última linha: `PRONTO` ou `BLOCKED: <motivo>`.
