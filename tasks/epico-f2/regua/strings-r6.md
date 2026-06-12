# Strings finais — Rodada 6 (toolbar · toasts · badges)

> **Autor:** Redator (WRITER) · **Data:** 2026-06-12 · **Consumo:** os devs leem daqui (eu não edito `.rs`).
> **Âncora:** território F2-0-D (caloroso, honesto, sem jargão) + registro anti-alarme do `vocabulario-f2-2.md` + decisão OP-1 (cor = significado).
> **Fontes lidas:** `spec-f2-2-5-toolbar.md` (verbos) · `canvas.rs:36-100` (BadgeKind/label/bg) · `inspector.rs:37-62` · `sidebar.rs:88-96` (toasts).

---

## 0. Achado de costura (ler primeiro) — UNIFICAR o vocabulário de estado

Hoje o mesmo estado de nó aparece com **três palavras diferentes** conforme a tela:

| Estado real | Badge do card (`canvas.rs`) | Inspector (`inspector.rs`) | Selado nesta r6 (§3) |
|---|---|---|---|
| produzindo saída | "rodando" | "Ativo" / "Trabalhando" | **trabalhando** |
| vivo, terminou, espera você | "aguardando" | "Ocioso" | **pronto** |
| recém-criado | (vira "rodando") | "Iniciando" | **chegando** |

Isso é drift de vocabulário — o leigo vê "rodando" num canto e "Ativo" no outro para a MESMA coisa. **Recomendação:** os 5 rótulos do §3 viram a fonte única; quem mostra estado de nó (badge, inspector, toolbar tooltip) usa essas palavras. Não edito `.rs` — sinalizo para a r6 alinhar `inspector.rs:37-62` e `canvas.rs:label()`.

---

## 1. Toolbar — 4 verbos finais

| Ícone | Rótulo final | Tooltip (1 linha leiga) | aria_label | Variante |
|---|---|---|---|---|
| ⚑ | **Ver o pedido** | "Leva você até quem está esperando uma resposta sua." | "Ver o pedido pendente" | `Confirm` |
| ✎ | **Editar** | "Muda o nome, o papel ou as instruções deste agente." | "Editar agente" | `Secondary` |
| ⤢ | **Centralizar** | "Traz este terminal para o centro da tela." | "Centralizar na tela" | `Secondary` |
| ✕ | **Encerrar** | "Fecha este terminal. Nada do trabalho se perde." | "Encerrar terminal" | `Destructive` |

**Porquês (onde a escolha não é óbvia):**
- **⚑ `Atender` → `Ver o pedido` (TROCA — a correção central da r6):** `GotoAtencao` só **navega** até o nó com gate pendente; aprovar/recusar é um passo SEPARADO depois (⌘⏎ / ⌘⇧⏎). "Atender" sugere *conceder o pedido* — o leigo poderia clicar achando que já decidiu. "Ver o pedido" promete só o que a ação faz: te leva lá. Pareia com o badge "precisa de você" (você vê quem precisa → você decide).
- **✕ `Encerrar` (não "Fechar"/"Excluir"):** "Encerrar" é definitivo sem ser violento; "Excluir" mentiria (o log guarda tudo — replay recupera). O "nada do trabalho se perde" no tooltip é a promessa anti-alarme que desarma o medo de apertar o botão vermelho.
- **✎ `Editar`:** sela o verbo. **Alinhar:** o overlay `?` (`vocabulario-f2-2.md` §3) dizia "Ajustar este agente" — trocar para **"Editar este agente"** para bater com a toolbar (uma ação, um verbo, em toda a superfície).
- **⤢ `Centralizar`:** já idiomático e consistente com o overlay. Mantido.

---

## 2. Toasts — existentes + padrão para os novos

**Padrão de todo toast (selado):** `fato` + (opcional) `próximo passo`. Nunca alarme, nunca "ERRO"/"FALHA". Voz de colega que avisa, não de sistema que grita.

### 2.1. Existentes em produção (levantados por grep)

| Local | Copy atual | Veredito | Final |
|---|---|---|---|
| `ui/toast.rs` (copiar p/ área de transferência) | **"Copiado"** | **MANTER** | "Copiado" |
| `sidebar.rs:93` (arquivar Espaço — live-region) | «Espaço "{nome}" arquivado — nada se perde. Desfazer disponível por alguns segundos.» | **MANTER** | (igual) |
| `sidebar.rs:88` (botão do toast) | **"Desfazer"** | **MANTER** | "Desfazer" |
| `sidebar.rs:50` (link rodapé) | **"Espaços arquivados ▸"** | **MANTER** | "Espaços arquivados ▸" |

> Os demais `Toast::new(...)` no grep são fixtures de teste ("x", "t{i}", "/tmp/sb-arch") — não são copy de produção. Nada a revisar neles.

A copy de arquivamento já é exemplar do padrão: fato («arquivado»), garantia anti-alarme («nada se perde») e próximo passo («Desfazer»). É o modelo para os toasts novos abaixo.

### 2.2. Toasts prováveis da r6 (proposta, prontos para o dev plugar)

Alinhados aos 5 estados de Poder do `vocabulario-f2-2.md` §2 e ao tom acima:

| Evento | Toast (fato + passo) | Ação (se houver) |
|---|---|---|
| Poder ativado | «Poder ativado: {nome}.» | — |
| Poder desativado | «Poder desativado: {nome}.» | "Reativar" |
| Poder deu erro em uso | «{nome} parou no meio — não é culpa sua.» | "Tentar de novo" |
| Poder precisa de configuração | «{nome} precisa de um passo pra funcionar.» | "Resolver" |
| Terminal encerrado | «Terminal encerrado. Nada se perde.» | "Desfazer" |
| Agente chegou | «{nome} entrou no Espaço.» | — |

**Porquê:** cada um é `fato` curto + `passo` só quando há algo a fazer. "não é culpa sua" e "nada se perde" são as duas muletas de calma da casa — usar sempre que o evento puder assustar.

---

## 3. Badges de estado do card — 5 palavras finais

Alinhadas a `canvas.rs:BadgeKind`, ao `vocabulario-f2-2.md` e à decisão OP-1 (cor = significado; "precisa de você" usa o MESMO warning do gate de custódia).

| BadgeKind | Palavra final | Com saída nova | Cor (token, OP-1) | Porquê |
|---|---|---|---|---|
| `NeedsYou` | **precisa de você** | "precisa de você" | `state.warning` (= gate) | Já perfeito. A única cor "quente" de atenção — é o que vence tudo. Mantido. |
| `Running` | **trabalhando** | "trabalhando · {n} novas" | `accent.confirm` | Troca "rodando"→"trabalhando": "rodando" é máquina; "trabalhando" é colega. Território quente pede a forma humana. |
| `Waiting` | **pronto** | "pronto · {n} novas" | `surface.raised_alt` (neutro) | Troca "aguardando"→"pronto": o nó terminou e a vez é sua. "aguardando" soa a carregando/travado; "pronto" diz "é com você". Neutro, NÃO alarmante. |
| `Stopped` | **encerrado** | "encerrado" | `surface.danger_muted` (fosco) | Mantido. Danger FOSCO (não vivo) — fim sereno, não emergência. |
| *(Starting/novo)* | **chegando** | — | `accent.confirm` (= trabalhando) | Estado novo de superfície: um terminal recém-criado está "chegando" à mesa (metáfora "estar no Espaço = estar no time"). Mais quente que "Iniciando". **Nota dev:** hoje Starting cai em `Running`/"trabalhando"; se quiserem distinguir o recém-nascido, precisa de um `BadgeKind::Arriving` — decisão de vocês; a palavra está selada. |

**Concordância:** todas concordam com "o terminal/agente" (masc.): "pronto", "encerrado", "chegando". Uniforme.

---

## 4. Resumo para quem consome

1. **Toolbar:** `Ver o pedido` (era "Atender" — só navega, não aprova) · `Editar` · `Centralizar` · `Encerrar`. Tooltips e aria_labels na tabela §1.
2. **Toasts:** padrão `fato + passo, nunca alarme`. Os 2 de produção ("Copiado", arquivar) ficam; 6 novos propostos para a r6 (§2.2).
3. **Badges:** **precisa de você · trabalhando · pronto · encerrado · chegando** — selados como fonte única; alinhar `canvas.rs` ("rodando"→"trabalhando", "aguardando"→"pronto") e `inspector.rs` ("Ativo/Ocioso/Iniciando"→trabalhando/pronto/chegando).
4. **Costura:** unificar o vocabulário de estado nas 3 telas (§0) e alinhar o verbo "Editar" no overlay `?`.

**PRONTO:** strings-r6.md entregue — 4 verbos da toolbar (com a troca Atender→Ver o pedido, o rótulo deixa de prometer aprovação), revisão dos toasts de produção + 6 novos no padrão anti-alarme, e os 5 badges selados como fonte única com alinhamento sinalizado para canvas.rs/inspector.rs. Fronteira respeitada (só este arquivo, sem .rs). Achado de costura: 3 vocabulários de estado divergentes unificados num só.
