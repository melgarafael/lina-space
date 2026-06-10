#!/bin/sh
# F1-5-1 · Gerador de carga REPRODUTÍVEL para a sonda [PROF] (sem CLI de IA real).
# Uso: loadgen.sh <burst|spinner|silence> [seed]
#   burst   — rajada contínua de linhas coloridas (~250 linhas/s: 50 linhas + 0.2s de pausa)
#   spinner — spinner ANSI in-place (\r, ~20 updates/s) + 1 linha de status a cada 100 ticks
#   silence — 1 linha de apresentação e silêncio (painel OCIOSO de verdade)
# As taxas são FIXAS no script (mude AQUI, não por env) — reprodutibilidade da matriz N∈{4,16,28}.
# O app sobe N painéis rodando isto via LINA_LOAD=N (ver tasks/epico-f1/prof-baseline.md).

MODE="${1:-burst}"
SEED="${2:-1}"

case "$MODE" in
  burst)
    i=0
    while :; do
      i=$((i + 1))
      # Cor ANSI varia por linha (3 runs de estilo por linha: cor + texto + reset) — exercita
      # o agrupamento de runs do render como output real de CLI exercitaria.
      printf '\033[3%dm[carga %s]\033[0m linha %06d: lorem ipsum dolor sit amet consectetur adipiscing elit deadbeefcafe\n' \
        "$((i % 6 + 1))" "$SEED" "$i"
      [ $((i % 50)) -eq 0 ] && sleep 0.2
    done
    ;;
  spinner)
    i=0
    while :; do
      i=$((i + 1))
      case $((i % 4)) in
        0) c='|' ;; 1) c='/' ;; 2) c='-' ;; 3) c='\' ;;
      esac
      printf '\r\033[36m%s trabalhando %06d (painel %s)\033[0m' "$c" "$i" "$SEED"
      [ $((i % 100)) -eq 0 ] && printf '\n\033[32mok etapa %d concluida\033[0m\n' "$((i / 100))"
      sleep 0.05
    done
    ;;
  silence)
    printf 'painel %s ocioso — sem output (cenario-alvo: fila visivel)\n' "$SEED"
    while :; do sleep 3600; done
    ;;
  *)
    echo "loadgen: modo desconhecido '$MODE' (use burst|spinner|silence)" >&2
    exit 2
    ;;
esac
