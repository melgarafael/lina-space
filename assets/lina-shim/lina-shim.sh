#!/bin/sh
# lina-shim.sh — PATH-shim do gate de execução do Lina Space (W3-6, tier 2, design §1.3).
#
# COMO FUNCIONA
#   O app prepõe `.lina/bin/` ao PATH do agente e instala AQUI links nomeados das ferramentas que
#   mutam o mundo (git, rm, kubectl, terraform, gh, deploy). Quando o agente roda `git push --force`,
#   o shell encontra ESTE script ANTES do git real. O shim:
#     1. reconstrói o comando real (`<tool> <args>`);
#     2. consulta o gate determinístico `lina guard --check-action` (pattern-match, ZERO LLM —
#        invariante #1), que APENDA `ActionGated` ao event log quando a decisão != allow;
#     3. se `allow`  → faz `exec` do binário REAL (resolvido no PATH SEM o diretório do shim);
#        se `ask|deny` → canal de confirmação STUB (default "não") → NÃO executa o binário real e
#        sai com código != 0. `LINA_CONFIRM=yes` aprova (humano disse sim).
#
# FURO CONHECIDO (honesto — design §1.3): caminho ABSOLUTO fura o shim
#   `/usr/bin/git push --force` IGNORA o `.lina/bin/git` e roda o git real. Só o hook PreToolUse
#   (Claude Code, tier 1) é gate verdadeiramente DURO. Para `gated-hard` externo (deploy/pagamento),
#   a camada inquebrável é a CUSTÓDIA DE SEGREDO (W0-7): o agente nunca recebe o token.
#
# AMBIENTE
#   LINA_AUTONOMY  nível (manual|assistido|autonomo); default "assistido".
#   LINA_SHIM_DIR  diretório deste shim, a ser excluído na busca do binário real. O app o injeta;
#                  fallback = dirname de $0 (frágil quando argv[0] é só o nome — daí o env).
#   LINA_CONFIRM   "yes" aprova ask/deny; qualquer outro valor (default) RECUSA. Stub do canal humano.

set -u

# Nome da ferramenta = como o shim foi invocado (link `git` → este arquivo ⇒ TOOL=git).
TOOL=$(basename -- "$0")
AUTONOMY="${LINA_AUTONOMY:-assistido}"

# Comando real reconstruído para o classificador determinístico.
CMD="$TOOL $*"

# Diretório do shim (excluído da resolução do binário real para não recursar em si mesmo).
shim_dir() {
    if [ -n "${LINA_SHIM_DIR:-}" ]; then
        printf '%s\n' "$LINA_SHIM_DIR"
    else
        # Fallback: dirname de $0. Confiável só quando $0 é um caminho (não só o nome).
        CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd
    fi
}

# Resolve o binário REAL varrendo o PATH, pulando o diretório do shim. Imprime o caminho ou nada.
resolve_real() {
    sdir=$(shim_dir)
    oldifs=$IFS
    IFS=:
    for dir in $PATH; do
        [ -z "$dir" ] && dir=.
        [ "$dir" = "$sdir" ] && continue
        if [ -x "$dir/$TOOL" ] && [ ! -d "$dir/$TOOL" ]; then
            printf '%s\n' "$dir/$TOOL"
            IFS=$oldifs
            return 0
        fi
    done
    IFS=$oldifs
    return 1
}

# Faz exec do binário real (ou falha com 127 se não houver). Usado nos caminhos allow / confirmado.
exec_real() {
    real=$(resolve_real || true)
    if [ -z "$real" ]; then
        echo "lina-shim: binário real '$TOOL' não encontrado no PATH (fora de $(shim_dir))" >&2
        exit 127
    fi
    exec "$real" "$@"
}

# 1) Gate determinístico. (Apenda ActionGated ao log quando a decisão != allow.)
DECISION=$(lina guard --check-action --cmd "$CMD" --autonomy "$AUTONOMY" 2>/dev/null)

# 2) allow → executa o binário real, transparente.
if [ "$DECISION" = "allow" ]; then
    exec_real "$@"
fi

# 3) ask|deny → canal de confirmação STUB. Default "não": NÃO executa, sai != 0.
CONFIRM="${LINA_CONFIRM:-no}"
if [ "$CONFIRM" = "yes" ]; then
    exec_real "$@"
fi

echo "lina-shim: ação '$CMD' BLOQUEADA pelo gate (decisão=$DECISION, confirmação=não)." >&2
echo "lina-shim: o binário real NÃO foi executado. Use o gate humano para aprovar." >&2
exit 3
