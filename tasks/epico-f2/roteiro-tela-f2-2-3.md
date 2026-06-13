# Roteiro de tela — F2-2-3 (identidade da fusão no chrome e nos cards)

> Para o fundador olhar a tela e **reconhecer o território** escolhido nos protótipos (T1+temperatura-T3).
> Cada item: o que olhar · o que deve estar verdadeiro · de onde veio.

## 1. Cards de terminal — cor SEMÂNTICA fixa (OP-1)
A cor do **indicador** (a bolinha no título) e da **borda** É a cor do que o nó significa, universal no app:
- [ ] **Pronto** (terminal Idle, sem pedido) → indicador **verde** (`state.success`).
- [ ] **Trabalhando** (Busy/Running) → indicador **âmbar** (`state.warning`).
- [ ] **Precisa de você** (gate de custódia pendente p/ o nó) → indicador **vermelho** E a **borda do card inteiro fica vermelha** (`state.danger`), mesmo na periferia — o card que pede você SALTA. *(Este é o OP-1: "precisa de você" vence o status; antes mostrava verde/âmbar e o vermelho só aparecia tarde.)*
- [ ] **Encerrado** (Dead/Crashed) → indicador **vermelho**.
- [ ] Card em **foco** (sem pedido pendente) → borda no **acento do usuário** (`focus.ring`).
- [ ] Título do card e chips (autonomia, ✎ Editar, avisos) em **IBM Plex Sans** (não a fonte default), na escala de token.

## 2. Momento Fraunces — "Seu Espaço está pronto"
- [ ] Abra um Espaço SEM terminais (ou ⌘N de um fresco): a tela de boas-vindas mostra **"Seu Espaço está pronto" em Fraunces** (serifa display) — a ÚNICA aparição de Fraunces, no momento de acolhimento. A instrução "Pressione ⌘N…" abaixo é Plex (UI). *(Fraunces é a "decoração" autorizada: um momento, nunca rótulo recorrente.)*

## 3. Cards de artefato (nota/pasta)
- [ ] Ícone + nome + tipo em **Plex**, escala de token. Sem mudança de tamanho (tokenização exata 15/11/40).

## 4. Painel de atividade/custos (dashboard)
- [ ] Cabeçalho, total "Hoje:", linhas por terminal e atividade em **Plex**, escala de token. As cores de custo/estado seguem a semântica (verde/âmbar/vermelho-suave) — inalteradas.

## 5. Ajustes visuais da r4 formalizados aqui (já no código desde a r4)
Esta story ASSUME estes ajustes deliberados da fusão (catálogo F2-2-1) — confira que ficaram bem na tela:
- [ ] **✕ de fechar** (modal de Agente) com tom `text.secondary` (antes `muted`) e fundo transparente (Ghost).
- [ ] **Rodapé do modal**: espaçamento vertical levemente menor (token `lg`=16 vs 20) e botão **Criar/Salvar** com padding/pesо do token (`confirm` semibold).
- [ ] **Botão "avançar"** do onboarding com padding do token (`Lg`=24).
- [ ] **Campo "cole a chave"** (ativar PRO) com superfície `raised` (antes `card`).
- [ ] Alvo de toque de **todo botão ≥24px** (WCAG 2.5.8).

## 6. Invariantes a confirmar
- [ ] **Flat honesto:** zero gradiente decorativo, zero sombra dramática, zero glassmorphism no chrome/cards.
- [ ] **Contraste WCAG** dos indicadores/textos OK nos 2 temas (o gate de CI cobre os tokens; confirmar leitura na tela).
- [ ] **[PROF]** sem regressão de frametime (rodar a cena de estresse antes/depois com a sonda).
- [ ] Periferia **legível** (desenha o grid), congela só o IDLE de verdade — honestidade ("a tela pausa, o trabalho não").

## Origem
Épico vault `38` §VIII (decisão 1 — statement da fusão) + §F2-2 (F2-2-3). Protótipos `tasks/pesquisa-f2/prototipos/t1-instrumento.html` (cor semântica fixa, OP-1, flat honesto) + `t3-atelie.html` (calor, momento de celebração). Despacho `tasks/epico-f2/despachos/r5-frontend-identidade.md`.

---
## Incremento r6 (F2-2-2 integração — P0 soberano)
- [ ] **VOZ no "precisa de você" (P0):** com um leitor de tela ligado (VoiceOver), um terminal que entra em "precisa de você" (gate de custódia pendente) deve ANUNCIAR sozinho, **sem você focar nele** (anúncio assertive, interrompe) — via o selo `Badge::needs_you()` no título. Antes só falava ao focar; agora a assimetria visual(vermelho)↔voz fechou. **Canal único:** ao focar o card, o leitor NÃO repete "precisa de você" 2× (o corpo lê "nome — status"; o selo carrega o pedido).
- [ ] **Selo visível:** o card que precisa de você mostra o selo "■ precisa de você" (vermelho de estado, glifo+texto+cor — nunca só cor) ao lado do nome, além do dot/borda vermelhos da r5.
- [ ] **Toolbar (quando montada):** ⚑ Atender leva você ATÉ o nó (foca+revela) sem aprovar nada; ✕ Encerrar fecha o nó (igual ao ✕ do header).

## Incremento r6 (attention_ui — item oficial Nº1)
- [ ] **Toast de permissão/custódia FALA sem foco:** com VoiceOver, quando um terminal pede permissão/custódia, o toast (canto inferior) anuncia ASSERTIVE (interrompe) o pedido — antes era mudo (Role::Status cru). O «+N pedidos» colapsado também anuncia.
- [ ] **Sino da topbar:** ESCALAÇÃO anuncia assertive (mesmo com o toast adiado/«Depois»); contagem anuncia polite (não martela — o toast já deu o detalhe). Canal único (sem eco duplo).
- Chips do título (autonomia/kit/cwd): **verificados — estado ESTÁTICO** (autonomia muda só pela edição focada; kit/cwd são fixados na criação) → não "mudam sem foco", não exigem live-region pelo critério.
