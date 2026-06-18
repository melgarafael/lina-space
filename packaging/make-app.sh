#!/usr/bin/env bash
# make-app.sh — monta Lina.app (macOS, SEM assinatura) + .dmg baixável.
#
# Decisão do fundador (2026-06-04): distribuir sem codesign/notarização. Os alunos abrem
# via bypass do Gatekeeper (ver INSTALAR-LINA.md). Este script NÃO assina nada.
#
# Layout do bundle (o resolver do app acha `lina` e `assets/` SEM mudança de código —
# `lina` ao lado do executável; `assets/` por ancestral do executável):
#   Lina.app/Contents/Info.plist
#   Lina.app/Contents/MacOS/{Lina, lina, assets/, profiles/}
#   Lina.app/Contents/Resources/AppIcon.icns   (opcional, se packaging/AppIcon.icns existir)
#
# Uso:
#   packaging/make-app.sh              # build release + bundle + dmg
#   packaging/make-app.sh --skip-build # só re-bundla (usa binários já buildados)
#   packaging/make-app.sh --no-dmg     # bundle sem gerar dmg
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="0.1.0"
APP_NAME="Lina"          # nome do bundle (Lina.app) e o que o aluno vê (via CFBundleDisplayName).
EXE_NAME="lina-gpui"     # nome do executável GUI DENTRO do bundle. NÃO pode ser "Lina": APFS é
                         # case-insensitive e colidiria com o CLI "lina" na mesma pasta MacOS/.
DIST="$REPO_ROOT/dist"
APP="$DIST/$APP_NAME.app"
GPUI_BIN="$REPO_ROOT/app/lina-gpui/target/release/lina-gpui"
LINA_BIN="$REPO_ROOT/target/release/lina"
ASSETS_SRC="$REPO_ROOT/assets"
PROFILES_SRC="$REPO_ROOT/profiles"

SKIP_BUILD=0
MAKE_DMG=1
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --no-dmg) MAKE_DMG=0 ;;
    *) echo "arg desconhecido: $arg" >&2; exit 2 ;;
  esac
done

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "==> [1/4] build release: CLI lina (workspace) + app lina-gpui (standalone)"
  cargo build --release -p lina-bootstrap --bin lina
  ( cd "$REPO_ROOT/app/lina-gpui" && cargo build --release )
fi

# Sanidade: os dois binários existem?
for b in "$GPUI_BIN" "$LINA_BIN"; do
  [[ -f "$b" ]] || { echo "ERRO: binário ausente: $b (rode sem --skip-build)" >&2; exit 1; }
done
[[ -d "$ASSETS_SRC" ]] || { echo "ERRO: assets/ ausente em $ASSETS_SRC" >&2; exit 1; }
# F1-0-2 critério 5: o app de produção carrega o claude-code.toml REAL — o bundle PRECISA dele.
[[ -f "$PROFILES_SRC/claude-code.toml" ]] || { echo "ERRO: profiles/claude-code.toml ausente em $PROFILES_SRC" >&2; exit 1; }

echo "==> [2/4] montando $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# O binário GUI real entra como "lina-gpui-bin". O CFBundleExecutable ($EXE_NAME =
# "lina-gpui") é o LAUNCHER (packaging/bundle-launcher.sh): recompila automaticamente
# quando o app é aberto de DENTRO do repo e dá exec no binário real. Fora do repo
# (ex.: copiado p/ /Applications) o launcher só executa — seguro p/ distribuição.
cp "$GPUI_BIN" "$APP/Contents/MacOS/lina-gpui-bin"
cp "$LINA_BIN" "$APP/Contents/MacOS/lina"
cp "$REPO_ROOT/packaging/bundle-launcher.sh" "$APP/Contents/MacOS/$EXE_NAME"
# REGRA F1-4-7: o bundle copia por ALLOWLIST (acima). lina-keygen (ferramenta do fundador,
# assina licencas) NUNCA entra no bundle — nao adicione cp dele aqui.
chmod +x "$APP/Contents/MacOS/lina-gpui-bin" "$APP/Contents/MacOS/lina" "$APP/Contents/MacOS/$EXE_NAME"
# Guarda anti-colisão (APFS case-insensitive): o GUI e o CLI têm de ser arquivos DISTINTOS.
if [[ "$(stat -f%z "$APP/Contents/MacOS/lina-gpui-bin")" == "$(stat -f%z "$APP/Contents/MacOS/lina")" ]]; then
  echo "AVISO: lina-gpui-bin e lina têm o mesmo tamanho — possível colisão de nome (case-insensitive)." >&2
fi

# assets/ (doutrina + skill) ao lado do executável → o resolver acha por ancestral. Sem .git/target.
cp -R "$ASSETS_SRC" "$APP/Contents/MacOS/assets"
rm -rf "$APP/Contents/MacOS/assets/.git" 2>/dev/null || true

# profiles/ (CLI Profiles TOML — F1-0-2: ready_timeout/busy_markers calibrados) ao lado do
# executável → `injection_profile_candidates` acha por ancestral do exe (bundle auto-contido,
# ANTES do caminho do repo). Mesmo padrão do assets/.
cp -R "$PROFILES_SRC" "$APP/Contents/MacOS/profiles"

# Info.plist (substitui a versão, caso queira bumpar via env VERSION).
sed "s|<string>0\.1\.0</string>|<string>$VERSION</string>|g" \
  "$REPO_ROOT/packaging/Info.plist" > "$APP/Contents/Info.plist"

# Ícone opcional.
if [[ -f "$REPO_ROOT/packaging/AppIcon.icns" ]]; then
  cp "$REPO_ROOT/packaging/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
  echo "    ícone aplicado."
else
  echo "    (sem packaging/AppIcon.icns — macOS usará o ícone genérico; ver INSTALAR-LINA.md p/ adicionar depois)"
fi

# PkgInfo (cosmético, ajuda o Finder).
printf 'APPL????' > "$APP/Contents/PkgInfo"

# Tamanho do bundle.
echo "    bundle pronto: $(du -sh "$APP" | awk '{print $1}')"

if [[ "$MAKE_DMG" -eq 1 ]]; then
  echo "==> [3/4] gerando .dmg (arrasta-pra-Aplicativos)"
  STAGING="$DIST/dmg-staging"
  rm -rf "$STAGING"; mkdir -p "$STAGING"
  cp -R "$APP" "$STAGING/"
  ln -s /Applications "$STAGING/Applications"
  DMG="$DIST/Lina-$VERSION.dmg"
  rm -f "$DMG"
  hdiutil create -volname "Lina Space" -srcfolder "$STAGING" -ov -format UDZO "$DMG" >/dev/null
  rm -rf "$STAGING"
  echo "    dmg: $DMG ($(du -sh "$DMG" | awk '{print $1}'))"
else
  echo "==> [3/4] (--no-dmg) pulei o dmg"
fi

echo "==> [4/4] PRONTO."
echo "    App:  $APP"
echo "    Teste local:  open \"$APP\""
echo "    Distribuir:   envie o .dmg; o aluno segue packaging/INSTALAR-LINA.md (bypass do Gatekeeper)."
echo
echo "    LEMBRE: o app é SEM assinatura. No 1º open o macOS avisa 'desenvolvedor não identificado'."
echo "    Pré-requisito do aluno: ter o Claude Code (\`claude\`) instalado e logado (o Lina orquestra, não traz a IA)."
