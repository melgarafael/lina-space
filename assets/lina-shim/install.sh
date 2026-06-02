#!/bin/sh
# install.sh — instala o PATH-shim do gate (W3-6 tier 2) num diretório-alvo (ex.: `<workspace>/.lina/bin`).
#
# O app chama isto (ou replica a lógica) ao preparar o ambiente do agente: copia `lina-shim.sh` para
# o alvo e cria um link nomeado por ferramenta gated. Depois, o app PREPÕE o alvo ao PATH do PTY do
# agente e injeta `LINA_SHIM_DIR=<alvo>` para a resolução do binário real ser robusta.
#
# Uso:  sh install.sh <dir-alvo>      (ex.: sh install.sh "$WORKSPACE/.lina/bin")
set -eu

TARGET="${1:?uso: sh install.sh <dir-alvo>}"
SRC_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

mkdir -p "$TARGET"
cp "$SRC_DIR/lina-shim.sh" "$TARGET/lina-shim.sh"
chmod +x "$TARGET/lina-shim.sh"

# Ferramentas interceptadas (as que mutam o mundo / custodiam segredo). Estender = uma linha aqui.
for tool in git rm kubectl terraform gh deploy; do
    # Link relativo dentro do mesmo diretório: `git` → `lina-shim.sh`.
    ln -sf "lina-shim.sh" "$TARGET/$tool"
done

echo "lina-shim: instalado em $TARGET (git rm kubectl terraform gh deploy)."
echo "lina-shim: o app deve prepor '$TARGET' ao PATH do agente e exportar LINA_SHIM_DIR='$TARGET'."
