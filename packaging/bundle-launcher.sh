#!/bin/bash
# bundle-launcher — executável do Lina.app (CFBundleExecutable).
#
# Por que existe: o .app carrega binários JÁ compilados; abrir/fechar nunca
# recompila. Este launcher faz o rebuild AUTOMÁTICO ao abrir, mas só quando o app
# está rodando de DENTRO do repositório (uso do dev/fundador). Se o bundle for
# copiado para /Applications sem o repo ao lado, ele só executa o binário —
# comportamento seguro para distribuição (decisão: app sem assinatura, F1-4-7).
#
# Fluxo:
#   1. descobre o repo subindo a partir do bundle
#   2. se houver fonte (.rs) mais nova que o binário → cargo build + copia p/ o bundle
#   3. exec do binário real (mesmo PID → macOS mantém o app associado ao bundle)
#
# Log de cada rebuild: /tmp/lina-rebuild.log
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"          # .../Lina.app/Contents/MacOS
BIN="$HERE/lina-gpui-bin"                       # binário GUI real (renomeado pelo make-app.sh)
REPO_ROOT="$(cd "$HERE/../../../.." 2>/dev/null && pwd || true)"  # MacOS → ... → repo
LOG=/tmp/lina-rebuild.log

log() { printf '%s %s\n' "$(date '+%H:%M:%S')" "$*" >>"$LOG" 2>&1; }

if [[ -n "$REPO_ROOT" && -f "$REPO_ROOT/app/lina-gpui/Cargo.toml" ]]; then
  # PATH do Finder é mínimo — garanta cargo/rustc visíveis.
  export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

  rebuild=0
  if [[ ! -f "$BIN" ]]; then
    rebuild=1
  else
    newest="$(find "$REPO_ROOT/app/lina-gpui/src" "$REPO_ROOT/crates" \
      -name '*.rs' -newer "$BIN" -print -quit 2>/dev/null || true)"
    [[ -n "$newest" ]] && rebuild=1
  fi

  if [[ "$rebuild" -eq 1 ]]; then
    log "mudança detectada — recompilando…"
    if ( cd "$REPO_ROOT" \
          && cargo build --release -p lina-bootstrap --bin lina \
          && cd app/lina-gpui && cargo build --release ) >>"$LOG" 2>&1; then
      cp "$REPO_ROOT/app/lina-gpui/target/release/lina-gpui" "$BIN" && chmod +x "$BIN"
      cp "$REPO_ROOT/target/release/lina" "$HERE/lina" && chmod +x "$HERE/lina"
      log "rebuild OK — abrindo versão nova."
    else
      log "rebuild FALHOU — abrindo a versão anterior (veja o erro acima)."
    fi
  fi
fi

exec "$BIN" "$@"
