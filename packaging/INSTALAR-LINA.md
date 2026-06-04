# Instalar o Lina Space (Mac)

Bem-vindo! O Lina é um espaço onde vários agentes de IA trabalham juntos pra você.
Siga os 3 passos abaixo — leva uns 5 minutos.

> **Importante:** o Lina **não tem a IA dentro dele** — ele *organiza* a IA que você já usa
> (o **Claude Code**). Por isso o **Passo 1** é necessário.

---

## Passo 1 — Ter o Claude Code instalado (uma vez só)

O Lina precisa do **Claude Code** instalado e logado na sua conta.

1. Instale o Claude Code seguindo o site oficial da Anthropic (peça o link a quem te enviou o Lina).
2. Abra o Terminal do Mac e rode `claude` uma vez pra **fazer login** na sua conta.
3. Se `claude` abrir e pedir login, está certo. Pode fechar.

*(Se você já usa o Claude Code no terminal, pode pular este passo.)*

---

## Passo 2 — Instalar o Lina

1. Abra o arquivo **`Lina-0.1.0.dmg`** que você baixou (clique duas vezes).
2. Vai aparecer uma janela com o ícone do **Lina** e uma pasta **Aplicativos**.
3. **Arraste o Lina para a pasta Aplicativos.** Pronto, está instalado.

---

## Passo 3 — Abrir pela primeira vez (importante!)

O Lina é distribuído **sem assinatura da Apple** (é um app feito sob medida, não vem da App Store).
Por isso, **só na primeira vez**, o Mac vai pedir uma confirmação. É normal e seguro.

**Faça assim:**

1. Vá em **Aplicativos**, **clique com o botão direito** (ou Control+clique) no **Lina**.
2. Escolha **Abrir**.
3. Vai aparecer um aviso "desenvolvedor não identificado". Clique em **Abrir** de novo.

> Se o botão "Abrir" não aparecer (Macs mais novos), vá em
> **Ajustes do Sistema → Privacidade e Segurança**, role até embaixo e clique em
> **"Abrir Assim Mesmo"** ao lado do nome do Lina.

Depois da primeira vez, é só clicar normalmente como qualquer app.

### Atalho pra quem usa o Terminal (opcional)

Se preferir, rode isto uma vez no Terminal pra tirar o aviso de uma vez:

```bash
xattr -dr com.apple.quarantine /Applications/Lina.app
```

Depois o Lina abre com clique duplo normal.

---

## O que acontece quando abrir

Na primeira vez, o Lina te guia pra **criar seu primeiro agente**. A partir daí você adiciona
mais agentes e eles **conversam entre si sozinhos** — é a mágica do Lina.

Seu trabalho fica **salvo automaticamente** na sua máquina (em *Application Support → Lina*),
e o Lina **recupera tudo** se algo travar.

---

## Problemas comuns

- **"O Lina abre mas os agentes não respondem"** → confira o Passo 1: o `claude` precisa estar
  instalado e logado. Abra o Terminal, rode `claude`, faça login, e reabra o Lina.
- **"Não consigo abrir, o Mac bloqueia"** → refaça o Passo 3 (botão direito → Abrir, ou
  Ajustes → Privacidade e Segurança → "Abrir Assim Mesmo").
- **Mac com chip Intel** → esta versão é pra Macs com chip Apple (M1/M2/M3/M4). Se o seu Mac é
  Intel, avise quem te enviou — precisa de uma versão separada.

Qualquer dúvida, fale com quem te enviou o Lina. 💜
