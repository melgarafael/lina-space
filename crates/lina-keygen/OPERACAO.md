# OPERAÇÃO — como emitir chaves de licença do Lina Space

Este guia é para VOCÊ, fundador. É a única pessoa que emite chaves. Tudo
acontece no seu computador, sem internet — ninguém de fora participa.

## O que existe aqui

- **`lina-keygen`** — a ferramenta de emitir chaves. Ela é separada do app:
  nunca vai junto na instalação que os usuários baixam.
- **Chave privada** — o seu "carimbo". Quem tem esse arquivo consegue criar
  licenças válidas do Lina. Por isso ele NUNCA pode entrar no repositório do
  código, num backup compartilhado ou num chat.
- **Chave pública** — a "lente" que o app usa para conferir o carimbo. Ela
  vai embutida dentro do app e não é segredo.

## Onde a chave privada vive

Hoje ela está em: `~/.lina/keygen/lina-signing.private` (só o seu usuário
consegue ler). Recomendações, da mais segura para a mais prática:

1. **Mídia offline:** copie o arquivo para um pendrive guardado em local
   físico seguro e apague da máquina; plugue só na hora de emitir.
2. **Manter na máquina:** aceitável enquanto só você usa este computador.
   Não sincronize a pasta `~/.lina/keygen/` com iCloud/Dropbox/Drive.

Faça UMA cópia de segurança offline. Se a privada se perder, você não emite
mais chaves novas (as já emitidas continuam funcionando) e precisa rotacionar.

## Emitir um lote para uma turma

```
lina-keygen gen --count 50 --tier pro --label turma-7 --expiry 12m \
  --private-key ~/.lina/keygen/lina-signing.private
```

Sai um arquivo `chaves-turma-7.csv` com 50 chaves, uma por linha:
chave · plano · validade · rótulo. Cada chave é única — se uma vazar, só
aquela está exposta; o resto do lote não corre risco.

- `--expiry 12m` = vale 12 meses de calendário a partir de hoje (aceita
  também dias `90d` e anos `1y`). Sem `--expiry`, a chave é vitalícia.
- `--workspace-limit 3` = limita a 3 Espaços (sem isso, ilimitado).
- O CSV é o que você distribui (uma chave por aluno, por e-mail ou planilha).
  O aluno cola a chave no app e pronto — sem cadastro, sem internet.

## Rotacionar a chave (se a privada vazar ou se perder)

1. Gere um par novo: `lina-keygen keypair --out ~/.lina/keygen-novo`
2. Peça ao time para trocar a constante `OFFICIAL_PUBLIC_KEY_HEX` em
   `crates/lina-license/src/token.rs` pela pública nova (a ferramenta
   imprime a linha exata) e lançar uma atualização do app.
3. A partir dessa atualização, só chaves emitidas com a privada nova valem.
   Chaves antigas continuam funcionando nas versões antigas do app — quem
   precisar, você reemite (o CSV diz de qual lote cada aluno veio).

## O que esta ferramenta NÃO faz (de propósito)

Sem portal de resgate, sem revogação remota, sem contato com servidor: o
Lina é local-first e a licença também. A proteção contra pirataria é
pragmática — quem cracka fica sem atualizações e sem a comunidade.
