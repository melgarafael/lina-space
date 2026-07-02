#!/usr/bin/env bash
# dev-watch.sh — vigia de desenvolvimento: recompila o Lina sozinho e avisa quando
# há versão nova pronta, pra ninguém precisar fechar/reabrir o app na mão.
#
# Por que existe: o Lina é Rust nativo — o .app carrega um binário JÁ compilado e NÃO recompila ao
# abrir (o entry-point virou Mach-O direto; o antigo bundle-launcher.sh, que recompilava ao abrir, foi
# removido do bundle p/ o microfone/TCC funcionar — achado /voice 2026-06-29). Logo, DURANTE O DEV rode
# este vigia: ele fica de olho nos .rs, recompila em segundo plano e, quando o build passa, pergunta
# "aplicar agora?". Em 1 clique troca pela versão nova; em "Depois" deixa o binário pronto (próxima
# abertura instantânea). SEM este vigia, abrir o app não pega código novo — rode make-app.sh --skip-build.
#
# NÃO reinicia sozinho de propósito: reabrir mata os terminais vivos do Espaço. O estado
# do Espaço é reconstruído do event log (invariante #4), mas um terminal NO MEIO de uma
# tarefa perderia o raciocínio. Por isso a aplicação espera você decidir a hora.
#
# Uso:  packaging/dev-watch.sh        # deixa rodando num terminal; Ctrl-C para parar
# Log:  /tmp/lina-rebuild.log         # o mesmo do launcher (continuidade)
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$REPO_ROOT/dist/Lina.app"
BUNDLE_MACOS="$APP/Contents/MacOS"
LOG=/tmp/lina-rebuild.log
SENTINEL=/tmp/lina-watch.stamp                  # marca "já vi esta geração de edições"
POLL=2                                           # segundos entre varreduras (find em src é barato)

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

log() { printf '%s %s\n' "$(date '+%H:%M:%S')" "$*" | tee -a "$LOG"; }

# Só observa daqui pra frente — o launcher já cobre o primeiro build na abertura.
touch "$SENTINEL"
log "vigia ligado — observando crates/ e app/lina-gpui/src/ (Ctrl-C para parar)."

changed_since_sentinel() {
  # ponytail: polling com find -newer em vez de fswatch — sem dependência nova, e src é
  # pequeno o bastante pra varrer a cada 2s. Troca por fswatch se a árvore crescer muito.
  [[ -n "$(find "$REPO_ROOT/app/lina-gpui/src" "$REPO_ROOT/crates" \
            -name '*.rs' -newer "$SENTINEL" -print -quit 2>/dev/null || true)" ]]
}

rebuild() {
  log "mudança detectada — recompilando em segundo plano…"
  if ( cd "$REPO_ROOT" \
        && cargo build --release -p lina-bootstrap --bin lina \
        && cd app/lina-gpui && cargo build --release ) >>"$LOG" 2>&1; then
    cp "$REPO_ROOT/app/lina-gpui/target/release/lina-gpui" "$BUNDLE_MACOS/lina-gpui-bin" && chmod +x "$BUNDLE_MACOS/lina-gpui-bin"
    cp "$REPO_ROOT/target/release/lina" "$BUNDLE_MACOS/lina" && chmod +x "$BUNDLE_MACOS/lina"
    # Re-assina com o cert de dev → identidade de TCC estável (permissões não somem). No-op sem o cert.
    "$REPO_ROOT/packaging/codesign-dev.sh" "$APP" || true
    log "rebuild OK — versão nova pronta no bundle."
    return 0
  fi
  log "rebuild FALHOU — versão anterior mantida (veja o erro acima no log)."
  return 1
}

ask_and_apply() {
  # Diálogo nativo do macOS: o "espera 1 clique" sem tocar a UI do gpui.
  local choice
  choice="$(osascript \
    -e 'display dialog "Há uma versão nova do Lina pronta. Aplicar agora vai reiniciar o app (os terminais ativos serão reabertos)." buttons {"Depois", "Aplicar agora"} default button "Aplicar agora" with title "Lina" with icon note' \
    -e 'button returned of result' 2>/dev/null || echo "Depois")"

  if [[ "$choice" == "Aplicar agora" ]]; then
    log "aplicando: reiniciando o app."
    pkill -x lina-gpui-bin 2>/dev/null || true
    sleep 1
    open "$APP"
  else
    log "adiado pelo usuário — binário novo fica pronto; a próxima abertura é instantânea."
  fi
}

while true; do
  if changed_since_sentinel; then
    touch "$SENTINEL"           # marca ANTES de buildar: build falho não vira loop; edição nova re-dispara
    rebuild && ask_and_apply
  fi
  sleep "$POLL"
done
