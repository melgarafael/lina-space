# PROPOSTA — permission-mode por terminal (destrava o teste de tela do gate F1-1)

R2 · Dev lina-hooks/dashboard · 2026-06-07 · **doc de decisão, ZERO código de spawn** (fiação = R2b após aprovação).
Origem: achado operacional (i) da F1-1-6 — esta máquina roda `defaultMode: bypassPermissions`
(`~/.claude/settings.json`, nível usuário), então permissão NUNCA é pedida e o toast da F1-1-7
não tem o que aprovar.

---

## 1. O que o claude 2.1.168 REALMENTE respeita (testado na prática, headless `-p`)

Sondas em `/tmp/lina-perm-probe/*` (cwds frescos, mesma máquina do fundador, `--output-format
json`; sinal = `permission_denials` + efeito no disco). Comando mutador (`touch`) como probe —
`echo` NÃO serve (classe *safe/read-only* do 2.1.x roda sem pedir em qualquer modo).

| # | setup | comando | resultado observado |
|---|---|---|---|
| A | herda usuário (`bypassPermissions`) | `echo`/`touch` | **executa**, `denials: []` (controle) |
| B | flag `--permission-mode default` | `touch probe-file.txt` | **NEGADO** — `denials: [{tool_name: Bash, tool_input: {command: "touch probe-file.txt"}}]`, arquivo NÃO criado |
| C | `<cwd>/.claude/settings.json` = `{"permissions":{"defaultMode":"default"}}` (SEM flag) | `touch` | **NEGADO** — idem; settings POR-CWD vence o bypass de usuário |
| D | mesmo cwd da C | `git init` | **executa sem pedir** — o `allow: ["Bash(git:*)"]` do usuário se FUNDE e auto-aprova git mesmo em `default` |

**Conclusões de precedência (provadas, não lidas de doc):**
1. **Flag de spawn `--permission-mode <m>` vence o settings de usuário** (B vs controle).
2. **Settings por-cwd (`<cwd>/.claude/settings.json`, o MESMO arquivo que o `lina-bootstrap`
   já escreve) também vence o settings de usuário** (C).
3. **Regras `allow` do usuário sobrevivem e se fundem em qualquer modo** (D): permissão por
   regra ≠ modo. Não dá para "des-permitir" via modo o que o usuário allowlistou.
4. Modos aceitos pela flag no 2.1.168: `acceptEdits | auto | bypassPermissions | default |
   dontAsk | plan`. (`bypassPermissions` via flag exige caso especial
   `--allow-dangerously-skip-permissions` — irrelevante: nunca vamos pedir isso.)
5. O payload de negação carrega `tool_name` + `tool_input.command` — exatamente os campos que
   o seam da Tarefa A desta rodada adicionou ao `HookEvent` (o toast pode dizer
   `touch probe-file.txt` literal).

## 2. Default por papel/terminal (sugestão p/ discussão)

Fio condutor do produto: **gate humano — NUNCA bypass silencioso**. O aluno não-técnico precisa
VER o pedido ("Esperando você" + toast), não descobrir depois que o agente fez `rm`.

| papel do terminal | modo proposto | por quê |
|---|---|---|
| **default de TODO terminal** | `default` | o claude pede como pede normalmente; o toast F1-1-7 vira a superfície de aprovação |
| Revisor / Explorador (read-only por doutrina) | `plan` | nem edita: aborda o "só olhar" sem confiar em prompt |
| Dev de confiança (opt-in EXPLÍCITO do usuário, por terminal, visível no card) | `acceptEdits` | edits fluem; Bash/push continuam pedindo |
| `bypassPermissions` / `dontAsk` / `auto` | **NUNCA por default** | bypass silencioso quebra o fio condutor; se um dia existir, é opt-in assustador com selo permanente no card |

Observação honesta: o modo NÃO anula allowlists do usuário (conclusão 3). Numa máquina como a
do fundador, `git:*` continua auto-aprovado em qualquer modo — o produto não deve prometer
"todo comando pede".

## 3. Onde fia (arquivo:linha — SEM editar nesta rodada; bridge é do LLM Engineer)

**Recomendação primária — settings por-cwd via bootstrap (caminho C):**
- `crates/lina-bootstrap/src/lib.rs:427-428` — o bootstrap JÁ escreve
  `<cwd>/.claude/settings.json` por terminal (hooks). Acrescentar o bloco
  `"permissions": {"defaultMode": "<modo do terminal>"}` ao JSON gerado por
  `hook_settings_json_with_observability` (`lib.rs:472-480`; variante pura `lib.rs:444`).
  1 escritor que já existe, 1 arquivo, zero mudança no spawn, provado na sonda C.
- O modo por terminal chega ao `BootstrapWriter` no spawn (mesma fiação que hoje injeta o
  token de hooks).

**Reforço/evolução multi-CLI — flag no spawn (caminho B):**
- `app/lina-gpui/src/bridge.rs:2454-2456` — `PtyCommand::new(&e.program)` + loop de `args`
  (struct `Engine{program,args}` em `bridge.rs:1582-1583`): appendar `--permission-mode <m>`.
- O NOME da flag é conhecimento de CLI ⇒ vive no TOML (inv#3):
  `profiles/claude-code.toml:11-13` (`program`/`args`) + campo novo opcional em
  `crates/lina-cli-profiles/src/lib.rs:107-110` (ex.: `permission_mode_flag =
  "--permission-mode"`; CLI sem o campo = não suporta ⇒ degrada sem flag).

Proposta R2b mínima: **só o caminho do bootstrap** (menor diff, sem tocar spawn); flag por
TOML fica registrada como evolução quando outro CLI entrar. Os dois convivem (flag > settings
por-cwd na precedência documentada — não conflitam se consistentes).

## 4. Interação com o trust-dialog 2.1.x (achado (ii) da F1-1-6)

- O trust dialog ("Quick safety check… do you trust this folder?") é um gate **independente**
  do permission-mode: apareceu no probe da F1-1-6 MESMO com o usuário em `bypassPermissions`.
  Mudar o modo não o suprime nem o cria.
- Em `-p` (headless) ele não aparece (sondas acima rodaram em cwds frescos sem trust prompt);
  em PTY interativo (o caso do produto) ele precede o 1º turno em **cwd novo**.
- Mitigação natural do produto: os cwds por terminal (`…/walking-skeleton/tN`) são ESTÁVEIS —
  o trust é 1× por pasta, persiste entre sessões; o atrito é só no 1º uso de cada terminal.
- **Ponto aberto p/ R2b:** o trust dialog é menu de setas (sem `(y/n)`) ⇒ NÃO casa com o
  catálogo regex do fallback de grid da F1-1-6. Ou se adiciona um pattern dedicado
  (ex.: `trust this folder`/`Quick safety check`), ou se aceita que o 1º turno de um terminal
  novo exige o usuário no terminal (e o card mostra "Esperando você" via heurística de idle).
  Pré-semear `~/.claude.json` (estado interno onde o trust persiste) foi considerado e
  DESCARTADO: formato não-documentado, frágil a updates do CLI.

## 5. Implicação direta p/ o TESTE DE TELA do gate F1-1

Com (qualquer) fiação aprovada, o roteiro do teste precisa de um comando que REALMENTE peça:
- **Usar comando mutador FORA do allowlist do usuário** (ex.: `touch arquivo-teste.txt` ou
  `mkdir`). `git push`/`git:*` NÃO pede nesta máquina (sonda D) — allowlist do usuário vence.
- NUNCA editar/remover o settings do fundador para o teste (config dele, não nossa).
- Sequência esperada: terminal em `default` → agente tenta `touch` → claude trava pedindo →
  `Notification`+`PreToolUse` pendente (detecção F1-1-6, com `message`/`tool_input` do seam
  R2-A no detail) → toast F1-1-7 → aprovação → `PostToolUse`.

## 6. O que se decide aqui (checklist p/ o Maestro)

- [ ] Caminho de fiação: bootstrap-only (recomendado) vs bootstrap+flag.
- [ ] Default `default` p/ todo terminal + mapa por papel (§2) — ou outro mapa.
- [ ] Trust-dialog: pattern novo no grid OU atrito aceito no 1º uso (§4).
- [ ] Roteiro do gate usa `touch`/comando fora de allowlist (§5).
