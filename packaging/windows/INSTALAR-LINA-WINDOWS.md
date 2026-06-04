# Instalar o Lina Space (Windows)

Bem-vindo! O Lina é um espaço onde vários agentes de IA trabalham juntos pra você.
Siga os 3 passos — leva uns 5 minutos.

> **Importante:** o Lina **não tem a IA dentro dele** — ele *organiza* a IA que você já usa
> (o **Claude Code**). Por isso o **Passo 1** é necessário.

---

## Passo 1 — Ter o Claude Code instalado (uma vez só)

1. Instale o **Claude Code** seguindo o site oficial da Anthropic (peça o link a quem te enviou o Lina).
2. Abra o **Terminal / PowerShell** e rode `claude` uma vez pra **fazer login** na sua conta.
3. Se abrir e pedir login, está certo. Pode fechar.

---

## Passo 2 — Instalar o Lina

1. Baixe o **`Lina-windows-x64.zip`** que te enviaram.
2. Clique com o botão direito no `.zip` → **Extrair tudo** (ex.: para `C:\Lina`).
3. Pronto — o Lina está na pasta extraída. (Opcional: clique direito em `lina-gpui.exe` →
   **Fixar na barra de tarefas** ou **Enviar para → Área de trabalho** pra criar um atalho.)

---

## Passo 3 — Abrir pela primeira vez (importante!)

O Lina é distribuído **sem assinatura** (é um app sob medida, não vem da Loja). Por isso, **só na
primeira vez**, o Windows mostra um aviso azul do **SmartScreen**. É normal e seguro.

**Faça assim:**

1. Dê **duplo-clique** em `lina-gpui.exe`.
2. Se aparecer **"O Windows protegeu o seu computador"**, clique em **"Mais informações"**.
3. Vai surgir o botão **"Executar assim mesmo"** — clique nele.

Depois da primeira vez, abre normalmente.

> Se o Windows Defender reclamar do arquivo, escolha **"Permitir no dispositivo"** / **"Manter"**
> (apps sem assinatura às vezes pedem isso). Não é vírus — é só a falta do certificado.

---

## O que acontece quando abrir

Na primeira vez, o Lina te guia pra **criar seu primeiro agente**. Depois você adiciona mais e eles
**conversam entre si sozinhos**. Seu trabalho fica **salvo automaticamente** na sua máquina.

---

## Problemas comuns

- **"Abre mas os agentes não respondem"** → confira o Passo 1: o `claude` precisa estar instalado e
  logado. Abra o PowerShell, rode `claude`, faça login, e reabra o Lina.
- **"O Windows não deixa abrir"** → refaça o Passo 3 ("Mais informações" → "Executar assim mesmo").
- **Faltou um arquivo `.dll`** → extraia o `.zip` INTEIRO (não rode o `.exe` de dentro do zip);
  o `lina-gpui.exe`, o `lina.exe` e a pasta `assets` precisam ficar juntos.

Qualquer dúvida, fale com quem te enviou o Lina. 💜
