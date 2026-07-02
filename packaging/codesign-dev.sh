#!/usr/bin/env bash
# codesign-dev.sh — SELA o Lina.app com o certificado LOCAL de dev, dando ao macOS uma identidade de
# código ESTÁVEL e ancorada no bundle. É isto que o TCC de DEVICE (microfone/câmera) exige p/ mostrar
# o prompt e fazer a permissão colar entre rebuilds.
#
# Por que SELAR o bundle (e não só assinar os Mach-O soltos): o TCC casa a permissão com a identidade
# do app (CFBundleIdentifier = space.lina.app) resolvida a partir do bundle selado. Sem o selo o macOS
# vê "code object is not signed at all" e não tem âncora → nega microfone EM SILÊNCIO e o prompt do
# /voice nunca aparece. Selar só é possível porque assets/ e profiles/ moram em Contents/Resources/
# (MacOS/ tem apenas Mach-O: o lina-gpui-bin, que É o CFBundleExecutable, e a CLI lina). Ver achado
# /voice 2026-06-29 e o comentário de layout no Info.plist.
#
# CONDICIONAL: só age se o certificado de dev existir nesta máquina (setup: codesign-dev-setup.sh).
# Sem o cert → no-op silencioso, exit 0 → a build de DISTRIBUIÇÃO segue SEM assinatura, como o
# fundador decidiu (make-app.sh). Nunca aborta o build: falha de assinatura vira aviso, não erro.
#
# Uso: packaging/codesign-dev.sh /caminho/para/Lina.app
set -uo pipefail

APP="${1:-}"
[[ -n "$APP" && -d "$APP" ]] || { echo "codesign-dev: bundle inexistente: '${APP:-}'" >&2; exit 0; }

CERT_CN="Lina Dev (local TCC)"
KC="lina-codesign.keychain"
BUNDLE_ID="space.lina.app"   # = CFBundleIdentifier do Info.plist; é a ele que o TCC casa o processo.
LOG=/tmp/lina-rebuild.log

# Cert presente? Senão no-op (preserva build de distribuição sem assinatura).
security find-certificate -c "$CERT_CN" "$KC" >/dev/null 2>&1 || exit 0

# A CLI lina é um Mach-O em MacOS/ que NÃO é o CFBundleExecutable. Assina-a primeiro, com identidade
# própria estável (não toca device; identifier só por estabilidade), antes de selar o bundle por cima.
if [[ -f "$APP/Contents/MacOS/lina" ]]; then
  codesign --force --sign "$CERT_CN" --identifier "space.lina.cli" --timestamp=none \
    "$APP/Contents/MacOS/lina" 2>>"$LOG" \
    && printf '%s codesign-dev: assinado lina (id=space.lina.cli)\n' "$(date '+%H:%M:%S')" >>"$LOG" \
    || printf '%s codesign-dev: FALHA ao assinar lina\n' "$(date '+%H:%M:%S')" | tee -a "$LOG" >&2
fi

# Sela o BUNDLE: assina o CFBundleExecutable (lina-gpui-bin) com o id do app e sela o Info.plist +
# recursos no _CodeSignature/CodeResources. Esta é a âncora de identidade que o TCC de device usa.
if codesign --force --sign "$CERT_CN" --identifier "$BUNDLE_ID" --timestamp=none "$APP" 2>>"$LOG"; then
  printf '%s codesign-dev: bundle selado (id=%s)\n' "$(date '+%H:%M:%S')" "$BUNDLE_ID" >>"$LOG"
else
  printf '%s codesign-dev: FALHA ao selar o bundle — app roda sem assinatura (TCC instável)\n' \
    "$(date '+%H:%M:%S')" | tee -a "$LOG" >&2
fi
exit 0
